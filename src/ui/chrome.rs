//! Window chrome in Neptune Terminal's arrangement: a full-height source list,
//! a toolbar with a path bar for the folder in view, and a tab strip that appears only
//! when there is more than one tab to tell apart.
use super::{Action, Dialog, UiState, breadcrumbs, name};
use crate::{
    config::{Config, ViewMode},
    controls,
    icons::{self, Icon},
    model::{Pane, Workspace},
    theme::{self, Palette, metrics},
};
use eframe::egui::{
    self, Align, Color32, CursorIcon, Layout, Pos2, Rect, Sense, Stroke, Ui, Vec2, WidgetInfo,
    WidgetType, pos2, vec2,
};
use std::path::{Path, PathBuf};

pub const SIDEBAR_WIDTH: (f32, f32) = Config::SIDEBAR_RANGE;
/// Centre of the sidebar toggle while the sidebar is hidden: beside the
/// window controls, exactly where Terminal keeps it.
pub const TOGGLE_HIDDEN: f32 = 94.0;
const ROW_HEIGHT: f32 = 30.0;

fn drag_region(ui: &mut Ui, rect: Rect, name: &str) {
    let drag = ui.interact(rect, ui.id().with(name), Sense::click_and_drag());
    if drag.drag_started() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if drag.double_clicked() {
        toggle_maximized(ui.ctx());
    }
}

fn toggle_maximized(ctx: &egui::Context) {
    let maximized = ctx.input(|input| input.viewport().maximized.unwrap_or(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
}

fn icon_at(
    ui: &mut Ui,
    center: Pos2,
    icon: Icon,
    label: &str,
    hint: &str,
    enabled: bool,
) -> egui::Response {
    controls::place(
        ui,
        Rect::from_center_size(center, Vec2::splat(28.0)),
        Layout::left_to_right(Align::Center),
        ("chrome-icon", label),
        |ui| {
            ui.add_enabled_ui(enabled, |ui| icons::button_with_hint(ui, icon, label, hint))
                .inner
        },
    )
}

/// An icon button that stays pressed while what it controls is on, as
/// Terminal's panel toggle does.
fn toggle_button(ui: &mut Ui, icon: Icon, label: &str, hint: &str, on: bool, p: Palette) -> bool {
    let surface = ui.painter().add(egui::Shape::Noop);
    let response = icons::button_with_hint(ui, icon, label, hint);
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, on, label));
    if on {
        ui.painter().set(
            surface,
            egui::Shape::rect_filled(response.rect.shrink(1.0), 7, p.pressed),
        );
    }
    response.clicked()
}

/// The window controls and the sidebar toggle. They are drawn over the
/// sidebar and toolbar, so the toggle can travel between them as it slides.
#[allow(clippy::too_many_arguments)]
pub fn leading_controls(
    ui: &mut Ui,
    bounds: Rect,
    p: Palette,
    config: &mut Config,
    available: bool,
    reveal: f32,
    width: f32,
    actions: &mut Vec<Action>,
) {
    let middle = bounds.top() + metrics::TOOLBAR_HEIGHT * 0.5;
    window_controls(ui, pos2(bounds.left() + 16.0, middle), p, actions);
    if available
        && icon_at(
            ui,
            pos2(
                bounds.left() + egui::lerp(TOGGLE_HIDDEN..=width - 22.0, reveal).round(),
                middle,
            ),
            Icon::Sidebar,
            "Toggle sidebar",
            &controls::shortcut("B"),
            true,
        )
        .clicked()
    {
        config.sidebar = !config.sidebar;
        actions.push(Action::PreferencesChanged);
    }
}

