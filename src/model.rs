//! Presentation state. Paths keep their original OsString identity, even when
//! two names have the same lossy display representation.
use crate::config::Config;
use finder_core::{Entry, SortOrder};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::PathBuf, time::Duration};

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
}
