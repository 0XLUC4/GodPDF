//! Window chrome around the document: tab row, header (menu, mode tabs, search), left tools
//! panel, right icon rail + panel, info banner, and the welcome screen.

use egui::{
    Align, Align2, Button, Color32, CornerRadius, FontId, Frame, Id, Layout, Margin, Panel, Popup,
    Rect, Response, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};

use super::GodApp;
use crate::commands::Cmd;
use crate::doc::{Budget, ViewMode};
use crate::jobs::Done;
use crate::sidebar::{self, Tab as View};
use crate::theme::Palette;
use crate::tools::{self, Action, Mode};
use crate::widgets::{self, icon, with_icon};
use crate::{comments, welcome};

/// Height of the document tab row.
const TAB_HEIGHT: f32 = 38.0;
/// Height of the header row (menu, mode tabs, search).
const HEADER_HEIGHT: f32 = 46.0;
/// Longest document name shown in a tab before "…".
const TAB_TEXT_MAX: f32 = 180.0;

/// What the menu of a document tab asks for, done once the tabs are drawn.
enum TabAction {
    CloseOthers(usize),
    Reveal(std::path::PathBuf),
}

/// Thin message under the header; dismissed per document.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Banner {
    Encrypted,
    Unsaved,
}

impl GodApp {
    /// True when a document (not the home screen) fills the window.
    pub(super) fn doc_view(&self) -> bool {
        !self.tabs.is_empty() && !self.home
    }

    // ------------------------------------------------------------ tab row

