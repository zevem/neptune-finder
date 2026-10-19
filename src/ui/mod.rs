//! Native presentation; widgets emit stable-target actions for the coordinator.
mod chrome;
mod content;
mod dialogs;
#[cfg(test)]
mod tests;
use crate::{
    config::{Config, ViewMode, ZoomStep},
    icons::Icon,
    model::{Pane, Workspace},
    theme::{self, Palette},
};
use eframe::egui::{self, Key, Modifiers, Rect, Stroke, StrokeKind, Vec2};
use finder_core::{Entry, EntryKind, SortField};
use finder_operations::Operation;
use std::path::{Path, PathBuf};

pub enum Action {
    Navigate(u64, PathBuf),
    Open(u64, PathBuf, bool),
    /// Travel a pane's history: backward, or forward when `true`.
    Back(u64, bool),
    Parent,
    Reload,
    ReloadPane(u64),
    Search(u64),
    Sort(u64, SortField),
    ToggleHidden,
    ToggleRecursive,
    NewTab(PathBuf),
    SelectTab(usize),
    CloseTab(usize),
    WindowClose,
    ToggleSplit,
    FocusPane(usize),
    Copy(Vec<PathBuf>, bool),
    Paste(PathBuf),
    CopyPath(PathBuf),
    Submit(Operation),
    QuickLook(PathBuf),
    /// Show an item's properties; the pane is where the action began.
    Properties(u64, Entry),
    /// Open a pane at an item's parent folder with the item selected.
    ShowInFolder(u64, PathBuf),
    Favorite(PathBuf),
    PreferencesChanged,
    View(ViewMode),
    Location(String),
    Zoom(ZoomStep),
    /// Extract an archive into a typed folder.
    ExtractTo(PathBuf, String),
}
pub enum Dialog {
    New {
        parent: PathBuf,
        name: String,
        folder: bool,
    },
    Rename {
        source: PathBuf,
        name: String,
    },
    Trash {
        paths: Vec<PathBuf>,
    },
    Compress {
        sources: Vec<PathBuf>,
        destination: PathBuf,
        name: String,
    },
    Extract {
        archive: PathBuf,
        destination: String,
    },
    Location {
        value: String,
    },
    Preferences,
    QuickLook {
        path: PathBuf,
    },
    /// The entry known when the sheet opened, shown until its properties load.
    Properties {
        pane: u64,
        entry: Entry,
    },
    About,
}
/// A location being typed into the toolbar's path bar. It belongs to the pane
/// and folder in view when editing began, and ends if either changes.
pub struct LocationEdit {
    pub pane: u64,
    pub path: PathBuf,
    pub text: String,
    pub focus: bool,
}
pub enum Preview {
    Text(String),
    Image(egui::TextureHandle),
    Message(String),
}

/// What a message reports: something to know, or a problem. Finished work
/// shows in the files themselves and needs no message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Info,
    Problem,
}

/// A message in the window's top trailing corner. Problems stay until they
/// are dismissed; information leaves by itself after a few seconds.
pub struct Notice {
    pub text: String,
    pub tone: Tone,
    shown: Option<f64>,
}
impl Notice {
    const LINGER: f64 = 4.0;
    pub fn new(tone: Tone, text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone,
            shown: None,
        }
    }
    pub fn info(text: impl Into<String>) -> Self {
        Self::new(Tone::Info, text)
    }
    pub fn problem(text: impl Into<String>) -> Self {
        Self::new(Tone::Problem, text)
    }
}

