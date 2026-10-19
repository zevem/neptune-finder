//! Folder content: each pane is a rounded surface in the window material with
//! its files, a search scope while searching, and a path bar that says where
//! it is and what it holds.
use super::{Action, Dialog, UiState, breadcrumbs, bytes, folder_entry, properties_shortcut};
use crate::{
    config::{Accent, ColumnWidths, Config, ViewMode},
    controls::{self, ButtonKind},
    icons::{self, Icon},
    model::{Band, Pane, Workspace},
    theme::{self, Palette, metrics},
    thumbnails::{self, Texture},
};
use eframe::egui::{
    self, Align, Align2, Color32, CursorIcon, Galley, Layout, Pos2, Rangef, Rect, Response, Sense,
    Stroke, StrokeKind, UiBuilder, Vec2, WidgetInfo, WidgetType,
    emath::GuiRounding,
    pos2,
    scroll_area::{DragScroll, ScrollSource},
    style::ScrollAnimation,
    vec2,
};
use finder_core::{Entry, EntryKind, SortField};
use finder_operations::Operation;
use std::{
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
};

const PATH_BAR: f32 = 30.0;
const HEADER: f32 = 28.0;
const SCOPE_BAR: f32 = 40.0;
/// File views scroll by wheel and scroll bar. A drag in them selects or moves
/// files, even after egui has seen a touch screen.
const FILES_SCROLL: ScrollSource = ScrollSource {
    scroll_bar: true,
    drag: DragScroll::Never,
    mouse_wheel: true,
};

#[derive(Clone)]
struct FileDrag {
    sources: Vec<PathBuf>,
}

pub fn panes(
    ui: &mut egui::Ui,
    stage: Rect,
    workspace: &mut Workspace,
    config: &mut Config,
    state: &mut UiState,
    p: Palette,
    actions: &mut Vec<Action>,
) {
    let tab = &mut workspace.tabs[workspace.active];
    let count = tab.panes.len();
    let active = tab.active;
    for ((index, pane), rect) in tab
        .panes
        .iter_mut()
        .enumerate()
        .zip(pane_rects(stage, count))
    {
        if state.dialog.is_none()
            && ui.rect_contains_pointer(rect)
            && ui.input(|i| i.pointer.any_pressed())
        {
            actions.push(Action::FocusPane(index));
        }
        let focused = count == 1 || active == index;
        ui.painter().rect_filled(rect, metrics::PANE_RADIUS, p.bg);
        let mut child = ui.new_child(
            UiBuilder::new()
                .id_salt(("pane", pane.id))
                .max_rect(rect)
                .layout(Layout::top_down(Align::Min)),
        );
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        child.spacing_mut().item_spacing = Vec2::ZERO;
        pane_ui(&mut child, rect, pane, config, state, p, focused, actions);
        // With two panes, the one that takes the keys carries the focus ring.
        if count > 1 && focused {
            ui.painter().rect_stroke(
                rect,
                metrics::PANE_RADIUS,
                Stroke::new(1.5, theme::tint(p.accent, 0.7)),
                StrokeKind::Inside,
            );
        }
        if state.dialog.is_none()
            && ui.rect_contains_pointer(rect)
            && ui.input(|i| i.pointer.any_released())
            && let Some(payload) = egui::DragAndDrop::take_payload::<FileDrag>(ui.ctx())
            && let Some(operation) = transfer(
                &payload.sources,
                &pane.path,
                ui.input(|i| i.modifiers.shift),
            )
        {
            actions.push(Action::Submit(operation));
        }
        let external = ui.input(|i| {
            if rect.contains(i.pointer.hover_pos().unwrap_or_default()) {
                i.raw
                    .dropped_files
                    .iter()
                    .map(|f| f.path().to_path_buf())
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            }
        });
        if let Some(operation) = transfer(&external, &pane.path, false) {
            actions.push(Action::Submit(operation));
        }
    }
}

/// Each pane's surface in the stage, side by side with a gutter between.
pub fn pane_rects(stage: Rect, count: usize) -> impl Iterator<Item = Rect> {
    let gutter = metrics::GUTTER;
    let width = (stage.width() - gutter * (count as f32 - 1.0)) / count as f32;
    (0..count).map(move |index| {
        Rect::from_min_size(
            pos2(stage.left() + index as f32 * (width + gutter), stage.top()),
            vec2(width, stage.height()),
        )
    })
}

#[allow(clippy::too_many_arguments)]
fn pane_ui(
    ui: &mut egui::Ui,
    rect: Rect,
    pane: &mut Pane,
    config: &mut Config,
    state: &mut UiState,
    p: Palette,
    focused: bool,
    actions: &mut Vec<Action>,
) {
    let bar = Rect::from_min_max(pos2(rect.left(), rect.bottom() - PATH_BAR), rect.max);
    path_bar(ui, bar, pane, state, p, actions);
    let mut body = Rect::from_min_max(rect.min, pos2(rect.right(), bar.top()));
    if !pane.query.is_empty() {
        let scope = Rect::from_min_size(body.min, vec2(body.width(), SCOPE_BAR));
        search_scope(ui, scope, pane, p, actions);
        body.min.y = scope.bottom();
    }
    controls::place(
        ui,
        body,
        Layout::top_down(Align::Min),
        ("pane-body", pane.id),
        |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            if let Some(error) = pane.error.clone() {
                let retry = message(
                    ui,
                    body,
                    p,
                    Icon::Warning,
                    "This folder can’t be opened",
                    &error,
                    Some("Try again"),
                );
                if retry {
                    actions.push(Action::ReloadPane(pane.id));
                }
                return;
            }
            if pane.entries.is_empty() {
                if pane.loading {
                    loading(ui, body, p);
                } else if !pane.query.is_empty() {
                    message(
                        ui,
                        body,
                        p,
                        Icon::Search,
                        "No results",
                        if pane.recursive {
                            "Try another name."
                        } else {
                            "Try another name, or search all subfolders."
                        },
                        None,
                    );
                } else {
                    message(
                        ui,
                        body,
                        p,
                        Icon::FolderOpen,
                        "This folder is empty",
                        "Drop files here, or create a new folder.",
                        None,
                    );
                }
                blank_menu(ui, body, pane, state, p, actions);
                return;
            }
            match config.view {
                ViewMode::List => list(ui, pane, config, state, p, focused, actions),
                ViewMode::Grid => grid(ui, pane, config, state, p, focused, actions),
            }
        },
    );
}

/// While searching, a strip at the top of the pane chooses where to look.
fn search_scope(
    ui: &mut egui::Ui,
    rect: Rect,
    pane: &mut Pane,
    p: Palette,
    actions: &mut Vec<Action>,
) {
    let painter = ui.painter();
    painter.line_segment(
        [
            pos2(rect.left() + 12.0, rect.bottom() - 0.5),
            pos2(rect.right() - 12.0, rect.bottom() - 0.5),
        ],
        p.hairline(),
    );
    let label = painter.layout_no_wrap("Search".into(), theme::medium(12.0), p.secondary);
    let label = controls::galley_at(painter, pos2(rect.left() + 14.0, rect.center().y), label);
    let width = (rect.width() - label.width() - 40.0).clamp(0.0, 260.0);
    if width < 120.0 {
        return;
    }
    let mut recursive = pane.recursive;
    let field = Rect::from_min_size(
        pos2(label.right() + 10.0, rect.center().y - 14.0),
        vec2(width, 28.0),
    );
    let changed = controls::place(
        ui,
        field,
        Layout::left_to_right(Align::Center),
        ("search-scope", pane.id),
        |ui| {
            controls::segmented(
                ui,
                p,
                "Search scope",
                &mut recursive,
                &[(false, "This folder"), (true, "All subfolders")],
                width,
            )
        },
    );
    if changed {
        pane.recursive = recursive;
        actions.push(Action::Search(pane.id));
    }
}