/// Neptune Terminal's lights, hover reveal, inactive-window material and
/// glyph geometry. Finder sends native viewport operations directly.
pub fn window_controls(ui: &mut Ui, left_center: Pos2, p: Palette, actions: &mut Vec<Action>) {
    let group = Rect::from_min_max(
        left_center + vec2(-6.0, -12.0),
        left_center + vec2(58.0, 12.0),
    );
    let hovered = ui.rect_contains_pointer(group);
    let active = ui.input(|input| input.focused);
    let reveal = controls::animate(ui.ctx(), ui.id().with("window-controls"), hovered, 0.12);
    for (index, (label, rgb)) in [
        ("Close window", 0xff5f57),
        ("Minimize", 0xfebc2e),
        ("Maximize", 0x28c840),
    ]
    .into_iter()
    .enumerate()
    {
        let center = left_center + vec2(6.0 + index as f32 * 20.0, 0.0);
        let response = ui.interact(
            Rect::from_center_size(center, Vec2::splat(20.0)),
            ui.id().with(("window-control", label)),
            Sense::click(),
        );
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));
        let color = theme::color(rgb);
        let painter = ui.painter();
        let fill = if active || hovered {
            color
        } else {
            theme::mix(p.chrome, p.muted, 0.55)
        };
        painter.circle_filled(
            center,
            6.0,
            if response.is_pointer_button_down_on() {
                theme::mix(fill, Color32::BLACK, 0.2)
            } else {
                fill
            },
        );
        painter.circle_stroke(center, 6.0, Stroke::new(0.5, Color32::from_black_alpha(48)));
        if response.has_focus() {
            painter.circle_stroke(center, 8.5, Stroke::new(1.5, theme::tint(p.accent, 0.8)));
        }
        if reveal > 0.0 {
            let (icon, side) = match index {
                0 => (Icon::Close, 10.3),
                1 => (Icon::Minus, 10.8),
                _ => (Icon::Plus, 10.8),
            };
            icons::paint(
                painter,
                Rect::from_center_size(center, Vec2::splat(side)),
                icon,
                theme::tint(theme::mix(color, Color32::BLACK, 0.62), reveal),
            );
        }
        if response.clicked() {
            match index {
                0 => actions.push(Action::WindowClose),
                1 => ui
                    .ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
                _ => toggle_maximized(ui.ctx()),
            }
        }
        response.on_hover_text(label);
    }
}

