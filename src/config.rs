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

/// A keyboard step of the app zoom, as in Terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoomStep {
    In,
    Out,
    Reset,
}

/// List view column widths. Name takes the room the others leave.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ColumnWidths {
    pub date: f32,
    pub size: f32,
    pub kind: f32,
}
impl Default for ColumnWidths {
    fn default() -> Self {
        Self {
            date: 168.0,
            size: 78.0,
            kind: 150.0,
        }
    }
}
impl ColumnWidths {
    pub const DATE_RANGE: (f32, f32) = (96.0, 480.0);
    pub const SIZE_RANGE: (f32, f32) = (64.0, 240.0);
    pub const KIND_RANGE: (f32, f32) = (72.0, 480.0);
    fn validate(&mut self) {
        let fallback = Self::default();
        let (low, high) = Self::DATE_RANGE;
        self.date = bounded(self.date, low, high, fallback.date);
        let (low, high) = Self::SIZE_RANGE;
        self.size = bounded(self.size, low, high, fallback.size);
        let (low, high) = Self::KIND_RANGE;
        self.kind = bounded(self.kind, low, high, fallback.kind);
    }
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
    pub window_zoom: f32,
    pub restore_session: bool,
    #[serde(with = "crate::path_serde::many")]
    pub favorites: Vec<PathBuf>,
    pub column_widths: ColumnWidths,
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
            window_zoom: Self::WINDOW_ZOOM,
            restore_session: true,
            favorites: Vec::new(),
            column_widths: ColumnWidths::default(),
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
    pub const WINDOW_ZOOM: f32 = 1.0;
    /// Terminal's app zoom bounds.
    pub const WINDOW_ZOOM_RANGE: (f32, f32) = (0.2, 5.0);
    pub fn validate(&mut self) {
        let (low, high) = Self::SIDEBAR_RANGE;
        self.sidebar_width = bounded(self.sidebar_width, low, high, Self::SIDEBAR_WIDTH);
        let (low, high) = Self::ICON_RANGE;
        self.icon_size = bounded(self.icon_size, low, high, Self::ICON_SIZE);
        let (low, high) = Self::ROW_RANGE;
        self.row_height = bounded(self.row_height, low, high, Self::ROW_HEIGHT);
        let (low, high) = Self::WINDOW_ZOOM_RANGE;
        self.window_zoom = bounded(self.window_zoom, low, high, Self::WINDOW_ZOOM);
        self.column_widths.validate();
        self.favorites.truncate(64);
        self.favorites.retain(|path| path.is_absolute());
    }
    /// Steps the app zoom by 10% on Terminal's grid of tenths, or resets it.
    /// At either end of the range the zoom keeps its value.
    pub fn step_zoom(&mut self, step: ZoomStep) {
        let (low, high) = Self::WINDOW_ZOOM_RANGE;
        let tenths = (self.window_zoom * 10.0).round();
        self.window_zoom = match step {
            ZoomStep::In => (tenths + 1.0) / 10.0,
            ZoomStep::Out => (tenths - 1.0) / 10.0,
            ZoomStep::Reset => Self::WINDOW_ZOOM,
        }
        .clamp(low, high);
    }
}
fn bounded(value: f32, min: f32, max: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_saved_before_window_zoom_open_at_full_size() {
        let config: Config = toml::from_str("theme = \"dusk\"\nrow_height = 30.0").unwrap();
        assert_eq!(config.window_zoom, 1.0);
        let saved = toml::to_string_pretty(&Config {
            window_zoom: 1.3,
            ..Config::default()
        })
        .unwrap();
        assert_eq!(toml::from_str::<Config>(&saved).unwrap().window_zoom, 1.3);
    }
    #[test]
    fn window_zoom_is_bounded_to_terminals_range() {
        for (zoom, expected) in [
            (f32::NAN, 1.0),
            (f32::INFINITY, 1.0),
            (0.0, 0.2),
            (5.5, 5.0),
            (1.35, 1.35),
        ] {
            let mut config = Config {
                window_zoom: zoom,
                ..Config::default()
            };
            config.validate();
            assert_eq!(config.window_zoom, expected, "from {zoom}");
        }
    }
    #[test]
    fn zoom_steps_by_tenths_within_range_and_resets() {
        let mut config = Config::default();
        config.step_zoom(ZoomStep::In);
        assert_eq!(config.window_zoom, 1.1);
        config.step_zoom(ZoomStep::In);
        assert_eq!(config.window_zoom, 1.2);
        config.step_zoom(ZoomStep::Out);
        assert_eq!(config.window_zoom, 1.1);
        config.step_zoom(ZoomStep::Reset);
        assert_eq!(config.window_zoom, 1.0);
        // A hand-edited value rejoins the grid at its nearest tenth.
        config.window_zoom = 1.25;
        config.step_zoom(ZoomStep::In);
        assert_eq!(config.window_zoom, 1.4);
        for (zoom, step) in [(0.2, ZoomStep::Out), (5.0, ZoomStep::In)] {
            config.window_zoom = zoom;
            config.step_zoom(step);
            assert_eq!(config.window_zoom, zoom);
        }
    }
}
