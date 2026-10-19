use std::fs::{self, File, Metadata, OpenOptions};
use std::io;
use std::ops::RangeInclusive;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::time::Duration;

const MAX_OPENERS: usize = 32;
static ACTIVE_OPENERS: AtomicUsize = AtomicUsize::new(0);
static REAPER: OnceLock<Result<SyncSender<Child>, String>> = OnceLock::new();

/// Platform rename APIs with an explicit exclusive/no-replace flag. A
/// check-then-std::fs::rename fallback would silently overwrite a racing item.
#[cfg(target_os = "linux")]
pub(super) fn rename_noreplace(source: &Path, target: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    unsafe extern "C" {
        fn renameat2(
            old_dir: i32,
            old: *const std::ffi::c_char,
            new_dir: i32,
            new: *const std::ffi::c_char,
            flags: u32,
        ) -> i32;
    }
    let source = CString::new(source.as_os_str().as_bytes())?;
    let target = CString::new(target.as_os_str().as_bytes())?;
    // SAFETY: Both C strings remain alive for this call. AT_FDCWD (-100)
    // resolves absolute paths; RENAME_NOREPLACE (1) guarantees exclusivity.
    let result = unsafe { renameat2(-100, source.as_ptr(), -100, target.as_ptr(), 1) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_os = "macos")]
pub(super) fn rename_noreplace(source: &Path, target: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    unsafe extern "C" {
        fn renamex_np(
            old: *const std::ffi::c_char,
            new: *const std::ffi::c_char,
            flags: u32,
        ) -> i32;
    }
    let source = CString::new(source.as_os_str().as_bytes())?;
    let target = CString::new(target.as_os_str().as_bytes())?;
    // SAFETY: Both paths are live NUL-terminated strings. RENAME_EXCL (4)
    // prevents an existing destination from being replaced.
    let result = unsafe { renamex_np(source.as_ptr(), target.as_ptr(), 4) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(windows)]
pub(super) fn rename_noreplace(source: &Path, target: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    let source: Vec<_> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<_> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: Both paths are live UTF-16 strings with terminators. Flags = 0
    // permits neither replacement nor an implicit copy across volumes.
    let result = unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), 0) };
    if result != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub(super) fn rename_noreplace(_source: &Path, _target: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "This platform does not have an implemented no-replace rename",
    ))
}

pub(super) fn open_regular_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        #[cfg(target_os = "linux")]
        options.custom_flags(0x2_0000); // O_NOFOLLOW
        #[cfg(target_os = "macos")]
        options.custom_flags(0x0100); // O_NOFOLLOW
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    options.open(path)
}

#[cfg(unix)]
pub(super) fn copy_symlink(source: &Path, target: &Path, _metadata: &Metadata) -> io::Result<()> {
    std::os::unix::fs::symlink(fs::read_link(source)?, target)
}

#[cfg(windows)]
pub(super) fn copy_symlink(source: &Path, target: &Path, metadata: &Metadata) -> io::Result<()> {
    use std::os::windows::fs::FileTypeExt;
    let link = fs::read_link(source)?;
    if metadata.file_type().is_symlink_dir() {
        std::os::windows::fs::symlink_dir(link, target)
    } else {
        std::os::windows::fs::symlink_file(link, target)
    }
}

#[cfg(not(any(unix, windows)))]
pub(super) fn copy_symlink(_source: &Path, _target: &Path, _metadata: &Metadata) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Symbolic link copying is unavailable on this platform",
    ))
}

/// Create a link to `target`, which an archive names and may not exist.
#[cfg(unix)]
pub(super) fn create_symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// An archive does not say whether a link names a folder; Windows needs the
/// symbolic-link privilege or Developer Mode either way.
#[cfg(windows)]
pub(super) fn create_symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}

#[cfg(not(any(unix, windows)))]
pub(super) fn create_symlink(_target: &Path, _link: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Symbolic link creation is unavailable on this platform",
    ))
}

