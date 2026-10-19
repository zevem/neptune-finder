//! The info panel at the window's trailing edge, where Terminal keeps its
//! right panel: the selected item's picture, name and kind, quick actions,
//! then its details and dates. With nothing selected it describes the folder
//! in view; with several items selected, how many. It reads nothing itself:
//! the app reads the item's properties and picture on workers.
use super::{
    Action, Dialog, UiState, bytes, content, dialogs, display_path, folder_entry,
    properties_shortcut,
};
use crate::{
    config::Config,
    controls,
    icons::{self, Icon},
    model::{Pane, Workspace},
    theme::{self, Palette, metrics},
    thumbnails,
};
use eframe::egui::{
    self, Align, Color32, CursorIcon, Layout, Rect, Sense, Stroke, Ui, UiBuilder, Vec2, WidgetInfo,
    WidgetType, pos2, vec2,
};
use finder_core::{Entry, EntryKind, Permissions, Properties};
use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

/// The files keep at least this much of the window beside the panel.
pub const MIN_STAGE: f32 = 360.0;
/// A details row, shorter than a settings row for a narrow column.
const ROW: f32 = 34.0;

/// What the panel describes.
#[derive(Clone, Copy)]
enum Subject<'a> {
    /// Nothing is selected: the folder in view.
    Folder,
    Item(&'a Entry),
    /// Several items, by number.
    Several(usize),
}

/// A quick action under the name. It acts on what the panel showed when it
/// was pressed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Verb {
    Open,
    QuickLook,
    Pin,
    Rename,
    CopyPath,
    Properties,
    Trash,
    NewFolder,
    Copy,
    Cut,
    Compress,
}

/// The panel in `rect`, which slides past the window's trailing edge while a
/// toggle is under way.
#[allow(clippy::too_many_arguments)]
pub fn panel(
    ui: &mut Ui,
    rect: Rect,
    window: Rect,
    reveal: f32,
    workspace: &Workspace,
    config: &mut Config,
    state: &mut UiState,
    p: Palette,
    actions: &mut Vec<Action>,
) {
    let mut child = ui.new_child(
        UiBuilder::new()
            .id_salt("info-panel")
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min)),
    );
    child.set_clip_rect(rect.intersect(window));
    child.multiply_opacity(reveal);
    child
        .interact(rect, child.id().with("region"), Sense::hover())
        .widget_info(|| WidgetInfo::labeled(WidgetType::Panel, true, "Info panel"));
    let pane = workspace.pane();
    let gutter = metrics::GUTTER;
    let body = Rect::from_min_max(
        pos2(rect.left() + gutter, rect.top()),
        pos2(rect.right() - gutter, rect.bottom() - gutter),
    );
    controls::place(
        &mut child,
        body,
        Layout::top_down(Align::Min),
        "info-body",
        |ui| {
            egui::ScrollArea::vertical()
                .id_salt("info-scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    contents(ui, pane, config, state, p, actions);
                    ui.add_space(12.0);
                });
        },
    );
    // The leading edge resizes the panel once it rests there.
    if reveal >= 1.0 {
        resize(&mut child, rect, window, config, state, p, actions);
    }
}

