//! Info banner over the canvas: fill `bg`, 36 high, padding 16 / 8, a `border` line under
//! it. Glyph 16 dim + text (13), then on the right a secondary button and a small dismiss
//! cross. Dismissed per document.

use gpui::prelude::*;
use gpui::{AnyElement, Context, Entity, div, px};

use super::Banner;
use crate::app::{GodPdfApp, Save};
use crate::dialogs::{self, RemovePassword};
use crate::doc::DocTab;
use crate::theme::{self, Palette};
use crate::widgets::{self, dispatch, icon};
use crate::{fonts, icons};

pub fn render(
    app: &GodPdfApp,
    tab: &Entity<DocTab>,
    palette: &Palette,
    cx: &mut Context<GodPdfApp>,
) -> Option<AnyElement> {
    let id = tab.entity_id();
    let kind = if dialogs::has_password(id, cx) {
        Banner::Encrypted
    } else if tab.read(cx).is_dirty() {
        Banner::Unsaved
    } else {
        return None;
    };
    let key = (id, kind);
    if app.shell.dismissed.contains(&key) {
        return None;
    }
    let (glyph, text, label) = match kind {
        Banner::Encrypted => (
            icons::LOCK,
            "This document will be saved with a password.",
            "Remove password",
        ),
        Banner::Unsaved => (icons::INFO, "This document has unsaved changes.", "Save"),
    };
    let action = widgets::framed_button("banner-action", label, true, palette);
    let action = match kind {
        Banner::Encrypted => action.on_click(dispatch(RemovePassword)),
        Banner::Unsaved => action.on_click(dispatch(Save)),
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
            .bg(palette.bg)
            .border_b_1()
            .border_color(palette.border)
            .text_size(px(fonts::BODY))
            .child(icon(glyph, 16.0, palette.text_dim))
            .child(text)
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .items_center()
                    .gap(px(theme::ITEM_SPACING.0))
                    .child(action)
                    .child(
                        widgets::small_icon_button("banner-dismiss", icons::X, "Dismiss", palette)
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.shell.dismissed.push(key);
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element(),
    )
}
