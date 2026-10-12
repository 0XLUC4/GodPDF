//! Application state, layout and command dispatch.

mod shell;

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::path::PathBuf;

use egui::{
    Button, CentralPanel, Color32, Context, CornerRadius, Frame, Id, Margin, Panel, RichText,
    Stroke, TextureHandle, Ui, ViewportCommand, vec2,
};
use godpdf_core::{self as core, Annotation, Metadata, Session};

use crate::commands::{self, Cmd, CommentTool, Group};
use crate::dialogs::{self, Dialog, Outcome, PaletteOutcome, PaletteState, TextView};
use crate::doc::{Budget, DocTab, ViewMode, guarded};
use crate::jobs::{Done, Jobs};
use crate::platform::{self, Inbox, PickedFile, Purpose, Saved};
use crate::quickbar::{self, StyleKind, StyleTarget, Tool};
use crate::selection;
use crate::textbox::{self, Placement, TextBox};
use crate::theme::{self, Palette};
use crate::toasts::Toasts;
use crate::tools::{CommentStyle, Mode};
use crate::widgets::{icon, with_icon};
use crate::{branding, comments, geometry, organize, ranges, sidebar, updates, viewer};

const MAX_RECENT: usize = 8;
const ZOOM_STEP: f32 = 1.2;

pub struct GodApp {
    tabs: Vec<DocTab>,
    active: usize,
    next_id: u64,
    dark: bool,
    /// Right panel (comments, pages, bookmarks, search) open.
    show_sidebar: bool,
    /// Header tab: what the left tools panel lists.
    mode: Mode,
    /// Left tools panel open (hidden anyway in Read mode).
    show_tools: bool,
    /// "View more" unfolded in the tools panel.
    tools_expanded: bool,
    /// Home (welcome) screen shown over open documents.
    home: bool,
    sidebar_tab: sidebar::Tab,
    comments: comments::Cache,
    /// Banners closed by the user, per tab id.
    dismissed: Vec<(u64, shell::Banner)>,
    recent: Vec<PathBuf>,
    logo: Option<TextureHandle>,
    inbox: Inbox,
    jobs: Jobs,
    /// For waking the UI from background jobs.
    ctx: Context,
    open_queue: VecDeque<PickedFile>,
    toasts: Toasts,
    dialog: Option<Dialog>,
    palette: Option<PaletteState>,
    text_view: Option<TextView>,
    /// What a click or drag on the page does (Select by default).
    tool: Tool,
    /// Style of new comments, per tool kind.
    styles: quickbar::Styles,
    /// An eraser drag is in progress: its removals fold into one undo step.
    erase_stroke: bool,
    /// Free-text box being typed on a page.
    text_box: Option<TextBox>,
    /// Command to run once the next document is open (welcome tool cards).
    pending_cmd: Option<Cmd>,
    focus_search: bool,
    quit_confirmed: bool,
    window_title: String,
    /// Tab shown in a second viewer, right of the one in front (side by side).
    split: Option<u64>,
    /// A click on a line of the page changes its words (Tools: Edit PDF text).
    edit_text: bool,
    updates: updates::Checker,
    restart: Option<PathBuf>,
}

impl GodApp {
    /// `files` are opened at startup (command line / "Open with").
    pub fn new(cc: &eframe::CreationContext<'_>, files: Vec<PathBuf>) -> Self {
        let get = |key: &str| cc.storage.and_then(|storage| storage.get_string(key));
        let flag = |key: &str, default: bool| get(key).map_or(default, |value| value == "1");
        // Key renamed when white became the default theme.
        let dark = flag("dark_mode", false);
        theme::install(&cc.egui_ctx, dark);
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        textbox::install_font(&mut fonts);
        cc.egui_ctx.set_fonts(fonts);
        let recent = get("recent")
            .map(|list| {
                list.lines()
                    .filter(|line| !line.is_empty())
                    .map(PathBuf::from)
                    .collect()
            })
            .unwrap_or_default();
        let mut app = Self {
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
            dark,
            show_sidebar: flag("sidebar", true),
            mode: Mode::AllTools,
            show_tools: flag("tools_panel", true),
            tools_expanded: false,
            home: false,
            sidebar_tab: sidebar::Tab::Pages,
            comments: comments::Cache::default(),
            dismissed: Vec::new(),
            recent,
            logo: branding::logo_texture(&cc.egui_ctx),
            inbox: Inbox::default(),
            jobs: Jobs::default(),
            ctx: cc.egui_ctx.clone(),
            open_queue: VecDeque::new(),
            toasts: Toasts::default(),
            dialog: None,
            palette: None,
            text_view: None,
            tool: Tool::Select,
            styles: quickbar::Styles::default(),
            erase_stroke: false,
            text_box: None,
            pending_cmd: None,
            focus_search: false,
            quit_confirmed: false,
            window_title: String::new(),
            split: None,
            edit_text: false,
            updates: updates::Checker::default(),
            restart: None,
        };
        updates::remove_old_program();
        app.updates.check(&cc.egui_ctx);
        for path in files {
            app.open_path(&path);
        }
        app
    }
}

impl eframe::App for GodApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.process_inbox();
        self.process_jobs();
        self.process_open_queue();
        self.handle_dropped_files(&ctx);
        self.handle_shortcuts(&ctx);
        self.process_update(&ctx);
        self.handle_close_request(&ctx);
        self.sync_text_box();
        if !ctx.input(|input| input.pointer.any_down()) {
            self.erase_stroke = false;
        }
        self.update_window_title(&ctx);

        let mut clicked: Vec<Cmd> = Vec::new();
        let palette = Palette::of(ui);

        Panel::top("tab-row")
            .frame(Frame::new().fill(palette.surface).inner_margin(Margin {
                left: 8,
                right: 8,
                top: 4,
                bottom: 0,
            }))
            .show_separator_line(false)
            .show(ui, |ui| self.tab_row(ui, &mut clicked));
        Panel::top("header")
            .frame(
                Frame::new()
                    .fill(palette.bg)
                    .inner_margin(Margin::symmetric(10, 0)),
            )
            .show(ui, |ui| self.header_row(ui, &mut clicked));

        let doc_view = self.doc_view();
        if doc_view {
            self.doc_panels(ui, &mut clicked);
        }

        // Welcome sits on the plain background; documents on the canvas so pages stand out.
        let central_fill = if doc_view { palette.canvas } else { palette.bg };
        CentralPanel::default()
            .frame(Frame::new().fill(central_fill))
            .show(ui, |ui| {
                if doc_view {
                    self.central(ui, &mut clicked);
                } else {
                    self.welcome_central(ui, &mut clicked);
                }
            });

        for cmd in clicked {
            self.run(&ctx, cmd);
        }
        self.show_overlays(&ctx);
        self.toasts.show(&ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let flag = |on: bool| if on { "1" } else { "0" }.to_owned();
        storage.set_string("dark_mode", flag(self.dark));
        storage.set_string("sidebar", flag(self.show_sidebar));
        storage.set_string("tools_panel", flag(self.show_tools));
        let recent: Vec<String> = self
            .recent
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        storage.set_string("recent", recent.join("\n"));
    }
}

// ---------------------------------------------------------------- layout

