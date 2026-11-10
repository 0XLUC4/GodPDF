//! Combine files: a list of PDFs to put one after the other into a new tab (ported from
//! `merge` in `crates/app/src/dialogs.rs`).

use gpui::prelude::*;
use gpui::{AnyElement, App, FontWeight, Window, div, px};

use super::kinds::open_tab;
use super::{DialogCancel, DialogConfirm, Open, card, close, state};
use crate::doc::guarded;
use crate::files::{self, PickedFile};
use crate::theme::Palette;
use crate::toasts;
use crate::widgets::{button, primary_button};

pub struct Merge {
    pub files: Vec<PickedFile>,
    pub error: Option<String>,
}

/// Ask for PDFs and read them; a file that cannot be read is said and left out.
pub fn pick(cx: &mut App) -> Vec<PickedFile> {
    files::pick_pdfs()
        .iter()
        .filter_map(|path| match files::read_file(path) {
            Ok(file) => Some(file),
            Err(err) => {
                toasts::error(cx, err);
                None
            }
        })
        .collect()
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
                    .w(px(24.))
                    .text_sm()
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
            .when(index > 0, |row| {
                row.child(button(("merge-up", index), "↑", &palette).on_click(
                    move |_, window, cx| {
                        with_merge(cx, |merge| merge.files.swap(index, index - 1));
                        window.refresh();
                    },
                ))
            })
            .when(index + 1 < count, |row| {
                row.child(button(("merge-down", index), "↓", &palette).on_click(
                    move |_, window, cx| {
                        with_merge(cx, |merge| merge.files.swap(index, index + 1));
                        window.refresh();
                    },
                ))
            })
            .child(
                button(("merge-remove", index), "×", &palette).on_click(move |_, window, cx| {
                    with_merge(cx, |merge| {
                        merge.files.remove(index);
                    });
                    window.refresh();
                }),
            )
    });
    card(&palette, 480.)
        .key_context("GodPdfDialog")
        .on_action(|_: &DialogConfirm, window, cx| super::confirm(false, window, cx))
        .on_action(|_: &DialogCancel, window, cx| close(window, cx))
        .child(
            div()
                .text_size(px(19.))
                .font_weight(FontWeight::BOLD)
                .child("Combine files"),
        )
        .child(
            div()
                .text_sm()
                .text_color(palette.text_dim)
                .child("PDFs are combined top to bottom, into a new tab."),
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
        .children(merge.error.clone().map(|error| {
            div()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .text_color(palette.error)
                .child(format!("⚠ {error}"))
        }))
        .child(
            div()
                .flex()
                .gap(px(8.))
                .pt(px(8.))
                .child(
                    primary_button("merge-go", "Merge", &palette)
                        .when(count < 2, |button| button.opacity(0.4))
                        .on_click(|_, window, cx| super::confirm(false, window, cx)),
                )
                .child(
                    button("merge-add", "Add files…", &palette)
                        .h(px(38.))
                        .on_click(|_, window, cx| add_files(window, cx)),
                )
                .child(
                    button("merge-cancel", "Cancel", &palette)
                        .h(px(38.))
                        .on_click(|_, window, cx| close(window, cx)),
                ),
        )
        .into_any_element()
}