/// The pane's footer: the path as breadcrumbs, and what the folder holds.
fn path_bar(
    ui: &mut egui::Ui,
    rect: Rect,
    pane: &Pane,
    state: &mut UiState,
    p: Palette,
    actions: &mut Vec<Action>,
) {
    let painter = ui.painter().clone();
    painter.line_segment(
        [
            pos2(rect.left(), rect.top() + 0.5),
            pos2(rect.right(), rect.top() + 0.5),
        ],
        p.hairline(),
    );
    let middle = rect.center().y;
    let text = status_text(pane, state);
    let status = painter.layout_no_wrap(text.clone(), theme::regular(11.5), p.muted);
    let status_width = if rect.width() > 300.0 && !text.is_empty() {
        status.size().x + 14.0
    } else {
        0.0
    };
    if status_width > 0.0 {
        let area = Rect::from_min_max(pos2(rect.right() - status_width, rect.top()), rect.max);
        ui.interact(area, ui.id().with("folder-status"), Sense::hover())
            .on_hover_text(if pane.truncated {
                "Results are limited; refine your search."
            } else {
                "Folder status"
            })
            .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &text));
        painter.galley(
            pos2(rect.right() - status_width, middle - status.size().y * 0.5),
            status,
            Color32::PLACEHOLDER,
        );
    }
    let left = rect.left() + 8.0;
    let right = rect.right() - status_width - 10.0;
    let parts = breadcrumbs(&pane.path, &state.home);
    // Measure every crumb, then keep as many trailing ones as fit.
    let measured = parts
        .iter()
        .enumerate()
        .map(|(i, (label, _, _))| {
            let last = i + 1 == parts.len();
            let galley = painter.layout_no_wrap(
                label.clone(),
                if last {
                    theme::medium(12.0)
                } else {
                    theme::regular(12.0)
                },
                if last { p.fg } else { p.secondary },
            );
            let width = galley.size().x + 12.0 + 18.0;
            (galley, width)
        })
        .collect::<Vec<_>>();
    let mut first = 0;
    let total = |from: usize| measured[from..].iter().map(|(_, w)| w + 14.0).sum::<f32>();
    while first + 1 < parts.len()
        && total(first) + if first > 0 { 26.0 } else { 0.0 } > right - left
    {
        first += 1;
    }
    let mut x = left;
    if first > 0 {
        painter.text(
            pos2(x + 8.0, middle),
            Align2::LEFT_CENTER,
            "…",
            theme::regular(12.0),
            p.muted,
        );
        x += 22.0;
    }
    for (i, (label, icon, path)) in parts.iter().enumerate().skip(first) {
        let last = i + 1 == parts.len();
        if i > first || first > 0 {
            icons::paint(
                &painter,
                Rect::from_center_size(pos2(x + 2.0, middle), Vec2::splat(10.0)),
                Icon::ChevronRight,
                p.muted,
            );
            x += 12.0;
        }
        let (galley, width) = &measured[i];
        let width = width.min(right - x).max(0.0);
        if width < 30.0 {
            break;
        }
        let crumb = Rect::from_min_size(pos2(x, middle - 11.0), vec2(width, 22.0));
        let response = ui.interact(crumb, ui.id().with(("crumb", path)), Sense::click());
        response.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Button, true, format!("Open {}", path.display()))
        });
        if response.hovered() {
            painter.rect_filled(crumb, 6, p.hover);
        }
        if response.has_focus() {
            controls::focus_ring(&painter, crumb, 6, p);
        }
        icons::paint(
            &painter,
            Rect::from_center_size(pos2(crumb.left() + 13.0, middle), Vec2::splat(13.0)),
            *icon,
            if last { p.accent } else { p.muted },
        );
        let text = if galley.size().x > crumb.width() - 30.0 {
            controls::elided(
                &painter,
                label,
                if last {
                    theme::medium(12.0)
                } else {
                    theme::regular(12.0)
                },
                if last { p.fg } else { p.secondary },
                crumb.width() - 30.0,
            )
        } else {
            galley.clone()
        };
        controls::galley_at(&painter, pos2(crumb.left() + 24.0, middle), text);
        x = crumb.right() + 2.0;
        if response.clicked() {
            actions.push(Action::Navigate(pane.id, path.clone()));
        } else if response.middle_clicked() {
            actions.push(Action::NewTab(path.clone()));
        }
        response
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text(path.display().to_string())
            .context_menu(|ui| {
                controls::menu_layout(ui, 210.0);
                if controls::menu_item(ui, p, Icon::Plus, "Open in new tab", "", false) {
                    actions.push(Action::NewTab(path.clone()));
                    ui.close();
                }
                if controls::menu_item(ui, p, Icon::Copy, "Copy path", "", false) {
                    actions.push(Action::CopyPath(path.clone()));
                    ui.close();
                }
                if controls::menu_item(
                    ui,
                    p,
                    Icon::ArrowUpRight,
                    "Go to folder…",
                    &controls::edit_shortcut("L"),
                    false,
                ) {
                    state.open_dialog(Dialog::Location {
                        value: pane.path.display().to_string(),
                    });
                    ui.close();
                }
            });
    }
}

fn status_text(pane: &Pane, state: &UiState) -> String {
    let count = pane.entries.len();
    let items = |n: usize| format!("{n} item{}", if n == 1 { "" } else { "s" });
    // A folder that failed to open has no contents to count.
    let mut text = if pane.error.is_some() {
        String::new()
    } else if pane.loading && count == 0 {
        "Loading…".into()
    } else if pane.selected.is_empty() {
        items(count)
    } else {
        format!("{} of {count} selected", pane.selected.len())
    };
    if pane.truncated {
        text.push_str(" · Results limited");
    }
    if state.busy > 0 {
        text.push_str(&format!(
            "{}{} operation{} running",
            if text.is_empty() { "" } else { " · " },
            state.busy,
            if state.busy == 1 { "" } else { "s" }
        ));
    }
    text
}

/// The narrowest the Name column becomes before the others give way.
const NAME_MIN: f32 = 120.0;

/// The list's columns in Finder's order: field, label, whether the label
/// trails, and the narrowest and widest the column may be.
const COLUMNS: [(SortField, &str, bool, (f32, f32)); 4] = [
    (SortField::Name, "Name", false, (NAME_MIN, f32::INFINITY)),
    (
        SortField::Modified,
        "Date Modified",
        false,
        ColumnWidths::DATE_RANGE,
    ),
    (SortField::Size, "Size", true, ColumnWidths::SIZE_RANGE),
    (SortField::Kind, "Kind", false, ColumnWidths::KIND_RANGE),
];

