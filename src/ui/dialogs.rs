//! Modal drafts retain their original file targets and emit owned operations.

use super::{Action, Dialog, Preview, UiState};
use crate::config::{Accent, Config, Theme, ViewMode};
use crate::controls::{self, ButtonKind, SheetPlacement};
use crate::icons::{self, Icon};
use crate::model::Workspace;
use crate::theme::{self, Palette};
use eframe::egui::{
    self, Align, Align2, Color32, CursorIcon, Id, Layout, Rect, Sense, Stroke, StrokeKind, Ui,
    Vec2, WidgetInfo, WidgetType, pos2, vec2,
};
use finder_operations::{Operation, validate_name};

pub fn show(
    ctx: &egui::Context,
    _workspace: &mut Workspace,
    config: &mut Config,
    state: &mut UiState,
    p: Palette,
    actions: &mut Vec<Action>,
) {
    let Some(mut dialog) = state.dialog.take() else {
        return;
    };
    // A modal can request another layout pass in the same frame. Remember the
    // frame as well as the focus draft so clearing focus in one pass cannot
    // let the opening Enter confirm in a later pass of that same frame.
    let frame = ctx.cumulative_frame_nr();
    let opening_id = Id::new(("finder-dialog-opening-frame", ctx.viewport_id()));
    let opened_this_frame = ctx.data_mut(|data| {
        if state.focus_dialog {
            data.insert_temp(opening_id, frame);
        }
        data.get_temp::<u64>(opening_id) == Some(frame)
    });
    let opening = state.focus_dialog || opened_this_frame;
    let enter = !opening && fresh_key(ctx, egui::Key::Enter);
    let allow_button_activation = !ctx.input(|input| {
        input.events.iter().any(|event| {
            matches!(event, egui::Event::Key {
                key: egui::Key::Enter, pressed: true, repeat, ..
            } if *repeat || opening)
        })
    });
    let confirmed = |clicked: bool| (clicked && allow_button_activation) || enter;
    let escape = fresh_key(ctx, egui::Key::Escape);
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Middle,
        Id::new("finder-dialog-scrim"),
    ));
    controls::scrim(
        &painter,
        ctx.content_rect(),
        super::window_radius(ctx),
        p,
        1.0,
    );
    let mut close = escape;
    match &mut dialog {
        Dialog::New {
            parent,
            name,
            folder,
        } => {
            let title = if *folder { "New folder" } else { "New file" };
            let action_label = "Create";
            let mut submit = false;
            let output = controls::sheet(ctx, p, title, 400.0, SheetPlacement::Center, |ui| {
                close |= controls::sheet_header(ui, p, title, Some("Close new item dialog"));
                scroll_body(ui, ctx, "new-item-body", |ui| {
                    caption(
                        ui,
                        p,
                        &format!("In {}", super::display_path(parent, &state.home)),
                    );
                    ui.add_space(12.0);
                    name_field(ui, p, name, &mut state.focus_dialog, "new-item-name");
                    validation_message(ui, p, name);
                });
                let valid = validate_name(name).is_ok();
                let buttons = footer(ui, p, action_label, ButtonKind::Primary, valid);
                submit = valid && confirmed(buttons.confirm);
                close |= buttons.cancel;
            });
            close |= output.backdrop_clicked;
            if submit && !close {
                actions.push(Action::Submit(if *folder {
                    Operation::CreateFolder {
                        parent: parent.clone(),
                        name: name.clone(),
                    }
                } else {
                    Operation::CreateFile {
                        parent: parent.clone(),
                        name: name.clone(),
                    }
                }));
                close = true;
            }
        }
        Dialog::Rename { source, name } => {
            let mut submit = false;
            let output = controls::sheet(ctx, p, "Rename", 400.0, SheetPlacement::Center, |ui| {
                close |= controls::sheet_header(ui, p, "Rename", Some("Close rename dialog"));
                scroll_body(ui, ctx, "rename-body", |ui| {
                    caption(ui, p, &format!("Currently “{}”", super::name(source)));
                    ui.add_space(12.0);
                    name_field(ui, p, name, &mut state.focus_dialog, "rename-name");
                    validation_message(ui, p, name);
                });
                let valid = validate_name(name).is_ok();
                let buttons = footer(ui, p, "Rename", ButtonKind::Primary, valid);
                submit = valid && confirmed(buttons.confirm);
                close |= buttons.cancel;
            });
            close |= output.backdrop_clicked;
            if submit && !close {
                // Accepting an unchanged lossy display name must not rename an
                // original non-UTF-8 path to replacement characters.
                if source
                    .file_name()
                    .is_none_or(|original| original.to_string_lossy().as_ref() != name.as_str())
                {
                    actions.push(Action::Submit(Operation::Rename {
                        source: source.clone(),
                        name: name.clone(),
                    }));
                }
                close = true;
            }
        }
        Dialog::Trash { paths } => {
            let mut submit = false;
            let title = if paths.len() == 1 {
                "Move to Trash?"
            } else {
                "Move items to Trash?"
            };
            let output = controls::sheet(ctx, p, title, 420.0, SheetPlacement::Center, |ui| {
                close |= controls::sheet_header(ui, p, title, Some("Cancel moving to Trash"));
                scroll_body(ui, ctx, "trash-body", |ui| {
                    let message = if paths.len() == 1 {
                        format!("Move “{}” to the system Trash?", super::name(&paths[0]))
                    } else {
                        format!("Move these {} items to the system Trash?", paths.len())
                    };
                    body_text(ui, p, &message);
                    ui.add_space(10.0);
                    caption(
                        ui,
                        p,
                        "You can restore them from Trash using your system file manager.",
                    );
                });
                let buttons = footer(
                    ui,
                    p,
                    "Move to Trash",
                    ButtonKind::Destructive,
                    !paths.is_empty(),
                );
                if opening && ui.is_enabled() && !ui.is_sizing_pass() {
                    state.focus_dialog = false;
                }
                submit = !paths.is_empty() && confirmed(buttons.confirm);
                close |= buttons.cancel;
            });
            close |= output.backdrop_clicked;
            if submit && !close {
                actions.push(Action::Submit(Operation::Trash {
                    paths: paths.clone(),
                }));
                close = true;
            }
        }
        Dialog::Location { value } => {
            let mut submit = false;
            let output = controls::sheet(
                ctx,
                p,
                "Go to folder",
                500.0,
                SheetPlacement::Center,
                |ui| {
                    close |= controls::sheet_header(
                        ui,
                        p,
                        "Go to folder",
                        Some("Close location dialog"),
                    );
                    scroll_body(ui, ctx, "location-body", |ui| {
                        caption(ui, p, "Enter a folder path. Use ~ for your home folder.");
                        ui.add_space(12.0);
                        let width = ui.available_width();
                        let field = controls::text_field(
                            ui,
                            p,
                            Id::new("location-path"),
                            value,
                            "~/Documents",
                            "Folder path",
                            width,
                        );
                        request_focus(ui, &field, &mut state.focus_dialog);
                    });
                    let valid = !value.trim().is_empty();
                    let buttons = footer(ui, p, "Go", ButtonKind::Primary, valid);
                    submit = valid && confirmed(buttons.confirm);
                    close |= buttons.cancel;
                },
            );
            close |= output.backdrop_clicked;
            if submit && !close {
                actions.push(Action::Location(value.clone()));
                close = true;
            }
        }
        Dialog::Preferences => {
            let mut changed = false;
            let output = preferences(ctx, config, state, p, &mut close, &mut changed);
            close |= output.backdrop_clicked;
            if changed {
                actions.push(Action::PreferencesChanged);
            }
        }
        Dialog::QuickLook { path } => {
            let body_height = (ctx.content_rect().height() - 24.0 - 52.0 - 60.0).max(40.0);
            let output =
                controls::sheet(ctx, p, "Quick Look", 760.0, SheetPlacement::Center, |ui| {
                    close |= quick_look_header(ui, p, path);
                    egui::ScrollArea::both()
                        .id_salt("quick-look-body")
                        .max_height(body_height)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            controls::padded(ui, 16.0, |ui| {
                                let width = ui.available_width();
                                match state.preview.as_ref() {
                                    None => {
                                        ui.add_space(24.0);
                                        ui.vertical_centered(|ui| {
                                            ui.spinner();
                                            ui.add_space(8.0);
                                            caption(ui, p, "Preparing preview…");
                                        });
                                        ui.add_space(24.0);
                                    }
                                    Some(Preview::Text(text)) => {
                                        egui::Frame::new()
                                            .fill(p.bg)
                                            .corner_radius(theme::metrics::PANE_RADIUS)
                                            .inner_margin(egui::Margin::same(14))
                                            .show(ui, |ui| {
                                                ui.set_width(width - 28.0);
                                                ui.add(
                                                    egui::Label::new(
                                                        egui::RichText::new(text)
                                                            .font(egui::FontId::monospace(12.0))
                                                            .color(p.fg),
                                                    )
                                                    .selectable(true)
                                                    .wrap(),
                                                );
                                            });
                                    }
                                    Some(Preview::Message(message)) => {
                                        ui.add_space(20.0);
                                        ui.vertical_centered(|ui| body_text(ui, p, message));
                                        ui.add_space(20.0);
                                    }
                                    Some(Preview::Image(texture)) => {
                                        let native = texture.size_vec2();
                                        let scale = (width / native.x)
                                            .min((body_height - 40.0).max(24.0) / native.y)
                                            .min(1.0);
                                        ui.vertical_centered(|ui| {
                                            ui.add(
                                                egui::Image::new((texture.id(), native))
                                                    .fit_to_exact_size(native * scale)
                                                    .corner_radius(6),
                                            );
                                        });
                                    }
                                }
                                ui.add_space(14.0);
                            });
                        });
                    let rect = controls::sheet_footer(ui, p);
                    controls::place(
                        ui,
                        rect,
                        Layout::right_to_left(Align::Center),
                        "quick-look-actions",
                        |ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            let done = controls::button(ui, p, "Done", ButtonKind::Primary);
                            request_focus(ui, &done, &mut state.focus_dialog);
                            close |= done.clicked() && allow_button_activation;
                            if controls::button(ui, p, "Open", ButtonKind::Secondary).clicked() {
                                actions
                                    .push(Action::Submit(Operation::Open { path: path.clone() }));
                                close = true;
                            }
                            let hint = controls::elided(
                                ui.painter(),
                                "Space closes",
                                theme::regular(11.5),
                                p.muted,
                                ui.available_width() - 8.0,
                            );
                            controls::galley_at(
                                ui.painter(),
                                pos2(rect.left() + 4.0, rect.center().y),
                                hint,
                            );
                        },
                    );
                });
            close |= output.backdrop_clicked || (!opening && fresh_key(ctx, egui::Key::Space));
        }
        Dialog::About => {
            let output = controls::sheet(
                ctx,
                p,
                "About Neptune Finder",
                420.0,
                SheetPlacement::Center,
                |ui| {
                    close |= controls::sheet_header(ui, p, "", Some("Close About Neptune Finder"));
                    scroll_body(ui, ctx, "about-body", |ui| {
                        ui.vertical_centered(|ui| {
                            if let Some(icon) = state.brand_icon.as_ref() {
                                ui.add(
                                    egui::Image::new((icon.id(), vec2(80.0, 80.0)))
                                        .fit_to_exact_size(vec2(80.0, 80.0)),
                                );
                                ui.add_space(14.0);
                            }
                            ui.label(
                                egui::RichText::new("Neptune Finder")
                                    .font(theme::semibold(17.0))
                                    .color(p.fg),
                            );
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(format!(
                                    "Version {}",
                                    env!("CARGO_PKG_VERSION")
                                ))
                                .font(theme::regular(12.0))
                                .color(p.secondary),
                            );
                            ui.add_space(14.0);
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(
                                        "A native file explorer for the Neptune ecosystem. \
                                         Browse in tabs, organize files and preview them \
                                         without leaving your folder.",
                                    )
                                    .font(theme::regular(12.5))
                                    .color(p.secondary),
                                )
                                .halign(Align::Center)
                                .wrap(),
                            );
                        });
                    });
                    close |= confirmed(done_footer(ui, p, "Done", &mut state.focus_dialog));
                },
            );
            close |= output.backdrop_clicked;
        }
    }
    if close {
        state.preview = None;
    } else {
        state.dialog = Some(dialog);
    }
}

