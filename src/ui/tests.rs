//! Behavioral proofs for bounded rendering and keyboard/confirmation ownership.
//! Native screenshots remain the visual acceptance test.

use super::*;
use crate::model::{Pane, Tab};
use eframe::egui::vec2;
use finder_core::{Entry, EntryKind};
use std::time::SystemTime;

struct Harness {
    ctx: egui::Context,
    workspace: Workspace,
    config: Config,
    state: UiState,
    time: f64,
    viewport: egui::ViewportInfo,
}

impl Harness {
    fn new(entries: usize) -> Self {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        let config = Config {
            sidebar: false,
            ..Config::default()
        };
        crate::theme::apply(&ctx, &config);
        let path = if cfg!(windows) {
            PathBuf::from("C:\\Users\\Neptune\\Documents")
        } else {
            PathBuf::from("/home/neptune/Documents")
        };
        let mut workspace = Workspace::new(path.clone());
        workspace.pane_mut().loading = false;
        workspace.pane_mut().entries = (0..entries)
            .map(|index| {
                let name = format!("document-{index}.txt");
                Entry {
                    path: path.join(&name),
                    name,
                    kind: EntryKind::File,
                    size: index as u64,
                    modified: Some(SystemTime::UNIX_EPOCH),
                    hidden: false,
                }
            })
            .collect();
        Self {
            ctx,
            workspace,
            config,
            state: UiState::new(path),
            time: 0.0,
            viewport: egui::ViewportInfo::default(),
        }
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> (Vec<Action>, egui::FullOutput) {
        self.frame_at(events, vec2(1_024.0, 720.0), None)
    }

    fn frame_at(
        &mut self,
        events: Vec<egui::Event>,
        size: Vec2,
        focus: Option<egui::Id>,
    ) -> (Vec<Action>, egui::FullOutput) {
        self.time += 1.0 / 60.0;
        let mut input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
            time: Some(self.time),
            events,
            focused: true,
            ..egui::RawInput::default()
        };
        input
            .viewports
            .insert(egui::ViewportId::ROOT, self.viewport.clone());
        let mut actions = Vec::new();
        let mut output = self.ctx.run_ui(input, |ui| {
            if let Some(id) = focus {
                ui.ctx().memory_mut(|memory| memory.request_focus(id));
            }
            super::show(
                ui,
                &mut self.workspace,
                &mut self.config,
                &mut self.state,
                &mut actions,
            );
        });
        // This harness checks behavior and shape budgets without a GPU painter.
        // Explicitly discard its texture commands as required by egui 0.36.
        output.textures_delta.clear();
        (actions, output)
    }

    fn warm(&mut self) {
        self.frame(Vec::new());
        self.frame(Vec::new());
    }

    fn focused_file(&self) -> egui::Id {
        *self
            .state
            .file_widgets
            .iter()
            .next()
            .expect("visible file response")
    }
}

/// Check the actual tessellated UI, including child panels and overlay layers.
/// A rounded background alone cannot prove that later shapes leave it clear.
fn painted_at(primitives: &[egui::ClippedPrimitive], point: egui::Pos2) -> bool {
    primitives.iter().any(|primitive| {
        if !primitive.clip_rect.contains(point) {
            return false;
        }
        let egui::epaint::Primitive::Mesh(mesh) = &primitive.primitive else {
            panic!("unexpected paint callback in window chrome");
        };
        mesh.indices.chunks_exact(3).any(|indices| {
            let [a, b, c] =
                [indices[0], indices[1], indices[2]].map(|index| mesh.vertices[index as usize]);
            if [a, b, c].iter().all(|vertex| vertex.color.a() == 0) {
                return false;
            }
            let cross = |u: Vec2, v: Vec2| u.x * v.y - u.y * v.x;
            let area = cross(b.pos - a.pos, c.pos - a.pos);
            if area.abs() < f32::EPSILON {
                return false;
            }
            let u = cross(b.pos - point, c.pos - point) / area;
            let v = cross(c.pos - point, a.pos - point) / area;
            let w = 1.0 - u - v;
            u >= 0.0 && v >= 0.0 && w >= 0.0
        })
    })
}

