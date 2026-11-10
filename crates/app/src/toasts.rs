//! Messages to the user. Success and info notes are dropped: the result shows on the page
//! and slow work shows the header's "Working…" spinner. Errors must not be lost: the last
//! one shows in a thin red-tinted bar under the header ([`bar`]) for 6 s or until closed.

use std::time::Duration;

use gpui::prelude::*;
use gpui::{AnyElement, App, Global, SharedString, div, px};

use crate::theme::{self, Palette};
use crate::{fonts, icons, widgets};

/// How long an error stays.
const ERROR_TTL: Duration = Duration::from_secs(6);

/// The error shown, latest wins.
#[derive(Default)]
pub struct Toasts {
    shown: Option<(u64, SharedString)>,
    next_id: u64,
}

impl Global for Toasts {}

/// Dropped (kept so callers need not care).
pub fn info(_cx: &mut App, _text: impl Into<SharedString>) {}

/// Dropped (kept so callers need not care).
pub fn success(_cx: &mut App, _text: impl Into<SharedString>) {}

/// Show `text` in the error bar, replacing the one shown; it goes after 6 s.
pub fn error(cx: &mut App, text: impl Into<SharedString>) {
    let toasts = cx.default_global::<Toasts>();
    let id = toasts.next_id;
    toasts.next_id += 1;
    toasts.shown = Some((id, text.into()));
    cx.refresh_windows();
    cx.spawn(async move |cx| {
        cx.background_executor().timer(ERROR_TTL).await;
        cx.update(|cx| dismiss(id, cx));
    })
    .detach();
}

/// Drop error `id` if it is still the one shown.
fn dismiss(id: u64, cx: &mut App) {
    let toasts = cx.default_global::<Toasts>();
    if toasts.shown.as_ref().is_some_and(|(shown, _)| *shown == id) {
        toasts.shown = None;
        cx.refresh_windows();
    }
}

/// The error bar: full width, error tint at 10 % over `bg`, 1 px line under it,
/// `WARNING_CIRCLE` 16 in `error`, text 13, close cross. `None` without an error.
pub fn bar(palette: &Palette, cx: &App) -> Option<AnyElement> {
    let (id, text) = cx.try_global::<Toasts>()?.shown.clone()?;
    Some(
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(theme::ITEM_SPACING.0))
            .min_h(px(36.))
            .pl(px(16.))
            .pr(px(8.))
            .py(px(4.))
            .bg(palette.error.opacity(0.1))
            .border_b_1()
            .border_color(palette.error.opacity(0.25))
            .text_size(px(fonts::BODY))
            .text_color(palette.text)
            .child(widgets::icon(icons::WARNING_CIRCLE, 16.0, palette.error))
            .child(div().flex_1().min_w_0().child(text))
            .child(
                widgets::small_icon_button("error-dismiss", icons::X, "Dismiss", palette)
                    .on_click(move |_, _, cx| dismiss(id, cx)),
            )
            .into_any_element(),
    )
}
