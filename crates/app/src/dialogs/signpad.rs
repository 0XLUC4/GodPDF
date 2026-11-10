//! The signature pad dialog: draw with the
//! mouse or a pen; confirming keeps it on disk, then signs where the Sign tool was clicked,
//! or picks the Sign tool.

use std::cell::Cell;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Bounds, MouseButton, Pixels, Point, WeakEntity, Window, canvas, div, point,
    px, rgb, size, white,
};

use super::{DialogCancel, DialogConfirm, Open, card, confirm, state};
use crate::doc::DocTab;
use crate::signature::{self, Signature};
use crate::theme::Palette;
use crate::widgets::{accent_button, framed_button, with_icon};
use crate::{icons, tools};

/// Size of the drawing pad on screen.
const PAD: (f32, f32) = (440.0, 170.0);

/// Where a click of the Sign tool asked for a signature: document, page, displayed point.
pub type Place = (WeakEntity<DocTab>, usize, (f32, f32));

pub struct Pad {
    /// Strokes in pad pixels, origin top-left.
    strokes: Vec<Vec<(f32, f32)>>,
    place: Option<Place>,
    /// A signature is already kept: confirming with nothing drawn uses it.
    saved: bool,
    drawing: bool,
    /// Top-left corner of the pad in the window, as of the last frame.
    origin: Rc<Cell<Point<Pixels>>>,
}

/// Open the pad; `place` is where the signature goes once drawn.
pub fn open(place: Option<Place>, cx: &mut App) {
    let saved = signature::saved(cx).is_some();
    state(cx).open = Some(Open::Signature(Pad {
        strokes: Vec::new(),
        place,
        saved,
        drawing: false,
        origin: Rc::default(),
    }));
    cx.refresh_windows();
}

fn with_pad(cx: &mut App, update: impl FnOnce(&mut Pad)) {
    if let Some(Open::Signature(pad)) = &mut state(cx).open {
        update(pad);
    }
}

fn pad_point(pad: &Pad, position: Point<Pixels>) -> (f32, f32) {
    let origin = pad.origin.get();
    (
        f32::from(position.x - origin.x).clamp(0.0, PAD.0),
        f32::from(position.y - origin.y).clamp(0.0, PAD.1),
    )
}

/// Keep what was drawn, then sign. `Err` keeps the dialog open.
pub fn confirm_pad(pad: &Pad, window: &mut Window, cx: &mut App) -> Result<(), String> {
    match Signature::from_pad(&pad.strokes) {
        Some(drawn) => signature::set(Some(drawn), cx),
        None if pad.saved => {}
        None => return Err("Draw your signature first".to_owned()),
    }
    match &pad.place {
        Some((tab, page, at)) => {
            if let Some(tab) = tab.upgrade() {
                tools::place_signature(&tab, *page, *at, cx);
            }
        }
        None => {
            window.dispatch_action(Box::new(tools::SignTool), cx);
            crate::toasts::info(cx, "Click where your signature goes");
        }
    }
    Ok(())
}

/// The strokes painted over the pad.
fn ink(strokes: Vec<Vec<(f32, f32)>>, origin: Rc<Cell<Point<Pixels>>>) -> AnyElement {
    canvas(
        move |bounds: Bounds<Pixels>, _, _| origin.set(bounds.origin),
        move |bounds, _, window, _| {
            // The line to sign on, 40 above the bottom.
            let line_y = bounds.size.height - px(40.);
            window.paint_quad(gpui::fill(
                Bounds::new(
                    bounds.origin + point(px(24.), line_y),
                    size(bounds.size.width - px(48.), px(1.)),
                ),
                rgb(0xC8C8C8),
            ));
            signature::paint_strokes(window, bounds.origin, &strokes, 2.5);
        },
    )
    .absolute()
    .inset_0()
    .into_any_element()
}

pub fn render(pad: &Pad, palette: &Palette) -> AnyElement {
    let palette = *palette;
    let drawn = pad.strokes.iter().any(|stroke| !stroke.is_empty());
    let label = if drawn || !pad.saved {
        "Use this signature"
    } else {
        "Use my saved signature"
    };
    let ready = drawn || pad.saved;
    let surface = div()
        .id("signature-pad")
        .relative()
        .w(px(PAD.0))
        .h(px(PAD.1))
        .flex_none()
        .rounded(px(6.))
        .bg(white())
        .border_1()
        .border_color(palette.border)
        .cursor_crosshair()
        .on_mouse_down(MouseButton::Left, |event, window, cx| {
            with_pad(cx, |pad| {
                let at = pad_point(pad, event.position);
                pad.strokes.push(vec![at]);
                pad.drawing = true;
            });
            window.refresh();
        })
        .on_mouse_move(|event, window, cx| {
            with_pad(cx, |pad| {
                if !pad.drawing || !event.dragging() {
                    pad.drawing = false;
                    return;
                }
                let at = pad_point(pad, event.position);
                if let Some(stroke) = pad.strokes.last_mut()
                    && stroke.last() != Some(&at)
                {
                    stroke.push(at);
                }
            });
            window.refresh();
        })
        .on_mouse_up(MouseButton::Left, |_, _, cx| {
            with_pad(cx, |pad| pad.drawing = false)
        })
        .on_mouse_up_out(MouseButton::Left, |_, _, cx| {
            with_pad(cx, |pad| pad.drawing = false)
        })
        .child(ink(pad.strokes.clone(), Rc::clone(&pad.origin)))
        .when(!drawn, |surface| {
            surface.child(
                div()
                    .absolute()
                    .inset_0()
                    .pb(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(16.))
                    .text_color(rgb(0xAAAAAA))
                    .child("Sign here"),
            )
        });
    card(&palette, PAD.0)
        .key_context("GodPdfDialog")
        .on_action(|_: &DialogConfirm, window, cx| confirm(false, window, cx))
        .on_action(|_: &DialogCancel, window, cx| super::close(window, cx))
        .child(super::title(with_icon(
            icons::SIGNATURE,
            "Your signature",
            18.,
            palette.text,
        )))
        .child(
            div()
                .text_color(palette.text_dim)
                .child("Draw it with the mouse or a pen. It is kept for the next time."),
        )
        .child(div().pt(px(6.)).child(surface))
        .child(
            super::button_row()
                .pt(px(8.))
                .child(
                    accent_button("signature-use", label, ready, &palette).when(ready, |button| {
                        button.on_click(|_, window, cx| confirm(false, window, cx))
                    }),
                )
                .child(
                    framed_button("signature-clear", "Clear", drawn, &palette).when(drawn, |b| {
                        b.on_click(|_, window, cx| {
                            with_pad(cx, |pad| pad.strokes.clear());
                            window.refresh();
                        })
                    }),
                )
                .child(
                    framed_button("signature-cancel", "Cancel", true, &palette)
                        .on_click(|_, window, cx| super::close(window, cx)),
                ),
        )
        .into_any_element()
}
