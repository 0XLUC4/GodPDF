//! Organize: the page grid (`grid.rs`, egui `organize.rs`), the page commands that act on
//! its selection (move earlier / later, insert from file, remove blank pages, interleave),
//! and "Side by side" (`split.rs`, egui `GodApp::companion`).
//!
//! App-wide state is the [`Organize`] global (the document in front, the Interleave dialog,
//! the second viewer). Per-document state is [`Grid`], a field of `DocTab`.

mod grid;
mod order;
mod split;

use std::collections::BTreeSet;
use std::sync::Arc;

use godpdf_core as core;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Entity, Global, KeyBinding, MouseButton, WeakEntity, Window, actions, div,
    hsla, px,
};

use crate::app::GodPdfApp;
use crate::doc::{DocTab, guarded};
use crate::files::PickedFile;
use crate::shell::Mode;
use crate::theme::Palette;
use crate::{import, panels, toasts};

actions!(
    godpdf,
    [
        ToggleOrganize,
        MoveEarlier,
        MoveLater,
        InsertFile,
        RemoveBlank,
        Interleave,
        SplitView
    ]
);

/// The Organize grid of one document.
#[derive(Default)]
pub struct Grid {
    /// The grid shows instead of the viewer (egui `ViewMode::Organize`).
    pub on: bool,
    /// Selected pages; cleared by every reload (`DocTab::reload`).
    pub selection: BTreeSet<usize>,
    /// Page a Shift+click selects from.
    anchor: Option<usize>,
    thumbs: panels::Thumbs,
    /// Width of the grid as of the last paint, for the cards per row.
    width: f32,
}

/// A PDF picked to interleave with the document `tab`.
struct InterleaveDialog {
    tab: WeakEntity<DocTab>,
    file: PickedFile,
    reverse: bool,
}

#[derive(Default)]
struct Organize {
    /// The document in front, as of the last frame (actions have no view at hand).
    front: Option<WeakEntity<DocTab>>,
    interleave: Option<InterleaveDialog>,
    split: Option<split::Split>,
}

impl Global for Organize {}

fn state(cx: &mut App) -> &mut Organize {
    cx.default_global::<Organize>()
}

/// The document in front, or a toast saying to open one.
fn front(cx: &mut App) -> Option<Entity<DocTab>> {
    let tab = state(cx).front.as_ref().and_then(WeakEntity::upgrade);
    if tab.is_none() {
        toasts::info(cx, "Open a PDF first");
    }
    tab
}

/// Alt+Left / Alt+Right move the pages (egui `Cmd::MoveEarlier` / `MoveLater`), never while
/// typing.
pub fn register(cx: &mut App) {
    cx.set_global(Organize::default());
    let keys = Some("GodPdf && !Input");
    cx.bind_keys([
        KeyBinding::new("alt-left", MoveEarlier, keys),
        KeyBinding::new("alt-right", MoveLater, keys),
    ]);
    cx.on_action(|_: &ToggleOrganize, cx| {
        if let Some(tab) = front(cx) {
            tab.update(cx, |tab, cx| {
                tab.grid.on = !tab.grid.on;
                cx.notify();
            });
        }
    });
    cx.on_action(|_: &MoveEarlier, cx| move_target(-1, cx));
    cx.on_action(|_: &MoveLater, cx| move_target(1, cx));
    cx.on_action(|_: &InsertFile, cx| {
        if let Some(tab) = front(cx) {
            let doc = tab.read(cx);
            let at = target_pages(doc)
                .into_iter()
                .max()
                .map_or(doc.page_count(), |page| page + 1);
            insert_files(&tab.downgrade(), at, cx);
        }
    });
    cx.on_action(|_: &RemoveBlank, cx| remove_blank(cx));
    cx.on_action(|_: &Interleave, cx| {
        let Some(tab) = front(cx) else { return };
        // shortcut: the first file picked is used, egui's dialog picks only one.
        if let Some(file) = import::pick_as_pdfs(cx).into_iter().next() {
            state(cx).interleave = Some(InterleaveDialog {
                tab: tab.downgrade(),
                file,
                reverse: false,
            });
            cx.refresh_windows();
        }
    });
    cx.on_action(|_: &SplitView, cx| split::toggle(cx));
}

