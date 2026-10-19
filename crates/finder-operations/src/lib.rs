//! Filesystem work and desktop handoffs, kept off the interactive frame.
//!
//! Copies are staged privately beside their destination and published with a
//! native no-replace rename. Existing items are never merged or overwritten.
//! Moves across filesystems recycle their original only after publication.

mod platform;

use std::collections::HashSet;
use std::fs::{self, File, FileTimes, Metadata, OpenOptions};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

const QUEUE_CAPACITY: usize = 32;
const MAX_SELECTION: usize = 4_096;
const MAX_COPY_DEPTH: usize = 256;

#[derive(Clone, Debug)]
pub enum Operation {
    CreateFolder {
        parent: PathBuf,
        name: String,
    },
    CreateFile {
        parent: PathBuf,
        name: String,
    },
    Rename {
        source: PathBuf,
        name: String,
    },
    Copy {
        sources: Vec<PathBuf>,
        destination: PathBuf,
    },
    Move {
        sources: Vec<PathBuf>,
        destination: PathBuf,
    },
    Trash {
        paths: Vec<PathBuf>,
    },
    Open {
        path: PathBuf,
    },
    Reveal {
        path: PathBuf,
    },
}

#[derive(Debug)]
pub struct OperationResult {
    pub id: u64,
    pub result: Result<String, String>,
}

struct Request {
    id: u64,
    operation: Operation,
}

/// A single worker with bounded input and completion queues. Submission and
/// polling never touch the filesystem or wait for a worker slot.
pub struct Operations {
    requests: mpsc::SyncSender<Request>,
    results: Mutex<mpsc::Receiver<OperationResult>>,
    next_id: AtomicU64,
    stopping: Arc<AtomicBool>,
    startup_error: Option<String>,
}

impl Operations {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        let (requests, pending) = mpsc::sync_channel::<Request>(QUEUE_CAPACITY);
        let (completed, results) = mpsc::sync_channel(QUEUE_CAPACITY);
        let stopping = Arc::new(AtomicBool::new(false));
        let worker_stopping = Arc::clone(&stopping);
        let startup_error = thread::Builder::new()
            .name("finder-operations".into())
            .spawn(move || {
                while let Ok(request) = pending.recv() {
                    if worker_stopping.load(Ordering::Acquire) {
                        break;
                    }
                    let result = std::panic::catch_unwind(|| execute(request.operation))
                        .unwrap_or_else(|_| Err("The file operation stopped unexpectedly".into()));
                    if completed
                        .send(OperationResult {
                            id: request.id,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(&wake));
                }
            })
            .err()
            .map(|error| format!("Could not start the file operation worker: {error}"));
        Self {
            requests,
            results: Mutex::new(results),
            next_id: AtomicU64::new(1),
            stopping,
            startup_error,
        }
    }

    pub fn submit(&self, operation: Operation) -> Result<u64, String> {
        if let Some(error) = &self.startup_error {
            return Err(error.clone());
        }
        validate_selection_size(&operation)?;
        let id = self
            .next_id
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| "File operation identifiers are exhausted".to_owned())?;
        self.requests
            .try_send(Request { id, operation })
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => {
                    "The file operation queue is full; try again after an operation finishes"
                        .to_owned()
                }
                mpsc::TrySendError::Disconnected(_) => {
                    "The file operation worker is unavailable".to_owned()
                }
            })?;
        Ok(id)
    }

    pub fn poll(&self) -> Option<OperationResult> {
        self.results.try_lock().ok()?.try_recv().ok()
    }
}

impl Drop for Operations {
    fn drop(&mut self) {
        // Do not block this caller on disk I/O. An in-flight operation can
        // finish while the process remains alive; queued work is discarded.
        // The application must wait for completions before process shutdown.
        self.stopping.store(true, Ordering::Release);
    }
}

