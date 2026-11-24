//! Text typed on a page before it becomes an annotation: a free-text box laid out like core's
//! FreeText appearance (same padding and line height, anchored by its top-left corner), or a
//! sticky note. Typed in a gpui-component textarea; a text box's textarea is invisible (it
//! keeps the keys, the caret and the selection) and the text is drawn over it by
//! [`richtext`], each character in its own style (bold, italic, underline, sub / superscript).
//!
//! Ctrl+Enter or a click on the page outside writes it; Esc leaves an edited text as it was.
//! The grip on its right edge widens or narrows the box.

use std::ops::Range;
use std::rc::Rc;

use godpdf_core::{
    Annotation, CharStyle, FREE_TEXT_LINE_HEIGHT, FREE_TEXT_PADDING, Font, style_at,
};
use gpui::component::input::{Escape, InputEditsEvent, InputEvent, Textarea, TextareaState};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, CursorStyle, DispatchPhase, Div, Entity, Hsla, KeyDownEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Subscription, Window,
    canvas, div, px, rgb,
};

use super::canvas::{self, report};
use super::geom::{self, DRect};
use super::{
    Format, Style, TEXT_BOX, ToggleBold, ToggleItalic, ToggleUnderline, common_style, hsla,
    richtext, toggle_all, tool_state,
};
use crate::doc::DocTab;
use crate::theme::Palette;

/// Width of a new text box, in points (narrowed to the page).
const DEFAULT_WIDTH: f32 = 220.0;
/// Narrowest text box, in font sizes.
const MIN_WIDTH_EMS: f32 = 2.0;
/// Side of a sticky note icon, in points.
const NOTE_SIZE: f32 = 24.0;
const NOTE_CARD_WIDTH: f32 = 240.0;
/// Box border; the page is always white, so this stays near-black in both themes.
const BORDER: u32 = 0x222222;
/// The width grip on the right edge: handle size, and how far either side of the edge the
/// strip that grabs it reaches (screen pixels).
const GRIP: (f32, f32) = (8.0, 18.0);
pub(super) const GRIP_REACH: f32 = 5.0;

pub enum Kind {
    /// A text box: top-left corner and width in displayed points. `original` is the id and
    /// the annotation it edits, left on the page (drawn lifted off it) until the box is written;
    /// `style` is that text's own style (a new box uses the text tool's).
    Text {
        anchor: (f32, f32),
        width: f32,
        original: Option<(String, Annotation)>,
        style: Option<Style>,
    },
    /// A sticky note at `rect` (displayed points); `id` when it edits an existing one.
    Note { rect: DRect, id: Option<String> },
}

pub struct Editor {
    pub page: usize,
    pub kind: Kind,
    pub state: Entity<TextareaState>,
    /// The style of each character of the text (an empty list: all plain).
    styles: Vec<CharStyle>,
    /// The text `styles` were last matched to.
    marked: String,
    /// Picked with nothing selected: the style of what is typed next.
    typing: Option<CharStyle>,
    /// The width grip being dragged: pointer x minus the right edge of the box, in points.
    widen: Option<f32>,
    drag_anchor: Option<usize>,
    caret_hint: Option<(usize, usize)>,
    _events: [Subscription; 3],
}

impl Editor {
    fn select(&mut self, anchor: usize, hit: (usize, usize), cx: &mut Context<DocTab>) {
        let (head, _) = hit;
        let byte = |index| {
            self.marked
                .char_indices()
                .nth(index)
                .map_or(self.marked.len(), |(offset, _)| offset)
        };
        let range = byte(anchor.min(head))..byte(anchor.max(head));
        self.caret_hint = Some(hit);
        self.state.update(cx, |state, cx| {
            state.set_selected_range_with_direction(range, head < anchor, cx);
        });
        cx.notify();
    }

    /// The selected characters (an empty range is the caret).
    fn selection(&self, cx: &App) -> Range<usize> {
        let state = self.state.read(cx);
        let text = state.value();
        let count = |at: usize| text.get(..at).map_or(0, |part| part.chars().count());
        let range = state.selected_range();
        count(range.start)..count(range.end)
    }

    /// The formats of the selected characters, or of what is typed next.
    pub fn active(&self, cx: &App) -> CharStyle {
        let range = self.selection(cx);
        if range.is_empty() {
            let before = range.start.saturating_sub(1);
            return self.typing.unwrap_or(style_at(&self.styles, before));
        }
        common_style(&self.styles, range)
    }
}

