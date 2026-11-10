//! The text of the pages: select and copy it with the Select tool (Ctrl+C), "Edit PDF text" line by
//! line on the page, "Extract text", "Recognize text (OCR)" and "Compare with…".
//!
//! App-wide state is the [`TextState`] global (edit-text mode, the open windows, the document
//! in front); each document keeps its [`TabText`] (text positions, selection, line typed).
//! Hooks: [`page_underlay`] (under the tools of a page), [`page_overlay`] (over them, in
//! edit-text mode) and [`overlay`] (the windows, drawn by the app).

mod compare;
mod lineedit;
mod ocr;
mod sel;
mod view;

pub(crate) use lineedit::reconcile;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use godpdf_render::TextChar;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClipboardItem, Context, CursorStyle, Entity, Global, Hsla, KeyBinding,
    MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, WeakEntity, Window, actions, div,
    px,
};

use crate::doc::{DocTab, guarded};
use crate::theme::Palette;
use crate::toasts;
use crate::tools::{Tool, ToolState};
use sel::TextSel;

actions!(
    godpdf,
    [
        EditText,
        ExtractText,
        RecognizeText,
        CompareWith,
        CopyText,
        SelectAllText,
        EscapeText
    ]
);

/// Same context as the single-key tool shortcuts: never while typing in a field.
const KEYS: &str = "GodPdf && !Input && !GodPdfDialog";
/// Fill of selected page text: the active blue, translucent.
const SELECTION_ALPHA: f32 = 0.3;

#[derive(Default)]
pub struct TextState {
    /// "Edit PDF text": a click on a line of the page opens it for typing.
    edit_text: bool,
    /// The tool when edit-text mode began: picking another ends it.
    tool: Tool,
    /// The document in front, as of the last frame (actions have no view).
    tab: Option<WeakEntity<DocTab>>,
    windows: view::Windows,
}

impl Global for TextState {}

fn state(cx: &mut App) -> &mut TextState {
    cx.default_global::<TextState>()
}

fn editing_text(cx: &App) -> bool {
    cx.try_global::<TextState>()
        .is_some_and(|state| state.edit_text)
}

fn tool(cx: &App) -> Tool {
    cx.try_global::<ToolState>()
        .map_or(Tool::Select, |state| state.tool)
}

/// The text of one document as the tools see it.
#[derive(Default)]
pub struct TabText {
    /// Document generation `chars` and `sel` belong to.
    generation: u64,
    chars: HashMap<usize, Arc<Vec<TextChar>>>,
    sel: Option<TextSel>,
    /// The pointer is over page text (for the I-beam of the text tools).
    over: bool,
    /// Edit-text mode: the line under the pointer (page, band in displayed points).
    hover: Option<(usize, [f32; 4])>,
    line: Option<lineedit::LineEdit>,
}

impl TabText {
    /// A line of the page is being edited (not yet in the document).
    pub fn editing(&self) -> bool {
        self.line.is_some()
    }
}

/// Forget what was read from older bytes.
fn sync(tab: &mut DocTab) {
    let generation = tab.generation();
    if tab.text.generation != generation {
        tab.text.generation = generation;
        tab.text.chars.clear();
        tab.text.sel = None;
        tab.text.hover = None;
    }
}

/// Text positions of `page`, read once per document revision; empty when it has none.
fn chars(tab: &mut DocTab, page: usize) -> Arc<Vec<TextChar>> {
    sync(tab);
    if let Some(chars) = tab.text.chars.get(&page) {
        return Arc::clone(chars);
    }
    let read = tab
        .renderer()
        .and_then(|doc| catch_unwind(AssertUnwindSafe(|| doc.text_chars(page).ok())).ok()?)
        .unwrap_or_default();
    let chars = Arc::new(read);
    tab.text.chars.insert(page, Arc::clone(&chars));
    chars
}