/// Columns shown at a pane width: name always; date, size and kind as room
/// allows, in Finder's order. Name takes the room the others leave; when that
/// is too little, they give way, the nearest first.
#[derive(Clone, Copy)]
struct Columns {
    name: f32,
    date: f32,
    size: f32,
    kind: f32,
}
impl Columns {
    fn new(width: f32, widths: ColumnWidths) -> Self {
        let date = if width > 460.0 { widths.date } else { 0.0 };
        let kind = if width > 760.0 { widths.kind } else { 0.0 };
        let size = if width > 340.0 { widths.size } else { 0.0 };
        let columns = Self {
            name: width - date - kind - size - 12.0,
            date,
            size,
            kind,
        };
        let short = NAME_MIN - columns.name;
        if short > 0.0 {
            columns.resized(1, short)
        } else {
            columns
        }
    }

    fn widths(&self) -> [f32; 4] {
        [self.name, self.date, self.size, self.kind]
    }

    /// Moves the divider before column `right` by `delta`. The column it
    /// leaves widens; the columns it moves toward give way, the nearest
    /// first, down to their minimums. Hidden columns stay hidden.
    fn resized(&self, right: usize, delta: f32) -> Self {
        let mut widths = self.widths();
        let toward = |step: usize| {
            if delta > 0.0 {
                Some(right + step).filter(|&i| i < COLUMNS.len())
            } else {
                right.checked_sub(step + 1)
            }
        };
        let slack = |widths: &[f32; 4], i: usize| {
            let (min, _) = COLUMNS[i].3;
            (widths[i] - min).max(0.0)
        };
        let grow = if delta > 0.0 {
            (0..right).rev().find(|&i| widths[i] > 0.0).unwrap_or(0)
        } else {
            right
        };
        let room: f32 = (0..COLUMNS.len())
            .filter_map(toward)
            .map(|i| slack(&widths, i))
            .sum();
        let (_, max) = COLUMNS[grow].3;
        let mut amount = delta.abs().min(max - widths[grow]).min(room).max(0.0);
        widths[grow] += amount;
        for i in (0..COLUMNS.len()).filter_map(toward) {
            let give = amount.min(slack(&widths, i));
            widths[i] -= give;
            amount -= give;
        }
        let [name, date, size, kind] = widths;
        Self {
            name,
            date,
            size,
            kind,
        }
    }

    /// The widths to save: those shown here, keeping hidden columns' own.
    fn saved(&self, widths: ColumnWidths) -> ColumnWidths {
        let shown = |width: f32, saved: f32| if width > 0.0 { width } else { saved };
        ColumnWidths {
            date: shown(self.date, widths.date),
            size: shown(self.size, widths.size),
            kind: shown(self.kind, widths.kind),
        }
    }
}