pub fn execute(operation: Operation) -> Result<String, String> {
    validate_selection_size(&operation)?;
    match operation {
        Operation::CreateFolder { parent, name } => {
            let target = new_child(&parent, &name)?;
            fs::create_dir(&target).map_err(|error| describe("create folder", &target, error))?;
            Ok(format!("Created folder {name}"))
        }
        Operation::CreateFile { parent, name } => {
            let target = new_child(&parent, &name)?;
            File::create_new(&target).map_err(|error| describe("create file", &target, error))?;
            Ok(format!("Created file {name}"))
        }
        Operation::Rename { source, name } => {
            validate_name(&name)?;
            let source = checked_item(&source)?;
            let target = source
                .parent()
                .ok_or("A filesystem root cannot be renamed")?
                .join(&name);
            if source == target {
                return Ok(format!("Already named {name}"));
            }
            ensure_absent(&target)?;
            platform::rename_noreplace(&source, &target)
                .map_err(|error| describe("rename", &source, error))?;
            Ok(format!("Renamed to {name}"))
        }
        Operation::Copy {
            sources,
            destination,
        } => transfer(sources, destination, false),
        Operation::Move {
            sources,
            destination,
        } => transfer(sources, destination, true),
        Operation::Trash { paths } => {
            let paths = checked_selection(paths)?;
            let total = paths.len();
            for (completed, path) in paths.iter().enumerate() {
                if let Err(error) = trash::delete(path) {
                    return Err(format!(
                        "Moved {completed} of {total} items to Trash; could not recycle {}: {error}",
                        path.display()
                    ));
                }
            }
            Ok(format!("Moved {total} {} to Trash", item_word(total)))
        }
        Operation::Open { path } => {
            let path = checked_open_item(&path)?;
            platform::open(&path)?;
            Ok(format!("Requested opening {}", path.display()))
        }
        Operation::Reveal { path } => {
            let path = checked_open_item(&path)?;
            platform::reveal(&path)?;
            Ok(format!(
                "Revealed {} in the system file manager",
                path.display()
            ))
        }
    }
}

fn validate_selection_size(operation: &Operation) -> Result<(), String> {
    let count = match operation {
        Operation::Copy { sources, .. } | Operation::Move { sources, .. } => sources.len(),
        Operation::Trash { paths } => paths.len(),
        _ => return Ok(()),
    };
    if count == 0 {
        return Err("Select at least one item".into());
    }
    if count > MAX_SELECTION {
        return Err(format!(
            "A file operation can contain at most {MAX_SELECTION} selected items"
        ));
    }
    Ok(())
}

/// Validate one new file name using the current platform's native rules.
/// This pure check performs no filesystem I/O; the filesystem may impose
/// additional limits, such as its maximum component length.
pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\0'])
        || Path::new(name).components().count() != 1
        || !matches!(
            Path::new(name).components().next(),
            Some(Component::Normal(_))
        )
    {
        return Err("Use a single file name without path separators or a null character".into());
    }
    #[cfg(windows)]
    {
        if name.ends_with([' ', '.'])
            || name
                .chars()
                .any(|character| u32::from(character) < 32 || "\\<>:\"|?*".contains(character))
        {
            return Err(
                "Windows file names cannot contain reserved characters or end with a dot or space"
                    .into(),
            );
        }
        let stem = name
            .split('.')
            .next()
            .unwrap_or_default()
            .trim_end_matches(' ')
            .to_ascii_uppercase();
        if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || stem.strip_prefix("COM").is_some_and(is_device_number)
            || stem.strip_prefix("LPT").is_some_and(is_device_number)
        {
            return Err("That name is reserved by Windows; choose another name".into());
        }
    }
    Ok(())
}

#[cfg(windows)]
fn is_device_number(number: &str) -> bool {
    matches!(
        number,
        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
    )
}

