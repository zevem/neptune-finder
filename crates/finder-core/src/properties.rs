//! Properties of one item: the metadata a listing omits and, for a folder, a
//! running count of everything beneath it.
//!
//! An [`Inspector`] owns one sleeping worker with one coalescing request slot
//! and one result slot, like [`crate::Scanner`]. A folder's count publishes a
//! snapshot at most every [`PROGRESS_INTERVAL`] and then a complete one. New
//! requests and [`Inspector::cancel`] stop a count in flight. Counting never
//! follows symbolic links, stays on the folder's own volume and has explicit
//! entry and depth budgets.

use crate::{Entry, EntryKind, MAX_RECURSION_DEPTH, entry_from_metadata};
use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime};

/// Maximum entries counted beneath a folder; a larger count is a minimum.
pub const MAX_COUNTED_ENTRIES: u64 = 1_000_000;
/// The longest a folder's running count waits before it is published again.
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Debug)]
pub struct Properties {
    /// The item itself. A link describes the link rather than its target.
    pub entry: Entry,
    pub created: Option<SystemTime>,
    /// Where a symbolic link points, exactly as the link stores it.
    pub link_target: Option<PathBuf>,
    /// What opening the item reaches, so a link reports its target's.
    /// `None` when they cannot be read, as for a broken link.
    pub permissions: Option<Permissions>,
    /// Space available to the current user on the item's volume.
    pub available_space: Option<u64>,
    /// Everything beneath a folder; `None` for any other kind.
    pub contents: Option<Contents>,
}

/// Files, folders and links beneath a folder, not counting the folder itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Contents {
    pub items: u64,
    /// Total length of the regular files counted.
    pub bytes: u64,
    /// The count has finished; otherwise it is still running.
    pub complete: bool,
    /// Something could not be read, or lies deeper than the depth budget.
    pub partial: bool,
    /// Counting stopped at [`MAX_COUNTED_ENTRIES`].
    pub limited: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Permissions {
    /// Permission bits (`mode & 0o7777`) with the owning user and group.
    Unix {
        mode: u32,
        owner: String,
        group: String,
        access: Access,
    },
    /// Platforms that expose only a read-only attribute.
    ReadOnly(bool),
}

/// What the current user may do, as the operating system answers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Access {
    pub read: bool,
    pub write: bool,
    pub execute: bool,
}

#[derive(Debug)]
pub struct PropertiesResult {
    pub generation: u64,
    pub path: PathBuf,
    pub result: Result<Properties, String>,
}

struct Request {
    generation: u64,
    path: PathBuf,
}

#[derive(Default)]
struct State {
    pending: Option<Request>,
    completed: Option<PropertiesResult>,
    closed: bool,
    startup_error: Option<String>,
}

struct Shared {
    generation: AtomicU64,
    state: Mutex<State>,
    changed: Condvar,
    wake: Arc<dyn Fn() + Send + Sync>,
}

/// One latest-request-wins properties worker.
///
/// Dropping it requests cancellation without joining a filesystem call that
/// may be blocked on a slow mount.
pub struct Inspector {
    shared: Arc<Shared>,
}

impl Inspector {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        let shared = Arc::new(Shared {
            generation: AtomicU64::new(0),
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            wake: Arc::new(wake),
        });
        let worker = Arc::clone(&shared);
        if let Err(error) = std::thread::Builder::new()
            .name("neptune-finder-properties".into())
            .spawn(move || run_worker(worker))
        {
            lock_state(&shared).startup_error =
                Some(format!("Could not start the properties worker: {error}"));
        }
        Self { shared }
    }

    /// Read the properties of `path`, superseding any earlier request.
    pub fn request(&self, path: PathBuf) -> u64 {
        let mut state = lock_state(&self.shared);
        let generation = self
            .shared
            .generation
            .fetch_add(1, AtomicOrdering::AcqRel)
            .wrapping_add(1);
        if let Some(error) = state.startup_error.clone() {
            state.completed = Some(PropertiesResult {
                generation,
                path,
                result: Err(error),
            });
            drop(state);
            (self.shared.wake)();
        } else {
            state.pending = Some(Request { generation, path });
            drop(state);
            self.shared.changed.notify_one();
        }
        generation
    }

    /// Stop the current request, including a folder that is still counting.
    pub fn cancel(&self) {
        let mut state = lock_state(&self.shared);
        state.pending = None;
        self.shared.generation.fetch_add(1, AtomicOrdering::AcqRel);
    }

    /// The newest snapshot of the current request, without waiting.
    pub fn poll(&self) -> Option<PropertiesResult> {
        let mut state = lock_state(&self.shared);
        let generation = state.completed.as_ref()?.generation;
        if generation != self.shared.generation.load(AtomicOrdering::Acquire) {
            return None;
        }
        state.completed.take()
    }
}

