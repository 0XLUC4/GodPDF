//! Text typed on a page before it becomes an annotation: a free-text box laid out like core's
//! FreeText appearance (same padding and line height, anchored by its top-left corner), or a
//! sticky note. Typed in a gpui-component textarea.
//!
//! Ctrl+Enter or a click on the page outside writes it; Esc restores an edited text as it was.

use godpdf_core::{Annotation, CharStyle, FREE_TEXT_LINE_HEIGHT, FREE_TEXT_PADDING, Font};
use gpui::component::input::{Escape, InputEvent, Textarea, TextareaState};
use gpui::prelude::*;
use gpui::{AnyElement, Context, Entity, FontWeight, Hsla, Subscription, Window, div, px, rgb};

use super::canvas::{self, report};
use super::geom::DRect;
use super::{
    Format, Style, TEXT_BOX, ToggleBold, ToggleItalic, ToggleUnderline, common_style, hsla,
    tool_state,
};
use crate::doc::DocTab;
use crate::theme::Palette;
use crate::toasts;

/// Width of a new text box, in points (narrowed to the page).
const DEFAULT_WIDTH: f32 = 220.0;
/// Narrowest text box, in font sizes.
const MIN_WIDTH_EMS: f32 = 2.0;
/// Side of a sticky note icon, in points.
const NOTE_SIZE: f32 = 24.0;
const NOTE_CARD_WIDTH: f32 = 240.0;
/// Box border; the page is always white, so this stays near-black in both themes.
const BORDER: u32 = 0x222222;

pub enum Kind {
    /// A text box: top-left corner and width in displayed points. `original` is the
    /// annotation it was opened from, already removed from the page while it is typed.
    Text {
        anchor: (f32, f32),
        width: f32,
        original: Option<Annotation>,
    },
    /// A sticky note at `rect` (displayed points); `id` when it edits an existing one.
    Note { rect: DRect, id: Option<String> },
}

pub struct Editor {
    pub page: usize,
    pub kind: Kind,
    pub state: Entity<TextareaState>,
    /// Format of the whole text box.
    pub format: CharStyle,
    /// The text, format and per-character styles it was opened with: kept on commit while
    /// neither text nor format changed (the box itself formats the whole text only).
    opened: Option<(String, CharStyle, Vec<CharStyle>)>,
    _enter: Subscription,
}

fn font_family(font: Font) -> &'static str {
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
            .default_value(text)
    });
    state.update(cx, |state, cx| state.focus(window, cx));
    // Keep the focus on the box: the window root would take it on this same click.
    window.prevent_default();
    let _enter = cx.subscribe(&state, |tab, _, event: &InputEvent, cx| {
        if let InputEvent::PressEnter {
            secondary: true, ..
        } = event
        {
            commit(tab, cx);
            cx.notify();
        }
    });
    tab.canvas.selected = None;
    Editor {
        page,
        kind,
        state,
        format: CharStyle::default(),
        opened: None,
        _enter,
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
        let text_style = &mut tool_state(cx).styles.text;
        text_style.font_size = *font_size;
        text_style.color = *color;
        text_style.font = *font;
        let result = tab.apply("Edit text", |pdf| {
            godpdf_core::remove_annotation(pdf, page, &item.id)
        });
        if let Err(err) = result {
            toasts::error(cx, format!("Cannot edit this text: {err}"));
            return;
        }
        let format = common_style(styles, text.chars().count());
        let opened = Some((text.clone(), format, styles.clone()));
        let kind = Kind::Text {
            anchor: (item.rect.x0, item.rect.y0),
            width: godpdf_core::free_text_width(*rect, *rotation),
            original: None,
        };
        let text = text.clone();
        let mut editor = open(tab, page, kind, text, window, cx);
        editor.format = format;
        editor.opened = opened;
        if let Kind::Text { original: slot, .. } = &mut editor.kind {
            *slot = Some(original);
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
            (None, rect, String::new())
        }
    };
    tab.canvas.editor = Some(open(tab, page, Kind::Note { rect, id }, text, window, cx));
}