#[test]
fn window_chrome_keeps_the_rounded_corners_clear() {
    use crate::config::Theme;
    for theme in [Theme::Graphite, Theme::Dusk, Theme::Light] {
        for size in [vec2(1_024.0, 720.0), vec2(520.0, 420.0)] {
            for sidebar in [false, true] {
                let mut harness = Harness::new(5);
                harness.config.theme = theme;
                harness.config.sidebar = sidebar;
                crate::theme::apply(&harness.ctx, &harness.config);
                for view in [ViewMode::List, ViewMode::Grid] {
                    harness.config.view = view;
                    for (maximized, fullscreen) in [
                        (false, false),
                        (true, false),
                        (false, false),
                        (false, true),
                        (false, false),
                    ] {
                        harness.viewport.maximized = Some(maximized);
                        harness.viewport.fullscreen = Some(fullscreen);
                        for dialog in [false, true] {
                            harness.state.dialog = dialog.then_some(Dialog::Preferences);
                            harness.frame_at(Vec::new(), size, None);
                            let (_, output) = harness.frame_at(Vec::new(), size, None);
                            let primitives = harness.ctx.tessellate(output.shapes, 1.0);
                            let rounded = !cfg!(windows)
                                && !fullscreen
                                && (!maximized || cfg!(target_os = "macos"));
                            for point in [
                                egui::pos2(2.5, 2.5),
                                egui::pos2(size.x - 2.5, 2.5),
                                egui::pos2(2.5, size.y - 2.5),
                                egui::pos2(size.x - 2.5, size.y - 2.5),
                            ] {
                                assert_eq!(
                                    painted_at(&primitives, point),
                                    !rounded,
                                    "window corner {point:?}: {theme:?}, {size:?}, sidebar={sidebar}, \
                                     {view:?}, maximized={maximized}, fullscreen={fullscreen}, dialog={dialog}"
                                );
                            }
                            assert!(painted_at(&primitives, egui::pos2(2.5, size.y / 2.0)));
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn window_edges_and_all_four_corners_start_native_resize() {
    use egui::ResizeDirection::*;
    let size = vec2(1_024.0, 720.0);
    for (direction, point) in [
        (North, egui::pos2(size.x / 2.0, 2.0)),
        (South, egui::pos2(size.x / 2.0, size.y - 2.0)),
        (West, egui::pos2(2.0, size.y / 2.0)),
        (East, egui::pos2(size.x - 2.0, size.y / 2.0)),
        (NorthWest, egui::pos2(4.0, 4.0)),
        (NorthEast, egui::pos2(size.x - 4.0, 4.0)),
        (SouthWest, egui::pos2(4.0, size.y - 4.0)),
        (SouthEast, egui::pos2(size.x - 4.0, size.y - 4.0)),
    ] {
        let mut harness = Harness::new(0);
        harness.warm();
        let (_, pressed) = harness.frame(vec![
            egui::Event::PointerMoved(point),
            egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ]);
        let moved = point + (size / 2.0 - point.to_vec2()).normalized() * 20.0;
        let (_, output) = harness.frame(vec![egui::Event::PointerMoved(moved)]);
        assert!(
            [pressed, output].iter().any(|frame| frame.viewport_output[&egui::ViewportId::ROOT]
                .commands.iter()
                .any(|command| matches!(command, egui::ViewportCommand::BeginResize(actual) if *actual == direction))),
            "dragging {direction:?} did not start a native resize"
        );
    }
}

fn key(key: Key, pressed: bool, repeat: bool, modifiers: Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat,
        modifiers,
    }
}

fn press(key_value: Key) -> egui::Event {
    key(key_value, true, false, Modifiers::NONE)
}
fn release(key_value: Key) -> egui::Event {
    key(key_value, false, false, Modifiers::NONE)
}

fn file_action(action: &Action) -> bool {
    matches!(
        action,
        Action::Open(..) | Action::QuickLook(..) | Action::Navigate(..)
    )
}

#[test]
fn hundred_thousand_items_create_only_visible_list_and_grid_widgets() {
    let mut harness = Harness::new(100_000);
    for view in [ViewMode::List, ViewMode::Grid] {
        harness.config.view = view;
        harness.warm();
        let (_, output) = harness.frame(Vec::new());
        let visible = harness.state.file_widgets.len();
        assert!(visible > 0, "file view rendered no interactive rows");
        assert!(
            visible < 100,
            "file view created {visible} rows for one viewport"
        );
        assert!(
            output.shapes.len() < 2_000,
            "file view submitted {} shapes for one viewport",
            output.shapes.len()
        );
        assert_eq!(harness.workspace.pane().entries.len(), 100_000);
    }
}

#[test]
fn chrome_focus_owns_enter_space_and_arrows() {
    let mut harness = Harness::new(20);
    harness.workspace.pane_mut().select(0, false, false);
    harness.warm();
    let selected = harness.workspace.pane().selected.clone();
    let chrome = egui::Id::new("a-focused-chrome-control");
    assert!(!harness.state.file_widgets.contains(&chrome));
    for key_value in [Key::Enter, Key::Space, Key::ArrowDown, Key::ArrowRight] {
        let (actions, _) =
            harness.frame_at(vec![press(key_value)], vec2(1_024.0, 720.0), Some(chrome));
        assert!(
            !actions.iter().any(file_action),
            "chrome key leaked to file actions"
        );
        assert_eq!(harness.workspace.pane().selected, selected);
        assert_eq!(harness.workspace.pane().focus_index, Some(0));
        harness.frame(vec![release(key_value)]);
    }
}

#[test]
fn a_focused_file_row_opens_previews_and_navigates_the_selected_file() {
    let mut harness = Harness::new(20);
    harness.workspace.pane_mut().select(0, false, false);
    harness.warm();
    let selected = harness.workspace.pane().entries[0].path.clone();
    let row = harness.focused_file();
    let (actions, _) = harness.frame_at(vec![press(Key::Enter)], vec2(1_024.0, 720.0), Some(row));
    assert!(
        actions
            .iter()
            .any(|action| matches!(action,Action::Open(_,path,false) if path==&selected))
    );
    harness.frame(vec![release(Key::Enter)]);
    let row = harness.focused_file();
    let (actions, _) = harness.frame_at(vec![press(Key::Space)], vec2(1_024.0, 720.0), Some(row));
    assert!(
        actions
            .iter()
            .any(|action| matches!(action,Action::QuickLook(path) if path==&selected))
    );
    harness.frame(vec![release(Key::Space)]);
    let row = harness.focused_file();
    harness.frame_at(vec![press(Key::ArrowDown)], vec2(1_024.0, 720.0), Some(row));
    assert_eq!(harness.workspace.pane().focus_index, Some(1));
    assert!(
        harness
            .workspace
            .pane()
            .selected
            .contains(&harness.workspace.pane().entries[1].path)
    );
}

#[test]
fn grid_arrow_navigation_uses_the_actual_visible_column_count() {
    let mut harness = Harness::new(50);
    harness.config.view = ViewMode::Grid;
    harness.workspace.pane_mut().select(0, false, false);
    harness.warm();
    let columns = harness.state.grid_columns[&harness.workspace.pane().id];
    assert!(columns > 1);
    let row = harness.focused_file();
    harness.frame_at(vec![press(Key::ArrowDown)], vec2(1_024.0, 720.0), Some(row));
    assert_eq!(harness.workspace.pane().focus_index, Some(columns));
}

#[test]
fn keyboard_focus_in_the_other_split_targets_that_panes_selection() {
    let mut harness = Harness::new(5);
    harness.workspace.pane_mut().select(0, false, false);
    let path = harness.workspace.pane().path.join("Other");
    let mut other = Pane::new(9, path.clone());
    let mut entry = harness.workspace.pane().entries[0].clone();
    entry.name = "other-pane.txt".into();
    entry.path = path.join(&entry.name);
    let target = entry.path.clone();
    other.entries.push(entry);
    other.loading = false;
    other.select(0, false, false);
    harness.workspace.tabs[0].panes.push(other);
    harness.warm();
    assert_ne!(harness.workspace.pane().id, 9);
    let row = harness
        .state
        .file_owners
        .iter()
        .find_map(|(widget, pane)| (*pane == 9).then_some(*widget))
        .expect("other split's visible row");
    let (actions, _) = harness.frame_at(vec![press(Key::Enter)], vec2(1_024.0, 720.0), Some(row));
    assert_eq!(harness.workspace.pane().id, 9);
    assert!(
        actions
            .iter()
            .any(|action| { matches!(action, Action::Open(9, path, false) if path == &target) })
    );
    assert!(
        !actions
            .iter()
            .any(|action| { matches!(action, Action::Open(pane, _, _) if *pane != 9) })
    );
}

#[test]
fn focus_from_a_hidden_tab_cannot_activate_the_new_tabs_files() {
    let mut harness = Harness::new(5);
    harness.workspace.pane_mut().select(0, false, false);
    harness.warm();
    let old_row = harness.focused_file();
    let path = harness.workspace.pane().path.join("New tab");
    let mut pane = Pane::new(11, path.clone());
    let mut entry = harness.workspace.pane().entries[0].clone();
    entry.path = path.join(&entry.name);
    pane.entries.push(entry);
    pane.loading = false;
    pane.select(0, false, false);
    harness.workspace.tabs.push(Tab {
        id: 10,
        panes: vec![pane],
        active: 0,
    });
    harness.workspace.active = 1;
    harness.warm();
    assert!(!harness.state.file_owners.contains_key(&old_row));
    let (actions, _) =
        harness.frame_at(vec![press(Key::Enter)], vec2(1_024.0, 720.0), Some(old_row));
    assert!(!actions.iter().any(file_action));
    assert_eq!(harness.workspace.active, 1);
    assert_eq!(harness.workspace.pane().id, 11);
}

#[test]
fn search_editing_keeps_select_all_copy_and_cut_out_of_file_actions() {
    let mut harness = Harness::new(20);
    harness.workspace.pane_mut().select(0, false, false);
    harness.workspace.pane_mut().query = "needle".into();
    harness.state.focus_search = true;
    harness.warm();
    assert!(
        harness.ctx.text_edit_focused(),
        "search never took keyboard focus"
    );
    let selected = harness.workspace.pane().selected.clone();
    for (key_value, event) in [
        (Key::A, None),
        (Key::C, Some(egui::Event::Copy)),
        (Key::X, Some(egui::Event::Cut)),
    ] {
        let mut events = vec![key(key_value, true, false, Modifiers::COMMAND)];
        if let Some(event) = event {
            events.push(event);
        }
        let (actions, _) = harness.frame(events);
        assert!(!actions.iter().any(|action| matches!(
            action,
            Action::Copy(..) | Action::Paste(..) | Action::Submit(..)
        )));
        assert_eq!(harness.workspace.pane().selected, selected);
        harness.frame(vec![key(key_value, false, false, Modifiers::COMMAND)]);
    }
    assert!(
        harness.workspace.pane().query.is_empty(),
        "cut did not act on the search text"
    );
}

#[test]
fn opening_or_repeated_enter_cannot_confirm_new_item_or_trash() {
    for trash in [false, true] {
        let mut harness = Harness::new(5);
        let original = harness.workspace.pane().entries[0].path.clone();
        harness.state.open_dialog(if trash {
            Dialog::Trash {
                paths: vec![original.clone()],
            }
        } else {
            Dialog::New {
                parent: harness.workspace.pane().path.clone(),
                name: "planned-folder".into(),
                folder: true,
            }
        });
        let (actions, _) = harness.frame(vec![press(Key::Enter)]);
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, Action::Submit(..)))
        );
        assert!(
            harness.state.dialog.is_some(),
            "opening Enter dismissed confirmation"
        );
        // Keep the opening key down: egui derives repeat from keys_down rather
        // than trusting a synthetic event's repeat flag after a key release.
        harness.warm();
        let (actions, _) = harness.frame(vec![key(Key::Enter, true, true, Modifiers::NONE)]);
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, Action::Submit(..)))
        );
        assert!(
            harness.state.dialog.is_some(),
            "repeated Enter confirmed a dialog"
        );
        harness.frame(vec![release(Key::Enter)]);
        let (actions, _) = harness.frame(vec![press(Key::Enter)]);
        assert!(
            actions.iter().any(|action| match action {
                Action::Submit(Operation::Trash { paths }) =>
                    trash && paths == &vec![original.clone()],
                Action::Submit(Operation::CreateFolder { name, .. }) =>
                    !trash && name == "planned-folder",
                _ => false,
            }),
            "a fresh Enter did not confirm the retained target"
        );
        assert!(harness.state.dialog.is_none());
    }
}

