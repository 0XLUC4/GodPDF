//! Window chrome around the document. See DESIGN.md for
//! every measurement. One file per region:
//!
//! - `tabrow.rs`: home button, document tabs, "+ Open", updates / about / theme, and on
//!   Windows the title bar: drag area and minimize / maximize / close.
//! - `header.rs`: Menu, mode tabs, search pill, properties / save / export / print.
//! - `leftpanel.rs`: the tools panel ("All tools", ... ) left of the canvas.
//! - `rail.rs`: right icon rail (views, page box, zoom). The panel it opens is `panels::right`.
//! - `banner.rs`: thin message over the canvas (unsaved / encrypted); the error bar
//!   (`toasts::bar`) sits above it.
//! - `welcome.rs`: start screen (no document, or the home tab).
//!
//! Layout, top to bottom: tab row, header, then either the welcome screen or
//! `[tools panel][banner over canvas][right panel][rail]`.

mod banner;
pub(crate) mod header;
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
/// Files whose last page read is kept.
pub const MAX_LAST_PAGES: usize = 200;

/// Ctrl+Shift+T reopens the last closed tab.
pub fn register(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(secondary!("shift-t"), ReopenTab, None)]);
}

/// Height of the document tab row.
pub const TAB_HEIGHT: f32 = 38.0;
/// The system title bar is hidden and the tab row stands for it (drag, double click,
/// minimize / maximize / close buttons). Windows only: elsewhere the system one stays.
pub const CUSTOM_TITLE_BAR: bool = cfg!(windows);
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
    /// The page read last in each file (0-based), files opened last first.
    pub last_pages: Vec<(PathBuf, usize)>,
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
            last_pages: Vec::new(),
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

    /// The page read last in the file `path`, being opened again: it comes first.
    pub fn reopened_page(&mut self, path: &std::path::Path) -> Option<usize> {
        let index = self
            .last_pages
            .iter()
            .position(|(known, _)| known == path)?;
        let entry = self.last_pages.remove(index);
        let page = entry.1;
        self.last_pages.insert(0, entry);
        Some(page)
    }

    /// Note that `page` of the file `path` is being read. A file not known yet comes first,
    /// the oldest one beyond [`MAX_LAST_PAGES`] is forgotten.
    pub fn remember_page(&mut self, path: &std::path::Path, page: usize) {
        match self.last_pages.iter_mut().find(|(known, _)| known == path) {
            Some(known) => known.1 = page,
            None => {
                self.last_pages.insert(0, (path.to_path_buf(), page));
                self.last_pages.truncate(MAX_LAST_PAGES);
            }
        }
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

    /// Switch the header mode (from the home tab, back to the document in that mode).
    pub fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        self.shell.home = false;
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

/// Narrowest canvas kept: side panels are left out (right first) below it.
const CANVAS_MIN: f32 = 360.0;
/// Width of the right rail.
const RAIL_WIDTH: f32 = 52.0;

/// Which side panels fit a `window` wide body next to a canvas of at least `CANVAS_MIN`:
/// `(tools, right)` given their widths when shown. Render-time only, nothing is saved.
fn fit_panels(window: f32, tools: Option<f32>, right: Option<f32>) -> (bool, bool) {
    let room = window - RAIL_WIDTH - CANVAS_MIN;
    let (tools, right) = (tools.unwrap_or(0.0), right.unwrap_or(0.0));
    if tools + right <= room {
        (true, true)
    } else {
        (tools <= room, false)
    }
}

/// The whole window content but the overlays (dialogs, palette).
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
    let header = header::render(app, &palette, window, cx);
    let collaboration = crate::collaboration::panel(app, &palette, window, cx);
    let body = match active.filter(|_| app.doc_view()) {
        Some(tab) => {
            let tools = leftpanel::render(app, &palette, cx);
            let right = panels::right(Some(tab), window, cx);
            let (show_tools, show_right) = fit_panels(
                f32::from(window.viewport_size().width),
                tools.as_ref().map(|_| app.shell.tools_width),
                right.as_ref().and_then(|_| panels::open_width(cx)),
            );
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .children(tools.filter(|_| show_tools))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .children(crate::toasts::bar(&palette, cx))
                        .children(banner::render(app, tab, &palette, cx))
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .child(crate::organize::center(app, tab, window, cx)),
                        ),
                )
                .children(right.filter(|_| show_right))
                .child(rail::render(tab, &palette, cx))
                .into_any_element()
        }
        None => div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .children(crate::toasts::bar(&palette, cx))
            .child(welcome::render(app, &palette, window, cx))
            .into_any_element(),
    };
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(tab_row)
        .child(header)
        .children(collaboration)
        .child(body)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::fit_panels;

    #[test]
    fn side_panels_give_way_to_the_canvas_right_first() {
        assert_eq!(fit_panels(1280.0, Some(280.0), Some(320.0)), (true, true));
        assert_eq!(fit_panels(900.0, Some(280.0), Some(320.0)), (true, false));
        assert_eq!(fit_panels(640.0, Some(280.0), Some(320.0)), (false, false));
        assert_eq!(fit_panels(640.0, None, Some(220.0)), (true, true));
        assert_eq!(fit_panels(700.0, None, Some(320.0)), (true, false));
    }

    #[test]
    fn last_pages_are_kept_per_file_within_bounds() {
        let mut shell = super::Shell::default();
        let path = |index: usize| std::path::PathBuf::from(format!("/{index}.pdf"));
        shell.remember_page(&path(0), 4);
        shell.remember_page(&path(0), 7);
        shell.remember_page(&path(1), 1);
        assert_eq!(shell.reopened_page(&path(0)), Some(7));
        assert_eq!(shell.last_pages[0].0, path(0));
        assert_eq!(shell.reopened_page(&path(2)), None);
        for index in 2..=super::MAX_LAST_PAGES {
            shell.remember_page(&path(index), index);
        }
        // The oldest is forgotten: page 1 was read before the file 0 was opened again.
        assert_eq!(shell.last_pages.len(), super::MAX_LAST_PAGES);
        assert_eq!(shell.reopened_page(&path(1)), None);
        assert_eq!(shell.reopened_page(&path(0)), Some(7));
    }
}
