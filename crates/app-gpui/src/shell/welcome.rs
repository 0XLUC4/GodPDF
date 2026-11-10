//! Start screen (egui `welcome::show`): logo 112 (radius 24), name (40), tagline (17, dim),
//! then Open / New / Merge buttons (170 × 44, radius 10, text 16) and the recent files.
//! Shown without a document or when the home tab is in front. Fill `bg`.

use std::sync::{Arc, LazyLock};

use gpui::prelude::*;
use gpui::{AnyElement, Context, Image, ImageFormat, div, img, px};

use crate::app::{GodPdfApp, Open};
use crate::dialogs::MergeFiles;
use crate::theme::Palette;
use crate::widgets::{self, dispatch};
use crate::{fonts, icons, toasts};

static LOGO: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Png,
        include_bytes!("../../../../assets/logo.png").to_vec(),
    ))
});

const TAGLINE: &str = "Every PDF tool. Local. Pure Rust.";

pub fn render(palette: &Palette, cx: &mut Context<GodPdfApp>) -> AnyElement {
    let button = |id: &'static str, glyph, label: &'static str, primary: bool| {
        let (fill, text) = if primary {
            (palette.accent, palette.on_accent)
        } else {
            (palette.bg, palette.text)
        };
        let hover = palette.hover;
        div()
            .id(id)
            .w(px(170.))
            .h(px(44.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(10.))
            .bg(fill)
            .when(!primary, |button| {
                button
                    .border_1()
                    .border_color(palette.accent)
                    .hover(move |style| style.bg(hover))
            })
            .when(primary, |button| button.hover(|style| style.opacity(0.9)))
            .cursor_pointer()
            .child(widgets::with_icon(glyph, label, 16.0, text))
    };
    let new_pdf = cx.listener(|app, _, _, cx| {
        // One empty A4 page to write, draw and insert into.
        match godpdf_core::blank_document(595.0, 842.0) {
            Ok(bytes) => {
                app.open_bytes("Untitled.pdf".to_owned(), None, bytes, cx);
                app.shell.home = false;
            }
            Err(err) => toasts::error(cx, err.to_string()),
        }
    });
    let open = cx.listener(|app, _, window, cx| {
        app.shell.home = false;
        window.dispatch_action(Box::new(Open), cx);
    });

    div()
        .id("welcome")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .bg(palette.bg)
        .text_size(px(fonts::BODY))
        .child(img(LOGO.clone()).size(px(112.)).rounded(px(24.)))
        .child(div().h(px(10. - 6.)))
        .child(
            div()
                .text_size(px(40.))
                .text_color(palette.text)
                .child(crate::APP_NAME),
        )
        .child(
            div()
                .text_size(px(17.))
                .text_color(palette.text_dim)
                .child(TAGLINE),
        )
        .child(div().h(px(24. - 6.)))
        .child(
            div()
                .flex()
                .gap(px(8.))
                .child(button("welcome-open", icons::FOLDER_OPEN, "Open PDF", true).on_click(open))
                .child(button("welcome-new", icons::FILE_PLUS, "New PDF", false).on_click(new_pdf))
                .child(
                    button("welcome-merge", icons::STACK, "Merge PDFs", false)
                        .on_click(dispatch(MergeFiles)),
                ),
        )
        // TODO(welcome): "RECENT" list (small, strong, `accent_text`) of FILE_PDF + name rows
        // once GPUI keeps recent files; "or drop PDF files here" is GPUI-only help.
        .child(div().h(px(26. - 6.)))
        .child(
            div()
                .text_size(px(fonts::BODY))
                .text_color(palette.text_dim)
                .child("or drop PDF files here"),
        )
        .into_any_element()
}