/// A window position as displayed page points of `page`.
fn page_point(tab: &DocTab, page: usize, position: Point<Pixels>) -> Option<(f32, f32)> {
    let rect = *tab.layout().rects.get(page)?;
    let zoom = tab.view.zoom;
    let origin = tab.view.bounds.origin;
    let x = f32::from(position.x - origin.x) + tab.view.scroll.0 - rect.x;
    let y = f32::from(position.y - origin.y) + tab.view.scroll.1 - rect.y;
    Some((x / zoom, y / zoom))
}

fn active_tab(cx: &App) -> Option<Entity<DocTab>> {
    cx.try_global::<TextState>()?.tab.as_ref()?.upgrade()
}

/// Run `update` on the document in front, if any.
fn with_active(cx: &mut App, update: impl FnOnce(&mut DocTab, &mut Context<DocTab>)) {
    if let Some(tab) = active_tab(cx) {
        tab.update(cx, |tab, cx| {
            update(tab, cx);
            cx.notify();
        });
    }
}

/// Register the actions and their keys.
pub fn register(cx: &mut App) {
    cx.set_global(TextState::default());
    // After the tools: at the same depth the later binding is tried first, so Esc comes here
    // first and passes on to theirs.
    cx.bind_keys([
        KeyBinding::new(secondary!("c"), CopyText, Some(KEYS)),
        KeyBinding::new(secondary!("a"), SelectAllText, Some(KEYS)),
        KeyBinding::new("escape", EscapeText, Some(KEYS)),
    ]);
    cx.on_action(|_: &EditText, cx| toggle_edit_text(cx));
    cx.on_action(|_: &ExtractText, cx| extract_text(cx));
    cx.on_action(|_: &RecognizeText, cx| recognize_text(cx));
    cx.on_action(|_: &CompareWith, cx| compare_with(cx));
    cx.on_action(|_: &CopyText, cx| copy_text(cx));
    cx.on_action(|_: &SelectAllText, cx| {
        with_active(cx, |tab, _| {
            let page = tab.view.current_page;
            let count = chars(tab, page).len();
            tab.text.sel = Some(TextSel {
                page,
                anchor: 0,
                head: count,
            });
        });
    });
    cx.on_action(|_: &EscapeText, cx| escape(cx));
    // Picking a tool ends the editing of the page's words.
    cx.observe_global::<ToolState>(|cx| {
        let picked = tool(cx);
        if editing_text(cx) && picked != state(cx).tool {
            end_edit_text(cx);
        }
    })
    .detach();
}

fn end_edit_text(cx: &mut App) {
    state(cx).edit_text = false;
    with_active(cx, |tab, cx| {
        lineedit::commit(tab, cx);
        tab.text.hover = None;
    });
    cx.refresh_windows();
}

fn toggle_edit_text(cx: &mut App) {
    if editing_text(cx) {
        end_edit_text(cx);
        return;
    }
    let picked = tool(cx);
    let text = state(cx);
    text.edit_text = true;
    text.tool = picked;
    with_active(cx, |tab, _| tab.text.sel = None);
    // Finish the text box being typed (the tools' Esc does it).
    cx.defer(|cx| cx.dispatch_action(&crate::tools::Deselect));
    toasts::info(
        cx,
        "Click a line of text to change its words. Esc to finish.",
    );
    cx.refresh_windows();
}

/// Esc: leave edit-text mode, else drop the text selection and do what the tools do.
fn escape(cx: &mut App) {
    if editing_text(cx) {
        end_edit_text(cx);
        return;
    }
    with_active(cx, |tab, _| tab.text.sel = None);
    // The next binding of Esc (the tools' Deselect) runs too.
    cx.propagate();
}

fn copy_text(cx: &mut App) {
    let Some(tab) = active_tab(cx) else { return };
    let copied = tab.update(cx, |tab, _| {
        let sel = tab.text.sel.clone()?;
        let chars = chars(tab, sel.page);
        Some(sel::text(&chars, sel.range()?))
    });
    if let Some(text) = copied.filter(|text| !text.is_empty()) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }
}

