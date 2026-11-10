//! Tool bar and per-tool canvas interactions. EMPTY in wave 0: these are the hooks.
//!
//! How a tool plugs in (see ARCHITECTURE.md):
//! - Keep the active tool app-wide in a `Global` defined here (e.g. `ToolState`), switched by
//!   actions registered in `register`.
//! - [`toolbar`] returns the tool bar shown under the main tool bar row.
//! - [`page_overlay`] is called by the viewer for every visible page. Return an element
//!   sized to the page (`.absolute().size_full()`): it receives mouse events in page space.
//!   Turn its positions into displayed page points with `(pos - page_origin) / zoom`, then
//!   into PDF space with `geometry::to_pdf`, and edit through `tab.apply(..)`.

use gpui::{AnyElement, App, Context, Window};

use crate::doc::DocTab;
use crate::geometry::PageRect;

/// Register tool actions and their key bindings. Called once at startup.
pub fn register(_cx: &mut App) {}

/// The tool bar row; `None` hides it.
pub fn toolbar(_window: &mut Window, _cx: &mut App) -> Option<AnyElement> {
    None
}

/// Interactive layer over one page. `rect` is the page in content space (logical pixels).
pub fn page_overlay(
    _tab: &mut DocTab,
    _page: usize,
    _rect: PageRect,
    _window: &mut Window,
    _cx: &mut Context<DocTab>,
) -> Option<AnyElement> {
    None
}
