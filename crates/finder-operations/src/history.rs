//! Records of completed work that Undo and Redo reverse.
//!
//! An operation journals each change as it completes, so a batch that stops
//! part way still reports the work it did. Reversing a record checks every
//! item again, never replaces an existing item and journals its own changes:
//! that record reverses the reversal, so Redo is the Undo of an Undo.
//! Removing what an operation made moves it to the Trash, never deletes it.

use crate::{Operation, checked_directory, describe, ensure_absent, item_word, move_one, platform};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The kind of operation a record holds. Reversing it keeps the kind, so
/// Redo names the operation Undo reversed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    NewFolder,
    NewFile,
    Rename,
    Copy,
    Move,
    Trash,
    Compress,
    Extract,
}

impl Kind {
    pub(crate) fn of(operation: &Operation) -> Option<Self> {
        Some(match operation {
            Operation::CreateFolder { .. } => Self::NewFolder,
            Operation::CreateFile { .. } => Self::NewFile,
            Operation::Rename { .. } => Self::Rename,
            Operation::Copy { .. } => Self::Copy,
            Operation::Move { .. } => Self::Move,
            Operation::Trash { .. } => Self::Trash,
            Operation::Compress { .. } => Self::Compress,
            Operation::Extract { .. } => Self::Extract,
            Operation::Reverse(record) => record.kind,
            Operation::Open { .. } | Operation::Reveal { .. } => return None,
        })
    }
}

/// The changes one operation completed, which `Operation::Reverse` undoes.
/// It holds at most one change per selected item.
#[derive(Clone, Debug)]
pub struct Record {
    kind: Kind,
    steps: Vec<Step>,
}

impl Record {
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// How many items the record changed.
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Where the record's items are now. Items in the Trash have no place.
    pub fn items(&self) -> impl Iterator<Item = &Path> {
        self.steps.iter().filter_map(|step| match step {
            Step::Moved { to, .. } => Some(to.as_path()),
            Step::Created { path, .. } => Some(path.as_path()),
            Step::Trashed { .. } => None,
        })
    }
}

#[derive(Clone, Debug)]
enum Step {
    /// An item moved or renamed from `from` to `to`. Reversing moves it back.
    Moved {
        from: PathBuf,
        to: PathBuf,
        folder: bool,
    },
    /// An item the operation made. Reversing moves it to the Trash.
    Created { path: PathBuf, folder: bool },
    /// An item moved to the Trash from `path`. Reversing restores it there.
    Trashed { path: PathBuf, ticket: Ticket },
}

/// How the Trash finds an item it received: the item's identity, and the
/// seconds since the Unix epoch between which it was recycled.
#[derive(Clone, Debug)]
pub(crate) struct Ticket {
    identity: platform::Identity,
    since: i64,
    until: i64,
}

/// The recoverable system Trash. Tests supply a fixture, so they never touch
/// the user's Trash.
pub(crate) trait Bin {
    fn recycle(&self, path: &Path) -> Result<Ticket, String>;
    /// Put an item back at `original`, which the caller has found free.
    fn restore(&self, ticket: &Ticket, original: &Path) -> Result<(), String>;
}

pub(crate) struct SystemBin;

impl Bin for SystemBin {
    fn recycle(&self, path: &Path) -> Result<Ticket, String> {
        let metadata =
            fs::symlink_metadata(path).map_err(|error| describe("inspect", path, error))?;
        let since = now();
        trash::delete(path).map_err(|error| error.to_string())?;
        Ok(Ticket {
            identity: platform::identity(&metadata),
            since,
            until: now(),
        })
    }

