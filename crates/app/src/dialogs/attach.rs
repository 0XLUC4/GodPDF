//! Attached files: the files carried inside the PDF, to add, save or remove.

use godpdf_core::{self as core, Attachment};
use gpui::prelude::*;
use gpui::{AnyElement, App, Entity, WeakEntity, div, px};

use super::{DialogCancel, Open, active_tab, card, close, plain, size_text, state};
use crate::doc::{DocTab, guarded};
use crate::theme::Palette;
use crate::widgets::{self, framed_button, small_icon_button, with_icon};
use crate::{files, fonts, icons, toasts};

pub struct Attach {
    tab: WeakEntity<DocTab>,
    files: Vec<Attachment>,
    /// Shown in the dialog: the error bar would sit dimmed under its backdrop.
    error: Option<String>,
}

/// The attached files of `tab`, read again (after an edit, or to open the dialog).
fn read(tab: &Entity<DocTab>, cx: &App) -> Result<Vec<Attachment>, String> {
    guarded(|| core::attachments(tab.read(cx).session.bytes()))
        .map_err(|err| format!("Cannot read attached files: {err}"))
}

/// Open the dialog on the document in front.
pub fn open(cx: &mut App) {
    let Some(tab) = active_tab(cx) else {
        toasts::info(cx, "Open a PDF first");
        return;
    };
    match read(&tab, cx) {
        Ok(files) => {
            state(cx).open = Some(Open::Attach(Attach {
                tab: tab.downgrade(),
                files,
                error: None,
            }));
            cx.refresh_windows();
        }
        Err(err) => toasts::error(cx, err),
    }
}

/// Show the files of the dialog's document as they are now.
fn refresh(tab: &Entity<DocTab>, cx: &mut App) {
    match read(tab, cx) {
        Ok(files) => {
            if let Some(Open::Attach(attach)) = &mut state(cx).open {
                attach.files = files;
            }
        }
        Err(err) => report(err, cx),
    }
    cx.refresh_windows();
}

/// Show `error` in the dialog, or in the error bar once it is closed.
fn report(error: String, cx: &mut App) {
    match &mut state(cx).open {
        Some(Open::Attach(attach)) => attach.error = Some(error),
        _ => toasts::error(cx, error),
    }
    cx.refresh_windows();
}

/// A new action starts without the error of the last one.
fn clear_error(cx: &mut App) {
    if let Some(Open::Attach(attach)) = &mut state(cx).open {
        attach.error = None;
    }
}

fn dialog_tab(cx: &App) -> Option<Entity<DocTab>> {
    match &cx.try_global::<super::Dialogs>()?.open {
        Some(Open::Attach(attach)) => attach.tab.upgrade(),
        _ => None,
    }
}

/// "Attach files…": any files, each one undo step.
fn add(cx: &mut App) {
    let Some(tab) = dialog_tab(cx) else { return };
    clear_error(cx);
    files::pick_any(cx, move |picked, cx| attach_paths(&tab, picked, cx));
}

fn attach_paths(tab: &Entity<DocTab>, picked: Vec<std::path::PathBuf>, cx: &mut App) {
    for path in picked {
        let file = match files::read_file(&path) {
            Ok(file) => file,
            Err(err) => {
                report(err, cx);
                continue;
            }
        };
        let result = tab.update(cx, |tab, cx| {
            let result = tab.apply(&format!("Attach {}", file.name), |pdf| {
                core::add_attachment(pdf, &file.name, &file.bytes)
            });
            cx.notify();
            result
        });
        if let Err(err) = result {
            report(plain(err), cx);
        }
    }
    refresh(tab, cx);
}

/// "Save a copy" of the file at `index`, where the user picks.
fn save(index: usize, cx: &mut App) {
    let Some(Open::Attach(attach)) = cx
        .try_global::<super::Dialogs>()
        .and_then(|dialogs| dialogs.open.as_ref())
    else {
        return;
    };
    let Some(file) = attach.files.get(index) else {
        return;
    };
    let (name, bytes) = (file.name.clone(), file.bytes.clone());
    clear_error(cx);
    files::pick_save(
        cx,
        &name.clone(),
        false,
        move |target, cx| match files::write_atomic(&target, &bytes) {
            Ok(()) => toasts::success(cx, format!("Saved “{name}”")),
            Err(err) => report(err, cx),
        },
    );
}

fn remove(index: usize, cx: &mut App) {
    let Some(tab) = dialog_tab(cx) else { return };
    let name = match &state(cx).open {
        Some(Open::Attach(attach)) => match attach.files.get(index) {
            Some(file) => file.name.clone(),
            None => return,
        },
        _ => return,
    };
    clear_error(cx);
    let result = tab.update(cx, |tab, cx| {
        let result = tab.apply(&format!("Remove {name}"), |pdf| {
            core::remove_attachment(pdf, &name)
        });
        cx.notify();
        result
    });
    if let Err(err) = result {
        report(plain(err), cx);
    }
    refresh(&tab, cx);
}

pub fn render(attach: &Attach, palette: &Palette) -> AnyElement {
    let palette = *palette;
    let rows = attach.files.iter().enumerate().map(|(index, file)| {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(widgets::icon(icons::FILE, fonts::BODY, palette.text))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(file.name.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .text_color(palette.text_dim)
                    .child(size_text(file.bytes.len())),
            )
            // Right to left: Remove, then Save a copy.
            .child(
                small_icon_button(
                    ("attach-save", index),
                    icons::DOWNLOAD_SIMPLE,
                    "Save a copy",
                    &palette,
                )
                .on_click(move |_, _, cx| save(index, cx)),
            )
            .child(
                small_icon_button(("attach-remove", index), icons::TRASH, "Remove", &palette)
                    .on_click(move |_, _, cx| remove(index, cx)),
            )
    });
    card(&palette, 420.)
        .key_context("GodPdfDialog")
        .on_action(|_: &DialogCancel, window, cx| close(window, cx))
        .child(super::title(with_icon(
            icons::PAPERCLIP,
            "Attached files",
            19.0,
            palette.text,
        )))
        .when(attach.files.is_empty(), |card| {
            card.child(
                div()
                    .text_color(palette.text_dim)
                    .child("No file is attached to this document."),
            )
        })
        .child(
            div()
                .id("attached-files")
                .flex()
                .flex_col()
                .gap(px(4.))
                .max_h(px(280.))
                .overflow_y_scroll()
                .children(rows),
        )
        .children(
            attach
                .error
                .clone()
                .map(|error| super::error_line(error, &palette)),
        )
        .child(
            super::button_row()
                .pt(px(8.))
                .child(
                    crate::widgets::accent_button(
                        "attach-add",
                        with_icon(icons::PLUS, "Attach files…", fonts::BODY, palette.on_accent),
                        true,
                        &palette,
                    )
                    .on_click(|_, _, cx| add(cx)),
                )
                .child(
                    framed_button("attach-close", "Close", true, &palette)
                        .on_click(|_, window, cx| close(window, cx)),
                ),
        )
        .into_any_element()
}