fn fresh_key(ctx: &egui::Context, key: egui::Key) -> bool {
    ctx.input(|input| {
        input.events.iter().any(|event| {
            matches!(event,
                egui::Event::Key { key: pressed, pressed: true, repeat: false, modifiers, .. }
                if *pressed == key && !modifiers.command && !modifiers.alt && !modifiers.ctrl
            )
        })
    })
}

fn request_focus(ui: &Ui, response: &egui::Response, pending: &mut bool) {
    if *pending && ui.is_enabled() && !ui.is_sizing_pass() {
        response.request_focus();
        *pending = false;
    }
}

fn name_field(ui: &mut Ui, p: Palette, name: &mut String, focus: &mut bool, salt: &str) {
    ui.add_space(4.0);
    let width = ui.available_width();
    let (_, rect) = ui.allocate_space(vec2(width, theme::metrics::CONTROL_HEIGHT));
    let id = Id::new(salt);
    // A name that cannot be used outlines the field in the problem colour.
    let valid = name.is_empty() || validate_name(name).is_ok();
    controls::field_frame(ui, p, rect, id, valid);
    let inner = rect.shrink2(vec2(10.0, 0.0));
    let field = controls::place(
        ui,
        inner,
        Layout::left_to_right(Align::Center),
        ("name-field", salt),
        |ui| controls::bare_text_edit(ui, id, name, "Name", 13.0, inner.width()),
    );
    field.widget_info(|| WidgetInfo::labeled(WidgetType::TextEdit, true, "File or folder name"));
    request_focus(ui, &field, focus);
}