pub struct UiState {
    pub home: PathBuf,
    pub places: Vec<(Icon, String, PathBuf)>,
    pub notice: Option<Notice>,
    /// The app zoom just stepped from the keyboard, named for a moment.
    pub level: Option<crate::controls::Level>,
    pub dialog: Option<Dialog>,
    pub preview: Option<Preview>,
    /// The newest properties read for the open Properties sheet.
    pub properties: Option<Result<finder_core::Properties, String>>,
    pub brand_icon: Option<egui::TextureHandle>,
    pub busy: usize,
    pub can_paste: bool,
    /// Items cut to the clipboard, shown faded until they are pasted or the
    /// clipboard holds something else.
    pub cut: std::collections::HashSet<PathBuf>,
    /// The paste chord held down has already pasted, so its release must not
    /// paste again.
    pub pasted: bool,
    pub focus_search: bool,
    pub focus_dialog: bool,
    pub location: Option<LocationEdit>,
    /// Whether the toolbar had room for its path bar in the last frame.
    pub path_bar: bool,
    /// The sidebar's width while its trailing edge is dragged.
    pub sidebar_drag: Option<f32>,
    /// The sidebar is open over the files of a window too narrow to keep it
    /// beside them. It is never saved; the docked preference stays as it was.
    pub sidebar_overlay: bool,
    /// The Preferences pane in view while the app runs.
    pub preferences_pane: dialogs::PreferencesPane,
    pub file_widgets: std::collections::HashSet<egui::Id>,
    pub file_owners: std::collections::HashMap<egui::Id, u64>,
    pub grid_columns: std::collections::HashMap<u64, usize>,
    pub thumbnails: crate::thumbnails::Cache,
}
impl UiState {
    pub fn new(home: PathBuf) -> Self {
        // UserDirs resolves the user's localized XDG locations once, outside frames.
        let dirs = directories::UserDirs::new();
        let directory = |f: fn(&directories::UserDirs) -> Option<&std::path::Path>,
                         fallback: &str| {
            dirs.as_ref()
                .and_then(f)
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| home.join(fallback))
        };
        let places = vec![
            (Icon::Home, "Home".into(), home.clone()),
            (
                Icon::Desktop,
                "Desktop".into(),
                directory(directories::UserDirs::desktop_dir, "Desktop"),
            ),
            (
                Icon::FileText,
                "Documents".into(),
                directory(directories::UserDirs::document_dir, "Documents"),
            ),
            (
                Icon::Download,
                "Downloads".into(),
                directory(directories::UserDirs::download_dir, "Downloads"),
            ),
            (
                Icon::Image,
                "Pictures".into(),
                directory(directories::UserDirs::picture_dir, "Pictures"),
            ),
            (
                Icon::Music,
                "Music".into(),
                directory(directories::UserDirs::audio_dir, "Music"),
            ),
            (
                Icon::Movie,
                "Videos".into(),
                directory(directories::UserDirs::video_dir, "Videos"),
            ),
        ];
        Self {
            home,
            places,
            notice: None,
            level: None,
            dialog: None,
            preview: None,
            properties: None,
            brand_icon: None,
            busy: 0,
            can_paste: false,
            cut: Default::default(),
            pasted: false,
            focus_search: false,
            focus_dialog: true,
            location: None,
            path_bar: false,
            sidebar_drag: None,
            sidebar_overlay: false,
            preferences_pane: dialogs::PreferencesPane::General,
            file_widgets: Default::default(),
            file_owners: Default::default(),
            grid_columns: Default::default(),
            thumbnails: Default::default(),
        }
    }
    pub fn open_dialog(&mut self, dialog: Dialog) {
        self.dialog = Some(dialog);
        self.focus_dialog = true;
    }
    /// Type a location for `pane`: in the toolbar's path bar when it has
    /// room, otherwise in the Go to folder dialog.
    pub fn go_to_folder(&mut self, pane: &Pane) {
        if self.path_bar {
            self.location = Some(LocationEdit {
                pane: pane.id,
                path: pane.path.clone(),
                text: pane.path.display().to_string(),
                focus: true,
            });
        } else {
            self.open_dialog(Dialog::Location {
                value: pane.path.display().to_string(),
            });
        }
    }
}

/// How long the sidebar takes to slide in or out, as in Terminal.
const SIDEBAR_SLIDE: f32 = 0.16;