fn contents(
    ui: &mut Ui,
    pane: &Pane,
    config: &Config,
    state: &mut UiState,
    p: Palette,
    actions: &mut Vec<Action>,
) {
    let width = ui.available_width();
    let (_, tile) = ui.allocate_space(vec2(width, (width * 0.75).clamp(140.0, 240.0).round()));
    ui.painter().rect_filled(tile, metrics::PANE_RADIUS, p.bg);
    let picture = tile.shrink(12.0);
    let subject = subject(pane, &mut state.info_index);
    let item = match subject {
        Subject::Item(entry) => Some(entry),
        _ => None,
    };
    let mut dimensions = None;
    let (title, kind, verbs) = match subject {
        Subject::Item(entry) => {
            state.info_subject = Some((entry.path.clone(), entry.modified));
            let pixels = thumbnails::pixels(
                picture.width().max(picture.height()),
                ui.ctx().pixels_per_point(),
            );
            let shown = state.info_picture.get(entry, pixels);
            dimensions = shown.and_then(|shown| shown.dimensions);
            match shown.and_then(|shown| shown.texture.as_ref()) {
                Some(texture) => content::paint_thumbnail(ui.painter(), entry, texture, picture, p),
                None => glyph(ui, entry, picture, p),
            }
            (
                entry.name.clone(),
                content::kind(entry),
                item_actions(entry, config),
            )
        }
        Subject::Folder => {
            let folder = folder_entry(pane);
            glyph(ui, &folder, picture, p);
            let pinned = config.favorites.contains(&pane.path);
            (
                folder.name,
                folder_status(pane),
                vec![
                    (
                        Verb::NewFolder,
                        Icon::FolderPlus,
                        "New folder",
                        controls::edit_shortcut("Shift+N"),
                    ),
                    (
                        Verb::Pin,
                        Icon::Pin,
                        if pinned {
                            "Unpin this folder"
                        } else {
                            "Pin this folder"
                        },
                        String::new(),
                    ),
                    (Verb::CopyPath, Icon::Copy, "Copy path", String::new()),
                    (
                        Verb::Properties,
                        Icon::Info,
                        "Properties",
                        properties_shortcut().into(),
                    ),
                ],
            )
        }
        Subject::Several(count) => {
            icons::paint(
                ui.painter(),
                Rect::from_center_size(picture.center(), Vec2::splat(64.0)),
                Icon::Files,
                p.secondary,
            );
            (
                format!("{count} items"),
                "Selected".into(),
                vec![
                    (Verb::Copy, Icon::Copy, "Copy", controls::edit_shortcut("C")),
                    (
                        Verb::Cut,
                        Icon::Scissors,
                        "Cut",
                        controls::edit_shortcut("X"),
                    ),
                    (Verb::Compress, Icon::FileZip, "Compress", String::new()),
                    (Verb::Trash, Icon::Trash, "Move to Trash", "Delete".into()),
                ],
            )
        }
    };
    ui.add_space(12.0);
    heading(ui, p, &title);
    ui.add_space(2.0);
    dialogs::centered_line(ui, p, &kind, theme::regular(12.0), p.secondary, false);
    ui.add_space(10.0);
    if let Some(verb) = quick_actions(ui, &verbs) {
        act(verb, pane, item, state, actions);
    }
    match subject {
        Subject::Item(entry) => {
            let read = state
                .info_read
                .as_ref()
                .filter(|(path, _)| path == &entry.path)
                .map(|(_, read)| read);
            if details(ui, p, pane, entry, read, dimensions, &state.home) {
                actions.push(Action::ShowInFolder(pane.id, entry.path.clone()));
            }
        }
        Subject::Folder => {
            ui.add_space(18.0);
            dialogs::centered_line(
                ui,
                p,
                "Select an item to see its details.",
                theme::regular(12.0),
                p.muted,
                false,
            );
        }
        Subject::Several(_) => {}
    }
}

/// What the panel describes in `pane`.
fn subject<'a>(pane: &'a Pane, hint: &mut Option<usize>) -> Subject<'a> {
    match pane.selected.len() {
        0 => Subject::Folder,
        1 => selected_entry(pane, hint).map_or(Subject::Folder, Subject::Item),
        count => Subject::Several(count),
    }
}

/// The one selected entry, looked for where the keyboard last was, then
/// where it was found last, and only then through the whole listing.
fn selected_entry<'a>(pane: &'a Pane, hint: &mut Option<usize>) -> Option<&'a Entry> {
    let path = pane.selected.first()?;
    let at = |index: usize| {
        pane.entries
            .get(index)
            .is_some_and(|entry| &entry.path == path)
            .then_some(index)
    };
    let index = pane
        .focus_index
        .and_then(at)
        .or_else(|| (*hint).and_then(at))
        .or_else(|| pane.entries.iter().position(|entry| &entry.path == path))?;
    *hint = Some(index);
    pane.entries.get(index)
}