/// Write what was typed. An emptied existing text stays removed: its "Edit text" step did it.
pub fn commit(tab: &mut DocTab, cx: &mut Context<DocTab>) {
    let Some(editor) = tab.canvas.editor.take() else {
        return;
    };
    let typed = editor.state.read(cx).value().to_string();
    let Some((geom, displayed)) = canvas::page_frame(tab, editor.page) else {
        return;
    };
    let page = editor.page;
    let result = match editor.kind {
        Kind::Text {
            anchor,
            width,
            original,
            ..
        } => {
            let text = typed.trim_end().to_owned();
            if text.trim().is_empty() {
                return;
            }
            let style: Style = tool_state(cx).styles.text;
            let len = text.chars().count();
            let styles = match editor.opened {
                Some((opened, format, styles)) if opened == text && format == editor.format => {
                    styles
                }
                _ if editor.format == CharStyle::default() => Vec::new(),
                _ => vec![editor.format; len],
            };
            let lines =
                godpdf_core::wrap_styled(&text, &styles, style.font, style.font_size, width).len();
            let height = lines.max(1) as f32 * style.font_size * FREE_TEXT_LINE_HEIGHT
                + 2.0 * FREE_TEXT_PADDING;
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
            let add = |pdf: &[u8]| godpdf_core::add_annotation(pdf, page, &annotation);
            if original.is_some() {
                tab.amend("Edit text", add)
            } else {
                tab.apply("Add text", add)
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

/// Esc: put an edited text back as it was; a new box is kept if it has text; a note is dropped.
fn cancel(tab: &mut DocTab, cx: &mut Context<DocTab>) {
    let Some(editor) = tab.canvas.editor.take() else {
        return;
    };
    match &editor.kind {
        Kind::Text {
            original: Some(original),
            ..
        } => {
            let page = editor.page;
            let result = tab.amend("Edit text", |pdf| {
                godpdf_core::add_annotation(pdf, page, original)
            });
            report(result, cx);
        }
        Kind::Text { .. } => {
            tab.canvas.editor = Some(editor);
            commit(tab, cx);
        }
        Kind::Note { .. } => {}
    }
}

/// Flip a format of the text box and give it the keyboard back.
pub fn toggle_format(
    tab: &mut DocTab,
    format: Format,
    window: &mut Window,
    cx: &mut Context<DocTab>,
) {
    let Some(editor) = tab.canvas.editor.as_mut() else {
        return;
    };
    let on = !format.is_on(editor.format);
    format.set(&mut editor.format, on);
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

fn with_alpha(color: Hsla, alpha: f32) -> Hsla {
    Hsla { a: alpha, ..color }
}

/// The box being typed on its page, `zoom` logical pixels per point.
pub fn render(
    tab: &mut DocTab,
    zoom: f32,
    _window: &mut Window,
    cx: &mut Context<DocTab>,
) -> Option<AnyElement> {
    let editor = tab.canvas.editor.as_ref()?;
    let palette = Palette::of(cx);
    let style = tool_state(cx).styles.text;
    let escape = cx.listener(|tab, _: &Escape, _, cx| {
        cancel(tab, cx);
        cx.notify();
    });
    let page_width = tab
        .page_sizes
        .get(editor.page)
        .map_or(0.0, |size| size.0 * zoom);
    let element = match &editor.kind {
        Kind::Text { anchor, width, .. } => {
            let format = editor.format;
            // Sub / superscript: smaller, moved off the baseline (core's size and rise).
            let font_px = format.size(style.font_size) * zoom;
            let line_px = style.font_size * FREE_TEXT_LINE_HEIGHT * zoom;
            let rise = format.rise(style.font_size) * zoom;
            let min_width = (MIN_WIDTH_EMS * style.font_size).min(*width);
            div()
                .id("text-box")
                .absolute()
                .left(px(anchor.0 * zoom))
                .top(px(anchor.1 * zoom))
                .w(px(width.max(min_width) * zoom))
                .p(px(FREE_TEXT_PADDING * zoom))
                .bg(with_alpha(gpui::white(), 0.67))
                .border_1()
                .border_color(rgb(BORDER))
                .occlude()
                .key_context(TEXT_BOX)
                .on_action(escape)
                .on_action(cx.listener(|tab, _: &ToggleBold, window, cx| {
                    toggle_format(tab, Format::Bold, window, cx)
                }))
                .on_action(cx.listener(|tab, _: &ToggleItalic, window, cx| {
                    toggle_format(tab, Format::Italic, window, cx)
                }))
                .on_action(cx.listener(|tab, _: &ToggleUnderline, window, cx| {
                    toggle_format(tab, Format::Underline, window, cx)
                }))
                .child(
                    div().relative().top(px(-rise)).child(
                        Textarea::new(&editor.state)
                            .appearance(false)
                            .text_size(px(font_px))
                            .line_height(px(line_px))
                            .font_family(font_family(style.font))
                            .text_color(hsla(style.color))
                            .when(format.bold, |text| text.font_weight(FontWeight::BOLD))
                            .when(format.italic, |text| text.italic())
                            .when(format.underline, |text| text.underline()),
                    ),
                )
                .into_any_element()
        }
        Kind::Note { rect, .. } => div()
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
            .flex()
            .flex_col()
            .gap(px(6.))
            .p(px(8.))
            .rounded(px(8.))
            .shadow_md()
            .bg(palette.panel)
            .border_1()
            .border_color(palette.border)
            .text_color(palette.text)
            .occlude()
            .key_context(TEXT_BOX)
            .on_action(escape)
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .child("Sticky note"),
            )
            .child(Textarea::new(&editor.state).h(px(88.)))
            .child(
                div()
                    .text_xs()
                    .text_color(palette.text_dim)
                    .child("Ctrl+Enter or click the page to save, Esc to cancel"),
            )
            .into_any_element(),
    };
    Some(element)
}
