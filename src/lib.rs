pub mod app;
pub mod config;
pub mod controls;
pub mod fonts;
pub mod icons;
pub mod model;
pub mod path_serde;
pub mod persistence;
pub mod preview;
pub mod theme;
pub mod thumbnails;
pub mod ui;
pub mod undo;
pub mod watch;

use std::path::PathBuf;
#[derive(Default)]
pub struct Launch {
    pub path: Option<PathBuf>,
    pub data_root: Option<PathBuf>,
    pub config: Option<PathBuf>,
    pub theme: Option<config::Theme>,
    pub size: Option<[f32; 2]>,
    pub screenshot: Option<PathBuf>,
    pub no_restore: bool,
}
impl Launch {
    /// Where settings, the saved session and the window size are kept.
    pub fn data_root(&self) -> PathBuf {
        self.data_root.clone().unwrap_or_else(|| {
            directories::ProjectDirs::from("rs", "Neptune", "NeptuneFinder")
                .map(|d| d.config_dir().to_path_buf())
                .unwrap_or_else(|| home_dir().join(".neptune-finder"))
        })
    }
}
/// The user's home directory, or the filesystem root without one.
pub fn home_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(if cfg!(windows) { "C:\\" } else { "/" }))
}