/// An item's quick actions: open it, look at it or pin it, rename it, copy
/// its path, show its properties or move it to the Trash.
fn item_actions(entry: &Entry, config: &Config) -> Vec<(Verb, Icon, &'static str, String)> {
    let archive = !entry.is_dir() && finder_operations::is_extractable(&entry.path);
    let open = if entry.is_dir() {
        (Verb::Open, Icon::FolderOpen, "Open", "Enter".into())
    } else if archive {
        (Verb::Open, Icon::FolderOpen, "Extract", "Enter".into())
    } else {
        (Verb::Open, Icon::ExternalLink, "Open", "Enter".into())
    };
    let look = if entry.is_dir() {
        let pinned = config.favorites.contains(&entry.path);
        (
            Verb::Pin,
            Icon::Pin,
            if pinned {
                "Unpin from sidebar"
            } else {
                "Pin to sidebar"
            },
            String::new(),
        )
    } else {
        (Verb::QuickLook, Icon::Eyes, "Quick Look", "Space".into())
    };
    vec![
        open,
        look,
        (Verb::Rename, Icon::Pencil, "Rename", "F2".into()),
        (Verb::CopyPath, Icon::Copy, "Copy path", String::new()),
        (
            Verb::Properties,
            Icon::Info,
            "Properties",
            properties_shortcut().into(),
        ),
        (Verb::Trash, Icon::Trash, "Move to Trash", "Delete".into()),
    ]
}

/// A centred row of icon buttons, each named with its shortcut. Returns the
/// one pressed.
fn quick_actions(ui: &mut Ui, buttons: &[(Verb, Icon, &str, String)]) -> Option<Verb> {
    let count = buttons.len() as f32;
    let width = count * 28.0 + (count - 1.0) * 4.0;
    let (_, row) = ui.allocate_space(vec2(ui.available_width(), 28.0));
    controls::place(
        ui,
        Rect::from_center_size(row.center(), vec2(width, 28.0)),
        Layout::left_to_right(Align::Center),
        "info-actions",
        |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let mut chosen = None;
            for (verb, icon, label, hint) in buttons {
                if icons::button_with_hint(ui, *icon, label, hint).clicked() {
                    chosen = Some(*verb);
                }
            }
            chosen
        },
    )
}

/// Carries out a quick action on the item shown (`item`), or else on the
/// folder in view or the items selected in it.
fn act(
    verb: Verb,
    pane: &Pane,
    item: Option<&Entry>,
    state: &mut UiState,
    actions: &mut Vec<Action>,
) {
    let path = item.map_or_else(|| pane.path.clone(), |entry| entry.path.clone());
    let targets = || -> Vec<PathBuf> {
        item.map_or_else(
            || pane.selected.iter().cloned().collect(),
            |entry| vec![entry.path.clone()],
        )
    };
    match verb {
        Verb::Open => {
            if let Some(entry) = item {
                actions.push(Action::Open(pane.id, path, entry.is_dir()));
            }
        }
        Verb::QuickLook => actions.push(Action::QuickLook(path)),
        Verb::Pin => actions.push(Action::Favorite(path)),
        Verb::Rename => {
            if let Some(entry) = item {
                state.open_dialog(Dialog::Rename {
                    source: path,
                    name: entry.name.clone(),
                });
            }
        }
        Verb::CopyPath => actions.push(Action::CopyPath(path)),
        Verb::Properties => actions.push(Action::Properties(
            pane.id,
            item.cloned().unwrap_or_else(|| folder_entry(pane)),
        )),
        Verb::Trash => state.open_dialog(Dialog::Trash { paths: targets() }),
        Verb::NewFolder => state.open_dialog(Dialog::New {
            parent: path,
            name: String::new(),
            folder: true,
        }),
        Verb::Copy | Verb::Cut => actions.push(Action::Copy(targets(), verb == Verb::Cut)),
        Verb::Compress => {
            let sources = targets();
            state.open_dialog(Dialog::Compress {
                name: content::archive_name(&sources),
                sources,
                destination: pane.path.clone(),
            });
        }
    }
}

