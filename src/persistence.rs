//! One storage worker; interactive frames only replace a bounded owned snapshot.
use crate::{config::Config, model::Session};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, mpsc},
    thread,
};

pub struct Bootstrap {
    pub config: Config,
    pub session: Option<Session>,
    pub warning: Option<String>,
    pub writable: bool,
}
struct Pending {
    snapshot: Option<Session>,
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
                    while pending.snapshot.is_none() && !pending.stopping {
                        pending = signal.wait(pending).unwrap_or_else(|e| e.into_inner());
                    }
                    let snapshot = pending.snapshot.take();
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
}
