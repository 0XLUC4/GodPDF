//! Root view: tool bar, document tabs, viewer, welcome screen, toasts; app-wide actions.

use std::path::PathBuf;

use godpdf_core::Session;
use gpui::prelude::*;
use gpui::{
    App, Context, Entity, ExternalPaths, FocusHandle, KeyBinding, PromptLevel, SharedString,
    Window, actions, div, px,
};

use crate::doc::{DocTab, guarded};
use crate::geometry::ZOOM_STEP;
use crate::theme::{self, Palette};
use crate::widgets::{action_button, primary_button, separator};
use crate::{dialogs, files, panels, toasts, tools};

actions!(
    godpdf,
    [
        Open,
        Save,
        SaveAs,
        CloseTab,
        NextTab,
        PreviousTab,
        Undo,
        Redo,
        ZoomIn,
        ZoomOut,
        FitWidth,
        ToggleTheme,
        Quit
    ]
);

/// Key context of the root view; bindings with `None` context apply everywhere.
const CONTEXT: &str = "GodPdf";

/// Shortcuts of the egui app (`crates/app/src/commands.rs`). `secondary!` = Ctrl (Cmd on macOS).
pub fn register(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(secondary!("o"), Open, None),
        KeyBinding::new(secondary!("s"), Save, None),
        KeyBinding::new(secondary!("shift-s"), SaveAs, None),
        KeyBinding::new(secondary!("w"), CloseTab, None),
        KeyBinding::new("ctrl-tab", NextTab, None),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, None),
        KeyBinding::new(secondary!("z"), Undo, None),
        KeyBinding::new(secondary!("y"), Redo, None),
        KeyBinding::new(secondary!("shift-z"), Redo, None),
        KeyBinding::new(secondary!("="), ZoomIn, None),
        KeyBinding::new(secondary!("+"), ZoomIn, None),
        KeyBinding::new(secondary!("-"), ZoomOut, None),
        KeyBinding::new(secondary!("0"), FitWidth, None),
        KeyBinding::new(secondary!("q"), Quit, None),
    ]);
    tools::register(cx);
    panels::register(cx);
    dialogs::register(cx);
}

pub struct GodPdfApp {
    pub tabs: Vec<Entity<DocTab>>,
    pub active: usize,
    focus: FocusHandle,
    /// Window title last set, to call the platform only on change.
    title: String,
}

impl GodPdfApp {
    pub fn new(paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        cx.observe_global::<toasts::Toasts>(|_, cx| cx.notify())
            .detach();
        cx.observe_global::<theme::Theme>(|_, cx| cx.notify())
            .detach();
        let mut app = Self {
            tabs: Vec::new(),
            active: 0,
            focus,
            title: String::new(),
        };
        for path in paths {
            app.open_path(&path, cx);
        }
        app
    }

    pub fn active_tab(&self) -> Option<&Entity<DocTab>> {
        self.tabs.get(self.active)
    }

    pub fn open_path(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        let already = self
            .tabs
            .iter()
            .position(|tab| tab.read(cx).session.path.as_deref() == Some(path));
        if let Some(index) = already {
            self.active = index;
            cx.notify();
            return;
        }
        match files::read_file(path) {
            Ok(file) => self.open_bytes(file.name, Some(file.path), file.bytes, cx),
            Err(err) => toasts::error(cx, err),
        }
    }

    /// Open a document in a new tab. Owner-password-only files open (saved unencrypted);
    /// files needing a password to read are refused for now.
    pub fn open_bytes(
        &mut self,
        name: String,
        path: Option<PathBuf>,
        bytes: Vec<u8>,
        cx: &mut Context<Self>,
    ) {
        let bytes = if godpdf_core::is_encrypted(&bytes) {
            match guarded(|| godpdf_core::decrypt(&bytes, "")) {
                Ok(bytes) => {
                    toasts::info(cx, "Opened without password: it will be saved unencrypted");
                    bytes
                }
                Err(_) => {
                    toasts::error(
                        cx,
                        format!("“{name}” is password-protected: not supported yet"),
                    );
                    return;
                }
            }
        } else {
            bytes
        };
        let mut session = Session::new(name, bytes);
        session.path = path;
        let tab = cx.new(|_| DocTab::new(session));
        if let Some(err) = tab.read(cx).render_error.clone() {
            toasts::error(cx, format!("Cannot display this PDF: {err}"));
        }
        // Tool bar shows the zoom and page of the active tab: redraw when it changes.
        cx.observe(&tab, |_, _, cx| cx.notify()).detach();
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        cx.notify();
    }

