//! Modal dialogs, the command palette (Ctrl+K) and the tool menus.
//!
//! An action opens a dialog: its handler (global, no window) records it as pending in the
//! [`Dialogs`] global; [`overlay`], drawn with the window, then creates its text fields and
//! focuses the first one. Confirming runs a `godpdf-core` edit on the document through
//! `DocTab::apply` (one undo step). Errors keep the dialog open, saying why, or are toasts.

mod attach;
mod commands;
mod form;
mod kinds;
mod merge;
mod output;
mod palette;
mod print;
mod ranges;
mod signpad;

use std::borrow::Cow;
use std::collections::HashMap;

use godpdf_core as core;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Div, Entity, EntityId, FocusHandle, Global, KeyBinding, MouseButton,
    WeakEntity, Window, actions, div, hsla, px,
};

use crate::app::GodPdfApp;
use crate::doc::{DocTab, guarded};
use crate::theme::Palette;
use crate::toasts;
pub use commands::{COMMANDS, Group};
use form::Form;
use kinds::Kind;
pub(crate) use kinds::open_tab;
use merge::Merge;
use palette::PaletteState;

actions!(
    godpdf,
    [
        CommandPalette,
        GoToPage,
        NumberPages,
        AddWatermark,
        CropPages,
        PageSize,
        PagesPerSheet,
        HalvePages,
        PageColours,
        ExportImages,
        ExtractPages,
        SplitDocument,
        SetPassword,
        RemovePassword,
        Properties,
        MergeFiles,
        Booklet,
        JoinPages,
        ReversePages,
        RotateLeft,
        RotateRight,
        RotateAll,
        DeletePage,
        InsertBlankPage,
        RemoveHiddenData,
        RemoveLinks,
        RemoveComments,
        Flatten,
        AttachedFiles,
        ContentsPage,
        Compress,
        Print,
        DialogConfirm,
        DialogCancel,
        PaletteUp,
        PaletteDown
    ]
);

/// Passwords set in the Protect dialog, applied when the document is saved.
pub struct Passwords {
    pub user: String,
    pub owner: String,
}

/// Opened by an action, built on the next frame (its text fields need the window).
enum Pending {
    Palette,
    Form(Kind),
}

pub enum Open {
    Palette(PaletteState),
    Form(Form),
    Merge(Merge),
    Signature(signpad::Pad),
    Attach(attach::Attach),
}

#[derive(Default)]
pub struct Dialogs {
    app: Option<WeakEntity<GodPdfApp>>,
    /// The document in front, as of the last frame.
    tab: Option<WeakEntity<DocTab>>,
    pending: Option<Pending>,
    open: Option<Open>,
    /// Focused before the dialog opened: focused again when it closes.
    return_focus: Option<FocusHandle>,
    passwords: HashMap<EntityId, Passwords>,
    printing: bool,
}

impl Global for Dialogs {}

fn state(cx: &mut App) -> &mut Dialogs {
    cx.default_global::<Dialogs>()
}

fn opened(cx: &App) -> Option<&Open> {
    cx.try_global::<Dialogs>()?.open.as_ref()
}

fn active_app(cx: &App) -> Option<Entity<GodPdfApp>> {
    cx.try_global::<Dialogs>()?.app.as_ref()?.upgrade()
}

fn active_tab(cx: &App) -> Option<Entity<DocTab>> {
    cx.try_global::<Dialogs>()?.tab.as_ref()?.upgrade()
}