impl GodApp {
    fn menus(&mut self, ui: &mut Ui, clicked: &mut Vec<Cmd>) {
        let has_doc = !self.tabs.is_empty();
        let item = |ui: &mut Ui, cmd: Cmd, clicked: &mut Vec<Cmd>| {
            let shortcut = cmd
                .shortcut()
                .map(|sc| ui.ctx().format_shortcut(&sc))
                .unwrap_or_default();
            let button = Button::new(with_icon(cmd.icon(), cmd.label())).shortcut_text(shortcut);
            if ui
                .add_enabled(has_doc || !cmd.needs_doc(), button)
                .clicked()
            {
                clicked.push(cmd);
            }
        };
        ui.menu_button("File", |ui| {
            item(ui, Cmd::Open, clicked);
            ui.menu_button("Open recent", |ui| {
                if self.recent.is_empty() {
                    ui.label(RichText::new("Nothing yet").weak());
                }
                let mut chosen = None;
                for path in &self.recent {
                    if ui.button(path.display().to_string()).clicked() {
                        chosen = Some(path.clone());
                    }
                }
                if let Some(path) = chosen {
                    self.open_path(&path);
                }
            });
            item(ui, Cmd::Merge, clicked);
            ui.separator();
            item(ui, Cmd::Save, clicked);
            item(ui, Cmd::SaveAs, clicked);
            item(ui, Cmd::Print, clicked);
            item(ui, Cmd::ExportPages, clicked);
            ui.separator();
            item(ui, Cmd::Close, clicked);
            if !cfg!(target_arch = "wasm32") {
                item(ui, Cmd::Quit, clicked);
            }
        });
        ui.menu_button("Edit", |ui| {
            let (undo, redo) = self.tabs.get(self.active).map_or((None, None), |tab| {
                (
                    tab.session.undo_label().map(str::to_owned),
                    tab.session.redo_label().map(str::to_owned),
                )
            });
            for (cmd, label) in [(Cmd::Undo, undo), (Cmd::Redo, redo)] {
                let text = match &label {
                    Some(label) => with_icon(cmd.icon(), &format!("{} “{label}”", cmd.label())),
                    None => with_icon(cmd.icon(), cmd.label()),
                };
                let shortcut = cmd
                    .shortcut()
                    .map(|sc| ui.ctx().format_shortcut(&sc))
                    .unwrap_or_default();
                if ui
                    .add_enabled(label.is_some(), Button::new(text).shortcut_text(shortcut))
                    .clicked()
                {
                    clicked.push(cmd);
                }
            }
            ui.separator();
            item(ui, Cmd::Palette, clicked);
            if ui.button(with_icon(icon::GEAR, "Preferences…")).clicked() {
                self.dialog = Some(Dialog::Preferences);
            }
        });
        ui.menu_button("View", |ui| {
            for cmd in commands::in_group(Group::View) {
                item(ui, cmd, clicked);
            }
            item(ui, Cmd::Organize, clicked);
        });
        ui.menu_button("Tools", |ui| {
            for group in Group::TOOLS {
                ui.menu_button(group.title(), |ui| {
                    for cmd in commands::in_group(group) {
                        item(ui, cmd, clicked);
                    }
                });
            }
        });
        ui.menu_button("Help", |ui| item(ui, Cmd::About, clicked));
    }

    fn central(&mut self, ui: &mut Ui, clicked: &mut Vec<Cmd>) {
        if self.tabs.is_empty() {
            self.welcome_central(ui, clicked);
            return;
        }
        // Its document was closed: back to one viewer.
        if self.split.is_some_and(|id| self.tab_index(id).is_none()) {
            self.split = None;
        }
        let reading = self.tabs[self.active].mode == ViewMode::Read;
        let mut left = ui.max_rect();
        let mut beside = None;
        if self.split.is_some() && reading {
            let (first, second) = left.split_left_right_at_fraction(0.5);
            left = first;
            let place = egui::UiBuilder::new().id_salt("companion").max_rect(second);
            beside = ui.scope_builder(place, |ui| self.companion(ui)).inner;
        }
        // Named the same with one viewer or two: the document stays scrolled where it was
        // when the second one opens or closes.
        let place = egui::UiBuilder::new().id_salt("document").max_rect(left);
        ui.scope_builder(place, |ui| self.document(ui, clicked, beside));
    }

    /// The second viewer, on the right: another open document, or another place of the one
    /// in front. It is for reading: the tools act on the first. Returns the pages it shows
    /// when its document is the one in front.
    fn companion(&mut self, ui: &mut Ui) -> Option<(usize, usize)> {
        let index = self.tab_index(self.split?)?;
        let same = index == self.active;
        let palette = Palette::of(ui);
        let rect = ui.max_rect();
        let line = Stroke::new(1.0, palette.border);
        ui.painter().vline(rect.left(), rect.y_range(), line);
        let names: Vec<(u64, String)> = self
            .tabs
            .iter()
            .map(|tab| (tab.id, tab.title().to_owned()))
            .collect();
        let tab = &mut self.tabs[index];
        if same {
            tab.swap_place();
        }
        let (shown, count, title) = (tab.id, tab.page_count(), tab.title().to_owned());
        let mut page = tab.current_page + 1;
        let mut pick = None;
        let mut close = false;
        let mut jump = false;
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            // The page number and the close button keep their room in a narrow viewer.
            let width = (ui.available_width() - 150.0).clamp(80.0, 240.0);
            egui::ComboBox::from_id_salt("companion-document")
                .selected_text(title)
                .width(width)
                .show_ui(ui, |ui| {
                    for (id, name) in &names {
                        if ui.selectable_label(*id == shown, name).clicked() {
                            pick = Some(*id);
                        }
                    }
                });
            let field = egui::DragValue::new(&mut page).range(1..=count.max(1));
            jump = ui.add(field).on_hover_text("Page shown here").changed();
            ui.label(RichText::new(format!("/ {count}")).color(palette.text_dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(6.0);
                let tip = "Close the second view";
                close = crate::widgets::small_icon_button(ui, icon::X, tip).clicked();
            });
        });
        if jump {
            tab.current_page = page - 1;
            tab.scroll_to_page = Some(page - 1);
        }
        let mut budget = Budget(1);
        let out = viewer::show(ui, tab, Tool::Hand, None, &self.styles, &mut budget, false);
        if same {
            tab.swap_place();
        } else if let Some((first, last)) = out.visible {
            tab.retain_pages(first.saturating_sub(2)..=last + 2);
        }
        if close {
            self.split = None;
        } else if let Some(id) = pick {
            let front = &mut self.tabs[self.active];
            if id == front.id && !same {
                front.start_second_view();
            }
            self.split = Some(id);
        }
        out.visible.filter(|_| same)
    }

    /// The document in front, with its tools. `beside`: the pages a second viewer of the
    /// same document shows.
    fn document(&mut self, ui: &mut Ui, clicked: &mut Vec<Cmd>, beside: Option<(usize, usize)>) {
        let mut budget = Budget(2);
        let tab = &mut self.tabs[self.active];
        match tab.mode {
            ViewMode::Read => {
                let canvas = ui.max_rect();
                let text_box = self
                    .text_box
                    .as_mut()
                    .filter(|text_box| text_box.tab == tab.id);
                let editing = text_box.is_some();
                let edit_text = self.edit_text;
                let (tool, styles) = (self.tool, &self.styles);
                let out = viewer::show(ui, tab, tool, text_box, styles, &mut budget, edit_text);
                // Pages far from every viewer of this document give their textures back.
                let seen = [out.visible, beside].into_iter().flatten();
                if let Some((first, last)) = seen.reduce(|a, b| (a.0.min(b.0), a.1.max(b.1))) {
                    tab.retain_pages(first.saturating_sub(2)..=last + 2);
                }
                let target = if editing {
                    StyleTarget::TextBox
                } else if let Some((kind, style)) = selected_style(tab) {
                    StyleTarget::Selected(kind, style)
                } else {
                    StyleTarget::Tool(self.tool)
                };
                let bar = quickbar::show(
                    ui,
                    canvas,
                    self.tool,
                    &mut self.styles,
                    target,
                    tab.session.undo_label().is_some(),
                    tab.session.redo_label().is_some(),
                );
                if let Some(tool) = bar.tool {
                    self.set_tool(tool);
                }
                clicked.extend(bar.cmd);
                if let Some(style) = bar.restyle {
                    self.restyle_selected(&style);
                }
                if let Some(finished) = out.gesture {
                    self.commit_gesture(finished);
                }
                match out.text_event {
                    Some(textbox::Event::Commit) => self.commit_text_box(),
                    Some(textbox::Event::Cancel) => self.cancel_text_box(),
                    // A click that just closed a box must not open another.
                    None => {
                        if let Some((page, point, new_allowed)) = out.open_text {
                            self.open_text_at(page, point, new_allowed);
                        }
                    }
                }
                match out.select {
                    Some(selection::Action::Move { page, id, moved }) => {
                        let label = format!("Move {}", moved.kind_name().to_lowercase());
                        let result = self.tabs[self.active].apply(&label, |pdf| {
                            core::replace_annotation(pdf, page, &id, &moved)
                        });
                        self.report(result, None);
                    }
                    Some(selection::Action::Resize { page, id, resized }) => {
                        let label = format!("Resize {}", resized.kind_name().to_lowercase());
                        let result = self.tabs[self.active].apply(&label, |pdf| {
                            core::replace_annotation(pdf, page, &id, &resized)
                        });
                        self.report(result, None);
                    }
                    Some(selection::Action::Open { page, point }) => {
                        self.open_text_at(page, point, false);
                    }
                    None => {}
                }
                if let Some((page, bands)) = out.highlight {
                    self.highlight_text(page, &bands);
                }
                if let Some((page, line, text)) = out.edit_line {
                    self.ask_new_text(page, line, text);
                }
                if let Some((page, id)) = out.erase {
                    self.erase(page, &id);
                }
            }
            ViewMode::Organize => {
                let out = organize::show(ui, tab, &mut budget);
                if let Some(page) = out.open_page {
                    tab.mode = ViewMode::Read;
                    self.goto_page(page);
                }
                if let Some((page, delta)) = out.move_page {
                    self.move_pages(&[page], delta);
                }
                if let Some((order, moved_to)) = out.reorder {
                    let tab = &mut self.tabs[self.active];
                    let result = tab.apply("Reorder pages", |pdf| core::reorder_pages(pdf, &order));
                    if result.is_ok() {
                        tab.selection = moved_to.into_iter().collect();
                    }
                    self.report(result, None);
                }
                clicked.extend(out.cmd);
            }
        }
    }

    fn show_overlays(&mut self, ctx: &Context) {
        if ctx.input(|input| !input.raw.hovered_files.is_empty()) {
            let rect = ctx.content_rect();
            let painter =
                ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("drop")));
            painter.rect_filled(rect, 0.0, Color32::from_black_alpha(170));
            painter.rect_stroke(
                rect.shrink(16.0),
                CornerRadius::same(16),
                Stroke::new(2.0, Color32::WHITE),
                egui::StrokeKind::Inside,
            );
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Drop PDFs to open them",
                egui::FontId::proportional(26.0),
                Color32::WHITE,
            );
        }

        if let Some(view) = &self.text_view
            && !dialogs::text_window(ctx, view)
        {
            self.text_view = None;
        }

        if let Some(state) = &mut self.palette {
            match dialogs::command_palette(ctx, state) {
                PaletteOutcome::Keep => {}
                PaletteOutcome::Close => self.palette = None,
                PaletteOutcome::Run(cmd) => {
                    self.palette = None;
                    self.run(ctx, cmd);
                }
            }
        }

        if let Some(mut dialog) = self.dialog.take() {
            let mut dark = self.dark;
            let env = dialogs::Env {
                logo: self.logo.as_ref(),
                dark: &mut dark,
                updates: &self.updates,
            };
            let outcome = dialogs::show(ctx, &mut dialog, env);
            if dark != self.dark {
                self.set_dark(ctx, dark);
            }
            match outcome {
                Outcome::Keep => self.dialog = Some(dialog),
                Outcome::Cancel => {
                    if matches!(dialog, Dialog::ConfirmQuit) {
                        self.restart = None;
                    }
                }
                Outcome::Primary | Outcome::Secondary => {
                    self.finish_dialog(ctx, dialog, outcome == Outcome::Primary);
                }
            }
        }
    }
}

