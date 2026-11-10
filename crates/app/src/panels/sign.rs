//! The Signature panel: the saved signature and what to do with it.

use gpui::prelude::*;
use gpui::{AnyElement, App, Window, canvas, div, point, px, white};

use crate::signature::{self, Signature};
use crate::theme::Palette;
use crate::widgets::{framed_button, with_icon};
use crate::{fonts, icons, toasts, tools};

/// Widest preview of the saved signature.
const PREVIEW_WIDTH: f32 = 280.0;

fn draw(cx: &mut App) {
    crate::dialogs::draw_signature(None, cx);
}

fn place(window: &mut Window, cx: &mut App) {
    window.dispatch_action(Box::new(tools::SignTool), cx);
    toasts::info(cx, "Click where your signature goes");
}

/// The saved signature on white, `room` wide at most, fitted with a margin of 8.
fn preview(signature: &Signature, room: f32) -> AnyElement {
    let width = PREVIEW_WIDTH.min(room).floor();
    let height = (signature.height() * width).clamp(40.0, 140.0) + 16.0;
    let scale = (width - 16.0).min((height - 16.0) / signature.height().max(0.01));
    let strokes: Vec<Vec<(f32, f32)>> = signature
        .strokes
        .iter()
        .map(|stroke| {
            stroke
                .iter()
                .map(|&(x, y)| (x * scale, y * scale))
                .collect()
        })
        .collect();
    div()
        .relative()
        .w(px(width))
        .h(px(height))
        .rounded(px(6.))
        .bg(white())
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let origin = bounds.origin + point(px(8.), px(8.));
                    signature::paint_strokes(window, origin, &strokes, 2.0);
                },
            )
            .absolute()
            .inset_0(),
        )
        .into_any_element()
}

/// The panel, `room` the width its content can take.
pub fn render(room: f32, cx: &mut App) -> AnyElement {
    let palette = Palette::of(cx);
    let Some(signature) = signature::saved(cx) else {
        return div()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(6.))
            .child(super::label(
                "Draw your signature once: it is kept for the next time.",
                palette.text_dim,
            ))
            .child(
                icon_button(
                    "signature-draw",
                    icons::PENCIL_SIMPLE,
                    "Draw my signature",
                    &palette,
                )
                .on_click(|_, _, cx| draw(cx)),
            )
            .into_any_element();
    };
    let sign = crate::widgets::accent_button(
        "signature-place",
        with_icon(
            icons::SIGNATURE,
            "Sign the page",
            fonts::BODY,
            palette.on_accent,
        ),
        true,
        &palette,
    )
    .on_click(|_, window, cx| place(window, cx));
    div()
        .flex()
        .flex_col()
        .items_start()
        .gap(px(6.))
        .child(preview(&signature, room))
        .child(div().pt(px(2.)).child(crate::widgets::tooltip(
            sign,
            "Then click where the signature goes",
        )))
        .child(
            div()
                .flex()
                .gap(px(8.))
                .child(
                    icon_button(
                        "signature-again",
                        icons::PENCIL_SIMPLE,
                        "Draw again",
                        &palette,
                    )
                    .on_click(|_, _, cx| draw(cx)),
                )
                .child(
                    icon_button("signature-forget", icons::TRASH, "Forget it", &palette)
                        .on_click(|_, _, cx| signature::set(None, cx)),
                ),
        )
        .into_any_element()
}

/// A framed button with a glyph and a label.
fn icon_button(
    id: &'static str,
    glyph: &'static str,
    label: &'static str,
    palette: &Palette,
) -> gpui::Stateful<gpui::Div> {
    framed_button(
        id,
        with_icon(glyph, label, fonts::BODY, palette.text),
        true,
        palette,
    )
}