fn absolute_path(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("An empty path is not a file or folder".into());
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("Could not read the current directory: {error}"))?
            .join(path)
    };
    if !absolute.is_absolute() {
        return Err("Drive-relative paths are ambiguous; use an absolute path".into());
    }
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::ParentDir => {
                return Err(
                    "Paths containing '..' are ambiguous; navigate to the folder first".into(),
                );
            }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    Ok(normalized)
}

/// Reject directory symlinks and junctions instead of traversing an alias into
/// a different tree. A final item may be a symlink: copying preserves the link.
fn checked_directory(path: &Path) -> Result<PathBuf, String> {
    let path = absolute_path(path)?;
    let mut ancestor = PathBuf::new();
    for component in path.components() {
        ancestor.push(component.as_os_str());
        // A Windows drive prefix alone (C: or \\?\C:) is not its root.
        // Inspecting it would resolve a drive-relative working directory or
        // reject otherwise valid canonical verbatim paths.
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        let metadata = fs::symlink_metadata(&ancestor)
            .map_err(|error| describe("inspect folder", &ancestor, error))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "Folder aliases cannot be traversed during file operations: {}. Open the real folder first",
                ancestor.display()
            ));
        }
        if !metadata.is_dir() {
            return Err(format!("{} is not a folder", ancestor.display()));
        }
    }
    fs::canonicalize(&path).map_err(|error| describe("resolve folder", &path, error))
}

fn checked_item(path: &Path) -> Result<PathBuf, String> {
    let absolute = absolute_path(path)?;
    let name = absolute
        .file_name()
        .ok_or("A filesystem root cannot be used as a file operation target")?;
    let parent = absolute
        .parent()
        .ok_or("The item has no containing folder")?;
    let path = checked_directory(parent)?.join(name);
    fs::symlink_metadata(&path).map_err(|error| describe("inspect", &path, error))?;
    Ok(path)
}

fn checked_open_item(path: &Path) -> Result<PathBuf, String> {
    let path = absolute_path(path)?;
    if path.file_name().is_none() {
        checked_directory(&path)
    } else {
        checked_item(&path)
    }
}

fn checked_selection(paths: Vec<PathBuf>) -> Result<Vec<PathBuf>, String> {
    let mut checked = Vec::with_capacity(paths.len());
    let mut unique = HashSet::with_capacity(paths.len());
    for path in paths {
        let path = checked_item(&path)?;
        if !unique.insert(path.clone()) {
            return Err(format!("{} was selected more than once", path.display()));
        }
        checked.push(path);
    }
    // Sorting avoids an O(n²) scan when a large selection contains nested
    // paths. Overlapping selections have ambiguous move/trash semantics.
    let mut ordered: Vec<_> = checked.iter().collect();
    ordered.sort();
    for pair in ordered.windows(2) {
        if pair[1].starts_with(pair[0]) {
            return Err(format!(
                "Select either {} or the items inside it, not both",
                pair[0].display()
            ));
        }
    }
    Ok(checked)
}

fn new_child(parent: &Path, name: &str) -> Result<PathBuf, String> {
    validate_name(name)?;
    let target = checked_directory(parent)?.join(name);
    ensure_absent(&target)?;
    Ok(target)
}

fn ensure_absent(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(format!(
            "{} already exists; existing items are never overwritten",
            path.display()
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(describe("inspect destination", path, error)),
    }
}

fn transfer(sources: Vec<PathBuf>, destination: PathBuf, moving: bool) -> Result<String, String> {
    let destination = checked_directory(&destination)?;
    let sources = checked_selection(sources)?;
    let mut targets = HashSet::with_capacity(sources.len());
    let mut transfers = Vec::with_capacity(sources.len());
    for source in sources {
        if destination.starts_with(&source) {
            return Err(format!(
                "A folder cannot be copied or moved into itself: {}",
                source.display()
            ));
        }
        let target = destination.join(source.file_name().ok_or("The source has no file name")?);
        ensure_absent(&target)?;
        if !targets.insert(target.clone()) {
            return Err(format!(
                "Multiple selected items would have the same destination: {}",
                target.display()
            ));
        }
        transfers.push((source, target));
    }
    let total = transfers.len();
    for (completed, (source, target)) in transfers.into_iter().enumerate() {
        let result = if moving {
            move_one(&source, &target)
        } else {
            copy_one(&source, &target)
        };
        if let Err(error) = result {
            return Err(format!(
                "{} {completed} of {total} items. {error}",
                if moving { "Moved" } else { "Copied" }
            ));
        }
    }
    Ok(format!(
        "{} {total} {}",
        if moving { "Moved" } else { "Copied" },
        item_word(total)
    ))
}

fn item_word(count: usize) -> &'static str {
    if count == 1 { "item" } else { "items" }
}