// ---------------------------------------------------------------- input

impl GodApp {
    fn process_inbox(&mut self) {
        for (purpose, files) in self.inbox.drain() {
            match purpose {
                Purpose::Open => self.open_queue.extend(files),
                Purpose::MergeAdd => match &mut self.dialog {
                    Some(Dialog::Merge { files: list }) => list.extend(files),
                    _ => self.dialog = Some(Dialog::Merge { files }),
                },
                Purpose::Insert { tab, at } => {
                    let Some(index) = self.tab_index(tab) else {
                        continue;
                    };
                    for file in files {
                        let mut bytes = file.bytes;
                        if core::is_encrypted(&bytes) {
                            // Owner-password-only files open with an empty password.
                            match guarded(|| core::decrypt(&bytes, "")) {
                                Ok(plain) => bytes = plain,
                                Err(_) => {
                                    self.toasts.error(format!(
                                        "“{}” is password-protected; open it and remove the password first",
                                        file.name
                                    ));
                                    continue;
                                }
                            }
                        }
                        let label = format!("Insert {}", file.name);
                        let result =
                            self.tabs[index].apply(&label, |pdf| core::insert_pdf(pdf, at, &bytes));
                        self.report(result, Some(&format!("Inserted “{}”", file.name)));
                    }
                }
            }
        }
    }

    fn process_open_queue(&mut self) {
        while self.dialog.is_none() {
            let Some(file) = self.open_queue.pop_front() else {
                break;
            };
            if core::is_encrypted(&file.bytes) {
                // Owner-password-only files need no password to open.
                match guarded(|| core::decrypt(&file.bytes, "")) {
                    Ok(bytes) => self.add_unlocked(file, bytes, String::new()),
                    Err(_) => {
                        self.dialog = Some(Dialog::Password {
                            file,
                            password: String::new(),
                            error: None,
                        });
                    }
                }
            } else {
                self.add_tab(file, false);
            }
        }
    }

    fn handle_dropped_files(&mut self, ctx: &Context) {
        let dropped = ctx.input(|input| input.raw.dropped_files.clone());
        for file in dropped {
            if let Some(bytes) = file.bytes {
                self.open_queue.push_back(PickedFile {
                    name: file.name,
                    path: None,
                    bytes: bytes.to_vec(),
                });
            } else if let Some(path) = file.path {
                match platform::read_file(&path) {
                    Ok(file) => self.open_queue.push_back(file),
                    Err(err) => self.toasts.error(err),
                }
            }
        }
    }