impl Drop for Inspector {
    fn drop(&mut self) {
        let mut state = lock_state(&self.shared);
        state.closed = true;
        state.pending = None;
        self.shared.generation.fetch_add(1, AtomicOrdering::AcqRel);
        drop(state);
        self.shared.changed.notify_one();
    }
}

fn lock_state(shared: &Shared) -> MutexGuard<'_, State> {
    shared
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

fn run_worker(shared: Arc<Shared>) {
    loop {
        let request = {
            let mut state = lock_state(&shared);
            while state.pending.is_none() && !state.closed {
                state = shared
                    .changed
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner());
            }
            if state.closed {
                let completed = state.completed.take();
                drop(state);
                drop(completed);
                return;
            }
            state.pending.take().expect("pending properties request")
        };
        let cancelled = || shared.generation.load(AtomicOrdering::Acquire) != request.generation;
        inspect(&request.path, &cancelled, PROGRESS_INTERVAL, |result| {
            let mut state = lock_state(&shared);
            if state.closed || cancelled() {
                return;
            }
            let previous = state.completed.replace(PropertiesResult {
                generation: request.generation,
                path: request.path.clone(),
                result,
            });
            drop(state);
            drop(previous);
            (shared.wake)();
        });
    }
}

/// Publish an item's metadata at once, then a folder's count as it runs and
/// when it finishes. A cancelled count publishes nothing further.
fn inspect(
    path: &Path,
    cancelled: &impl Fn() -> bool,
    interval: Duration,
    mut publish: impl FnMut(Result<Properties, String>),
) {
    if cancelled() {
        return;
    }
    let mut properties = match read_properties(path) {
        Ok(properties) => properties,
        Err(error) => return publish(Err(error)),
    };
    if properties.contents.is_none() {
        return publish(Ok(properties));
    }
    publish(Ok(properties.clone()));
    let mut due = Instant::now() + interval;
    let counted = count(path, cancelled, CountBudget::PRODUCTION, |running| {
        if Instant::now() >= due {
            due = Instant::now() + interval;
            properties.contents = Some(running);
            publish(Ok(properties.clone()));
        }
    });
    if let Some(contents) = counted {
        properties.contents = Some(contents);
        publish(Ok(properties));
    }
}

/// Read an item's own metadata without following a final symbolic link.
fn read_properties(path: &Path) -> Result<Properties, String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("Could not read this item: {error}"))?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.display().to_string());
    let entry = entry_from_metadata(path.to_path_buf(), name, &metadata);
    let link = metadata.file_type().is_symlink();
    let permissions = if link {
        fs::metadata(path)
            .ok()
            .map(|target| permissions(path, &target))
    } else {
        Some(permissions(path, &metadata))
    };
    // A folder answers for its own volume; anything else for its parent's.
    let volume = if entry.kind == EntryKind::Directory {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    Ok(Properties {
        created: metadata.created().ok(),
        link_target: link.then(|| fs::read_link(path).ok()).flatten(),
        permissions,
        available_space: platform::available_space(volume),
        contents: (entry.kind == EntryKind::Directory).then(Contents::default),
        entry,
    })
}

#[derive(Clone, Copy)]
struct CountBudget {
    entries: u64,
    depth: usize,
}