/// Size, colour and font of the box being typed: the edited text's own, else the text tool's.
pub(super) fn text_style(tab: &DocTab, cx: &mut App) -> Style {
    match tab.canvas.editor.as_ref().map(|editor| &editor.kind) {
        Some(Kind::Text {
            style: Some(style), ..
        }) => *style,
        _ => tool_state(cx).styles.text,
    }
}

/// The style the style card changes while a box is typed (see [`text_style`]).
pub(super) fn text_style_mut<'a>(tab: &'a mut DocTab, cx: &'a mut App) -> &'a mut Style {
    match tab.canvas.editor.as_mut().map(|editor| &mut editor.kind) {
        Some(Kind::Text {
            style: Some(style), ..
        }) => style,
        _ => &mut tool_state(cx).styles.text,
    }
}

/// Width of a text box at `anchor_x` whose grip, grabbed `grab` points right of its edge, is
/// at `pointer_x`: at least two font sizes, never past the page edge.
fn widened(anchor_x: f32, pointer_x: f32, grab: f32, page_width: f32, font_size: f32) -> f32 {
    let room = (page_width - anchor_x).max(0.0);
    let min = (MIN_WIDTH_EMS * font_size).min(room);
    (pointer_x - grab - anchor_x).clamp(min, room)
}

pub(super) fn font_family(font: Font) -> &'static str {
    match font {
        Font::Helvetica => "Arial",
        Font::Times => "Times New Roman",
        Font::Courier => "Courier New",
    }
}

pub(super) fn font_label(font: Font) -> (&'static str, &'static str) {
    (font_family(font), font.name())
}

fn open(
    tab: &mut DocTab,
    page: usize,
    kind: Kind,
    text: String,
    window: &mut Window,
    cx: &mut Context<DocTab>,
) -> Editor {
    let placeholder = match kind {
        Kind::Text { .. } => "Type here",
        Kind::Note { .. } => "Type the note",
    };
    let state = cx.new(|cx| {
        TextareaState::new(window, cx)
            .auto_grow(1, 40)
            .placeholder(placeholder)
            .default_value(text.clone())
    });
    state.update(cx, |state, cx| state.focus(window, cx));
    // Keep the focus on the box: the window root would take it on this same click.
    window.prevent_default();
    let enter = cx.subscribe(&state, |tab, _, event: &InputEvent, cx| {
        if let InputEvent::PressEnter {
            secondary: true, ..
        } = event
        {
            commit(tab, cx);
            cx.notify();
        }
    });
    let edited = cx.subscribe(&state, |tab, state, event: &InputEditsEvent, cx| {
        let Some(editor) = tab.canvas.editor.as_mut() else {
            return;
        };
        let typing = editor.typing.take();
        let cursor = state.read(cx).cursor();
        for (edit, after) in event.edits() {
            let typed = after.to_string();
            let replaced = editor.marked.get(edit.start_byte..edit.old_end_byte);
            let inserted = typed.get(edit.start_byte..edit.new_end_byte);
            match (replaced, inserted) {
                (Some(replaced), Some(inserted))
                    if edit.start_byte != 0 || edit.old_end_byte != editor.marked.len() =>
                {
                    let start = editor.marked[..edit.start_byte].chars().count();
                    let end = start + replaced.chars().count();
                    crate::text::replace_marks(
                        &mut editor.styles,
                        editor.marked.chars().count(),
                        start..end,
                        inserted.chars().count(),
                        typing,
                    );
                }
                // Composition can arrive as a whole-document edit. Keep the unchanged styles.
                _ => crate::text::reconcile_at(
                    &mut editor.styles,
                    &editor.marked,
                    &typed,
                    typing,
                    typed.get(..cursor).map(|prefix| prefix.chars().count()),
                ),
            }
            editor.marked = typed;
        }
        editor.caret_hint = None;
        cx.notify();
    });
    // The box draws the caret and the selection: redraw when they move.
    let moved = cx.observe(&state, |_, _, cx| cx.notify());
    tab.canvas.selected = None;
    Editor {
        page,
        kind,
        state,
        styles: Vec::new(),
        marked: text,
        typing: None,
        widen: None,
        drag_anchor: None,
        caret_hint: None,
        _events: [enter, edited, moved],
    }
}