#[cfg(unix)]
pub(super) fn same_identity(before: &Metadata, after: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    before.dev() == after.dev() && before.ino() == after.ino()
}

#[cfg(windows)]
pub(super) fn same_identity(before: &Metadata, after: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    before.creation_time() == after.creation_time()
}

#[cfg(not(any(unix, windows)))]
pub(super) fn same_identity(_before: &Metadata, _after: &Metadata) -> bool {
    true
}

/// What still identifies an item after a rename on its own volume, as a
/// move into the Trash is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Identity(u64, u64);

#[cfg(unix)]
pub(super) fn identity(metadata: &Metadata) -> Identity {
    use std::os::unix::fs::MetadataExt;
    Identity(metadata.dev(), metadata.ino())
}

#[cfg(windows)]
pub(super) fn identity(metadata: &Metadata) -> Identity {
    use std::os::windows::fs::MetadataExt;
    Identity(metadata.creation_time(), u64::from(metadata.is_dir()))
}

#[cfg(not(any(unix, windows)))]
pub(super) fn identity(_metadata: &Metadata) -> Identity {
    Identity(0, 0)
}

/// Move an item this application put in the system Trash back to
/// `original`, which the caller has found free. `deleted` brackets when it
/// was recycled, in seconds since the Unix epoch.
#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "ios", target_os = "android"))
))]
pub(super) fn restore_trashed(
    original: &Path,
    identity: Identity,
    deleted: RangeInclusive<i64>,
) -> Result<(), String> {
    let folders = trash::os_limited::trash_folders()
        .map_err(|error| format!("Could not find the Trash: {error}"))?;
    restore_from(
        folders.iter().map(std::path::PathBuf::as_path),
        original,
        identity,
        deleted,
    )
}

/// Find an item in freedesktop.org Trash folders and put it back. The Trash
/// keeps it under its own name or a numbered one (“name.2”) beside an info
/// file naming where it came from and when. Recycling renamed it on its own
/// volume, so its identity picks it out; when that changed (a copy into the
/// home Trash, or a volume without stable identities), the info file's time
/// must fall within `deleted`.
#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "ios", target_os = "android"))
))]
pub(super) fn restore_from<'a>(
    folders: impl IntoIterator<Item = &'a Path>,
    original: &Path,
    identity: Identity,
    deleted: RangeInclusive<i64>,
) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt;
    let name = original
        .file_name()
        .ok_or("A filesystem root cannot be restored")?;
    let mut recorded = None;
    for folder in folders {
        let Ok(entries) = fs::read_dir(folder.join("files")) else {
            continue;
        };
        for entry in entries.flatten() {
            let stored = entry.file_name();
            let numbered = stored
                .as_bytes()
                .strip_prefix(name.as_bytes())
                .and_then(|rest| rest.strip_prefix(b"."));
            if stored != name
                && !numbered.is_some_and(|n| !n.is_empty() && n.iter().all(u8::is_ascii_digit))
            {
                continue;
            }
            let mut info = stored;
            info.push(".trashinfo");
            let info = folder.join("info").join(info);
            let Some((path, times)) = trash_info(&info) else {
                continue;
            };
            if path != original {
                continue;
            }
            let file = entry.path();
            if fs::symlink_metadata(&file)
                .is_ok_and(|metadata| self::identity(&metadata) == identity)
            {
                return put_back(&file, &info, original);
            }
            if recorded.is_none() && times.iter().any(|time| deleted.contains(time)) {
                recorded = Some((file, info));
            }
        }
    }
    let (file, info) =
        recorded.ok_or_else(|| format!("{} is no longer in the Trash", original.display()))?;
    put_back(&file, &info, original)
}