/// The item's details and dates. Rows keep their place while its properties
/// load. Returns whether "Open parent folder" was pressed.
fn details(
    ui: &mut Ui,
    p: Palette,
    pane: &Pane,
    entry: &Entry,
    read: Option<&Result<Properties, String>>,
    dimensions: Option<[u32; 2]>,
    home: &Path,
) -> bool {
    let loaded = read.and_then(|read| read.as_ref().ok());
    // A value that has not loaded yet is a dash; one the system cannot give
    // says so.
    let missing = if loaded.is_some() {
        "Unavailable"
    } else {
        "—"
    };
    let contents = loaded.and_then(|loaded| loaded.contents);
    let mut show_in_folder = false;
    ui.add_space(18.0);
    controls::section_label(ui, p, "Details");
    controls::group(ui, p, |ui, rows| {
        if entry.kind == EntryKind::Directory {
            // A folder's count runs on, and its rows follow it.
            let counting = !matches!(read, Some(Err(_)))
                && !contents.is_some_and(|contents| contents.complete);
            // A folder that cannot be listed counts nothing, which is not empty.
            let unreadable = contents.is_some_and(|contents| {
                contents.complete && contents.partial && contents.items == 0
            });
            let counted = contents
                .filter(|contents| (contents.complete || contents.items > 0) && !unreadable);
            let items = if unreadable {
                "None readable".to_owned()
            } else {
                counted.map_or_else(
                    || "—".to_owned(),
                    |contents| {
                        format!(
                            "{}{}",
                            if contents.limited { "At least " } else { "" },
                            dialogs::grouped(contents.items)
                        )
                    },
                )
            };
            rows.row_with_height(ui, "Items", ROW, |ui| {
                dialogs::value(ui, p, "Items", &items, &items, false);
                if counting {
                    spinner(ui, p);
                }
            });
            let size = if unreadable {
                missing.to_owned()
            } else {
                counted.map_or_else(|| "—".to_owned(), |contents| bytes(contents.bytes))
            };
            fact(ui, rows, p, "Size", &size, &size);
        } else if entry.kind == EntryKind::File {
            fact(
                ui,
                rows,
                p,
                "Size",
                &bytes(entry.size),
                &dialogs::file_size(entry.size),
            );
        }
        if let Some([width, height]) = dimensions {
            let text = format!("{width} × {height}");
            fact(ui, rows, p, "Dimensions", &text, &text);
        }
        if matches!(entry.kind, EntryKind::Symlink | EntryKind::DirectoryLink) {
            let target = loaded
                .and_then(|loaded| loaded.link_target.as_ref())
                .map_or_else(|| missing.to_owned(), |target| target.display().to_string());
            rows.row_with_height(ui, "Link target", ROW, |ui| {
                dialogs::value(ui, p, "Link target", &target, &target, true)
            });
        }
        if let Some(parent) = entry.path.parent() {
            // A search result elsewhere offers its own folder.
            let elsewhere = parent != pane.path;
            rows.row_with_height(ui, "Where", ROW, |ui| {
                if elsewhere {
                    show_in_folder =
                        icons::button(ui, Icon::FolderOpen, "Open parent folder").clicked();
                    ui.add_space(2.0);
                }
                dialogs::value(
                    ui,
                    p,
                    "Where",
                    &display_path(parent, home),
                    &parent.display().to_string(),
                    true,
                );
            });
        }
        let access = match loaded.and_then(|loaded| loaded.permissions.as_ref()) {
            Some(Permissions::Unix { access, .. }) => {
                dialogs::access_words(access.read, access.write, access.execute, entry.is_dir())
            }
            Some(Permissions::ReadOnly(true)) => "Read only".into(),
            Some(Permissions::ReadOnly(false)) => "Read and write".into(),
            None => missing.to_owned(),
        };
        fact(ui, rows, p, "Your access", &access, &access);
        if entry.kind == EntryKind::Directory {
            let free = loaded
                .and_then(|loaded| loaded.available_space)
                .map_or_else(|| missing.to_owned(), bytes);
            fact(ui, rows, p, "Free space", &free, &free);
        }
        if let Some(Err(error)) = read {
            rows.note(ui, error);
        } else if let Some(note) = contents.and_then(dialogs::count_note) {
            rows.note(ui, note);
        }
    });
    ui.add_space(18.0);
    controls::section_label(ui, p, "Dates");
    controls::group(ui, p, |ui, rows| {
        let today = chrono::Local::now().date_naive();
        date(ui, rows, p, "Modified", entry.modified, missing, today);
        let created = loaded.and_then(|loaded| loaded.created);
        date(ui, rows, p, "Created", created, missing, today);
    });
    show_in_folder
}

/// A labelled value at the trailing edge; a truncated value shows `whole`.
fn fact(ui: &mut Ui, rows: &mut controls::Group, p: Palette, label: &str, text: &str, whole: &str) {
    rows.row_with_height(ui, label, ROW, |ui| {
        dialogs::value(ui, p, label, text, whole, false)
    });
}

/// A date as the list writes it, with the full time in its tooltip.
fn date(
    ui: &mut Ui,
    rows: &mut controls::Group,
    p: Palette,
    label: &str,
    time: Option<SystemTime>,
    missing: &str,
    today: chrono::NaiveDate,
) {
    let text = match time {
        Some(_) => content::relative_date(time, today),
        None => missing.to_owned(),
    };
    let whole = dialogs::date_time(time).unwrap_or_else(|| text.clone());
    fact(ui, rows, p, label, &text, &whole);
}