/// Type text at `point`: edit the free text under it, or (when `new_allowed`) start a new
/// box with the middle of its first line under the click.
pub fn open_text(
    tab: &mut DocTab,
    page: usize,
    point: (f32, f32),
    new_allowed: bool,
    window: &mut Window,
    cx: &mut Context<DocTab>,
) {
    let Some((_, displayed)) = canvas::page_frame(tab, page) else {
        return;
    };
    let hit = canvas::items(tab, page, cx).and_then(|items| {
        items.into_iter().rev().find(|item| {
            matches!(item.annotation, Some(Annotation::FreeText { .. }))
                && item.rect.contains(point, 0.0)
        })
    });
    if let Some(item) = hit {
        let Some(original @ Annotation::FreeText { .. }) = item.annotation else {
            return;
        };
        let Annotation::FreeText {
            rect,
            text,
            font_size,
            color,
            rotation,
            font,
            styles,
        } = &original
        else {
            return;
        };
        let style = Style {
            font_size: *font_size,
            color: *color,
            font: *font,
            ..tool_state(cx).styles.text
        };
        let width = godpdf_core::free_text_width(*rect, *rotation);
        let kind = Kind::Text {
            anchor: (item.rect.x0, item.rect.y0),
            width,
            original: None,
            style: Some(style),
        };
        let layout = richtext::Layout::new(text, styles, *font, *font_size, width);
        let top = item.rect.y0.min(displayed.1 - layout.height()).max(0.0);
        let hit = layout.hit_test(point.0 - item.rect.x0, point.1 - top);
        let (text, styles) = (text.clone(), styles.clone());
        // The page shows without it while it is typed.
        tab.lift(page, &item.id, cx);
        let mut editor = open(tab, page, kind, text, window, cx);
        editor.styles = styles;
        editor.select(hit.0, hit, cx);
        if let Kind::Text { original: slot, .. } = &mut editor.kind {
            *slot = Some((item.id, original));
        }
        tab.canvas.editor = Some(editor);
    } else if new_allowed {
        let font_size = tool_state(cx).styles.text.font_size;
        let width = DEFAULT_WIDTH.min(displayed.0);
        let anchor = (
            (point.0 - FREE_TEXT_PADDING).clamp(0.0, (displayed.0 - width).max(0.0)),
            (point.1 - FREE_TEXT_PADDING - font_size * 0.6).max(0.0),
        );
        let kind = Kind::Text {
            anchor,
            width,
            original: None,
            style: None,
        };
        tab.canvas.editor = Some(open(tab, page, kind, String::new(), window, cx));
    }
}