/// Register dialog actions and their key bindings.
pub fn register(cx: &mut App) {
    cx.set_global(Dialogs::default());
    // In a text field, "Dialog > Input" outranks the field's own Enter / Esc / arrows.
    cx.bind_keys([
        KeyBinding::new(secondary!("k"), CommandPalette, None),
        KeyBinding::new(secondary!("p"), Print, None),
        KeyBinding::new(secondary!("g"), GoToPage, None),
        KeyBinding::new("enter", DialogConfirm, Some("GodPdfDialog > Input")),
        KeyBinding::new("escape", DialogCancel, Some("GodPdfDialog")),
        KeyBinding::new("escape", DialogCancel, Some("GodPdfDialog > Input")),
        KeyBinding::new("enter", DialogConfirm, Some("CommandPalette > Input")),
        KeyBinding::new("escape", DialogCancel, Some("CommandPalette > Input")),
        KeyBinding::new("up", PaletteUp, Some("CommandPalette > Input")),
        KeyBinding::new("down", PaletteDown, Some("CommandPalette > Input")),
    ]);
    cx.on_action(|_: &CommandPalette, cx| open_later(Pending::Palette, cx));
    cx.on_action(|_: &GoToPage, cx| open_form(Kind::GoToPage, cx));
    cx.on_action(|_: &NumberPages, cx| open_form(Kind::NumberPages, cx));
    cx.on_action(|_: &AddWatermark, cx| open_form(Kind::Watermark, cx));
    cx.on_action(|_: &CropPages, cx| open_form(Kind::Crop, cx));
    cx.on_action(|_: &PageSize, cx| open_form(Kind::PageSize, cx));
    cx.on_action(|_: &PagesPerSheet, cx| open_form(Kind::PerSheet, cx));
    cx.on_action(|_: &HalvePages, cx| open_form(Kind::Halve, cx));
    cx.on_action(|_: &PageColours, cx| open_form(Kind::Colours, cx));
    cx.on_action(|_: &ExportImages, cx| open_form(Kind::Images, cx));
    cx.on_action(|_: &ExtractPages, cx| open_form(Kind::Extract, cx));
    cx.on_action(|_: &SplitDocument, cx| open_form(Kind::Split, cx));
    cx.on_action(|_: &SetPassword, cx| open_form(Kind::Encrypt, cx));
    cx.on_action(|_: &Properties, cx| open_form(Kind::Properties, cx));
    cx.on_action(|_: &MergeFiles, cx| open_merge(cx));
    cx.on_action(|_: &AttachedFiles, cx| attach::open(cx));
    cx.on_action(|_: &RemovePassword, cx| remove_password(cx));
    cx.on_action(|_: &Print, cx| print_active(cx));
    cx.on_action(|_: &Compress, cx| compress(cx));
    cx.on_action(|_: &RotateLeft, cx| rotate_current(-90, cx));
    cx.on_action(|_: &RotateRight, cx| rotate_current(90, cx));
    cx.on_action(|_: &RotateAll, cx| {
        if let Some((tab, _, count)) = current(cx) {
            let pages: Vec<usize> = (0..count).collect();
            crate::organize::keeping_selection(&tab, cx, |cx| {
                edit_active(cx, "Rotate all pages", None, |pdf| {
                    core::rotate_pages(pdf, &pages, 90)
                });
            });
        }
    });
    // The pages selected in the Organize grid, else the current one.
    cx.on_action(|_: &DeletePage, cx| {
        if let Some((tab, _, _)) = current(cx) {
            let pages = crate::organize::target_pages(tab.read(cx));
            let label = format!("Delete {} page(s)", pages.len());
            edit_active(cx, &label, Some(&label), |pdf| {
                core::delete_pages(pdf, &pages)
            });
        }
    });
    cx.on_action(|_: &InsertBlankPage, cx| {
        if let Some((tab, _, count)) = current(cx) {
            let doc = tab.read(cx);
            let after = crate::organize::target_pages(doc)
                .into_iter()
                .max()
                .unwrap_or(0);
            let (width, height) = doc.page_sizes.get(after).copied().unwrap_or((595.0, 842.0));
            let at = (after + 1).min(count);
            edit_active(cx, "Insert blank page", None, |pdf| {
                core::insert_blank_page(pdf, at, width, height)
            });
        }
    });
    cx.on_action(|_: &ReversePages, cx| {
        if let Some((_, _, count)) = current(cx) {
            let order: Vec<usize> = (0..count).rev().collect();
            edit_active(
                cx,
                "Reverse page order",
                Some("Page order reversed"),
                |pdf| core::reorder_pages(pdf, &order),
            );
        }
    });
    cx.on_action(|_: &Booklet, cx| {
        let done = "Booklet made: print on both sides, turning on the short edge";
        edit_active(cx, "Make a booklet", Some(done), core::booklet);
    });
    cx.on_action(|_: &JoinPages, cx| {
        let done = "Pages joined into one";
        edit_active(cx, "Join into one long page", Some(done), core::join_pages);
    });
    cx.on_action(|_: &RemoveLinks, cx| {
        edit_active(
            cx,
            "Remove all links",
            Some("Links removed"),
            core::remove_links,
        );
    });
    cx.on_action(|_: &RemoveComments, cx| {
        let done = "Comments removed";
        edit_active(cx, "Remove all comments", Some(done), core::remove_comments);
    });
    cx.on_action(|_: &Flatten, cx| {
        let done = "Comments and form fields are now part of the pages";
        edit_active(
            cx,
            "Flatten comments and forms",
            Some(done),
            core::flatten_annotations,
        );
    });
    cx.on_action(|_: &RemoveHiddenData, cx| {
        let done = "Properties, scripts and attached files removed";
        edit_active(
            cx,
            "Remove hidden data",
            Some(done),
            core::remove_hidden_data,
        );
    });
    cx.on_action(|_: &ContentsPage, cx| {
        let done = "Contents page added: each line leads to its page";
        if edit_active(cx, "Contents page", Some(done), |pdf| {
            core::contents_page(pdf, "Contents")
        }) && let Some(tab) = active_tab(cx)
        {
            tab.update(cx, |tab, cx| {
                tab.scroll_to_page(0);
                cx.notify();
            });
        }
    });
}