/// The reason a name cannot be used, under the field; quiet otherwise.
fn validation_message(ui: &mut Ui, p: Palette, name: &str) {
    let Err(error) = validate_name(name) else {
        return;
    };
    if name.is_empty() {
        return;
    }
    ui.add_space(8.0);
    ui.add(
        egui::Label::new(
            egui::RichText::new(error.to_string())
                .font(theme::regular(11.5))
                .color(p.red),
        )
        .wrap(),
    );
}

fn scroll_body(ui: &mut Ui, ctx: &egui::Context, salt: &str, contents: impl FnOnce(&mut Ui)) {
    let footer = if ui.available_width() < 280.0 {
        94.0
    } else {
        60.0
    };
    let height = (ctx.content_rect().height() - 24.0 - 52.0 - footer).max(20.0);
    egui::ScrollArea::vertical()
        .id_salt(salt)
        .max_height(height)
        .show(ui, |ui| {
            controls::padded(ui, 20.0, |ui| {
                contents(ui);
                ui.add_space(18.0);
            });
        });
}

struct Footer {
    confirm: bool,
    cancel: bool,
}

fn footer(ui: &mut Ui, p: Palette, confirm: &str, kind: ButtonKind, enabled: bool) -> Footer {
    let narrow = ui.available_width() < 280.0;
    let rect = if narrow {
        let (_, rect) = ui.allocate_space(vec2(ui.available_width(), 94.0));
        ui.painter()
            .line_segment([rect.left_top(), rect.right_top()], p.hairline());
        rect.shrink2(vec2(16.0, 10.0))
    } else {
        controls::sheet_footer(ui, p)
    };
    controls::place(
        ui,
        rect,
        if narrow {
            Layout::top_down(Align::Max)
        } else {
            Layout::right_to_left(Align::Center)
        },
        "dialog-actions",
        |ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            let confirm = ui
                .add_enabled_ui(enabled, |ui| {
                    controls::button(ui, p, confirm, kind).clicked()
                })
                .inner;
            let cancel = controls::button(ui, p, "Cancel", ButtonKind::Secondary).clicked();
            Footer { confirm, cancel }
        },
    )
}