/// Type a sticky note at `point`, or edit `existing` (its id and displayed rect).
pub fn open_note(
    tab: &mut DocTab,
    page: usize,
    point: (f32, f32),
    existing: Option<(String, DRect)>,
    window: &mut Window,
    cx: &mut Context<DocTab>,
) {
    let (id, rect, text) = match existing {
        Some((id, rect)) => {
            let text = canvas::annotations(tab, page, cx)
                .iter()
                .find(|placed| placed.id == id)
                .and_then(|placed| match &placed.annotation {
                    Some(Annotation::Note { text, .. }) => Some(text.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            (Some(id), rect, text)
        }
        None => {
            let rect = DRect::span(point, (point.0 + NOTE_SIZE, point.1 + NOTE_SIZE));
            let displayed = tab.page_sizes.get(page).copied().unwrap_or_default();
            let rect = rect.translate(geom::clamp_offset(rect, displayed, (0.0, 0.0)));
            (None, rect, String::new())
        }
    };
    tab.canvas.editor = Some(open(tab, page, Kind::Note { rect, id }, text, window, cx));
}

/// Write what was typed: an edited text is replaced only when something changed, and removed
/// when emptied.
pub fn commit(tab: &mut DocTab, cx: &mut Context<DocTab>) {
    let Some(editor) = tab.canvas.editor.take() else {
        return;
    };
    let style = match &editor.kind {
        Kind::Text {
            style: Some(style), ..
        } => *style,
        _ => tool_state(cx).styles.text,
    };
    let typed = editor.state.read(cx).value().to_string();
    let Some((geom, displayed)) = canvas::page_frame(tab, editor.page) else {
        return;
    };
    let page = editor.page;
    let Editor {
        kind,
        mut styles,
        marked,
        typing,
        ..
    } = editor;
    crate::text::reconcile(&mut styles, &marked, &typed, typing);
    let result = match kind {
        Kind::Text {
            anchor,
            width,
            original,
            ..
        } => {
            let text = typed.trim_end().to_owned();
            if text.trim().is_empty() {
                if let Some((id, _)) = original {
                    let result = tab.apply("Delete text", |pdf| {
                        godpdf_core::remove_annotation(pdf, page, &id)
                    });
                    report(result, cx);
                }
                return;
            }
            styles.truncate(text.chars().count());
            if styles.iter().all(|style| *style == CharStyle::default()) {
                styles.clear();
            }
            let height =
                richtext::Layout::new(&text, &styles, style.font, style.font_size, width).height();
            let top = anchor.1.min(displayed.1 - height).max(0.0);
            let shown = DRect {
                x0: anchor.0,
                y0: top,
                x1: anchor.0 + width,
                y1: top + height,
            };
            let annotation = Annotation::FreeText {
                rect: shown.to_pdf(&geom, displayed),
                text,
                font_size: style.font_size,
                color: style.color,
                rotation: geom.rotation,
                font: style.font,
                styles,
            };
            match original {
                Some((_, original)) if same_text(&original, &annotation, width) => return,
                Some((id, _)) => tab.apply("Edit text", |pdf| {
                    godpdf_core::replace_annotation(pdf, page, &id, &annotation)
                }),
                None => tab.apply("Add text", |pdf| {
                    godpdf_core::add_annotation(pdf, page, &annotation)
                }),
            }
        }
        Kind::Note { rect, id } => {
            let text = typed.trim().to_owned();
            let annotation = Annotation::Note {
                rect: rect.to_pdf(&geom, displayed),
                text,
            };
            match id {
                Some(id) => tab.apply("Edit note", |pdf| {
                    godpdf_core::replace_annotation(pdf, page, &id, &annotation)
                }),
                None if typed.trim().is_empty() => return,
                None => tab.apply("Sticky note", |pdf| {
                    godpdf_core::add_annotation(pdf, page, &annotation)
                }),
            }
        }
    };
    report(result, cx);
}

/// Esc: an edited text stays as it was; a new box is kept if it has text; a note is dropped.
fn cancel(tab: &mut DocTab, cx: &mut Context<DocTab>) {
    let Some(editor) = tab.canvas.editor.take() else {
        return;
    };
    if let Kind::Text { original: None, .. } = editor.kind {
        tab.canvas.editor = Some(editor);
        commit(tab, cx);
    }
}

/// The text box typed from `original`, `width` points wide, says and looks the same.
fn same_text(original: &Annotation, typed: &Annotation, width: f32) -> bool {
    let (
        Annotation::FreeText {
            rect: was,
            text: old_text,
            font_size: old_size,
            color: old_color,
            rotation: old_rotation,
            font: old_font,
            styles: old_styles,
        },
        Annotation::FreeText {
            text,
            font_size,
            color,
            rotation,
            font,
            styles,
            ..
        },
    ) = (original, typed)
    else {
        return false;
    };
    (
        old_text,
        old_size,
        old_color,
        old_rotation,
        old_font,
        old_styles,
    ) == (text, font_size, color, rotation, font, styles)
        && (godpdf_core::free_text_width(*was, *old_rotation) - width).abs() < 0.01
}

/// Flip a format on the selected characters, or for what is typed next, and give the box
/// the keyboard back.
pub fn toggle_format(
    tab: &mut DocTab,
    format: Format,
    window: &mut Window,
    cx: &mut Context<DocTab>,
) {
    let Some(editor) = tab.canvas.editor.as_ref() else {
        return;
    };
    let range = editor.selection(cx);
    let active = editor.active(cx);
    let Some(editor) = tab.canvas.editor.as_mut() else {
        return;
    };
    if range.is_empty() {
        let mut typing = active;
        format.set(&mut typing, !format.is_on(active));
        editor.typing = Some(typing);
    } else {
        let len = editor.marked.chars().count();
        toggle_all(&mut editor.styles, len, range, format);
    }
    refocus(tab, window, cx);
}

/// After a click on the style card: typing goes on in the box.
pub fn refocus(tab: &mut DocTab, window: &mut Window, cx: &mut Context<DocTab>) {
    if let Some(editor) = &tab.canvas.editor {
        let state = editor.state.clone();
        state.update(cx, |state, cx| state.focus(window, cx));
    }
    cx.notify();
}

/// The width grip was grabbed at `position` (window coordinates).
fn grab_grip(tab: &mut DocTab, position: Point<Pixels>) {
    let Some(editor) = tab.canvas.editor.as_ref() else {
        return;
    };
    let Kind::Text { anchor, width, .. } = editor.kind else {
        return;
    };
    let Some(point) = canvas::page_point(tab, editor.page, position) else {
        return;
    };
    if let Some(editor) = tab.canvas.editor.as_mut() {
        editor.widen = Some(point.0 - (anchor.0 + width));
    }
}

/// The grip dragged to `position`: the box takes the new width, the text re-wraps.
fn drag_grip(tab: &mut DocTab, position: Point<Pixels>, cx: &mut Context<DocTab>) {
    let Some(editor) = tab.canvas.editor.as_ref() else {
        return;
    };
    let (page, Some(grab)) = (editor.page, editor.widen) else {
        return;
    };
    let (Some(point), Some(&(page_width, _))) = (
        canvas::page_point(tab, page, position),
        tab.page_sizes.get(page),
    ) else {
        return;
    };
    let font_size = text_style(tab, cx).font_size;
    if let Some(Editor {
        kind: Kind::Text { anchor, width, .. },
        ..
    }) = tab.canvas.editor.as_mut()
    {
        *width = widened(anchor.0, point.0, grab, page_width, font_size);
    }
    cx.notify();
}

/// While the grip is dragged: follow the pointer anywhere in the window, let go on release.
fn grip_tracker(cx: &mut Context<DocTab>) -> AnyElement {
    let this = cx.entity().downgrade();
    canvas(
        |_, _, _| {},
        move |_, _, window, _| {
            let moves = this.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                if phase == DispatchPhase::Bubble {
                    let _ = moves.update(cx, |tab, cx| drag_grip(tab, event.position, cx));
                }
            });
            let ups = this.clone();
            window.on_mouse_event(move |_: &MouseUpEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble {
                    let _ = ups.update(cx, |tab, cx| {
                        if let Some(editor) = tab.canvas.editor.as_mut() {
                            editor.widen = None;
                        }
                        refocus(tab, window, cx);
                    });
                }
            });
        },
    )
    .absolute()
    .size_0()
    .into_any_element()
}

