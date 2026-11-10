//! Root view: the window chrome (`shell/`), dialogs, toasts; app-wide actions.

use std::path::PathBuf;

use godpdf_core::Session;
use gpui::prelude::*;
use gpui::{
    App, Context, Entity, ExternalPaths, FocusHandle, KeyBinding, PromptLevel, Window, actions,
    div, px,
};

use crate::doc::{DocTab, guarded};
use crate::geometry::ZOOM_STEP;
use crate::shell::{self, FitPage, Shell};
use crate::theme::{self, Palette};
use crate::{dialogs, files, fonts, panels, text, toasts, tools};

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
    shell::register(cx);
    tools::register(cx);
    crate::signature::register(cx);
    panels::register(cx);
    dialogs::register(cx);
    // After the tools: its Esc binding takes precedence over theirs.
    text::register(cx);
}

pub struct GodPdfApp {
    pub tabs: Vec<Entity<DocTab>>,
    pub active: usize,
    /// What the window chrome shows (mode, tools panel, home tab, banners).
    pub shell: Shell,
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
            shell: Shell::default(),
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
            Ok(file) => {
                self.shell.remember_recent(path);
                self.open_bytes(file.name, Some(file.path), file.bytes, cx)
            }
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
        self.shell.home = false;
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

    pub fn remove_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.tabs.len() {
            let tab = self.tabs.remove(index);
            if let Some(path) = tab.read(cx).session.path.clone() {
                self.shell.closed.retain(|closed| closed != &path);
                self.shell.closed.push(path);
            }
            // egui `close_tab`: the tab after the closed one comes in front.
            if self.active > index || self.active >= self.tabs.len() {
                self.active = self.active.saturating_sub(1);
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
        let hover = palette.hover;
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(palette.bg)
            .font_family(fonts::UI)
            .font_weight(fonts::UI_WEIGHT)
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
            .on_action(cx.listener(|app, _: &FitPage, _, cx| {
                app.with_tab(cx, |tab, _| tab.fit_page());
            }))
            .on_action(cx.listener(Self::toggle_tools_panel))
            .on_action(cx.listener(Self::reopen_tab))
            .on_action(|_: &ToggleTheme, _, cx| theme::toggle(cx))
            .on_drop(cx.listener(|app, paths: &ExternalPaths, _, cx| {
                for path in paths.paths() {
                    app.open_path(path, cx);
                }
            }))
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(hover))
            .text_size(px(fonts::BODY))
            .children(tools::toolbar(window, cx))
            .child(shell::render(self, active.as_ref(), window, cx))
            .children(dialogs::overlay(
                cx.weak_entity(),
                active.as_ref(),
                window,
                cx,
            ))
            .children(text::overlay(active.as_ref(), window, cx))
            .children(toasts::render(cx))
    }
}