/// The source list: places, pinned folders and locations, with "New folder"
/// and Preferences in its footer. `settled` is false while it slides.
#[allow(clippy::too_many_arguments)]
pub fn sidebar(
    ui: &mut Ui,
    rect: Rect,
    workspace: &Workspace,
    config: &mut Config,
    state: &mut UiState,
    p: Palette,
    settled: bool,
    actions: &mut Vec<Action>,
) {
    let strip = Rect::from_min_size(rect.min, vec2(rect.width(), metrics::TOOLBAR_HEIGHT));
    drag_region(ui, strip, "sidebar-drag");
    let footer_top = rect.bottom() - 46.0;
    let viewport = Rect::from_min_max(
        pos2(rect.left() + 8.0, strip.bottom() + 2.0),
        pos2(rect.right() - 8.0, footer_top - 2.0),
    );
    let current = workspace.pane().path.clone();
    let id = workspace.pane().id;
    controls::place(
        ui,
        viewport,
        Layout::top_down(Align::Min),
        "places-list",
        |ui| {
            egui::ScrollArea::vertical()
                .id_salt("places-scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    section(ui, p, "Favorites", true);
                    for (icon, label, path) in &state.places {
                        let response = ui
                            .push_id(("place", path), |ui| {
                                place_row(ui, p, *icon, label, &current == path)
                            })
                            .inner;
                        if response.clicked() {
                            actions.push(Action::Navigate(id, path.clone()));
                        } else if response.middle_clicked() {
                            actions.push(Action::NewTab(path.clone()));
                        }
                        response
                            .on_hover_text(path.display().to_string())
                            .context_menu(|ui| {
                                controls::menu_layout(ui, 200.0);
                                if controls::menu_item(
                                    ui,
                                    p,
                                    Icon::Plus,
                                    "Open in new tab",
                                    "",
                                    false,
                                ) {
                                    actions.push(Action::NewTab(path.clone()));
                                    ui.close();
                                }
                                if controls::menu_item(ui, p, Icon::Copy, "Copy path", "", false) {
                                    actions.push(Action::CopyPath(path.clone()));
                                    ui.close();
                                }
                            });
                    }
                    if !config.favorites.is_empty() {
                        section(ui, p, "Pinned", false);
                    }
                    for path in &config.favorites {
                        let response = ui
                            .push_id(("favorite", path), |ui| {
                                place_row(ui, p, Icon::Folder, &name(path), &current == path)
                            })
                            .inner;
                        if response.clicked() {
                            actions.push(Action::Navigate(id, path.clone()));
                        } else if response.middle_clicked() {
                            actions.push(Action::NewTab(path.clone()));
                        }
                        response
                            .on_hover_text(path.display().to_string())
                            .context_menu(|ui| {
                                controls::menu_layout(ui, 200.0);
                                if controls::menu_item(
                                    ui,
                                    p,
                                    Icon::Plus,
                                    "Open in new tab",
                                    "",
                                    false,
                                ) {
                                    actions.push(Action::NewTab(path.clone()));
                                    ui.close();
                                }
                                if controls::menu_item(ui, p, Icon::Copy, "Copy path", "", false) {
                                    actions.push(Action::CopyPath(path.clone()));
                                    ui.close();
                                }
                                controls::menu_separator(ui, p);
                                if controls::menu_item(ui, p, Icon::Pin, "Unpin folder", "", false)
                                {
                                    actions.push(Action::Favorite(path.clone()));
                                    ui.close();
                                }
                            });
                    }
                    section(ui, p, "Locations", false);
                    let root = filesystem_root(&state.home);
                    let response = ui
                        .push_id("filesystem", |ui| {
                            place_row(ui, p, Icon::Computer, "File system", current == root)
                        })
                        .inner;
                    if response.clicked() {
                        actions.push(Action::Navigate(id, root));
                    } else if response.middle_clicked() {
                        actions.push(Action::NewTab(root));
                    }
                    ui.add_space(8.0);
                });
        },
    );

    // Footer: the primary creation action, with Preferences beside it.
    let create = Rect::from_min_max(
        pos2(rect.left() + 8.0, footer_top + 8.0),
        pos2(rect.right() - 44.0, footer_top + 38.0),
    );
    let response = ui.interact(create, ui.id().with("sidebar-new-folder"), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "New folder"));
    let painter = ui.painter();
    if response.is_pointer_button_down_on() {
        painter.rect_filled(create, metrics::ROW_RADIUS, p.pressed);
    } else if response.hovered() {
        painter.rect_filled(create, metrics::ROW_RADIUS, p.hover);
    }
    if response.has_focus() {
        controls::focus_ring(painter, create, metrics::ROW_RADIUS, p);
    }
    let ink = if response.hovered() {
        p.fg
    } else {
        p.secondary
    };
    icons::paint(
        painter,
        Rect::from_center_size(
            pos2(create.left() + 15.0, create.center().y),
            Vec2::splat(14.0),
        ),
        Icon::FolderPlus,
        ink,
    );
    controls::galley_at(
        painter,
        pos2(create.left() + 31.0, create.center().y),
        controls::elided(
            painter,
            "New folder",
            theme::medium(12.5),
            ink,
            create.width() - 36.0,
        ),
    );
    if response
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(format!(
            "New folder   {}",
            controls::edit_shortcut("Shift+N")
        ))
        .clicked()
    {
        state.open_dialog(Dialog::New {
            parent: current.clone(),
            name: String::new(),
            folder: true,
        });
    }
    if icon_at(
        ui,
        pos2(rect.right() - 24.0, create.center().y),
        Icon::Settings,
        "Preferences",
        &controls::edit_shortcut(","),
        true,
    )
    .clicked()
    {
        state.open_dialog(Dialog::Preferences);
    }

    // The trailing edge resizes the sidebar once it rests there; the width
    // is saved on release, and a double click restores the default.
    if !settled {
        return;
    }
    let handle = Rect::from_min_max(
        pos2(rect.right() - 3.0, strip.bottom()),
        pos2(rect.right() + 3.0, rect.bottom() - 12.0),
    );
    let resize = ui
        .interact(
            handle,
            ui.id().with("sidebar-resize"),
            Sense::click_and_drag(),
        )
        .on_hover_and_drag_cursor(CursorIcon::ResizeHorizontal);
    resize.widget_info(|| WidgetInfo::labeled(WidgetType::ResizeHandle, true, "Resize sidebar"));
    if resize.dragged()
        && let Some(pointer) = resize.interact_pointer_pos()
    {
        state.sidebar_drag =
            Some((pointer.x - rect.left()).clamp(SIDEBAR_WIDTH.0, SIDEBAR_WIDTH.1));
    }
    if resize.drag_stopped()
        && let Some(width) = state.sidebar_drag.take()
    {
        config.sidebar_width = width.round();
        actions.push(Action::PreferencesChanged);
    }
    if resize.double_clicked() || resize.triple_clicked() {
        state.sidebar_drag = None;
        config.sidebar_width = Config::SIDEBAR_WIDTH;
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
                pos2(rect.right(), strip.bottom() + 8.0),
                pos2(rect.right(), rect.bottom() - 20.0),
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

fn filesystem_root(home: &Path) -> PathBuf {
    home.ancestors().last().unwrap_or(home).to_owned()
}

/// A muted heading over a group of rows in the source list.
fn section(ui: &mut Ui, p: Palette, title: &str, first: bool) {
    let (_, rect) = ui.allocate_space(vec2(ui.available_width(), if first { 24.0 } else { 34.0 }));
    ui.painter().text(
        pos2(rect.left() + 10.0, rect.bottom() - 9.0),
        egui::Align2::LEFT_CENTER,
        title,
        theme::medium(11.5),
        p.muted,
    );
}

/// A source-list row, marked as Terminal marks a selected workspace.
fn place_row(ui: &mut Ui, p: Palette, icon: Icon, label: &str, selected: bool) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    response.widget_info(|| {
        WidgetInfo::selected(
            WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            label,
        )
    });
    let painter = ui.painter();
    if selected || response.is_pointer_button_down_on() {
        painter.rect_filled(rect, metrics::ROW_RADIUS, p.pressed);
    } else if response.hovered() {
        painter.rect_filled(rect, metrics::ROW_RADIUS, p.hover);
    }
    if response.has_focus() {
        controls::focus_ring(painter, rect.shrink(2.0), metrics::ROW_RADIUS - 2, p);
    }
    let ink = if selected || response.hovered() {
        p.fg
    } else {
        p.secondary
    };
    icons::paint(
        painter,
        Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), Vec2::splat(16.0)),
        icon,
        if selected { p.accent } else { ink },
    );
    controls::galley_at(
        painter,
        pos2(rect.left() + 34.0, rect.center().y),
        controls::elided(
            painter,
            label,
            theme::medium(13.0),
            ink,
            rect.width() - 42.0,
        ),
    );
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// The toolbar shows the folder in view as a path bar and holds navigation, search, views
/// and window-wide actions. `leading` is where its own content may begin.
#[allow(clippy::too_many_arguments)]
pub fn toolbar(
    ui: &mut Ui,
    rect: Rect,
    leading: f32,
    workspace: &mut Workspace,
    config: &mut Config,
    state: &mut UiState,
    p: Palette,
    actions: &mut Vec<Action>,
) {
    drag_region(ui, rect, "toolbar-drag");
    let middle = rect.center().y;
    let left = leading.max(rect.left() + 8.0);
    let pane = workspace.pane();
    if icon_at(
        ui,
        pos2(left + 14.0, middle),
        Icon::ChevronLeft,
        "Back",
        "Alt+Left",
        pane.history_index > 0,
    )
    .clicked()
    {
        actions.push(Action::Back(pane.id, false));
    }
    if icon_at(
        ui,
        pos2(left + 44.0, middle),
        Icon::ChevronRight,
        "Forward",
        "Alt+Right",
        pane.history_index + 1 < pane.history.len(),
    )
    .clicked()
    {
        actions.push(Action::Back(pane.id, true));
    }
    let title_left = left + 68.0;
    let split = workspace.tabs[workspace.active].panes.len() > 1;
    let roomy = rect.right() - title_left >= 520.0;
    let cluster = controls::place(
        ui,
        Rect::from_min_max(
            pos2(title_left, rect.top() + 8.0),
            pos2(rect.right() - 8.0, rect.bottom() - 8.0),
        ),
        Layout::right_to_left(Align::Center),
        "toolbar-actions",
        |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let menu = icons::button(ui, Icon::Ellipsis, "More actions");
            more_menu(&menu, workspace, config, state, p, roomy, actions);
            if roomy {
                if toggle_button(
                    ui,
                    Icon::SplitVertical,
                    "Toggle split view",
                    &controls::edit_shortcut("\\"),
                    split,
                    p,
                ) {
                    actions.push(Action::ToggleSplit);
                }
                ui.add_space(6.0);
                if toggle_button(
                    ui,
                    Icon::Grid,
                    "Icon view",
                    &controls::edit_shortcut("1"),
                    config.view == ViewMode::Grid,
                    p,
                ) {
                    actions.push(Action::View(ViewMode::Grid));
                }
                if toggle_button(
                    ui,
                    Icon::List,
                    "List view",
                    &controls::edit_shortcut("2"),
                    config.view == ViewMode::List,
                    p,
                ) {
                    actions.push(Action::View(ViewMode::List));
                }
                ui.add_space(6.0);
            }
            if icons::button_with_hint(ui, Icon::Plus, "New tab", &controls::edit_shortcut("T"))
                .clicked()
            {
                actions.push(Action::NewTab(workspace.pane().path.clone()));
            }
            ui.min_rect().left()
        },
    );
    let right = cluster - 10.0;

    // The search field trails the toolbar, so the path bar keeps the room
    // between navigation and the actions.
    let height = metrics::CONTROL_HEIGHT;
    let width = (rect.width() * 0.26)
        .clamp(180.0, 260.0)
        .min(right - title_left - 160.0);
    let search = (width >= 100.0).then(|| {
        Rect::from_min_max(
            pos2(right - width, middle - height * 0.5),
            pos2(right, middle + height * 0.5),
        )
    });
    let bar = Rect::from_min_max(
        pos2(title_left, middle - height * 0.5),
        pos2(
            search.map_or(right, |field| field.left() - 8.0),
            middle + height * 0.5,
        ),
    );
    let pane = workspace.pane_mut();
    if let Some(field) = search {
        let response = controls::place(
            ui,
            field,
            Layout::left_to_right(Align::Center),
            ("toolbar-search", pane.id),
            |ui| {
                controls::search_field(
                    ui,
                    p,
                    egui::Id::new(("search", pane.id)),
                    &mut pane.query,
                    "Search",
                    "Search files",
                    field.width(),
                )
            },
        );
        if pane.query.is_empty() && !response.has_focus() {
            controls::keycaps(
                ui.painter(),
                pos2(field.right() - 6.0, middle),
                &controls::edit_shortcut("F"),
                p.muted,
            );
        }
        if state.focus_search {
            response.request_focus();
            state.focus_search = false;
        }
        if response.changed() {
            actions.push(Action::Search(pane.id));
        }
        if response.has_focus()
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            pane.query.clear();
            response.surrender_focus();
            actions.push(Action::Search(pane.id));
        }
    }
    // A location being typed belongs to the folder it began in.
    state.path_bar = bar.width() >= 120.0;
    if state
        .location
        .as_ref()
        .is_some_and(|edit| !state.path_bar || edit.pane != pane.id || edit.path != pane.path)
    {
        state.location = None;
    }
    if state.location.is_some() {
        location_field(ui, bar, state, p, actions);
    } else if state.path_bar {
        path_bar(ui, bar, pane, state, p, actions);
    } else if bar.width() >= 40.0 {
        controls::galley_at(
            ui.painter(),
            pos2(title_left, middle),
            controls::elided(
                ui.painter(),
                &name(&pane.path),
                theme::medium(13.0),
                p.fg,
                bar.width(),
            ),
        );
    }
}

