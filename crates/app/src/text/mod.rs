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
        EscapeText
    ]
);

/// Same context as the single-key tool shortcuts: never while typing in a field.
const KEYS: &str = "GodPdf && !Input";
/// Fill of selected page text (black at alpha 56, the UI stays monochrome).
const SELECTION_ALPHA: f32 = 56.0 / 255.0;

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
    /// A selection drag is going on.
    dragging: bool,
    /// Edit-text mode: the line under the pointer (page, band in displayed points).
    hover: Option<(usize, [f32; 4])>,
    line: Option<lineedit::LineEdit>,
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
        KeyBinding::new("escape", EscapeText, Some(KEYS)),
    ]);
    cx.on_action(|_: &EditText, cx| toggle_edit_text(cx));
    cx.on_action(|_: &ExtractText, cx| extract_text(cx));
    cx.on_action(|_: &RecognizeText, cx| recognize_text(cx));
    cx.on_action(|_: &CompareWith, cx| compare_with(cx));
    cx.on_action(|_: &CopyText, cx| copy_text(cx));
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
        Some(sel::text(&chars, sel.range()))
    });
    if let Some(text) = copied.filter(|text| !text.is_empty()) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }
}

/// Run `work` in the background, then `done` with its result.
fn in_background<T: Send + 'static>(
    cx: &mut App,
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(T, &mut App) + 'static,
) {
    let task = crate::services::jobs::background(cx, async move { work() });
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
                state(cx).windows.text = Some(view::TextView {
                    title,
                    text,
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
                    toasts::info(cx, "The document changed meanwhile: run it again");
                }
                Ok(pages) if pages.is_empty() => toasts::info(cx, "Every page already has text"),
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
    let Some(tab) = active_tab(cx) else { return };
    let Some(path) = rfd::FileDialog::new()
        .add_filter("PDF document", &["pdf"])
        .pick_file()
    else {
        return;
    };
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

fn select_down(tab: &mut DocTab, page: usize, event: &MouseDownEvent) {
    // The tools layer above saw the click first: on a comment, it selected that.
    let on_comment = tab.canvas.selected.is_some();
    let Some(point) = page_point(tab, page, event.position) else {
        return;
    };
    let chars = chars(tab, page);
    tab.text.sel = sel::char_near(&chars, point)
        .filter(|_| !on_comment)
        .map(|index| TextSel {
            page,
            anchor: index,
            head: index,
        });
    tab.text.dragging = tab.text.sel.is_some();
}

fn select_move(tab: &mut DocTab, page: usize, event: &MouseMoveEvent, cx: &mut Context<DocTab>) {
    if !tab.text.dragging {
        return;
    }
    if !event.dragging() {
        tab.text.dragging = false;
        return;
    }
    let Some(point) = page_point(tab, page, event.position) else {
        return;
    };
    let chars = chars(tab, page);
    let head = sel::nearest(&chars, point);
    if let (Some(sel), Some(head)) = (tab.text.sel.as_mut().filter(|sel| sel.page == page), head)
        && sel.head != head
    {
        sel.head = head;
        cx.notify();
    }
}

fn band_div(band: [f32; 4], zoom: f32, grow: f32) -> gpui::Div {
    div()
        .absolute()
        .left(px(band[0] * zoom - grow))
        .top(px(band[1] * zoom - grow))
        .w(px((band[2] - band[0]) * zoom + 2.0 * grow))
        .h(px((band[3] - band[1]) * zoom + 2.0 * grow))
}

/// Under the tools of `page`: the selected text, and the Select tool's drag over text.
pub fn page_underlay(
    tab: &mut DocTab,
    page: usize,
    _window: &mut Window,
    cx: &mut Context<DocTab>,
) -> Option<AnyElement> {
    if editing_text(cx) || tool(cx) != Tool::Select {
        return None;
    }
    sync(tab);
    let zoom = tab.view.zoom;
    let bands = match tab.text.sel.clone().filter(|sel| sel.page == page) {
        Some(sel) => sel::bands(&chars(tab, page), sel.range()),
        None => Vec::new(),
    };
    let fill = Hsla {
        a: SELECTION_ALPHA,
        ..gpui::black()
    };
    Some(
        div()
            .absolute()
            .inset_0()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |tab, event, _, cx| {
                    select_down(tab, page, event);
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(move |tab, event, _, cx| select_move(tab, page, event, cx)))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|tab, _, _, _| tab.text.dragging = false),
            )
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