/// Pages an organize command acts on: the selection in the grid, else the current page.
pub fn target_pages(tab: &DocTab) -> Vec<usize> {
    if tab.grid.on && !tab.grid.selection.is_empty() {
        tab.grid.selection.iter().copied().collect()
    } else if tab.page_count() > 0 {
        vec![tab.view.current_page]
    } else {
        Vec::new()
    }
}

/// The pages a page tool is offered for: those selected in the grid (more than one), or all.
pub fn offered_pages(tab: &DocTab) -> Vec<usize> {
    if tab.grid.selection.len() > 1 {
        tab.grid.selection.iter().copied().collect()
    } else {
        (0..tab.page_count()).collect()
    }
}

/// Run `edit` on `tab` (an edit that keeps the pages where they are, e.g. a rotation), then
/// select again what was selected.
pub fn keeping_selection(tab: &Entity<DocTab>, cx: &mut App, edit: impl FnOnce(&mut App)) {
    let selection = tab.read(cx).grid.selection.clone();
    let generation = tab.read(cx).generation();
    edit(cx);
    tab.update(cx, |tab, _| {
        if tab.generation() != generation {
            tab.grid.selection = selection;
        }
    });
}

/// Header mode follows the grid of the document in front (egui `sync_mode`): the grid can
/// also be left from inside it.
pub fn sync_mode(mode: &mut Mode, tab: &Entity<DocTab>, cx: &App) {
    if tab.read(cx).grid.on {
        *mode = Mode::Organize;
    } else if *mode == Mode::Organize {
        *mode = Mode::AllTools;
    }
}

/// The header switched to (or away from) Organize: show the grid or the viewer.
pub fn set_grid(tab: &Entity<DocTab>, on: bool, cx: &mut App) {
    tab.update(cx, |tab, cx| {
        if tab.grid.on != on {
            tab.grid.on = on;
            cx.notify();
        }
    });
}

/// What fills the middle of the window: the grid, the viewer, or two viewers side by side.
pub fn center(
    app: &GodPdfApp,
    tab: &Entity<DocTab>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    if tab.read(cx).grid.on {
        return grid::render(tab, window, cx);
    }
    match split::render(app, tab, window, cx) {
        Some(second) => div()
            .size_full()
            .flex()
            .child(div().flex_1().min_w_0().h_full().child(tab.clone()))
            .child(second)
            .into_any_element(),
        None => tab.clone().into_any_element(),
    }
}

/// Move the target pages one place, keeping them selected (egui `move_pages`).
fn move_target(delta: isize, cx: &mut App) {
    if let Some(tab) = front(cx) {
        let pages = target_pages(tab.read(cx));
        move_pages(&tab, &pages, delta, cx);
    }
}

fn move_pages(tab: &Entity<DocTab>, pages: &[usize], delta: isize, cx: &mut App) {
    let order = order::move_selection(tab.read(cx).page_count(), pages, delta);
    if order.iter().enumerate().all(|(index, &page)| index == page) {
        return;
    }
    let moved = order::new_places(&order, pages);
    let label = if delta < 0 {
        "Move pages earlier"
    } else {
        "Move pages later"
    };
    apply_order(tab, label, &order, moved, cx);
}

/// Reorder the pages of `tab`, then select `moved` (their new places) and, out of the grid,
/// show the first one.
fn apply_order(
    tab: &Entity<DocTab>,
    label: &str,
    order: &[usize],
    moved: Vec<usize>,
    cx: &mut App,
) {
    let result = tab.update(cx, |tab, cx| {
        let result = tab.apply(label, |pdf| core::reorder_pages(pdf, order));
        if result.is_ok() {
            if let Some(&first) = moved.first()
                && !tab.grid.on
            {
                tab.scroll_to_page(first);
            }
            tab.grid.selection = moved.into_iter().collect();
        }
        cx.notify();
        result
    });
    if let Err(err) = result {
        toasts::error(cx, err);
    }
}