    fn handle_shortcuts(&mut self, ctx: &Context) {
        if self.dialog.is_some() || self.palette.is_some() {
            return;
        }
        let typing = ctx.egui_wants_keyboard_input();
        let fired: Vec<Cmd> = ctx.input_mut(|input| {
            Cmd::key_bindings()
                .into_iter()
                .filter(|(_, cmd)| {
                    !(typing
                        && matches!(
                            cmd,
                            Cmd::Undo
                                | Cmd::Redo
                                | Cmd::ZoomIn
                                | Cmd::ZoomOut
                                | Cmd::MoveEarlier
                                | Cmd::MoveLater
                        ))
                })
                .filter(|(shortcut, _)| input.consume_shortcut(shortcut))
                .map(|(_, cmd)| cmd)
                .collect()
        });
        if !typing && self.doc_view() {
            if let Some(tool) = quickbar::key_tool(ctx) {
                self.set_tool(tool);
            }
            let (escape, delete) = ctx.input(|input| {
                (
                    input.key_pressed(egui::Key::Escape),
                    input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace),
                )
            });
            if escape && self.edit_text {
                self.edit_text = false;
            } else if escape && self.tool != Tool::Select {
                self.set_tool(Tool::Select);
            }
            if delete && self.tool == Tool::Select {
                self.delete_selected();
            }
        }
        for cmd in fired {
            self.run(ctx, cmd);
        }
    }

    fn handle_close_request(&mut self, ctx: &Context) {
        let requested = ctx.input(|input| input.viewport().close_requested());
        if requested {
            self.commit_text_box();
        }
        if requested && !self.quit_confirmed && self.tabs.iter().any(DocTab::is_dirty) {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.dialog = Some(Dialog::ConfirmQuit);
        } else if requested && let Some(program) = self.restart.take() {
            #[cfg(not(target_arch = "wasm32"))]
            if let Err(err) = std::process::Command::new(&program).spawn() {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                self.toasts
                    .error(format!("GodPDF is updated; start it again ({err})"));
            }
            #[cfg(target_arch = "wasm32")]
            let _ = program;
        }
    }

    fn process_update(&mut self, ctx: &Context) {
        if let Some(program) = self.updates.take_installed() {
            self.restart = Some(program);
            self.dialog = None;
            self.commit_text_box();
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    fn update_window_title(&mut self, ctx: &Context) {
        let title = match self.tabs.get(self.active) {
            Some(tab) => format!(
                "{}{} - {}",
                if tab.is_dirty() { "* " } else { "" },
                tab.title(),
                crate::APP_NAME
            ),
            None => crate::APP_NAME.to_owned(),
        };
        if title != self.window_title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }
}

// ---------------------------------------------------------------- commands

impl GodApp {
    fn run(&mut self, ctx: &Context, cmd: Cmd) {
        if cmd.needs_doc() && self.tabs.is_empty() {
            self.toasts.info("Open a PDF first");
            return;
        }
        // Any document command finishes the text being typed first; view commands keep it.
        if !matches!(cmd.group(), Group::View | Group::Help) && cmd != Cmd::Palette {
            self.commit_text_box();
        }
        if let Some(tool) = cmd.comment_tool() {
            self.toggle_tool(tool);
            return;
        }
        match cmd {
            Cmd::Open => self.pick(ctx, true, Purpose::Open),
            Cmd::Merge => {
                let files: Vec<PickedFile> = self
                    .tabs
                    .get(self.active)
                    .map(|tab| PickedFile {
                        name: tab.title().to_owned(),
                        path: None,
                        bytes: tab.session.bytes().to_vec(),
                    })
                    .into_iter()
                    .collect();
                let empty = files.is_empty();
                self.dialog = Some(Dialog::Merge { files });
                if empty {
                    self.pick(ctx, true, Purpose::MergeAdd);
                }
            }
            Cmd::Save => {
                self.save(self.active, false);
            }
            Cmd::SaveAs => {
                self.save(self.active, true);
            }
            Cmd::Print => self.print(),
            Cmd::Close => self.request_close(self.active),
            Cmd::Quit => ctx.send_viewport_cmd(ViewportCommand::Close),
            Cmd::Undo | Cmd::Redo => self.undo_redo(cmd == Cmd::Undo),
            Cmd::ToggleTheme => self.set_dark(ctx, !self.dark),
            Cmd::ZoomIn => self.with_tab(|tab| tab.set_zoom(tab.zoom * ZOOM_STEP)),
            Cmd::ZoomOut => self.with_tab(|tab| tab.set_zoom(tab.zoom / ZOOM_STEP)),
            Cmd::FitWidth => self.with_tab(|tab| {
                tab.set_zoom(viewer::fit_width_zoom(tab));
                tab.mode = ViewMode::Read;
            }),
            Cmd::FitPage => self.with_tab(|tab| {
                tab.set_zoom(viewer::fit_page_zoom(tab));
                tab.scroll_to_page = Some(tab.current_page);
                tab.mode = ViewMode::Read;
            }),
            Cmd::ToggleThumbnails => self.show_sidebar = !self.show_sidebar,
            Cmd::Organize => self.with_tab(|tab| {
                tab.mode = if tab.mode == ViewMode::Organize {
                    ViewMode::Read
                } else {
                    ViewMode::Organize
                };
            }),
            Cmd::RotateLeft | Cmd::RotateRight => {
                let degrees = if cmd == Cmd::RotateLeft { -90 } else { 90 };
                let tab = &mut self.tabs[self.active];
                let pages = tab.target_pages();
                let selection = tab.selection.clone();
                let result = tab.apply(cmd.label(), |pdf| core::rotate_pages(pdf, &pages, degrees));
                if result.is_ok() {
                    // Rotation keeps page indices: keep the selection.
                    tab.selection = selection;
                }
                self.report(result, None);
            }
            Cmd::RotateAll => {
                let tab = &mut self.tabs[self.active];
                let pages: Vec<usize> = (0..tab.page_count()).collect();
                let selection = tab.selection.clone();
                let result = tab.apply(cmd.label(), |pdf| core::rotate_pages(pdf, &pages, 90));
                if result.is_ok() {
                    tab.selection = selection;
                }
                self.report(result, None);
            }
            Cmd::DeletePages => {
                let tab = &mut self.tabs[self.active];
                let pages = tab.target_pages();
                let label = format!("Delete {} page(s)", pages.len());
                let result = tab.apply(&label, |pdf| core::delete_pages(pdf, &pages));
                self.report(result, Some(&label));
            }
            Cmd::MoveEarlier | Cmd::MoveLater => {
                let pages = self.tabs[self.active].target_pages();
                self.move_pages(&pages, if cmd == Cmd::MoveEarlier { -1 } else { 1 });
            }
            Cmd::InsertBlank => {
                let tab = &mut self.tabs[self.active];
                let after = tab.target_pages().into_iter().max().unwrap_or(0);
                let size = tab
                    .page_sizes
                    .get(after)
                    .copied()
                    .unwrap_or(vec2(595.0, 842.0));
                let at = (after + 1).min(tab.page_count());
                let result = tab.apply("Insert blank page", |pdf| {
                    core::insert_blank_page(pdf, at, size.x, size.y)
                });
                self.report(result, None);
            }
            Cmd::InsertFile => {
                let tab = &self.tabs[self.active];
                let at = tab
                    .target_pages()
                    .into_iter()
                    .max()
                    .map_or(tab.page_count(), |page| page + 1);
                let id = tab.id;
                self.pick(ctx, false, Purpose::Insert { tab: id, at });
            }
            Cmd::Extract | Cmd::ExportPages => {
                let tab = &self.tabs[self.active];
                let mut pages = tab.target_pages();
                if tab.mode == ViewMode::Read {
                    pages = (0..tab.page_count()).collect();
                }
                pages.sort_unstable();
                self.dialog = Some(Dialog::Extract {
                    tab: tab.id,
                    range: ranges::format(&pages),
                    error: None,
                });
            }
            Cmd::Split => {
                let id = self.tabs[self.active].id;
                self.dialog = Some(Dialog::Split {
                    tab: id,
                    every: true,
                    n: 1,
                    starts: String::new(),
                    error: None,
                });
            }
            Cmd::Encrypt => {
                let id = self.tabs[self.active].id;
                self.dialog = Some(Dialog::Encrypt {
                    tab: id,
                    user: String::new(),
                    owner: String::new(),
                    confirm: String::new(),
                    error: None,
                });
            }
            Cmd::RemovePassword => {
                self.tabs[self.active].encrypt_on_save = None;
                self.toasts
                    .info("The document will be saved without a password");
            }
            Cmd::Properties => self.open_properties(),
            Cmd::ExtractText => self.extract_text(),
            Cmd::Find => {
                self.show_sidebar = true;
                self.sidebar_tab = sidebar::Tab::Search;
                self.focus_search = true;
            }
            Cmd::EditText => {
                self.edit_text = !self.edit_text;
                if self.edit_text {
                    self.commit_text_box();
                    self.with_tab(|tab| tab.mode = ViewMode::Read);
                    self.toasts
                        .info("Click a line of text to change its words. Esc to finish.");
                }
            }
            Cmd::SplitView => {
                if self.split.take().is_none() {
                    // The document in front, twice; the second viewer can show another.
                    let tab = &mut self.tabs[self.active];
                    tab.mode = ViewMode::Read;
                    tab.start_second_view();
                    self.split = Some(tab.id);
                }
            }
            Cmd::GoToPage => {
                let tab = &self.tabs[self.active];
                self.dialog = Some(Dialog::GoToPage {
                    tab: tab.id,
                    page: String::new(),
                    count: tab.page_count(),
                    error: None,
                });
            }
            Cmd::Palette => self.palette = Some(PaletteState::default()),
            Cmd::About => self.dialog = Some(Dialog::About),
            Cmd::Updates => {
                if !matches!(self.updates.state(), updates::State::Ready(_)) {
                    self.updates.check(ctx);
                }
                self.dialog = Some(Dialog::Updates);
            }
            Cmd::Highlight | Cmd::Note | Cmd::FreeText | Cmd::Rectangle | Cmd::Ink => {}
        }
    }

    fn with_tab(&mut self, edit: impl FnOnce(&mut DocTab)) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            edit(tab);
        }
    }

    /// A comment command: pick its tool, or go back to Select when it is already active.
    fn toggle_tool(&mut self, tool: CommentTool) {
        let tool = Tool::from_comment(tool);
        if self.tool == tool {
            self.set_tool(Tool::Select);
            return;
        }
        self.set_tool(tool);
        self.with_tab(|tab| tab.mode = ViewMode::Read);
    }

    /// Change the canvas tool; a text box being typed is committed first.
    /// A line of the page was clicked in edit-text mode: ask for its new words.
    fn ask_new_text(&mut self, page: usize, line: egui::Rect, text: String) {
        let tab = &self.tabs[self.active];
        let Some(&size) = tab.page_sizes.get(page) else {
            return;
        };
        match geometry::page_geometry(tab.session.bytes(), page) {
            Ok(geom) => {
                self.dialog = Some(Dialog::EditText {
                    tab: tab.id,
                    page,
                    area: geometry::to_pdf_rect(&geom, size, line.min, line.max),
                    text,
                    error: None,
                });
            }
            Err(err) => self.toasts.error(format!("Cannot edit this text: {err}")),
        }
    }

    fn set_tool(&mut self, tool: Tool) {
        // Picking a tool ends the editing of the page's words.
        self.edit_text = false;
        if self.tool != tool {
            self.commit_text_box();
            self.with_tab(|tab| tab.canvas = selection::CanvasState::default());
        }
        self.tool = tool;
    }

    /// Selected annotation of the active tab: (page, id, parsed annotation).
    fn selected_annotation(&mut self) -> Option<(usize, String, Option<Annotation>)> {
        let tab = self.tabs.get_mut(self.active)?;
        let selected = tab.canvas.selected.clone()?;
        let annotation = tab
            .annotations(selected.page)
            .ok()?
            .iter()
            .find(|placed| placed.id == selected.id)?
            .annotation
            .clone();
        Some((selected.page, selected.id, annotation))
    }

    fn delete_selected(&mut self) {
        let Some((page, id, _)) = self.selected_annotation() else {
            return;
        };
        let result = self.tabs[self.active].apply("Delete comment", |pdf| {
            core::remove_annotation(pdf, page, &id)
        });
        self.report(result, None);
    }

    /// Highlight the text lines `bands` (displayed points) of `page`, as one undo step.
    fn highlight_text(&mut self, page: usize, bands: &[egui::Rect]) {
        let color = core_color(self.styles.highlight.color);
        let tab = &mut self.tabs[self.active];
        let Some(&size) = tab.page_sizes.get(page) else {
            return;
        };
        let result = tab.page_geom(page).and_then(|geom| {
            let highlights: Vec<Annotation> = bands
                .iter()
                .map(|band| Annotation::Highlight {
                    rect: geometry::to_pdf_rect(&geom, size, band.min, band.max),
                    color,
                })
                .collect();
            if highlights.is_empty() {
                return Ok(());
            }
            tab.apply("Highlight text", |pdf| {
                core::add_annotations(pdf, page, &highlights)
            })
        });
        self.report(result, None);
    }

    /// Eraser hit: remove the annotation; one drag is one undo step.
    fn erase(&mut self, page: usize, id: &str) {
        let tab = &mut self.tabs[self.active];
        let edit = |pdf: &[u8]| core::remove_annotation(pdf, page, id);
        let result = if self.erase_stroke {
            tab.amend("Erase", edit)
        } else {
            tab.apply("Erase", edit)
        };
        self.erase_stroke |= result.is_ok();
        self.report(result, None);
    }

    /// Apply the style card to the selected annotation.
    fn restyle_selected(&mut self, style: &CommentStyle) {
        let Some((page, id, Some(annotation))) = self.selected_annotation() else {
            return;
        };
        let color = core_color(style.color);
        let restyled = match annotation {
            Annotation::Highlight { rect, .. } => Annotation::Highlight { rect, color },
            Annotation::Square { rect, .. } => Annotation::Square {
                rect,
                color,
                width: style.width,
            },
            Annotation::Ink { strokes, .. } => Annotation::Ink {
                strokes,
                color,
                width: style.width,
            },
            Annotation::FreeText {
                rect,
                text,
                rotation,
                ..
            } => Annotation::FreeText {
                // Same top edge and width; core refits the height to the wrapped text.
                rect,
                text,
                font_size: style.font_size,
                color,
                rotation,
            },
            Annotation::Note { .. } => return,
        };
        let result = self.tabs[self.active].apply("Change style", |pdf| {
            core::replace_annotation(pdf, page, &id, &restyled)
        });
        self.report(result, None);
    }

    fn set_dark(&mut self, ctx: &Context, dark: bool) {
        self.dark = dark;
        theme::set_dark(ctx, dark);
    }

    fn pick(&mut self, ctx: &Context, multiple: bool, purpose: Purpose) {
        if let Err(err) = platform::pick_pdfs(multiple, purpose, &self.inbox, ctx) {
            self.toasts.error(err);
        }
    }

    fn open_path(&mut self, path: &std::path::Path) {
        match platform::read_file(path) {
            Ok(file) => self.open_queue.push_back(file),
            Err(err) => {
                self.toasts.error(err);
                self.recent.retain(|recent| recent != path);
            }
        }
    }

    fn tab_index(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }

    fn goto_page(&mut self, page: usize) {
        self.with_tab(|tab| {
            tab.current_page = page;
            tab.scroll_to_page = Some(page);
        });
    }

    /// Toast an edit result.
    fn report(&mut self, result: Result<(), String>, success: Option<&str>) {
        match result {
            Ok(()) => {
                if let Some(text) = success {
                    self.toasts.success(text);
                }
            }
            Err(err) => self.toasts.error(err),
        }
    }

    fn add_tab(&mut self, file: PickedFile, derived: bool) {
        let mut session = Session::new(file.name, file.bytes);
        session.path = file.path.clone();
        let mut tab = DocTab::new(self.next_id, session);
        tab.derived = derived;
        self.next_id += 1;
        if let Some(err) = &tab.render_error {
            self.toasts
                .error(format!("Cannot display “{}”: {err}", tab.title()));
        }
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        self.home = false;
        if let Some(path) = file.path {
            self.remember_recent(path);
        }
        if let Some(cmd) = self.pending_cmd.take() {
            self.run_pending(cmd);
        }
    }

    /// Run a tool card picked on the welcome screen, now that a document is open.
    fn run_pending(&mut self, cmd: Cmd) {
        if let Some(tool) = cmd.comment_tool() {
            self.toggle_tool(tool);
            return;
        }
        // Organize-style commands from the welcome screen open the grid instead of acting blindly.
        match cmd {
            Cmd::Organize
            | Cmd::RotateLeft
            | Cmd::RotateAll
            | Cmd::DeletePages
            | Cmd::InsertBlank
            | Cmd::InsertFile => {
                self.with_tab(|tab| tab.mode = ViewMode::Organize);
            }
            Cmd::Find => {
                self.show_sidebar = true;
                self.sidebar_tab = sidebar::Tab::Search;
                self.focus_search = true;
            }
            _ => self.pending_dialog(cmd),
        }
    }

    fn pending_dialog(&mut self, cmd: Cmd) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let id = tab.id;
        match cmd {
            Cmd::Extract | Cmd::ExportPages => {
                let pages: Vec<usize> = (0..tab.page_count()).collect();
                self.dialog = Some(Dialog::Extract {
                    tab: id,
                    range: ranges::format(&pages),
                    error: None,
                });
            }
            Cmd::Split => {
                self.dialog = Some(Dialog::Split {
                    tab: id,
                    every: true,
                    n: 1,
                    starts: String::new(),
                    error: None,
                });
            }
            Cmd::Encrypt => {
                self.dialog = Some(Dialog::Encrypt {
                    tab: id,
                    user: String::new(),
                    owner: String::new(),
                    confirm: String::new(),
                    error: None,
                });
            }
            Cmd::RemovePassword => self
                .toasts
                .info("Opened without password: it will be saved unencrypted"),
            Cmd::Properties => self.open_properties(),
            Cmd::ExtractText => self.extract_text(),
            _ => {}
        }
    }

    fn remember_recent(&mut self, path: PathBuf) {
        self.recent.retain(|recent| recent != &path);
        self.recent.insert(0, path);
        self.recent.truncate(MAX_RECENT);
    }

    fn undo_redo(&mut self, undo: bool) {
        let tab = &mut self.tabs[self.active];
        let label = if undo {
            tab.session.undo_label()
        } else {
            tab.session.redo_label()
        }
        .map(str::to_owned);
        let done = if undo { tab.undo() } else { tab.redo() };
        match (done, label) {
            (true, Some(label)) => self
                .toasts
                .info(format!("{} {label}", if undo { "Undid" } else { "Redid" })),
            _ => self.toasts.info(if undo {
                "Nothing to undo"
            } else {
                "Nothing to redo"
            }),
        }
    }

    /// Returns true when written.
    /// Hand the document in front, as it is now, to the system to print.
    fn print(&mut self) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        match platform::print_pdf(&tab.session.name, tab.session.bytes()) {
            Ok(done) => self.toasts.success(done),
            Err(err) => self.toasts.error(err),
        }
    }

    fn save(&mut self, index: usize, save_as: bool) -> bool {
        let Some(tab) = self.tabs.get_mut(index) else {
            return false;
        };
        let bytes = match &tab.encrypt_on_save {
            Some((user, owner)) => {
                match guarded(|| core::encrypt(tab.session.bytes(), user, owner)) {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        self.toasts.error(format!("Encryption failed: {err}"));
                        return false;
                    }
                }
            }
            None => tab.session.bytes().to_vec(),
        };
        let path = if save_as {
            None
        } else {
            tab.session.path.clone()
        };
        match platform::save_pdf(&tab.session.name, path.as_deref(), &bytes) {
            Ok(Saved::To(path)) => {
                if let Some(name) = path.as_ref().and_then(|path| path.file_name()) {
                    tab.session.name = name.to_string_lossy().into_owned();
                }
                tab.session.path = path.clone().or(tab.session.path.take());
                tab.session.mark_saved();
                tab.saved_encryption = tab.encrypt_on_save.clone();
                tab.derived = false;
                let name = tab.title().to_owned();
                if let Some(path) = path {
                    self.remember_recent(path);
                }
                self.toasts.success(format!("Saved “{name}”"));
                true
            }
            Ok(Saved::Cancelled) => false,
            Err(err) => {
                self.toasts.error(err);
                false
            }
        }
    }

    fn request_close(&mut self, index: usize) {
        self.commit_text_box();
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        if tab.is_dirty() {
            self.dialog = Some(Dialog::ConfirmClose { tab: tab.id });
        } else {
            self.close_tab(index);
        }
    }

    fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        if self.active > index || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        if self.tabs.is_empty() {
            self.tool = Tool::Select;
            self.text_view = None;
        }
    }

    fn open_properties(&mut self) {
        let tab = &self.tabs[self.active];
        match core::read_metadata(tab.session.bytes()) {
            Ok(meta) => {
                let fields = [
                    meta.title,
                    meta.author,
                    meta.subject,
                    meta.keywords,
                    meta.creator,
                    meta.producer,
                ]
                .map(Option::unwrap_or_default);
                self.dialog = Some(Dialog::Properties {
                    tab: tab.id,
                    fields,
                });
            }
            Err(err) => self.toasts.error(format!("Cannot read properties: {err}")),
        }
    }

    fn extract_text(&mut self) {
        let tab = &self.tabs[self.active];
        let title = format!("Text - {}", tab.title());
        let bytes = tab.session.bytes().to_vec();
        self.jobs.spawn(&self.ctx, move || Done::Text {
            title,
            result: guarded(|| core::extract_all_text(&bytes)).map_err(|err| err.to_string()),
        });
    }

    /// Start the search the sidebar asked for, in the background.
    fn start_search(&mut self) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let Some(query) = tab.search.request.take() else {
            return;
        };
        tab.search.busy = true;
        let (id, generation) = (tab.id, tab.generation());
        let bytes = tab.session.bytes().to_vec();
        self.jobs.spawn(&self.ctx, move || Done::Search {
            tab: id,
            generation,
            result: guarded(|| core::search(&bytes, &query)).map_err(|err| err.to_string()),
            query,
        });
    }

    fn process_jobs(&mut self) {
        for done in self.jobs.drain() {
            match done {
                Done::Search {
                    tab,
                    generation,
                    query,
                    result,
                } => {
                    let Some(index) = self.tab_index(tab) else {
                        continue;
                    };
                    let tab = &mut self.tabs[index];
                    tab.search.busy = false;
                    if tab.generation() != generation {
                        continue; // The document changed meanwhile.
                    }
                    match result {
                        Ok(hits) => {
                            tab.search.hits = hits;
                            tab.search.error = None;
                        }
                        Err(err) => {
                            tab.search.hits.clear();
                            tab.search.error = Some(err);
                        }
                    }
                    tab.search.searched = query;
                }
                Done::Text { title, result } => match result {
                    Ok(pages) => {
                        let mut text = String::new();
                        for (page, page_text) in pages.iter().enumerate() {
                            let _ = writeln!(text, "──── Page {} ────", page + 1);
                            text.push_str(page_text.trim_end());
                            text.push_str("\n\n");
                        }
                        self.text_view = Some(TextView { title, text });
                    }
                    Err(err) => self.toasts.error(format!("Text extraction failed: {err}")),
                },
                Done::Split {
                    tab,
                    base,
                    to_disk,
                    every,
                    n,
                    starts,
                    result,
                } => {
                    let parts = match result {
                        Ok(parts) => parts,
                        Err(err) => {
                            if self.dialog.is_none() && self.tab_index(tab).is_some() {
                                self.dialog = Some(Dialog::Split {
                                    tab,
                                    every,
                                    n,
                                    starts,
                                    error: Some(err),
                                });
                            } else {
                                self.toasts.error(format!("Split failed: {err}"));
                            }
                            continue;
                        }
                    };
                    self.deliver_parts(&base, parts, to_disk);
                }
                Done::Comments {
                    tab,
                    generation,
                    result,
                } => self.comments.store(tab, generation, result),
                Done::Failed(err) => {
                    self.comments.drop_loading();
                    for tab in &mut self.tabs {
                        tab.search.busy = false;
                    }
                    self.toasts.error(err);
                }
            }
        }
    }

    /// Save split parts to a folder, or open each one in a new tab.
    fn deliver_parts(&mut self, base: &str, parts: Vec<Vec<u8>>, to_disk: bool) {
        if to_disk {
            match platform::save_parts(base, &parts) {
                Ok(Some(folder)) => self
                    .toasts
                    .success(format!("Saved {} files to {folder}", parts.len())),
                Ok(None) => {}
                Err(err) => self.toasts.error(err),
            }
            return;
        }
        let count = parts.len();
        for (number, bytes) in parts.into_iter().enumerate() {
            let name = platform::part_name(base, number + 1);
            self.add_tab(
                PickedFile {
                    name,
                    path: None,
                    bytes,
                },
                true,
            );
        }
        self.toasts.success(format!("Opened {count} parts"));
    }

    /// Open a decrypted file. A real password is kept for saving (the user can drop it
    /// with "Remove password"); an empty one means only usage restrictions, removed on save.
    fn add_unlocked(&mut self, file: PickedFile, bytes: Vec<u8>, password: String) {
        self.add_tab(PickedFile { bytes, ..file }, false);
        if password.is_empty() {
            self.toasts
                .info("This file has usage restrictions; they will be removed when you save.");
            return;
        }
        if let Some(tab) = self.tabs.last_mut() {
            let keep = Some((password.clone(), password));
            tab.encrypt_on_save = keep.clone();
            tab.saved_encryption = keep;
        }
        self.toasts
            .info("Unlocked. Saving keeps the password. Use “Remove password” to drop it.");
    }

    fn commit_gesture(&mut self, finished: viewer::Finished) {
        let Some(tool) = self.tool.comment() else {
            return;
        };
        let tab = &self.tabs[self.active];
        let (Some(&first), Some(&last)) = (finished.points.first(), finished.points.last()) else {
            return;
        };
        let Some(&size) = tab.page_sizes.get(finished.page) else {
            return;
        };
        let geom = match geometry::page_geometry(tab.session.bytes(), finished.page) {
            Ok(geom) => geom,
            Err(err) => {
                self.toasts.error(format!("Cannot place comment: {err}"));
                return;
            }
        };
        let style = self.styles.get(tool);
        let color = core_color(style.color);
        let tiny = (last - first).abs().min_elem() < 3.0;
        let rect = geometry::to_pdf_rect(&geom, size, first, last);
        let annotation = match tool {
            CommentTool::Highlight | CommentTool::Rectangle if tiny => return,
            CommentTool::Highlight => Annotation::Highlight { rect, color },
            CommentTool::Rectangle => Annotation::Square {
                rect,
                color,
                width: style.width,
            },
            CommentTool::Ink => {
                if finished.points.len() < 2 {
                    return;
                }
                let stroke = finished
                    .points
                    .iter()
                    .map(|p| geometry::to_pdf(&geom, size, *p))
                    .collect();
                Annotation::Ink {
                    strokes: vec![stroke],
                    color,
                    width: style.width,
                }
            }
            // Free text is typed in a text box, never through a gesture.
            CommentTool::FreeText => return,
            CommentTool::Note => {
                let rect = geometry::to_pdf_rect(&geom, size, first, first + vec2(24.0, 24.0));
                self.dialog = Some(Dialog::CommentText {
                    tab: tab.id,
                    page: finished.page,
                    rect,
                    text: String::new(),
                });
                return;
            }
        };
        self.add_annotation(self.active, finished.page, annotation);
    }

    fn add_annotation(&mut self, index: usize, page: usize, annotation: Annotation) {
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        let label = match &annotation {
            Annotation::Highlight { .. } => "Highlight",
            Annotation::Note { .. } => "Sticky note",
            Annotation::FreeText { .. } => "Free text",
            Annotation::Square { .. } => "Rectangle",
            Annotation::Ink { .. } => "Ink",
        };
        let result = tab.apply(label, |pdf| core::add_annotation(pdf, page, &annotation));
        self.report(result, None);
    }

    /// Move `pages` one place earlier (`delta` < 0) or later, keeping them selected.
    fn move_pages(&mut self, pages: &[usize], delta: isize) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let order = organize::move_selection(tab.page_count(), pages, delta);
        if order.iter().enumerate().all(|(index, &page)| index == page) {
            return;
        }
        let moved: Vec<usize> = order
            .iter()
            .enumerate()
            .filter(|(_, page)| pages.contains(page))
            .map(|(index, _)| index)
            .collect();
        let label = if delta < 0 {
            "Move pages earlier"
        } else {
            "Move pages later"
        };
        let result = tab.apply(label, |pdf| core::reorder_pages(pdf, &order));
        if result.is_ok() {
            tab.selection = moved.iter().copied().collect();
            if let Some(&first) = moved.first()
                && tab.mode == ViewMode::Read
            {
                tab.current_page = first;
                tab.scroll_to_page = Some(first);
            }
        }
        self.report(result, None);
    }

    /// Commit the text box when its tab is no longer shown in the reader.
    fn sync_text_box(&mut self) {
        let Some(text_box) = &self.text_box else {
            return;
        };
        let shown = self
            .tabs
            .get(self.active)
            .is_some_and(|tab| tab.id == text_box.tab && tab.mode == ViewMode::Read);
        if !shown {
            self.commit_text_box();
        }
    }

    /// Open a text box at `point`: on the FreeText annotation there (edit it), else a new
    /// empty one when `new_allowed`.
    fn open_text_at(&mut self, page: usize, point: egui::Pos2, new_allowed: bool) {
        self.commit_text_box();
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let Some(&size) = tab.page_sizes.get(page) else {
            return;
        };
        let geom = match geometry::page_geometry(tab.session.bytes(), page) {
            Ok(geom) => geom,
            Err(err) => {
                self.toasts.error(format!("Cannot place text: {err}"));
                return;
            }
        };
        let (x, y) = geometry::to_pdf(&geom, size, point);
        let hit = match tab.annotations(page) {
            // Topmost (last drawn) first.
            Ok(list) => list
                .iter()
                .rev()
                .find_map(|placed| match &placed.annotation {
                    Some(annotation @ Annotation::FreeText { rect, .. })
                        if (rect.x0..=rect.x1).contains(&x) && (rect.y0..=rect.y1).contains(&y) =>
                    {
                        Some((placed.id.clone(), annotation.clone()))
                    }
                    _ => None,
                }),
            Err(err) => {
                if !new_allowed {
                    self.toasts.error(format!("Cannot read annotations: {err}"));
                }
                None
            }
        };
        let tab_id = tab.id;
        match hit {
            Some((id, original)) => {
                let Annotation::FreeText {
                    rect,
                    text,
                    font_size,
                    color,
                    rotation,
                } = &original
                else {
                    return;
                };
                let anchor = geometry::from_pdf_rect(&geom, size, rect).min;
                let width = core::free_text_width(*rect, *rotation);
                let text = text.clone();
                self.styles.text.font_size = *font_size;
                self.styles.text.color = egui_color(*color);
                let result = tab.apply("Edit text", |pdf| core::remove_annotation(pdf, page, &id));
                if let Err(err) = result {
                    self.toasts.error(format!("Cannot edit this text: {err}"));
                    return;
                }
                let place = Placement {
                    page,
                    anchor,
                    width: Some(width),
                    geom,
                    displayed: size,
                };
                self.text_box = Some(TextBox::new(tab_id, place, text, Some(original)));
            }
            None if new_allowed => {
                // Put the middle of the first line under the click.
                let font_size = self.styles.text.font_size;
                let offset = vec2(
                    core::FREE_TEXT_PADDING,
                    core::FREE_TEXT_PADDING + font_size * 0.6,
                );
                let anchor = (point - offset).max(egui::Pos2::ZERO);
                let place = Placement {
                    page,
                    anchor,
                    width: None,
                    geom,
                    displayed: size,
                };
                self.text_box = Some(TextBox::new(tab_id, place, String::new(), None));
            }
            None => {}
        }
    }

    /// Write the text box into the document (an empty box is dropped).
    fn commit_text_box(&mut self) {
        let Some(text_box) = self.text_box.take() else {
            return;
        };
        // An emptied existing box stays removed: its "Edit text" step already did that.
        if text_box.text.trim().is_empty() {
            return;
        }
        let Some(index) = self.tab_index(text_box.tab) else {
            return;
        };
        let style = &self.styles.text;
        let tab = &mut self.tabs[index];
        let annotation = Annotation::FreeText {
            // The box exactly as edited: same corner, width and line breaks.
            rect: text_box.pdf_rect(style.font_size),
            text: text_box.text.trim_end().to_owned(),
            font_size: style.font_size,
            color: core_color(style.color),
            rotation: text_box.rotation(),
        };
        let add = |pdf: &[u8]| core::add_annotation(pdf, text_box.page, &annotation);
        let result = if text_box.original.is_some() {
            tab.amend("Edit text", add)
        } else {
            tab.apply("Add text", add)
        };
        self.report(result, None);
    }

    /// Esc: restore an edited annotation as it was; a new box is kept if it has text.
    fn cancel_text_box(&mut self) {
        let Some(original) = self.text_box.as_mut().and_then(|tb| tb.original.take()) else {
            self.commit_text_box();
            return;
        };
        let Some(text_box) = self.text_box.take() else {
            return;
        };
        let Some(index) = self.tab_index(text_box.tab) else {
            return;
        };
        let result = self.tabs[index].amend("Edit text", |pdf| {
            core::add_annotation(pdf, text_box.page, &original)
        });
        self.report(result, None);
    }
}

