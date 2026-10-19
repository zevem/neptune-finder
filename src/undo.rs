//! Undo and Redo of completed file operations, for the whole window and
//! newest last. Records name paths, never file contents, and are not saved:
//! a restored session starts with an empty history.

use finder_operations::Record;
use std::collections::VecDeque;

/// Records kept for Undo; the oldest go first.
pub const MAX_RECORDS: usize = 64;
/// Items the kept records may name together. One record can name every item
/// of a large selection, so this still keeps several of the largest.
const MAX_ITEMS: usize = 16_384;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Undo,
    Redo,
}

/// What asking for an Undo or Redo led to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asked {
    /// There is nothing to reverse.
    Nothing,
    /// It waits for the file work in progress, and then reverses that.
    Waiting,
    Ready,
}

#[derive(Default)]
pub struct History {
    undo: VecDeque<Record>,
    redo: Vec<Record>,
    /// Undo (above zero) or Redo (below) steps not yet started.
    queued: isize,
}

impl History {
    /// A new operation completed: it is undone next, and what was undone
    /// before it can no longer be redone.
    pub fn record(&mut self, record: Record) {
        if record.is_empty() {
            return;
        }
        self.redo.clear();
        self.undo.push_back(record);
        self.trim();
    }

    /// Ask for an Undo or Redo. While file work is running, it waits for
    /// that work to finish and join the history, so it reverses that work
    /// rather than something older. An Undo and a Redo asked for together
    /// cancel out.
    pub fn ask(&mut self, direction: Direction, busy: bool) -> Asked {
        if !busy && self.next(direction).is_none() {
            return Asked::Nothing;
        }
        let limit = MAX_RECORDS as isize;
        self.queued = match direction {
            Direction::Undo => self.queued + 1,
            Direction::Redo => self.queued - 1,
        }
        .clamp(-limit, limit);
        if busy && self.queued != 0 {
            Asked::Waiting
        } else {
            Asked::Ready
        }
    }

    /// The next asked-for reversal, once no file work is running. It runs
    /// alone: steps after it wait for it in turn.
    pub fn start(&mut self, busy: bool) -> Option<(Direction, Record)> {
        if busy || self.queued == 0 {
            return None;
        }
        let direction = if self.queued > 0 {
            Direction::Undo
        } else {
            Direction::Redo
        };
        self.queued -= self.queued.signum();
        let record = self.take(direction);
        if record.is_none() {
            self.queued = 0;
        }
        record.map(|record| (direction, record))
    }

    /// Forget the steps not yet started: the work they waited for failed, or
    /// a reversal did, and the next step would reverse something else.
    pub fn cancel_waiting(&mut self) {
        self.queued = 0;
    }

    /// The record Undo or Redo reverses next.
    pub fn next(&self, direction: Direction) -> Option<&Record> {
        match direction {
            Direction::Undo => self.undo.back(),
            Direction::Redo => self.redo.last(),
        }
    }

    fn take(&mut self, direction: Direction) -> Option<Record> {
        match direction {
            Direction::Undo => self.undo.pop_back(),
            Direction::Redo => self.redo.pop(),
        }
    }

    /// Return a record whose reversal could not start.
    pub fn put_back(&mut self, direction: Direction, record: Record) {
        match direction {
            Direction::Undo => self.undo.push_back(record),
            Direction::Redo => self.redo.push(record),
        }
    }

    /// An Undo or Redo finished: the record of what it changed reverses it
    /// in turn. Items it could not change back are no longer in the history.
    pub fn reversed(&mut self, direction: Direction, record: Record) {
        if record.is_empty() {
            return;
        }
        match direction {
            Direction::Undo => self.redo.push(record),
            Direction::Redo => {
                self.undo.push_back(record);
                self.trim();
            }
        }
    }