fn done_footer(ui: &mut Ui, p: Palette, label: &str, focus: &mut bool) -> bool {
    let rect = controls::sheet_footer(ui, p);
    controls::place(
        ui,
        rect,
        Layout::right_to_left(Align::Center),
        "dialog-done",
        |ui| {
            let done = controls::button(ui, p, label, ButtonKind::Primary);
            request_focus(ui, &done, focus);
            done.clicked()
        },
    )
}

fn body_text(ui: &mut Ui, p: Palette, text: &str) {
    ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .font(theme::regular(13.0))
                .color(p.fg),
        )
        .wrap(),
    );
}

fn caption(ui: &mut Ui, p: Palette, text: &str) {
    ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .font(theme::regular(11.0))
                .color(p.secondary),
        )
        .wrap(),
    );
}

/// The panes of the Preferences sheet, named in its source list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PreferencesPane {
    General,
    Appearance,
    Files,
}
impl PreferencesPane {
    const ALL: [Self; 3] = [Self::General, Self::Appearance, Self::Files];
    fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Files => "Files",
        }
    }
    fn icon(self) -> Icon {
        match self {
            Self::General => Icon::Settings,
            Self::Appearance => Icon::Sun,
            Self::Files => Icon::Folder,
        }
    }
}

/// Terminal's Preferences geometry: a source list concentric with the sheet
/// beside one pane of labelled cards, one height for every pane.
const PREFERENCES_HEIGHT: f32 = 480.0;
const PREFERENCES_HEADER: f32 = 52.0;
const PREFERENCES_INSET: f32 = 6.0;
const PREFERENCES_LIST: f32 = 176.0;
const PREFERENCES_RAIL: f32 = 44.0;
const PREFERENCES_COMPACT: f32 = 540.0;

