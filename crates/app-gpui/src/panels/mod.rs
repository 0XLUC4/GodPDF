//! Left sidebar (thumbnails, bookmarks, search) and right panels (comments, properties).
//! EMPTY in wave 0: these are the hooks.
//!
//! How a panel plugs in (see ARCHITECTURE.md): keep which panel is open in a `Global` or in
//! a field of `GodPdfApp`; return it from [`left`] / [`right`]. Read the active document
//! with `tab.read(cx)`, edit it with `tab.update(cx, |tab, cx| tab.apply(..))`, and jump with
//! `tab.scroll_to_page(page)` followed by `cx.notify()`.

use gpui::{AnyElement, App, Entity, Window};

use crate::doc::DocTab;

/// Register panel actions and their key bindings. Called once at startup.
pub fn register(_cx: &mut App) {}

/// Left of the viewer; `None` hides it. `tab` is the active document, if any.
pub fn left(
    _tab: Option<&Entity<DocTab>>,
    _window: &mut Window,
    _cx: &mut App,
) -> Option<AnyElement> {
    None
}

/// Right of the viewer; `None` hides it.
pub fn right(
    _tab: Option<&Entity<DocTab>>,
    _window: &mut Window,
    _cx: &mut App,
) -> Option<AnyElement> {
    None
}
