//! Native notifications are configured on a worker; no watch setup in frames.
use notify::{RecursiveMode, Watcher};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};
pub struct Watches {
    paths: Arc<Mutex<Vec<PathBuf>>>,
    signal: mpsc::SyncSender<()>,
    changed: Arc<AtomicBool>,
}
impl Watches {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        let paths = Arc::new(Mutex::new(Vec::<PathBuf>::new()));
        let requested = paths.clone();
        let changed = Arc::new(AtomicBool::new(false));
        let dirty = changed.clone();
        let (signal, receiver) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("finder-watches".into())
            .spawn(move || {
                let mut watcher = match notify::recommended_watcher(
                    move |result: notify::Result<notify::Event>| {
                        if let Ok(event) = result
                            && !matches!(event.kind, notify::EventKind::Access(_))
                        {
                            dirty.store(true, Ordering::Release);
                            wake();
                        }
                    },
                ) {
                    Ok(value) => value,
                    Err(_) => return,
                };
                let mut watched = BTreeSet::new();
                while receiver.recv().is_ok() {
                    let next = requested
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .iter()
                        .cloned()
                        .collect::<BTreeSet<_>>();
                    for path in watched.difference(&next) {
                        let _ = watcher.unwatch(path);
                    }
                    for path in next.difference(&watched) {
                        let _ = watcher.watch(path, RecursiveMode::NonRecursive);
                    }
                    watched = next;
                }
            })
            .expect("start watch worker");
        Self {
            paths,
            signal,
            changed,
        }
    }
    pub fn set_paths(&self, paths: Vec<PathBuf>) {
        *self.paths.lock().unwrap_or_else(|e| e.into_inner()) = paths;
        let _ = self.signal.try_send(());
    }
    pub fn take_changed(&self) -> bool {
        self.changed.swap(false, Ordering::AcqRel)
    }
}
