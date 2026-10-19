use crate::{
    Launch,
    config::Config,
    model::{Pane, Tab, Workspace},
    persistence::Storage,
    preview::{PreviewContent, Previews},
    ui::{self, Action, Dialog, UiState},
    watch::Watches,
};
use eframe::egui;
use finder_core::{Entry, ScanOptions, Scanner};
use finder_operations::{Operation, Operations};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub struct FinderApp {
    pub workspace: Workspace,
    pub config: Config,
    pub ui: UiState,
    scanners: HashMap<u64, Scanner>,
    disposals: HashMap<u64, Vec<Entry>>,
    operations: Operations,
    previews: Previews,
    watches: Watches,
    storage: Storage,
    launch: Launch,
    home: PathBuf,
    initialized: bool,
    writable: bool,
    applied_hidden: bool,
    clipboard: Option<(Vec<PathBuf>, bool)>,
    busy: HashSet<u64>,
    mutations: HashSet<u64>,
    pending_cut: HashMap<u64, Vec<PathBuf>>,
    search_deadline: Option<(Instant, u64)>,
    watch_deadline: Option<Instant>,
    save_deadline: Option<Instant>,
    preview_generation: u64,
    started: Instant,
    capture_sent: bool,
    ephemeral: bool,
}
impl FinderApp {
    pub fn new(cc: &eframe::CreationContext<'_>, launch: Launch) -> Self {
        crate::fonts::install(&cc.egui_ctx);
        // Finder steps the zoom itself, so it is saved and named as it changes.
        cc.egui_ctx
            .options_mut(|options| options.zoom_with_keyboard = false);
        let home = directories::BaseDirs::new()
            .map(|d| d.home_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from(if cfg!(windows) { "C:\\" } else { "/" }));
        let path = launch
            .path
            .as_deref()
            .map(absolute)
            .unwrap_or_else(|| home.clone());
        let root = launch.data_root.clone().unwrap_or_else(|| {
            directories::ProjectDirs::from("rs", "Neptune", "NeptuneFinder")
                .map(|d| d.config_dir().to_path_buf())
                .unwrap_or_else(|| home.join(".neptune-finder"))
        });
        let config_path = launch
            .config
            .clone()
            .unwrap_or_else(|| root.join("config.toml"));
        let wake = cc.egui_ctx.clone();
        let storage = Storage::new(root, config_path, move || wake.request_repaint());
        let wake = cc.egui_ctx.clone();
        let operations = Operations::new(move || wake.request_repaint());
        let wake = cc.egui_ctx.clone();
        let previews = Previews::new(move || wake.request_repaint());
        let wake = cc.egui_ctx.clone();
        let watches = Watches::new(move || wake.request_repaint());
        let config = Config::default();
        crate::theme::apply(&cc.egui_ctx, &config);
        let ephemeral = launch.screenshot.is_some();
        let mut ui = UiState::new(home.clone());
        if let Ok(icon) =
            image::load_from_memory(include_bytes!("../assets/branding/neptune-finder-icon.png"))
        {
            let rgba = icon.to_rgba8();
            ui.brand_icon = Some(cc.egui_ctx.load_texture(
                "neptune-finder-brand",
                egui::ColorImage::from_rgba_unmultiplied(
                    [rgba.width() as usize, rgba.height() as usize],
                    rgba.as_raw(),
                ),
                egui::TextureOptions::LINEAR,
            ));
        }
        Self {
            workspace: Workspace::new(path),
            config,
            ui,
            scanners: HashMap::new(),
            disposals: HashMap::new(),
            operations,
            previews,
            watches,
            storage,
            launch,
            home,
            initialized: false,
            writable: true,
            applied_hidden: false,
            clipboard: None,
            busy: HashSet::new(),
            mutations: HashSet::new(),
            pending_cut: HashMap::new(),
            search_deadline: None,
            watch_deadline: None,
            save_deadline: None,
            preview_generation: 0,
            started: Instant::now(),
            capture_sent: false,
            ephemeral,
        }
    }
    fn reload(&mut self, id: u64, ctx: &egui::Context) {
        let Some(pane) = self.workspace.find_mut(id) else {
            return;
        };
        let scanner = self.scanners.entry(id).or_insert_with(|| {
            let ctx = ctx.clone();
            Scanner::new(move || ctx.request_repaint())
        });
        pane.generation = scanner.request(
            pane.path.clone(),
            ScanOptions {
                show_hidden: self.config.show_hidden,
                query: pane.query.clone(),
                recursive: pane.recursive && !pane.query.is_empty(),
                sort: pane.sort,
            },
        );
        pane.loading = true;
        pane.error = None;
    }
    fn retire_entries(&mut self, id: u64, entries: Vec<Entry>, ctx: &egui::Context) {
        if entries.is_empty() {
            return;
        }
        let scanner = self.scanners.entry(id).or_insert_with(|| {
            let ctx = ctx.clone();
            Scanner::new(move || ctx.request_repaint())
        });
        if let Err(entries) = scanner.retire(entries) {
            debug_assert!(!self.disposals.contains_key(&id));
            self.disposals.insert(id, entries);
        }
    }
    fn drain_pane(&mut self, id: u64, ctx: &egui::Context) {
        if let Some(pane) = self.workspace.find_mut(id) {
            let entries = std::mem::take(&mut pane.entries);
            self.retire_entries(id, entries, ctx);
        }
    }
    fn suspend_current(&mut self, ctx: &egui::Context) {
        let ids = self.workspace.tabs[self.workspace.active]
            .panes
            .iter()
            .map(|pane| pane.id)
            .collect::<Vec<_>>();
        for id in ids {
            self.drain_pane(id, ctx);
            if let Some(pane) = self.workspace.find_mut(id) {
                pane.generation = 0;
                pane.loading = true;
            }
            if !self.disposals.contains_key(&id) {
                self.scanners.remove(&id);
            }
        }
    }
    fn reload_visible(&mut self, ctx: &egui::Context) {
        let ids = self.workspace.tabs[self.workspace.active]
            .panes
            .iter()
            .map(|p| p.id)
            .collect::<Vec<_>>();
        for id in ids {
            self.reload(id, ctx);
        }
    }
    fn watch_visible(&self) {
        self.watches.set_paths(
            self.workspace.tabs[self.workspace.active]
                .panes
                .iter()
                .map(|p| p.path.clone())
                .collect(),
        );
    }
    fn changed(&mut self, ctx: &egui::Context) {
        self.save_deadline = Some(Instant::now() + Duration::from_millis(350));
        ctx.request_repaint_after(Duration::from_millis(350));
    }
    fn poll(&mut self, ctx: &egui::Context) {
        for (id, entries) in std::mem::take(&mut self.disposals) {
            self.retire_entries(id, entries, ctx);
        }
        if !self.initialized
            && let Some(boot) = self.storage.poll_bootstrap()
        {
            self.config = boot.config;
            self.writable = boot.writable;
            if self.launch.path.is_none()
                && !self.launch.no_restore
                && self.config.restore_session
                && let Some(session) = boot.session
            {
                self.workspace = Workspace::restore(&session, self.home.clone());
            }
            if let Some(theme) = self.launch.theme {
                self.config.theme = theme;
            }
            self.applied_hidden = self.config.show_hidden;
            crate::theme::apply(ctx, &self.config);
            ctx.set_zoom_factor(self.config.window_zoom);
            self.ui.notice = boot.warning.map(ui::Notice::problem);
            self.initialized = true;
            self.reload_visible(ctx);
            self.watch_visible();
        }
        let visible = self.workspace.tabs[self.workspace.active]
            .panes
            .iter()
            .map(|pane| pane.id)
            .collect::<HashSet<_>>();
        self.scanners
            .retain(|id, _| visible.contains(id) || self.disposals.contains_key(id));
        for (&id, scanner) in &self.scanners {
            if !visible.contains(&id) || self.disposals.contains_key(&id) {
                continue;
            }
            if let Some(result) = scanner.poll()
                && let Some(pane) = self.workspace.find_mut(id)
                && pane.generation == result.generation
            {
                pane.loading = false;
                match result.result {
                    Ok(listing) => {
                        let anchor_path = pane
                            .anchor
                            .and_then(|index| pane.entries.get(index))
                            .map(|entry| entry.path.clone());
                        let focus_path = pane
                            .focus_index
                            .and_then(|index| pane.entries.get(index))
                            .map(|entry| entry.path.clone());
                        pane.elapsed = listing.elapsed;
                        pane.truncated = listing.truncated;
                        // No display-name lookup: retain native path identities.
                        if !pane.selected.is_empty() {
                            pane.selected = listing
                                .entries
                                .iter()
                                .filter(|e| pane.selected.contains(&e.path))
                                .map(|e| e.path.clone())
                                .collect();
                        }
                        pane.path = listing.path;
                        let obsolete = std::mem::replace(&mut pane.entries, listing.entries);
                        if let Err(entries) = scanner.retire(obsolete) {
                            self.disposals.insert(id, entries);
                        }
                        pane.error = None;
                        pane.anchor = anchor_path.and_then(|path| {
                            pane.entries.iter().position(|entry| entry.path == path)
                        });
                        pane.focus_index = focus_path.and_then(|path| {
                            pane.entries.iter().position(|entry| entry.path == path)
                        });
                    }
                    Err(error) => {
                        let obsolete = std::mem::take(&mut pane.entries);
                        if let Err(entries) = scanner.retire(obsolete) {
                            self.disposals.insert(id, entries);
                        }
                        pane.selected.clear();
                        pane.error = Some(error);
                    }
                }
            }
        }
        while let Some(result) = self.operations.poll() {
            self.busy.remove(&result.id);
            if let Some(paths) = self.pending_cut.remove(&result.id)
                && result.result.is_ok()
                && self
                    .clipboard
                    .as_ref()
                    .is_some_and(|(current, cut)| *cut && current == &paths)
            {
                self.clipboard = None;
            }
            // Finished work shows in the files themselves; only a failure
            // needs a message.
            if let Err(error) = result.result {
                self.ui.notice = Some(ui::Notice::problem(format!(
                    "Operation could not be completed: {error}"
                )));
            }
            if self.mutations.remove(&result.id) {
                self.reload_visible(ctx);
            }
        }
        if let Some(result) = self.previews.poll()
            && result.generation == self.preview_generation
        {
            self.ui.preview = Some(match result.result {
                Ok(PreviewContent::Text(text)) => ui::Preview::Text(text),
                Ok(PreviewContent::Image { size, rgba }) => ui::Preview::Image(ctx.load_texture(
                    "quick-look",
                    egui::ColorImage::from_rgba_unmultiplied(size, &rgba),
                    egui::TextureOptions::LINEAR,
                )),
                Ok(PreviewContent::Unsupported(message)) => ui::Preview::Message(message),
                Err(error) => ui::Preview::Message(error),
            });
        }
        if let Some(error) = self.storage.poll_error() {
            self.ui.notice = Some(ui::Notice::problem(error));
        }
        if self.watches.take_changed() && self.watch_deadline.is_none() {
            self.watch_deadline = Some(Instant::now() + Duration::from_millis(160));
        }
        if let Some(deadline) = self.watch_deadline {
            if Instant::now() >= deadline {
                self.watch_deadline = None;
                self.reload_visible(ctx);
            } else {
                ctx.request_repaint_after(deadline.saturating_duration_since(Instant::now()));
            }
        }
        if let Some((deadline, id)) = self.search_deadline {
            if Instant::now() >= deadline {
                self.search_deadline = None;
                self.reload(id, ctx);
            } else {
                ctx.request_repaint_after(deadline.saturating_duration_since(Instant::now()));
            }
        }
        if let Some(deadline) = self.save_deadline {
            if Instant::now() >= deadline {
                self.save_deadline = None;
                if self.writable && !self.ephemeral {
                    self.storage
                        .save(self.workspace.snapshot(self.config.clone()));
                }
            } else {
                ctx.request_repaint_after(deadline.saturating_duration_since(Instant::now()));
            }
        }
        self.ui.busy = self.busy.len();
        self.ui.can_paste = self.clipboard.is_some();
    }
    fn navigate(&mut self, id: u64, path: PathBuf, ctx: &egui::Context) {
        self.drain_pane(id, ctx);
        if let Some(pane) = self.workspace.find_mut(id) {
            pane.navigate(path);
        }
        self.search_deadline = None;
        self.reload(id, ctx);
        self.watch_visible();
        self.changed(ctx);
    }
    fn submit(&mut self, operation: Operation) -> Option<u64> {
        let mutating = !matches!(
            &operation,
            Operation::Open { .. } | Operation::Reveal { .. }
        );
        match self.operations.submit(operation) {
            Ok(id) => {
                self.busy.insert(id);
                if mutating {
                    self.mutations.insert(id);
                }
                Some(id)
            }
            Err(error) => {
                self.ui.notice = Some(ui::Notice::problem(error));
                None
            }
        }
    }
    fn quick_look(&mut self, path: PathBuf) {
        self.preview_generation = self.previews.request(path.clone());
        self.ui.preview = None;
        self.ui.open_dialog(Dialog::QuickLook { path });
    }
    fn dispatch(&mut self, action: Action, ctx: &egui::Context) {
        let id = self.workspace.pane().id;
        match action {
            Action::Navigate(target, path) => self.navigate(target, path, ctx),
            Action::Open(target, path, directory) => {
                if directory {
                    self.navigate(target, path, ctx);
                } else {
                    self.submit(Operation::Open { path });
                }
            }
            Action::Back(target, forward) => {
                let Some(pane) = self.workspace.find_mut(target) else {
                    return;
                };
                let possible = if forward {
                    pane.history_index + 1 < pane.history.len()
                } else {
                    pane.history_index > 0
                };
                if possible {
                    self.drain_pane(target, ctx);
                }
                if self
                    .workspace
                    .find_mut(target)
                    .is_some_and(|pane| pane.travel(forward))
                {
                    self.reload(target, ctx);
                    self.watch_visible();
                    self.changed(ctx);
                }
            }
            Action::Parent => {
                if let Some(parent) = self.workspace.pane().path.parent() {
                    self.navigate(id, parent.to_owned(), ctx);
                }
            }
            Action::Reload => self.reload(id, ctx),
            Action::ReloadPane(target) => self.reload(target, ctx),
            Action::Search(target) => {
                self.search_deadline = Some((Instant::now() + Duration::from_millis(160), target));
                ctx.request_repaint_after(Duration::from_millis(160));
            }
            Action::Sort(target, field) => {
                if let Some(pane) = self.workspace.find_mut(target) {
                    if pane.sort.field == field {
                        pane.sort.descending = !pane.sort.descending;
                    } else {
                        pane.sort.field = field;
                        pane.sort.descending = false;
                    }
                }
                self.reload(target, ctx);
            }
            Action::ToggleHidden => {
                self.config.show_hidden = !self.config.show_hidden;
                self.applied_hidden = self.config.show_hidden;
                self.reload_visible(ctx);
                self.changed(ctx);
            }
            Action::ToggleRecursive => {
                self.workspace.pane_mut().recursive = !self.workspace.pane().recursive;
                self.reload(id, ctx);
            }
            Action::NewTab(path) => {
                if self.workspace.tabs.len() >= 24 {
                    self.ui.notice = Some(ui::Notice::info("You can open up to 24 tabs."));
                    return;
                }
                self.suspend_current(ctx);
                let tid = self.workspace.allocate();
                let pid = self.workspace.allocate();
                self.workspace.tabs.push(Tab {
                    id: tid,
                    panes: vec![Pane::new(pid, path)],
                    active: 0,
                });
                self.workspace.active = self.workspace.tabs.len() - 1;
                self.reload(pid, ctx);
                self.watch_visible();
                self.changed(ctx);
            }
            Action::SelectTab(index) => {
                if index < self.workspace.tabs.len() {
                    if index != self.workspace.active {
                        self.suspend_current(ctx);
                    }
                    self.workspace.active = index;
                    self.reload_visible(ctx);
                    self.watch_visible();
                    self.changed(ctx);
                }
            }
            Action::CloseTab(index) => {
                if self.workspace.tabs.len() == 1 {
                    self.close(ctx);
                    return;
                }
                if index < self.workspace.tabs.len() {
                    let ids = self.workspace.tabs[index]
                        .panes
                        .iter()
                        .map(|pane| pane.id)
                        .collect::<Vec<_>>();
                    for id in ids {
                        self.drain_pane(id, ctx);
                    }
                    for pane in self.workspace.tabs.remove(index).panes {
                        if !self.disposals.contains_key(&pane.id) {
                            self.scanners.remove(&pane.id);
                        }
                    }
                    if index < self.workspace.active {
                        self.workspace.active -= 1;
                    }
                    self.workspace.active =
                        self.workspace.active.min(self.workspace.tabs.len() - 1);
                    self.reload_visible(ctx);
                    self.watch_visible();
                    self.changed(ctx);
                }
            }
            Action::WindowClose => self.close(ctx),
            Action::ToggleSplit => {
                if self.workspace.tabs[self.workspace.active].panes.len() == 2 {
                    let id = self.workspace.tabs[self.workspace.active].panes[1].id;
                    self.drain_pane(id, ctx);
                    let pane = self.workspace.tabs[self.workspace.active]
                        .panes
                        .pop()
                        .unwrap();
                    if !self.disposals.contains_key(&pane.id) {
                        self.scanners.remove(&pane.id);
                    }
                    self.workspace.tabs[self.workspace.active].active = 0;
                } else {
                    let pid = self.workspace.allocate();
                    let path = self.workspace.pane().path.clone();
                    self.workspace.tabs[self.workspace.active]
                        .panes
                        .push(Pane::new(pid, path));
                    self.reload(pid, ctx);
                }
                self.watch_visible();
                self.changed(ctx);
            }
            Action::FocusPane(index) => {
                if index < self.workspace.tabs[self.workspace.active].panes.len() {
                    self.workspace.tabs[self.workspace.active].active = index;
                }
            }
            Action::Copy(selected, cut) => {
                if !selected.is_empty() {
                    self.clipboard = Some((selected, cut));
                }
            }
            Action::Paste(destination) => {
                let Some((paths, cut)) = self.clipboard.clone() else {
                    return;
                };
                if cut && self.pending_cut.values().any(|pending| pending == &paths) {
                    self.ui.notice = Some(ui::Notice::info("That move is already queued."));
                    return;
                }
                let operation = if cut {
                    // Cut items pasted into their own folder are already in place.
                    let sources: Vec<_> = paths
                        .iter()
                        .filter(|source| source.parent() != Some(destination.as_path()))
                        .cloned()
                        .collect();
                    if sources.is_empty() {
                        return;
                    }
                    Operation::Move {
                        sources,
                        destination,
                    }
                } else {
                    Operation::Copy {
                        sources: paths.clone(),
                        destination,
                    }
                };
                if let Some(id) = self.submit(operation)
                    && cut
                {
                    self.pending_cut.insert(id, paths);
                }
            }
            Action::CopyPath(path) => ctx.copy_text(path.to_string_lossy().into_owned()),
            Action::Submit(operation) => {
                self.ui.dialog = None;
                self.submit(operation);
            }
            Action::QuickLook(path) => self.quick_look(path),
            Action::Favorite(path) => {
                if let Some(index) = self.config.favorites.iter().position(|p| p == &path) {
                    self.config.favorites.remove(index);
                } else if self.config.favorites.len() < 64 {
                    self.config.favorites.push(path);
                }
                self.changed(ctx);
            }
            Action::PreferencesChanged => {
                self.config.validate();
                crate::theme::apply(ctx, &self.config);
                // Reset preferences also returns the zoom to 100%.
                ctx.set_zoom_factor(self.config.window_zoom);
                if self.applied_hidden != self.config.show_hidden {
                    self.applied_hidden = self.config.show_hidden;
                    self.reload_visible(ctx);
                }
                self.changed(ctx);
            }
            Action::View(mode) => {
                self.config.view = mode;
                self.changed(ctx);
            }
            Action::Zoom(step) => {
                let previous = self.config.window_zoom;
                self.config.step_zoom(step);
                if self.config.window_zoom != previous {
                    ctx.set_zoom_factor(self.config.window_zoom);
                    self.changed(ctx);
                }
                // At either end of the range the chip still answers, with the
                // value kept.
                self.ui.level = Some(crate::controls::Level {
                    name: "Window zoom",
                    value: format!("{:.0}%", self.config.window_zoom * 100.0),
                    shown: ctx.input(|input| input.time),
                });
                ctx.request_repaint();
            }
            Action::Location(path) => {
                let text = path.trim();
                let path = if text == "~" {
                    self.home.clone()
                } else if let Some(tail) = text.strip_prefix("~/") {
                    self.home.join(tail)
                } else {
                    let p = PathBuf::from(text);
                    if p.is_absolute() {
                        p
                    } else {
                        self.workspace.pane().path.join(p)
                    }
                };
                self.ui.dialog = None;
                self.navigate(id, path, ctx);
            }
        }
    }
    fn close(&mut self, ctx: &egui::Context) {
        if self.busy.is_empty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            self.ui.notice = Some(ui::Notice::info(
                "Please wait for the file operation to finish before closing this window.",
            ));
        }
    }
    fn capture(&mut self, ctx: &egui::Context) {
        if self.launch.screenshot.is_none() {
            return;
        }
        if self.started.elapsed() > Duration::from_secs(3) && !self.capture_sent {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            self.capture_sent = true;
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event
                && let Some(path) = self.launch.screenshot.take()
            {
                let width = image.size[0] as u32;
                let height = image.size[1] as u32;
                let bytes = image
                    .pixels
                    .iter()
                    .flat_map(|p| p.to_array())
                    .collect::<Vec<_>>();
                match image::save_buffer(&path, &bytes, width, height, image::ColorType::Rgba8) {
                    Ok(()) => println!("Screenshot saved to {}", path.display()),
                    Err(e) => eprintln!("Screenshot: {e}"),
                };
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}
fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}
impl eframe::App for FinderApp {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        if !self.busy.is_empty() && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.ui.notice = Some(ui::Notice::info(
                "Please wait for the file operation to finish before closing this window.",
            ));
        }
        let mut actions = Vec::new();
        ui::show(
            ui,
            &mut self.workspace,
            &mut self.config,
            &mut self.ui,
            &mut actions,
        );
        if self.initialized {
            for action in actions {
                self.dispatch(action, &ctx);
            }
        }
        self.capture(&ctx);
    }
    fn on_exit(&mut self) {
        if self.initialized && self.writable && !self.ephemeral {
            self.storage
                .save(self.workspace.snapshot(self.config.clone()));
        }
        self.storage.finish();
    }
}