/// Ask for files and put their pages at `at` in `tab`, each its own undo step (egui
/// `Purpose::Insert`). Pictures and text become pages too.
pub fn insert_files(tab: &WeakEntity<DocTab>, at: usize, cx: &mut App) {
    for file in import::pick_as_pdfs(cx) {
        let mut bytes = file.bytes;
        if core::is_encrypted(&bytes) {
            // Owner-password-only files open with an empty password.
            match guarded(|| core::decrypt(&bytes, "")) {
                Ok(plain) => bytes = plain,
                Err(_) => {
                    toasts::error(
                        cx,
                        format!(
                            "“{}” is password-protected; open it and remove the password first",
                            file.name
                        ),
                    );
                    continue;
                }
            }
        }
        let label = format!("Insert {}", file.name);
        let result = tab.update(cx, |tab, cx| {
            let result = tab.apply(&label, |pdf| core::insert_pdf(pdf, at, &bytes));
            cx.notify();
            result
        });
        match result {
            Ok(Ok(())) => toasts::success(cx, format!("Inserted “{}”", file.name)),
            Ok(Err(err)) => toasts::error(cx, err),
            // The tab was closed meanwhile.
            Err(_) => return,
        }
    }
}

/// Find the empty pages in the background, then delete them (egui `Cmd::RemoveBlank`).
fn remove_blank(cx: &mut App) {
    let Some(tab) = front(cx) else { return };
    let doc = tab.read(cx);
    let (generation, bytes) = (doc.generation(), doc.session.bytes().to_vec());
    toasts::info(cx, "Looking for blank pages…");
    let found = crate::services::jobs::background(cx, async move { blank_pages(bytes) });
    let tab = tab.downgrade();
    cx.spawn(async move |cx| {
        let result = found.await;
        cx.update(|cx| {
            let Some(tab) = tab.upgrade() else { return };
            match result {
                Ok(_) if tab.read(cx).generation() != generation => {
                    toasts::info(cx, "The document changed meanwhile: look again");
                }
                Ok(pages) if pages.is_empty() => toasts::info(cx, "No blank page found"),
                Ok(pages) => {
                    let result = tab.update(cx, |tab, cx| {
                        let result =
                            tab.apply("Remove blank pages", |pdf| core::delete_pages(pdf, &pages));
                        cx.notify();
                        result
                    });
                    match result {
                        Ok(()) if pages.len() == 1 => {
                            toasts::success(cx, format!("Removed blank page {}", pages[0] + 1))
                        }
                        Ok(()) => {
                            toasts::success(cx, format!("Removed {} blank pages", pages.len()))
                        }
                        Err(err) => toasts::error(cx, err),
                    }
                }
                Err(err) => toasts::error(cx, err),
            }
        });
    })
    .detach();
}

/// Pages with next to no ink (egui `pagetools::blank_pages`): fewer than 1 pixel in 500 of
/// a small rendering darker than light grey.
fn blank_pages(pdf: Vec<u8>) -> Result<Vec<usize>, String> {
    let doc = godpdf_render::Document::open(Arc::new(pdf), None).map_err(|err| err.to_string())?;
    let mut blank = Vec::new();
    for page in 0..doc.page_count() {
        let bitmap = doc.thumbnail(page, 300).map_err(|err| err.to_string())?;
        if is_blank(&bitmap.rgba) {
            blank.push(page);
        }
    }
    Ok(blank)
}