    fn restore(&self, ticket: &Ticket, original: &Path) -> Result<(), String> {
        // Trash records keep whole seconds, perhaps from another clock.
        platform::restore_trashed(
            original,
            ticket.identity,
            ticket.since.saturating_sub(1)..=ticket.until.saturating_add(1),
        )
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

/// The changes an operation has completed so far.
#[derive(Default)]
pub(crate) struct Journal(Vec<Step>);

impl Journal {
    /// An item now at `path` that did not exist before.
    pub(crate) fn created(&mut self, path: &Path) {
        if let Some(folder) = folder(path) {
            self.0.push(Step::Created {
                path: path.to_owned(),
                folder,
            });
        }
    }

    pub(crate) fn moved(&mut self, from: &Path, to: &Path) {
        if let Some(folder) = folder(to) {
            self.0.push(Step::Moved {
                from: from.to_owned(),
                to: to.to_owned(),
                folder,
            });
        }
    }

    pub(crate) fn trashed(&mut self, path: &Path, ticket: Ticket) {
        self.0.push(Step::Trashed {
            path: path.to_owned(),
            ticket,
        });
    }

    pub(crate) fn finish(self, kind: Option<Kind>) -> Option<Record> {
        let kind = kind.filter(|_| !self.0.is_empty())?;
        Some(Record {
            kind,
            steps: self.0,
        })
    }
}

/// Whether the item at `path` is a folder. An item that cannot be inspected
/// right after the operation made it is not journaled, as it could not be
/// checked again before reversing.
fn folder(path: &Path) -> Option<bool> {
    fs::symlink_metadata(path)
        .ok()
        .map(|metadata| metadata.is_dir())
}

/// Reverse every change in `record`, newest first. Each item is checked and
/// reversed on its own, so one that has since moved, or whose place is now
/// taken, does not hold back the others; it is reported and left alone.
pub(crate) fn reverse(
    record: Record,
    bin: &dyn Bin,
    journal: &mut Journal,
) -> Result<String, String> {
    let total = record.steps.len();
    let mut failed = 0;
    let mut first = None;
    for step in record.steps.into_iter().rev() {
        if let Err(error) = reverse_step(step, bin, journal) {
            failed += 1;
            first.get_or_insert(error);
        }
    }
    let Some(mut message) = first else {
        return Ok(format!("Reversed {total} {}", item_word(total)));
    };
    if total > 1 {
        message = format!(
            "{} of {total} items were changed back. {message}",
            total - failed
        );
    }
    if failed > 1 {
        message.push_str(&format!(
            " {} other {} could not be changed back either.",
            failed - 1,
            item_word(failed - 1)
        ));
    }
    Err(message)
}

fn reverse_step(step: Step, bin: &dyn Bin, journal: &mut Journal) -> Result<(), String> {
    match step {
        Step::Moved { from, to, folder } => {
            let current = present(&to, folder)?;
            let original = vacant(&from)?;
            match move_one(&current, &original, bin) {
                Ok(()) => {
                    journal.moved(&current, &original);
                    Ok(())
                }
                Err(error) => {
                    // A move between volumes that kept the item where it was
                    // still published a complete copy in its old place.
                    if error.copied {
                        journal.created(&original);
                    }
                    Err(error.message)
                }
            }
        }
        Step::Created { path, folder } => {
            let current = present(&path, folder)?;
            let ticket = bin.recycle(&current).map_err(|error| {
                format!("Could not move {} to the Trash: {error}", current.display())
            })?;
            journal.trashed(&current, ticket);
            Ok(())
        }
        Step::Trashed { path, ticket } => {
            let original = vacant(&path)?;
            bin.restore(&ticket, &original)?;
            journal.created(&original);
            Ok(())
        }
    }
}

/// The recorded item at `path`, if it is still there and still a folder, or
/// still not one, as recorded. Its contents may have changed: they go with it.
fn present(path: &Path, folder: bool) -> Result<PathBuf, String> {
    let path = in_real_folder(path)?;
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_dir() == folder => Ok(path),
        Ok(_) => Err(format!(
            "{} has been replaced by a different kind of item, which was left in place",
            path.display()
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Err(format!("{} is no longer there", path.display()))
        }
        Err(error) => Err(describe("inspect", &path, error)),
    }
}

/// `path`, if its folder still exists and nothing has taken its place.
fn vacant(path: &Path) -> Result<PathBuf, String> {
    let path = in_real_folder(path)?;
    ensure_absent(&path)?;
    Ok(path)
}

fn in_real_folder(path: &Path) -> Result<PathBuf, String> {
    let name = path
        .file_name()
        .ok_or("A filesystem root cannot be changed back")?;
    let parent = path.parent().ok_or("The item has no containing folder")?;
    Ok(checked_directory(parent)?.join(name))
}

/// A Trash for tests: recycled items move into a fixture folder, never into
/// the user's Trash.
#[cfg(test)]
pub(crate) struct FixtureBin {
    folder: PathBuf,
    next: std::sync::atomic::AtomicUsize,
    items: std::sync::Mutex<Vec<(PathBuf, PathBuf)>>,
}

#[cfg(test)]
impl FixtureBin {
    pub(crate) fn new(folder: PathBuf) -> Self {
        fs::create_dir_all(&folder).expect("create fixture Trash");
        Self {
            folder,
            next: Default::default(),
            items: Default::default(),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.items.lock().unwrap().len()
    }
}

#[cfg(test)]
impl Bin for FixtureBin {
    fn recycle(&self, path: &Path) -> Result<Ticket, String> {
        let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
        let next = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let binned = self.folder.join(next.to_string());
        fs::rename(path, &binned).map_err(|error| error.to_string())?;
        self.items.lock().unwrap().push((path.to_owned(), binned));
        Ok(Ticket {
            identity: platform::identity(&metadata),
            since: 0,
            until: 0,
        })
    }

    fn restore(&self, _ticket: &Ticket, original: &Path) -> Result<(), String> {
        let mut items = self.items.lock().unwrap();
        let index = items
            .iter()
            .rposition(|(path, _)| path == original)
            .ok_or("not in the fixture Trash")?;
        platform::rename_noreplace(&items[index].1, original).map_err(|error| error.to_string())?;
        items.remove(index);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Operation, run};

    struct Fixture {
        _directory: tempfile::TempDir,
        root: PathBuf,
        bin: FixtureBin,
    }

    impl Fixture {
        fn new() -> Self {
            let directory = tempfile::tempdir().expect("create isolated test fixture");
            // macOS exposes its temporary root through /var -> /private/var.
            let root = fs::canonicalize(directory.path()).expect("canonical test fixture");
            let bin = FixtureBin::new(root.join("fixture-trash"));
            Self {
                _directory: directory,
                root,
                bin,
            }
        }

        fn run(&self, operation: Operation) -> (Result<String, String>, Option<Record>) {
            run(operation, &self.bin)
        }

        /// Perform an operation that must succeed, returning its record.
        fn perform(&self, operation: Operation) -> Record {
            let (result, record) = self.run(operation);
            result.expect("operation failed");
            record.expect("operation recorded nothing")
        }

        fn reverse(&self, record: Record) -> Record {
            self.perform(Operation::Reverse(record))
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.root.join(relative)
        }

        fn write(&self, relative: &str, contents: &str) {
            fs::write(self.path(relative), contents).unwrap();
        }

        fn read(&self, relative: &str) -> String {
            fs::read_to_string(self.path(relative)).unwrap()
        }

        fn exists(&self, relative: &str) -> bool {
            fs::symlink_metadata(self.path(relative)).is_ok()
        }
    }

    #[test]
    fn undoing_a_move_puts_items_back_and_redo_moves_them_again() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.path("from/folder/nested")).unwrap();
        fs::create_dir(fixture.path("to")).unwrap();
        fixture.write("from/a.txt", "a");
        fixture.write("from/folder/nested/b.txt", "b");
        let moved = fixture.perform(Operation::Move {
            sources: vec![fixture.path("from/a.txt"), fixture.path("from/folder")],
            destination: fixture.path("to"),
        });
        assert_eq!(moved.kind(), Kind::Move);
        assert_eq!(moved.len(), 2);
        assert_eq!(
            moved.items().collect::<Vec<_>>(),
            [fixture.path("to/a.txt"), fixture.path("to/folder")]
        );

        let undone = fixture.reverse(moved);
        assert_eq!(undone.kind(), Kind::Move);
        assert_eq!(fixture.read("from/a.txt"), "a");
        assert_eq!(fixture.read("from/folder/nested/b.txt"), "b");
        assert!(!fixture.exists("to/a.txt") && !fixture.exists("to/folder"));
        let mut back = undone.items().map(Path::to_owned).collect::<Vec<_>>();
        back.sort();
        assert_eq!(
            back,
            [fixture.path("from/a.txt"), fixture.path("from/folder")]
        );

        fixture.reverse(undone);
        assert_eq!(fixture.read("to/a.txt"), "a");
        assert_eq!(fixture.read("to/folder/nested/b.txt"), "b");
        assert!(!fixture.exists("from/a.txt") && !fixture.exists("from/folder"));
        assert_eq!(fixture.bin.len(), 0, "a move never needs the Trash");
    }

    #[test]
    fn undoing_a_copy_moves_only_the_copies_to_the_trash_and_redo_restores_them() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.path("from/folder")).unwrap();
        fs::create_dir(fixture.path("to")).unwrap();
        fixture.write("from/a.txt", "a");
        fixture.write("from/folder/b.txt", "b");
        let copied = fixture.perform(Operation::Copy {
            sources: vec![fixture.path("from/a.txt"), fixture.path("from/folder")],
            destination: fixture.path("to"),
        });
        // Edits to a copy, even one saved as a new file, go with it.
        fs::remove_file(fixture.path("to/a.txt")).unwrap();
        fixture.write("to/a.txt", "edited copy");