impl CountBudget {
    const PRODUCTION: Self = Self {
        entries: MAX_COUNTED_ENTRIES,
        depth: MAX_RECURSION_DEPTH,
    };
}

/// Count everything beneath `root`, reporting the running total after each
/// entry. Returns `None` when cancelled.
fn count(
    root: &Path,
    cancelled: &impl Fn() -> bool,
    budget: CountBudget,
    mut progress: impl FnMut(Contents),
) -> Option<Contents> {
    let mut contents = Contents::default();
    let root_volume = fs::symlink_metadata(root)
        .ok()
        .and_then(|metadata| volume(&metadata));
    let mut directories = match fs::read_dir(root) {
        Ok(children) => vec![(children, 0_usize)],
        Err(_) => {
            contents.partial = true;
            Vec::new()
        }
    };
    // An iterator stack bounds open directory handles to the depth budget.
    while !directories.is_empty() {
        if cancelled() {
            return None;
        }
        let (directory, depth) = directories.last_mut().expect("nonempty count stack");
        let depth = *depth;
        let next = match directory.next() {
            Some(Ok(entry)) => entry,
            Some(Err(_)) => {
                contents.partial = true;
                continue;
            }
            None => {
                directories.pop();
                continue;
            }
        };
        if contents.items == budget.entries {
            contents.limited = true;
            break;
        }
        // DirEntry::metadata does not follow symbolic links, so a link is
        // counted as itself and never entered.
        let Ok(metadata) = next.metadata() else {
            contents.partial = true;
            continue;
        };
        contents.items += 1;
        if metadata.is_file() {
            contents.bytes = contents.bytes.saturating_add(metadata.len());
        } else if metadata.is_dir() && volume(&metadata) == root_volume {
            // A folder mounted from another volume is counted, not entered.
            if depth >= budget.depth {
                contents.partial = true;
            } else {
                match fs::read_dir(next.path()) {
                    Ok(children) => directories.push((children, depth + 1)),
                    Err(_) => contents.partial = true,
                }
            }
        }
        progress(contents);
    }
    if cancelled() {
        return None;
    }
    contents.complete = true;
    Some(contents)
}

#[cfg(unix)]
fn volume(metadata: &Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(metadata.dev())
}

/// Mounted folders elsewhere are reparse points, which are never followed.
#[cfg(not(unix))]
fn volume(_: &Metadata) -> Option<u64> {
    None
}

#[cfg(unix)]
fn permissions(path: &Path, metadata: &Metadata) -> Permissions {
    use std::os::unix::fs::MetadataExt;
    Permissions::Unix {
        mode: metadata.mode() & 0o7777,
        owner: platform::user_name(metadata.uid()).unwrap_or_else(|| metadata.uid().to_string()),
        group: platform::group_name(metadata.gid()).unwrap_or_else(|| metadata.gid().to_string()),
        access: platform::access(path),
    }
}

#[cfg(not(unix))]
fn permissions(_: &Path, metadata: &Metadata) -> Permissions {
    Permissions::ReadOnly(metadata.permissions().readonly())
}

#[cfg(unix)]
mod platform {
    use super::Access;
    use std::ffi::{CStr, CString, c_char};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    /// The largest buffer offered to the user and group databases.
    const MAX_RECORD_BUFFER: usize = 64 * 1024;

    fn native(path: &Path) -> Option<CString> {
        CString::new(path.as_os_str().as_bytes()).ok()
    }

    /// The operating system's answer for the current user, which includes
    /// ACLs, read-only mounts and administrator rights.
    pub(super) fn access(path: &Path) -> Access {
        let Some(path) = native(path) else {
            return Access::default();
        };
        // SAFETY: `path` is a live NUL-terminated string for each call.
        let allowed = |mode| unsafe { libc::access(path.as_ptr(), mode) } == 0;
        Access {
            read: allowed(libc::R_OK),
            write: allowed(libc::W_OK),
            execute: allowed(libc::X_OK),
        }
    }