fn list(
    ui: &mut egui::Ui,
    pane: &mut Pane,
    config: &mut Config,
    state: &mut UiState,
    p: Palette,
    focused: bool,
    actions: &mut Vec<Action>,
) {
    let width = ui.available_width();
    let mut columns = Columns::new(width, config.column_widths);
    let (head, _) = ui.allocate_exact_size(vec2(width, HEADER), Sense::hover());
    // A dragged divider moves under the pointer before anything is laid out,
    // so the header and rows follow it in the same frame. It moves from where
    // the columns stood when it was pressed, so dragging back undoes a push,
    // and in whole physical pixels.
    let pixels = ui.ctx().pixels_per_point();
    for (right, &(_, label, _, _)) in COLUMNS.iter().enumerate().skip(1) {
        let id = ui.id().with(("resize", label));
        if ui.ctx().drag_started_id() == Some(id) {
            let origin = ui.input(|i| i.pointer.press_origin().map(|pos| pos.x));
            if let Some(origin) = origin {
                ui.data_mut(|data| data.insert_temp(id, (columns, origin)));
            }
        }
        if ui.ctx().is_being_dragged(id)
            && columns.widths()[right] > 0.0
            && let Some((start, origin)) = ui.data(|data| data.get_temp::<(Columns, f32)>(id))
            && let Some(pointer) = ui.input(|i| i.pointer.interact_pos())
        {
            let delta = ((pointer.x - origin) * pixels).round() / pixels;
            columns = start.resized(right, delta);
            config.column_widths = columns.saved(config.column_widths);
        }
    }
    let widths = columns.widths();
    let mut x = head.left() + 6.0;
    for (&(field, label, trailing, _), column) in COLUMNS.iter().zip(widths) {
        if column <= 0.0 {
            continue;
        }
        let cell = Rect::from_min_size(pos2(x, head.top()), vec2(column, HEADER));
        header(
            ui,
            cell,
            pane,
            p,
            field,
            label,
            trailing,
            x > head.left() + 6.0,
            actions,
        );
        x += column;
    }
    // Each divider trades width between the columns either side of it. The
    // widths are saved on release, and a double click restores the defaults.
    // Dividers have no keyboard behaviour, so they stay out of the focus order.
    for (right, &(_, label, _, _)) in COLUMNS.iter().enumerate().skip(1) {
        if widths[right] <= 0.0 {
            continue;
        }
        let x = head.left() + 6.0 + widths[..right].iter().sum::<f32>();
        let left = (0..right).rev().find(|&i| widths[i] > 0.0).unwrap_or(0);
        let resize = ui
            .interact(
                Rect::from_center_size(pos2(x, head.center().y), vec2(8.0, HEADER)),
                ui.id().with(("resize", label)),
                Sense::CLICK | Sense::DRAG,
            )
            .on_hover_and_drag_cursor(CursorIcon::ResizeHorizontal);
        resize.widget_info(|| {
            WidgetInfo::labeled(
                WidgetType::ResizeHandle,
                true,
                format!("Resize {} column", COLUMNS[left].1),
            )
        });
        if resize.drag_stopped() {
            ui.data_mut(|data| data.remove::<(Columns, f32)>(resize.id));
            actions.push(Action::PreferencesChanged);
        }
        if resize.double_clicked() || resize.triple_clicked() {
            config.column_widths = ColumnWidths::default();
            actions.push(Action::PreferencesChanged);
        }
        let grip = controls::animate(
            ui.ctx(),
            resize.id.with("grip"),
            resize.hovered() || resize.dragged(),
            0.12,
        );
        if grip > 0.0 {
            ui.painter().line_segment(
                [
                    pos2(x, head.center().y - 8.0),
                    pos2(x, head.center().y + 8.0),
                ],
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
    ui.painter().line_segment(
        [
            pos2(head.left(), head.bottom() - 0.5),
            pos2(head.right(), head.bottom() - 0.5),
        ],
        p.hairline(),
    );
    let today = chrono::Local::now().date_naive();
    let area = ui.available_rect_before_wrap();
    let background = files_background(ui, area);
    let mut scroll = egui::ScrollArea::vertical()
        .id_salt(("files-list", pane.id, &pane.path, &pane.query))
        .auto_shrink([false, false])
        .scroll_source(FILES_SCROLL);
    if let Some(index) = pane.scroll_to.take() {
        scroll = scroll.vertical_scroll_offset(index as f32 * config.row_height);
    }
    let pitch = config.row_height;
    let count = pane.entries.len();
    scroll.show_rows(ui, pitch, count, |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let view = View {
            origin: ui.max_rect().left_top() - vec2(0.0, range.start as f32 * pitch),
            area,
            height: count as f32 * pitch,
        };
        // A rectangle covers every row it crosses.
        let marquee = sweep(ui, pane, &background, &view, |rect| Band {
            rows: span(rect.y_range(), pitch, Rangef::new(0.0, pitch), count),
            columns: 0..1,
            stride: 1,
        });
        let mut items = Vec::new();
        for index in range {
            let entry = pane.entries[index].clone();
            let selected = pane.selected.contains(&entry.path);
            ui.push_id(&entry.path, |ui| {
                let (slot, response) = ui.allocate_exact_size(
                    vec2(ui.available_width(), config.row_height),
                    Sense::click(),
                );
                response.widget_info(|| {
                    WidgetInfo::selected(
                        WidgetType::SelectableLabel,
                        true,
                        selected,
                        format!("{}, {}", entry.name, entry.kind_label()),
                    )
                });
                let rect = slot.shrink2(vec2(6.0, 0.0));
                // Alternate rows carry a faint band, as a native list does.
                if index % 2 == 1 && !selected {
                    ui.painter().rect_filled(rect, 6, band(p));
                }
                row_surface(ui, &response, rect, selected, focused, p);
                let middle = rect.center().y;
                let (icon, ink) = file_icon(&entry, p);
                let icon_size = (config.row_height - 10.0).clamp(14.0, 20.0);
                paint_icon(
                    ui,
                    &entry,
                    icon,
                    ink,
                    Rect::from_center_size(
                        pos2(rect.left() + 16.0, middle),
                        Vec2::splat(icon_size),
                    ),
                    p,
                );
                let text = controls::elided(
                    ui.painter(),
                    &entry.name,
                    theme::regular(13.0),
                    if entry.hidden { p.secondary } else { p.fg },
                    columns.name - 40.0,
                );
                let name =
                    controls::galley_at(ui.painter(), pos2(rect.left() + 32.0, middle), text);
                // The glyph and name pick the file up; the rest of the row
                // starts a rectangle, as in Finder's list view.
                items.push((
                    Rect::from_min_max(slot.min, pos2(name.right() + 6.0, slot.bottom())),
                    index,
                ));
                let mut x = rect.left() + columns.name;
                if columns.date > 0.0 {
                    cell_text(
                        ui,
                        &modified(&entry, today),
                        x + 8.0,
                        middle,
                        columns.date - 16.0,
                        p.secondary,
                    );
                    x += columns.date;
                }
                if columns.size > 0.0 {
                    let size = if entry.is_dir() {
                        "—".into()
                    } else {
                        bytes(entry.size)
                    };
                    ui.painter().text(
                        pos2(x + columns.size - 10.0, middle),
                        Align2::RIGHT_CENTER,
                        size,
                        theme::regular(12.0),
                        if entry.is_dir() { p.muted } else { p.secondary },
                    );
                    x += columns.size;
                }
                if columns.kind > 0.0 {
                    cell_text(
                        ui,
                        &kind(&entry),
                        x + 14.0,
                        middle,
                        columns.kind - 20.0,
                        p.secondary,
                    );
                }
                interact(ui, response, pane, (index, &entry), state, p, actions);
            });
        }
        begin_drag(ui, pane, &background, view.origin, &items);
        if let Some(marquee) = marquee {
            paint_band(ui, area, marquee, p);
        }
    });
    if background.clicked() {
        pane.selected.clear();
    }
    background.context_menu(|ui| folder_menu(ui, pane, state, p, actions));
}

fn band(p: Palette) -> Color32 {
    if p.dark {
        Color32::from_white_alpha(5)
    } else {
        Color32::from_black_alpha(7)
    }
}

#[allow(clippy::too_many_arguments)]
fn header(
    ui: &mut egui::Ui,
    cell: Rect,
    pane: &Pane,
    p: Palette,
    field: SortField,
    label: &str,
    trailing: bool,
    divider: bool,
    actions: &mut Vec<Action>,
) {
    let response = ui.interact(cell, ui.id().with(("sort", label)), Sense::click());
    response
        .widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, format!("Sort by {label}")));
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(cell.shrink2(vec2(2.0, 4.0)), 6, p.hover);
    }
    if response.has_focus() {
        controls::focus_ring(painter, cell.shrink2(vec2(2.0, 4.0)), 6, p);
    }
    if divider {
        painter.line_segment(
            [
                pos2(cell.left(), cell.center().y - 7.0),
                pos2(cell.left(), cell.center().y + 7.0),
            ],
            p.hairline(),
        );
    }
    let sorted = pane.sort.field == field;
    let galley = controls::elided(
        painter,
        label,
        if sorted {
            theme::medium(11.5)
        } else {
            theme::regular(11.5)
        },
        if sorted { p.fg } else { p.secondary },
        cell.width() - 34.0,
    );
    let text_width = galley.size().x;
    let left = if trailing {
        cell.right() - 10.0 - text_width - if sorted { 14.0 } else { 0.0 }
    } else if field == SortField::Name {
        cell.left() + 26.0
    } else {
        cell.left() + 8.0 + if field == SortField::Kind { 6.0 } else { 0.0 }
    };
    controls::galley_at(painter, pos2(left, cell.center().y), galley);
    if sorted {
        icons::paint(
            painter,
            Rect::from_center_size(
                pos2(left + text_width + 9.0, cell.center().y),
                Vec2::splat(10.0),
            ),
            if pane.sort.descending {
                Icon::ChevronDown
            } else {
                Icon::ChevronUp
            },
            p.secondary,
        );
    }
    if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
        actions.push(Action::Sort(pane.id, field));
    }
}

