//! Messages to the user. Success and info notes are dropped: the result shows on the page
//! and slow work shows the header's "Working…" spinner. Errors must not be lost: the last
//! one shows in a thin red-tinted bar under the header ([`bar`]) for 6 s or until closed.
//! [`note`] uses the same bar, neutral, for an action that changed nothing (or whose only
//! result is a number): without it the click would seem to do nothing.

use std::time::Duration;

use gpui::prelude::*;
use gpui::{AnyElement, App, Global, SharedString, div, px};

use crate::theme::{self, Palette};
use crate::{fonts, icons, widgets};

/// How long an error stays.
const ERROR_TTL: Duration = Duration::from_secs(6);

/// The message shown (id, text, is an error), latest wins.
#[derive(Default)]
pub struct Toasts {
    shown: Option<(u64, SharedString, bool)>,
    next_id: u64,
}

impl Global for Toasts {}

/// Dropped (kept so callers need not care).
pub fn info(_cx: &mut App, _text: impl Into<SharedString>) {}

/// Dropped (kept so callers need not care).
pub fn success(_cx: &mut App, _text: impl Into<SharedString>) {}

/// Show `text` in the error bar, replacing the one shown; it goes after 6 s.
pub fn error(cx: &mut App, text: impl Into<SharedString>) {
    show(cx, text.into(), true);
}

/// Show `text` in the bar, neutral: the outcome of an action that changed nothing.
pub fn note(cx: &mut App, text: impl Into<SharedString>) {
    show(cx, text.into(), false);
}

fn show(cx: &mut App, text: SharedString, is_error: bool) {
    let toasts = cx.default_global::<Toasts>();
    let id = toasts.next_id;
    toasts.next_id += 1;
    toasts.shown = Some((id, text, is_error));
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
    if toasts
        .shown
        .as_ref()
        .is_some_and(|(shown, ..)| *shown == id)
    {
        toasts.shown = None;
        cx.refresh_windows();
    }
}

/// The error bar: full width, error tint at 10 % over `bg`, 1 px line under it,
/// `WARNING_CIRCLE` 16 in `error`, text 13, close cross; a note: `surface`, `border` line,
/// `INFO` in `text_dim`. `None` without a message.
pub fn bar(palette: &Palette, cx: &App) -> Option<AnyElement> {
    let (id, text, is_error) = cx.try_global::<Toasts>()?.shown.clone()?;
    let (fill, line, glyph, tint) = if is_error {
        let error = palette.error;
        (
            // Opaque: the tint over `bg`, whatever sits behind the bar.
            palette.bg.blend(error.opacity(0.1)),
            error.opacity(0.25),
            icons::WARNING_CIRCLE,
            error,
        )
    } else {
        (
            palette.surface,
            palette.border,
            icons::INFO,
            palette.text_dim,
        )
    };
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
            .bg(fill)
            .border_b_1()
            .border_color(line)
            .text_size(px(fonts::BODY))
            .text_color(palette.text)
            .child(widgets::icon(glyph, 16.0, tint))
            .child(div().flex_1().min_w_0().child(text))
            .child(
                widgets::small_icon_button("error-dismiss", icons::X, "Dismiss", palette)
                    .on_click(move |_, _, cx| dismiss(id, cx)),
            )
            .into_any_element(),
    )
}