fn move_one(source: &Path, target: &Path) -> Result<(), String> {
    move_one_with(source, target, platform::rename_noreplace, |source| {
        trash::delete(source).map_err(|error| error.to_string())
    })
}

fn move_one_with(
    source: &Path,
    target: &Path,
    rename: impl FnOnce(&Path, &Path) -> io::Result<()>,
    recycle: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let original = fs::symlink_metadata(source)
        .map_err(|error| describe("inspect move source", source, error))?;
    match rename(source, target) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::CrossesDevices => {
            copy_one(source, target)?;
            let current = fs::symlink_metadata(source).map_err(|error| format!("A complete copy is available at {}, but the original could no longer be verified and was not recycled: {error}", target.display()))?;
            if !unchanged_metadata(&original, &current) {
                return Err(format!(
                    "A complete copy is available at {}, but the original changed during the transfer and was left in place. Both items remain",
                    target.display()
                ));
            }
            recycle(source).map_err(|error| format!("A complete copy is available at {}, but the original could not be moved to Trash: {error}. Both items remain; no original was permanently deleted", target.display()))
        }
        Err(error) => Err(describe("move", source, error)),
    }
}

fn copy_one(source: &Path, target: &Path) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or("The copy destination has no parent folder")?;
    let stage = tempfile::Builder::new()
        .prefix(".neptune-copy-")
        .tempdir_in(parent)
        .map_err(|error| describe("prepare copy in", parent, error))?;
    let payload = stage.path().join("item");
    copy_entry(source, &payload, 0)?;
    platform::rename_noreplace(&payload, target)
        .map_err(|error| describe("publish copy", target, error))?;
    Ok(())
}