/// The folder in view as a path: each enclosing folder opens with a click,
/// and a click anywhere else turns the bar into an editable location.
fn path_bar(
    ui: &mut Ui,
    rect: Rect,
    pane: &Pane,
    state: &mut UiState,
    p: Palette,
    actions: &mut Vec<Action>,
) {
    const SEPARATOR: f32 = 14.0;
    const MORE: f32 = 28.0;
    let background = ui.interact(rect, ui.id().with("path-bar"), Sense::click());
    background.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "Edit location"));
    controls::field_frame(ui, p, rect, location_id(pane.id), true);
    let painter = ui.painter().clone();
    if background.has_focus() {
        controls::focus_ring(&painter, rect, metrics::CONTROL_RADIUS, p);
    }
    let middle = rect.center().y;
    let parts = breadcrumbs(&pane.path, &state.home);
    let style = |last: bool| {
        if last {
            (theme::medium(13.0), p.fg)
        } else {
            (theme::regular(13.0), p.secondary)
        }
    };
    // Measure every crumb, then keep as many trailing ones as fit; the
    // folders that do not fit wait behind a menu at the leading edge.
    let measured = parts
        .iter()
        .enumerate()
        .map(|(i, (label, _, _))| {
            let (font, ink) = style(i + 1 == parts.len());
            let galley = painter.layout_no_wrap(label.clone(), font, ink);
            let width = galley.size().x + 16.0 + if i == 0 { 20.0 } else { 0.0 };
            (galley, width)
        })
        .collect::<Vec<_>>();
    let left = rect.left() + 4.0;
    let right = rect.right() - 4.0;
    let total = |from: usize| {
        measured[from..].iter().map(|(_, w)| w).sum::<f32>()
            + SEPARATOR * (parts.len() - from - 1) as f32
            + if from > 0 { MORE + SEPARATOR } else { 0.0 }
    };
    let mut first = 0;
    while first + 1 < parts.len() && total(first) > right - left {
        first += 1;
    }
    let separator = |x: f32| {
        painter.text(
            pos2(x + SEPARATOR * 0.5, middle),
            egui::Align2::CENTER_CENTER,
            "/",
            theme::regular(13.0),
            p.muted,
        );
        x + SEPARATOR
    };
    let mut x = left;
    if first > 0 {
        let more = Rect::from_min_size(pos2(x, middle - 12.0), vec2(MORE, 24.0));
        let response = ui.interact(more, ui.id().with("path-bar-more"), Sense::click());
        response.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Button, true, "Show enclosing folders")
        });
        crumb_surface(&painter, &response, more, p);
        painter.text(
            more.center(),
            egui::Align2::CENTER_CENTER,
            "…",
            theme::medium(13.0),
            p.secondary,
        );
        let response = response
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text("Enclosing folders");
        egui::Popup::menu(&response).show(|ui| {
            controls::menu_layout(ui, 220.0);
            for (label, icon, path) in &parts[..first] {
                if controls::menu_item(ui, p, *icon, label, "", false) {
                    actions.push(Action::Navigate(pane.id, path.clone()));
                    ui.close();
                }
            }
        });
        x = separator(more.right());
    }
    for (i, (label, icon, path)) in parts.iter().enumerate().skip(first) {
        let last = i + 1 == parts.len();
        let (galley, width) = &measured[i];
        let width = width.min(right - x);
        if width < 28.0 {
            break;
        }
        let crumb = Rect::from_min_size(pos2(x, middle - 12.0), vec2(width, 24.0));
        // The folder in view is part of the bar: clicking it edits the path.
        let response = (!last).then(|| {
            let response = ui.interact(crumb, ui.id().with(("path-crumb", path)), Sense::click());
            response.widget_info(|| {
                WidgetInfo::labeled(WidgetType::Button, true, format!("Go to {label}"))
            });
            crumb_surface(&painter, &response, crumb, p);
            response
        });
        let mut text_left = crumb.left() + 8.0;
        if i == 0 {
            icons::paint(
                &painter,
                Rect::from_center_size(pos2(text_left + 7.0, middle), Vec2::splat(14.0)),
                *icon,
                if last { p.accent } else { p.secondary },
            );
            text_left += 20.0;
        }
        let room = crumb.right() - 8.0 - text_left;
        let text = if galley.size().x > room {
            let (font, ink) = style(last);
            controls::elided(&painter, label, font, ink, room)
        } else {
            galley.clone()
        };
        controls::galley_at(&painter, pos2(text_left, middle), text);
        x = crumb.right();
        if !last {
            x = separator(x);
        }
        let Some(response) = response else { continue };
        if response.clicked() {
            actions.push(Action::Navigate(pane.id, path.clone()));
        } else if response.clicked_by(egui::PointerButton::Middle) {
            actions.push(Action::NewTab(path.clone()));
        }
        response
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text(path.display().to_string())
            .context_menu(|ui| {
                controls::menu_layout(ui, 200.0);
                if controls::menu_item(ui, p, Icon::Plus, "Open in new tab", "", false) {
                    actions.push(Action::NewTab(path.clone()));
                    ui.close();
                }
                if controls::menu_item(ui, p, Icon::Copy, "Copy path", "", false) {
                    actions.push(Action::CopyPath(path.clone()));
                    ui.close();
                }
            });
    }
    if background.on_hover_cursor(CursorIcon::Text).clicked() {
        state.go_to_folder(pane);
    }
}