pub fn show(
    ui: &mut egui::Ui,
    workspace: &mut Workspace,
    config: &mut Config,
    state: &mut UiState,
    actions: &mut Vec<Action>,
) {
    let ctx = ui.ctx().clone();
    let p = Palette::for_config(config);
    let bounds = ui.max_rect();
    // The sidebar yields to files in narrow windows, and opens over them on
    // request instead.
    let available = bounds.width() >= theme::metrics::SIDEBAR_MIN_WINDOW;
    if available || state.dialog.is_some() {
        state.sidebar_overlay = false;
    }
    // The sidebar spans the full height, like a native source list, and
    // slides when toggled. Both slides are sampled before keys are handled: a
    // toggle starts from the next frame, so the first frame after an idle
    // window cannot carry the whole slide.
    let reveal = ctx.animate_bool_with_time_and_easing(
        egui::Id::new("finder-sidebar-reveal").with(ctx.viewport_id()),
        config.sidebar && available,
        if available { SIDEBAR_SLIDE } else { 0.0 },
        egui::emath::easing::cubic_out,
    );
    let overlay = ctx.animate_bool_with_time_and_easing(
        egui::Id::new("finder-sidebar-overlay-reveal").with(ctx.viewport_id()),
        state.sidebar_overlay,
        if available { 0.0 } else { SIDEBAR_SLIDE },
        egui::emath::easing::cubic_out,
    );
    keyboard(&ctx, workspace, config, state, available, actions);
    state.file_widgets.clear();
    state.file_owners.clear();
    state.grid_columns.clear();
    let radius = window_radius(&ctx);
    // One rounded material for the whole window. Regions draw no rectangular
    // fills of their own, so the transparent corners stay clear.
    ui.painter().rect_filled(bounds, radius, p.chrome);

    let width = state
        .sidebar_drag
        .unwrap_or(config.sidebar_width)
        .clamp(chrome::SIDEBAR_WIDTH.0, chrome::SIDEBAR_WIDTH.1);
    let edge = (width * reveal).round();
    if edge > 0.0 {
        let side = Rect::from_min_size(
            egui::pos2(bounds.left() + edge - width, bounds.top()),
            Vec2::new(width, bounds.height()),
        );
        chrome::sidebar(
            ui,
            side,
            workspace,
            config,
            state,
            p,
            reveal >= 1.0,
            actions,
        );
    }
    let toolbar = Rect::from_min_max(
        egui::pos2(bounds.left() + edge, bounds.top()),
        egui::pos2(
            bounds.right(),
            bounds.top() + theme::metrics::TOOLBAR_HEIGHT,
        ),
    );
    let toggle = egui::lerp(chrome::TOGGLE_HIDDEN..=width - 22.0, reveal);
    chrome::toolbar(
        ui,
        toolbar,
        (bounds.left() + toggle + 24.0).max(toolbar.left() + 8.0),
        workspace,
        config,
        state,
        p,
        actions,
    );
    // Content sits in the material as inset rounded surfaces; the sidebar
    // supplies its own trailing margin.
    let gutter = theme::metrics::GUTTER;
    let mut top = toolbar.bottom();
    let left = bounds.left() + edge.max(gutter);
    if workspace.tabs.len() > 1 {
        let strip = Rect::from_min_max(
            egui::pos2(left, top),
            egui::pos2(bounds.right() - gutter, top + theme::metrics::PANE_HEADER),
        );
        chrome::tabs(ui, strip, workspace, p, actions);
        top = strip.bottom() + 2.0;
    }
    let stage = Rect::from_min_max(
        egui::pos2(left, top),
        egui::pos2(bounds.right() - gutter, bounds.bottom() - gutter),
    );
    history_buttons(&ctx, workspace, state, stage, actions);
    if stage.is_positive() {
        content::panes(ui, stage, workspace, config, state, p, actions);
    }
    // The open overlay carries the window controls and toggle with it.
    if overlay > 0.0 {
        chrome::sidebar_overlay(
            ui, bounds, width, overlay, workspace, config, state, p, actions,
        );
    } else if chrome::leading_controls(ui, bounds, p, toggle, actions) {
        toggle_sidebar(config, state, available, actions);
    }
    if radius > 0 {
        ui.painter().rect_stroke(
            bounds,
            radius,
            Stroke::new(1.0, p.border),
            StrokeKind::Inside,
        );
    }
    resize_edges(ui, bounds);
    dialogs::show(&ctx, workspace, config, state, p, actions);
    show_notice(&ctx, p, state);
    if let Some(level) = &state.level
        && !crate::controls::level(&ctx, p, level)
    {
        state.level = None;
    }
}