/// The kind glyph, large, in the middle of the picture's place.
fn glyph(ui: &Ui, entry: &Entry, area: Rect, p: Palette) {
    let (icon, ink) = content::file_icon(entry, p);
    content::paint_icon(
        ui.painter(),
        entry,
        icon,
        ink,
        Rect::from_center_size(area.center(), Vec2::splat(64.0_f32.min(area.height()))),
        p,
    );
}

/// The name, centred over up to three lines, whole in its tooltip when cut.
fn heading(ui: &mut Ui, p: Palette, text: &str) {
    let width = ui.available_width();
    let mut job =
        egui::text::LayoutJob::simple(text.to_owned(), theme::semibold(15.0), p.fg, width);
    job.halign = Align::Center;
    job.wrap = egui::text::TextWrapping {
        max_width: width,
        max_rows: 3,
        break_anywhere: false,
        overflow_character: Some('…'),
    };
    let galley = ui.painter().layout_job(job);
    let elided = galley.elided;
    let (rect, response) = ui.allocate_exact_size(vec2(width, galley.size().y), Sense::hover());
    ui.painter().galley(
        pos2(rect.center().x, rect.top()),
        galley,
        Color32::PLACEHOLDER,
    );
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, text));
    if elided {
        response.on_hover_text(text);
    }
}

/// What the folder in view holds, as its path bar says it.
fn folder_status(pane: &Pane) -> String {
    let count = pane.entries.len();
    if pane.error.is_some() {
        "Can’t be opened".into()
    } else if pane.loading && count == 0 {
        "Loading…".into()
    } else {
        let noun = if pane.query.is_empty() {
            "item"
        } else {
            "result"
        };
        format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
    }
}

/// A small spinner beside a value that is still being counted.
fn spinner(ui: &mut Ui, p: Palette) {
    let (_, rect) = ui.allocate_space(vec2(18.0, 14.0));
    let angle = ui.input(|input| input.time) as f32 * std::f32::consts::TAU;
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(16));
    icons::paint_rotated(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(13.0)),
        Icon::Loader2,
        p.muted,
        angle,
    );
}

/// The leading edge's handle: a drag resizes the panel, saved on release,
/// and a double click restores its width.
fn resize(
    ui: &mut Ui,
    rect: Rect,
    window: Rect,
    config: &mut Config,
    state: &mut UiState,
    p: Palette,
    actions: &mut Vec<Action>,
) {
    let gutter = metrics::GUTTER;
    let handle = Rect::from_min_max(
        pos2(rect.left(), rect.top() + 4.0),
        pos2(rect.left() + gutter, rect.bottom() - 12.0),
    );
    let resize = ui
        .interact(handle, ui.id().with("info-resize"), Sense::click_and_drag())
        .on_hover_and_drag_cursor(CursorIcon::ResizeHorizontal);
    resize.widget_info(|| WidgetInfo::labeled(WidgetType::ResizeHandle, true, "Resize info panel"));
    let (low, high) = Config::INFO_PANEL_RANGE;
    if resize.dragged()
        && let Some(pointer) = resize.interact_pointer_pos()
    {
        state.info_drag = Some((window.right() - pointer.x).clamp(low, high));
    }
    if resize.drag_stopped()
        && let Some(width) = state.info_drag.take()
    {
        config.info_panel_width = width.round();
        actions.push(Action::PreferencesChanged);
    }
    if resize.double_clicked() || resize.triple_clicked() {
        state.info_drag = None;
        config.info_panel_width = Config::INFO_PANEL_WIDTH;
        actions.push(Action::PreferencesChanged);
    }
    let grip = controls::animate(
        ui.ctx(),
        resize.id.with("grip"),
        resize.hovered() || resize.dragged(),
        0.12,
    );
    if grip > 0.0 {
        let x = rect.left() + gutter * 0.5;
        ui.painter().line_segment(
            [pos2(x, rect.top() + 8.0), pos2(x, rect.bottom() - 20.0)],
            Stroke::new(
                2.0,
                theme::tint(
                    if resize.dragged() { p.accent } else { p.muted },
                    grip * 0.8,
                ),
            ),
        );
    }
}