fn crumb_surface(painter: &egui::Painter, response: &egui::Response, rect: Rect, p: Palette) {
    if response.is_pointer_button_down_on() {
        painter.rect_filled(rect, 6, p.pressed);
    } else if response.hovered() {
        painter.rect_filled(rect, 6, p.hover);
    }
    if response.has_focus() {
        controls::focus_ring(painter, rect, 6, p);
    }
}

fn location_id(pane: u64) -> egui::Id {
    egui::Id::new(("location", pane))
}

/// The path bar as an editable location. Enter opens what was typed; Escape,
/// the close button or a click elsewhere leaves the pane where it is.
fn location_field(
    ui: &mut Ui,
    rect: Rect,
    state: &mut UiState,
    p: Palette,
    actions: &mut Vec<Action>,
) {
    let Some(edit) = state.location.as_mut() else {
        return;
    };
    let id = location_id(edit.pane);
    controls::field_frame(ui, p, rect, id, true);
    let inner = Rect::from_min_max(
        pos2(rect.left() + 12.0, rect.top()),
        pos2(rect.right() - 58.0, rect.bottom()),
    );
    let response = controls::place(
        ui,
        inner,
        Layout::left_to_right(Align::Center),
        ("location-text", id),
        |ui| {
            controls::bare_text_edit(
                ui,
                id,
                &mut edit.text,
                "Type a folder path",
                13.0,
                inner.width(),
            )
        },
    );
    response.widget_info(|| WidgetInfo::labeled(WidgetType::TextEdit, true, "Location"));
    if edit.focus {
        edit.focus = false;
        response.request_focus();
        // The whole path starts selected, so typing replaces it.
        if let Some(mut text) = egui::TextEdit::load_state(ui.ctx(), response.id) {
            text.cursor
                .set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(edit.text.chars().count()),
                )));
            text.store(ui.ctx(), response.id);
        }
    }
    let middle = rect.center().y;
    let clear = field_button(
        ui,
        pos2(rect.right() - 42.0, middle),
        Icon::Eraser,
        12.0,
        "Clear location",
        p,
    );
    let close = field_button(
        ui,
        pos2(rect.right() - 16.0, middle),
        Icon::Close,
        9.0,
        "Stop editing location",
        p,
    );
    let entered = response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
    let typed = edit.text.trim().to_owned();
    if entered && !typed.is_empty() {
        actions.push(Action::Location(typed));
        state.location = None;
    } else if close.clicked() {
        state.location = None;
    } else if entered || clear.clicked() {
        if clear.clicked() {
            edit.text.clear();
        }
        edit.focus = true;
    } else if !ui.memory(|memory| memory.has_focus(response.id))
        && !edit.focus
        && !ui.input(|input| input.pointer.any_down())
    {
        // Escape or a click elsewhere. Widget focus, not `Response::has_focus`,
        // which is also false while another window is in front.
        state.location = None;
    }
}