fn text_selection_surface(
    layout: Rc<richtext::Layout>,
    page: usize,
    origin: (f32, f32),
    input: Div,
    cx: &mut Context<DocTab>,
) -> AnyElement {
    let hit = move |tab: &DocTab, position| {
        canvas::page_point(tab, page, position).map(|(x, y)| (x - origin.0, y - origin.1))
    };
    let drag_layout = Rc::clone(&layout);
    let this = cx.entity().downgrade();
    let tracker = canvas(
        |_, _, _| {},
        move |_, _, window, _| {
            let this = this.clone();
            let layout = Rc::clone(&drag_layout);
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                if phase != DispatchPhase::Capture {
                    return;
                }
                let _ = this.update(cx, |tab, cx| {
                    let Some(anchor) = tab
                        .canvas
                        .editor
                        .as_ref()
                        .and_then(|editor| editor.drag_anchor)
                    else {
                        return;
                    };
                    let Some((x, y)) = hit(tab, event.position) else {
                        return;
                    };
                    let Some(editor) = tab.canvas.editor.as_mut() else {
                        return;
                    };
                    if event.pressed_button != Some(MouseButton::Left) {
                        editor.drag_anchor = None;
                    } else {
                        let head = layout.hit_test(x, y);
                        if editor.caret_hint != Some(head) {
                            editor.select(anchor, head, cx);
                        }
                    }
                });
            });
        },
    )
    .absolute()
    .size_0();
    div()
        .id("text-selection")
        .absolute()
        .inset_0()
        .block_mouse_except_scroll()
        .cursor(CursorStyle::IBeam)
        .capture_any_mouse_down(cx.listener(move |tab, event: &MouseDownEvent, window, cx| {
            if event.button != MouseButton::Left {
                return;
            }
            let Some((x, y)) = hit(tab, event.position) else {
                return;
            };
            let Some(editor) = tab.canvas.editor.as_mut() else {
                return;
            };
            let (index, line) = layout.hit_test(x, y);
            let range = &layout.lines[line];
            let (anchor, head) = if event.modifiers.shift {
                let selection = editor.selection(cx);
                let state = editor.state.read(cx);
                let anchor = if state.cursor() == state.selected_range().start {
                    selection.end
                } else {
                    selection.start
                };
                (anchor, index)
            } else if event.click_count >= 3 {
                (range.start, range.end)
            } else if event.click_count == 2 {
                layout.word_at(x, line)
            } else {
                (index, index)
            };
            editor.drag_anchor =
                (event.click_count == 1 || event.modifiers.shift).then_some(anchor);
            editor.select(anchor, (head, line), cx);
            refocus(tab, window, cx);
            window.prevent_default();
            cx.stop_propagation();
        }))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|tab, _, _, _| {
                if let Some(editor) = tab.canvas.editor.as_mut() {
                    editor.drag_anchor = None;
                }
            }),
        )
        .on_mouse_up_out(
            MouseButton::Left,
            cx.listener(|tab, _, _, _| {
                if let Some(editor) = tab.canvas.editor.as_mut() {
                    editor.drag_anchor = None;
                }
            }),
        )
        .child(input)
        .child(tracker)
        .into_any_element()
}