/// A password was set for the document `tab`: it is encrypted when saved.
pub fn has_password(tab: EntityId, cx: &App) -> bool {
    cx.try_global::<Dialogs>()
        .is_some_and(|dialogs| dialogs.passwords.contains_key(&tab))
}

/// The document `tab` was opened with `password`: saving keeps it.
pub fn keep_password(tab: EntityId, password: String, cx: &mut App) {
    let passwords = Passwords {
        user: password.clone(),
        owner: password,
    };
    state(cx).passwords.insert(tab, passwords);
}

/// The bytes to write for the document `tab`: encrypted when a password was set for it.
pub fn bytes_to_save<'a>(
    tab: EntityId,
    bytes: &'a [u8],
    cx: &App,
) -> Result<Cow<'a, [u8]>, String> {
    let passwords = cx
        .try_global::<Dialogs>()
        .and_then(|dialogs| dialogs.passwords.get(&tab));
    match passwords {
        None => Ok(Cow::Borrowed(bytes)),
        Some(Passwords { user, owner }) => guarded(|| core::encrypt(bytes, user, owner))
            .map(Cow::Owned)
            .map_err(|err| format!("Encryption failed: {err}")),
    }
}

/// Drawn over the window when a dialog is open. `app` and `tab` (the document in front) are
/// kept for the action handlers, which have no view.
pub fn overlay(
    app: WeakEntity<GodPdfApp>,
    tab: Option<&Entity<DocTab>>,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    let dialogs = state(cx);
    dialogs.app = Some(app);
    dialogs.tab = tab.map(Entity::downgrade);
    if let Some(pending) = dialogs.pending.take() {
        build(pending, tab, window, cx);
    }
    let colors = Palette::of(cx);
    // The palette hangs 90 below the top, the other dialogs are centered.
    let centered = !matches!(opened(cx)?, Open::Palette(_));
    let card = match opened(cx)? {
        Open::Palette(palette) => palette::render(palette, tab.is_some(), &colors, window, cx),
        Open::Form(form) => form::render(form, &colors),
        Open::Merge(merge) => merge::render(merge, &colors),
        Open::Signature(pad) => signpad::render(pad, &colors),
        Open::Attach(attach) => attach::render(attach, &colors),
    };
    Some(
        div()
            .id("dialog-backdrop")
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .justify_center()
            .when(centered, |backdrop| backdrop.items_center())
            .when(!centered, |backdrop| backdrop.items_start().pt(px(90.)))
            // Backdrop: black at alpha 100.
            .bg(hsla(0., 0., 0., 100. / 255.))
            .on_mouse_down(MouseButton::Left, |_, window, cx| close(window, cx))
            .child(card)
            .into_any_element(),
    )
}