/// A small icon button inside a field's trailing edge.
fn field_button(
    ui: &mut Ui,
    center: Pos2,
    icon: Icon,
    size: f32,
    label: &str,
    p: Palette,
) -> egui::Response {
    let target = Rect::from_center_size(center, Vec2::splat(22.0));
    let response = ui.interact(
        target,
        ui.id().with(("field-button", label)),
        Sense::click(),
    );
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));
    let painter = ui.painter();
    if response.hovered() || response.has_focus() {
        painter.rect_filled(target.shrink(1.0), 6, p.hover);
    }
    if response.has_focus() {
        controls::focus_ring(painter, target.shrink(1.0), 6, p);
    }
    icons::paint(
        painter,
        Rect::from_center_size(target.center(), Vec2::splat(size)),
        icon,
        if response.hovered() {
            p.fg
        } else {
            p.secondary
        },
    );
    response
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(label)
}

/// Folder, view and window actions. Views and split move here when the
/// toolbar has no room for their buttons.
fn more_menu(
    menu: &egui::Response,
    workspace: &Workspace,
    config: &Config,
    state: &mut UiState,
    p: Palette,
    roomy: bool,
    actions: &mut Vec<Action>,
) {
    egui::Popup::menu(menu).show(|ui| {
        controls::menu_layout(ui, 230.0);
        let path = workspace.pane().path.clone();
        if controls::menu_item(
            ui,
            p,
            Icon::FolderPlus,
            "New folder…",
            &controls::edit_shortcut("Shift+N"),
            false,
        ) {
            state.open_dialog(Dialog::New {
                parent: path.clone(),
                name: String::new(),
                folder: true,
            });
            ui.close();
        }
        if controls::menu_item(ui, p, Icon::FilePlus, "New file…", "", false) {
            state.open_dialog(Dialog::New {
                parent: path.clone(),
                name: String::new(),
                folder: false,
            });
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
            state.go_to_folder(workspace.pane());
            ui.close();
        }
        controls::menu_separator(ui, p);
        if !roomy {
            if controls::menu_item(
                ui,
                p,
                Icon::List,
                "List view",
                &controls::edit_shortcut("2"),
                false,
            ) {
                actions.push(Action::View(ViewMode::List));
                ui.close();
            }
            if controls::menu_item(
                ui,
                p,
                Icon::Grid,
                "Icon view",
                &controls::edit_shortcut("1"),
                false,
            ) {
                actions.push(Action::View(ViewMode::Grid));
                ui.close();
            }
            if controls::menu_item(
                ui,
                p,
                Icon::SplitVertical,
                "Toggle split view",
                &controls::edit_shortcut("\\"),
                false,
            ) {
                actions.push(Action::ToggleSplit);
                ui.close();
            }
        }
        if controls::menu_item(
            ui,
            p,
            Icon::Eyes,
            if config.show_hidden {
                "Hide hidden files"
            } else {
                "Show hidden files"
            },
            &if cfg!(target_os = "macos") {
                String::from("⌘⇧.")
            } else {
                controls::edit_shortcut("H")
            },
            false,
        ) {
            actions.push(Action::ToggleHidden);
            ui.close();
        }
        let pinned = config.favorites.contains(&path);
        if controls::menu_item(
            ui,
            p,
            Icon::Pin,
            if pinned {
                "Unpin this folder"
            } else {
                "Pin this folder"
            },
            "",
            false,
        ) {
            actions.push(Action::Favorite(path.clone()));
            ui.close();
        }
        if controls::menu_item(ui, p, Icon::Refresh, "Refresh", "F5", false) {
            actions.push(Action::Reload);
            ui.close();
        }
        controls::menu_separator(ui, p);
        if controls::menu_item(
            ui,
            p,
            Icon::Settings,
            "Preferences…",
            &controls::edit_shortcut(","),
            false,
        ) {
            state.open_dialog(Dialog::Preferences);
            ui.close();
        }
        if controls::menu_item(ui, p, Icon::Info, "About Neptune Finder", "", false) {
            state.open_dialog(Dialog::About);
            ui.close();
        }
    });
}