/// Shows the current message. Problems wait for dismissal; anything else
/// leaves after a few seconds, sleeping until then.
fn show_notice(ctx: &egui::Context, p: Palette, state: &mut UiState) {
    let Some(notice) = state.notice.as_mut() else {
        return;
    };
    let now = ctx.input(|input| input.time);
    let shown = *notice.shown.get_or_insert(now);
    if notice.tone != Tone::Problem {
        let left = shown + Notice::LINGER - now;
        if left <= 0.0 {
            state.notice = None;
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(left));
    }
    let mark = match notice.tone {
        Tone::Info => (Icon::Info, p.secondary),
        Tone::Problem => (Icon::Warning, p.red),
    };
    if crate::controls::notice(ctx, p, &notice.text, mark) {
        state.notice = None;
    }
}

/// Match Terminal's native shape: Windows supplies its own rounding; macOS
/// keeps rounded corners when maximized, while fullscreen is always square.
fn window_radius(ctx: &egui::Context) -> u8 {
    if !cfg!(target_os = "windows")
        && ctx.input(|i| {
            !i.viewport().fullscreen.unwrap_or(false)
                && (cfg!(target_os = "macos") || !i.viewport().maximized.unwrap_or(false))
        })
    {
        theme::metrics::WINDOW_RADIUS
    } else {
        0
    }
}

/// The mouse's back and forward buttons travel the history of the pane under
/// the pointer, or of the active pane anywhere else in the window.
fn history_buttons(
    ctx: &egui::Context,
    workspace: &Workspace,
    state: &UiState,
    stage: Rect,
    actions: &mut Vec<Action>,
) {
    if state.dialog.is_some() || state.sidebar_overlay {
        return;
    }
    let presses = ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|event| match event {
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed: true,
                    ..
                } => match button {
                    egui::PointerButton::Extra1 => Some((*pos, false)),
                    egui::PointerButton::Extra2 => Some((*pos, true)),
                    _ => None,
                },
                _ => None,
            })
            .collect::<Vec<_>>()
    });
    let tab = &workspace.tabs[workspace.active];
    for (pos, forward) in presses {
        let target = content::pane_rects(stage, tab.panes.len())
            .position(|rect| rect.contains(pos))
            .map_or(workspace.pane().id, |index| tab.panes[index].id);
        actions.push(Action::Back(target, forward));
    }
}

/// Docks or hides the sidebar in a wide window; opens or closes it over the
/// files in a narrow one, leaving the saved preference alone.
fn toggle_sidebar(
    config: &mut Config,
    state: &mut UiState,
    available: bool,
    actions: &mut Vec<Action>,
) {
    if available {
        config.sidebar = !config.sidebar;
        actions.push(Action::PreferencesChanged);
    } else {
        state.sidebar_overlay = !state.sidebar_overlay;
    }
}