/// The original path and possible deletion times an info file records. Its
/// local time is ambiguous when clocks go back, so both readings are kept.
#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "ios", target_os = "android"))
))]
fn trash_info(info: &Path) -> Option<(std::path::PathBuf, Vec<i64>)> {
    use chrono::TimeZone;
    use std::io::Read;
    use std::os::unix::ffi::OsStringExt;
    let mut text = String::new();
    File::open(info)
        .ok()?
        .take(64 * 1024)
        .read_to_string(&mut text)
        .ok()?;
    let mut path = None;
    let mut times = Vec::new();
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("Path=") {
            let decoded = std::path::PathBuf::from(std::ffi::OsString::from_vec(percent_decoded(
                value.trim(),
            )));
            path = decoded.is_absolute().then_some(decoded);
        } else if let Some(value) = line.strip_prefix("DeletionDate=")
            && let Ok(local) =
                chrono::NaiveDateTime::parse_from_str(value.trim(), "%Y-%m-%dT%H:%M:%S")
        {
            let local = chrono::Local.from_local_datetime(&local);
            times.extend(
                local
                    .earliest()
                    .into_iter()
                    .chain(local.latest())
                    .map(|time| time.timestamp()),
            );
        }
    }
    Some((path?, times))
}

#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "ios", target_os = "android"))
))]
fn percent_decoded(value: &str) -> Vec<u8> {
    let bytes = value.as_bytes();
    let hex = |index: usize| {
        bytes
            .get(index)
            .and_then(|byte| char::from(*byte).to_digit(16))
    };
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match (bytes[index], hex(index + 1), hex(index + 2)) {
            (b'%', Some(high), Some(low)) => {
                decoded.push((high * 16 + low) as u8);
                index += 3;
            }
            (byte, _, _) => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    decoded
}

/// Rename a trashed item back without replacing anything, then drop its
/// info file. A leftover info file would only name an item no longer there.
#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "ios", target_os = "android"))
))]
fn put_back(file: &Path, info: &Path, original: &Path) -> Result<(), String> {
    rename_noreplace(file, original).map_err(|error| {
        if error.kind() == io::ErrorKind::CrossesDevices {
            format!(
                "{} is in a Trash on another drive; restore it from the Trash in your system file manager",
                original.display()
            )
        } else {
            format!("Could not restore {}: {error}", original.display())
        }
    })?;
    let _ = fs::remove_file(info);
    Ok(())
}

/// The Recycle Bin records when it received each item; the newest one from
/// `original` inside `deleted` is the one this application recycled.
#[cfg(windows)]
pub(super) fn restore_trashed(
    original: &Path,
    _identity: Identity,
    deleted: RangeInclusive<i64>,
) -> Result<(), String> {
    let wanted = recycled_path(original);
    let item = trash::os_limited::list()
        .map_err(|error| format!("Could not read the Recycle Bin: {error}"))?
        .into_iter()
        .filter(|item| {
            deleted.contains(&item.time_deleted) && recycled_path(&item.original_path()) == wanted
        })
        .max_by_key(|item| item.time_deleted)
        .ok_or_else(|| format!("{} is no longer in the Recycle Bin", original.display()))?;
    trash::os_limited::restore_all([item])
        .map_err(|error| format!("Could not restore {}: {error}", original.display()))
}

/// A path as the Recycle Bin records it: without a verbatim prefix, and in
/// one case, as Windows compares names.
#[cfg(windows)]
fn recycled_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    let plain = match text.strip_prefix(r"\\?\UNC\") {
        Some(share) => format!(r"\\{share}"),
        None => text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned(),
    };
    plain.trim_end_matches('\\').to_lowercase()
}

#[cfg(not(any(
    windows,
    all(
        unix,
        not(any(target_os = "macos", target_os = "ios", target_os = "android"))
    )
)))]
pub(super) fn restore_trashed(
    original: &Path,
    _identity: Identity,
    _deleted: RangeInclusive<i64>,
) -> Result<(), String> {
    Err(format!(
        "{} is in the Trash. Neptune Finder can't take items back out of this system's Trash yet; use Put Back in the Trash instead",
        original.display()
    ))
}

pub(super) fn open(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(path);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg("--").arg(path);
        command
    };
    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("explorer.exe");
        command.arg(path);
        command
    };
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    return Err(format!(
        "Opening {} is unavailable on this platform",
        path.display()
    ));
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    launch(&mut command)
}