pub fn egui_color(color: core::Color) -> Color32 {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(channel(color.r), channel(color.g), channel(color.b))
}

fn core_color(color: Color32) -> core::Color {
    core::Color {
        r: f32::from(color.r()) / 255.0,
        g: f32::from(color.g()) / 255.0,
        b: f32::from(color.b()) / 255.0,
    }
}

fn stem(name: &str) -> &str {
    name.strip_suffix(".pdf")
        .or_else(|| name.strip_suffix(".PDF"))
        .unwrap_or(name)
}

// ---------------------------------------------------------------- dialog results

impl GodApp {
    fn finish_dialog(&mut self, ctx: &Context, dialog: Dialog, primary: bool) {
        match dialog {
            Dialog::Password { file, password, .. } => {
                match guarded(|| core::decrypt(&file.bytes, &password)) {
                    Ok(bytes) => self.add_unlocked(file, bytes, password),
                    Err(core::Error::BadPassword) => {
                        self.dialog = Some(Dialog::Password {
                            file,
                            password: String::new(),
                            error: Some("Wrong password".to_owned()),
                        });
                    }
                    Err(err) => self
                        .toasts
                        .error(format!("Cannot decrypt “{}”: {err}", file.name)),
                }
            }
            Dialog::ConfirmClose { tab } => {
                let Some(index) = self.tab_index(tab) else {
                    return;
                };
                if !primary || self.save(index, false) {
                    self.close_tab(index);
                }
            }
            Dialog::ConfirmQuit => {
                if !primary {
                    self.quit_confirmed = true;
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
            Dialog::Merge { files } => {
                if !primary {
                    self.dialog = Some(Dialog::Merge { files });
                    self.pick(ctx, true, Purpose::MergeAdd);
                    return;
                }
                let inputs: Vec<&[u8]> = files.iter().map(|file| file.bytes.as_slice()).collect();
                match guarded(|| core::merge(&inputs)) {
                    Ok(bytes) => {
                        self.add_tab(
                            PickedFile {
                                name: "merged.pdf".to_owned(),
                                path: None,
                                bytes,
                            },
                            true,
                        );
                        self.toasts.success(format!("Merged {} files", files.len()));
                    }
                    Err(err) => {
                        self.toasts.error(format!("Merge failed: {err}"));
                        self.dialog = Some(Dialog::Merge { files });
                    }
                }
            }
            Dialog::Extract { tab, range, .. } => {
                let Some(index) = self.tab_index(tab) else {
                    return;
                };
                let doc = &self.tabs[index];
                let result = ranges::parse(&range, doc.page_count()).and_then(|pages| {
                    guarded(|| core::extract_pages(doc.session.bytes(), &pages))
                        .map_err(|err| err.to_string())
                });
                let bytes = match result {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        self.dialog = Some(Dialog::Extract {
                            tab,
                            range,
                            error: Some(err),
                        });
                        return;
                    }
                };
                let mut name = format!("{} (pages {range}).pdf", stem(doc.title()));
                if name.chars().count() > 80 {
                    name = format!("{} (selection).pdf", stem(doc.title()));
                }
                if primary {
                    self.add_tab(
                        PickedFile {
                            name,
                            path: None,
                            bytes,
                        },
                        true,
                    );
                } else {
                    match platform::save_pdf(&name, None, &bytes) {
                        Ok(Saved::To(_)) => self.toasts.success("Pages exported"),
                        Ok(Saved::Cancelled) => {}
                        Err(err) => self.toasts.error(err),
                    }
                }
            }
            Dialog::Split {
                tab,
                every,
                n,
                starts,
                ..
            } => {
                let Some(index) = self.tab_index(tab) else {
                    return;
                };
                let doc = &self.tabs[index];
                let starts_pages = if every {
                    Ok(Vec::new())
                } else {
                    ranges::parse(&starts, doc.page_count())
                };
                let pages = match starts_pages {
                    Ok(pages) => pages,
                    Err(err) => {
                        self.dialog = Some(Dialog::Split {
                            tab,
                            every,
                            n,
                            starts,
                            error: Some(err),
                        });
                        return;
                    }
                };
                let base = doc.title().to_owned();
                let bytes = doc.session.bytes().to_vec();
                self.jobs.spawn(ctx, move || {
                    let result = guarded(|| {
                        if every {
                            core::split_every(&bytes, n)
                        } else {
                            core::split_at(&bytes, &pages)
                        }
                    })
                    .map_err(|err| err.to_string());
                    Done::Split {
                        tab,
                        base,
                        to_disk: primary,
                        every,
                        n,
                        starts,
                        result,
                    }
                });
            }
            Dialog::Encrypt {
                tab,
                user,
                owner,
                confirm,
                ..
            } => {
                let error = if user.is_empty() {
                    Some("Enter a password")
                } else if user != confirm {
                    Some("Passwords do not match")
                } else {
                    None
                };
                if let Some(error) = error {
                    self.dialog = Some(Dialog::Encrypt {
                        tab,
                        user,
                        owner,
                        confirm,
                        error: Some(error.to_owned()),
                    });
                    return;
                }
                if let Some(index) = self.tab_index(tab) {
                    self.tabs[index].encrypt_on_save = Some((user, owner));
                    self.toasts
                        .success("Password set: the file is encrypted when you save");
                }
            }
            Dialog::Properties { tab, fields } => {
                let Some(index) = self.tab_index(tab) else {
                    return;
                };
                let [title, author, subject, keywords, creator, producer] = fields
                    .map(|value| Some(value.trim().to_owned()).filter(|value| !value.is_empty()));
                let meta = Metadata {
                    title,
                    author,
                    subject,
                    keywords,
                    creator,
                    producer,
                };
                let result = self.tabs[index]
                    .apply("Edit properties", |pdf| core::write_metadata(pdf, &meta));
                self.report(result, Some("Properties updated"));
            }
            Dialog::CommentText {
                tab,
                page,
                rect,
                text,
            } => {
                let Some(index) = self.tab_index(tab) else {
                    return;
                };
                if text.trim().is_empty() {
                    return;
                }
                self.add_annotation(index, page, Annotation::Note { rect, text });
            }
            Dialog::EditText {
                tab,
                page,
                area,
                text,
                ..
            } => {
                let Some(index) = self.tab_index(tab).filter(|_| primary) else {
                    return;
                };
                let result = self.tabs[index].apply("Edit text", |pdf| {
                    core::replace_text(pdf, page, area, &text)
                });
                // Not changed: the box stays open and says why.
                if let Err(err) = result {
                    self.dialog = Some(Dialog::EditText {
                        tab,
                        page,
                        area,
                        text,
                        error: Some(err),
                    });
                }
            }
            Dialog::GoToPage {
                tab, page, count, ..
            } => {
                if !primary {
                    return;
                }
                // A number of this document, or the box stays open saying so.
                match page.trim().parse::<usize>() {
                    Ok(number) if (1..=count).contains(&number) => {
                        if let Some(index) = self.tab_index(tab) {
                            self.active = index;
                            self.with_tab(|tab| tab.mode = ViewMode::Read);
                            self.goto_page(number - 1);
                        }
                    }
                    _ => {
                        self.dialog = Some(Dialog::GoToPage {
                            tab,
                            page,
                            count,
                            error: Some(format!("Type a number from 1 to {count}")),
                        });
                    }
                }
            }
            Dialog::About | Dialog::Updates | Dialog::Preferences => {}
        }
    }
}

/// Style of the selected annotation, for the style card of the quick bar.
fn selected_style(tab: &mut DocTab) -> Option<(StyleKind, CommentStyle)> {
    let selected = tab.canvas.selected.clone()?;
    let annotation = tab
        .annotations(selected.page)
        .ok()?
        .iter()
        .find(|placed| placed.id == selected.id)?
        .annotation
        .clone()?;
    let style = |color: core::Color, width: f32, font_size: f32| CommentStyle {
        color: egui_color(color),
        width,
        font_size,
    };
    match annotation {
        Annotation::FreeText {
            font_size, color, ..
        } => Some((StyleKind::Text, style(color, 2.0, font_size))),
        Annotation::Square { color, width, .. } | Annotation::Ink { color, width, .. } => {
            Some((StyleKind::Stroke, style(color, width, 14.0)))
        }
        Annotation::Highlight { color, .. } => Some((StyleKind::Fill, style(color, 2.0, 14.0))),
        Annotation::Note { .. } => None,
    }
}