fn take(ctx: &egui::Context, modifiers: Modifiers, key: Key) -> bool {
    ctx.input_mut(|i| i.consume_key(modifiers, key))
}
/// Whether this frame asks to copy, cut or paste. The native window reports
/// Command+C, X and V as Copy, Cut and Paste events, never as pressed keys,
/// and sends Paste only while the system clipboard holds text; without any,
/// releasing V pastes instead, unless that press already pasted.
fn clipboard_chords(ctx: &egui::Context, pasted: &mut bool) -> (bool, bool, bool) {
    ctx.input(|i| {
        let (mut copy, mut cut, mut paste) = (false, false, false);
        for event in &i.events {
            match event {
                egui::Event::Copy => copy = true,
                egui::Event::Cut => cut = true,
                egui::Event::Paste(_) => {
                    paste = true;
                    *pasted = true;
                }
                egui::Event::Key {
                    key: key @ (Key::V | Key::Paste),
                    pressed: false,
                    modifiers,
                    ..
                } => {
                    let chord = *key == Key::Paste || modifiers.command;
                    paste |= !std::mem::take(pasted) && chord;
                }
                _ => {}
            }
        }
        (copy, cut, paste)
    })
}
fn keyboard(
    ctx: &egui::Context,
    workspace: &mut Workspace,
    config: &mut Config,
    state: &mut UiState,
    available: bool,
    actions: &mut Vec<Action>,
) {
    let command = Modifiers::COMMAND;
    // Terminal's app zoom chords, which also answer over dialogs and fields.
    if take(ctx, command, Key::Plus) || take(ctx, command, Key::Equals) {
        actions.push(Action::Zoom(ZoomStep::In));
    }
    if take(ctx, command, Key::Minus) {
        actions.push(Action::Zoom(ZoomStep::Out));
    }
    if take(ctx, command, Key::Num0) {
        actions.push(Action::Zoom(ZoomStep::Reset));
    }
    if state.dialog.is_some() {
        return;
    }
    // Terminal's sidebar chord, so both windows answer the same keys.
    let host = if cfg!(target_os = "macos") {
        Modifiers::COMMAND
    } else {
        Modifiers::COMMAND | Modifiers::SHIFT
    };
    if take(ctx, host, Key::B) {
        toggle_sidebar(config, state, available, actions);
        ctx.request_repaint();
    }
    // The overlay owns the remaining keys while open; Escape closes it.
    if state.sidebar_overlay {
        return;
    }
    if let Some(owner) = ctx
        .memory(|m| m.focused())
        .and_then(|id| state.file_owners.get(&id).copied())
    {
        let tab = &mut workspace.tabs[workspace.active];
        if let Some(index) = tab.panes.iter().position(|p| p.id == owner) {
            tab.active = index;
        }
    }
    // egui releases a field's focus on Escape before the frame begins, so
    // Escape is claimed here: it ends typing a location and goes no further.
    if state.location.is_some() && take(ctx, Modifiers::NONE, Key::Escape) {
        state.location = None;
    }
    let shift = Modifiers {
        shift: true,
        ..command
    };
    if take(ctx, command, Key::L) || take(ctx, shift, Key::G) {
        state.go_to_folder(workspace.pane());
        return;
    }
    if take(ctx, command, Key::F) {
        state.focus_search = true;
    }
    if take(ctx, command, Key::T) {
        actions.push(Action::NewTab(workspace.pane().path.clone()));
    }
    if take(ctx, command, Key::W) {
        actions.push(Action::CloseTab(workspace.active));
    }
    if take(ctx, command, Key::Comma) {
        state.open_dialog(Dialog::Preferences);
        return;
    }
    if take(ctx, command, Key::Period)
        || take(ctx, shift, Key::Period)
        || (!cfg!(target_os = "macos") && take(ctx, command, Key::H))
    {
        actions.push(Action::ToggleHidden);
    }
    if take(ctx, Modifiers::NONE, Key::F5) {
        actions.push(Action::Reload);
    }
    if take(ctx, command, Key::R) {
        actions.push(Action::Reload);
    }
    if take(ctx, command, Key::Num1) {
        actions.push(Action::View(ViewMode::Grid));
    }
    if take(ctx, command, Key::Num2) {
        actions.push(Action::View(ViewMode::List));
    }
    if take(ctx, Modifiers::ALT, Key::ArrowLeft) {
        actions.push(Action::Back(workspace.pane().id, false));
    }
    if take(ctx, Modifiers::ALT, Key::ArrowRight) {
        actions.push(Action::Back(workspace.pane().id, true));
    }
    if take(ctx, Modifiers::ALT, Key::ArrowUp) {
        actions.push(Action::Parent);
    }
    if take(ctx, shift, Key::N) {
        state.open_dialog(Dialog::New {
            parent: workspace.pane().path.clone(),
            name: String::new(),
            folder: true,
        });
        return;
    }
    if take(ctx, command, Key::Backslash) {
        actions.push(Action::ToggleSplit);
    }
    if ctx.text_edit_focused() {
        return;
    }
    let (copy, cut, paste) = clipboard_chords(ctx, &mut state.pasted);
    if copy || cut {
        actions.push(Action::Copy(
            workspace.pane().selected.iter().cloned().collect(),
            cut,
        ));
    }
    if paste {
        actions.push(Action::Paste(workspace.pane().path.clone()));
    }
    // Ubuntu Files' and Windows' Alt+Enter, and Finder's Get Info chord. The
    // chorded Enter is taken before a plain Enter can open the item.
    if take(ctx, Modifiers::ALT, Key::Enter) || take(ctx, command, Key::I) {
        let pane = workspace.pane();
        if let Some(entry) = pane.selected_entry() {
            actions.push(Action::Properties(pane.id, entry.clone()));
        } else if pane.selected.is_empty() {
            actions.push(Action::Properties(pane.id, folder_entry(pane)));
        } else {
            state.notice = Some(Notice::info("Select one item to see its properties."));
        }
    }
    if take(ctx, command, Key::A) {
        workspace.pane_mut().selected = workspace
            .pane()
            .entries
            .iter()
            .map(|e| e.path.clone())
            .collect();
    }
    // Native chrome controls keep Enter/Space/arrows while they own focus.
    if ctx
        .memory(|m| m.focused())
        .is_some_and(|id| !state.file_widgets.contains(&id))
    {
        return;
    }
    if take(ctx, Modifiers::NONE, Key::Escape) {
        workspace.pane_mut().selected.clear();
    }
    if take(ctx, Modifiers::NONE, Key::F2)
        && let Some(entry) = workspace.pane().selected_entry()
    {
        state.open_dialog(Dialog::Rename {
            source: entry.path.clone(),
            name: entry.name.clone(),
        });
        return;
    }
    if (take(ctx, Modifiers::NONE, Key::Delete)
        || (cfg!(target_os = "macos") && take(ctx, command, Key::Backspace)))
        && !workspace.pane().selected.is_empty()
    {
        state.open_dialog(Dialog::Trash {
            paths: workspace.pane().selected.iter().cloned().collect(),
        });
        return;
    }
    if take(ctx, Modifiers::NONE, Key::Space)
        && let Some(entry) = workspace.pane().selected_entry()
    {
        actions.push(Action::QuickLook(entry.path.clone()));
    }
    // A held Enter opens once: repeating it would queue duplicate extractions
    // or launch an application again and again.
    let repeated = ctx.input(|i| {
        i.events.iter().any(|event| {
            matches!(
                event,
                egui::Event::Key {
                    key: Key::Enter,
                    pressed: true,
                    repeat: true,
                    ..
                }
            )
        })
    });
    if take(ctx, Modifiers::NONE, Key::Enter)
        && !repeated
        && let Some(entry) = workspace.pane().selected_entry()
    {
        actions.push(Action::Open(
            workspace.pane().id,
            entry.path.clone(),
            entry.is_dir(),
        ));
    }
    // Arrow navigation changes only visible selection; scrolling is virtualized.
    let stride = if config.view == ViewMode::Grid {
        state
            .grid_columns
            .get(&workspace.pane().id)
            .copied()
            .unwrap_or(1)
            .max(1) as isize
    } else {
        1
    };
    for (key, delta) in [
        (Key::ArrowDown, stride),
        (Key::ArrowUp, -stride),
        (Key::ArrowRight, 1),
        (Key::ArrowLeft, -1),
    ] {
        let extend = ctx.input(|i| i.modifiers.shift);
        if take(
            ctx,
            if extend {
                Modifiers::SHIFT
            } else {
                Modifiers::NONE
            },
            key,
        ) {
            let pane = workspace.pane_mut();
            if pane.entries.is_empty() {
                continue;
            }
            let index = (pane.focus_index.unwrap_or(0) as isize + delta)
                .clamp(0, pane.entries.len() as isize - 1) as usize;
            pane.select(index, false, extend);
            pane.scroll_to = Some(index);
        }
    }
    if take(ctx, Modifiers::NONE, Key::Home) && !workspace.pane().entries.is_empty() {
        workspace.pane_mut().select(0, false, false);
        workspace.pane_mut().scroll_to = Some(0);
    }
    if take(ctx, Modifiers::NONE, Key::End) && !workspace.pane().entries.is_empty() {
        let i = workspace.pane().entries.len() - 1;
        workspace.pane_mut().select(i, false, false);
        workspace.pane_mut().scroll_to = Some(i);
    }
}
fn resize_edges(ui: &egui::Ui, bounds: Rect) {
    if ui.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    use egui::ResizeDirection::*;
    for (direction, rect, cursor) in [
        (
            East,
            Rect::from_min_max(
                bounds.right_top() - Vec2::new(4.0, -8.0),
                bounds.right_bottom() - Vec2::new(0.0, 8.0),
            ),
            egui::CursorIcon::ResizeHorizontal,
        ),
        (
            West,
            Rect::from_min_max(
                bounds.left_top() + Vec2::new(0.0, 8.0),
                bounds.left_bottom() + Vec2::new(4.0, -8.0),
            ),
            egui::CursorIcon::ResizeHorizontal,
        ),
        (
            South,
            Rect::from_min_max(
                bounds.left_bottom() + Vec2::new(8.0, -4.0),
                bounds.right_bottom() - Vec2::new(8.0, 0.0),
            ),
            egui::CursorIcon::ResizeVertical,
        ),
        (
            NorthWest,
            Rect::from_min_size(bounds.min, Vec2::splat(8.0)),
            egui::CursorIcon::ResizeNwSe,
        ),
        (
            NorthEast,
            Rect::from_min_size(bounds.right_top() - Vec2::new(8.0, 0.0), Vec2::splat(8.0)),
            egui::CursorIcon::ResizeNeSw,
        ),
        (
            SouthWest,
            Rect::from_min_size(bounds.left_bottom() - Vec2::new(0.0, 8.0), Vec2::splat(8.0)),
            egui::CursorIcon::ResizeNeSw,
        ),
        (
            SouthEast,
            Rect::from_min_size(bounds.max - Vec2::splat(8.0), Vec2::splat(8.0)),
            egui::CursorIcon::ResizeNwSe,
        ),
        (
            North,
            Rect::from_min_max(
                bounds.min + Vec2::new(8.0, 0.0),
                bounds.right_top() + Vec2::new(-8.0, 4.0),
            ),
            egui::CursorIcon::ResizeVertical,
        ),
    ] {
        let response = ui
            .interact(
                rect,
                ui.id().with(("resize", format!("{direction:?}"))),
                egui::Sense::drag(),
            )
            .on_hover_cursor(cursor);
        if response.drag_started() {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
        }
    }
}