fn preferences(
    ctx: &egui::Context,
    config: &mut Config,
    state: &mut UiState,
    p: Palette,
    close: &mut bool,
    changed: &mut bool,
) -> controls::SheetOutput<()> {
    let screen = ctx.content_rect();
    controls::sheet(ctx, p, "Preferences", 680.0, SheetPlacement::Center, |ui| {
        // One height for every pane, so choosing one never moves the sheet.
        // Short windows keep enough margin for the shadow to stay inside the
        // rounded window corners; the pane scrolls instead.
        let height = (screen.height() - 96.0).clamp(160.0, PREFERENCES_HEIGHT);
        let (_, rect) = ui.allocate_space(vec2(ui.available_width(), height));
        let compact = rect.width() < PREFERENCES_COMPACT;
        let list = Rect::from_min_size(
            rect.min + Vec2::splat(PREFERENCES_INSET),
            vec2(
                if compact {
                    PREFERENCES_RAIL
                } else {
                    PREFERENCES_LIST
                },
                height - PREFERENCES_INSET * 2.0,
            ),
        );
        let content = Rect::from_min_max(pos2(list.right(), rect.top()), rect.max);
        let radius = theme::metrics::SHEET_RADIUS - PREFERENCES_INSET as u8;
        ui.painter().rect_filled(list, radius, p.chrome);
        ui.painter()
            .rect_stroke(list, radius, p.hairline(), StrokeKind::Inside);
        let panes = if compact {
            list.with_min_y(list.top() + PREFERENCES_INSET)
        } else {
            let top = list.top() + PREFERENCES_HEADER - PREFERENCES_INSET;
            ui.painter().text(
                pos2(list.left() + 14.0, (list.top() + top) * 0.5),
                Align2::LEFT_CENTER,
                "Preferences",
                theme::semibold(15.0),
                p.fg,
            );
            list.with_min_y(top)
        };
        controls::place(
            ui,
            panes,
            Layout::top_down(Align::Min),
            "preferences-panes",
            |ui| pane_list(ui, p, &mut state.preferences_pane, compact),
        );
        let pane = state.preferences_pane;
        controls::place(
            ui,
            content,
            Layout::top_down(Align::Min),
            "preferences-pane",
            |ui| {
                *close |= controls::sheet_header(
                    ui,
                    p,
                    if compact { "Preferences" } else { pane.title() },
                    Some("Close preferences"),
                );
                egui::ScrollArea::vertical()
                    .id_salt(("preferences-body", pane))
                    .max_height(height - PREFERENCES_HEADER)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        controls::padded(ui, 20.0, |ui| {
                            match pane {
                                PreferencesPane::General => general(ui, p, config, changed),
                                PreferencesPane::Appearance => appearance(ui, p, config, changed),
                                PreferencesPane::Files => files(ui, p, config, changed),
                            }
                            ui.add_space(20.0);
                        });
                    });
            },
        );
        // Settings apply as they change; there is nothing to confirm, and
        // Escape leaves the sheet as its close control does.
        state.focus_dialog = false;
    })
}