#[test]
fn rename_confirmation_retains_its_original_source_after_navigation() {
    let mut harness = Harness::new(3);
    let source = harness.workspace.pane().entries[0].path.clone();
    harness.state.open_dialog(Dialog::Rename {
        source: source.clone(),
        name: "renamed.txt".into(),
    });
    harness
        .workspace
        .pane_mut()
        .navigate(source.parent().unwrap().join("another-folder"));
    harness.warm();
    let (actions, _) = harness.frame(vec![press(Key::Enter)]);
    assert!(actions.iter().any(|action|matches!(action,Action::Submit(Operation::Rename {source:target,name}) if target==&source&&name=="renamed.txt")));
}

/// Presses at `from`, keeps the button down for `hold` seconds, then drags to
/// `to` and releases there. Returns every action the gesture emitted.
fn drag(harness: &mut Harness, from: egui::Pos2, to: egui::Pos2, hold: f64) -> Vec<Action> {
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    let mut actions = Vec::new();
    actions.extend(
        harness
            .frame(vec![egui::Event::PointerMoved(from), button(from, true)])
            .0,
    );
    harness.time += hold;
    actions.extend(harness.frame(Vec::new()).0);
    actions.extend(harness.frame(vec![egui::Event::PointerMoved(to)]).0);
    actions.extend(harness.frame(vec![button(to, false)]).0);
    actions.extend(harness.frame(Vec::new()).0);
    actions
}