    fn open(&mut self, _: &Open, _: &mut Window, cx: &mut Context<Self>) {
        for path in files::pick_pdfs() {
            self.open_path(&path, cx);
        }
    }

    /// Returns true when written.
    pub fn save_tab(&mut self, index: usize, save_as: bool, cx: &mut Context<Self>) -> bool {
        let Some(tab) = self.tabs.get(index).cloned() else {
            return false;
        };
        let saved = tab.update(cx, |tab, cx| {
            let path = if save_as {
                None
            } else {
                tab.session.path.clone()
            };
            // A password set in the Protect dialog encrypts what is written, not the session.
            let bytes = dialogs::bytes_to_save(cx.entity_id(), tab.session.bytes(), cx)?;
            let result = files::save_pdf(&tab.session.name, path.as_deref(), &bytes);
            if let Ok(Some(path)) = &result {
                if let Some(name) = path.file_name() {
                    tab.session.name = name.to_string_lossy().into_owned();
                }
                tab.session.path = Some(path.clone());
                tab.session.mark_saved();
                cx.notify();
            }
            result.map(|path| path.map(|_| tab.title().to_owned()))
        });
        match saved {
            Ok(Some(name)) => {
                toasts::success(cx, format!("Saved “{name}”"));
                true
            }
            Ok(None) => false,
            Err(err) => {
                toasts::error(cx, err);
                false
            }
        }
    }

    fn save(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        self.save_tab(self.active, false, cx);
    }

    fn save_as(&mut self, _: &SaveAs, _: &mut Window, cx: &mut Context<Self>) {
        self.save_tab(self.active, true, cx);
    }

    fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        self.request_close(self.active, window, cx);
    }

    /// Close tab `index`, asking first when it has unsaved changes.
    pub fn request_close(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        if !tab.read(cx).is_dirty() {
            self.remove_tab(index, cx);
            return;
        }
        let message = format!("Save changes to “{}”?", tab.read(cx).title());
        let answer = window.prompt(
            PromptLevel::Warning,
            &message,
            None,
            &["Save", "Don't save", "Cancel"],
            cx,
        );
        let tab = tab.downgrade();
        cx.spawn(async move |this, cx| {
            let Ok(choice) = answer.await else { return };
            let _ = this.update(cx, |app, cx| {
                // The tabs may have moved while the prompt was open.
                let Some(index) = app.tabs.iter().position(|open| open.downgrade() == tab) else {
                    return;
                };
                let close = match choice {
                    0 => app.save_tab(index, false, cx),
                    1 => true,
                    _ => false,
                };
                if close {
                    app.remove_tab(index, cx);
                }
            });
        })
        .detach();
    }

    fn remove_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.tabs.len() {
            self.tabs.remove(index);
            if self.active >= index && self.active > 0 {
                self.active -= 1;
            }
            cx.notify();
        }
    }

    /// Window close / quit: true closes now; with unsaved changes, asks and quits on "Discard".
    pub fn confirm_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let dirty = self
            .tabs
            .iter()
            .filter(|tab| tab.read(cx).is_dirty())
            .count();
        if dirty == 0 {
            return true;
        }
        let message = format!("{dirty} document(s) have unsaved changes. Quit anyway?");
        let answer = window.prompt(
            PromptLevel::Warning,
            &message,
            None,
            &["Discard and quit", "Cancel"],
            cx,
        );
        cx.spawn(async move |_, cx| {
            if let Ok(0) = answer.await {
                cx.update(|cx| cx.quit());
            }
        })
        .detach();
        false
    }

    fn quit(&mut self, _: &Quit, window: &mut Window, cx: &mut Context<Self>) {
        if self.confirm_quit(window, cx) {
            cx.quit();
        }
    }

    fn cycle_tab(&mut self, forward: bool, cx: &mut Context<Self>) {
        let count = self.tabs.len();
        if count > 1 {
            self.active = if forward {
                (self.active + 1) % count
            } else {
                (self.active + count - 1) % count
            };
            cx.notify();
        }
    }

    /// Run `update` on the active tab, if any.
    fn with_tab<R>(
        &mut self,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut DocTab, &mut Context<DocTab>) -> R,
    ) -> Option<R> {
        let tab = self.active_tab()?.clone();
        Some(tab.update(cx, |tab, cx| {
            let result = update(tab, cx);
            cx.notify();
            result
        }))
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        match self.with_tab(cx, |tab, _| tab.undo()) {
            Some(Some(label)) => toasts::info(cx, format!("Undone: {label}")),
            Some(None) => toasts::info(cx, "Nothing to undo"),
            None => {}
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        match self.with_tab(cx, |tab, _| tab.redo()) {
            Some(Some(label)) => toasts::info(cx, format!("Redone: {label}")),
            Some(None) => toasts::info(cx, "Nothing to redo"),
            None => {}
        }
    }

    fn render_toolbar(&self, palette: &Palette, cx: &App) -> impl IntoElement {
        let tab = self.active_tab().map(|tab| tab.read(cx));
        let doc_buttons = tab.map(|tab| {
            let page = if tab.page_count() == 0 {
                "No pages".to_owned()
            } else {
                format!("Page {} / {}", tab.view.current_page + 1, tab.page_count())
            };
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .child(action_button("save", "Save", Save, palette))
                .child(action_button("save-as", "Save as", SaveAs, palette))
                .child(separator(palette))
                .child(action_button("undo", "Undo", Undo, palette))
                .child(action_button("redo", "Redo", Redo, palette))
                .child(separator(palette))
                .child(action_button("zoom-out", "−", ZoomOut, palette))
                .child(
                    div()
                        .w(px(52.))
                        .text_center()
                        .text_sm()
                        .child(format!("{:.0}%", tab.view.zoom * 100.0)),
                )
                .child(action_button("zoom-in", "+", ZoomIn, palette))
                .child(action_button("fit-width", "Fit width", FitWidth, palette))
                .child(separator(palette))
                .child(
                    div()
                        .px(px(8.))
                        .text_sm()
                        .text_color(palette.text_dim)
                        .child(page),
                )
        });
        let dark = cx
            .try_global::<theme::Theme>()
            .is_some_and(|theme| theme.dark);
        div()
            .flex()
            .items_center()
            .gap(px(2.))
            .h(px(44.))
            .px(px(10.))
            .bg(palette.panel)
            .border_b_1()
            .border_color(palette.border)
            .child(
                div()
                    .px(px(8.))
                    .font_weight(gpui::FontWeight::BOLD)
                    .child(crate::APP_NAME),
            )
            .child(action_button("open", "Open", Open, palette))
            .child(dialogs::menus(palette))
            .children(doc_buttons)
            .child(div().flex_1())
            .child(action_button(
                "theme",
                if dark { "Light" } else { "Dark" },
                ToggleTheme,
                palette,
            ))
    }

    fn render_tabs(&self, palette: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *palette;
        div()
            .id("tabs")
            .flex()
            .h(px(34.))
            .px(px(8.))
            .gap(px(4.))
            .items_end()
            .bg(palette.surface)
            .border_b_1()
            .border_color(palette.border)
            .overflow_x_scroll()
            .children(self.tabs.iter().enumerate().map(|(index, tab)| {
                let tab = tab.read(cx);
                let active = index == self.active;
                let title: SharedString = if tab.is_dirty() {
                    format!("● {}", tab.title()).into()
                } else {
                    tab.title().to_owned().into()
                };
                div()
                    .id(("tab", index))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(28.))
                    .pl(px(12.))
                    .pr(px(4.))
                    .max_w(px(220.))
                    .rounded_t(px(7.))
                    .text_sm()
                    .cursor_pointer()
                    .when(active, |chip| chip.bg(palette.bg).text_color(palette.text))
                    .when(!active, |chip| {
                        chip.text_color(palette.text_dim)
                            .hover(move |style| style.bg(palette.hover))
                    })
                    .on_click(cx.listener(move |app, _, _, cx| {
                        app.active = index;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(title),
                    )
                    .child(
                        div()
                            .id(("close-tab", index))
                            .px(px(6.))
                            .rounded(px(5.))
                            .hover(move |style| style.bg(palette.hover))
                            .child("×")
                            .on_click(cx.listener(move |app, _, window, cx| {
                                cx.stop_propagation();
                                app.request_close(index, window, cx);
                            })),
                    )
            }))
    }

    fn render_welcome(&self, palette: &Palette) -> impl IntoElement {
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(14.))
            .bg(palette.bg)
            .child(
                div()
                    .text_size(px(40.))
                    .font_weight(gpui::FontWeight::BOLD)
                    .child(crate::APP_NAME),
            )
            .child(
                div()
                    .text_size(px(17.))
                    .text_color(palette.text_dim)
                    .child("Read, comment and edit PDFs."),
            )
            .child(
                primary_button("welcome-open", "Open PDF", palette).on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(Open), cx);
                }),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(palette.text_dim)
                    .child("or drop PDF files here"),
            )
    }
}