/// The source list: the panes by name, or by icon alone in a narrow sheet.
fn pane_list(ui: &mut Ui, p: Palette, current: &mut PreferencesPane, compact: bool) {
    let width = ui.available_width();
    for entry in PreferencesPane::ALL {
        let (_, slot) = ui.allocate_space(vec2(width, 32.0));
        let row = slot.shrink2(vec2(PREFERENCES_INSET, 1.0));
        let response = ui.interact(row, Id::new(("preferences-pane", entry)), Sense::click());
        if response.clicked() {
            *current = entry;
        }
        let selected = *current == entry;
        response.widget_info(|| {
            WidgetInfo::selected(WidgetType::RadioButton, true, selected, entry.title())
        });
        let painter = ui.painter();
        if selected || response.is_pointer_button_down_on() {
            painter.rect_filled(row, theme::metrics::ROW_RADIUS, p.pressed);
        } else if response.hovered() {
            painter.rect_filled(row, theme::metrics::ROW_RADIUS, p.hover);
        }
        if response.has_focus() {
            controls::focus_ring(painter, row.shrink(2.0), theme::metrics::ROW_RADIUS - 2, p);
        }
        let ink = if selected || response.hovered() {
            p.fg
        } else {
            p.secondary
        };
        let icon = if compact {
            row.center()
        } else {
            pos2(row.left() + 16.0, row.center().y)
        };
        icons::paint(
            painter,
            Rect::from_center_size(icon, Vec2::splat(16.0)),
            entry.icon(),
            ink,
        );
        if !compact {
            controls::galley_at(
                painter,
                pos2(row.left() + 32.0, row.center().y),
                controls::elided(
                    painter,
                    entry.title(),
                    theme::medium(13.0),
                    ink,
                    row.width() - 40.0,
                ),
            );
        }
        let response = response.on_hover_cursor(CursorIcon::PointingHand);
        if compact {
            response.on_hover_text(entry.title());
        }
    }
}

fn card(
    ui: &mut Ui,
    p: Palette,
    title: &str,
    first: bool,
    rows: impl FnOnce(&mut Ui, &mut controls::Group),
) {
    if !first {
        ui.add_space(14.0);
    }
    controls::section_label(ui, p, title);
    controls::group(ui, p, rows);
}

fn general(ui: &mut Ui, p: Palette, config: &mut Config, changed: &mut bool) {
    card(ui, p, "Window", true, |ui, rows| {
        rows.row(ui, "Show sidebar", |ui| {
            *changed |= controls::toggle(ui, p, &mut config.sidebar, "Show sidebar").changed();
        });
        rows.row(ui, "Reopen tabs at launch", |ui| {
            *changed |= controls::toggle(
                ui,
                p,
                &mut config.restore_session,
                "Restore tabs when opening Neptune Finder",
            )
            .changed();
        });
        rows.note(
            ui,
            "Tabs and their folders return as you left them. File operations never resume.",
        );
    });
    card(ui, p, "Defaults", false, |ui, rows| {
        rows.row(ui, "Reset preferences", |ui| {
            if controls::button(ui, p, "Reset", ButtonKind::Secondary).clicked() {
                let favorites = std::mem::take(&mut config.favorites);
                *config = Config {
                    favorites,
                    ..Config::default()
                };
                *changed = true;
            }
        });
        rows.note(ui, "Pinned folders are kept.");
    });
}