fn grid(
    ui: &mut egui::Ui,
    pane: &mut Pane,
    config: &Config,
    state: &mut UiState,
    p: Palette,
    focused: bool,
    actions: &mut Vec<Action>,
) {
    let body = ui.available_rect_before_wrap();
    let background = files_background(ui, body);
    ui.add_space(6.0);
    let area = ui.available_rect_before_wrap();
    let available = ui.available_width() - 12.0;
    let cell_width = (config.icon_size + 56.0).max(96.0);
    let columns = (available / cell_width).floor().max(1.0) as usize;
    state.grid_columns.insert(pane.id, columns);
    let width = available / columns as f32;
    let height = config.icon_size + 62.0;
    let rows = pane.entries.len().div_ceil(columns);
    let pixels = thumbnails::pixels(config.icon_size, ui.ctx().pixels_per_point());
    let mut scroll = egui::ScrollArea::vertical()
        .id_salt(("files-grid", pane.id, &pane.path, &pane.query))
        .auto_shrink([false, false])
        .scroll_source(FILES_SCROLL);
    if let Some(index) = pane.scroll_to.take() {
        scroll = scroll.vertical_scroll_offset((index / columns) as f32 * height);
    }
    // A rectangle covers an item where it meets the glyph's column, from the
    // glyph down through the name, so off-screen rows need no text layout.
    let tile = grid_tile(
        Rect::from_min_size(pos2(6.0, 0.0), vec2(width, height)),
        config.icon_size,
    );
    scroll.show_rows(ui, height, rows, |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let view = View {
            origin: ui.max_rect().left_top() - vec2(0.0, range.start as f32 * height),
            area,
            height: rows as f32 * height,
        };
        let marquee = sweep(ui, pane, &background, &view, |rect| Band {
            rows: span(
                rect.y_range(),
                height,
                Rangef::new(tile.top(), height - 6.0),
                rows,
            ),
            columns: span(rect.x_range(), width, tile.x_range(), columns),
            stride: columns,
        });
        let mut items = Vec::new();
        for row in range {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(6.0);
                for column in 0..columns {
                    let index = row * columns + column;
                    if index >= pane.entries.len() {
                        break;
                    }
                    let entry = pane.entries[index].clone();
                    let selected = pane.selected.contains(&entry.path);
                    ui.push_id(&entry.path, |ui| {
                        let (_, cell) = ui.allocate_space(vec2(width, height));
                        let item = GridItem::new(
                            ui.painter(),
                            cell,
                            &entry,
                            config,
                            selected && focused,
                            p,
                        );
                        // Only the glyph and name answer the pointer; the
                        // space between items is the folder's.
                        let bounds = item.tile.union(item.capsule);
                        let response = ui.interact(bounds, ui.id().with("item"), Sense::click());
                        response.widget_info(|| {
                            WidgetInfo::selected(
                                WidgetType::SelectableLabel,
                                true,
                                selected,
                                format!("{}, {}", entry.name, entry.kind_label()),
                            )
                        });
                        let thumbnail = state.thumbnails.get(&entry, pixels);
                        item.paint(ui, &response, &entry, thumbnail, selected, focused, p);
                        items.push((bounds, index));
                        interact(ui, response, pane, (index, &entry), state, p, actions);
                    });
                }
            });
        }
        begin_drag(ui, pane, &background, view.origin, &items);
        if let Some(marquee) = marquee {
            paint_band(ui, area, marquee, p);
        }
    });
    if background.clicked() {
        pane.selected.clear();
    }
    background.context_menu(|ui| folder_menu(ui, pane, state, p, actions));
}

fn grid_tile(cell: Rect, size: f32) -> Rect {
    Rect::from_center_size(
        pos2(cell.center().x, cell.top() + 8.0 + size * 0.5 + 6.0),
        Vec2::splat(size + 12.0),
    )
}

