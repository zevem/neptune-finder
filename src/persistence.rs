//! One storage worker; interactive frames only replace a bounded owned snapshot.
use crate::{config::Config, model::Session};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, mpsc},
    thread,
    time::Duration,
};

const WINDOW_FILE: &str = "window.json";

/// The window's size, kept apart from settings and the session: neither
/// `--no-restore` nor a damaged session loses it, and it is read before the
/// window opens.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowState {
    pub version: u32,
    /// Logical dimensions of the last window that was not maximized.
    pub inner_size: [f32; 2],
    pub maximized: bool,
}
impl Default for WindowState {
    fn default() -> Self {
        Self {
            version: 1,
            inner_size: [1240.0, 800.0],
            maximized: false,
        }
    }
}
impl WindowState {
    /// The window's minimum size.
    pub const MIN_SIZE: [f32; 2] = [520.0, 360.0];
    pub fn valid_size(size: [f32; 2]) -> bool {
        size.iter()
            .zip(Self::MIN_SIZE)
            .all(|(side, min)| side.is_finite() && (min..=16384.0).contains(side))
    }
    /// Records the window as it is now, and whether that changed what is
    /// saved. A maximized, minimized or fullscreen window keeps the size it
    /// returns to.
    pub fn observe(
        &mut self,
        size: [f32; 2],
        maximized: Option<bool>,
        minimized: bool,
        fullscreen: bool,
    ) -> bool {
        if minimized || fullscreen {
            return false;
        }
        let Some(maximized) = maximized else {
            return false;
        };
        let previous = *self;
        self.maximized = maximized;
        if !maximized && Self::valid_size(size) {
            self.inner_size = size;
        }
        *self != previous
    }
}
#[derive(Default)]
pub struct WindowLoad {
    pub state: WindowState,
    /// Whether the saved window may be replaced this launch.
    pub writable: bool,
    pub warning: Option<String>,
}
/// Reads the saved window on its own thread, so a slow mount delays opening
/// the window by `wait` at most. A read that takes longer opens the default
/// window and leaves the saved one in place.
pub fn restore_window(root: &Path, wait: Duration) -> WindowLoad {
    let path = root.join(WINDOW_FILE);
    let (tx, rx) = mpsc::sync_channel(1);
    let started = thread::Builder::new()
        .name("finder-window-restore".into())
        .spawn(move || {
            let _ = tx.send(load_window(&path));
        });
    match started.ok().and_then(|_| rx.recv_timeout(wait).ok()) {
        Some(load) => load,
        None => WindowLoad {
            warning: Some("The saved window size could not be read in time. It has been preserved; the window size will not be saved for this launch.".into()),
            ..WindowLoad::default()
        },
    }
}
fn load_window(path: &Path) -> WindowLoad {
    let saved = match fs::File::open(path) {
        Ok(file) => {
            // A size needs a few bytes; bound reads of a damaged file.
            let mut bytes = Vec::new();
            file.take(4097)
                .read_to_end(&mut bytes)
                .ok()
                .filter(|_| bytes.len() <= 4096)
                .and_then(|_| serde_json::from_slice::<WindowState>(&bytes).ok())
                .filter(|state| state.version == 1 && WindowState::valid_size(state.inner_size))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(WindowState::default()),
        Err(_) => None,
    };
    match saved {
        Some(state) => WindowLoad {
            state,
            writable: true,
            warning: None,
        },
        None => WindowLoad {
            warning: Some("The saved window size is unreadable, damaged or from a newer version. It has been preserved; the window size will not be saved for this launch.".into()),
            ..WindowLoad::default()
        },
    }
}

pub struct Bootstrap {
    pub config: Config,
    pub session: Option<Session>,
    pub warning: Option<String>,
    pub writable: bool,
}
struct Pending {
    snapshot: Option<Session>,
    window: Option<WindowState>,
    stopping: bool,
}
pub struct Storage {
    pending: Arc<(Mutex<Pending>, Condvar)>,
    bootstrap: mpsc::Receiver<Bootstrap>,
    errors: mpsc::Receiver<String>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Storage {
    pub fn new(root: PathBuf, config_path: PathBuf, wake: impl Fn() + Send + 'static) -> Self {
        let pending = Arc::new((
            Mutex::new(Pending {
                snapshot: None,
                window: None,
                stopping: false,
            }),
            Condvar::new(),
        ));
        let work = pending.clone();
        let (boot_tx, bootstrap) = mpsc::sync_channel(1);
        let (err_tx, errors) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("finder-storage".into())
            .spawn(move || {
                let boot = load(&root, &config_path);
                let writable = boot.writable;
                let _ = boot_tx.send(boot);
                wake();
                loop {
                    let (lock, signal) = &*work;
                    let mut pending = lock.lock().unwrap_or_else(|e| e.into_inner());
                    while pending.snapshot.is_none()
                        && pending.window.is_none()
                        && !pending.stopping
                    {
                        pending = signal.wait(pending).unwrap_or_else(|e| e.into_inner());
                    }
                    let snapshot = pending.snapshot.take();
                    let window = pending.window.take();
                    let stop = pending.stopping;
                    drop(pending);
                    if writable && let Some(snapshot) = snapshot {
                        let result = (|| -> Result<(), String> {
                            let config = toml::to_string_pretty(&snapshot.config)
                                .map_err(|e| e.to_string())?;
                            atomic_write(&config_path, config.as_bytes())?;
                            let json = serde_json::to_vec(&snapshot).map_err(|e| e.to_string())?;
                            atomic_write(&root.join("session.json"), &json)
                        })();
                        if let Err(error) = result {
                            let _ = err_tx.try_send(format!("Could not save settings: {error}"));
                            wake();
                        }
                    }
                    // The application decides whether the saved window may be
                    // replaced; the session's state does not.
                    if let Some(window) = window {
                        let result = serde_json::to_vec(&window)
                            .map_err(|e| e.to_string())
                            .and_then(|json| atomic_write(&root.join(WINDOW_FILE), &json));
                        if let Err(error) = result {
                            let _ =
                                err_tx.try_send(format!("Could not save the window size: {error}"));
                            wake();
                        }
                    }
                    if stop {
                        break;
                    }
                }
            })
            .expect("start storage worker");
        Self {
            pending,
            bootstrap,
            errors,
            worker: Some(worker),
        }
    }
    pub fn poll_bootstrap(&self) -> Option<Bootstrap> {
        self.bootstrap.try_recv().ok()
    }
    pub fn poll_error(&self) -> Option<String> {
        self.errors.try_recv().ok()
    }
    pub fn save(&self, snapshot: Session) {
        let (lock, signal) = &*self.pending;
        lock.lock().unwrap_or_else(|e| e.into_inner()).snapshot = Some(snapshot);
        signal.notify_one();
    }
    pub fn save_window(&self, window: WindowState) {
        let (lock, signal) = &*self.pending;
        lock.lock().unwrap_or_else(|e| e.into_inner()).window = Some(window);
        signal.notify_one();
    }
    pub fn finish(&mut self) {
        let (lock, signal) = &*self.pending;
        lock.lock().unwrap_or_else(|e| e.into_inner()).stopping = true;
        signal.notify_one();
        if let Some(worker) = self.worker.take() {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while !worker.is_finished() && std::time::Instant::now() < deadline {
                thread::sleep(std::time::Duration::from_millis(5));
            }
            if worker.is_finished() {
                let _ = worker.join();
            } else {
                eprintln!("Settings save did not finish before the shutdown deadline");
            }
        }
    }
}
impl Drop for Storage {
    fn drop(&mut self) {
        self.finish();
    }
}
fn read_optional(path: &Path) -> Result<Option<String>, String> {
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let mut value = String::new();
    file.take(4 * 1024 * 1024 + 1)
        .read_to_string(&mut value)
        .map_err(|e| e.to_string())?;
    if value.len() > 4 * 1024 * 1024 {
        return Err("saved file exceeds 4 MiB".into());
    }
    Ok(Some(value))
}
fn load(root: &Path, config_path: &Path) -> Bootstrap {
    let mut config = Config::default();
    let mut session = None;
    let mut warnings = Vec::new();
    let mut writable = true;
    match read_optional(config_path) {
        Ok(Some(value)) => match toml::from_str::<Config>(&value) {
            Ok(value) => config = value,
            Err(e) => {
                writable = false;
                warnings.push(format!("Settings are invalid and have been preserved: {e}"));
            }
        },
        Ok(None) => {}
        Err(e) => {
            writable = false;
            warnings.push(format!("Could not read settings: {e}"));
        }
    }
    config.validate();
    match read_optional(&root.join("session.json")) {
        Ok(Some(value)) => match serde_json::from_str::<Session>(&value) {
            Ok(value) if value.version == 1 => session = Some(value),
            _ => {
                writable = false;
                warnings.push("Saved session is damaged or from a newer version. It has been preserved; saving is disabled for this launch.".into());
            }
        },
        Ok(None) => {}
        Err(e) => {
            writable = false;
            warnings.push(format!("Could not read saved session: {e}"));
        }
    }
    Bootstrap {
        config,
        session,
        warning: (!warnings.is_empty()).then(|| warnings.join("\n")),
        writable,
    }
}
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("settings path has no parent")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temp.write_all(bytes)
        .and_then(|_| temp.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    temp.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn corrupt_and_future_state_are_never_replaced() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config.toml");
        fs::write(&config, "theme = \"future\"").unwrap();
        let boot = load(root.path(), &config);
        assert!(!boot.writable);
        fs::remove_file(&config).unwrap();
        fs::write(root.path().join("session.json"), "{\"version\":42}").unwrap();
        assert!(!load(root.path(), &config).writable);
    }
    #[test]
    fn storage_flushes_last_snapshot_and_restores() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.toml");
        let mut storage = Storage::new(root.path().to_owned(), path.clone(), || {});
        let workspace = crate::model::Workspace::new(root.path().to_owned());
        let config = Config {
            show_hidden: true,
            window_zoom: 1.3,
            ..Config::default()
        };
        storage.save(workspace.snapshot(config));
        storage.finish();
        let boot = load(root.path(), &path);
        assert!(boot.writable);
        assert!(boot.config.show_hidden);
        assert_eq!(boot.config.window_zoom, 1.3);
        assert!(boot.session.is_some());
    }
    #[test]
    fn maximize_minimize_and_fullscreen_keep_the_normal_window_size() {
        let mut state = WindowState::default();
        assert!(state.observe([900.0, 640.0], Some(false), false, false));
        assert!(state.observe([1920.0, 1080.0], Some(true), false, false));
        assert!(!state.observe([0.0, 0.0], Some(false), true, false));
        assert!(!state.observe([1920.0, 1080.0], Some(false), false, true));
        assert!(!state.observe([1240.0, 800.0], None, false, false));
        assert_eq!(state.inner_size, [900.0, 640.0]);
        assert!(state.maximized);
        assert!(state.observe([900.0, 640.0], Some(false), false, false));
        assert!(!state.observe([900.0, 640.0], Some(false), false, false));
        // A size the window cannot have is never saved.
        assert!(!state.observe([100.0, 640.0], Some(false), false, false));
        assert_eq!(state.inner_size, [900.0, 640.0]);
    }
    #[test]
    fn the_window_opens_at_its_saved_size_and_a_new_one_at_the_default() {
        let root = tempfile::tempdir().unwrap();
        let wait = Duration::from_secs(5);
        let first = restore_window(root.path(), wait);
        assert_eq!(first.state, WindowState::default());
        assert!(first.writable);
        assert!(first.warning.is_none());
        let mut storage = Storage::new(
            root.path().to_owned(),
            root.path().join("config.toml"),
            || {},
        );
        let resized = WindowState {
            inner_size: [960.0, 700.0],
            maximized: true,
            ..WindowState::default()
        };
        storage.save_window(resized);
        storage.finish();
        let next = restore_window(root.path(), wait);
        assert_eq!(next.state, resized);
        assert!(next.writable);
    }
    #[test]
    fn damaged_future_and_unreadable_window_state_is_never_replaced() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(WINDOW_FILE);
        for bytes in [
            b"broken".to_vec(),
            br#"{"version":2,"inner_size":[900,640],"maximized":true}"#.to_vec(),
            br#"{"version":1,"inner_size":[0,640],"maximized":true}"#.to_vec(),
            br#"{"version":1,"inner_size":[900,640],"maximized":true,"x":1}"#.to_vec(),
            vec![b' '; 4097],
        ] {
            fs::write(&path, &bytes).unwrap();
            let load = load_window(&path);
            assert!(!load.writable);
            assert!(load.warning.is_some());
            assert_eq!(load.state, WindowState::default());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        assert!(!load_window(root.path()).writable);
    }
}
