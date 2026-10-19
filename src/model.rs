//! Presentation state. Paths keep their original OsString identity, even when
//! two names have the same lossy display representation.
use crate::config::Config;
use finder_core::{Entry, SortOrder};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    ops::Range,
    path::{Path, PathBuf},
    time::Duration,
};

/// The entries a selection rectangle covers: `rows` × `columns` of a layout
/// `stride` entries wide. A list is one column wide.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Band {
    pub rows: Range<usize>,
    pub columns: Range<usize>,
    pub stride: usize,
}
impl Band {
    fn contains(&self, index: usize) -> bool {
        self.stride > 0
            && self.rows.contains(&(index / self.stride))
            && self.columns.contains(&(index % self.stride))
    }
    /// Covered indices in ascending order, below `len`.
    fn indices(&self, len: usize) -> impl Iterator<Item = usize> + '_ {
        let rows = if self.stride > 0 {
            self.rows.clone()
        } else {
            0..0
        };
        rows.flat_map(move |row| {
            self.columns
                .clone()
                .map(move |column| row * self.stride + column)
        })
        .take_while(move |index| *index < len)
    }
}

/// A selection rectangle being dragged: the selection it started from, and
/// what it covers now.
struct Marquee {
    base: BTreeSet<PathBuf>,
    toggle: bool,
    band: Band,
}
impl Marquee {
    /// Whether a path is selected while covered (or not): covered paths are
    /// added, or toggled against the starting selection; others keep it.
    fn selects(&self, path: &Path, covered: bool) -> bool {
        let before = self.base.contains(path);
        if covered {
            !(self.toggle && before)
        } else {
            before
        }
    }
}

