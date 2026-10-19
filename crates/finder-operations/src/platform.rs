use std::fs::{self, File, Metadata, OpenOptions};
use std::io;
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
