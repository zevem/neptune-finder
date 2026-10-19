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
pub mod ui;
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