fn copy_entry(source: &Path, target: &Path, depth: usize) -> Result<(), String> {
    if depth > MAX_COPY_DEPTH {
        return Err(format!(
            "The folder tree is deeper than the supported {MAX_COPY_DEPTH} levels: {}",
            source.display()
        ));
    }
    let metadata = fs::symlink_metadata(source)
        .map_err(|error| describe("inspect copy source", source, error))?;
    if metadata.file_type().is_symlink() {
        return platform::copy_symlink(source, target, &metadata)
            .map_err(|error| describe("copy symbolic link", source, error));
    }
    if metadata.is_dir() {
        fs::create_dir(target).map_err(|error| describe("create copied folder", target, error))?;
        for entry in
            fs::read_dir(source).map_err(|error| describe("read copied folder", source, error))?
        {
            let entry =
                entry.map_err(|error| describe("read copied folder entry", source, error))?;
            copy_entry(&entry.path(), &target.join(entry.file_name()), depth + 1)?;
        }
        let after = fs::symlink_metadata(source)
            .map_err(|error| describe("verify copied folder", source, error))?;
        ensure_unchanged(source, &metadata, &after)?;
        fs::set_permissions(target, metadata.permissions())
            .map_err(|error| describe("preserve folder permissions", target, error))?;
        return Ok(());
    }
    if !metadata.is_file() {
        return Err(format!(
            "Special files (devices, sockets, and pipes) cannot be copied: {}",
            source.display()
        ));
    }
    let mut input = platform::open_regular_file(source)
        .map_err(|error| describe("open copy source", source, error))?;
    let opened = input
        .metadata()
        .map_err(|error| describe("inspect opened copy source", source, error))?;
    ensure_unchanged(source, &metadata, &opened)?;
    if !opened.is_file() {
        return Err(format!(
            "The copy source changed its file type: {}",
            source.display()
        ));
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .map_err(|error| describe("create copied file", target, error))?;
    let copied =
        io::copy(&mut input, &mut output).map_err(|error| describe("copy file", source, error))?;
    let after = input
        .metadata()
        .map_err(|error| describe("verify copied file", source, error))?;
    ensure_unchanged(source, &opened, &after)?;
    if copied != opened.len() {
        return Err(format!(
            "The source changed length while copying: {}. Retry when writes have finished",
            source.display()
        ));
    }
    output
        .set_permissions(metadata.permissions())
        .map_err(|error| describe("preserve file permissions", target, error))?;
    let mut times = FileTimes::new();
    if let Ok(modified) = metadata.modified() {
        times = times.set_modified(modified);
    }
    if let Ok(accessed) = metadata.accessed() {
        times = times.set_accessed(accessed);
    }
    output
        .set_times(times)
        .map_err(|error| describe("preserve file timestamps", target, error))?;
    output
        .sync_all()
        .map_err(|error| describe("finish copied file", target, error))?;
    Ok(())
}

fn ensure_unchanged(path: &Path, before: &Metadata, after: &Metadata) -> Result<(), String> {
    if !unchanged_metadata(before, after) {
        return Err(format!(
            "{} changed while copying. Retry when writes have finished; the incomplete copy was discarded",
            path.display()
        ));
    }
    Ok(())
}

fn unchanged_metadata(before: &Metadata, after: &Metadata) -> bool {
    before.file_type() == after.file_type()
        && before.len() == after.len()
        && before.modified().ok() == after.modified().ok()
        && platform::same_identity(before, after)
}

fn describe(action: &str, path: &Path, error: io::Error) -> String {
    format!("Could not {action} {}: {error}", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, Instant};

    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().expect("create isolated test fixture");
        // macOS exposes its temporary root through /var -> /private/var.
        // Fixtures use the real directory, as operations require.
        let path = fs::canonicalize(directory.path()).expect("canonical test fixture");
        (directory, path)
    }

    #[test]
    fn names_are_single_native_components() {
        for invalid in ["", ".", "..", "../outside", "a/b", "a\0b"] {
            assert!(validate_name(invalid).is_err(), "accepted {invalid:?}");
        }
        for valid in [
            "notes.txt",
            "New Folder",
            ".hidden",
            "照片",
            "compass",
            "COM10",
        ] {
            assert!(validate_name(valid).is_ok(), "rejected {valid:?}");
        }
        #[cfg(windows)]
        for invalid in [
            "a\\b",
            "name.",
            "name ",
            "NUL",
            "con.txt",
            "COM1.log",
            "LPT²",
            "file:stream",
            "*",
            "line\nfeed",
        ] {
            assert!(validate_name(invalid).is_err(), "accepted {invalid:?}");
        }
        #[cfg(unix)]
        for valid in [
            "a\\b",
            "name.",
            "name ",
            "NUL",
            "con.txt",
            "COM1.log",
            "LPT²",
            "file:stream",
            "*",
            "line\nfeed",
        ] {
            assert!(validate_name(valid).is_ok(), "rejected {valid:?}");
        }
    }

    #[test]
    fn creates_files_and_folders_without_overwriting() {
        let (_fixture, root) = fixture();
        execute(Operation::CreateFolder {
            parent: root.clone(),
            name: "Documents".into(),
        })
        .unwrap();
        execute(Operation::CreateFile {
            parent: root.clone(),
            name: "notes.txt".into(),
        })
        .unwrap();
        fs::write(root.join("notes.txt"), b"keep me").unwrap();
        assert!(
            execute(Operation::CreateFile {
                parent: root.clone(),
                name: "notes.txt".into()
            })
            .is_err()
        );
        assert_eq!(fs::read(root.join("notes.txt")).unwrap(), b"keep me");
        assert!(root.join("Documents").is_dir());
        assert!(
            execute(Operation::CreateFolder {
                parent: root.clone(),
                name: "../escape".into()
            })
            .is_err()
        );
    }

    #[test]
    fn rename_and_move_preserve_existing_destinations() {
        let (_fixture, root) = fixture();
        fs::write(root.join("source"), b"source").unwrap();
        fs::write(root.join("existing"), b"existing").unwrap();
        assert!(
            execute(Operation::Rename {
                source: root.join("source"),
                name: "existing".into()
            })
            .is_err()
        );
        assert!(platform::rename_noreplace(&root.join("source"), &root.join("existing")).is_err());
        assert_eq!(fs::read(root.join("source")).unwrap(), b"source");
        assert_eq!(fs::read(root.join("existing")).unwrap(), b"existing");
        execute(Operation::Rename {
            source: root.join("source"),
            name: "renamed".into(),
        })
        .unwrap();
        fs::create_dir(root.join("destination")).unwrap();
        execute(Operation::Move {
            sources: vec![root.join("renamed")],
            destination: root.join("destination"),
        })
        .unwrap();
        assert!(!root.join("renamed").exists());
        assert_eq!(
            fs::read(root.join("destination/renamed")).unwrap(),
            b"source"
        );
    }

    #[test]
    fn complete_tree_copy_keeps_originals_and_cleans_staging() {
        let (_fixture, root) = fixture();
        fs::create_dir_all(root.join("source/nested")).unwrap();
        fs::write(root.join("source/nested/payload"), b"hello Neptune").unwrap();
        fs::create_dir(root.join("destination")).unwrap();
        execute(Operation::Copy {
            sources: vec![root.join("source")],
            destination: root.join("destination"),
        })
        .unwrap();
        assert_eq!(
            fs::read(root.join("destination/source/nested/payload")).unwrap(),
            b"hello Neptune"
        );
        assert_eq!(
            fs::read(root.join("source/nested/payload")).unwrap(),
            b"hello Neptune"
        );
        assert_eq!(fs::read_dir(root.join("destination")).unwrap().count(), 1);
        assert!(
            execute(Operation::Copy {
                sources: vec![root.join("source")],
                destination: root.join("destination")
            })
            .is_err()
        );
    }

    #[test]
    fn copy_publication_cannot_replace_an_item_created_after_preflight() {
        let (_fixture, root) = fixture();
        fs::write(root.join("source"), b"source").unwrap();
        fs::write(
            root.join("racing-destination"),
            b"created by another application",
        )
        .unwrap();
        assert!(copy_one(&root.join("source"), &root.join("racing-destination")).is_err());
        assert_eq!(
            fs::read(root.join("racing-destination")).unwrap(),
            b"created by another application"
        );
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
    }

    #[test]
    fn refuses_descendants_and_overlapping_selections_before_mutation() {
        let (_fixture, root) = fixture();
        fs::create_dir_all(root.join("source/nested")).unwrap();
        let result = execute(Operation::Copy {
            sources: vec![root.join("source")],
            destination: root.join("source/nested"),
        });
        assert!(result.is_err());
        assert!(
            fs::read_dir(root.join("source/nested"))
                .unwrap()
                .next()
                .is_none()
        );
        assert!(
            execute(Operation::Trash {
                paths: vec![root.join("source"), root.join("source/nested")]
            })
            .is_err()
        );
        assert!(root.join("source/nested").is_dir());
    }

    #[test]
    fn duplicate_destination_names_fail_before_copying_any_item() {
        let (_fixture, root) = fixture();
        for directory in ["left", "right", "destination"] {
            fs::create_dir(root.join(directory)).unwrap();
        }
        fs::write(root.join("left/item"), b"left").unwrap();
        fs::write(root.join("right/item"), b"right").unwrap();
        assert!(
            execute(Operation::Copy {
                sources: vec![root.join("left/item"), root.join("right/item")],
                destination: root.join("destination")
            })
            .is_err()
        );
        assert_eq!(fs::read_dir(root.join("destination")).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn copy_preserves_links_without_following_external_targets() {
        use std::os::unix::fs::symlink;
        let (_fixture, root) = fixture();
        fs::create_dir_all(root.join("source/nested")).unwrap();
        fs::create_dir(root.join("destination")).unwrap();
        fs::write(root.join("external"), b"external data").unwrap();
        symlink("../../external", root.join("source/nested/link")).unwrap();
        symlink("missing-target", root.join("source/dangling")).unwrap();
        execute(Operation::Copy {
            sources: vec![root.join("source")],
            destination: root.join("destination"),
        })
        .unwrap();
        assert_eq!(
            fs::read_link(root.join("destination/source/nested/link")).unwrap(),
            Path::new("../../external")
        );
        assert_eq!(
            fs::read_link(root.join("destination/source/dangling")).unwrap(),
            Path::new("missing-target")
        );
        assert_eq!(fs::read(root.join("external")).unwrap(), b"external data");
    }

    #[cfg(unix)]
    #[test]
    fn directory_aliases_cannot_bypass_descendant_checks() {
        use std::os::unix::fs::symlink;
        let (_fixture, root) = fixture();
        fs::create_dir_all(root.join("source/nested")).unwrap();
        symlink(root.join("source/nested"), root.join("alias")).unwrap();
        assert!(
            execute(Operation::Copy {
                sources: vec![root.join("source")],
                destination: root.join("alias")
            })
            .is_err()
        );
        assert!(
            execute(Operation::CreateFile {
                parent: root.join("alias"),
                name: "outside".into()
            })
            .is_err()
        );
        assert_eq!(fs::read_dir(root.join("source/nested")).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn unsupported_special_file_discards_staged_copy() {
        use std::os::unix::net::UnixListener;
        let (_fixture, root) = fixture();
        fs::create_dir(root.join("source")).unwrap();
        fs::create_dir(root.join("destination")).unwrap();
        fs::write(root.join("source/regular"), b"unchanged").unwrap();
        let _socket = UnixListener::bind(root.join("source/socket")).unwrap();
        assert!(
            execute(Operation::Copy {
                sources: vec![root.join("source")],
                destination: root.join("destination")
            })
            .is_err()
        );
        assert_eq!(fs::read_dir(root.join("destination")).unwrap().count(), 0);
        assert_eq!(fs::read(root.join("source/regular")).unwrap(), b"unchanged");
    }

    #[test]
    fn cross_device_move_recycles_only_after_complete_publication() {
        let (_fixture, root) = fixture();
        let source = root.join("source");
        let target = root.join("destination");
        let recycled = root.join("fixture-trash");
        fs::write(&source, b"recoverable").unwrap();
        move_one_with(
            &source,
            &target,
            |_, _| Err(io::ErrorKind::CrossesDevices.into()),
            |path| {
                assert_eq!(fs::read(&target).unwrap(), b"recoverable");
                fs::rename(path, &recycled).map_err(|error| error.to_string())
            },
        )
        .unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"recoverable");
        assert_eq!(fs::read(&recycled).unwrap(), b"recoverable");
        assert!(!source.exists());
    }

    #[test]
    fn cross_device_recycle_failure_keeps_both_complete_copies() {
        let (_fixture, root) = fixture();
        let source = root.join("source");
        let target = root.join("destination");
        fs::write(&source, b"recoverable").unwrap();
        let error = move_one_with(
            &source,
            &target,
            |_, _| Err(io::ErrorKind::CrossesDevices.into()),
            |_| Err("fixture recycle failure".into()),
        )
        .unwrap_err();
        assert!(error.contains("Both items remain"));
        assert_eq!(fs::read(&source).unwrap(), b"recoverable");
        assert_eq!(fs::read(&target).unwrap(), b"recoverable");
    }

    #[test]
    fn cross_device_failed_copy_does_not_recycle_original() {
        let (_fixture, root) = fixture();
        let source = root.join("source");
        let target = root.join("destination");
        fs::write(&source, b"original").unwrap();
        fs::write(&target, b"unrelated").unwrap();
        let called = AtomicBool::new(false);
        assert!(
            move_one_with(
                &source,
                &target,
                |_, _| Err(io::ErrorKind::CrossesDevices.into()),
                |_| {
                    called.store(true, Ordering::Relaxed);
                    Ok(())
                }
            )
            .is_err()
        );
        assert!(!called.load(Ordering::Relaxed));
        assert_eq!(fs::read(&source).unwrap(), b"original");
        assert_eq!(fs::read(&target).unwrap(), b"unrelated");
    }

    #[test]
    fn cross_device_changed_original_is_left_in_place() {
        let (_fixture, root) = fixture();
        let source = root.join("source");
        let target = root.join("destination");
        fs::write(&source, b"initial").unwrap();
        let called = AtomicBool::new(false);
        let error = move_one_with(
            &source,
            &target,
            |source, _| {
                fs::write(source, b"changed by a writer").unwrap();
                Err(io::ErrorKind::CrossesDevices.into())
            },
            |_| {
                called.store(true, Ordering::Relaxed);
                Ok(())
            },
        )
        .unwrap_err();
        assert!(error.contains("original changed"));
        assert!(!called.load(Ordering::Relaxed));
        assert_eq!(fs::read(&source).unwrap(), b"changed by a writer");
        assert_eq!(fs::read(&target).unwrap(), b"changed by a writer");
    }

    #[test]
    fn root_and_ambiguous_paths_are_rejected_for_mutation() {
        let (_fixture, root) = fixture();
        assert!(checked_item(root.ancestors().last().unwrap()).is_err());
        assert!(checked_open_item(root.ancestors().last().unwrap()).is_ok());
        assert!(checked_directory(&root.join("..")).is_err());
        assert!(
            execute(Operation::Copy {
                sources: vec![],
                destination: root
            })
            .is_err()
        );
    }

    #[test]
    fn worker_delivers_a_correlated_completion_and_wakes_the_ui() {
        let (_fixture, root) = fixture();
        let wakes = Arc::new(AtomicUsize::new(0));
        let worker_wakes = Arc::clone(&wakes);
        let operations = Operations::new(move || {
            worker_wakes.fetch_add(1, Ordering::Relaxed);
        });
        let id = operations
            .submit(Operation::CreateFile {
                parent: root.clone(),
                name: "background".into(),
            })
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(completion) = operations.poll() {
                assert_eq!(completion.id, id);
                assert!(completion.result.is_ok());
                break;
            }
            assert!(Instant::now() < deadline, "worker did not complete in time");
            thread::sleep(Duration::from_millis(2));
        }
        assert!(root.join("background").is_file());
        while wakes.load(Ordering::Relaxed) == 0 {
            assert!(Instant::now() < deadline, "worker did not wake the UI");
            thread::yield_now();
        }
    }

    #[test]
    fn source_mutation_detection_rejects_changed_data() {
        let (_fixture, root) = fixture();
        let path = root.join("source");
        fs::write(&path, b"before").unwrap();
        let before = fs::metadata(&path).unwrap();
        fs::write(&path, b"changed after opening").unwrap();
        let after = fs::metadata(&path).unwrap();
        assert!(ensure_unchanged(&path, &before, &after).is_err());
    }
}