fn appearance(ui: &mut Ui, p: Palette, config: &mut Config, changed: &mut bool) {
    card(ui, p, "Theme", true, |ui, rows| {
        let rect = rows.custom(ui, 132.0);
        let themes = [
            (Theme::Graphite, "Graphite"),
            (Theme::Dusk, "Dusk"),
            (Theme::Light, "Light"),
        ];
        let gap = 12.0;
        let inner = rect.shrink2(vec2(14.0, 12.0));
        let width = ((inner.width() - gap * 2.0) / 3.0).min(132.0);
        let start = inner.center().x - (width * 3.0 + gap * 2.0) * 0.5;
        for (index, (theme, label)) in themes.into_iter().enumerate() {
            let cell = Rect::from_min_size(
                pos2(start + index as f32 * (width + gap), inner.top()),
                vec2(width, inner.height()),
            );
            if theme_choice(
                ui,
                p,
                cell,
                theme,
                label,
                config.theme == theme,
                config.accent,
            ) && config.theme != theme
            {
                config.theme = theme;
                *changed = true;
            }
        }
        rows.note(
            ui,
            "Matches Neptune Terminal's Graphite, Dusk and Light materials.",
        );
    });
    card(ui, p, "Accent color", false, |ui, rows| {
        rows.row(ui, "Accent", |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (accent, label) in [
                (Accent::Graphite, "Graphite"),
                (Accent::Green, "Green"),
                (Accent::Yellow, "Yellow"),
                (Accent::Orange, "Orange"),
                (Accent::Red, "Red"),
                (Accent::Pink, "Pink"),
                (Accent::Purple, "Purple"),
                (Accent::Indigo, "Indigo"),
                (Accent::Blue, "Blue"),
            ] {
                if swatch(ui, p, accent, label, config.accent == accent) && config.accent != accent
                {
                    config.accent = accent;
                    *changed = true;
                }
            }
        });
        rows.note(ui, "Marks selection, focus and the primary action.");
    });
}

fn files(ui: &mut Ui, p: Palette, config: &mut Config, changed: &mut bool) {
    card(ui, p, "Browsing", true, |ui, rows| {
        rows.row(ui, "Default view", |ui| {
            *changed |= controls::segmented(
                ui,
                p,
                "Default view",
                &mut config.view,
                &[(ViewMode::List, "List"), (ViewMode::Grid, "Icons")],
                150.0,
            );
        });
        rows.row(ui, "Show hidden files", |ui| {
            *changed |=
                controls::toggle(ui, p, &mut config.show_hidden, "Show hidden files").changed();
        });
    });
    card(ui, p, "Sizes", false, |ui, rows| {
        for (label, value, range, step) in [
            (
                "List row height",
                &mut config.row_height,
                Config::ROW_RANGE,
                2.0,
            ),
            ("Icon size", &mut config.icon_size, Config::ICON_RANGE, 8.0),
            (
                "Sidebar width",
                &mut config.sidebar_width,
                Config::SIDEBAR_RANGE,
                8.0,
            ),
        ] {
            rows.row(ui, label, |ui| {
                let text = format!("{value:.0} pt");
                *changed |= controls::stepper(ui, p, label, value, range.0..=range.1, step, &text);
            });
        }
        rows.note(ui, "Changes apply immediately.");
    });
}

