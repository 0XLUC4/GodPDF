//! Window chrome around the document. See DESIGN.md for
//! every measurement. One file per region:
//!
//! - `tabrow.rs`: home button, document tabs, "+ Open", updates / about / theme.
//! - `header.rs`: Menu, mode tabs, search pill, properties / save / export / print.
//! - `leftpanel.rs`: the tools panel ("All tools", ... ) left of the canvas.
//! - `rail.rs`: right icon rail (views, page box, zoom). The panel it opens is `panels::right`.
//! - `banner.rs`: thin message over the canvas (unsaved / encrypted).
//! - `welcome.rs`: start screen (no document, or the home tab).
//!
//! Layout, top to bottom: tab row, header, then either the welcome screen or
//! `[tools panel][banner over canvas][right panel][rail]`.

mod banner;
mod header;
mod leftpanel;
mod rail;
mod tabrow;
pub mod welcome;

use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, EntityId, KeyBinding, Window, actions, div};

use crate::app::GodPdfApp;
use crate::doc::DocTab;
use crate::panels;
use crate::theme::Palette;

actions!(godpdf, [ToggleToolsPanel, FitPage, ReopenTab]);

/// Most recent files kept.
pub const MAX_RECENT: usize = 8;

/// Ctrl+Shift+T reopens the last closed tab.
pub fn register(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(secondary!("shift-t"), ReopenTab, None)]);
}

/// Height of the document tab row.
pub const TAB_HEIGHT: f32 = 38.0;
/// Height of the header row (menu, mode tabs, search).
pub const HEADER_HEIGHT: f32 = 46.0;

/// Header tab: decides what the left panel lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    AllTools,
    Read,
    Organize,
    Comment,
    Protect,
}

impl Mode {
    pub const ALL: [Mode; 5] = [
        Mode::AllTools,
        Mode::Read,
        Mode::Organize,
        Mode::Comment,
        Mode::Protect,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::AllTools => "All tools",
            Self::Read => "Read",
            Self::Organize => "Organize",
            Self::Comment => "Comment",
            Self::Protect => "Protect",
        }
    }

    pub fn panel_title(self) -> &'static str {
        match self {
            Self::Organize => "Organize pages",
            Self::Comment => "Add comments",
            Self::Protect => "Protect a PDF",
            Self::AllTools | Self::Read => "All tools",
        }
    }
}

/// Thin message under the header; dismissed per document.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Banner {
    Encrypted,
    Unsaved,
}

/// What the chrome shows.
pub struct Shell {
    pub mode: Mode,
    /// Left tools panel shown (hidden in `Read`).
    pub show_tools: bool,
    /// "View more" open in the All tools list.
    pub tools_expanded: bool,
    /// The home tab is in front: the welcome screen shows even with documents open.
    pub home: bool,
    /// Banners closed with their cross, per document.
    pub dismissed: Vec<(EntityId, Banner)>,
    /// Files of the tabs closed, the last one last: what "Reopen closed tab" opens.
    pub closed: Vec<PathBuf>,
    /// Files opened, newest first (welcome screen, Menu > File > Open recent).
    pub recent: Vec<PathBuf>,
    /// Width of the tools panel (resizable 220..=420, 280 at first).
    pub tools_width: f32,
}

impl Default for Shell {
    fn default() -> Self {
        Self {
            mode: Mode::AllTools,
            show_tools: true,
            tools_expanded: false,
            home: false,
            dismissed: Vec::new(),
            closed: Vec::new(),
            recent: Vec::new(),
            tools_width: 280.0,
        }
    }
}

impl Shell {
    /// Put `path` first in the recent files.
    pub fn remember_recent(&mut self, path: &std::path::Path) {
        self.recent.retain(|recent| recent != path);
        self.recent.insert(0, path.to_path_buf());
        self.recent.truncate(MAX_RECENT);
    }
}

impl GodPdfApp {
    /// True when a document (not the home screen) fills the window.
    pub fn doc_view(&self) -> bool {
        !self.tabs.is_empty() && !self.shell.home
    }

    /// Menu > "Tools panel" (showing it leaves Read for All tools).
    pub fn toggle_tools_panel(
        &mut self,
        _: &ToggleToolsPanel,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let shell = &mut self.shell;
        shell.show_tools = !shell.show_tools;
        if shell.show_tools && shell.mode == Mode::Read {
            shell.mode = Mode::AllTools;
        }
        cx.notify();
    }

    /// Switch the header mode.
    pub fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if !self.doc_view() {
            self.shell.home = self.tabs.is_empty();
            cx.notify();
            return;
        }
        self.shell.mode = mode;
        if mode != Mode::Read {
            self.shell.show_tools = true;
        }
        if let Some(tab) = self.active_tab().cloned() {
            crate::organize::set_grid(&tab, mode == Mode::Organize, cx);
        }
        if mode == Mode::Comment {
            panels::show(panels::Panel::Comments, cx);
        }
        cx.notify();
    }
}

/// The whole window content but the overlays (dialogs, toasts).
pub fn render(
    app: &mut GodPdfApp,
    active: Option<&Entity<DocTab>>,
    window: &mut Window,
    cx: &mut Context<GodPdfApp>,
) -> AnyElement {
    let palette = Palette::of(cx);
    if let Some(tab) = active.filter(|_| app.doc_view()) {
        crate::organize::sync_mode(&mut app.shell.mode, tab, cx);
    }
    let tab_row = tabrow::render(app, &palette, window, cx);
    let header = header::render(app, &palette, cx);
    let body = match active.filter(|_| app.doc_view()) {
        Some(tab) => div()
            .flex_1()
            .min_h_0()
            .flex()
            .children(leftpanel::render(app, &palette, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .children(banner::render(app, tab, &palette, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .child(crate::organize::center(app, tab, window, cx)),
                    ),
            )
            .children(panels::right(Some(tab), window, cx))
            .child(rail::render(tab, &palette, cx))
            .into_any_element(),
        None => welcome::render(app, &palette, cx),
    };
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(tab_row)
        .child(header)
        .child(body)
        .into_any_element()
}