    /// Reversals only move records between the stacks, so bounding Undo when
    /// it grows bounds them both.
    fn trim(&mut self) {
        let mut items: usize = self.undo.iter().chain(&self.redo).map(Record::len).sum();
        while self.undo.len() > 1 && (self.undo.len() > MAX_RECORDS || items > MAX_ITEMS) {
            if let Some(oldest) = self.undo.pop_front() {
                items -= oldest.len();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use finder_operations::{Operation, Operations};
    use std::path::Path;
    use std::time::{Duration, Instant};

    /// Records of real operations in an isolated fixture: one new folder
    /// each, made one after another.
    fn records(root: &Path, names: &[&str]) -> Vec<Record> {
        let operations = Operations::new(|| {});
        names
            .iter()
            .map(|name| {
                operations
                    .submit(Operation::CreateFolder {
                        parent: root.to_owned(),
                        name: (*name).to_owned(),
                    })
                    .unwrap();
                let deadline = Instant::now() + Duration::from_secs(10);
                loop {
                    if let Some(completion) = operations.poll() {
                        break completion.record.expect("a folder record");
                    }
                    assert!(Instant::now() < deadline, "the operation did not finish");
                    std::thread::sleep(Duration::from_millis(1));
                }
            })
            .collect()
    }

    fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        (directory, root)
    }

    fn newest(history: &History, direction: Direction) -> Option<std::path::PathBuf> {
        history
            .next(direction)
            .and_then(|record| record.items().next().map(Path::to_owned))
    }

    #[test]
    fn undo_and_redo_alternate_and_a_new_operation_ends_redo() {
        let (_directory, root) = fixture();
        let mut made = records(&root, &["a", "b", "c"]).into_iter();
        let mut history = History::default();
        history.record(made.next().unwrap());
        history.record(made.next().unwrap());
        assert_eq!(newest(&history, Direction::Undo), Some(root.join("b")));
        assert!(history.next(Direction::Redo).is_none());

        // An Undo's record (here, any record) becomes the next Redo.
        let undone = history.take(Direction::Undo).unwrap();
        history.reversed(Direction::Undo, undone);
        assert_eq!(newest(&history, Direction::Undo), Some(root.join("a")));
        assert_eq!(newest(&history, Direction::Redo), Some(root.join("b")));
        let redone = history.take(Direction::Redo).unwrap();
        history.reversed(Direction::Redo, redone);
        assert_eq!(newest(&history, Direction::Undo), Some(root.join("b")));

        let undone = history.take(Direction::Undo).unwrap();
        history.reversed(Direction::Undo, undone);
        history.record(made.next().unwrap());
        assert_eq!(newest(&history, Direction::Undo), Some(root.join("c")));
        assert!(history.next(Direction::Redo).is_none());
    }

    #[test]
    fn steps_asked_for_during_file_work_wait_for_it_and_then_reverse_it() {
        let (_directory, root) = fixture();
        let mut made = records(&root, &["older", "newer"]).into_iter();
        let mut history = History::default();
        history.record(made.next().unwrap());
        // Undo is asked for twice while the newer operation still runs.
        assert_eq!(history.ask(Direction::Undo, true), Asked::Waiting);
        assert_eq!(history.ask(Direction::Undo, true), Asked::Waiting);
        assert!(history.start(true).is_none(), "a step started during work");
        history.record(made.next().unwrap());
        let (direction, record) = history.start(false).unwrap();
        assert_eq!(direction, Direction::Undo);
        assert_eq!(record.items().next(), Some(root.join("newer").as_path()));
        // The second waits for the first reversal, then reverses the older.
        assert!(history.start(true).is_none());
        history.reversed(Direction::Undo, record);
        let (_, record) = history.start(false).unwrap();
        assert_eq!(record.items().next(), Some(root.join("older").as_path()));
        assert!(history.start(false).is_none());
    }

    #[test]
    fn undo_and_redo_asked_for_together_cancel_out_and_failures_stop_the_rest() {
        let (_directory, root) = fixture();
        let mut history = History::default();
        assert_eq!(history.ask(Direction::Undo, false), Asked::Nothing);
        assert_eq!(history.ask(Direction::Redo, false), Asked::Nothing);
        history.record(records(&root, &["a"]).remove(0));
        assert_eq!(history.ask(Direction::Undo, true), Asked::Waiting);
        assert_eq!(history.ask(Direction::Redo, true), Asked::Ready);
        assert!(history.start(false).is_none());
        assert!(
            history.next(Direction::Undo).is_some(),
            "nothing was undone"
        );

        assert_eq!(history.ask(Direction::Undo, true), Asked::Waiting);
        assert_eq!(history.ask(Direction::Undo, true), Asked::Waiting);
        history.cancel_waiting();
        assert!(history.start(false).is_none());
        // More steps than records start only as many reversals as there are.
        assert_eq!(history.ask(Direction::Undo, false), Asked::Ready);
        assert_eq!(history.ask(Direction::Undo, true), Asked::Waiting);
        assert!(history.start(false).is_some());
        assert!(history.start(false).is_none());
        assert_eq!(history.ask(Direction::Undo, false), Asked::Nothing);
    }

    #[test]
    fn a_record_whose_reversal_could_not_start_stays_next() {
        let (_directory, root) = fixture();
        let mut history = History::default();
        history.record(records(&root, &["a"]).remove(0));
        let record = history.take(Direction::Undo).unwrap();
        assert!(history.next(Direction::Undo).is_none());
        history.put_back(Direction::Undo, record);
        assert_eq!(newest(&history, Direction::Undo), Some(root.join("a")));
    }

    #[test]
    fn the_history_keeps_only_the_newest_records() {
        let (_directory, root) = fixture();
        let names = (0..MAX_RECORDS + 3)
            .map(|index| format!("{index:03}"))
            .collect::<Vec<_>>();
        let names = names.iter().map(String::as_str).collect::<Vec<_>>();
        let mut history = History::default();
        for record in records(&root, &names) {
            history.record(record);
        }
        let mut kept = 0;
        while let Some(record) = history.take(Direction::Undo) {
            kept += 1;
            assert_eq!(
                record.items().next(),
                Some(
                    root.join(format!("{:03}", MAX_RECORDS + 3 - kept))
                        .as_path()
                )
            );
        }
        assert_eq!(kept, MAX_RECORDS);
    }
}
