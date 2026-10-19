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
    /// Whether the window has the operating system's keyboard focus.
    window_focused: bool,
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
            window_focused: true,
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
            focused: self.window_focused,
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

    /// Bounds of the visible rows or icons, in reading order.
    fn item_rects(&self) -> Vec<Rect> {
        let mut rects = self
            .state
            .file_widgets
            .iter()
            .filter_map(|id| self.ctx.read_response(*id))
            .map(|response| response.rect)
            .collect::<Vec<_>>();
        rects.sort_by(|a, b| {
            (a.top(), a.left())
                .partial_cmp(&(b.top(), b.left()))
                .unwrap()
        });
        rects
    }

    /// Presses at `from` and moves far enough to start a drag.
    fn begin_drag(&mut self, from: egui::Pos2) {
        self.frame(vec![egui::Event::PointerMoved(from), button(from, true)]);
        self.frame(vec![egui::Event::PointerMoved(from + vec2(0.0, 10.0))]);
    }

    fn selected_rows(&self) -> Vec<usize> {
        let pane = self.workspace.pane();
        (0..pane.entries.len())
            .filter(|&i| pane.selected.contains(&pane.entries[i].path))
            .collect()
    }
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

/// Midway between two neighbouring icons.
fn gap(left: Rect, right: Rect) -> f32 {
    (left.right() + right.left()) / 2.0
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
fn the_sidebar_overlay_keeps_the_rounded_corners_clear_while_it_slides() {
    use crate::config::Theme;
    let size = vec2(600.0, 420.0);
    let rounded = !cfg!(windows);
    for theme in [Theme::Graphite, Theme::Dusk, Theme::Light] {
        let mut harness = Harness::new(5);
        harness.config.theme = theme;
        crate::theme::apply(&harness.ctx, &harness.config);
        harness.frame_at(Vec::new(), size, None);
        // Every frame of the slide in and out, including the first, when
        // only a sliver of the overlay is inside the window.
        for open in [true, false] {
            harness.state.sidebar_overlay = open;
            for frame in 0..14 {
                let (_, output) = harness.frame_at(Vec::new(), size, None);
                let primitives = harness.ctx.tessellate(output.shapes, 1.0);
                for point in [
                    egui::pos2(2.5, 2.5),
                    egui::pos2(size.x - 2.5, 2.5),
                    egui::pos2(2.5, size.y - 2.5),
                    egui::pos2(size.x - 2.5, size.y - 2.5),
                ] {
                    assert_eq!(
                        painted_at(&primitives, point),
                        !rounded,
                        "window corner {point:?}: {theme:?}, open={open}, frame {frame}"
                    );
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
fn dragging_beside_names_or_between_icons_selects_what_the_rectangle_covers() {
    for view in [ViewMode::List, ViewMode::Grid] {
        let mut harness = Harness::new(40);
        harness.config.view = view;
        harness.workspace.pane_mut().select(0, false, false);
        harness.warm();
        let items = harness.item_rects();
        let (from, to, expected) = if view == ViewMode::List {
            // From the second row's Kind column to the fifth's.
            (
                egui::pos2(items[1].right() - 40.0, items[1].center().y),
                egui::pos2(items[4].right() - 40.0, items[4].center().y),
                vec![1, 2, 3, 4],
            )
        } else {
            let columns = harness.state.grid_columns[&harness.workspace.pane().id];
            assert!(columns >= 4);
            (
                egui::pos2(gap(items[0], items[1]), items[0].top() + 10.0),
                egui::pos2(
                    gap(items[columns + 2], items[columns + 3]),
                    items[columns + 2].center().y,
                ),
                vec![1, 2, columns + 1, columns + 2],
            )
        };
        harness.begin_drag(from);
        harness.frame(vec![egui::Event::PointerMoved(to)]);
        assert_eq!(harness.selected_rows(), expected, "{view:?}");
        assert!(
            !egui::DragAndDrop::has_any_payload(&harness.ctx),
            "a selection rectangle picked files up"
        );
        let (actions, _) = harness.frame(vec![button(to, false)]);
        assert!(!actions.iter().any(|a| matches!(a, Action::Submit(..))));
        assert!(!harness.workspace.pane().marquee_active());
        assert_eq!(harness.selected_rows(), expected, "{view:?}");
    }
}

#[test]
fn clicks_beside_a_name_select_the_row_and_between_icons_clear_the_selection() {
    for view in [ViewMode::List, ViewMode::Grid] {
        let mut harness = Harness::new(40);
        harness.config.view = view;
        harness.workspace.pane_mut().select(0, false, false);
        harness.warm();
        let items = harness.item_rects();
        let point = if view == ViewMode::List {
            egui::pos2(items[3].right() - 40.0, items[3].center().y)
        } else {
            egui::pos2(gap(items[0], items[1]), items[0].center().y)
        };
        harness.frame(vec![egui::Event::PointerMoved(point), button(point, true)]);
        harness.frame(vec![button(point, false)]);
        let expected = if view == ViewMode::List {
            vec![3]
        } else {
            vec![]
        };
        assert_eq!(harness.selected_rows(), expected, "{view:?}");
    }
}

#[test]
fn dragging_a_name_still_picks_up_the_files() {
    let mut harness = Harness::new(40);
    harness.workspace.pane_mut().select(0, false, false);
    harness.warm();
    let items = harness.item_rects();
    let from = egui::pos2(items[2].left() + 50.0, items[2].center().y);
    harness.begin_drag(from);
    harness.frame(vec![egui::Event::PointerMoved(from + vec2(0.0, 80.0))]);
    assert!(!harness.workspace.pane().marquee_active());
    assert_eq!(harness.selected_rows(), [2]);
    assert!(egui::DragAndDrop::has_any_payload(&harness.ctx));
}

#[test]
fn a_selected_row_is_picked_up_anywhere_on_its_highlight() {
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
    harness.workspace.pane_mut().select(1, false, false);
    harness.workspace.pane_mut().select(2, true, false);
    harness.warm();
    let rows = harness.item_rects();
    // Beside the first selected file's name, in its Kind column.
    let from = egui::pos2(rows[1].right() - 40.0, rows[1].center().y);
    harness.begin_drag(from);
    assert!(!harness.workspace.pane().marquee_active());
    assert_eq!(harness.selected_rows(), [1, 2]);
    assert!(egui::DragAndDrop::has_any_payload(&harness.ctx));
    let to = rows[0].center();
    harness.frame(vec![egui::Event::PointerMoved(to)]);
    let (actions, _) = harness.frame(vec![button(to, false)]);
    let mut expected = vec![
        harness.workspace.pane().entries[1].path.clone(),
        harness.workspace.pane().entries[2].path.clone(),
    ];
    expected.sort();
    assert!(
        actions.iter().any(|action| matches!(
            action,
            Action::Submit(Operation::Move { sources, destination })
                if destination == &folder && {
                    let mut sources = sources.clone();
                    sources.sort();
                    sources == expected
                }
        )),
        "the selection dropped on a folder was not moved into it"
    );
}

#[test]
fn a_touch_screen_leaves_drags_to_selection_and_moving_files() {
    let mut harness = Harness::new(40);
    let touch = |phase| egui::Event::Touch {
        device_id: egui::TouchDeviceId(1),
        id: egui::TouchId(1),
        phase,
        pos: egui::pos2(500.0, 400.0),
        force: None,
    };
    harness.frame(vec![touch(egui::TouchPhase::Start)]);
    harness.frame(vec![touch(egui::TouchPhase::End)]);
    assert!(harness.ctx.input(|i| i.has_touch_screen()));
    harness.warm();
    let items = harness.item_rects();
    let from = egui::pos2(items[1].right() - 40.0, items[1].center().y);
    harness.begin_drag(from);
    let to = egui::pos2(from.x, items[3].center().y);
    harness.frame(vec![egui::Event::PointerMoved(to)]);
    assert_eq!(harness.selected_rows(), [1, 2, 3]);
    harness.frame(vec![button(to, false)]);
    harness.begin_drag(egui::pos2(items[5].left() + 50.0, items[5].center().y));
    assert!(egui::DragAndDrop::has_any_payload(&harness.ctx));
}

#[test]
fn only_the_primary_button_draws_a_rectangle_or_picks_files_up() {
    let mut harness = Harness::new(40);
    harness.workspace.pane_mut().select(0, false, false);
    harness.warm();
    let items = harness.item_rects();
    for button in [egui::PointerButton::Middle, egui::PointerButton::Secondary] {
        for from in [
            egui::pos2(items[1].right() - 40.0, items[1].center().y),
            egui::pos2(items[1].left() + 50.0, items[1].center().y),
        ] {
            let to = egui::pos2(from.x, items[3].center().y);
            harness.frame(vec![
                egui::Event::PointerMoved(from),
                pointer_button(from, button, true),
            ]);
            harness.frame(vec![egui::Event::PointerMoved(from + vec2(0.0, 10.0))]);
            harness.frame(vec![egui::Event::PointerMoved(to)]);
            assert!(!harness.workspace.pane().marquee_active(), "{button:?}");
            assert!(
                !egui::DragAndDrop::has_any_payload(&harness.ctx),
                "{button:?}"
            );
            assert_eq!(harness.selected_rows(), [0], "{button:?}");
            harness.frame(vec![pointer_button(to, button, false)]);
        }
    }
}

#[test]
fn a_rectangle_past_the_bottom_scrolls_on_and_keeps_covering() {
    let mut harness = Harness::new(500);
    harness.warm();
    let visible = harness.state.file_widgets.len();
    let items = harness.item_rects();
    let from = egui::pos2(items[1].right() - 40.0, items[1].center().y);
    harness.begin_drag(from);
    let below = egui::pos2(from.x, 790.0);
    let (_, mut output) = harness.frame(vec![egui::Event::PointerMoved(below)]);
    for _ in 0..40 {
        (_, output) = harness.frame(Vec::new());
    }
    assert_eq!(
        output.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
        std::time::Duration::ZERO,
        "scrolling stopped while the pointer stayed past the edge"
    );
    let covered = harness.selected_rows();
    assert!(
        covered.len() > visible + 10,
        "covered {} rows with {visible} visible",
        covered.len()
    );
    assert_eq!(covered, (1..=covered.len()).collect::<Vec<_>>());
    harness.frame(vec![button(below, false)]);
    assert!(!harness.workspace.pane().marquee_active());
}

#[test]
fn icon_view_asks_for_visible_files_only_and_draws_their_thumbnails() {
    use crate::thumbnails::{Picture, Thumbnail};
    let mut harness = Harness::new(100_000);
    harness.config.view = ViewMode::Grid;
    harness.workspace.pane_mut().entries[0].kind = EntryKind::Directory;
    harness.frame(Vec::new());
    let wanted = harness.state.thumbnails.end_frame();
    let entries = &harness.workspace.pane().entries;
    assert!(!wanted.is_empty() && wanted.len() < 100);
    assert_eq!(wanted[0].path, entries[1].path, "folders have no thumbnail");
    let file = entries[1].clone();

    harness.state.thumbnails.insert(
        &harness.ctx,
        Thumbnail {
            key: wanted[0].clone(),
            picture: Some(Picture {
                image: egui::ColorImage::filled([8, 8], egui::Color32::WHITE),
                opaque: true,
            }),
            dimensions: None,
        },
    );
    let (_, output) = harness.frame(Vec::new());
    let texture = harness
        .state
        .thumbnails
        .get(&file, wanted[0].pixels)
        .expect("uploaded thumbnail")
        .handle
        .id();
    let primitives = harness.ctx.tessellate(output.shapes, 1.0);
    assert!(primitives.iter().any(|primitive| matches!(
        &primitive.primitive,
        egui::epaint::Primitive::Mesh(mesh) if mesh.texture_id == texture
    )));
    let wanted = harness.state.thumbnails.end_frame();
    assert!(!wanted.iter().any(|key| key.path == file.path));
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
fn a_held_enter_opens_the_selected_archive_once() {
    let mut harness = Harness::new(3);
    let archive = harness.workspace.pane().path.join("Photos.zip");
    harness.workspace.pane_mut().entries[0] = Entry {
        path: archive.clone(),
        name: "Photos.zip".into(),
        kind: EntryKind::File,
        size: 1,
        modified: Some(SystemTime::UNIX_EPOCH),
        hidden: false,
    };
    harness.workspace.pane_mut().select(0, false, false);
    harness.warm();
    let opens = |actions: &[Action]| {
        actions
            .iter()
            .filter(|action| matches!(action, Action::Open(_, path, false) if path == &archive))
            .count()
    };
    let row = harness.focused_file();
    let (actions, _) = harness.frame_at(vec![press(Key::Enter)], vec2(1_024.0, 720.0), Some(row));
    assert_eq!(opens(&actions), 1);
    for _ in 0..3 {
        let row = harness.focused_file();
        let (actions, _) = harness.frame_at(
            vec![key(Key::Enter, true, true, Modifiers::NONE)],
            vec2(1_024.0, 720.0),
            Some(row),
        );
        assert_eq!(
            opens(&actions),
            0,
            "a repeated Enter opened the archive again"
        );
    }
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

/// The list header's column dividers, leading to trailing: thin, header-high
/// handles, unlike a scroll bar.
fn column_dividers(harness: &Harness) -> Vec<Rect> {
    let mut dividers: Vec<Rect> = harness
        .ctx
        .viewport_for(egui::ViewportId::ROOT, |viewport| {
            viewport
                .prev_pass
                .widgets
                .layers()
                .flat_map(|(_, widgets)| widgets)
                .filter(|widget| {
                    widget.sense == egui::Sense::CLICK | egui::Sense::DRAG
                        && widget.rect.width() < 12.0
                        && widget.rect.height() <= 28.0
                })
                .map(|widget| widget.rect)
                .collect()
        });
    dividers.sort_by(|a, b| a.left().total_cmp(&b.left()));
    dividers
}

fn pointer(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

#[test]
fn column_dividers_follow_the_pointer_and_save_on_release() {
    use crate::config::ColumnWidths;
    let mut harness = Harness::new(20);
    harness.warm();
    let dividers = column_dividers(&harness);
    assert_eq!(dividers.len(), 3, "Name, Date Modified, Size and Kind");
    let start = dividers[1].center();
    let mut actions = Vec::new();
    actions.extend(
        harness
            .frame(vec![egui::Event::PointerMoved(start), pointer(start, true)])
            .0,
    );
    // Date widens; Size gives way to its minimum, then Kind.
    actions.extend(
        harness
            .frame(vec![egui::Event::PointerMoved(start + vec2(40.0, 0.0))])
            .0,
    );
    let widths = harness.config.column_widths;
    assert_eq!(
        (widths.date, widths.size, widths.kind),
        (208.0, 64.0, 124.0)
    );
    harness.frame(Vec::new());
    assert_eq!(column_dividers(&harness)[1].center().x, start.x + 40.0);
    // Returning within the same drag restores what it pushed.
    actions.extend(harness.frame(vec![egui::Event::PointerMoved(start)]).0);
    assert_eq!(harness.config.column_widths, ColumnWidths::default());
    // Movement lands on whole pixels.
    let end = start - vec2(20.6, 0.0);
    actions.extend(harness.frame(vec![egui::Event::PointerMoved(end)]).0);
    assert!(
        !actions
            .iter()
            .any(|action| matches!(action, Action::PreferencesChanged | Action::Sort(..)))
    );
    let (released, _) = harness.frame(vec![pointer(end, false)]);
    assert!(
        released
            .iter()
            .any(|action| matches!(action, Action::PreferencesChanged))
    );
    let widths = harness.config.column_widths;
    assert_eq!(
        (widths.date, widths.size, widths.kind),
        (147.0, 99.0, 150.0)
    );

    // A click on a divider never sorts; a double click restores the defaults.
    harness.frame(Vec::new());
    let divider = column_dividers(&harness)[1].center();
    let mut clicks = Vec::new();
    for pressed in [true, false, true, false] {
        clicks.extend(harness.frame(vec![pointer(divider, pressed)]).0);
    }
    assert!(
        !clicks
            .iter()
            .any(|action| matches!(action, Action::Sort(..)))
    );
    assert_eq!(harness.config.column_widths, ColumnWidths::default());
}

#[test]
fn columns_give_way_so_names_stay_legible_in_narrow_panes() {
    use crate::config::ColumnWidths;
    let mut harness = Harness::new(20);
    let wide = ColumnWidths {
        date: 480.0,
        size: 240.0,
        kind: 480.0,
    };
    harness.config.column_widths = wide;
    harness.warm();
    // Date, the nearest to Name, gives way first; the saved widths remain.
    let dividers = column_dividers(&harness);
    assert_eq!(dividers.len(), 3);
    assert_eq!(dividers[1].center().x - dividers[0].center().x, 160.0);
    assert_eq!(dividers[2].center().x - dividers[1].center().x, 240.0);
    assert_eq!(harness.config.column_widths, wide);
    // Hidden columns take their dividers with them.
    for (width, shown) in [(640.0, 2), (420.0, 1), (300.0, 0)] {
        let size = vec2(width, 480.0);
        harness.frame_at(Vec::new(), size, None);
        harness.frame_at(Vec::new(), size, None);
        assert_eq!(column_dividers(&harness).len(), shown, "{width}");
    }
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

/// The centre of the widget AccessKit names `label` in the frame's output.
fn widget_center(output: &egui::FullOutput, label: &str) -> Option<egui::Pos2> {
    let update = output.platform_output.accesskit_update.as_ref()?;
    update.nodes.iter().find_map(|(_, node)| {
        let bounds = node.bounds().filter(|_| node.label() == Some(label))?;
        Some(egui::pos2(
            ((bounds.x0 + bounds.x1) * 0.5) as f32,
            ((bounds.y0 + bounds.y1) * 0.5) as f32,
        ))
    })
}

fn click_events(at: egui::Pos2) -> [Vec<egui::Event>; 2] {
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    [
        vec![egui::Event::PointerMoved(at), button(true)],
        vec![button(false)],
    ]
}

fn location_actions(actions: &[Action]) -> Vec<String> {
    actions
        .iter()
        .filter_map(|action| match action {
            Action::Location(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn path_bar_crumbs_open_enclosing_folders_of_the_active_pane() {
    let mut harness = Harness::new(3);
    let documents = harness.workspace.pane().path.clone();
    let home = documents.parent().unwrap().to_owned();
    harness.state.home = home.clone();
    harness.ctx.enable_accesskit();
    harness.warm();
    let (_, output) = harness.frame(Vec::new());
    assert!(
        harness.state.path_bar,
        "toolbar left no room for its path bar"
    );
    assert!(
        widget_center(&output, "Go to Documents").is_none(),
        "the folder in view is not a navigation target"
    );
    let home_crumb = widget_center(&output, "Go to Home").expect("Home crumb");
    let mut navigated = Vec::new();
    for events in click_events(home_crumb) {
        navigated.extend(harness.frame(events).0);
    }
    let id = harness.workspace.pane().id;
    assert!(
        navigated
            .iter()
            .any(|action| matches!(action, Action::Navigate(target, path) if *target == id && *path == home)),
        "clicking Home did not open it in the active pane"
    );
    assert!(harness.state.location.is_none());
}

#[test]
fn path_bar_edits_a_location_that_enter_opens_and_escape_cancels() {
    for confirm in [true, false] {
        let mut harness = Harness::new(3);
        harness.workspace.pane_mut().select(0, false, false);
        harness.warm();
        let selected = harness.workspace.pane().selected.clone();
        harness.frame(vec![key(Key::L, true, false, Modifiers::COMMAND)]);
        harness.frame(vec![key(Key::L, false, false, Modifiers::COMMAND)]);
        assert!(
            harness.state.dialog.is_none(),
            "Go to folder opened a dialog"
        );
        assert!(
            harness.state.location.is_some(),
            "path bar did not start editing"
        );
        assert!(harness.ctx.text_edit_focused(), "location never took focus");
        // The current path starts selected, so typing replaces it.
        let (actions, _) = harness.frame(vec![egui::Event::Text("~/Downloads".into())]);
        assert!(!actions.iter().any(file_action));
        assert_eq!(harness.state.location.as_ref().unwrap().text, "~/Downloads");
        let finish = if confirm { Key::Enter } else { Key::Escape };
        let (actions, _) = harness.frame(vec![press(finish)]);
        harness.frame(vec![release(finish)]);
        assert_eq!(
            location_actions(&actions),
            if confirm {
                vec!["~/Downloads".to_owned()]
            } else {
                Vec::new()
            }
        );
        assert!(
            !actions.iter().any(file_action),
            "{finish:?} reached the files"
        );
        assert!(
            harness.state.location.is_none(),
            "{finish:?} left the editor open"
        );
        assert_eq!(harness.workspace.pane().selected, selected);
    }
}

#[test]
fn location_edit_survives_another_window_taking_focus() {
    let mut harness = Harness::new(3);
    harness.warm();
    harness.frame(vec![key(Key::L, true, false, Modifiers::COMMAND)]);
    harness.frame(vec![key(Key::L, false, false, Modifiers::COMMAND)]);
    assert!(harness.state.location.is_some());
    harness.window_focused = false;
    for _ in 0..3 {
        harness.frame(Vec::new());
    }
    assert!(
        harness.state.location.is_some(),
        "an inactive window ended the location edit"
    );
    harness.window_focused = true;
    harness.frame(vec![egui::Event::Text("~/Downloads".into())]);
    let (actions, _) = harness.frame(vec![press(Key::Enter)]);
    assert_eq!(location_actions(&actions), ["~/Downloads"]);
}

#[test]
fn location_edit_ends_when_its_pane_changes_folder() {
    let mut harness = Harness::new(3);
    harness.warm();
    harness.frame(vec![key(Key::L, true, false, Modifiers::COMMAND)]);
    assert!(harness.state.location.is_some());
    let elsewhere = harness.workspace.pane().path.join("elsewhere");
    harness.workspace.pane_mut().navigate(elsewhere);
    let (actions, _) = harness.frame(vec![press(Key::Enter)]);
    assert!(harness.state.location.is_none());
    assert!(location_actions(&actions).is_empty());
}

#[test]
fn breadcrumbs_start_at_home_or_the_file_system_root() {
    let (home, outside) = if cfg!(windows) {
        (
            PathBuf::from("C:\\Users\\Neptune"),
            PathBuf::from("D:\\srv\\Media"),
        )
    } else {
        (PathBuf::from("/home/neptune"), PathBuf::from("/srv/Media"))
    };
    let labels = |path: &std::path::Path| {
        breadcrumbs(path, &home)
            .into_iter()
            .map(|(label, _, _)| label)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        labels(&home.join("Documents").join("Projects")),
        ["Home", "Documents", "Projects"]
    );
    assert_eq!(labels(&home), ["Home"]);
    assert_eq!(labels(&outside), ["File system", "srv", "Media"]);
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
    // Files are picked up by their names; the rest of a row selects.
    let name = |row: Rect| egui::pos2(row.left() + 50.0, row.center().y);
    // A slow click turns into a drag without moving; releasing it in place
    // used to copy the file onto itself and report a collision.
    let file = name(rows[1]);
    assert!(!submitted(&drag(&mut harness, file, file, 1.0)));
    assert!(!submitted(&drag(&mut harness, file, rows[2].center(), 0.0)));
    // A folder is no target for itself.
    let own = name(rows[0]);
    let beside = own + vec2(40.0, 0.0);
    assert!(!submitted(&drag(&mut harness, own, beside, 0.0)));
    let source = harness.workspace.pane().entries[1].path.clone();
    let actions = drag(&mut harness, file, rows[0].center(), 0.0);
    assert!(
        actions.iter().any(|action| matches!(
            action,
            Action::Submit(Operation::Move { sources, destination })
                if sources == std::slice::from_ref(&source) && destination == &folder
        )),
        "a file dropped on a folder was not moved into it"
    );
    // The platform's copy modifier copies instead.
    harness.frame(vec![egui::Event::ModifiersChanged(copy_modifier())]);
    let actions = drag(&mut harness, file, rows[0].center(), 0.0);
    assert!(
        actions.iter().any(|action| matches!(
            action,
            Action::Submit(Operation::Copy { sources, destination })
                if sources == std::slice::from_ref(&source) && destination == &folder
        )),
        "a file dropped with the copy modifier was not copied"
    );
}

/// Ctrl, or Option on macOS, makes a drop copy.
fn copy_modifier() -> Modifiers {
    if cfg!(target_os = "macos") {
        Modifiers::ALT
    } else {
        Modifiers::CTRL
    }
}

#[test]
fn dragged_files_follow_the_pointer_with_their_count() {
    let mut harness = Harness::new(4);
    harness.workspace.pane_mut().select(1, false, false);
    harness.workspace.pane_mut().select(2, true, false);
    harness.warm();
    let rows = harness.item_rects();
    let from = egui::pos2(rows[1].left() + 50.0, rows[1].center().y);
    let to = egui::pos2(rows[3].left() + 200.0, rows[3].bottom() + 40.0);
    let card =
        |pointer: egui::Pos2| Rect::from_min_size(pointer + vec2(12.0, 14.0), Vec2::splat(56.0));
    let painted = |output: &egui::FullOutput, rect: Rect| {
        output
            .shapes
            .iter()
            .any(|clipped| matches!(&clipped.shape, egui::Shape::Rect(shape) if shape.rect == rect))
    };
    let count = |output: &egui::FullOutput| {
        output.shapes.iter().any(|clipped| {
            matches!(&clipped.shape, egui::Shape::Text(text) if text.galley.job.text == "2")
        })
    };
    harness.begin_drag(from);
    let (_, output) = harness.frame(vec![egui::Event::PointerMoved(to)]);
    assert!(painted(&output, card(to)), "no picture under the pointer");
    assert!(
        count(&output),
        "the picture does not say how many items move"
    );
    harness.frame(vec![button(to, false)]);
    let (_, output) = harness.frame(Vec::new());
    assert!(
        !painted(&output, card(to)) && !count(&output),
        "the picture stayed after the drop"
    );
}

#[test]
fn the_native_clipboard_chords_copy_cut_and_paste_files() {
    let mut harness = Harness::new(5);
    harness.workspace.pane_mut().select(1, false, false);
    harness.workspace.pane_mut().select(2, true, false);
    harness.warm();
    let selected = harness
        .workspace
        .pane()
        .selected
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    let folder = harness.workspace.pane().path.clone();
    let copied = |actions: &[Action], cut: bool| {
        actions.iter().any(|action| {
            matches!(action, Action::Copy(paths, moving) if paths == &selected && *moving == cut)
        })
    };
    let pastes = |actions: &[Action]| {
        actions
            .iter()
            .filter(|action| matches!(action, Action::Paste(to) if to == &folder))
            .count()
    };
    // The native window reports Command+X, C and V as these events, never as
    // pressed keys.
    let (actions, _) = harness.frame(vec![egui::Event::Cut]);
    assert!(
        copied(&actions, true),
        "Command+X did not cut the selection"
    );
    let (actions, _) = harness.frame(vec![egui::Event::Copy]);
    assert!(
        copied(&actions, false),
        "Command+C did not copy the selection"
    );
    // With text on the system clipboard the press pastes, and its release
    // does not paste again.
    let (actions, _) = harness.frame(vec![egui::Event::Paste("notes".into())]);
    assert_eq!(pastes(&actions), 1);
    let (actions, _) = harness.frame(vec![key(Key::V, false, false, Modifiers::COMMAND)]);
    assert_eq!(pastes(&actions), 0);
    // With no text there, the press reports nothing and the release pastes.
    let (actions, _) = harness.frame(vec![key(Key::V, false, false, Modifiers::COMMAND)]);
    assert_eq!(pastes(&actions), 1);
    let (actions, _) = harness.frame(vec![
        egui::Event::Paste("notes".into()),
        key(Key::V, false, false, Modifiers::COMMAND),
    ]);
    assert_eq!(pastes(&actions), 1, "one quick chord pasted twice");
    let (actions, _) = harness.frame(vec![press(Key::V), release(Key::V)]);
    assert_eq!(pastes(&actions), 0, "a plain V pasted");
}

#[test]
fn cut_items_fade_until_the_clipboard_changes() {
    for view in [ViewMode::List, ViewMode::Grid] {
        let mut harness = Harness::new(4);
        harness.config.view = view;
        let entries = harness.workspace.pane().entries.clone();
        harness.state.cut.insert(entries[1].path.clone());
        harness.warm();
        let alpha = |output: &egui::FullOutput, name: &str| {
            output
                .shapes
                .iter()
                .find_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) if text.galley.job.text == name => text
                        .galley
                        .rows
                        .iter()
                        .flat_map(|row| row.row.visuals.mesh.vertices.iter())
                        .map(|vertex| vertex.color.a())
                        .max(),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{name} was not painted in {view:?}"))
        };
        let (_, output) = harness.frame(Vec::new());
        assert!(
            alpha(&output, &entries[1].name) < alpha(&output, &entries[0].name),
            "a cut item looks like the others in {view:?}"
        );
        harness.state.cut.clear();
        let (_, output) = harness.frame(Vec::new());
        assert_eq!(
            alpha(&output, &entries[1].name),
            alpha(&output, &entries[0].name),
            "an item stayed faded after the clipboard changed in {view:?}"
        );
    }
}

#[test]
fn archive_dialogs_retain_their_original_targets_after_navigation() {
    let mut harness = Harness::new(3);
    let folder = harness.workspace.pane().path.clone();
    let sources = vec![
        harness.workspace.pane().entries[0].path.clone(),
        harness.workspace.pane().entries[1].path.clone(),
    ];
    harness.state.open_dialog(Dialog::Compress {
        sources: sources.clone(),
        destination: folder.clone(),
        name: "Archive.zip".into(),
    });
    harness
        .workspace
        .pane_mut()
        .navigate(folder.join("another-folder"));
    harness.warm();
    let (actions, _) = harness.frame(vec![press(Key::Enter)]);
    assert!(actions.iter().any(|action| matches!(action,
        Action::Submit(Operation::Compress { sources: chosen, destination, name })
            if chosen == &sources && destination == &folder && name == "Archive.zip")));
    assert!(harness.state.dialog.is_none());
    harness.frame(vec![release(Key::Enter)]);

    let archive = folder.join("Photos.zip");
    harness.state.open_dialog(Dialog::Extract {
        archive: archive.clone(),
        destination: "~/Pictures".into(),
    });
    harness
        .workspace
        .pane_mut()
        .navigate(folder.join("elsewhere"));
    harness.warm();
    let (actions, _) = harness.frame(vec![press(Key::Enter)]);
    assert!(actions.iter().any(|action| matches!(action,
        Action::ExtractTo(target, destination) if target == &archive && destination == "~/Pictures")));
    assert!(harness.state.dialog.is_none());
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

fn pointer_button(pos: egui::Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

fn travels(actions: &[Action]) -> Vec<(u64, bool)> {
    actions
        .iter()
        .filter_map(|action| match action {
            Action::Back(pane, forward) => Some((*pane, *forward)),
            _ => None,
        })
        .collect()
}

#[test]
fn mouse_back_and_forward_travel_the_pane_under_the_pointer() {
    let mut harness = Harness::new(5);
    let other = Pane::new(9, harness.workspace.pane().path.join("Other"));
    harness.workspace.tabs[0].panes.push(other);
    harness.warm();
    let active = harness.workspace.pane().id;
    assert_ne!(active, 9);
    for (point, target) in [
        (egui::pos2(768.0, 360.0), 9),
        (egui::pos2(256.0, 360.0), active),
        // The toolbar belongs to no pane, so the active pane travels.
        (egui::pos2(300.0, 22.0), active),
    ] {
        for (button, forward) in [
            (egui::PointerButton::Extra1, false),
            (egui::PointerButton::Extra2, true),
        ] {
            let (actions, _) = harness.frame(vec![
                egui::Event::PointerMoved(point),
                pointer_button(point, button, true),
            ]);
            assert_eq!(
                travels(&actions),
                vec![(target, forward)],
                "{button:?} at {point:?}"
            );
            let (actions, _) = harness.frame(vec![pointer_button(point, button, false)]);
            assert!(travels(&actions).is_empty(), "release travelled again");
        }
    }
}

#[test]
fn mouse_history_buttons_leave_an_open_dialog_alone() {
    let mut harness = Harness::new(5);
    harness.state.open_dialog(Dialog::Preferences);
    harness.warm();
    let point = egui::pos2(512.0, 360.0);
    for button in [egui::PointerButton::Extra1, egui::PointerButton::Extra2] {
        let (actions, _) = harness.frame(vec![
            egui::Event::PointerMoved(point),
            pointer_button(point, button, true),
        ]);
        assert!(travels(&actions).is_empty());
        harness.frame(vec![pointer_button(point, button, false)]);
    }
}

#[test]
fn middle_click_opens_a_folder_in_a_new_tab_and_a_file_directly() {
    for view in [ViewMode::List, ViewMode::Grid] {
        let mut harness = Harness::new(3);
        harness.config.view = view;
        let folder = harness.workspace.pane().path.join("Projects");
        harness.workspace.pane_mut().entries[0] = Entry {
            path: folder,
            name: "Projects".into(),
            kind: EntryKind::Directory,
            size: 0,
            modified: None,
            hidden: false,
        };
        harness.warm();
        let pane = harness.workspace.pane().id;
        // Visible rows and tiles in entry order: top to bottom, then across.
        let mut cells = harness
            .state
            .file_widgets
            .iter()
            .filter_map(|id| harness.ctx.read_response(*id))
            .map(|response| response.rect)
            .collect::<Vec<_>>();
        cells.sort_by(|a, b| {
            (a.top(), a.left())
                .partial_cmp(&(b.top(), b.left()))
                .unwrap()
        });
        let entries = harness.workspace.pane().entries.clone();
        assert_eq!(cells.len(), entries.len());
        for (cell, entry) in cells.iter().zip(&entries).take(2) {
            let point = cell.center();
            harness.frame(vec![
                egui::Event::PointerMoved(point),
                pointer_button(point, egui::PointerButton::Middle, true),
            ]);
            let (actions, _) = harness.frame(vec![pointer_button(
                point,
                egui::PointerButton::Middle,
                false,
            )]);
            assert!(
                actions.iter().any(|action| match action {
                    Action::NewTab(path) => entry.is_dir() && path == &entry.path,
                    Action::Open(target, path, false) =>
                        !entry.is_dir() && *target == pane && path == &entry.path,
                    _ => false,
                }),
                "{view:?}: middle click on {} did not open it",
                entry.name
            );
            assert!(
                !actions.iter().any(|action| matches!(
                    action,
                    Action::Navigate(..) | Action::Open(_, _, true)
                )),
                "{view:?}: a middle click navigated the pane"
            );
            assert!(
                harness.workspace.pane().selected.is_empty(),
                "{view:?}: a middle click changed the selection"
            );
        }
    }
}

#[test]
fn alt_enter_and_command_i_show_properties_of_one_item_or_the_folder() {
    let mut harness = Harness::new(5);
    harness.workspace.pane_mut().select(0, false, false);
    harness.warm();
    let pane = harness.workspace.pane().id;
    let selected = harness.workspace.pane().entries[0].path.clone();
    let row = harness.focused_file();
    let alt_enter = |pressed| key(Key::Enter, pressed, false, Modifiers::ALT);
    let (actions, _) = harness.frame_at(vec![alt_enter(true)], vec2(1_024.0, 720.0), Some(row));
    assert!(actions.iter().any(
        |action| matches!(action, Action::Properties(id, entry) if *id == pane && entry.path == selected)
    ));
    assert!(
        !actions.iter().any(file_action),
        "Alt+Enter also opened the item"
    );
    harness.frame(vec![alt_enter(false)]);

    // With nothing selected, the chord describes the folder in view.
    harness.workspace.pane_mut().selected.clear();
    let folder = harness.workspace.pane().path.clone();
    let get_info = |pressed| key(Key::I, pressed, false, Modifiers::COMMAND);
    let (actions, _) = harness.frame(vec![get_info(true)]);
    assert!(actions.iter().any(|action| matches!(
        action,
        Action::Properties(id, entry) if *id == pane && entry.path == folder && entry.is_dir()
    )));
    harness.frame(vec![get_info(false)]);

    harness.workspace.pane_mut().select(0, false, false);
    harness.workspace.pane_mut().select(1, true, false);
    let (actions, _) = harness.frame(vec![get_info(true)]);
    assert!(
        !actions
            .iter()
            .any(|action| matches!(action, Action::Properties(..))),
        "a multiple selection chose one item's properties"
    );
    assert!(harness.state.notice.is_some());
}

#[test]
fn properties_stay_open_through_the_opening_chord_and_fit_a_narrow_window() {
    let mut harness = Harness::new(3);
    let entry = harness.workspace.pane().entries[0].clone();
    let pane = harness.workspace.pane().id;
    harness.state.open_dialog(Dialog::Properties {
        pane,
        entry: entry.clone(),
    });
    let (_, _) = harness.frame(vec![key(Key::Enter, true, false, Modifiers::ALT)]);
    assert!(
        harness.state.dialog.is_some(),
        "the opening chord closed Properties"
    );
    harness.frame(vec![key(Key::Enter, false, false, Modifiers::ALT)]);
    harness.state.properties = Some(Ok(finder_core::Properties {
        entry,
        created: Some(SystemTime::UNIX_EPOCH),
        link_target: None,
        permissions: Some(finder_core::Permissions::Unix {
            mode: 0o644,
            owner: "neptune".into(),
            group: "neptune".into(),
            access: finder_core::Access {
                read: true,
                write: true,
                execute: false,
            },
        }),
        available_space: Some(227_000_000_000),
        contents: None,
    }));
    for size in [vec2(1_024.0, 720.0), vec2(360.0, 420.0)] {
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
    }
    harness.frame(vec![press(Key::Enter)]);
    assert!(
        harness.state.dialog.is_none(),
        "a fresh Enter left Properties open"
    );
    assert!(
        harness.state.properties.is_none(),
        "closing kept the item's properties"
    );
}

#[test]
fn a_narrow_window_opens_the_sidebar_over_files_and_closes_it_after_a_choice() {
    let mut harness = Harness::new(20);
    harness.workspace.pane_mut().select(0, false, false);
    let selected = harness.workspace.pane().selected.clone();
    let chord = if cfg!(target_os = "macos") {
        Modifiers::COMMAND
    } else {
        Modifiers::COMMAND | Modifiers::SHIFT
    };
    let narrow = vec2(600.0, 480.0);
    let settle = |harness: &mut Harness, size: Vec2| {
        for _ in 0..20 {
            harness.frame_at(Vec::new(), size, None);
        }
    };
    settle(&mut harness, narrow);

    // The chord opens the overlay without changing the docked preference.
    let (actions, _) = harness.frame_at(vec![key(Key::B, true, false, chord)], narrow, None);
    assert!(harness.state.sidebar_overlay);
    assert!(!harness.config.sidebar);
    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, Action::PreferencesChanged))
    );
    settle(&mut harness, narrow);

    // The mouse's history buttons leave the files under it alone.
    let point = egui::pos2(420.0, 300.0);
    let (actions, _) = harness.frame_at(
        vec![
            egui::Event::PointerMoved(point),
            pointer_button(point, egui::PointerButton::Extra1, true),
        ],
        narrow,
        None,
    );
    assert!(travels(&actions).is_empty());
    harness.frame_at(
        vec![pointer_button(point, egui::PointerButton::Extra1, false)],
        narrow,
        None,
    );
    assert!(harness.state.sidebar_overlay);

    // Escape closes it and leaves the file selection alone.
    harness.frame_at(vec![press(Key::Escape)], narrow, None);
    assert!(!harness.state.sidebar_overlay);
    assert_eq!(harness.workspace.pane().selected, selected);
    harness.frame_at(vec![release(Key::Escape)], narrow, None);

    // Choosing a place navigates there and closes it.
    harness.state.sidebar_overlay = true;
    settle(&mut harness, narrow);
    let home = egui::pos2(60.0, 87.0);
    let button = |pressed| egui::Event::PointerButton {
        pos: home,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    harness.frame_at(vec![egui::Event::PointerMoved(home)], narrow, None);
    harness.frame_at(vec![button(true)], narrow, None);
    let (actions, _) = harness.frame_at(vec![button(false)], narrow, None);
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, Action::Navigate(_, path) if path == &harness.state.home))
    );
    assert!(!harness.state.sidebar_overlay);

    // A window wide enough to dock the sidebar closes the overlay.
    harness.state.sidebar_overlay = true;
    settle(&mut harness, narrow);
    harness.frame_at(Vec::new(), vec2(1_024.0, 720.0), None);
    assert!(!harness.state.sidebar_overlay);

    // In a wide window the chord still docks or hides the sidebar.
    let (actions, _) = harness.frame_at(
        vec![key(Key::B, true, false, chord)],
        vec2(1_024.0, 720.0),
        None,
    );
    assert!(harness.config.sidebar);
    assert!(!harness.state.sidebar_overlay);
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, Action::PreferencesChanged))
    );
}

/// Whether the frame's output says `text`: a control's name, or the text
/// of a label, which AccessKit carries as its value.
fn says(output: &egui::FullOutput, text: &str) -> bool {
    output
        .platform_output
        .accesskit_update
        .as_ref()
        .is_some_and(|update| {
            update.nodes.iter().any(|(_, node)| {
                node.bounds().is_some()
                    && (node.label() == Some(text) || node.value() == Some(text))
            })
        })
}

/// Terminal's right panel chord.
fn info_chord(pressed: bool) -> egui::Event {
    let chord = if cfg!(target_os = "macos") {
        Modifiers::COMMAND
    } else {
        Modifiers::COMMAND | Modifiers::SHIFT
    };
    key(Key::O, pressed, false, chord)
}

/// The rightmost edge of the visible file rows.
fn files_right(harness: &Harness) -> f32 {
    harness
        .item_rects()
        .iter()
        .map(|rect| rect.right())
        .fold(f32::MIN, f32::max)
}

fn settle(harness: &mut Harness, size: Vec2) -> egui::FullOutput {
    let mut last = None;
    for _ in 0..20 {
        last = Some(harness.frame_at(Vec::new(), size, None).1);
    }
    last.unwrap()
}

#[test]
fn the_info_panel_toggles_beside_the_files_and_follows_the_selection() {
    let wide = vec2(1_024.0, 720.0);
    let mut harness = Harness::new(20);
    harness.ctx.enable_accesskit();
    let output = settle(&mut harness, wide);
    assert!(widget_center(&output, "Info panel").is_none());
    assert!(widget_center(&output, "Toggle info panel").is_some());
    let full = files_right(&harness);

    // The chord opens it and saves the choice; the files give it room.
    let (actions, _) = harness.frame_at(vec![info_chord(true)], wide, None);
    assert!(harness.config.info_panel);
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, Action::PreferencesChanged))
    );
    harness.frame_at(vec![info_chord(false)], wide, None);
    let output = settle(&mut harness, wide);
    assert!(widget_center(&output, "Info panel").is_some());
    let beside = files_right(&harness);
    assert!(
        (full - beside - Config::INFO_PANEL_WIDTH).abs() <= theme::metrics::GUTTER + 1.0,
        "files ended at {beside}, from {full}"
    );

    // With nothing selected it describes the folder and reads nothing.
    assert!(says(&output, "Select an item to see its details."));
    assert!(widget_center(&output, "New folder").is_some());
    assert!(harness.state.info_subject.is_none());

    // One item: its name, details and quick actions, read by the app.
    harness.workspace.pane_mut().select(3, false, false);
    let entry = harness.workspace.pane().entries[3].clone();
    let output = settle(&mut harness, wide);
    assert_eq!(
        harness.state.info_subject,
        Some((entry.path.clone(), entry.modified))
    );
    assert!(says(&output, &entry.name));
    assert!(says(&output, "Size: 3 B"));
    assert!(says(&output, "Created: —"));

    // Its properties fill in once read; another item's never do.
    let read = || {
        Ok(finder_core::Properties {
            entry: entry.clone(),
            created: None,
            link_target: None,
            permissions: Some(finder_core::Permissions::ReadOnly(false)),
            available_space: None,
            contents: None,
        })
    };
    let other = harness.workspace.pane().entries[4].path.clone();
    harness.state.info_read = Some((other, read()));
    let output = settle(&mut harness, wide);
    assert!(says(&output, "Your access: —"));
    harness.state.info_read = Some((entry.path.clone(), read()));
    let output = settle(&mut harness, wide);
    assert!(says(&output, "Your access: Read and write"));
    assert!(says(&output, "Created: Unavailable"));

    // A quick action keeps the item it was pressed for.
    let rename = widget_center(&output, "Rename").expect("Rename quick action");
    for events in click_events(rename) {
        harness.frame_at(events, wide, None);
    }
    assert!(matches!(
        &harness.state.dialog,
        Some(Dialog::Rename { source, .. }) if *source == entry.path
    ));
    harness.state.dialog = None;

    // Several items are counted, not read.
    harness.workspace.pane_mut().select(5, true, false);
    let output = settle(&mut harness, wide);
    assert!(harness.state.info_subject.is_none());
    assert!(says(&output, "2 items"));
    let trash = widget_center(&output, "Move to Trash").expect("Trash quick action");
    for events in click_events(trash) {
        harness.frame_at(events, wide, None);
    }
    assert!(matches!(
        &harness.state.dialog,
        Some(Dialog::Trash { paths }) if paths.len() == 2 && paths.contains(&entry.path)
    ));
    harness.state.dialog = None;

    // The toggle hides it again, and the panel stops asking.
    let toggle = widget_center(&output, "Toggle info panel").unwrap();
    harness.workspace.pane_mut().select(3, false, false);
    for events in click_events(toggle) {
        harness.frame_at(events, wide, None);
    }
    assert!(!harness.config.info_panel);
    let output = settle(&mut harness, wide);
    assert!(widget_center(&output, "Info panel").is_none());
    assert!(harness.state.info_subject.is_none());
    assert_eq!(files_right(&harness), full);
}

#[test]
fn a_window_too_narrow_for_the_info_panel_keeps_its_files_and_no_toggle() {
    let mut harness = Harness::new(20);
    harness.ctx.enable_accesskit();
    harness.config.info_panel = true;
    harness.workspace.pane_mut().select(0, false, false);
    let narrow = vec2(560.0, 480.0);
    let output = settle(&mut harness, narrow);
    assert!(!harness.state.info_available);
    assert!(widget_center(&output, "Info panel").is_none());
    assert!(widget_center(&output, "Toggle info panel").is_none());
    assert!(harness.state.info_subject.is_none());
    assert!(files_right(&harness) > 540.0);

    // A wider window brings it back as it was left.
    let output = settle(&mut harness, vec2(1_024.0, 720.0));
    assert!(widget_center(&output, "Info panel").is_some());
    assert!(harness.state.info_subject.is_some());
    assert!(harness.config.info_panel);
}

#[test]
fn the_info_panel_does_not_call_an_unreadable_folder_empty() {
    let wide = vec2(1_024.0, 720.0);
    let mut harness = Harness::new(4);
    harness.ctx.enable_accesskit();
    harness.config.info_panel = true;
    harness.workspace.pane_mut().entries[1].kind = EntryKind::Directory;
    harness.workspace.pane_mut().select(1, false, false);
    let entry = harness.workspace.pane().entries[1].clone();
    let read = |contents| {
        let properties = finder_core::Properties {
            entry: entry.clone(),
            created: None,
            link_target: None,
            permissions: None,
            available_space: None,
            contents: Some(contents),
        };
        Some((entry.path.clone(), Ok(properties)))
    };
    harness.state.info_read = read(finder_core::Contents {
        complete: true,
        partial: true,
        ..Default::default()
    });
    let output = settle(&mut harness, wide);
    assert!(says(&output, "Items: None readable"));
    assert!(says(&output, "Size: Unavailable"));

    // A folder that was read and holds nothing is empty.
    harness.state.info_read = read(finder_core::Contents {
        complete: true,
        ..Default::default()
    });
    let output = settle(&mut harness, wide);
    assert!(says(&output, "Items: 0"));
    assert!(says(&output, "Size: 0 B"));
}

#[test]
fn the_info_panel_asks_for_a_larger_measured_picture_of_its_file_only() {
    let mut harness = Harness::new(4);
    harness.config.info_panel = true;
    let image = harness.workspace.pane().path.join("photo.png");
    harness.workspace.pane_mut().entries[2].path = image.clone();
    harness.workspace.pane_mut().entries[1].kind = EntryKind::Directory;
    harness.workspace.pane_mut().select(2, false, false);
    settle(&mut harness, vec2(1_024.0, 720.0));
    harness.frame(Vec::new());
    let wanted = harness.state.info_picture.end_frame().expect("a picture");
    assert_eq!(wanted.path, image);
    assert!(wanted.measure);
    assert!(wanted.pixels >= 256, "{} pixels", wanted.pixels);
    // List view draws no thumbnails, so only the panel asks.
    assert!(harness.state.thumbnails.end_frame().is_empty());

    // A folder has no picture.
    harness.workspace.pane_mut().select(1, false, false);
    harness.frame(Vec::new());
    assert!(harness.state.info_picture.end_frame().is_none());
}