/// A theme drawn in its own materials: a small window with a sidebar, a
/// content surface and a selected row. The one in use carries the accent ring.
#[allow(clippy::too_many_arguments)]
fn theme_choice(
    ui: &mut Ui,
    current: Palette,
    cell: Rect,
    theme: Theme,
    label: &str,
    selected: bool,
    accent: Accent,
) -> bool {
    let response = ui.interact(cell, Id::new(("theme-choice", label)), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, true, selected, label));
    let t = Palette::with_accent(&theme, accent);
    let painter = ui.painter();
    let window = Rect::from_min_size(cell.min, vec2(cell.width(), cell.height() - 26.0));
    painter.rect_filled(window, 8, t.chrome);
    painter.rect_stroke(window, 8, Stroke::new(1.0, t.border), StrokeKind::Inside);
    for (index, rgb) in [0xff5f57, 0xfebc2e, 0x28c840].into_iter().enumerate() {
        painter.circle_filled(
            pos2(window.left() + 9.0 + index as f32 * 7.0, window.top() + 9.0),
            2.2,
            theme::color(rgb),
        );
    }
    let sidebar_width = window.width() * 0.3;
    for row in 0..3 {
        painter.rect_filled(
            Rect::from_min_size(
                pos2(window.left() + 6.0, window.top() + 20.0 + row as f32 * 10.0),
                vec2(sidebar_width - 10.0, 5.0),
            ),
            2,
            if row == 0 { t.pressed } else { t.control },
        );
    }
    let surface = Rect::from_min_max(
        pos2(window.left() + sidebar_width, window.top() + 16.0),
        window.max - vec2(4.0, 4.0),
    );
    painter.rect_filled(surface, 5, t.bg);
    for row in 0..4 {
        let y = surface.top() + 6.0 + row as f32 * 11.0;
        if y + 8.0 > surface.bottom() {
            break;
        }
        let line = Rect::from_min_size(
            pos2(surface.left() + 4.0, y),
            vec2(surface.width() - 8.0, 8.0),
        );
        if row == 1 {
            painter.rect_filled(
                line,
                2,
                theme::tint(t.accent, if t.dark { 0.32 } else { 0.2 }),
            );
        }
        painter.circle_filled(pos2(line.left() + 5.0, line.center().y), 2.5, t.accent);
        painter.rect_filled(
            Rect::from_min_size(
                pos2(line.left() + 11.0, line.center().y - 1.5),
                vec2(line.width() * (0.55 - row as f32 * 0.07), 3.0),
            ),
            1,
            t.secondary,
        );
    }
    if selected {
        painter.rect_stroke(
            window.expand(3.0),
            11,
            Stroke::new(2.0, current.accent),
            StrokeKind::Inside,
        );
    } else if response.hovered() {
        painter.rect_stroke(
            window.expand(3.0),
            11,
            Stroke::new(2.0, current.border),
            StrokeKind::Inside,
        );
    }
    if response.has_focus() {
        controls::focus_ring(painter, window, 8, current);
    }
    painter.text(
        pos2(cell.center().x, cell.bottom() - 9.0),
        Align2::CENTER_CENTER,
        label,
        if selected {
            theme::medium(12.0)
        } else {
            theme::regular(12.0)
        },
        if selected {
            current.fg
        } else {
            current.secondary
        },
    );
    response.on_hover_cursor(CursorIcon::PointingHand).clicked()
}

/// One accent as a coloured disc, as a native settings window offers it.
fn swatch(ui: &mut Ui, p: Palette, accent: Accent, label: &str, selected: bool) -> bool {
    let (_, rect) = ui.allocate_space(Vec2::splat(20.0));
    let response = ui.interact(rect, Id::new(("accent", label)), Sense::click());
    response.widget_info(|| {
        WidgetInfo::selected(
            WidgetType::RadioButton,
            true,
            selected,
            format!("{label} accent"),
        )
    });
    let painter = ui.painter();
    let fill = theme::accent_color(accent, p.dark);
    let center = rect.center();
    if selected {
        painter.circle_stroke(center, 10.0, Stroke::new(2.0, theme::tint(fill, 0.55)));
    }
    painter.circle_filled(center, 7.5, fill);
    painter.circle_stroke(center, 7.5, Stroke::new(0.5, Color32::from_black_alpha(40)));
    if selected {
        painter.circle_filled(center, 2.5, Color32::WHITE);
    } else if response.hovered() {
        painter.circle_stroke(center, 9.5, Stroke::new(1.0, p.border));
    }
    if response.has_focus() {
        painter.circle_stroke(center, 11.5, Stroke::new(2.0, theme::tint(p.accent, 0.55)));
    }
    response
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(label)
        .clicked()
}

/// Quick Look's title row: the file's icon and name, with the close control.
fn quick_look_header(ui: &mut Ui, p: Palette, path: &std::path::Path) -> bool {
    let (_, rect) = ui.allocate_space(vec2(ui.available_width(), 52.0));
    let painter = ui.painter();
    icons::paint(
        painter,
        Rect::from_center_size(pos2(rect.left() + 28.0, rect.center().y), Vec2::splat(18.0)),
        Icon::Eyes,
        p.secondary,
    );
    controls::galley_at(
        painter,
        pos2(rect.left() + 46.0, rect.center().y),
        controls::elided(
            painter,
            &super::name(path),
            theme::semibold(15.0),
            p.fg,
            rect.width() - 100.0,
        ),
    );
    controls::place(
        ui,
        Rect::from_center_size(
            pos2(rect.right() - 26.0, rect.center().y),
            Vec2::splat(28.0),
        ),
        Layout::left_to_right(Align::Center),
        "quick-look-close",
        |ui| icons::button(ui, Icon::Close, "Close Quick Look").clicked(),
    )
}