        let undone = fixture.reverse(copied);
        assert!(!fixture.exists("to/a.txt") && !fixture.exists("to/folder"));
        assert_eq!(fixture.read("from/a.txt"), "a");
        assert_eq!(fixture.read("from/folder/b.txt"), "b");
        assert_eq!(fixture.bin.len(), 2, "copies are recycled, never deleted");
        assert_eq!(undone.items().count(), 0, "the copies are in the Trash");

        let redone = fixture.reverse(undone);
        assert_eq!(fixture.read("to/a.txt"), "edited copy");
        assert_eq!(fixture.read("to/folder/b.txt"), "b");
        assert_eq!(fixture.bin.len(), 0);
        assert_eq!(redone.kind(), Kind::Copy);
    }

    #[test]
    fn rename_new_items_and_trash_reverse_and_redo() {
        let fixture = Fixture::new();
        let folder = fixture.perform(Operation::CreateFolder {
            parent: fixture.root.clone(),
            name: "New Folder".into(),
        });
        let file = fixture.perform(Operation::CreateFile {
            parent: fixture.root.clone(),
            name: "notes.txt".into(),
        });
        assert_eq!(
            (folder.kind(), file.kind()),
            (Kind::NewFolder, Kind::NewFile)
        );
        let renamed = fixture.perform(Operation::Rename {
            source: fixture.path("notes.txt"),
            name: "plans.txt".into(),
        });
        fixture.write("plans.txt", "kept through every step");
        let trashed = fixture.perform(Operation::Trash {
            paths: vec![fixture.path("plans.txt"), fixture.path("New Folder")],
        });
        assert!(!fixture.exists("plans.txt") && !fixture.exists("New Folder"));

        let restored = fixture.reverse(trashed);
        assert_eq!(fixture.read("plans.txt"), "kept through every step");
        assert!(fixture.path("New Folder").is_dir());
        // Redo recycles them again; Undo of that restores them again.
        let retrashed = fixture.reverse(restored);
        assert!(!fixture.exists("plans.txt"));
        fixture.reverse(retrashed);
        assert!(fixture.exists("plans.txt"));

        let unrenamed = fixture.reverse(renamed);
        assert_eq!(fixture.read("notes.txt"), "kept through every step");
        fixture.reverse(unrenamed);
        assert!(fixture.exists("plans.txt") && !fixture.exists("notes.txt"));

        fixture.reverse(folder);
        assert!(!fixture.exists("New Folder"));
        assert_eq!(fixture.bin.len(), 1);
    }

    #[test]
    fn archives_and_extractions_are_recycled_on_undo() {
        let fixture = Fixture::new();
        fixture.write("report.txt", "report");
        let compressed = fixture.perform(Operation::Compress {
            sources: vec![fixture.path("report.txt")],
            destination: fixture.root.clone(),
            name: "report.zip".into(),
        });
        fs::create_dir(fixture.path("out")).unwrap();
        let extracted = fixture.perform(Operation::Extract {
            archive: fixture.path("report.zip"),
            destination: fixture.path("out"),
        });
        assert_eq!(
            extracted.items().collect::<Vec<_>>(),
            [fixture.path("out/report.txt")]
        );
        fixture.reverse(extracted);
        assert!(!fixture.exists("out/report.txt"));
        fixture.reverse(compressed);
        assert!(!fixture.exists("report.zip"));
        assert_eq!(fixture.read("report.txt"), "report");
    }

    #[test]
    fn reversal_never_replaces_an_item_and_still_reverses_the_others() {
        let fixture = Fixture::new();
        for folder in ["from", "to"] {
            fs::create_dir(fixture.path(folder)).unwrap();
        }
        for name in ["a", "b", "c"] {
            fixture.write(&format!("from/{name}"), name);
        }
        let moved = fixture.perform(Operation::Move {
            sources: ["a", "b", "c"]
                .map(|name| fixture.path(&format!("from/{name}")))
                .into(),
            destination: fixture.path("to"),
        });
        // Something new took b's old place, and c moved on elsewhere.
        fixture.write("from/b", "newer b");
        fs::rename(fixture.path("to/c"), fixture.path("c-elsewhere")).unwrap();

        let (result, undone) = fixture.run(Operation::Reverse(moved));
        let error = result.unwrap_err();
        assert!(
            error.starts_with("1 of 3 items were changed back."),
            "{error}"
        );
        assert!(
            error.contains("1 other item could not be changed back"),
            "{error}"
        );
        assert_eq!(fixture.read("from/a"), "a");
        assert_eq!(fixture.read("from/b"), "newer b");
        assert_eq!(fixture.read("to/b"), "b");
        assert_eq!(fixture.read("c-elsewhere"), "c");
        // Only the item that moved back is redone.
        let undone = undone.unwrap();
        assert_eq!(undone.items().collect::<Vec<_>>(), [fixture.path("from/a")]);
        fixture.reverse(undone);
        assert_eq!(fixture.read("to/a"), "a");
    }

    #[test]
    fn reversal_leaves_an_item_replaced_by_a_different_kind_alone() {
        let fixture = Fixture::new();
        let created = fixture.perform(Operation::CreateFolder {
            parent: fixture.root.clone(),
            name: "draft".into(),
        });
        fs::remove_dir(fixture.path("draft")).unwrap();
        fixture.write("draft", "a file now");
        let (result, record) = fixture.run(Operation::Reverse(created));
        assert!(result.unwrap_err().contains("different kind of item"));
        assert!(record.is_none());
        assert_eq!(fixture.read("draft"), "a file now");
        assert_eq!(fixture.bin.len(), 0);
    }

    #[test]
    fn restoring_from_the_trash_never_replaces_a_new_item() {
        let fixture = Fixture::new();
        fixture.write("notes.txt", "trashed");
        let trashed = fixture.perform(Operation::Trash {
            paths: vec![fixture.path("notes.txt")],
        });
        fixture.write("notes.txt", "written later");
        let (result, record) = fixture.run(Operation::Reverse(trashed));
        assert!(result.unwrap_err().contains("already exists"));
        assert!(record.is_none());
        assert_eq!(fixture.read("notes.txt"), "written later");
        assert_eq!(fixture.bin.len(), 1, "the trashed item stays recoverable");
    }

    #[cfg(unix)]
    #[test]
    fn a_batch_that_fails_part_way_records_the_items_it_completed() {
        use std::os::unix::net::UnixListener;
        let fixture = Fixture::new();
        for folder in ["first", "second", "to"] {
            fs::create_dir(fixture.path(folder)).unwrap();
        }
        fixture.write("first/a", "a");
        let _socket = UnixListener::bind(fixture.path("second/socket")).unwrap();
        let (result, record) = fixture.run(Operation::Copy {
            sources: vec![fixture.path("first"), fixture.path("second")],
            destination: fixture.path("to"),
        });
        assert!(result.unwrap_err().starts_with("Copied 1 of 2 items."));
        let record = record.expect("the completed copy is recorded");
        assert_eq!(
            record.items().collect::<Vec<_>>(),
            [fixture.path("to/first")]
        );
        fixture.reverse(record);
        assert!(!fixture.exists("to/first"));
        assert_eq!(fixture.read("first/a"), "a");
    }

    #[test]
    fn open_and_reveal_record_nothing_and_an_empty_journal_is_no_record() {
        let fixture = Fixture::new();
        assert!(
            Kind::of(&Operation::Open {
                path: fixture.root.clone()
            })
            .is_none()
        );
        assert!(
            Kind::of(&Operation::Reveal {
                path: fixture.root.clone()
            })
            .is_none()
        );
        let (result, record) = fixture.run(Operation::CreateFolder {
            parent: fixture.path("missing"),
            name: "x".into(),
        });
        assert!(result.is_err() && record.is_none());
    }
}