impl Render for GodPdfApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::of(cx);
        let title = match self.active_tab() {
            Some(tab) => format!("{} - {}", tab.read(cx).title(), crate::APP_NAME),
            None => crate::APP_NAME.to_owned(),
        };
        if title != self.title {
            window.set_window_title(&title);
            self.title = title;
        }

        let active = self.active_tab().cloned();
        let body = match &active {
            Some(tab) => div()
                .flex_1()
                .min_w_0()
                .child(tab.clone())
                .into_any_element(),
            None => self.render_welcome(&palette).into_any_element(),
        };
        let hover = palette.hover;
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(palette.bg)
            .text_color(palette.text)
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::open))
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::save_as))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::quit))
            .on_action(cx.listener(|app, _: &NextTab, _, cx| app.cycle_tab(true, cx)))
            .on_action(cx.listener(|app, _: &PreviousTab, _, cx| app.cycle_tab(false, cx)))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(|app, _: &ZoomIn, _, cx| {
                app.with_tab(cx, |tab, _| tab.set_zoom(tab.view.zoom * ZOOM_STEP, None));
            }))
            .on_action(cx.listener(|app, _: &ZoomOut, _, cx| {
                app.with_tab(cx, |tab, _| tab.set_zoom(tab.view.zoom / ZOOM_STEP, None));
            }))
            .on_action(cx.listener(|app, _: &FitWidth, _, cx| {
                app.with_tab(cx, |tab, _| tab.fit_width());
            }))
            .on_action(|_: &ToggleTheme, _, cx| theme::toggle(cx))
            .on_drop(cx.listener(|app, paths: &ExternalPaths, _, cx| {
                for path in paths.paths() {
                    app.open_path(path, cx);
                }
            }))
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(hover))
            .child(self.render_toolbar(&palette, cx))
            .children(tools::toolbar(window, cx))
            .when(!self.tabs.is_empty(), |root| {
                root.child(self.render_tabs(&palette, cx))
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .children(panels::left(active.as_ref(), window, cx))
                    .child(body)
                    .children(panels::right(active.as_ref(), window, cx)),
            )
            .children(dialogs::overlay(
                cx.weak_entity(),
                active.as_ref(),
                window,
                cx,
            ))
            .children(toasts::render(cx))
    }
}