/// One tab per folder history, of equal width, drawn like Terminal's tabs:
/// the tab in view takes a faint fill and each closes from its own button.
pub fn tabs(ui: &mut Ui, rect: Rect, workspace: &Workspace, p: Palette, actions: &mut Vec<Action>) {
    let count = workspace.tabs.len();
    let gap = 4.0;
    let width = ((rect.width() - gap * (count as f32 - 1.0)) / count as f32).clamp(120.0, 260.0);
    controls::place(
        ui,
        rect,
        Layout::left_to_right(Align::Center),
        "tab-strip",
        |ui| {
            egui::ScrollArea::horizontal()
                .id_salt("tab-scroll")
                .auto_shrink([false, false])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    ui.horizontal_centered(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for (index, tab) in workspace.tabs.iter().enumerate() {
                            ui.push_id(("tab", tab.id), |ui| {
                                tab_button(ui, workspace, index, width, p, actions);
                            });
                        }
                    });
                });
        },
    );
}

fn tab_button(
    ui: &mut Ui,
    workspace: &Workspace,
    index: usize,
    width: f32,
    p: Palette,
    actions: &mut Vec<Action>,
) {
    let tab = &workspace.tabs[index];
    let active = workspace.active == index;
    let pane = &tab.panes[tab.active];
    let (rect, response) = ui.allocate_exact_size(vec2(width, 28.0), Sense::click());
    let label = name(&pane.path);
    response.widget_info(|| {
        WidgetInfo::selected(
            WidgetType::SelectableLabel,
            true,
            active,
            format!("Tab {}: {label}", index + 1),
        )
    });
    let painter = ui.painter();
    if active {
        painter.rect_filled(rect, metrics::CONTROL_RADIUS, p.pressed);
    } else if response.hovered() {
        painter.rect_filled(rect, metrics::CONTROL_RADIUS, p.hover);
    }
    if response.has_focus() {
        controls::focus_ring(painter, rect.shrink(2.0), 6, p);
    }
    let close_rect =
        Rect::from_center_size(pos2(rect.left() + 15.0, rect.center().y), Vec2::splat(20.0));
    let close = ui.interact(close_rect, ui.id().with("close-tab"), Sense::click());
    close.widget_info(|| {
        WidgetInfo::labeled(WidgetType::Button, true, format!("Close tab {}", index + 1))
    });
    let ink = if active || response.hovered() {
        p.fg
    } else {
        p.secondary
    };
    // As in Safari and Finder, the close control takes the leading corner
    // under the pointer; the name stays centred either way.
    if response.hovered() || close.hovered() || close.has_focus() {
        if close.hovered() {
            painter.rect_filled(close_rect.shrink(1.0), 5, p.hover);
        }
        icons::paint(painter, close_rect.shrink(5.0), Icon::Close, p.secondary);
    }
    if close.has_focus() {
        controls::focus_ring(painter, close_rect, 6, p);
    }
    let text = controls::elided(
        painter,
        &label,
        theme::medium(12.0),
        ink,
        rect.width() - 64.0,
    );
    let icon = 14.0;
    let total = icon + 6.0 + text.size().x;
    let start = rect.center().x - total * 0.5;
    icons::paint(
        painter,
        Rect::from_center_size(pos2(start + icon * 0.5, rect.center().y), Vec2::splat(icon)),
        Icon::Folder,
        if active { p.accent } else { p.muted },
    );
    controls::galley_at(painter, pos2(start + icon + 6.0, rect.center().y), text);
    if close.clicked() || response.clicked_by(egui::PointerButton::Middle) {
        actions.push(Action::CloseTab(index));
    } else if response.clicked() {
        actions.push(Action::SelectTab(index));
    }
    response
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(pane.path.display().to_string());
}