    /// Calls a reentrant user/group lookup, growing its buffer on ERANGE,
    /// and returns the record's name.
    fn lookup<R>(
        call: impl Fn(&mut R, &mut [c_char], &mut *mut R) -> i32,
        name: impl Fn(&R) -> *const c_char,
    ) -> Option<String> {
        let mut buffer: Vec<c_char> = vec![0; 1024];
        loop {
            // SAFETY: Both records consist of integers and pointers, for which
            // all-zero bytes are valid; the lookup overwrites them.
            let mut record: R = unsafe { std::mem::zeroed() };
            let mut found = std::ptr::null_mut();
            let status = call(&mut record, &mut buffer, &mut found);
            if status == libc::ERANGE && buffer.len() < MAX_RECORD_BUFFER {
                buffer.resize(buffer.len() * 2, 0);
                continue;
            }
            let name = name(&record);
            if status != 0 || found.is_null() || name.is_null() {
                return None;
            }
            // SAFETY: On success the name points to a NUL-terminated string
            // inside `buffer`, which outlives this borrow.
            return Some(
                unsafe { CStr::from_ptr(name) }
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }

    pub(super) fn user_name(uid: libc::uid_t) -> Option<String> {
        lookup(
            |record: &mut libc::passwd, buffer, found| {
                // SAFETY: Every pointer refers to live storage of the stated size.
                unsafe { libc::getpwuid_r(uid, record, buffer.as_mut_ptr(), buffer.len(), found) }
            },
            |record| record.pw_name.cast_const(),
        )
    }

    pub(super) fn group_name(gid: libc::gid_t) -> Option<String> {
        lookup(
            |record: &mut libc::group, buffer, found| {
                // SAFETY: Every pointer refers to live storage of the stated size.
                unsafe { libc::getgrgid_r(gid, record, buffer.as_mut_ptr(), buffer.len(), found) }
            },
            |record| record.gr_name.cast_const(),
        )
    }

    #[cfg(not(target_vendor = "apple"))]
    #[allow(clippy::unnecessary_cast)] // Field widths differ between platforms.
    pub(super) fn available_space(path: &Path) -> Option<u64> {
        let path = native(path)?;
        // SAFETY: statvfs is plain integers; the call overwrites it.
        let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: `path` is NUL-terminated and `stats` is writable.
        (unsafe { libc::statvfs(path.as_ptr(), &mut stats) } == 0)
            .then(|| (stats.f_bavail as u64).saturating_mul(stats.f_frsize as u64))
    }

    /// Apple's statvfs block counts are 32-bit; statfs reports them in full.
    #[cfg(target_vendor = "apple")]
    pub(super) fn available_space(path: &Path) -> Option<u64> {
        let path = native(path)?;
        // SAFETY: statfs is integers and byte arrays; the call overwrites it.
        let mut stats: libc::statfs = unsafe { std::mem::zeroed() };
        // SAFETY: `path` is NUL-terminated and `stats` is writable.
        (unsafe { libc::statfs(path.as_ptr(), &mut stats) } == 0)
            .then(|| stats.f_bavail.saturating_mul(u64::from(stats.f_bsize)))
    }
}

#[cfg(windows)]
mod platform {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    pub(super) fn available_space(path: &Path) -> Option<u64> {
        #[link(name = "Kernel32")]
        unsafe extern "system" {
            fn GetDiskFreeSpaceExW(
                directory: *const u16,
                available: *mut u64,
                total: *mut u64,
                free: *mut u64,
            ) -> i32;
        }
        let path: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut available = 0_u64;
        // SAFETY: `path` is a live NUL-terminated UTF-16 string, `available`
        // is writable, and the total/free outputs are optional.
        let result = unsafe {
            GetDiskFreeSpaceExW(
                path.as_ptr(),
                &mut available,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        (result != 0).then_some(available)
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    pub(super) fn available_space(_: &std::path::Path) -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let unique = TEMP_COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
            let epoch = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "neptune-finder-properties-{}-{epoch}-{unique}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(fs::canonicalize(path).unwrap())
        }

        fn file(&self, name: &str, bytes: usize) -> PathBuf {
            let path = self.0.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, vec![b'n'; bytes]).unwrap();
            path
        }

        fn directory(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            fs::create_dir_all(&path).unwrap();
            path
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            #[cfg(unix)]
            for entry in walk(&self.0) {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(entry, fs::Permissions::from_mode(0o755));
            }
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    fn walk(root: &Path) -> Vec<PathBuf> {
        let mut found = vec![root.to_path_buf()];
        let mut index = 0;
        while index < found.len() {
            if let Ok(children) = fs::read_dir(&found[index]) {
                found.extend(children.flatten().map(|child| child.path()).filter(|path| {
                    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir())
                }));
            }
            index += 1;
        }
        found
    }

    fn counted(root: &Path, budget: CountBudget) -> Contents {
        count(root, &|| false, budget, |_| {}).expect("uncancelled count")
    }

    #[test]
    fn a_file_reports_its_kind_size_dates_and_permissions() {
        let fixture = Fixture::new();
        let path = fixture.file("notes.txt", 5);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        }
        let properties = read_properties(&path).unwrap();
        assert_eq!(properties.entry.path, path);
        assert_eq!(properties.entry.name, "notes.txt");
        assert_eq!(properties.entry.kind, EntryKind::File);
        assert_eq!(properties.entry.size, 5);
        assert!(properties.entry.modified.is_some());
        assert!(properties.contents.is_none());
        assert!(properties.link_target.is_none());
        #[cfg(unix)]
        {
            let Some(Permissions::Unix {
                mode,
                owner,
                group,
                access,
            }) = properties.permissions
            else {
                panic!("expected POSIX permissions");
            };
            assert_eq!(mode, 0o640);
            assert!(!owner.is_empty() && !group.is_empty());
            assert!(access.read && !access.execute);
            assert!(properties.available_space.is_some());
        }
        #[cfg(windows)]
        assert_eq!(properties.permissions, Some(Permissions::ReadOnly(false)));
    }

    #[test]
    fn a_folder_counts_every_nested_item_and_regular_file_bytes() {
        let fixture = Fixture::new();
        fixture.file("a.txt", 3);
        fixture.file("nested/b.txt", 4);
        fixture.file("nested/deeper/c.txt", 5);
        fixture.directory("empty");
        let properties = read_properties(&fixture.0).unwrap();
        assert_eq!(properties.entry.kind, EntryKind::Directory);
        assert_eq!(properties.contents, Some(Contents::default()));
        assert_eq!(
            counted(&fixture.0, CountBudget::PRODUCTION),
            Contents {
                items: 6,
                bytes: 12,
                complete: true,
                partial: false,
                limited: false,
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn links_are_counted_as_themselves_and_never_entered() {
        let fixture = Fixture::new();
        let outside = Fixture::new();
        let target = outside.file("large.bin", 4_096);
        let folder = fixture.directory("folder");
        fixture.file("folder/own.txt", 2);
        std::os::unix::fs::symlink(&outside.0, folder.join("elsewhere")).unwrap();
        std::os::unix::fs::symlink(&fixture.0, folder.join("cycle")).unwrap();
        let link = folder.join("large-link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let contents = counted(&folder, CountBudget::PRODUCTION);
        assert_eq!((contents.items, contents.bytes), (4, 2));
        assert!(contents.complete && !contents.partial);

        let properties = read_properties(&link).unwrap();
        assert_eq!(properties.entry.kind, EntryKind::Symlink);
        assert_eq!(properties.link_target, Some(target));
        assert!(properties.contents.is_none());
        assert!(matches!(
            properties.permissions,
            Some(Permissions::Unix { access, .. }) if access.read
        ));
        let directory_link = read_properties(&folder.join("elsewhere")).unwrap();
        assert_eq!(directory_link.entry.kind, EntryKind::DirectoryLink);
        assert!(
            directory_link.contents.is_none(),
            "a folder link was counted"
        );
    }

    #[test]
    fn budgets_report_a_minimum_and_an_incomplete_depth() {
        let fixture = Fixture::new();
        for name in ["one", "two", "three"] {
            fixture.file(name, 1);
        }
        fixture.file("nested/inside.txt", 1);
        let limited = counted(
            &fixture.0,
            CountBudget {
                entries: 2,
                depth: MAX_RECURSION_DEPTH,
            },
        );
        assert_eq!(limited.items, 2);
        assert!(limited.limited && limited.complete);
        let exact = counted(
            &fixture.0,
            CountBudget {
                entries: 5,
                depth: MAX_RECURSION_DEPTH,
            },
        );
        assert_eq!(exact.items, 5);
        assert!(
            !exact.limited,
            "an exactly full budget was reported as limited"
        );
        let shallow = counted(
            &fixture.0,
            CountBudget {
                entries: MAX_COUNTED_ENTRIES,
                depth: 0,
            },
        );
        assert_eq!((shallow.items, shallow.bytes), (4, 3));
        assert!(shallow.partial && !shallow.limited);
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_folder_makes_the_count_partial() {
        use std::os::unix::fs::PermissionsExt;
        // SAFETY: geteuid has no preconditions.
        if unsafe { libc::geteuid() } == 0 {
            return; // An administrator can read every folder.
        }
        let fixture = Fixture::new();
        fixture.file("locked/secret.txt", 9);
        fixture.file("open.txt", 1);
        fs::set_permissions(fixture.0.join("locked"), fs::Permissions::from_mode(0o000)).unwrap();
        let contents = counted(&fixture.0, CountBudget::PRODUCTION);
        assert_eq!((contents.items, contents.bytes), (2, 1));
        assert!(contents.partial && contents.complete);
        let Some(Permissions::Unix { access, .. }) = read_properties(&fixture.0.join("locked"))
            .unwrap()
            .permissions
        else {
            panic!("expected POSIX permissions");
        };
        assert_eq!(access, Access::default());
    }

    #[test]
    fn a_missing_item_is_an_error_without_its_path() {
        let fixture = Fixture::new();
        let missing = fixture.0.join("missing-item");
        let error = read_properties(&missing).unwrap_err();
        assert!(!error.contains("missing-item"));
    }

    #[test]
    fn a_folder_publishes_its_metadata_then_a_running_and_complete_count() {
        let fixture = Fixture::new();
        for index in 0..20 {
            fixture.file(&format!("file-{index}"), 1);
        }
        let mut published = Vec::new();
        inspect(&fixture.0, &|| false, Duration::ZERO, |result| {
            published.push(result.unwrap().contents.unwrap());
        });
        assert_eq!(published.first(), Some(&Contents::default()));
        assert!(published.len() > 2, "no running count was published");
        assert!(published[..published.len() - 1].iter().all(|c| !c.complete));
        let last = published.last().unwrap();
        assert!(last.complete && last.items == 20 && last.bytes == 20);

        let mut after_cancel = 0;
        inspect(&fixture.0, &|| true, Duration::ZERO, |_| after_cancel += 1);
        assert_eq!(after_cancel, 0);
    }

    #[test]
    fn the_worker_returns_only_the_latest_request_and_stops_when_cancelled() {
        let fixture = Fixture::new();
        let first = fixture.file("first.txt", 1);
        fixture.file("folder/inside.txt", 7);
        let folder = fixture.0.join("folder");
        let inspector = Inspector::new(|| {});
        inspector.request(first);
        let generation = inspector.request(folder.clone());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(result) = inspector.poll() {
                assert_eq!(
                    result.generation, generation,
                    "a superseded request escaped"
                );
                assert_eq!(result.path, folder);
                let contents = result.result.unwrap().contents.unwrap();
                if contents.complete {
                    assert_eq!((contents.items, contents.bytes), (1, 7));
                    break;
                }
            }
            assert!(
                Instant::now() < deadline,
                "properties worker did not finish"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        inspector.request(fixture.0.clone());
        inspector.cancel();
        std::thread::sleep(Duration::from_millis(50));
        assert!(inspector.poll().is_none(), "a cancelled request published");
    }
}
