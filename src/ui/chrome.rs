//! Window chrome in Neptune Terminal's arrangement: a full-height source list,
//! a toolbar that names the folder in view, and a tab strip that appears only
//! when there is more than one tab to tell apart.
use super::{Action, Dialog, UiState, name};
use crate::{
    config::{Config, ViewMode},
    controls,
    icons::{self, Icon},
    model::Workspace,
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

/// The toolbar names the folder in view and holds navigation, search, views
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

    // The search field stays centred on the toolbar while there is room, as
    // Terminal's command field does; otherwise it trails the title.
    let width = (rect.width() * 0.32).clamp(200.0, 300.0);
    let centred = Rect::from_center_size(rect.center(), vec2(width, metrics::CONTROL_HEIGHT));
    let field = if centred.left() >= title_left + 120.0 && centred.right() <= right {
        centred
    } else {
        let width = (right - title_left - 90.0).clamp(0.0, 220.0);
        Rect::from_min_max(
            pos2(right - width, middle - metrics::CONTROL_HEIGHT * 0.5),
            pos2(right, middle + metrics::CONTROL_HEIGHT * 0.5),
        )
    };
    let pane = workspace.pane_mut();
    if field.width() >= 90.0 {
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
    let budget = field.left().min(right) - title_left - 14.0;
    if budget >= 40.0 {
        controls::galley_at(
            ui.painter(),
            pos2(title_left, middle),
            controls::elided(
                ui.painter(),
                &name(&pane.path),
                theme::medium(13.0),
                p.fg,
                budget,
            ),
        );
    }
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
            state.open_dialog(Dialog::Location {
                value: path.display().to_string(),
            });
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
