//! Filesystem work for Neptune Finder, independent of its native renderer.
//!
//! A [`Scanner`] owns one sleeping worker, one pending request, one result and
//! one retirement slot for disposing of an old displayed snapshot off the UI.
//! New requests replace queued work and cancel in-flight enumeration. All I/O,
//! filtering and sorting happens on the worker; the UI only exchanges owned
//! snapshots. Recursive scans never follow symbolic links and have explicit
//! result, traversal and depth budgets. An [`Inspector`] reads one item's
//! properties the same way, counting a folder's contents within budgets.
//!
//! Enumeration is a read-only view of a changing filesystem, not a symlink
//! security boundary: an external process can replace an inspected directory
//! between its metadata lookup and opening its children. Mutating operations
//! must validate their targets independently.

mod properties;

pub use properties::{
    Access, Contents, Inspector, MAX_COUNTED_ENTRIES, PROGRESS_INTERVAL, Permissions, Properties,
    PropertiesResult,
};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fs::{self, DirEntry, Metadata};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime};

/// Maximum entries retained by one listing, including a recursive search.
pub const MAX_LISTING_ENTRIES: usize = 100_000;
/// Maximum directory entries inspected by a recursive search.
pub const MAX_VISITED_ENTRIES: usize = 250_000;
/// Maximum nested directory depth, with the requested directory at depth zero.
pub const MAX_RECURSION_DEPTH: usize = 64;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SortField {
    #[default]
    Name,
    Modified,
    Size,
    Kind,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SortOrder {
    pub field: SortField,
    pub descending: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Directory,
    /// An explicitly navigable directory symlink/junction, never recursive.
    DirectoryLink,
    File,
    Symlink,
    Other,
}

#[derive(Clone, Debug)]
pub struct Entry {
    /// The exact native path; lossy display names are never used for I/O.
    pub path: PathBuf,
    pub name: String,
    pub kind: EntryKind,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub hidden: bool,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        matches!(self.kind, EntryKind::Directory | EntryKind::DirectoryLink)
    }

    pub fn kind_label(&self) -> &str {
        match self.kind {
            EntryKind::Directory => "Folder",
            EntryKind::DirectoryLink => "Folder link",
            EntryKind::File => "File",
            EntryKind::Symlink => "Symbolic link",
            EntryKind::Other => "Other",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Listing {
    pub path: PathBuf,
    pub entries: Vec<Entry>,
    pub elapsed: Duration,
    /// A traversal budget, inaccessible child or recursion depth omitted data.
    pub truncated: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ScanOptions {
    pub show_hidden: bool,
    pub query: String,
    pub recursive: bool,
    pub sort: SortOrder,
}

#[derive(Debug)]
pub struct ScanResult {
    pub generation: u64,
    pub result: Result<Listing, String>,
}

struct Request {
    generation: u64,
    path: PathBuf,
    options: ScanOptions,
}

#[derive(Default)]
struct WorkerState {
    pending: Option<Request>,
    completed: Option<ScanResult>,
    retired: Option<Vec<Entry>>,
    closed: bool,
    startup_error: Option<String>,
}

struct Shared {
    generation: AtomicU64,
    state: Mutex<WorkerState>,
    changed: Condvar,
    wake: Arc<dyn Fn() + Send + Sync>,
}

/// One bounded, latest-request-wins filesystem worker.
///
/// Dropping the scanner requests cancellation without joining a potentially
/// blocked filesystem syscall on the UI thread. The worker exits when the
/// current syscall returns. It owns no application state or native window.
pub struct Scanner {
    shared: Arc<Shared>,
}

impl Scanner {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        let shared = Arc::new(Shared {
            generation: AtomicU64::new(0),
            state: Mutex::new(WorkerState::default()),
            changed: Condvar::new(),
            wake: Arc::new(wake),
        });
        let worker = Arc::clone(&shared);
        if let Err(error) = std::thread::Builder::new()
            .name("neptune-finder-scan".into())
            .spawn(move || run_worker(worker))
        {
            lock_state(&shared).startup_error =
                Some(format!("Could not start the filesystem worker: {error}"));
        }
        Self { shared }
    }

    /// Queue a new listing and return its identity. Existing work is superseded.
    pub fn request(&self, path: PathBuf, options: ScanOptions) -> u64 {
        let mut state = lock_state(&self.shared);
        let generation = self
            .shared
            .generation
            .fetch_add(1, AtomicOrdering::AcqRel)
            .wrapping_add(1);
        if let Some(error) = state.startup_error.clone() {
            state.completed = Some(ScanResult {
                generation,
                result: Err(error),
            });
            drop(state);
            (self.shared.wake)();
        } else {
            state.pending = Some(Request {
                generation,
                path,
                options,
            });
            drop(state);
            self.shared.changed.notify_one();
        }
        generation
    }

    /// Take a completed current listing without waiting for filesystem work.
    pub fn poll(&self) -> Option<ScanResult> {
        let mut state = lock_state(&self.shared);
        let generation = state.completed.as_ref()?.generation;
        if generation != self.shared.generation.load(AtomicOrdering::Acquire) {
            // Retain the stale snapshot for the worker to release. Dropping a
            // large result in request/poll would add allocator work to a frame.
            return None;
        }
        state.completed.take()
    }

    /// Transfer an obsolete displayed listing for destruction on the worker.
    ///
    /// One retirement slot bounds retained memory. If it is occupied, ownership
    /// returns to the caller, which can retain the vector and retry after the
    /// worker wakes the UI. An empty vector needs no worker or queue slot.
    pub fn retire(&self, entries: Vec<Entry>) -> Result<(), Vec<Entry>> {
        if entries.is_empty() {
            return Ok(());
        }
        let mut state = lock_state(&self.shared);
        if state.closed || state.startup_error.is_some() || state.retired.is_some() {
            return Err(entries);
        }
        state.retired = Some(entries);
        drop(state);
        self.shared.changed.notify_one();
        Ok(())
    }
}

impl Drop for Scanner {
    fn drop(&mut self) {
        let mut state = lock_state(&self.shared);
        state.closed = true;
        state.pending = None;
        self.shared.generation.fetch_add(1, AtomicOrdering::AcqRel);
        drop(state);
        self.shared.changed.notify_one();
    }
}

fn lock_state(shared: &Shared) -> MutexGuard<'_, WorkerState> {
    shared
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

fn run_worker(shared: Arc<Shared>) {
    loop {
        let (retired, request) = {
            let mut state = lock_state(&shared);
            while state.pending.is_none() && state.retired.is_none() && !state.closed {
                state = shared
                    .changed
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner());
            }
            if state.closed {
                // Explicitly empty large snapshots on this thread. Relying on
                // the final Arc drop could race Scanner::drop on the UI thread.
                let retired = state.retired.take();
                let completed = state.completed.take();
                let pending = state.pending.take();
                drop(state);
                drop(retired);
                drop(completed);
                drop(pending);
                return;
            }
            (state.retired.take(), state.pending.take())
        };
        if let Some(retired) = retired {
            drop(retired);
            // A caller that received backpressure may now retry its one owned
            // deferred snapshot. The callback never runs under the queue lock.
            (shared.wake)();
        }
        let Some(request) = request else {
            continue;
        };
        let cancelled = || shared.generation.load(AtomicOrdering::Acquire) != request.generation;
        let Some(result) = scan_with_cancellation(&request.path, &request.options, cancelled)
        else {
            continue;
        };
        let mut state = lock_state(&shared);
        if state.closed || cancelled() {
            continue;
        }
        let previous = state.completed.replace(ScanResult {
            generation: request.generation,
            result,
        });
        drop(state);
        // Release superseded snapshots off the UI and outside the queue lock.
        drop(previous);
        (shared.wake)();
    }
}

/// Read, filter and sort a directory on the calling thread.
///
/// Use this entry point for command-line tools and performance measurement.
/// Interactive applications should use [`Scanner`] to keep I/O off the UI.
pub fn scan_directory(path: &Path, options: &ScanOptions) -> Result<Listing, String> {
    scan_with_cancellation(path, options, || false).expect("uncancellable directory scan")
}

fn scan_with_cancellation(
    path: &Path,
    options: &ScanOptions,
    cancelled: impl Fn() -> bool,
) -> Option<Result<Listing, String>> {
    scan_with_budget(path, options, cancelled, ScanBudget::PRODUCTION)
}

#[derive(Clone, Copy)]
struct ScanBudget {
    results: usize,
    visited: usize,
    depth: usize,
}

impl ScanBudget {
    const PRODUCTION: Self = Self {
        results: MAX_LISTING_ENTRIES,
        visited: MAX_VISITED_ENTRIES,
        depth: MAX_RECURSION_DEPTH,
    };
}

fn scan_with_budget(
    path: &Path,
    options: &ScanOptions,
    cancelled: impl Fn() -> bool,
    budget: ScanBudget,
) -> Option<Result<Listing, String>> {
    let started = Instant::now();
    if cancelled() {
        return None;
    }
    // Resolve an explicitly opened directory alias once, on the worker. All
    // returned targets then use a real root path, including /tmp aliases on
    // macOS, so mutation workers can reject symlink ancestors consistently.
    let canonical = match fs::canonicalize(path) {
        Ok(path) => path,
        Err(error) => return Some(Err(format!("Could not open {}: {error}", path.display()))),
    };
    if cancelled() {
        return None;
    }
    let path = canonical.as_path();
    let root = match fs::read_dir(path) {
        Ok(root) => root,
        Err(error) => return Some(Err(format!("Could not open {}: {error}", path.display()))),
    };
    let query = options.query.to_lowercase();
    let mut entries = Vec::with_capacity(256);
    // An iterator stack bounds open directory handles and pending traversal to
    // the depth budget, even for a tree with millions of child directories.
    let mut directories = vec![(root, 0_usize)];
    let mut visited = 0_usize;
    let mut truncated = false;

    while !directories.is_empty() {
        if cancelled() {
            return None;
        }
        let (directory, depth) = directories.last_mut().expect("nonempty traversal stack");
        let depth = *depth;
        let next = match directory.next() {
            Some(Ok(entry)) => entry,
            Some(Err(error)) => {
                if depth == 0 {
                    return Some(Err(format!(
                        "Could not read an entry in {}: {error}",
                        path.display()
                    )));
                }
                truncated = true;
                continue;
            }
            None => {
                directories.pop();
                continue;
            }
        };
        visited += 1;
        if visited > budget.visited {
            truncated = true;
            break;
        }
        let name = next.file_name().to_string_lossy().into_owned();
        // Dotfiles need no metadata syscall when they cannot be returned or
        // traversed. Likewise, nonmatching files need no metadata during a
        // search. DirEntry::file_type uses the enumeration's type information
        // where the OS provides it; it does not follow directory symlinks.
        if !options.show_hidden && name.starts_with('.') {
            continue;
        }
        let matches = query.is_empty() || name.to_lowercase().contains(&query);
        if !matches {
            if !options.recursive {
                continue;
            }
            match next.file_type() {
                Ok(kind) if kind.is_dir() => {}
                Ok(_) => continue,
                Err(_) => {
                    truncated = true;
                    continue;
                }
            }
        }
        let entry = match entry_from_directory(next, name) {
            Ok(entry) => entry,
            // A disappearing or inaccessible file should not discard an
            // otherwise usable directory. The listing exposes incompleteness.
            Err(_) => {
                truncated = true;
                continue;
            }
        };
        if !options.show_hidden && entry.hidden {
            continue;
        }
        // Directory links are navigable when explicitly opened, but must never
        // be followed by a recursive search (including cycles and mount aliases).
        if options.recursive && entry.kind == EntryKind::Directory {
            if depth >= budget.depth {
                truncated = true;
            } else {
                match fs::read_dir(&entry.path) {
                    Ok(children) => directories.push((children, depth + 1)),
                    Err(_) => truncated = true,
                }
            }
        }
        if matches {
            if entries.len() == budget.results {
                truncated = true;
                break;
            }
            entries.push(entry);
        }
    }
    if cancelled() {
        return None;
    }
    entries.sort_unstable_by(|left, right| compare_entries(left, right, options.sort));
    if cancelled() {
        return None;
    }
    Some(Ok(Listing {
        path: path.to_path_buf(),
        entries,
        elapsed: started.elapsed(),
        truncated,
    }))
}

fn entry_from_directory(entry: DirEntry, name: String) -> std::io::Result<Entry> {
    // DirEntry::metadata does not follow symlinks. Keep the original native
    // OsString in the path, including filenames that are not valid UTF-8.
    let metadata = entry.metadata()?;
    Ok(entry_from_metadata(entry.path(), name, &metadata))
}

/// An entry from metadata that did not follow a final symbolic link.
fn entry_from_metadata(path: PathBuf, name: String, metadata: &Metadata) -> Entry {
    let file_type = metadata.file_type();
    let kind = if file_type.is_symlink() {
        if fs::metadata(&path).is_ok_and(|target| target.is_dir()) {
            EntryKind::DirectoryLink
        } else {
            EntryKind::Symlink
        }
    } else if file_type.is_dir() {
        EntryKind::Directory
    } else if file_type.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    let hidden = name.starts_with('.') || has_hidden_attribute(metadata);
    Entry {
        path,
        name,
        kind,
        size: if matches!(kind, EntryKind::Directory | EntryKind::DirectoryLink) {
            0
        } else {
            metadata.len()
        },
        modified: metadata.modified().ok(),
        hidden,
    }
}

#[cfg(windows)]
fn has_hidden_attribute(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x2 != 0
}

#[cfg(not(windows))]
fn has_hidden_attribute(_: &Metadata) -> bool {
    false
}

fn compare_entries(left: &Entry, right: &Entry, sort: SortOrder) -> Ordering {
    // Folders always come first; descending changes the requested key only.
    match right.is_dir().cmp(&left.is_dir()) {
        Ordering::Equal => {}
        different => return different,
    }
    let key = match sort.field {
        SortField::Name => natural_cmp(&left.name, &right.name),
        SortField::Modified => left.modified.cmp(&right.modified),
        SortField::Size => left.size.cmp(&right.size),
        SortField::Kind => left.kind_label().cmp(right.kind_label()),
    };
    let key = if sort.descending { key.reverse() } else { key };
    key.then_with(|| natural_cmp(&left.name, &right.name))
        .then_with(|| left.path.cmp(&right.path))
}

/// Allocation-free natural ordering of ASCII numeric runs, with a stable
/// case-insensitive ASCII comparison and exact native-name tie breaker.
fn natural_cmp(left: &str, right: &str) -> Ordering {
    let original = (left, right);
    let (left, right) = (left.as_bytes(), right.as_bytes());
    let (mut a, mut b) = (0, 0);
    while a < left.len() && b < right.len() {
        if left[a].is_ascii_digit() && right[b].is_ascii_digit() {
            let (a_start, b_start) = (a, b);
            while a < left.len() && left[a].is_ascii_digit() {
                a += 1;
            }
            while b < right.len() && right[b].is_ascii_digit() {
                b += 1;
            }
            let (mut a_significant, mut b_significant) = (a_start, b_start);
            while a_significant < a && left[a_significant] == b'0' {
                a_significant += 1;
            }
            while b_significant < b && right[b_significant] == b'0' {
                b_significant += 1;
            }
            let order = (a - a_significant)
                .cmp(&(b - b_significant))
                .then_with(|| left[a_significant..a].cmp(&right[b_significant..b]))
                .then_with(|| (a - a_start).cmp(&(b - b_start)));
            if order != Ordering::Equal {
                return order;
            }
        } else {
            let order = left[a]
                .to_ascii_lowercase()
                .cmp(&right[b].to_ascii_lowercase());
            if order != Ordering::Equal {
                return order;
            }
            a += 1;
            b += 1;
        }
    }
    (left.len() - a)
        .cmp(&(right.len() - b))
        .then_with(|| original.0.cmp(original.1))
}

#[cfg(test)]
mod tests;