pub(super) fn reveal(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        // xdg-open is portable across GNOME, KDE, and other Freedesktop
        // desktops. It opens the containing folder; selection is OS-specific.
        open(path.parent().unwrap_or(path))
    }
    #[cfg(target_os = "macos")]
    {
        launch(Command::new("open").arg("-R").arg("--").arg(path))
    }
    #[cfg(windows)]
    {
        let mut selection = std::ffi::OsString::from("/select,");
        selection.push(path.as_os_str());
        launch(Command::new("explorer.exe").arg(selection))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    Err(format!(
        "Revealing {} is unavailable on this platform",
        path.display()
    ))
}

/// Reap desktop launchers on one shared sleeping worker. Limit live launcher
/// processes, so rapid Open actions cannot create unbounded threads or handles.
fn launch(command: &mut Command) -> Result<(), String> {
    let reaper = REAPER
        .get_or_init(|| {
            let (sender, receiver) = mpsc::sync_channel::<Child>(MAX_OPENERS);
            std::thread::Builder::new()
                .name("finder-openers".into())
                .spawn(move || {
                    let mut children = Vec::<Child>::with_capacity(MAX_OPENERS);
                    loop {
                        let next = if children.is_empty() {
                            receiver
                                .recv()
                                .map_err(|_| mpsc::RecvTimeoutError::Disconnected)
                        } else {
                            receiver.recv_timeout(Duration::from_millis(250))
                        };
                        match next {
                            Ok(child) => children.push(child),
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        }
                        let mut index = 0;
                        while index < children.len() {
                            match children[index].try_wait() {
                                Ok(None) => index += 1,
                                Ok(Some(_)) => {
                                    // wait returns the status cached by try_wait
                                    // and makes ownership/reaping explicit.
                                    let mut child = children.swap_remove(index);
                                    let _ = child.wait();
                                    ACTIVE_OPENERS.fetch_sub(1, Ordering::Release);
                                }
                                Err(_) => {
                                    // Only terminate a launcher owned by this
                                    // worker, then reap it off the UI thread.
                                    let mut child = children.swap_remove(index);
                                    let _ = child.kill();
                                    let _ = child.wait();
                                    ACTIVE_OPENERS.fetch_sub(1, Ordering::Release);
                                }
                            }
                        }
                    }
                })
                .map_err(|error| format!("Could not start the desktop launcher worker: {error}"))?;
            Ok(sender)
        })
        .as_ref()
        .map_err(Clone::clone)?;
    ACTIVE_OPENERS
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
            (active < MAX_OPENERS).then_some(active + 1)
        })
        .map_err(|_| {
            "Too many desktop applications are still opening; try again shortly".to_owned()
        })?;
    let child = match command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            ACTIVE_OPENERS.fetch_sub(1, Ordering::Release);
            return Err(format!("Could not launch the system application: {error}"));
        }
    };
    if let Err(error) = reaper.try_send(child) {
        let mut child = match error {
            mpsc::TrySendError::Full(child) | mpsc::TrySendError::Disconnected(child) => child,
        };
        let _ = child.kill();
        let _ = child.wait();
        ACTIVE_OPENERS.fetch_sub(1, Ordering::Release);
        return Err(
            "The desktop launcher worker is unavailable; try opening the item again".into(),
        );
    }
    Ok(())
}