/// Run `work` in the background, then `done` with its result.
fn in_background<T: Send + 'static>(
    cx: &mut App,
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
    done: impl FnOnce(Result<T, String>, &mut App) + 'static,
) {
    let task = crate::services::jobs::background(cx, work);
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| done(result, cx));
    })
    .detach();
}

fn extract_text(cx: &mut App) {
    let Some(tab) = active_tab(cx) else { return };
    let (title, bytes) = {
        let tab = tab.read(cx);
        (
            format!("Text - {}", tab.title()),
            tab.session.bytes().to_vec(),
        )
    };
    in_background(
        cx,
        move || guarded(|| godpdf_core::extract_all_text(&bytes)).map_err(|err| err.to_string()),
        move |result, cx| match result {
            Ok(pages) => {
                let mut text = String::new();
                for (page, page_text) in pages.iter().enumerate() {
                    let _ = writeln!(text, "──── Page {} ────", page + 1);
                    text.push_str(page_text.trim_end());
                    text.push_str("\n\n");
                }
                // The page headers are not text of the document: not counted.
                let chars = pages.iter().map(|page| page.trim().chars().count()).sum();
                state(cx).windows.text = Some(view::TextView {
                    title,
                    text,
                    chars,
                    area: None,
                });
                cx.refresh_windows();
            }
            Err(err) => toasts::error(cx, format!("Text extraction failed: {err}")),
        },
    );
}

/// Read the words of the scanned pages of the document in front, in the background.
fn recognize_text(cx: &mut App) {
    let Some(tab) = active_tab(cx) else { return };
    let (generation, bytes) = {
        let tab = tab.read(cx);
        (tab.generation(), tab.session.bytes().to_vec())
    };
    toasts::info(
        cx,
        if ocr::ready() {
            "Reading the scanned pages…"
        } else {
            "First use: downloading text recognition (12 MB), then reading the pages…"
        },
    );
    let weak = tab.downgrade();
    in_background(
        cx,
        move || ocr::recognize(bytes),
        move |result, cx| {
            let Some(tab) = weak.upgrade() else { return };
            match result {
                Ok(_) if tab.read(cx).generation() != generation => {
                    toasts::note(
                        cx,
                        "The document changed while its text was read: run Recognize text again",
                    );
                }
                Ok(pages) if pages.is_empty() => toasts::note(cx, "Every page already has text"),
                Ok(pages) => {
                    let count = pages.iter().filter(|(_, words)| !words.is_empty()).count();
                    let result = tab.update(cx, |tab, cx| {
                        cx.notify();
                        tab.apply("Recognize text", |pdf| {
                            godpdf_core::add_text_layer(pdf, &pages)
                        })
                    });
                    match result {
                        Ok(()) => toasts::success(
                            cx,
                            format!(
                                "Text recognized on {count} pages: it can be searched and selected now"
                            ),
                        ),
                        Err(err) => toasts::error(cx, err),
                    }
                }
                Err(err) => toasts::error(cx, err),
            }
        },
    );
}

/// Pick another PDF and compare the document in front with it, in the background.
fn compare_with(cx: &mut App) {
    if active_tab(cx).is_none() {
        return;
    }
    crate::files::pick_file(cx, ("PDF document", &["pdf"]), compare_file);
}

fn compare_file(path: std::path::PathBuf, cx: &mut App) {
    let Some(tab) = active_tab(cx) else { return };
    let file = match crate::files::read_file(&path) {
        Ok(file) => file,
        Err(err) => return toasts::error(cx, err),
    };
    let (title, bytes) = {
        let tab = tab.read(cx);
        let title = format!("{} compared with {}", tab.title(), file.name);
        (title, tab.session.bytes().to_vec())
    };
    toasts::info(cx, "Comparing…");
    in_background(
        cx,
        move || compare::run(bytes, file.bytes, title),
        |result, cx| match result {
            Ok(view) => {
                state(cx).windows.compare = Some(view);
                cx.refresh_windows();
            }
            Err(err) => toasts::error(cx, format!("Cannot compare: {err}")),
        },
    );
}