#[test]
fn a_drag_released_in_its_own_folder_or_on_itself_does_nothing() {
    let mut harness = Harness::new(3);
    let folder = harness.workspace.pane().path.join("Projects");
    harness.workspace.pane_mut().entries.insert(
        0,
        Entry {
            path: folder.clone(),
            name: "Projects".into(),
            kind: EntryKind::Directory,
            size: 0,
            modified: Some(SystemTime::UNIX_EPOCH),
            hidden: false,
        },
    );
    harness.warm();
    let mut rows: Vec<Rect> = harness
        .state
        .file_widgets
        .iter()
        .filter_map(|id| harness.ctx.read_response(*id))
        .map(|response| response.rect)
        .collect();
    rows.sort_by(|a, b| a.top().total_cmp(&b.top()));
    let submitted = |actions: &[Action]| {
        actions
            .iter()
            .any(|action| matches!(action, Action::Submit(_)))
    };
    // A slow click turns into a drag without moving; releasing it in place
    // used to copy the file onto itself and report a collision.
    let file = rows[1].center();
    assert!(!submitted(&drag(&mut harness, file, file, 1.0)));
    assert!(!submitted(&drag(&mut harness, file, rows[2].center(), 0.0)));
    // A folder is no target for itself.
    let own = rows[0].center();
    let beside = own + vec2(40.0, 0.0);
    assert!(!submitted(&drag(&mut harness, own, beside, 0.0)));
    let source = harness.workspace.pane().entries[1].path.clone();
    let actions = drag(&mut harness, file, own, 0.0);
    assert!(
        actions.iter().any(|action| matches!(
            action,
            Action::Submit(Operation::Copy { sources, destination })
                if sources == std::slice::from_ref(&source) && destination == &folder
        )),
        "a file dropped on a folder was not copied into it"
    );
}