#[cfg(all(
    test,
    unix,
    not(any(target_os = "macos", target_os = "ios", target_os = "android"))
))]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    /// A freedesktop.org Trash folder made for the test, never the user's.
    struct Trash {
        _directory: tempfile::TempDir,
        root: std::path::PathBuf,
        trash: std::path::PathBuf,
    }

    impl Trash {
        fn new() -> Self {
            let directory = tempfile::tempdir().expect("create isolated test fixture");
            let root = fs::canonicalize(directory.path()).unwrap();
            let trash = root.join("Trash");
            fs::create_dir_all(trash.join("files")).unwrap();
            fs::create_dir_all(trash.join("info")).unwrap();
            Self {
                _directory: directory,
                root,
                trash,
            }
        }

        /// Recycle `original` as another Trash implementation would.
        fn recycle(&self, original: &Path, stored: &OsStr, date: &str) -> Identity {
            let identity = identity(&fs::symlink_metadata(original).unwrap());
            fs::rename(original, self.trash.join("files").join(stored)).unwrap();
            fs::write(
                self.info(stored),
                format!(
                    "[Trash Info]\nPath={}\nDeletionDate={date}\n",
                    encoded(original)
                ),
            )
            .unwrap();
            identity
        }

        fn info(&self, stored: &OsStr) -> std::path::PathBuf {
            let mut name = stored.to_owned();
            name.push(".trashinfo");
            self.trash.join("info").join(name)
        }

        fn restore(
            &self,
            original: &Path,
            identity: Identity,
            deleted: RangeInclusive<i64>,
        ) -> Result<(), String> {
            restore_from([self.trash.as_path()], original, identity, deleted)
        }
    }

    fn encoded(path: &Path) -> String {
        path.as_os_str()
            .as_bytes()
            .iter()
            .map(|&byte| {
                if byte.is_ascii_alphanumeric() || b"/-_.~".contains(&byte) {
                    char::from(byte).to_string()
                } else {
                    format!("%{byte:02X}")
                }
            })
            .collect()
    }

    fn now() -> i64 {
        chrono::Local::now().timestamp()
    }

    fn date() -> String {
        chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
    }

    #[test]
    fn a_trashed_item_is_found_by_identity_under_a_numbered_name() {
        let trash = Trash::new();
        // Not UTF-8, with characters the info file escapes.
        let name = OsStr::from_bytes(b"caf\xe9 50% notes.txt");
        let original = trash.root.join(name);
        fs::create_dir(trash.root.join("elsewhere")).unwrap();
        let unrelated = trash.root.join("elsewhere").join(name);
        fs::write(&unrelated, b"another item of the same name").unwrap();
        trash.recycle(&unrelated, name, &date());
        fs::write(&original, b"ours").unwrap();
        let mut stored = name.to_owned();
        stored.push(".2");
        let identity = trash.recycle(&original, &stored, "2001-01-01T00:00:00");

        trash.restore(&original, identity, 0..=0).unwrap();
        assert_eq!(fs::read(&original).unwrap(), b"ours");
        assert!(!trash.info(&stored).exists(), "its info file is removed");
        assert!(trash.trash.join("files").join(name).exists());
        assert!(
            !unrelated.exists(),
            "an item from elsewhere stays in the Trash"
        );
    }

    #[test]
    fn without_its_identity_the_recorded_time_must_match() {
        let trash = Trash::new();
        let original = trash.root.join("report.pdf");
        fs::write(&original, b"report").unwrap();
        let identity = trash.recycle(&original, OsStr::new("report.pdf"), &date());
        let changed = Identity(identity.0, identity.1.wrapping_add(1));

        let error = trash.restore(&original, changed, 0..=1).unwrap_err();
        assert!(error.contains("no longer in the Trash"), "{error}");
        assert!(!original.exists());
        trash
            .restore(&original, changed, now() - 2..=now() + 2)
            .unwrap();
        assert_eq!(fs::read(&original).unwrap(), b"report");
    }

    #[test]
    fn restoring_never_replaces_an_item_created_since() {
        let trash = Trash::new();
        let original = trash.root.join("draft");
        fs::create_dir(&original).unwrap();
        let identity = trash.recycle(&original, OsStr::new("draft"), &date());
        fs::write(&original, b"a newer item").unwrap();

        assert!(trash.restore(&original, identity, 0..=0).is_err());
        assert_eq!(fs::read(&original).unwrap(), b"a newer item");
        assert!(trash.trash.join("files/draft").is_dir());
        assert!(trash.info(OsStr::new("draft")).exists());
    }
}