fn is_blank(rgba: &[u8]) -> bool {
    let pixels = rgba.as_chunks::<4>().0.iter();
    let total = pixels.len().max(1);
    let inked = pixels
        .filter(|pixel| pixel[0].min(pixel[1]).min(pixel[2]) < 225)
        .count();
    inked * 500 < total
}

/// The Interleave dialog over the window (egui `Dialog::Interleave`), `None` when closed.
/// Also remembers the document in front for the actions.
pub fn overlay(front: Option<&Entity<DocTab>>, _: &mut Window, cx: &mut App) -> Option<AnyElement> {
    let organize = state(cx);
    organize.front = front.map(Entity::downgrade);
    let dialog = organize.interleave.as_ref()?;
    let (name, reverse) = (dialog.file.name.clone(), dialog.reverse);
    let palette = Palette::of(cx);
    let card = crate::dialogs::card(&palette, 360.)
        .child(crate::dialogs::title(crate::widgets::with_icon(
            crate::icons::SHUFFLE,
            "Interleave pages",
            19.0,
            palette.text,
        )))
        .child(format!(
            "The pages of this document and of “{name}”, taken in turn."
        ))
        .child(
            div().pt(px(6.)).child(crate::widgets::tooltip(
                div().id("interleave-reverse").child(
                    gpui::component::checkbox::Checkbox::new("interleave-reverse-box")
                        .label("Read the second one from its last page")
                        .checked(reverse)
                        .on_click(|checked: &bool, _, cx: &mut App| {
                            if let Some(dialog) = &mut state(cx).interleave {
                                dialog.reverse = *checked;
                            }
                            cx.refresh_windows();
                        }),
                ),
                "For the backs of sheets scanned as a pile, which come out in reverse",
            )),
        )
        .child(
            crate::dialogs::button_row()
                .child(
                    crate::widgets::accent_button("interleave-go", "Interleave", true, &palette)
                        .on_click(|_, _, cx| interleave(cx)),
                )
                .child(
                    crate::widgets::framed_button("interleave-cancel", "Cancel", true, &palette)
                        .on_click(|_, _, cx| close_dialog(cx)),
                ),
        );
    Some(
        div()
            .id("interleave-backdrop")
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .justify_center()
            .items_center()
            // egui `Modal` backdrop: black at alpha 100.
            .bg(hsla(0., 0., 0., 100. / 255.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| close_dialog(cx))
            .child(card)
            .into_any_element(),
    )
}

fn close_dialog(cx: &mut App) {
    state(cx).interleave = None;
    cx.refresh_windows();
}

/// The pages of the document and of the picked file in turn, into a new tab.
fn interleave(cx: &mut App) {
    let Some(dialog) = state(cx).interleave.take() else {
        return;
    };
    cx.refresh_windows();
    let Some(tab) = dialog.tab.upgrade() else {
        return;
    };
    let doc = tab.read(cx);
    let name = format!(
        "{} + {} (interleaved).pdf",
        stem(doc.title()),
        stem(&dialog.file.name)
    );
    let made =
        guarded(|| core::interleave(doc.session.bytes(), &dialog.file.bytes, dialog.reverse));
    match made {
        Ok(bytes) => {
            crate::dialogs::open_tab(name, bytes, cx);
            toasts::success(cx, "Pages interleaved");
        }
        Err(err) => toasts::error(cx, format!("Cannot interleave: {err}")),
    }
}

/// A file name without its `.pdf`.
fn stem(name: &str) -> &str {
    name.strip_suffix(".pdf")
        .or_else(|| name.strip_suffix(".PDF"))
        .unwrap_or(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_with_a_little_ink_is_blank() {
        let white = [255u8; 4 * 1000];
        assert!(is_blank(&white));
        let mut inked = white;
        for pixel in inked.chunks_exact_mut(4).take(3) {
            pixel[..3].copy_from_slice(&[0, 0, 0]);
        }
        assert!(!is_blank(&inked));
        assert_eq!(stem("scan.pdf"), "scan");
        assert_eq!(stem("notes"), "notes");
    }
}