#[test]
fn narrow_split_views_and_scrollable_preferences_remain_bounded() {
    let mut harness = Harness::new(100);
    let pane = Pane::new(9, harness.workspace.pane().path.clone());
    harness.workspace.tabs[0].panes.push(pane);
    let size = vec2(420.0, 420.0);
    for view in [ViewMode::List, ViewMode::Grid] {
        harness.config.view = view;
        let (_, output) = harness.frame_at(Vec::new(), size, None);
        assert!(harness.state.file_widgets.len() < 100);
        assert!(output.shapes.len() < 2_000);
    }
    harness.state.open_dialog(Dialog::Preferences);
    harness.frame_at(Vec::new(), size, None);
    let (_, output) = harness.frame_at(Vec::new(), size, None);
    assert!(harness.state.dialog.is_some());
    assert!(output.shapes.len() < 2_000);
    assert!(
        output
            .shapes
            .iter()
            .all(|shape| shape.clip_rect.is_finite())
    );
    harness.frame_at(vec![press(Key::Escape)], size, None);
    assert!(
        harness.state.dialog.is_none(),
        "preferences could not close in a narrow window"
    );
}

#[test]
fn zoom_chords_step_the_saved_zoom_over_files_fields_and_dialogs() {
    let zooms = |actions: &[Action]| {
        actions
            .iter()
            .filter_map(|action| match action {
                Action::Zoom(step) => Some(*step),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    for owner in ["files", "search", "dialog"] {
        let mut harness = Harness::new(5);
        match owner {
            "search" => harness.state.focus_search = true,
            "dialog" => harness.state.open_dialog(Dialog::Preferences),
            _ => {}
        }
        harness.warm();
        assert_eq!(harness.ctx.text_edit_focused(), owner == "search");
        for (key_value, modifiers, step) in [
            (Key::Equals, Modifiers::COMMAND, ZoomStep::In),
            (
                Key::Plus,
                Modifiers::COMMAND | Modifiers::SHIFT,
                ZoomStep::In,
            ),
            (Key::Minus, Modifiers::COMMAND, ZoomStep::Out),
            (Key::Num0, Modifiers::COMMAND, ZoomStep::Reset),
        ] {
            let (actions, _) = harness.frame(vec![key(key_value, true, false, modifiers)]);
            assert_eq!(zooms(&actions), vec![step], "{key_value:?} over {owner}");
            harness.frame(vec![key(key_value, false, false, modifiers)]);
            // The app owns the zoom, so the toolkit's own shortcut stays out.
            assert_eq!(harness.ctx.zoom_factor(), 1.0);
        }
        assert_eq!(harness.state.dialog.is_some(), owner == "dialog");
    }
}

#[test]
fn a_zoom_chip_shows_its_value_then_leaves() {
    let mut harness = Harness::new(5);
    harness.warm();
    harness.state.level = Some(crate::controls::Level {
        name: "Window zoom",
        value: "110%".into(),
        shown: harness.time,
    });
    harness.frame(Vec::new());
    assert!(harness.state.level.is_some());
    harness.time += 2.0;
    harness.frame(Vec::new());
    assert!(harness.state.level.is_none());
}