pub struct Pane {
    pub id: u64,
    pub path: PathBuf,
    pub history: Vec<PathBuf>,
    pub history_index: usize,
    pub entries: Vec<Entry>,
    pub selected: BTreeSet<PathBuf>,
    pub anchor: Option<usize>,
    pub query: String,
    pub recursive: bool,
    pub sort: SortOrder,
    pub loading: bool,
    pub error: Option<String>,
    pub generation: u64,
    pub elapsed: Duration,
    pub truncated: bool,
    pub focus_index: Option<usize>,
    pub scroll_to: Option<usize>,
    marquee: Option<Marquee>,
}
impl Pane {
    pub fn new(id: u64, path: PathBuf) -> Self {
        Self {
            id,
            history: vec![path.clone()],
            path,
            history_index: 0,
            entries: Vec::new(),
            selected: BTreeSet::new(),
            anchor: None,
            query: String::new(),
            recursive: false,
            sort: SortOrder::default(),
            loading: true,
            error: None,
            generation: 0,
            elapsed: Duration::ZERO,
            truncated: false,
            focus_index: None,
            scroll_to: None,
            marquee: None,
        }
    }
    pub fn navigate(&mut self, path: PathBuf) {
        if path != self.path {
            self.history.truncate(self.history_index + 1);
            self.history.push(path.clone());
            if self.history.len() > 128 {
                self.history.remove(0);
            }
            self.history_index = self.history.len() - 1;
            self.path = path;
        }
        self.clear_location_state();
    }
    pub fn travel(&mut self, forward: bool) -> bool {
        let next = if forward {
            self.history_index
                .checked_add(1)
                .filter(|i| *i < self.history.len())
        } else {
            self.history_index.checked_sub(1)
        };
        if let Some(next) = next {
            self.history_index = next;
            self.path = self.history[next].clone();
            self.clear_location_state();
            true
        } else {
            false
        }
    }
    fn clear_location_state(&mut self) {
        self.query.clear();
        self.entries.clear();
        self.selected.clear();
        self.anchor = None;
        self.focus_index = None;
        self.scroll_to = None;
        self.error = None;
        self.marquee = None;
    }
    pub fn select(&mut self, index: usize, toggle: bool, range: bool) {
        if index >= self.entries.len() {
            return;
        }
        if range {
            if !toggle {
                self.selected.clear();
            }
            let anchor = self.anchor.unwrap_or(index).min(self.entries.len() - 1);
            for i in anchor.min(index)..=anchor.max(index) {
                self.selected.insert(self.entries[i].path.clone());
            }
        } else if toggle {
            let path = self.entries[index].path.clone();
            if !self.selected.remove(&path) {
                self.selected.insert(path);
            }
            self.anchor = Some(index);
        } else {
            self.selected.clear();
            self.selected.insert(self.entries[index].path.clone());
            self.anchor = Some(index);
        }
        self.focus_index = Some(index);
    }
    pub fn selected_entry(&self) -> Option<&Entry> {
        (self.selected.len() == 1)
            .then(|| {
                self.entries
                    .iter()
                    .find(|e| self.selected.contains(&e.path))
            })
            .flatten()
    }
    /// Starts a selection rectangle. It replaces the selection, adds to it
    /// (`extend`), or toggles what it covers (`toggle`).
    pub fn begin_marquee(&mut self, extend: bool, toggle: bool) {
        if !extend && !toggle {
            self.selected.clear();
        }
        self.marquee = Some(Marquee {
            base: self.selected.clone(),
            toggle,
            band: Band::default(),
        });
    }
    pub fn marquee_active(&self) -> bool {
        self.marquee.is_some()
    }
    /// Covers `band`, changing only the entries that entered or left it, so
    /// a sweep across a large folder costs what it changes.
    pub fn marquee_to(&mut self, band: Band) {
        let Some(marquee) = self.marquee.as_mut() else {
            return;
        };
        if marquee.band == band {
            return;
        }
        let previous = std::mem::replace(&mut marquee.band, band);
        let len = self.entries.len();
        let changed = previous
            .indices(len)
            .filter(|&index| !marquee.band.contains(index))
            .map(|index| (index, false))
            .chain(
                marquee
                    .band
                    .indices(len)
                    .filter(|&index| !previous.contains(index))
                    .map(|index| (index, true)),
            );
        for (index, covered) in changed {
            let path = &self.entries[index].path;
            if !marquee.selects(path, covered) {
                self.selected.remove(path);
            } else if !self.selected.contains(path) {
                self.selected.insert(path.clone());
            }
        }
    }
    /// The listing under a selection rectangle changed: it now covers the
    /// entries in its place.
    pub fn refresh_marquee(&mut self) {
        if let Some(marquee) = &self.marquee {
            self.selected = self
                .entries
                .iter()
                .enumerate()
                .filter(|(index, entry)| {
                    marquee.selects(&entry.path, marquee.band.contains(*index))
                })
                .map(|(_, entry)| entry.path.clone())
                .collect();
        }
    }
    /// Ends a selection rectangle; the keyboard continues from the first
    /// entry it covered.
    pub fn end_marquee(&mut self) {
        if let Some(marquee) = self.marquee.take()
            && let Some(first) = marquee.band.indices(self.entries.len()).next()
        {
            self.anchor = Some(first);
            self.focus_index = Some(first);
        }
    }
}
pub struct Tab {
    pub id: u64,
    pub panes: Vec<Pane>,
    pub active: usize,
}
pub struct Workspace {
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub next_id: u64,
}
impl Workspace {
    pub fn new(path: PathBuf) -> Self {
        Self {
            tabs: vec![Tab {
                id: 1,
                panes: vec![Pane::new(2, path)],
                active: 0,
            }],
            active: 0,
            next_id: 3,
        }
    }
    pub fn pane(&self) -> &Pane {
        let tab = &self.tabs[self.active];
        &tab.panes[tab.active]
    }
    pub fn pane_mut(&mut self) -> &mut Pane {
        let tab = &mut self.tabs[self.active];
        &mut tab.panes[tab.active]
    }
    pub fn find_mut(&mut self, id: u64) -> Option<&mut Pane> {
        self.tabs
            .iter_mut()
            .flat_map(|t| &mut t.panes)
            .find(|p| p.id == id)
    }
    pub fn allocate(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
    pub fn snapshot(&self, config: Config) -> Session {
        Session {
            version: 1,
            config,
            active: self.active,
            tabs: self
                .tabs
                .iter()
                .map(|t| SavedTab {
                    paths: t.panes.iter().map(|p| p.path.clone()).collect(),
                    active: t.active,
                })
                .collect(),
        }
    }
    pub fn restore(session: &Session, fallback: PathBuf) -> Self {
        let mut workspace = Self {
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
        };
        for saved in session.tabs.iter().take(24) {
            let id = workspace.allocate();
            let panes = saved
                .paths
                .iter()
                .filter(|p| p.is_absolute())
                .take(2)
                .map(|path| Pane::new(workspace.allocate(), path.clone()))
                .collect::<Vec<_>>();
            if !panes.is_empty() {
                let active = saved.active.min(panes.len() - 1);
                workspace.tabs.push(Tab { id, panes, active });
            }
        }
        if workspace.tabs.is_empty() {
            return Self::new(fallback);
        }
        workspace.active = session.active.min(workspace.tabs.len() - 1);
        workspace
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub version: u32,
    pub config: Config,
    pub tabs: Vec<SavedTab>,
    pub active: usize,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedTab {
    #[serde(with = "crate::path_serde::many")]
    pub paths: Vec<PathBuf>,
    pub active: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_branches_and_bounds() {
        let mut p = Pane::new(1, PathBuf::from("/a"));
        p.navigate("/b".into());
        p.navigate("/c".into());
        assert!(p.travel(false));
        p.navigate("/d".into());
        assert!(!p.travel(true));
        assert_eq!(p.history.len(), 3);
        for i in 0..200 {
            p.navigate(PathBuf::from(format!("/{i}")));
        }
        assert_eq!(p.history.len(), 128);
        assert_eq!(p.history_index, 127);
    }
    #[test]
    fn restore_rejects_unbounded_and_relative_locations() {
        let mut session = Workspace::new(PathBuf::from("/a")).snapshot(Config::default());
        session.tabs = vec![
            SavedTab {
                paths: vec!["relative".into()],
                active: 99
            };
            100
        ];
        let w = Workspace::restore(&session, "/fallback".into());
        assert_eq!(w.tabs.len(), 1);
        assert_eq!(w.pane().path, PathBuf::from("/fallback"));
    }
    #[test]
    fn stale_selection_anchor_is_clamped_after_refresh() {
        let mut pane = Pane::new(1, "/a".into());
        pane.anchor = Some(90);
        pane.entries = vec![Entry {
            path: "/a/b".into(),
            name: "b".into(),
            kind: finder_core::EntryKind::File,
            size: 0,
            modified: None,
            hidden: false,
        }];
        pane.select(0, false, true);
        assert_eq!(pane.selected.len(), 1);
    }

    fn entry(path: &str) -> Entry {
        Entry {
            path: path.into(),
            name: path.rsplit('/').next().unwrap_or_default().into(),
            kind: finder_core::EntryKind::File,
            size: 0,
            modified: None,
            hidden: false,
        }
    }
    fn pane_of(count: usize) -> Pane {
        let mut pane = Pane::new(1, "/a".into());
        pane.entries = (0..count).map(|i| entry(&format!("/a/{i}"))).collect();
        pane
    }
    fn selected_rows(pane: &Pane) -> Vec<usize> {
        (0..pane.entries.len())
            .filter(|&i| pane.selected.contains(&pane.entries[i].path))
            .collect()
    }
    fn rows(rows: Range<usize>) -> Band {
        Band {
            rows,
            columns: 0..1,
            stride: 1,
        }
    }

    #[test]
    fn marquee_replaces_extends_or_toggles_its_starting_selection() {
        let mut pane = pane_of(8);
        pane.select(0, false, false);
        pane.begin_marquee(false, false);
        assert!(pane.selected.is_empty());
        pane.marquee_to(rows(2..5));
        assert_eq!(selected_rows(&pane), [2, 3, 4]);
        // Shrinking releases what the rectangle no longer covers.
        pane.marquee_to(rows(3..4));
        assert_eq!(selected_rows(&pane), [3]);
        pane.end_marquee();
        assert!(!pane.marquee_active());
        assert_eq!((pane.anchor, pane.focus_index), (Some(3), Some(3)));

        for (extend, toggle, covering, after) in [
            (true, false, vec![0, 2, 3, 4], vec![0, 3, 4]),
            (false, true, vec![0, 2, 4], vec![0, 4]),
        ] {
            let mut pane = pane_of(8);
            pane.select(0, false, false);
            pane.select(3, true, false);
            pane.begin_marquee(extend, toggle);
            pane.marquee_to(rows(2..5));
            assert_eq!(selected_rows(&pane), covering);
            // Leaving an entry returns it to the starting selection.
            pane.marquee_to(rows(3..5));
            assert_eq!(selected_rows(&pane), after);
            pane.marquee_to(Band::default());
            assert_eq!(selected_rows(&pane), [0, 3]);
        }
    }

    #[test]
    fn grid_marquee_covers_rows_by_columns_of_existing_entries() {
        let mut pane = pane_of(10);
        pane.begin_marquee(false, false);
        pane.marquee_to(Band {
            rows: 1..3,
            columns: 1..3,
            stride: 4,
        });
        assert_eq!(selected_rows(&pane), [5, 6, 9]);
        pane.end_marquee();
        assert_eq!(pane.focus_index, Some(5));
    }

    #[test]
    fn marquee_follows_a_refreshed_listing_and_ends_with_navigation() {
        let mut pane = pane_of(6);
        pane.select(5, false, false);
        pane.begin_marquee(true, false);
        pane.marquee_to(rows(1..3));
        assert_eq!(selected_rows(&pane), [1, 2, 5]);
        // A new file sorts first: the rectangle keeps its place on screen.
        pane.entries.insert(0, entry("/a/new"));
        pane.refresh_marquee();
        let expected: BTreeSet<PathBuf> = ["/a/0", "/a/1", "/a/5"].map(PathBuf::from).into();
        assert_eq!(pane.selected, expected);
        pane.navigate("/b".into());
        assert!(!pane.marquee_active());
        pane.marquee_to(rows(0..2));
        assert!(pane.selected.is_empty());
    }
}