    pub(super) fn tab_row(&mut self, ui: &mut Ui, clicked: &mut Vec<Cmd>) {
        let palette = Palette::of(ui);
        let mut select = None;
        let mut close = None;
        let mut action = None;
        let row = vec2(ui.available_width(), TAB_HEIGHT);
        ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_space(2.0);
            let home = tab_shape(ui, &palette, !self.doc_view(), vec2(40.0, TAB_HEIGHT));
            centered_glyph(ui, home.rect, icon::HOUSE, palette.text);
            if home.on_hover_text("Home").clicked() && !self.tabs.is_empty() {
                self.home = true;
            }

            // Share what is left between the tabs; names shrink before anything overflows.
            let reserved = 2.0 * widgets::ICON_BUTTON.x + 100.0;
            let chrome = 12.0 + 16.0 + 8.0 + 8.0 + widgets::SMALL_ICON_BUTTON.x + 22.0;
            let per_tab = (ui.available_width() - reserved) / self.tabs.len().max(1) as f32;
            let text_max = (per_tab - chrome).clamp(40.0, TAB_TEXT_MAX);
            for (index, tab) in self.tabs.iter().enumerate() {
                let active = self.doc_view() && index == self.active;
                let (response, close_it) =
                    doc_tab(ui, &palette, tab.title(), tab.is_dirty(), active, text_max);
                if response.clicked() {
                    select = Some(index);
                }
                if close_it || response.middle_clicked() {
                    close = Some(index);
                }
                response.context_menu(|ui| {
                    if ui.button(with_icon(icon::X, "Close")).clicked() {
                        close = Some(index);
                    }
                    let others = Button::new(with_icon(icon::X_CIRCLE, "Close the others"));
                    if ui.add_enabled(self.tabs.len() > 1, others).clicked() {
                        action = Some(TabAction::CloseOthers(index));
                    }
                    let reopen =
                        Button::new(with_icon(Cmd::ReopenTab.icon(), Cmd::ReopenTab.label()));
                    if ui.add_enabled(!self.closed.is_empty(), reopen).clicked() {
                        clicked.push(Cmd::ReopenTab);
                    }
                    let path = tab.session.path.as_ref();
                    if let Some(path) = path.filter(|_| !cfg!(target_arch = "wasm32")) {
                        ui.separator();
                        if ui
                            .button(with_icon(icon::FOLDER_OPEN, "Show in folder"))
                            .clicked()
                        {
                            action = Some(TabAction::Reveal(path.clone()));
                        }
                        if ui.button(with_icon(icon::COPY, "Copy path")).clicked() {
                            ui.ctx().copy_text(path.display().to_string());
                        }
                    }
                });
            }
            ui.add_space(6.0);
            let open = Button::new(RichText::new(with_icon(icon::PLUS, "Open")).size(14.0))
                .frame_when_inactive(false)
                .stroke(egui::Stroke::NONE);
            if ui.add(open).on_hover_text("Open PDF (Ctrl+O)").clicked() {
                clicked.push(Cmd::Open);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (glyph, tip) = if self.dark {
                    (icon::SUN, "Light mode")
                } else {
                    (icon::MOON, "Dark mode")
                };
                if widgets::icon_button(ui, glyph, tip).clicked() {
                    clicked.push(Cmd::ToggleTheme);
                }
                if widgets::icon_button(ui, icon::QUESTION, "About GodPDF").clicked() {
                    clicked.push(Cmd::About);
                }
                let newest = self.updates.newest();
                let tip = newest.as_ref().map_or_else(
                    || "Updates".to_owned(),
                    |release| format!("Update available: {}", release.tag),
                );
                let response = widgets::icon_button(ui, icon::ARROW_CIRCLE_UP, &tip);
                if newest.is_some() {
                    let dot = response.rect.right_top() + vec2(-6.0, 6.0);
                    ui.painter()
                        .circle_filled(dot, 4.0, crate::dialogs::DOWNLOAD_BLUE);
                }
                if response.clicked() {
                    clicked.push(Cmd::Updates);
                }
            });
        });
        if let Some(index) = select {
            self.active = index;
            self.home = false;
        }
        if let Some(index) = close {
            self.request_close(index);
        }
        match action {
            Some(TabAction::CloseOthers(index)) => self.close_others(index),
            Some(TabAction::Reveal(path)) => {
                if let Err(err) = crate::platform::reveal(&path) {
                    self.toasts.error(err);
                }
            }
            None => {}
        }
    }

    // ------------------------------------------------------------ header

    pub(super) fn header_row(&mut self, ui: &mut Ui, clicked: &mut Vec<Cmd>) {
        let palette = Palette::of(ui);
        let doc_view = self.doc_view();
        if doc_view {
            self.sync_mode();
        }
        let row = vec2(ui.available_width(), HEADER_HEIGHT);
        ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
            let menu =
                Button::new(RichText::new(with_icon(icon::SIDEBAR_SIMPLE, "Menu")).size(14.0))
                    .frame_when_inactive(false)
                    .stroke(egui::Stroke::NONE)
                    .min_size(vec2(0.0, 32.0));
            let menu = ui.add(menu);
            Popup::menu(&menu).show(|ui| {
                ui.set_min_width(200.0);
                let mut shown = self.show_tools;
                let toggle =
                    ui.add_enabled(doc_view, egui::Checkbox::new(&mut shown, "Tools panel"));
                if toggle.changed() {
                    self.show_tools = shown;
                    if shown && self.mode == Mode::Read {
                        self.mode = Mode::AllTools;
                    }
                }
                ui.separator();
                self.menus(ui, clicked);
            });
            ui.add_space(14.0);
            ui.spacing_mut().item_spacing.x = 2.0;
            for mode in Mode::ALL {
                let enabled = doc_view || mode == Mode::AllTools;
                let active = if doc_view {
                    self.mode == mode
                } else {
                    mode == Mode::AllTools
                };
                if mode_tab(ui, &palette, mode.title(), active, enabled).clicked() {
                    self.set_mode(mode, clicked);
                }
            }
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let buttons = [
                    (icon::PRINTER, "Print… (Ctrl+P)", Cmd::Print),
                    (icon::EXPORT, "Save a copy… (Ctrl+Shift+S)", Cmd::SaveAs),
                    (icon::FLOPPY_DISK, "Save (Ctrl+S)", Cmd::Save),
                    (icon::INFO, "Document properties", Cmd::Properties),
                ];
                for (glyph, tip, cmd) in buttons {
                    let response = ui
                        .add_enabled_ui(doc_view, |ui| widgets::icon_button(ui, glyph, tip))
                        .inner;
                    if response.clicked() {
                        clicked.push(cmd);
                    }
                }
                ui.add_space(6.0);
                let width = (ui.available_width() - 40.0).clamp(140.0, 300.0);
                if search_field(ui, &palette, width).clicked() {
                    clicked.push(Cmd::Palette);
                }
                if self.jobs.busy() {
                    ui.spinner().on_hover_text("Working…");
                }
            });
        });
    }

    /// Organize mode follows the view: the grid can also be left from inside it.
    fn sync_mode(&mut self) {
        let organizing = self.tabs[self.active].mode == ViewMode::Organize;
        if organizing {
            self.mode = Mode::Organize;
        } else if self.mode == Mode::Organize {
            self.mode = Mode::AllTools;
        }
    }

    fn set_mode(&mut self, mode: Mode, clicked: &mut Vec<Cmd>) {
        if !self.doc_view() {
            self.home = self.tabs.is_empty();
            return;
        }
        self.mode = mode;
        if mode != Mode::Read {
            self.show_tools = true;
        }
        let organizing = self.tabs[self.active].mode == ViewMode::Organize;
        if (mode == Mode::Organize) != organizing {
            clicked.push(Cmd::Organize);
        }
        if mode == Mode::Comment {
            self.show_sidebar = true;
            self.sidebar_tab = View::Comments;
        }
    }

    // ------------------------------------------------------------ side panels

    /// Tools panel (left), icon rail + view panel (right), banner (top of the canvas).
    pub(super) fn doc_panels(&mut self, ui: &mut Ui, clicked: &mut Vec<Cmd>) {
        let palette = Palette::of(ui);
        if self.show_tools && self.mode != Mode::Read {
            let mode = self.mode;
            let expanded = &mut self.tools_expanded;
            let out = Panel::left("tools-panel")
                .resizable(true)
                .default_size(280.0)
                .size_range(220.0..=420.0)
                .frame(
                    Frame::new()
                        .fill(palette.bg)
                        .inner_margin(Margin::symmetric(12, 6)),
                )
                .show(ui, |ui| tools::panel(ui, mode, expanded))
                .inner;
            if out.close {
                self.show_tools = false;
            }
            match out.action {
                Some(Action::Run(cmd)) => clicked.push(cmd),
                Some(Action::Mode(mode)) => self.set_mode(mode, clicked),
                None => {}
            }
        }

        Panel::right("rail")
            .resizable(false)
            .exact_size(52.0)
            .frame(
                Frame::new()
                    .fill(palette.bg)
                    .inner_margin(Margin::symmetric(0, 10)),
            )
            .show(ui, |ui| self.rail(ui, clicked));

        if self.show_sidebar {
            self.right_panel(ui, &palette);
        }
        self.banner(ui, &palette, clicked);
    }

    fn rail(&mut self, ui: &mut Ui, clicked: &mut Vec<Cmd>) {
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            for view in View::ALL {
                let selected = self.show_sidebar && self.sidebar_tab == view;
                if widgets::rail_button(ui, view.icon(), view.title(), selected).clicked() {
                    self.show_sidebar = !selected;
                    self.sidebar_tab = view;
                    self.focus_search = view == View::Search && !selected;
                }
            }
        });
        let palette = Palette::of(ui);
        let tab = &self.tabs[self.active];
        let (current, count, zoom) = (tab.current_page, tab.page_count(), tab.zoom);
        let mut goto = None;
        ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let buttons = [
                (
                    icon::MAGNIFYING_GLASS_MINUS,
                    "Zoom out (Ctrl+-)",
                    Cmd::ZoomOut,
                ),
                (icon::MAGNIFYING_GLASS_PLUS, "Zoom in (Ctrl++)", Cmd::ZoomIn),
                (icon::FRAME_CORNERS, "Fit page", Cmd::FitPage),
                (
                    icon::ARROWS_OUT_LINE_HORIZONTAL,
                    "Fit width (Ctrl+0)",
                    Cmd::FitWidth,
                ),
            ];
            for (glyph, tip, cmd) in buttons {
                if widgets::rail_button(ui, glyph, tip, false).clicked() {
                    clicked.push(cmd);
                }
            }
            ui.label(
                RichText::new(format!("{:.0}%", zoom * 100.0))
                    .size(12.0)
                    .color(palette.text_dim),
            );
            ui.add_space(8.0);
            let next = ui
                .add_enabled_ui(current + 1 < count, |ui| {
                    widgets::rail_button(ui, icon::CARET_DOWN, "Next page", false)
                })
                .inner;
            if next.clicked() {
                goto = Some(current + 1);
            }
            let previous = ui
                .add_enabled_ui(current > 0, |ui| {
                    widgets::rail_button(ui, icon::CARET_UP, "Previous page", false)
                })
                .inner;
            if previous.clicked() {
                goto = Some(current - 1);
            }
            ui.label(
                RichText::new(format!("/ {count}"))
                    .size(12.0)
                    .color(palette.text_dim),
            );
            let mut page = current + 1;
            let field = egui::DragValue::new(&mut page).range(1..=count.max(1));
            if ui
                .add_sized(vec2(38.0, 26.0), field)
                .on_hover_text("Page number (drag or click to type)")
                .changed()
            {
                goto = Some(page - 1);
            }
        });
        if let Some(page) = goto {
            self.goto_page(page);
        }
    }

    fn right_panel(&mut self, ui: &mut Ui, palette: &Palette) {
        let view = self.sidebar_tab;
        if view == View::Comments {
            self.request_comments();
        }
        let tab = &self.tabs[self.active];
        let (id, generation) = (tab.id, tab.generation());
        let mut budget = Budget(3);
        let mut sign = None;
        let (out, close) = Panel::right("right-panel")
            .resizable(true)
            .default_size(320.0)
            .size_range(240.0..=520.0)
            .frame(
                Frame::new()
                    .fill(palette.bg)
                    .inner_margin(Margin::symmetric(14, 6)),
            )
            .show(ui, |ui| {
                let found = self.comments.get(id, generation);
                let mut close = false;
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(view.title()).size(17.0).strong());
                    if view == View::Comments
                        && let Some(count) = comments::count(found)
                    {
                        ui.label(
                            RichText::new(count.to_string())
                                .size(15.0)
                                .color(palette.text_dim),
                        );
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        close = widgets::icon_button(ui, icon::X, "Close panel").clicked();
                    });
                });
                ui.add_space(8.0);
                let tab = &mut self.tabs[self.active];
                let out = if view == View::Comments {
                    sidebar::Output {
                        jump: comments::show(ui, found, tab.current_page),
                        ..sidebar::Output::default()
                    }
                } else if view == View::Sign {
                    sign = crate::signature::panel(ui, self.signature.as_ref());
                    sidebar::Output::default()
                } else {
                    sidebar::show(ui, tab, view, &mut budget)
                };
                (out, close)
            })
            .inner;
        if close {
            self.show_sidebar = false;
        }
        if let Some(page) = out.jump {
            self.goto_page(page);
        }
        if let Some((page, delta)) = out.move_page {
            self.commit_text_box();
            self.move_pages(&[page], delta);
        }
        if let Some((label, items)) = out.outline {
            let result =
                self.tabs[self.active].apply(label, |pdf| godpdf_core::write_outline(pdf, &items));
            self.report(result, None);
        }
        if let Some(cmd) = out.run {
            let ctx = ui.ctx().clone();
            self.run(&ctx, cmd);
        }
        if let Some(changes) = out.fill {
            let result = self.tabs[self.active]
                .apply("Fill in form", |pdf| godpdf_core::fill_form(pdf, &changes));
            self.report(result, Some("Form filled in"));
        }
        if let Some(kind) = out.add_field {
            self.field_kind = kind;
            self.set_tool(crate::quickbar::Tool::Field);
            self.toasts.info("Drag the box of the field on the page");
        }
        match sign {
            Some(crate::signature::PanelAction::Draw) => {
                self.dialog = Some(crate::dialogs::Dialog::Signature {
                    strokes: Vec::new(),
                    place: None,
                    saved: false,
                });
            }
            Some(crate::signature::PanelAction::Place) => {
                self.set_tool(crate::quickbar::Tool::Sign);
                self.toasts.info("Click where your signature goes");
            }
            Some(crate::signature::PanelAction::Forget) => self.signature = None,
            None => {}
        }
        self.start_search();
        if std::mem::take(&mut self.focus_search) {
            ui.ctx()
                .memory_mut(|memory| memory.request_focus(Id::new(sidebar::SEARCH_ID)));
        }
    }

    /// Read the active document's comments in the background, once per revision.
    fn request_comments(&mut self) {
        let tabs = &self.tabs;
        self.comments
            .retain_tabs(|id| tabs.iter().any(|tab| tab.id == id));
        let tab = &self.tabs[self.active];
        let (id, generation) = (tab.id, tab.generation());
        if !self.comments.start(id, generation) {
            return;
        }
        let bytes = tab.session.bytes().to_vec();
        let pages = tab.page_count();
        self.jobs.spawn(&self.ctx, move || Done::Comments {
            tab: id,
            generation,
            result: comments::collect(&bytes, pages),
        });
    }

    fn banner(&mut self, ui: &mut Ui, palette: &Palette, clicked: &mut Vec<Cmd>) {
        let tab = &self.tabs[self.active];
        let kind = if tab.encrypt_on_save.is_some() {
            Banner::Encrypted
        } else if tab.is_dirty() {
            Banner::Unsaved
        } else {
            return;
        };
        let key = (tab.id, kind);
        if self.dismissed.contains(&key) {
            return;
        }
        let (glyph, text, action, cmd) = match kind {
            Banner::Encrypted => (
                icon::LOCK,
                "This document will be saved with a password.",
                "Remove password",
                Cmd::RemovePassword,
            ),
            Banner::Unsaved => (
                icon::INFO,
                "This document has unsaved changes.",
                "Save",
                Cmd::Save,
            ),
        };
        Panel::top("banner")
            .frame(
                Frame::new()
                    .fill(palette.surface)
                    .inner_margin(Margin::symmetric(16, 6)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(glyph).size(16.0));
                    ui.label(text);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::small_icon_button(ui, icon::X, "Dismiss").clicked() {
                            self.dismissed.push(key);
                        }
                        let button = Button::new(action)
                            .fill(palette.bg)
                            .stroke(Stroke::new(1.0, palette.text_dim))
                            .corner_radius(CornerRadius::same(14));
                        if ui.add(button).clicked() {
                            clicked.push(cmd);
                        }
                    });
                });
            });
    }

    // ------------------------------------------------------------ welcome

    pub(super) fn welcome_central(&mut self, ui: &mut Ui, clicked: &mut Vec<Cmd>) {
        let out = welcome::show(ui, self.logo.as_ref(), &self.recent);
        if let Some(path) = out.open_recent {
            self.open_path(&path);
        }
        if out.new_pdf {
            // One empty A4 page to write, draw and insert into.
            match godpdf_core::blank_document(595.0, 842.0) {
                Ok(bytes) => {
                    let file = crate::platform::PickedFile {
                        name: "Untitled.pdf".to_owned(),
                        path: None,
                        bytes,
                    };
                    self.add_tab(file, true);
                }
                Err(err) => self.toasts.error(err.to_string()),
            }
        }
        if let Some(cmd) = out.cmd {
            if cmd.needs_doc() {
                self.pending_cmd = Some(cmd);
                clicked.push(Cmd::Open);
            } else {
                clicked.push(cmd);
            }
        }
    }
}