/// The folder a pane shows, for actions on the folder itself. Its details
/// arrive with the properties read.
pub fn folder_entry(pane: &Pane) -> Entry {
    let name = name(&pane.path);
    Entry {
        path: pane.path.clone(),
        hidden: name.starts_with('.'),
        name,
        kind: EntryKind::Directory,
        size: 0,
        modified: None,
    }
}

/// Properties' keyboard chord as the platform writes it.
pub fn properties_shortcut() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘I"
    } else {
        "Alt+Enter"
    }
}

pub fn name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| path.display().to_string())
}
/// The folders from the file system root, or from Home when inside it.
pub fn breadcrumbs(path: &Path, home: &Path) -> Vec<(String, Icon, PathBuf)> {
    let mut parts = Vec::new();
    for ancestor in path.ancestors() {
        if ancestor == home {
            parts.push(("Home".to_owned(), Icon::Home, ancestor.to_owned()));
            break;
        }
        if ancestor.parent().is_none() {
            parts.push((
                "File system".to_owned(),
                Icon::Computer,
                ancestor.to_owned(),
            ));
            break;
        }
        parts.push((name(ancestor), Icon::Folder, ancestor.to_owned()));
    }
    parts.reverse();
    parts
}
/// A path as a person reads it, with the home folder written as `~`.
pub fn display_path(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        Err(_) => path.display().to_string(),
    }
}
pub fn bytes(value: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = value as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < 4 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} B", value as u64)
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
