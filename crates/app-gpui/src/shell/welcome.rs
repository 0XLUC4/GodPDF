//! Start screen (egui `welcome::show`): logo 112 (radius 24), name (40), tagline (17, dim),
//! then Open / New / Merge buttons (170 × 44, radius 10, text 16) and the recent files.
//! Shown without a document or when the home tab is in front. Fill `bg`, rows 6 px apart
//! (egui item spacing), centered in the view.

use std::sync::{Arc, LazyLock};

use gpui::prelude::*;
use gpui::{AnyElement, Context, Image, ImageFormat, SharedString, div, img, px};

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
/// egui `TextStyle::Small`.
const SMALL: f32 = 9.0;

pub fn render(app: &GodPdfApp, palette: &Palette, cx: &mut Context<GodPdfApp>) -> AnyElement {
    let button = |id: &'static str, glyph, label: &'static str, primary: bool| {
        // egui: Open has a fixed `accent` fill; the others the widget fills (surface, hover,
        // pressed `selected`) inside a 1 px `accent` stroke.
        let (hover, pressed) = (palette.hover, palette.selected);
        let text = if primary {
            palette.on_accent
        } else {
            palette.text
        };
        div()
            .id(id)
            .w(px(170.))
            .h(px(44.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(10.))
            .when(primary, |button| button.bg(palette.accent))
            .when(!primary, |button| {
                button
                    .bg(palette.surface)
                    .border_1()
                    .border_color(palette.accent)
                    .hover(move |style| style.bg(hover))
                    .active(move |style| style.bg(pressed))
            })
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

    let recent: Vec<_> = app
        .shell
        .recent
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let name: SharedString = path
                .file_name()
                .map_or_else(
                    || path.display().to_string(),
                    |name| name.to_string_lossy().into_owned(),
                )
                .into();
            let tip = path.display().to_string();
            let path = path.clone();
            let row = div()
                .id(("recent", index))
                .child(widgets::with_icon(
                    icons::FILE_PDF,
                    name,
                    fonts::BODY,
                    palette.text,
                ))
                .on_click(cx.listener(move |app, _, _, cx| app.open_path(&path, cx)));
            widgets::tooltip(row, tip)
        })
        .collect();
    let has_recent = !recent.is_empty();

    div()
        .id("welcome")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .bg(palette.bg)
        .text_size(px(fonts::BODY))
        .child(
            // egui centers this column in the view, 16 px at least from the top.
            div()
                .min_h_full()
                .py(px(16.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(6.))
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
                        .child(
                            button("welcome-open", icons::FOLDER_OPEN, "Open PDF", true)
                                .on_click(open),
                        )
                        .child(
                            button("welcome-new", icons::FILE_PLUS, "New PDF", false)
                                .on_click(new_pdf),
                        )
                        .child(
                            button("welcome-merge", icons::STACK, "Merge PDFs", false)
                                .on_click(dispatch(MergeFiles)),
                        ),
                )
                .when(has_recent, |column| {
                    column
                        .child(div().h(px(26. - 6.)))
                        .child(
                            div()
                                .text_size(px(SMALL))
                                .text_color(palette.accent_text)
                                .child("RECENT"),
                        )
                        .children(recent)
                }),
        )
        .into_any_element()
}