/// An icon or thumbnail over its name, as the native icon view draws it:
/// selection backs the picture with a soft tile and sets the name in an
/// accent capsule.
struct GridItem {
    tile: Rect,
    name: Arc<Galley>,
    origin: Pos2,
    capsule: Rect,
}
impl GridItem {
    fn new(
        painter: &egui::Painter,
        cell: Rect,
        entry: &Entry,
        config: &Config,
        accented: bool,
        p: Palette,
    ) -> Self {
        let tile = grid_tile(cell, config.icon_size);
        let ink = if accented {
            p.on_accent
        } else if entry.hidden {
            p.secondary
        } else {
            p.fg
        };
        let mut job = egui::text::LayoutJob::simple(
            entry.name.clone(),
            theme::regular(12.0),
            ink,
            cell.width() - 16.0,
        );
        job.halign = Align::Center;
        job.wrap = egui::text::TextWrapping {
            max_width: cell.width() - 16.0,
            max_rows: 2,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
        let name = painter.layout_job(job);
        let origin = pos2(cell.center().x, tile.bottom() + 6.0);
        let mut capsule = name
            .rect
            .translate(origin.to_vec2())
            .expand2(vec2(6.0, 1.5));
        capsule.min.y -= 0.5;
        Self {
            tile,
            name,
            origin,
            capsule,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint(
        self,
        ui: &egui::Ui,
        response: &Response,
        entry: &Entry,
        thumbnail: Option<&Texture>,
        selected: bool,
        focused: bool,
        p: Palette,
    ) {
        let painter = ui.painter();
        if selected {
            painter.rect_filled(self.tile, 10, p.pressed);
        } else if response.hovered() {
            painter.rect_filled(self.tile, 10, p.hover);
        }
        if response.has_focus() {
            controls::focus_ring(painter, self.tile, 10, p);
        }
        if let Some(thumbnail) = thumbnail {
            paint_thumbnail(ui, entry, thumbnail, self.tile.shrink(6.0), p);
        } else {
            let (icon, ink) = file_icon(entry, p);
            paint_icon(ui, entry, icon, ink, self.tile.shrink(6.0), p);
        }
        if selected {
            painter.rect_filled(self.capsule, 5, if focused { p.accent } else { p.pressed });
        }
        painter.galley(self.origin, self.name, Color32::PLACEHOLDER);
    }
}

/// The files' own surface under the rows: a click clears the selection, a
/// secondary click offers folder actions, and every drag in the file view
/// begins here, so it outlives rows that scroll away. It takes no focus.
fn files_background(ui: &egui::Ui, area: Rect) -> Response {
    ui.interact(
        area,
        ui.id().with("files-background"),
        Sense::CLICK | Sense::DRAG,
    )
}

/// Where a file view's content sits: its scrolled top-left on screen, the
/// visible area and the content's full height.
struct View {
    origin: Pos2,
    area: Rect,
    height: f32,
}

/// Which of `count` items along one axis meet `range`, the `k`th spanning
/// `item` offset by `k * pitch`.
fn span(range: Rangef, pitch: f32, item: Rangef, count: usize) -> Range<usize> {
    let first = ((range.min - item.max) / pitch).ceil().max(0.0) as usize;
    let end = (((range.max - item.min) / pitch).floor() + 1.0).clamp(0.0, count as f32) as usize;
    first.min(end)..end
}

/// Carries a selection rectangle to the pointer before the files paint, so
/// they show what it covers in the same frame, and scrolls while the pointer
/// is past the top or bottom of the files. Returns the rectangle on screen.
fn sweep(
    ui: &egui::Ui,
    pane: &mut Pane,
    background: &Response,
    view: &View,
    cover: impl Fn(Rect) -> Band,
) -> Option<Rect> {
    if !background.dragged() {
        if pane.marquee_active() {
            pane.end_marquee();
            ui.ctx().data_mut(|d| d.remove::<Pos2>(background.id));
        }
        return None;
    }
    let start = ui
        .ctx()
        .data(|d| d.get_temp::<Pos2>(background.id))
        .filter(|_| pane.marquee_active())?;
    let pointer = ui.input(|i| i.pointer.interact_pos())?;
    let band = Rect::from_two_pos(start, pointer - view.origin.to_vec2());
    pane.marquee_to(cover(band));
    // Scroll toward a pointer past an edge, faster the further it is.
    let (top, bottom) = (view.area.top(), view.area.bottom());
    let beyond = (pointer.y - top).min(0.0) + (pointer.y - bottom).max(0.0);
    let offset = top - view.origin.y;
    if beyond < 0.0 && offset > 0.0 || beyond > 0.0 && offset < view.height - view.area.height() {
        let step = beyond.clamp(-120.0, 120.0) * 12.0 * ui.input(|i| i.stable_dt).min(0.1);
        ui.scroll_with_delta_animation(vec2(0.0, -step), ScrollAnimation::none());
    }
    Some(band.translate(view.origin.to_vec2()))
}

/// A primary-button drag that starts on an item's glyph or name moves the
/// selection; one that starts anywhere else in the files draws a selection
/// rectangle.
fn begin_drag(
    ui: &egui::Ui,
    pane: &mut Pane,
    background: &Response,
    origin: Pos2,
    items: &[(Rect, usize)],
) {
    let Some(press) = ui
        .input(|i| i.pointer.press_origin())
        .filter(|_| background.drag_started_by(egui::PointerButton::Primary))
    else {
        return;
    };
    if let Some(&(_, index)) = items.iter().find(|(rect, _)| rect.contains(press)) {
        if !pane.selected.contains(&pane.entries[index].path) {
            pane.select(index, false, false);
        }
        egui::DragAndDrop::set_payload(
            ui.ctx(),
            FileDrag {
                sources: pane.selected.iter().cloned().collect(),
            },
        );
    } else {
        let modifiers = ui.input(|i| i.modifiers);
        pane.begin_marquee(modifiers.shift, modifiers.command || modifiers.ctrl);
        ui.ctx()
            .data_mut(|d| d.insert_temp(background.id, press - origin.to_vec2()));
        ui.ctx().request_repaint();
    }
}

/// The selection rectangle, in the accent reserved for selection.
fn paint_band(ui: &egui::Ui, area: Rect, band: Rect, p: Palette) {
    ui.painter_at(area).rect(
        band,
        4,
        theme::tint(p.accent, if p.dark { 0.16 } else { 0.12 }),
        Stroke::new(1.0, theme::tint(p.accent, 0.8)),
        StrokeKind::Inside,
    );
}

fn row_surface(
    ui: &egui::Ui,
    response: &Response,
    rect: Rect,
    selected: bool,
    focused: bool,
    p: Palette,
) {
    if selected {
        // The pane that takes the keys shows its selection in the accent; the
        // other pane's selection stays, quietly.
        let fill = if focused {
            theme::tint(p.accent, if p.dark { 0.32 } else { 0.2 })
        } else {
            p.pressed
        };
        ui.painter().rect_filled(rect, 6, fill);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 6, p.hover);
    }
    if response.has_focus() {
        controls::focus_ring(ui.painter(), rect, 6, p);
    }
}

/// The icon and ink for an entry. Colour is decoration only; the kind is
/// always also said in words.
pub(super) fn file_icon(entry: &Entry, p: Palette) -> (Icon, Color32) {
    if entry.is_dir() {
        return (Icon::FolderFilled, p.accent);
    }
    let tint = |accent| theme::accent_color(accent, p.dark);
    match extension(&entry.path).as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "tif" | "tiff" | "heic"
        | "ico" => (Icon::Image, tint(Accent::Green)),
        "mp4" | "mov" | "webm" | "mkv" | "avi" | "m4v" => (Icon::Movie, tint(Accent::Purple)),
        "mp3" | "wav" | "ogg" | "flac" | "m4a" | "aac" | "opus" => {
            (Icon::Music, tint(Accent::Pink))
        }
        "zip" | "tar" | "gz" | "tgz" | "xz" | "bz2" | "zst" | "7z" | "rar" | "deb" | "rpm"
        | "dmg" | "iso" => (Icon::FileZip, tint(Accent::Orange)),
        "rs" | "js" | "mjs" | "ts" | "tsx" | "jsx" | "py" | "json" | "toml" | "yaml" | "yml"
        | "html" | "css" | "scss" | "sh" | "zsh" | "bash" | "c" | "h" | "cpp" | "hpp" | "cc"
        | "go" | "java" | "kt" | "swift" | "rb" | "php" | "lua" | "sql" | "xml" | "lock" => {
            (Icon::FileCode, tint(Accent::Indigo))
        }
        "txt" | "md" | "markdown" | "rtf" | "pdf" | "doc" | "docx" | "odt" | "csv" | "log" => {
            (Icon::FileText, p.secondary)
        }
        _ => (Icon::File, p.secondary),
    }
}

pub(super) fn paint_icon(
    ui: &egui::Ui,
    entry: &Entry,
    icon: Icon,
    ink: Color32,
    rect: Rect,
    p: Palette,
) {
    icons::paint(
        ui.painter(),
        rect,
        icon,
        if entry.hidden {
            ink.gamma_multiply(0.55)
        } else {
            ink
        },
    );
    link_badge(ui, entry, rect, p);
}

/// The file's picture centred in the icon's square, never enlarged past its
/// own pixels. Opaque pictures get a hairline edge so a light picture keeps
/// its shape on a light pane.
fn paint_thumbnail(ui: &egui::Ui, entry: &Entry, thumbnail: &Texture, rect: Rect, p: Palette) {
    let pixels_per_point = ui.ctx().pixels_per_point();
    let natural = thumbnail.handle.size_vec2() / pixels_per_point;
    let scale = (rect.width() / natural.x)
        .min(rect.height() / natural.y)
        .min(1.0);
    let size = natural * scale;
    // Whole pixels keep a picture shown at its own size sharp.
    let picture = Rect::from_min_size(
        (rect.center() - size * 0.5).round_to_pixels(pixels_per_point),
        size,
    );
    let tint = if entry.hidden {
        Color32::WHITE.gamma_multiply(0.55)
    } else {
        Color32::WHITE
    };
    let painter = ui.painter();
    painter.add(
        egui::epaint::RectShape::filled(picture, 3, tint).with_texture(
            thumbnail.handle.id(),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        ),
    );
    if thumbnail.opaque {
        painter.rect_stroke(picture, 3, p.hairline(), StrokeKind::Outside);
    }
    // The badge sits on the picture's corner, at the icon's badge size.
    link_badge(
        ui,
        entry,
        Rect::from_min_max(picture.right_bottom() - rect.size(), picture.right_bottom()),
        p,
    );
}

fn link_badge(ui: &egui::Ui, entry: &Entry, rect: Rect, p: Palette) {
    if matches!(entry.kind, EntryKind::Symlink | EntryKind::DirectoryLink) {
        let badge = Rect::from_center_size(
            rect.right_bottom() - vec2(rect.width() * 0.12, rect.height() * 0.12),
            Vec2::splat(rect.width() * 0.5),
        );
        ui.painter()
            .circle_filled(badge.center(), badge.width() * 0.42, p.bg);
        icons::paint(
            ui.painter(),
            badge.shrink(badge.width() * 0.12),
            Icon::ArrowUpRight,
            p.fg,
        );
    }
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// What an entry is, in words a person uses.
pub(super) fn kind(entry: &Entry) -> String {
    match entry.kind {
        EntryKind::Directory => return "Folder".into(),
        EntryKind::DirectoryLink => return "Folder alias".into(),
        EntryKind::Symlink => return "Alias".into(),
        EntryKind::Other => return "Other".into(),
        EntryKind::File => {}
    }
    let extension = extension(&entry.path);
    let known = match extension.as_str() {
        "png" => "PNG image",
        "jpg" | "jpeg" => "JPEG image",
        "gif" => "GIF image",
        "webp" => "WebP image",
        "svg" => "SVG image",
        "bmp" => "BMP image",
        "heic" => "HEIC image",
        "tif" | "tiff" => "TIFF image",
        "ico" => "Icon image",
        "mp4" | "m4v" => "MPEG-4 video",
        "mov" => "QuickTime movie",
        "webm" => "WebM video",
        "mkv" => "Matroska video",
        "avi" => "AVI video",
        "mp3" => "MP3 audio",
        "m4a" | "aac" => "AAC audio",
        "wav" => "WAVE audio",
        "flac" => "FLAC audio",
        "ogg" | "opus" => "Ogg audio",
        "txt" => "Plain text",
        "md" | "markdown" => "Markdown document",
        "pdf" => "PDF document",
        "rtf" => "Rich text document",
        "doc" | "docx" => "Word document",
        "odt" => "OpenDocument text",
        "xls" | "xlsx" => "Excel spreadsheet",
        "ods" => "OpenDocument spreadsheet",
        "ppt" | "pptx" => "PowerPoint presentation",
        "csv" => "CSV document",
        "log" => "Log file",
        "json" => "JSON document",
        "toml" => "TOML document",
        "yaml" | "yml" => "YAML document",
        "xml" => "XML document",
        "html" | "htm" => "HTML document",
        "css" | "scss" => "Stylesheet",
        "rs" => "Rust source",
        "py" => "Python script",
        "js" | "mjs" | "jsx" => "JavaScript",
        "ts" | "tsx" => "TypeScript",
        "sh" | "bash" | "zsh" => "Shell script",
        "c" => "C source",
        "h" | "hpp" => "C header",
        "cpp" | "cc" => "C++ source",
        "go" => "Go source",
        "java" => "Java source",
        "kt" => "Kotlin source",
        "swift" => "Swift source",
        "rb" => "Ruby script",
        "php" => "PHP script",
        "lua" => "Lua script",
        "sql" => "SQL script",
        "lock" => "Lockfile",
        "zip" => "ZIP archive",
        "tar" => "Tar archive",
        "gz" | "tgz" => "Gzip archive",
        "xz" => "XZ archive",
        "bz2" => "Bzip2 archive",
        "zst" => "Zstandard archive",
        "7z" => "7-Zip archive",
        "rar" => "RAR archive",
        "deb" => "Debian package",
        "rpm" => "RPM package",
        "dmg" => "Disk image",
        "iso" => "Disc image",
        "appimage" => "Application",
        "exe" | "msi" => "Windows application",
        _ => "",
    };
    if !known.is_empty() {
        known.into()
    } else if extension.is_empty() || extension.len() > 8 {
        "Document".into()
    } else {
        format!("{} file", extension.to_ascii_uppercase())
    }
}

fn cell_text(ui: &egui::Ui, text: &str, left: f32, middle: f32, width: f32, color: Color32) {
    let text = controls::elided(ui.painter(), text, theme::regular(12.0), color, width);
    controls::galley_at(ui.painter(), pos2(left, middle), text);
}

/// "Today at 14:02", "Yesterday at 09:13", or the full date.
fn modified(entry: &Entry, today: chrono::NaiveDate) -> String {
    entry
        .modified
        .map(|time| {
            let date: chrono::DateTime<chrono::Local> = time.into();
            let day = date.date_naive();
            if day == today {
                date.format("Today at %H:%M").to_string()
            } else if today.pred_opt() == Some(day) {
                date.format("Yesterday at %H:%M").to_string()
            } else {
                date.format("%b %-d, %Y at %H:%M").to_string()
            }
        })
        .unwrap_or_else(|| "—".into())
}

fn interact(
    ui: &mut egui::Ui,
    response: Response,
    pane: &mut Pane,
    item: (usize, &Entry),
    state: &mut UiState,
    p: Palette,
    actions: &mut Vec<Action>,
) {
    let (index, entry) = item;
    state.file_widgets.insert(response.id);
    state.file_owners.insert(response.id, pane.id);
    if response.clicked() {
        let modifiers = ui.input(|i| i.modifiers);
        pane.select(index, modifiers.command || modifiers.ctrl, modifiers.shift);
    }
    if response.secondary_clicked() && !pane.selected.contains(&entry.path) {
        pane.select(index, false, false);
    }
    if response.double_clicked() {
        actions.push(Action::Open(pane.id, entry.path.clone(), entry.is_dir()));
    }
    // A middle click opens a folder in a new tab, or a file as usual.
    if response.middle_clicked() {
        actions.push(if entry.is_dir() {
            Action::NewTab(entry.path.clone())
        } else {
            Action::Open(pane.id, entry.path.clone(), false)
        });
    }
    if entry.is_dir() {
        if response
            .dnd_hover_payload::<FileDrag>()
            .is_some_and(|payload| accepts_drop(&payload.sources, &entry.path))
        {
            ui.painter().rect_filled(
                response.rect.shrink2(vec2(6.0, 0.0)),
                6,
                theme::tint(p.accent, 0.18),
            );
            ui.painter().rect_stroke(
                response.rect.shrink2(vec2(6.0, 0.0)),
                6,
                Stroke::new(1.5, p.accent),
                StrokeKind::Inside,
            );
        }
        // Releasing over a folder always takes the drag, so a refused drop
        // cannot fall through to the pane's own folder.
        if let Some(payload) = response.dnd_release_payload::<FileDrag>()
            && let Some(operation) = transfer(
                &payload.sources,
                &entry.path,
                ui.input(|i| i.modifiers.shift),
            )
        {
            actions.push(Action::Submit(operation));
        }
    }
    response.clone().on_hover_text(if pane.recursive {
        entry.path.display().to_string()
    } else {
        entry.name.clone()
    });
    response.context_menu(|ui| {
        controls::menu_layout(ui, 236.0);
        if controls::menu_item(
            ui,
            p,
            if entry.is_dir() {
                Icon::FolderOpen
            } else {
                Icon::ExternalLink
            },
            "Open",
            "Enter",
            false,
        ) {
            actions.push(Action::Open(pane.id, entry.path.clone(), entry.is_dir()));
            ui.close();
        }
        if entry.is_dir() && controls::menu_item(ui, p, Icon::Plus, "Open in new tab", "", false) {
            actions.push(Action::NewTab(entry.path.clone()));
            ui.close();
        }
        if controls::menu_item(ui, p, Icon::Eyes, "Quick Look", "Space", false) {
            actions.push(Action::QuickLook(entry.path.clone()));
            ui.close();
        }
        if controls::menu_item(
            ui,
            p,
            Icon::ArrowUpRight,
            "Show in system file manager",
            "",
            false,
        ) {
            actions.push(Action::Submit(Operation::Reveal {
                path: entry.path.clone(),
            }));
            ui.close();
        }
        controls::menu_separator(ui, p);
        if controls::menu_item(
            ui,
            p,
            Icon::Scissors,
            "Cut",
            &controls::edit_shortcut("X"),
            false,
        ) {
            actions.push(Action::Copy(pane.selected.iter().cloned().collect(), true));
            ui.close();
        }
        if controls::menu_item(
            ui,
            p,
            Icon::Copy,
            "Copy",
            &controls::edit_shortcut("C"),
            false,
        ) {
            actions.push(Action::Copy(pane.selected.iter().cloned().collect(), false));
            ui.close();
        }
        if state.can_paste
            && controls::menu_item(
                ui,
                p,
                Icon::Clipboard,
                "Paste",
                &controls::edit_shortcut("V"),
                false,
            )
        {
            actions.push(Action::Paste(pane.path.clone()));
            ui.close();
        }
        if controls::menu_item(ui, p, Icon::Copy, "Copy path", "", false) {
            actions.push(Action::CopyPath(entry.path.clone()));
            ui.close();
        }
        controls::menu_separator(ui, p);
        if pane.selected.len() == 1
            && controls::menu_item(ui, p, Icon::Pencil, "Rename…", "F2", false)
        {
            state.open_dialog(Dialog::Rename {
                source: entry.path.clone(),
                name: entry.name.clone(),
            });
            ui.close();
        }
        if entry.is_dir() && controls::menu_item(ui, p, Icon::Pin, "Pin to sidebar", "", false) {
            actions.push(Action::Favorite(entry.path.clone()));
            ui.close();
        }
        if controls::menu_item(ui, p, Icon::Trash, "Move to Trash…", "Delete", true) {
            state.open_dialog(Dialog::Trash {
                paths: pane.selected.iter().cloned().collect(),
            });
            ui.close();
        }
        controls::menu_separator(ui, p);
        if controls::menu_item(
            ui,
            p,
            Icon::Info,
            "Properties",
            properties_shortcut(),
            false,
        ) {
            actions.push(Action::Properties(pane.id, entry.clone()));
            ui.close();
        }
    });
}

/// Whether dropping these items on `destination` would do anything. A folder
/// is no target for itself or anything inside it, and items already in the
/// destination stay where they are, so a drag released where it began (as a
/// slow click is) does nothing rather than reporting a collision.
fn accepts_drop(sources: &[PathBuf], destination: &Path) -> bool {
    !sources.iter().any(|source| destination.starts_with(source))
        && sources.iter().any(|source| !lives_in(source, destination))
}

fn lives_in(source: &Path, folder: &Path) -> bool {
    source.parent() == Some(folder)
}

/// The operation a drop on `destination` asks for, without the items that
/// are already there; `None` when it would do nothing.
fn transfer(sources: &[PathBuf], destination: &Path, moving: bool) -> Option<Operation> {
    if !accepts_drop(sources, destination) {
        return None;
    }
    let sources = sources
        .iter()
        .filter(|source| !lives_in(source, destination))
        .cloned()
        .collect();
    let destination = destination.to_owned();
    Some(if moving {
        Operation::Move {
            sources,
            destination,
        }
    } else {
        Operation::Copy {
            sources,
            destination,
        }
    })
}

fn blank_menu(
    ui: &mut egui::Ui,
    rect: Rect,
    pane: &mut Pane,
    state: &mut UiState,
    p: Palette,
    actions: &mut Vec<Action>,
) {
    let response = ui.interact(rect, ui.id().with("blank-folder"), Sense::click());
    if response.clicked() {
        pane.selected.clear();
    }
    response.context_menu(|ui| folder_menu(ui, pane, state, p, actions));
}

fn folder_menu(
    ui: &mut egui::Ui,
    pane: &Pane,
    state: &mut UiState,
    p: Palette,
    actions: &mut Vec<Action>,
) {
    controls::menu_layout(ui, 220.0);
    if controls::menu_item(
        ui,
        p,
        Icon::FolderPlus,
        "New folder…",
        &controls::edit_shortcut("Shift+N"),
        false,
    ) {
        state.open_dialog(Dialog::New {
            parent: pane.path.clone(),
            name: String::new(),
            folder: true,
        });
        ui.close();
    }
    if controls::menu_item(ui, p, Icon::FilePlus, "New file…", "", false) {
        state.open_dialog(Dialog::New {
            parent: pane.path.clone(),
            name: String::new(),
            folder: false,
        });
        ui.close();
    }
    if state.can_paste
        && controls::menu_item(
            ui,
            p,
            Icon::Clipboard,
            "Paste",
            &controls::edit_shortcut("V"),
            false,
        )
    {
        actions.push(Action::Paste(pane.path.clone()));
        ui.close();
    }
    controls::menu_separator(ui, p);
    if controls::menu_item(ui, p, Icon::Pin, "Pin to sidebar", "", false) {
        actions.push(Action::Favorite(pane.path.clone()));
        ui.close();
    }
    if controls::menu_item(ui, p, Icon::Refresh, "Refresh", "F5", false) {
        actions.push(Action::ReloadPane(pane.id));
        ui.close();
    }
    controls::menu_separator(ui, p);
    if controls::menu_item(ui, p, Icon::Info, "Properties", "", false) {
        actions.push(Action::Properties(pane.id, folder_entry(pane)));
        ui.close();
    }
}

/// A quiet statement in the middle of an empty pane, as Terminal says that a
/// search found nothing. Returns whether its one action was pressed.
fn message(
    ui: &mut egui::Ui,
    rect: Rect,
    p: Palette,
    icon: Icon,
    title: &str,
    detail: &str,
    action: Option<&str>,
) -> bool {
    let center = rect.center() - vec2(0.0, if action.is_some() { 30.0 } else { 18.0 });
    // Painted text, named for assistive technology as one statement.
    ui.interact(
        Rect::from_center_size(center, vec2(rect.width().min(360.0), 24.0)),
        ui.id().with(("message", title)),
        Sense::hover(),
    )
    .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, format!("{title}. {detail}")));
    let painter = ui.painter();
    icons::paint(
        painter,
        Rect::from_center_size(center - vec2(0.0, 34.0), Vec2::splat(28.0)),
        icon,
        p.muted,
    );
    painter.text(
        center,
        Align2::CENTER_CENTER,
        title,
        theme::medium(14.0),
        p.fg,
    );
    // Centre each line of a wrapped explanation.
    let mut job = egui::text::LayoutJob::simple(
        detail.to_owned(),
        theme::regular(12.0),
        p.secondary,
        (rect.width() - 48.0).clamp(80.0, 360.0),
    );
    job.halign = Align::Center;
    let detail = painter.layout_job(job);
    let detail_rect = Rect::from_center_size(
        center + vec2(0.0, 14.0 + detail.size().y * 0.5),
        detail.size(),
    );
    painter.galley(
        pos2(center.x, detail_rect.top()),
        detail,
        Color32::PLACEHOLDER,
    );
    let Some(label) = action else {
        return false;
    };
    let button = Rect::from_center_size(
        pos2(center.x, detail_rect.bottom() + 30.0),
        vec2(120.0, metrics::CONTROL_HEIGHT),
    );
    controls::place(
        ui,
        button,
        Layout::top_down(Align::Center),
        ("message-action", label),
        |ui| controls::button(ui, p, label, ButtonKind::Secondary).clicked(),
    )
}

fn loading(ui: &mut egui::Ui, rect: Rect, p: Palette) {
    let angle = ui.input(|input| input.time) as f32 * std::f32::consts::TAU;
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(16));
    let center = rect.center() - vec2(0.0, 18.0);
    ui.interact(
        Rect::from_center_size(center, vec2(120.0, 24.0)),
        ui.id().with("loading"),
        Sense::hover(),
    )
    .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, "Loading…"));
    icons::paint_rotated(
        ui.painter(),
        Rect::from_center_size(center - vec2(0.0, 16.0), Vec2::splat(20.0)),
        Icon::Loader2,
        p.muted,
        angle,
    );
    ui.painter().text(
        center + vec2(0.0, 12.0),
        Align2::CENTER_CENTER,
        "Loading…",
        theme::regular(12.0),
        p.secondary,
    );
}