/// Bring `page` of the document in front into view.
fn show_page(page: usize, cx: &mut App) {
    with_active(cx, |tab, _| tab.scroll_to_page(page));
}

/// True for the tools that pick page text: Select and the text markups.
pub(crate) fn picks_text(tool: Tool) -> bool {
    matches!(
        tool,
        Tool::Select | Tool::Highlight | Tool::Underline | Tool::StrikeOut
    )
}

/// Note whether `point` (displayed points of `page`) is on text; true when that changed.
pub(crate) fn hover(tab: &mut DocTab, page: usize, point: (f32, f32)) -> bool {
    let over = sel::char_near(&chars(tab, page), point).is_some();
    std::mem::replace(&mut tab.text.over, over) != over
}

/// The pointer was last seen over page text.
pub(crate) fn over_text(tab: &DocTab) -> bool {
    tab.text.over
}

/// Start selecting at `point` when it is on text: a caret (a drag selects from it), the word
/// on a double click, the line on a triple one. Any older selection goes; false off text.
pub(crate) fn select_from(tab: &mut DocTab, page: usize, point: (f32, f32), clicks: usize) -> bool {
    let chars = chars(tab, page);
    let carets = |range: std::ops::RangeInclusive<usize>| (*range.start(), *range.end() + 1);
    tab.text.sel = sel::char_near(&chars, point).map(|index| {
        let (anchor, head) = match clicks {
            0 | 1 => {
                // A plain click is an empty selection at the caret.
                let caret = sel::caret(&chars, point).unwrap_or(index);
                (caret, caret)
            }
            2 => carets(sel::word_of(&chars, index)),
            _ => carets(sel::line_of(&chars, index)),
        };
        TextSel { page, anchor, head }
    });
    tab.text.sel.is_some()
}

/// Move the end of the selection being dragged to `point` (displayed points of its page).
pub(crate) fn select_to(tab: &mut DocTab, point: (f32, f32)) {
    let Some(page) = selection_page(tab) else {
        return;
    };
    let head = sel::caret(&chars(tab, page), point);
    if let (Some(sel), Some(head)) = (tab.text.sel.as_mut(), head) {
        sel.head = head;
    }
}

/// The selected text as one box per line, in displayed points of its page.
pub(crate) fn selected_lines(tab: &mut DocTab) -> Option<(usize, Vec<[f32; 4]>)> {
    let sel = tab.text.sel.clone()?;
    let range = sel.range()?;
    Some((sel.page, sel::bands(&chars(tab, sel.page), range)))
}

/// The page the text selection is on.
pub(crate) fn selection_page(tab: &DocTab) -> Option<usize> {
    tab.text.sel.as_ref().map(|sel| sel.page)
}

pub(crate) fn clear_selection(tab: &mut DocTab) {
    tab.text.sel = None;
}

fn band_div(band: [f32; 4], zoom: f32, grow: f32) -> gpui::Div {
    div()
        .absolute()
        .left(px(band[0] * zoom - grow))
        .top(px(band[1] * zoom - grow))
        .w(px((band[2] - band[0]) * zoom + 2.0 * grow))
        .h(px((band[3] - band[1]) * zoom + 2.0 * grow))
}