/// The width grip on the right edge of a text box `height` pixels high: a dark bar along the
/// edge, a small white handle in its middle, and around the edge the strip that grabs it.
pub(super) fn grip(height: f32) -> Div {
    let border = rgb(BORDER);
    let handle = GRIP.1.min(height);
    div()
        .absolute()
        .top_0()
        .right(px(-GRIP_REACH))
        .w(px(2.0 * GRIP_REACH))
        .h(px(height))
        .cursor(CursorStyle::ResizeLeftRight)
        .child(
            div()
                .absolute()
                .top_0()
                .left(px(GRIP_REACH - 1.0))
                .w(px(2.0))
                .h_full()
                .bg(border),
        )
        .child(
            div()
                .absolute()
                .left(px(GRIP_REACH - GRIP.0 / 2.0))
                .top(px(((height - handle) / 2.0).round()))
                .w(px(GRIP.0))
                .h(px(handle))
                .rounded(px(2.0))
                .bg(gpui::white())
                .border_1()
                .border_color(border),
        )
}

fn with_alpha(color: Hsla, alpha: f32) -> Hsla {
    Hsla { a: alpha, ..color }
}

/// Where the text being edited on `page` was, from the page rendered without it (once ready):
/// drawn under the box, as the text stays on the page until the box is written.
pub(super) fn lifted_page(tab: &mut DocTab, page: usize, zoom: f32) -> Option<AnyElement> {
    let editor = tab
        .canvas
        .editor
        .as_ref()
        .filter(|editor| editor.page == page)?;
    let Kind::Text {
        original: Some((id, Annotation::FreeText { rect, .. })),
        ..
    } = &editor.kind
    else {
        return None;
    };
    let rect = *rect;
    let image = tab.lifted(page, id)?;
    let (geom, displayed) = canvas::page_frame(tab, page)?;
    let area = DRect::from_pdf(&geom, displayed, &rect);
    Some(canvas::cut_out(image, area, area, displayed, zoom))
}

