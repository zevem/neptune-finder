use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Graphite,
    Dusk,
    Light,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Accent {
    #[default]
    Blue,
    Indigo,
    Purple,
    Pink,
    Red,
    Orange,
    Yellow,
    Green,
    Graphite,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewMode {
    #[default]
    List,
    Grid,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub theme: Theme,
    pub accent: Accent,
    pub view: ViewMode,
    pub show_hidden: bool,
    pub sidebar: bool,
    pub sidebar_width: f32,
    pub icon_size: f32,
    pub row_height: f32,
    pub restore_session: bool,
    #[serde(with = "crate::path_serde::many")]
    pub favorites: Vec<PathBuf>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            theme: Theme::Graphite,
            accent: Accent::Blue,
            view: ViewMode::List,
            show_hidden: false,
            sidebar: true,
            sidebar_width: Self::SIDEBAR_WIDTH,
            icon_size: Self::ICON_SIZE,
            row_height: Self::ROW_HEIGHT,
            restore_session: true,
            favorites: Vec::new(),
        }
    }
}
impl Config {
    /// Terminal's sidebar width, so the two windows share one geometry.
    pub const SIDEBAR_WIDTH: f32 = 216.0;
    pub const SIDEBAR_RANGE: (f32, f32) = (170.0, 360.0);
    pub const ICON_SIZE: f32 = 56.0;
    pub const ICON_RANGE: (f32, f32) = (32.0, 96.0);
    pub const ROW_HEIGHT: f32 = 28.0;
    pub const ROW_RANGE: (f32, f32) = (24.0, 44.0);
    pub fn validate(&mut self) {
        let (low, high) = Self::SIDEBAR_RANGE;
        self.sidebar_width = bounded(self.sidebar_width, low, high, Self::SIDEBAR_WIDTH);
        let (low, high) = Self::ICON_RANGE;
        self.icon_size = bounded(self.icon_size, low, high, Self::ICON_SIZE);
        let (low, high) = Self::ROW_RANGE;
        self.row_height = bounded(self.row_height, low, high, Self::ROW_HEIGHT);
        self.favorites.truncate(64);
        self.favorites.retain(|path| path.is_absolute());
    }
}
fn bounded(value: f32, min: f32, max: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}