// ---------------------------------------------------------------- painted pieces

/// Tab-shaped hit area: white with rounded top corners when active, joining the header.
fn tab_shape(ui: &mut Ui, palette: &Palette, active: bool, size: egui::Vec2) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let radius = CornerRadius {
        nw: 8,
        ne: 8,
        sw: 0,
        se: 0,
    };
    let fill = if active {
        palette.bg
    } else if response.hovered() {
        palette.hover
    } else {
        Color32::TRANSPARENT
    };
    // Active tabs reach the bottom edge so they merge with the header below.
    let shape = if active {
        rect.with_min_y(rect.min.y + 4.0)
    } else {
        rect.shrink2(vec2(0.0, 5.0))
    };
    let radius = if active {
        radius
    } else {
        CornerRadius::same(8)
    };
    ui.painter().rect_filled(shape, radius, fill);
    response
}

fn centered_glyph(ui: &Ui, rect: Rect, glyph: &str, color: Color32) {
    ui.painter().text(
        rect.center() + vec2(0.0, 2.0),
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(17.0),
        color,
    );
}

/// One document tab; returns it and whether its cross was clicked.
fn doc_tab(
    ui: &mut Ui,
    palette: &Palette,
    title: &str,
    dirty: bool,
    active: bool,
    text_max: f32,
) -> (Response, bool) {
    let text_color = if active {
        palette.text
    } else {
        palette.text_dim
    };
    let mut job = egui::text::LayoutJob::simple_singleline(
        title.to_owned(),
        FontId::proportional(14.0),
        text_color,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(text_max);
    let galley = ui.painter().layout_job(job);
    let dot = if dirty { 14.0 } else { 0.0 };
    let close_size = widgets::SMALL_ICON_BUTTON;
    let width = 12.0 + 16.0 + 8.0 + galley.size().x + dot + 8.0 + close_size.x + 8.0;
    let response = tab_shape(ui, palette, active, vec2(width, TAB_HEIGHT));
    let rect = response.rect;
    let mid = rect.center().y + 2.0;
    let painter = ui.painter();
    painter.text(
        pos2(rect.min.x + 20.0, mid),
        Align2::CENTER_CENTER,
        icon::FILE_PDF,
        FontId::proportional(16.0),
        text_color,
    );
    let text_x = rect.min.x + 36.0;
    painter.galley(
        pos2(text_x, mid - galley.size().y / 2.0),
        galley.clone(),
        text_color,
    );
    if dirty {
        // Unsaved changes: a plain asterisk after the name.
        painter.text(
            pos2(text_x + galley.size().x + 4.0, mid),
            Align2::LEFT_CENTER,
            "*",
            FontId::proportional(15.0),
            text_color,
        );
    }
    let close_rect =
        Rect::from_center_size(pos2(rect.max.x - 8.0 - close_size.x / 2.0, mid), close_size);
    let close = ui
        .put(close_rect, widgets::icon_widget(icon::X, 13.0))
        .on_hover_text("Close");
    let response = if dirty {
        response.on_hover_text(format!("{title} (unsaved changes)"))
    } else {
        response.on_hover_text(title)
    };
    (response, close.clicked())
}

/// Text tab of the header with an underline when active.
fn mode_tab(ui: &mut Ui, palette: &Palette, label: &str, active: bool, enabled: bool) -> Response {
    let color = if !enabled {
        palette.text_dim.gamma_multiply(0.5)
    } else if active {
        palette.text
    } else {
        palette.text_dim
    };
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), FontId::proportional(15.0), color);
    let size = vec2(galley.size().x + 22.0, HEADER_HEIGHT);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let painter = ui.painter();
    if enabled && !active && response.hovered() {
        painter.rect_filled(
            rect.shrink2(vec2(0.0, 8.0)),
            CornerRadius::same(6),
            palette.hover,
        );
    }
    painter.galley(rect.center() - galley.size() / 2.0, galley, color);
    if active {
        let line = Rect::from_min_max(
            pos2(rect.min.x + 11.0, rect.max.y - 2.0),
            pos2(rect.max.x - 11.0, rect.max.y),
        );
        painter.rect_filled(line, CornerRadius::same(1), palette.text);
    }
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// Pill that looks like a search box and opens the command palette.
fn search_field(ui: &mut Ui, palette: &Palette, width: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, 32.0), Sense::click());
    let border = if response.hovered() {
        palette.text_dim
    } else {
        palette.border
    };
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(16),
        palette.bg,
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );
    painter.text(
        pos2(rect.min.x + 18.0, rect.center().y),
        Align2::CENTER_CENTER,
        icon::MAGNIFYING_GLASS,
        FontId::proportional(15.0),
        palette.text_dim,
    );
    let hint = Cmd::Palette
        .shortcut()
        .map(|shortcut| ui.ctx().format_shortcut(&shortcut))
        .unwrap_or_default();
    let hint_width = if width > 200.0 {
        let galley = painter.layout_no_wrap(hint, FontId::proportional(12.0), palette.text_dim);
        let width = galley.size().x;
        painter.galley(
            pos2(
                rect.max.x - 14.0 - width,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            palette.text_dim,
        );
        width + 14.0
    } else {
        0.0
    };
    let mut job = egui::text::LayoutJob::simple_singleline(
        "Find tools and commands".to_owned(),
        FontId::proportional(14.0),
        palette.text_dim,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(width - 50.0 - hint_width);
    let galley = painter.layout_job(job);
    painter.galley(
        pos2(rect.min.x + 34.0, rect.center().y - galley.size().y / 2.0),
        galley,
        palette.text_dim,
    );
    response
        .on_hover_cursor(egui::CursorIcon::Text)
        .on_hover_text("Command palette (Ctrl+K)")
}
