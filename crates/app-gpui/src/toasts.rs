//! Transient notifications, bottom-right. A [`Global`]: push from anywhere with
//! [`info`] / [`success`] / [`error`]; the root view observes it and redraws.

use std::time::Duration;

use gpui::prelude::*;
use gpui::{App, Global, SharedString, div, px};

use crate::theme::Palette;
use crate::{fonts, icons, widgets};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Info,
    Success,
    Error,
}

struct Toast {
    id: u64,
    level: Level,
    text: SharedString,
}

#[derive(Default)]
pub struct Toasts {
    items: Vec<Toast>,
    next_id: u64,
}

impl Global for Toasts {}

/// At most this many toasts show at once; the oldest goes first.
const MAX_SHOWN: usize = 5;

pub fn push(cx: &mut App, level: Level, text: impl Into<SharedString>) {
    let toasts = cx.default_global::<Toasts>();
    let id = toasts.next_id;
    toasts.next_id += 1;
    toasts.items.push(Toast {
        id,
        level,
        text: text.into(),
    });
    if toasts.items.len() > MAX_SHOWN {
        toasts.items.remove(0);
    }
    let ttl = Duration::from_secs(if level == Level::Error { 8 } else { 4 });
    cx.spawn(async move |cx| {
        cx.background_executor().timer(ttl).await;
        cx.update_global::<Toasts, _>(|toasts, _| toasts.items.retain(|toast| toast.id != id));
    })
    .detach();
}

pub fn info(cx: &mut App, text: impl Into<SharedString>) {
    push(cx, Level::Info, text);
}

pub fn success(cx: &mut App, text: impl Into<SharedString>) {
    push(cx, Level::Success, text);
}

pub fn error(cx: &mut App, text: impl Into<SharedString>) {
    push(cx, Level::Error, text);
}

/// The toast stack, absolutely placed in the bottom-right of its parent (egui: anchored
/// 16 from the right and 40 from the bottom, 6 of space after each toast). A click on a toast
/// dismisses it.
pub fn render(cx: &App) -> Option<impl IntoElement> {
    let toasts = cx.try_global::<Toasts>()?;
    if toasts.items.is_empty() {
        return None;
    }
    let palette = Palette::of(cx);
    Some(
        div()
            .absolute()
            .bottom(px(46.))
            .right(px(16.))
            .flex()
            .flex_col()
            .items_end()
            // egui item spacing 6 + the 6 of space after each toast.
            .gap(px(12.))
            .children(toasts.items.iter().map(|toast| {
                let (accent, glyph) = match toast.level {
                    Level::Info => (palette.accent, icons::INFO),
                    Level::Success => (palette.success, icons::CHECK_CIRCLE),
                    Level::Error => (palette.error, icons::WARNING_CIRCLE),
                };
                let id = toast.id;
                let toast = div()
                    .id(("toast", id as usize))
                    .flex()
                    .items_center()
                    .gap(px(crate::theme::ITEM_SPACING.0))
                    .max_w(px(380. + 28.))
                    .px(px(14.))
                    .py(px(10.))
                    .rounded(px(10.))
                    .shadow(widgets::popup_shadow(&palette))
                    .bg(palette.surface)
                    .border_1()
                    .border_color(accent)
                    .text_size(px(fonts::BODY))
                    .text_color(palette.text)
                    .cursor_pointer()
                    .child(widgets::icon(glyph, 16.0, accent))
                    .child(toast.text.clone())
                    .on_click(move |_, _, cx| {
                        cx.update_global::<Toasts, _>(|toasts, _| {
                            toasts.items.retain(|toast| toast.id != id)
                        });
                        cx.refresh_windows();
                    });
                widgets::tooltip(toast, "Click to dismiss")
            })),
    )
}