/// Under the tools of `page`: the selected text (with the Select tool; the markup tools
/// preview their own look over it).
pub fn page_underlay(
    tab: &mut DocTab,
    page: usize,
    _window: &mut Window,
    cx: &mut Context<DocTab>,
) -> Option<AnyElement> {
    if editing_text(cx) || tool(cx) != Tool::Select {
        return None;
    }
    let zoom = tab.view.zoom;
    let bands = match selected_lines(tab) {
        Some((at, bands)) if at == page => bands,
        _ => return None,
    };
    let fill = Hsla {
        a: SELECTION_ALPHA,
        ..Hsla::from(gpui::rgb(crate::theme::BLUE))
    };
    Some(
        div()
            .absolute()
            .inset_0()
            .children(
                bands
                    .into_iter()
                    .map(|band| band_div(band, zoom, 0.0).bg(fill)),
            )
            .into_any_element(),
    )
}

fn edit_down(
    tab: &mut DocTab,
    page: usize,
    event: &MouseDownEvent,
    window: &mut Window,
    cx: &mut Context<DocTab>,
) {
    // While a line is open, a click only writes it.
    if tab.text.line.is_some() {
        lineedit::commit(tab, cx);
        return;
    }
    let Some(point) = page_point(tab, page, event.position) else {
        return;
    };
    let chars = chars(tab, page);
    if let Some((band, letters)) = lineedit::line_at(&chars, point) {
        tab.text.hover = None;
        lineedit::open(tab, page, band, letters, window, cx);
    }
}

fn edit_hover(tab: &mut DocTab, page: usize, event: &MouseMoveEvent, cx: &mut Context<DocTab>) {
    let hover = match page_point(tab, page, event.position) {
        Some(point) if tab.text.line.is_none() => {
            let chars = chars(tab, page);
            lineedit::line_at(&chars, point).map(|(band, _)| (page, band))
        }
        _ => None,
    };
    if tab.text.hover != hover {
        tab.text.hover = hover;
        cx.notify();
    }
}

/// Over the tools of `page` in edit-text mode: the line under the pointer is framed, a click
/// opens it; the line being typed.
pub fn page_overlay(
    tab: &mut DocTab,
    page: usize,
    _window: &mut Window,
    cx: &mut Context<DocTab>,
) -> Option<AnyElement> {
    if !editing_text(cx) {
        return None;
    }
    sync(tab);
    let palette = Palette::of(cx);
    let zoom = tab.view.zoom;
    let frame = tab
        .text
        .hover
        .filter(|(hovered, _)| *hovered == page)
        .map(|(_, band)| {
            band_div(band, zoom, 2.0 + 1.5)
                .rounded(px(2.))
                .border_1()
                .border_color(palette.accent)
                .cursor(CursorStyle::IBeam)
        });
    let line = tab
        .text
        .line
        .as_ref()
        .filter(|line| line.page == page)
        .map(|line| lineedit::render(line, zoom, &palette, cx));
    Some(
        div()
            .id(("edit-text-layer", page))
            .absolute()
            .inset_0()
            // Changing the words of the page comes before any tool.
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |tab, event, window, cx| {
                    edit_down(tab, page, event, window, cx);
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(move |tab, event, _, cx| edit_hover(tab, page, event, cx)))
            .on_hover(cx.listener(move |tab, hovered: &bool, _, cx| {
                if !hovered && tab.text.hover.is_some_and(|(at, _)| at == page) {
                    tab.text.hover = None;
                    cx.notify();
                }
            }))
            .children(frame)
            .children(line)
            .into_any_element(),
    )
}

/// The windows (extracted text, comparison), drawn by the app over everything. Remembers the
/// document in front, and writes a line typed in a document no longer in front.
pub fn overlay(
    active: Option<&Entity<DocTab>>,
    window: &mut Window,
    cx: &mut App,
) -> Vec<AnyElement> {
    let active = active.map(Entity::downgrade);
    let previous = state(cx).tab.clone();
    if previous != active {
        if let Some(tab) = previous.and_then(|weak| weak.upgrade()) {
            tab.update(cx, lineedit::commit);
        }
        state(cx).tab = active;
    }
    view::render(window, cx)
}
