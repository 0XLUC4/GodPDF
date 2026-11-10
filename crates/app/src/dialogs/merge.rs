//! Combine files: a list of PDFs to put one after the other into a new tab (ported from
//! `merge` in the egui `src/dialogs.rs`).

use gpui::prelude::*;
use gpui::{AnyElement, App, Window, div, px};

use super::kinds::open_tab;
use super::{DialogCancel, DialogConfirm, Open, card, close, state};
use crate::doc::guarded;
use crate::files::PickedFile;
use crate::theme::Palette;
use crate::toasts;
use crate::widgets::{accent_button, framed_button, small_icon_button};

pub struct Merge {
    pub files: Vec<PickedFile>,
    pub error: Option<String>,
}

/// Ask for PDFs, pictures or text files, each made into a PDF.
pub fn pick(cx: &mut App) -> Vec<PickedFile> {
    crate::import::pick_as_pdfs(cx)
}

fn with_merge(cx: &mut App, change: impl FnOnce(&mut Merge)) {
    if let Some(Open::Merge(merge)) = &mut state(cx).open {
        change(merge);
    }
}

fn add_files(window: &mut Window, cx: &mut App) {
    let picked = pick(cx);
    with_merge(cx, |merge| merge.files.extend(picked));
    window.refresh();
}

/// Merge into a new tab. `Err` keeps the dialog open.
pub fn confirm(merge: &Merge, cx: &mut App) -> Result<(), String> {
    if merge.files.len() < 2 {
        return Err("Add at least two files".to_owned());
    }
    let inputs: Vec<&[u8]> = merge
        .files
        .iter()
        .map(|file| file.bytes.as_slice())
        .collect();
    let bytes =
        guarded(|| godpdf_core::merge(&inputs)).map_err(|err| format!("Merge failed: {err}"))?;
    open_tab("merged.pdf".to_owned(), bytes, cx);
    toasts::success(cx, format!("Merged {} files", merge.files.len()));
    Ok(())
}

pub fn render(merge: &Merge, palette: &Palette) -> AnyElement {
    let palette = *palette;
    let count = merge.files.len();
    let rows = merge.files.iter().enumerate().map(|(index, file)| {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .flex_none()
                    .text_color(palette.text_dim)
                    .child(format!("{}.", index + 1)),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(file.name.clone()),
            )
            // egui: right to left, Remove, Move down, Move up; the ends are disabled.
            .child(arrow(
                ("merge-up", index),
                crate::icons::CARET_UP,
                "Move up",
                index > 0,
                &palette,
                move |merge| merge.files.swap(index, index - 1),
            ))
            .child(arrow(
                ("merge-down", index),
                crate::icons::CARET_DOWN,
                "Move down",
                index + 1 < count,
                &palette,
                move |merge| merge.files.swap(index, index + 1),
            ))
            .child(
                small_icon_button(("merge-remove", index), crate::icons::X, "Remove", &palette)
                    .on_click(move |_, window, cx| {
                        with_merge(cx, |merge| {
                            merge.files.remove(index);
                        });
                        window.refresh();
                    }),
            )
    });
    card(&palette, 420.)
        .key_context("GodPdfDialog")
        .on_action(|_: &DialogConfirm, window, cx| super::confirm(false, window, cx))
        .on_action(|_: &DialogCancel, window, cx| close(window, cx))
        .child(super::title(crate::widgets::with_icon(
            crate::icons::STACK,
            "Combine files",
            19.0,
            palette.text,
        )))
        .child(
            div()
                .pb(px(6.))
                .text_color(palette.text_dim)
                .child("PDFs, images and text files are combined top to bottom."),
        )
        .child(
            div()
                .id("merge-files")
                .flex()
                .flex_col()
                .gap(px(4.))
                .max_h(px(260.))
                .overflow_y_scroll()
                .when(count == 0, |list| {
                    list.child(div().text_color(palette.text_dim).child("No files yet."))
                })
                .children(rows),
        )
        .children(
            merge
                .error
                .clone()
                .map(|error| super::error_line(error, &palette)),
        )
        .child(
            super::button_row()
                .child(
                    accent_button("merge-go", "Merge", count >= 2, &palette)
                        .on_click(|_, window, cx| super::confirm(false, window, cx)),
                )
                .child(
                    framed_button("merge-add", "Add files…", true, &palette)
                        .on_click(|_, window, cx| add_files(window, cx)),
                )
                .child(
                    framed_button("merge-cancel", "Cancel", true, &palette)
                        .on_click(|_, window, cx| close(window, cx)),
                ),
        )
        .into_any_element()
}

/// A 22 × 22 move button of a file row (egui `small_icon_button`, disabled at the ends).
fn arrow(
    id: (&'static str, usize),
    glyph: &'static str,
    tip: &'static str,
    enabled: bool,
    palette: &Palette,
    swap: impl Fn(&mut Merge) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let button = crate::widgets::icon_widget(
        id,
        glyph,
        crate::widgets::SMALL_ICON_BUTTON,
        13.0,
        enabled,
        palette,
    );
    let button = crate::widgets::tooltip(button, tip);
    if !enabled {
        return button;
    }
    button.on_click(move |_, window, cx| {
        with_merge(cx, |merge| swap(merge));
        window.refresh();
    })
}
