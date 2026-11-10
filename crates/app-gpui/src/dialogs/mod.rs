//! Modal dialogs and the command palette. EMPTY in wave 0: this is the hook.
//!
//! How a dialog plugs in (see ARCHITECTURE.md): an action handler opens it (store its state
//! in a `Global` defined here, or use `gpui::component` dialogs through
//! `window.open_dialog(..)` since the window root is a gpui-kit `Root`). [`overlay`] is
//! drawn over the whole window; return `None` when nothing is open. Text fields:
//! `gpui::component::input::{Input, InputState}`.

use gpui::{AnyElement, App, Window};

/// Register dialog actions (command palette: Ctrl+K) and their key bindings.
pub fn register(_cx: &mut App) {}

/// Drawn over the window when a dialog is open.
pub fn overlay(_window: &mut Window, _cx: &mut App) -> Option<AnyElement> {
    None
}
