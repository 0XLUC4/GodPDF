//! Small shared controls in the palette of `theme.rs`.

use gpui::prelude::*;
use gpui::{Action, App, Div, ElementId, SharedString, Stateful, div, px};

use crate::theme::Palette;

/// A flat tool bar button. Add `.on_click(..)`, or use [`action_button`].
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    palette: &Palette,
) -> Stateful<Div> {
    let hover = palette.hover;
    div()
        .id(id)
        .flex()
        .items_center()
        .px(px(10.))
        .h(px(28.))
        .rounded(px(7.))
        .text_sm()
        .text_color(palette.text)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(label.into())
}

/// A button that dispatches `action`, so it runs the same handler as its shortcut.
pub fn action_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    action: impl Action + Clone,
    palette: &Palette,
) -> Stateful<Div> {
    button(id, label, palette).on_click(move |_, window, cx: &mut App| {
        window.dispatch_action(action.boxed_clone(), cx);
    })
}

/// The filled button of a screen's main action.
pub fn primary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    palette: &Palette,
) -> Stateful<Div> {
    let (accent, on_accent) = (palette.accent, palette.on_accent);
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .px(px(18.))
        .h(px(38.))
        .rounded(px(9.))
        .bg(accent)
        .text_color(on_accent)
        .cursor_pointer()
        .hover(|style| style.opacity(0.85))
        .child(label.into())
}

/// A thin vertical rule between tool bar groups.
pub fn separator(palette: &Palette) -> Div {
    div().w(px(1.)).h(px(18.)).mx(px(4.)).bg(palette.border)
}
