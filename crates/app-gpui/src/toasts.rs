//! Transient notifications, bottom-right. A [`Global`]: push from anywhere with
//! [`info`] / [`success`] / [`error`]; the root view observes it and redraws.

use std::time::Duration;

use gpui::prelude::*;
use gpui::{App, Global, SharedString, div, px};

use crate::theme::Palette;

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

/// The toast stack, absolutely placed in the bottom-right of its parent.
pub fn render(cx: &App) -> Option<impl IntoElement> {
    let toasts = cx.try_global::<Toasts>()?;
    if toasts.items.is_empty() {
        return None;
    }
    let palette = Palette::of(cx);
    Some(
        div()
            .absolute()
            .bottom(px(16.))
            .right(px(16.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .max_w(px(420.))
            .children(toasts.items.iter().map(|toast| {
                let marker = match toast.level {
                    Level::Info => "i",
                    Level::Success => "✓",
                    Level::Error => "!",
                };
                div()
                    .flex()
                    .gap(px(10.))
                    .px(px(14.))
                    .py(px(10.))
                    .rounded(px(9.))
                    .shadow_md()
                    .bg(palette.panel)
                    .border_1()
                    .border_color(if toast.level == Level::Error {
                        palette.accent
                    } else {
                        palette.border
                    })
                    .text_color(palette.text)
                    .text_sm()
                    .child(div().font_weight(gpui::FontWeight::BOLD).child(marker))
                    .child(toast.text.clone())
            })),
    )
}