/// The card of a dialog, `width` of content wide: panel fill,
/// 1 px border, radius 9, margin 8, popup shadow, item spacing 6. A click in it does not
/// close it.
pub(crate) fn card(palette: &Palette, width: f32) -> Div {
    let margin = crate::theme::MENU_MARGIN;
    div()
        .flex()
        .flex_col()
        .gap(px(crate::theme::ITEM_SPACING.1))
        .w(px(width + 2.0 * margin + 2.0))
        .max_w_full()
        .p(px(margin))
        .rounded(px(crate::theme::MENU_RADIUS))
        .shadow(crate::widgets::popup_shadow(palette))
        .bg(palette.panel)
        .border_1()
        .border_color(palette.border)
        .text_size(px(crate::fonts::BODY))
        .text_color(palette.text)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

fn open_later(pending: Pending, cx: &mut App) {
    state(cx).pending = Some(pending);
    cx.refresh_windows();
}

fn open_form(kind: Kind, cx: &mut App) {
    if active_tab(cx).is_none() {
        toasts::info(cx, "Open a PDF first");
        return;
    }
    open_later(Pending::Form(kind), cx);
}

/// The signature pad; `place` is where the signature goes once drawn.
pub fn draw_signature(place: Option<signpad::Place>, cx: &mut App) {
    signpad::open(place, cx);
}

/// Ask kind and name of a form field to add over `rect` (PDF space) of `page`.
pub fn new_field(page: usize, rect: core::Rect, cx: &mut App) {
    open_form(Kind::NewField { page, rect }, cx);
}

/// The document in front first, then the files picked.
fn open_merge(cx: &mut App) {
    let mut files: Vec<crate::files::PickedFile> = active_tab(cx)
        .map(|tab| {
            let doc = tab.read(cx);
            crate::files::PickedFile {
                name: doc.title().to_owned(),
                path: doc.session.path.clone().unwrap_or_default(),
                bytes: doc.session.bytes().to_vec(),
            }
        })
        .into_iter()
        .collect();
    if files.is_empty() {
        files = merge::pick(cx);
        if files.is_empty() {
            return;
        }
    }
    state(cx).open = Some(Open::Merge(Merge { files, error: None }));
    cx.refresh_windows();
}

/// Create the dialog `pending` and focus its first field.
fn build(pending: Pending, tab: Option<&Entity<DocTab>>, window: &mut Window, cx: &mut App) {
    let open = match pending {
        Pending::Palette => Open::Palette(palette::new(window, cx)),
        Pending::Form(kind) => {
            let Some(tab) = tab else {
                toasts::info(cx, "Open a PDF first");
                return;
            };
            match kinds::build(kind, tab, window, cx) {
                Ok(form) => Open::Form(form),
                Err(err) => {
                    toasts::error(cx, err);
                    return;
                }
            }
        }
    };
    let input = match &open {
        Open::Palette(palette) => Some(palette.query.clone()),
        Open::Form(form) => form.first_input(),
        Open::Merge(_) | Open::Signature(_) | Open::Attach(_) => None,
    };
    let focused = window.focused(cx);
    let dialogs = state(cx);
    if dialogs.open.is_none() {
        dialogs.return_focus = focused;
    }
    dialogs.open = Some(open);
    if let Some(input) = input {
        input.update(cx, |input, cx| input.focus(window, cx));
    }
}

fn close(window: &mut Window, cx: &mut App) {
    let dialogs = state(cx);
    dialogs.open = None;
    if let Some(focus) = dialogs.return_focus.take() {
        window.focus(&focus, cx);
    }
    window.refresh();
}

/// The main (or `secondary`) button of the open dialog.
fn confirm(secondary: bool, window: &mut Window, cx: &mut App) {
    let Some(mut open) = state(cx).open.take() else {
        return;
    };
    let result = match &open {
        Open::Form(form) => kinds::confirm(form, secondary, cx),
        Open::Merge(merge) => merge::confirm(merge, cx),
        Open::Palette(_) | Open::Attach(_) => Ok(()),
        Open::Signature(pad) => signpad::confirm_pad(pad, window, cx),
    };
    match (result, &mut open) {
        (Ok(()), _) => close(window, cx),
        (Err(error), Open::Form(Form { error: shown, .. }))
        | (Err(error), Open::Merge(Merge { error: shown, .. })) => {
            *shown = Some(error);
            state(cx).open = Some(open);
            window.refresh();
        }
        (Err(error), Open::Signature(_)) => {
            toasts::info(cx, error);
            state(cx).open = Some(open);
            window.refresh();
        }
        (Err(error), Open::Palette(_) | Open::Attach(_)) => toasts::error(cx, error),
    }
}

/// The document in front, its current page and its page count.
fn current(cx: &mut App) -> Option<(Entity<DocTab>, usize, usize)> {
    let Some(tab) = active_tab(cx) else {
        toasts::info(cx, "Open a PDF first");
        return None;
    };
    let doc = tab.read(cx);
    let (page, count) = (doc.view.current_page, doc.page_count());
    Some((tab, page, count))
}

/// One undo step on the document in front, then say how it went. True when done.
fn edit_active(
    cx: &mut App,
    label: &str,
    done: Option<&str>,
    edit: impl FnOnce(&[u8]) -> core::Result<Vec<u8>>,
) -> bool {
    let Some(tab) = active_tab(cx) else {
        toasts::info(cx, "Open a PDF first");
        return false;
    };
    let result = tab.update(cx, |tab, cx| {
        let result = tab.apply(label, edit);
        cx.notify();
        result
    });
    match result {
        Ok(()) => {
            if let Some(done) = done {
                toasts::success(cx, done.to_owned());
            }
            true
        }
        Err(err) => {
            toasts::error(cx, plain(err));
            false
        }
    }
}

fn rotate_current(degrees: i32, cx: &mut App) {
    if let Some((tab, _, _)) = current(cx) {
        let label = if degrees < 0 {
            "Rotate left"
        } else {
            "Rotate right"
        };
        let pages = crate::organize::target_pages(tab.read(cx));
        crate::organize::keeping_selection(&tab, cx, |cx| {
            edit_active(cx, label, None, |pdf| {
                core::rotate_pages(pdf, &pages, degrees)
            });
        });
    }
}

fn compress(cx: &mut App) {
    let Some(tab) = active_tab(cx) else {
        toasts::info(cx, "Open a PDF first");
        return;
    };
    let before = tab.read(cx).session.bytes().len();
    if edit_active(cx, "Compress", None, core::compress) {
        let after = tab.read(cx).session.bytes().len();
        let saved = 100.0 - after as f64 * 100.0 / before.max(1) as f64;
        toasts::success(
            cx,
            format!(
                "{} → {} ({saved:.0} % smaller)",
                size_text(before),
                size_text(after)
            ),
        );
    }
}

fn remove_password(cx: &mut App) {
    let Some(tab) = active_tab(cx) else {
        toasts::info(cx, "Open a PDF first");
        return;
    };
    state(cx).passwords.remove(&tab.entity_id());
    toasts::info(cx, "The document will be saved without a password");
}

/// Print the document in front as it is now (unsaved changes included), in the background.
fn print_active(cx: &mut App) {
    let Some(tab) = active_tab(cx) else {
        toasts::info(cx, "Open a PDF first");
        return;
    };
    if std::mem::replace(&mut state(cx).printing, true) {
        toasts::error(cx, "Still printing: wait for it to be over");
        return;
    }
    let doc = tab.read(cx);
    let (name, bytes) = (doc.title().to_owned(), doc.session.bytes().to_vec());
    let printed = crate::services::jobs::background(cx, async move { print::print(&name, bytes) });
    cx.spawn(async move |cx| {
        let result = printed.await;
        cx.update(|cx| {
            state(cx).printing = false;
            match result {
                Ok(Some(done)) => toasts::success(cx, done),
                Ok(None) => {}
                Err(err) => toasts::error(cx, err),
            }
        });
    })
    .detach();
}

/// `1.2 MB`, `340 KB`, `12 bytes`.
fn size_text(bytes: usize) -> String {
    match bytes {
        0..1_000 => format!("{bytes} bytes"),
        1_000..1_000_000 => format!("{:.0} KB", bytes as f64 / 1e3),
        _ => format!("{:.1} MB", bytes as f64 / 1e6),
    }
}

/// An error of the core as a sentence: without the kind of error in front.
fn plain(error: String) -> String {
    let text = error.trim_start_matches("invalid argument: ");
    let mut letters = text.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_well() {
        assert_eq!(size_text(12), "12 bytes");
        assert_eq!(size_text(340_000), "340 KB");
        assert_eq!(size_text(1_200_000), "1.2 MB");
    }

    #[test]
    fn core_errors_become_sentences() {
        assert_eq!(plain("invalid argument: no pages".into()), "No pages");
        assert_eq!(plain("bad file".into()), "Bad file");
        assert_eq!(plain(String::new()), "");
    }
}

/// Dialog title: 19 px (strong is only a colour), then 8 of space.
pub(crate) fn title(text: impl IntoElement) -> Div {
    div().pb(px(2.)).text_size(px(19.)).child(text)
}

/// Error line: warning glyph and text in the error colour.
pub(crate) fn error_line(error: impl Into<gpui::SharedString>, palette: &Palette) -> Div {
    crate::widgets::with_icon(
        crate::icons::WARNING_CIRCLE,
        error,
        crate::fonts::BODY,
        palette.error,
    )
}

/// Button row: 12 of space above, item spacing 8 between.
pub(crate) fn button_row() -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(crate::theme::ITEM_SPACING.0))
        .pt(px(12.))
}