/// The box being typed on its page, `zoom` logical pixels per point.
pub fn render(
    tab: &mut DocTab,
    zoom: f32,
    _window: &mut Window,
    cx: &mut Context<DocTab>,
) -> Option<AnyElement> {
    let style = text_style(tab, cx);
    let editor = tab.canvas.editor.as_ref()?;
    let palette = Palette::of(cx);
    let escape = cx.listener(|tab, _: &Escape, _, cx| {
        cancel(tab, cx);
        cx.notify();
    });
    let page_size = tab.page_sizes.get(editor.page).copied().unwrap_or_default();
    let page_width = page_size.0 * zoom;
    let element = match &editor.kind {
        Kind::Text { anchor, width, .. } => {
            let layout = Rc::new(richtext::Layout::new(
                &editor.marked,
                &editor.styles,
                style.font,
                style.font_size,
                *width,
            ));
            let height = layout.height();
            // As it is written: a box running off the bottom of the page goes up.
            let top = anchor.1.min(page_size.1 - height).max(0.0);
            let pad = FREE_TEXT_PADDING * zoom;
            let look = richtext::Look {
                family: font_family(style.font),
                color: hsla(style.color),
                selection: Some(editor.selection(cx)),
                caret_hint: editor.caret_hint,
            };
            let placeholder = editor.marked.is_empty().then(|| {
                div()
                    .absolute()
                    .left(px(pad))
                    .top(px(pad))
                    .text_size(px(style.font_size * zoom))
                    .line_height(px(style.font_size * FREE_TEXT_LINE_HEIGHT * zoom))
                    .font_family(font_family(style.font))
                    .text_color(gpui::rgb(0x888888))
                    .child("Type here")
            });
            // The input handles keyboard editing, context menus and IME.
            let input = div()
                .absolute()
                .left(px(pad))
                .top(px(pad))
                .w(px((width * zoom - 2.0 * pad).max(1.0)))
                .opacity(0.0)
                .child(
                    Textarea::new(&editor.state)
                        .appearance(false)
                        .text_size(px(style.font_size * zoom))
                        .line_height(px(style.font_size * FREE_TEXT_LINE_HEIGHT * zoom))
                        .font_family(font_family(style.font)),
                );
            div()
                .id("text-box")
                .absolute()
                .left(px(anchor.0 * zoom))
                .top(px(top * zoom))
                .w(px(width * zoom))
                .h(px(height * zoom))
                .bg(with_alpha(gpui::white(), 0.67))
                .block_mouse_except_scroll()
                .cursor(CursorStyle::IBeam)
                .key_context(TEXT_BOX)
                .on_action(escape)
                .capture_key_down(cx.listener(|tab, _: &KeyDownEvent, _, cx| {
                    if let Some(editor) = tab.canvas.editor.as_mut()
                        && editor.caret_hint.take().is_some()
                    {
                        cx.notify();
                    }
                }))
                .on_action(cx.listener(|tab, _: &ToggleBold, window, cx| {
                    toggle_format(tab, Format::Bold, window, cx)
                }))
                .on_action(cx.listener(|tab, _: &ToggleItalic, window, cx| {
                    toggle_format(tab, Format::Italic, window, cx)
                }))
                .on_action(cx.listener(|tab, _: &ToggleUnderline, window, cx| {
                    toggle_format(tab, Format::Underline, window, cx)
                }))
                .children(placeholder)
                .child(richtext::element(Rc::clone(&layout), look, zoom))
                // Over the box, not around it: the text starts at the box's own corner, as
                // once written.
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .border_1()
                        .border_color(rgb(BORDER)),
                )
                .child(text_selection_surface(
                    layout,
                    editor.page,
                    (anchor.0, top),
                    input,
                    cx,
                ))
                .child(
                    grip(height * zoom)
                        .block_mouse_except_scroll()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|tab, event: &MouseDownEvent, window, cx| {
                                grab_grip(tab, event.position);
                                // The box keeps the keyboard.
                                window.prevent_default();
                                cx.stop_propagation();
                                cx.notify();
                            }),
                        ),
                )
                .when(editor.widen.is_some(), |text_box| {
                    text_box.child(grip_tracker(cx))
                })
                .into_any_element()
        }
        Kind::Note { rect, .. } => super::quickbar::card(&palette)
            .id("note-box")
            .absolute()
            // Right of the note, or left of it when the page has no room there.
            .left(px(
                if rect.x1 * zoom + 6.0 + NOTE_CARD_WIDTH <= page_width {
                    rect.x1 * zoom + 6.0
                } else {
                    (rect.x0 * zoom - 6.0 - NOTE_CARD_WIDTH).max(0.0)
                },
            ))
            .top(px(rect.y0 * zoom))
            .w(px(NOTE_CARD_WIDTH))
            // No grey outlines: the card's shadow alone sets it off the page.
            .border_0()
            .text_size(px(crate::fonts::BODY))
            .key_context(TEXT_BOX)
            .on_action(escape)
            .child(Textarea::new(&editor.state).bordered(false).h(px(88.)))
            .into_any_element(),
    };
    Some(element)
}

#[cfg(test)]
mod tests {
    use super::widened;

    #[test]
    fn the_grip_widens_within_the_page() {
        // Box at x 100, grabbed 2 points right of its edge, pointer at 302: 200 wide.
        assert_eq!(widened(100.0, 302.0, 2.0, 612.0, 12.0), 200.0);
        // Never narrower than two font sizes, never past the page edge.
        assert_eq!(widened(100.0, 50.0, 0.0, 612.0, 12.0), 24.0);
        assert_eq!(widened(100.0, 900.0, 0.0, 612.0, 12.0), 512.0);
    }
}
