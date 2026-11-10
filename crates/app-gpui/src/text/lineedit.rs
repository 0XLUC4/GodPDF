//! Changing a line the page draws, where it is (ported from `crates/app/src/lineedit.rs`):
//! each word keeps the style (bold, slanted, underlined) and the colour the page had for it;
//! the keys or the small bar above the line change those of the selected words.
//!
//! Enter or a click away writes it, Esc drops it.

use godpdf_core::{CharStyle, Font, LineLook, Rect, style_at};
use gpui::component::input::{Escape, Input, InputEvent, InputState};
use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Entity, FontWeight, Hsla, MouseButton, Rgba, SharedString, Subscription,
    Window, div, px, rgb, white,
};

use super::sel;
use crate::doc::{DocTab, guarded};
use crate::theme::{self, Palette};
use crate::tools::{Format, ToggleBold, ToggleItalic, ToggleUnderline};
use crate::{geometry, icons, toasts, widgets};

/// Frame of the line being changed (egui: rgb 25 63 EB).
const FRAME_BLUE: u32 = 0x2563EB;

pub struct LineEdit {
    pub page: usize,
    /// The line on the page, displayed points.
    band: [f32; 4],
    /// The line in PDF space, what core rewrites.
    area: Rect,
    input: Entity<InputState>,
    /// One per character of the text.
    styles: Vec<CharStyle>,
    /// The colour of each character: which of `colors`.
    inks: Vec<usize>,
    /// The text, styles and colours when the line was opened.
    original: (String, Vec<CharStyle>, Vec<usize>),
    /// The text `styles` and `inks` were last matched to.
    marked: String,
    /// Picked on the bar with nothing selected: the style and colour of what is typed next.
    typing: Option<(CharStyle, usize)>,
    font: Font,
    size: f32,
    colors: Vec<Hsla>,
    _events: Subscription,
}

impl LineEdit {
    fn changed(&self) -> bool {
        (&self.marked, &self.styles, &self.inks)
            != (&self.original.0, &self.original.1, &self.original.2)
    }

    /// The selected characters (char indices), from the field's byte range.
    fn selection(&self, cx: &gpui::App) -> std::ops::Range<usize> {
        let state = self.input.read(cx);
        let text = state.value();
        let count = |at: usize| text.get(..at).map_or(0, |part| part.chars().count());
        let range = state.selected_range();
        count(range.start)..count(range.end)
    }

    /// The style and colour of the character before `at`, or of the first one.
    fn mark_before(&self, at: usize) -> (CharStyle, usize) {
        let at = at.saturating_sub(1);
        (
            style_at(&self.styles, at),
            self.inks.get(at).copied().unwrap_or(0),
        )
    }

    /// The style of the selected words, or of what is typed next.
    fn active(&self, cx: &gpui::App) -> CharStyle {
        let range = self.selection(cx);
        if range.is_empty() {
            return self.typing.unwrap_or(self.mark_before(range.start)).0;
        }
        let mut common = CharStyle::default();
        for format in Format::ALL {
            let on = range
                .clone()
                .all(|index| format.is_on(style_at(&self.styles, index)));
            format.set(&mut common, on);
        }
        common
    }
}

/// Have `marks` (one per character of `old`) follow the words to `new`: what stayed keeps
/// its mark, what was typed takes `typing`, or the mark of the character before it.
fn reconcile<T: Copy + Default>(marks: &mut Vec<T>, old: &str, new: &str, typing: Option<T>) {
    let old: Vec<char> = old.chars().collect();
    let new: Vec<char> = new.chars().collect();
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    marks.resize(old.len(), T::default());
    let mark = typing
        .or_else(|| prefix.checked_sub(1).map(|at| marks[at]))
        .or_else(|| marks.get(prefix).copied())
        .unwrap_or_default();
    let kept = marks.split_off(old.len() - suffix);
    marks.truncate(prefix);
    marks.extend(std::iter::repeat_n(mark, new.len() - prefix - suffix));
    marks.extend(kept);
}

/// Each letter of a line with the middle of its glyph, displayed points.
type Letters = Vec<(char, (f32, f32))>;

fn font_family(font: Font) -> &'static str {
    match font {
        Font::Helvetica => "Arial",
        Font::Times => "Times New Roman",
        Font::Courier => "Courier New",
    }
}

/// A line of `page` was clicked: open it for typing. `band` in displayed points, `letters`
/// as [`sel::spelled`] gives them.
pub fn open(
    tab: &mut DocTab,
    page: usize,
    band: [f32; 4],
    letters: Letters,
    window: &mut Window,
    cx: &mut Context<DocTab>,
) {
    commit(tab, cx);
    let Some(&size) = tab.page_sizes.get(page) else {
        return;
    };
    let fail = |err: String, cx: &mut Context<DocTab>| {
        toasts::error(cx, format!("Cannot edit this text: {err}"));
    };
    let geom = match tab.page_geom(page) {
        Ok(geom) => geom,
        Err(err) => return fail(err, cx),
    };
    let area = geometry::to_pdf_rect(&geom, size, (band[0], band[1]), (band[2], band[3]));
    let look: LineLook = match guarded(|| godpdf_core::line_look(tab.session.bytes(), page, area)) {
        Ok(look) => look,
        Err(err) => return fail(err.to_string(), cx),
    };
    // Each letter is as the page draws it where it is.
    let marks: Vec<(char, CharStyle, usize)> = letters
        .into_iter()
        .map(|(letter, at)| {
            let (x, _) = geometry::to_pdf(&geom, size, at);
            (letter, look.style_at(x), look.ink_at(x))
        })
        .collect();
    let text: String = marks.iter().map(|(letter, ..)| *letter).collect();
    let styles: Vec<CharStyle> = marks.iter().map(|(_, style, _)| *style).collect();
    let inks: Vec<usize> = marks.iter().map(|(.., ink)| *ink).collect();
    let colors = look
        .inks
        .iter()
        .map(|ink| {
            ink.map_or(gpui::black(), |color| {
                Rgba {
                    r: color.r,
                    g: color.g,
                    b: color.b,
                    a: 1.0,
                }
                .into()
            })
        })
        .collect();

    let end = text.len();
    let input = cx.new(|cx| InputState::new(window, cx).default_value(text.clone()));
    input.update(cx, |state, cx| {
        state.set_selected_range(end..end, cx);
        state.focus(window, cx);
    });
    // Keep the focus in the field: the window root would take it on this same click.
    window.prevent_default();
    let _events = cx.subscribe(&input, |tab, input, event: &InputEvent, cx| match event {
        InputEvent::Change => {
            let typed = input.read(cx).value().to_string();
            if let Some(line) = tab.text.line.as_mut() {
                let typing = line.typing.take();
                reconcile(&mut line.styles, &line.marked, &typed, typing.map(|t| t.0));
                reconcile(&mut line.inks, &line.marked, &typed, typing.map(|t| t.1));
                line.marked = typed;
            }
            cx.notify();
        }
        // A click anywhere but in the field or its bar writes it.
        InputEvent::PressEnter { .. } | InputEvent::Blur => {
            commit(tab, cx);
            cx.notify();
        }
        InputEvent::Focus => {}
    });
    tab.text.line = Some(LineEdit {
        page,
        band,
        area,
        input,
        original: (text.clone(), styles.clone(), inks.clone()),
        marked: text,
        styles,
        inks,
        typing: None,
        font: look.font,
        size: look.size,
        colors,
        _events,
    });
}

/// Write the line when it changed.
pub fn commit(tab: &mut DocTab, cx: &mut Context<DocTab>) {
    let Some(edit) = tab.text.line.take().filter(LineEdit::changed) else {
        return;
    };
    let result = tab.apply("Edit text", |pdf| {
        godpdf_core::replace_styled(
            pdf,
            edit.page,
            edit.area,
            &edit.marked,
            &edit.styles,
            &edit.inks,
        )
    });
    if let Err(err) = result {
        toasts::error(cx, format!("Cannot edit this text: {err}"));
    }
}

/// Flip `format` on the selected words, or for what is typed next; the field keeps the keys.
fn toggle(tab: &mut DocTab, format: Format, window: &mut Window, cx: &mut Context<DocTab>) {
    let Some(line) = tab.text.line.as_mut() else {
        return;
    };
    let range = line.selection(cx);
    if range.is_empty() {
        let mut typing = line.typing.unwrap_or(line.mark_before(range.start));
        let on = !format.is_on(typing.0);
        format.set(&mut typing.0, on);
        line.typing = Some(typing);
    } else {
        let on = !format.is_on(line.active(cx));
        line.styles
            .resize(line.marked.chars().count(), CharStyle::default());
        for style in line.styles.iter_mut().take(range.end).skip(range.start) {
            format.set(style, on);
        }
    }
    let input = line.input.clone();
    input.update(cx, |state, cx| state.focus(window, cx));
    cx.notify();
}

/// The line being typed over its page, `zoom` logical pixels per point.
pub fn render(
    line: &LineEdit,
    zoom: f32,
    palette: &Palette,
    cx: &mut Context<DocTab>,
) -> AnyElement {
    let [x0, y0, x1, y1] = line.band;
    let font_px = line.size * zoom;
    // Styled like most of its letters: the field draws one style.
    // shortcut: one font style and colour for the whole field (egui draws each word's own);
    // upgrade with a styled text layout under a transparent field.
    let count = line.styles.len().max(1);
    let most = |format: Format| {
        line.styles
            .iter()
            .filter(|style| format.is_on(**style))
            .count()
            * 2
            > count
    };
    let color = line
        .inks
        .first()
        .and_then(|ink| line.colors.get(*ink))
        .copied()
        .unwrap_or(gpui::black());
    let letters = line.marked.chars().count() as f32;
    let width = ((x1 - x0) * zoom).max((letters * 0.55 + 1.0) * font_px);
    let height = ((y1 - y0) * zoom).max(font_px * 1.25);
    let top = (y0 + y1) / 2.0 * zoom - height / 2.0;
    let keep_focus = |_: &gpui::MouseDownEvent, window: &mut Window, cx: &mut gpui::App| {
        window.prevent_default();
        cx.stop_propagation();
    };

    let field = div()
        .id("line-edit")
        .absolute()
        .left(px(x0 * zoom - 2.0))
        .top(px(top - 2.0))
        .w(px(width + 4.0))
        .h(px(height + 4.0))
        .px(px(2.0))
        .flex()
        .items_center()
        .bg(white())
        .rounded(px(2.0))
        .border_1()
        .border_color(rgb(FRAME_BLUE))
        // Same key context as the text box: Ctrl+B / I / U are bound there.
        .key_context("TextBox")
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_action(cx.listener(|tab, _: &Escape, _, cx| {
            tab.text.line = None;
            cx.notify();
        }))
        .on_action(
            cx.listener(|tab, _: &ToggleBold, window, cx| toggle(tab, Format::Bold, window, cx)),
        )
        .on_action(
            cx.listener(|tab, _: &ToggleItalic, window, cx| {
                toggle(tab, Format::Italic, window, cx)
            }),
        )
        .on_action(cx.listener(|tab, _: &ToggleUnderline, window, cx| {
            toggle(tab, Format::Underline, window, cx)
        }))
        .child(
            Input::new(&line.input)
                .appearance(false)
                .w_full()
                .text_size(px(font_px))
                .font_family(font_family(line.font))
                .text_color(color)
                .when(most(Format::Bold), |text| {
                    text.font_weight(FontWeight::BOLD)
                })
                .when(most(Format::Italic), |text| text.italic())
                .when(most(Format::Underline), |text| text.underline()),
        );

    // egui: a popup frame (margin 4) 6 px above the line, its three toggles 2 px apart.
    let active = line.active(cx);
    let bar_height = widgets::ICON_BUTTON + 8.0 + 2.0;
    let toggles = Format::ALL.into_iter().map(|format| {
        let (glyph, tip) = match format {
            Format::Bold => (icons::TEXT_B, "Bold (Ctrl+B)"),
            Format::Italic => (icons::TEXT_ITALIC, "Italic (Ctrl+I)"),
            Format::Underline => (icons::TEXT_UNDERLINE, "Underline (Ctrl+U)"),
        };
        let selected = palette.selected;
        let id = SharedString::from(format!("line-format-{format:?}"));
        widgets::tooltip(
            widgets::icon_widget(id, glyph, widgets::ICON_BUTTON, 16.0, true, palette)
                .when(format.is_on(active), |button| button.bg(selected))
                .on_click(cx.listener(move |tab, _, window, cx| toggle(tab, format, window, cx))),
            tip,
        )
    });
    let bar = div()
        .absolute()
        .left(px(x0 * zoom - 2.0))
        .top(px(top - 2.0 - bar_height - 6.0))
        .flex()
        .gap(px(2.0))
        .p(px(4.0))
        .rounded(px(theme::MENU_RADIUS))
        .shadow(widgets::popup_shadow(palette))
        .bg(palette.panel)
        .border_1()
        .border_color(palette.border)
        .on_mouse_down(MouseButton::Left, keep_focus)
        .children(toggles);

    div()
        .absolute()
        .inset_0()
        .child(field)
        .child(bar)
        .into_any_element()
}

/// The line under the pointer at `point` (displayed points): its band and its letters.
pub fn line_at(
    chars: &[godpdf_render::TextChar],
    point: (f32, f32),
) -> Option<([f32; 4], Letters)> {
    let line = sel::line_of(chars, sel::char_near(chars, point)?);
    let band = sel::bands(chars, line.clone())
        .into_iter()
        .reduce(|all, band| {
            [
                all[0].min(band[0]),
                all[1].min(band[1]),
                all[2].max(band[2]),
                all[3].max(band[3]),
            ]
        })?;
    Some((band, sel::spelled(chars, line)))
}

#[cfg(test)]
mod tests {
    use super::reconcile;

    #[test]
    fn marks_follow_the_words() {
        // Typed in the middle: the new letters take the mark of what is typed.
        let mut marks = vec![1, 1, 2, 2];
        reconcile(&mut marks, "abcd", "abXYcd", Some(9));
        assert_eq!(marks, [1, 1, 9, 9, 2, 2]);
        // Taken out: the others keep theirs.
        reconcile(&mut marks, "abXYcd", "ad", Some(9));
        assert_eq!(marks, [1, 2]);
        // Nothing picked: like the letter before, or the first one at the start.
        reconcile(&mut marks, "ad", "aXd", None);
        assert_eq!(marks, [1, 1, 2]);
        reconcile(&mut marks, "aXd", "YaXd", None);
        assert_eq!(marks, [1, 1, 1, 2]);
        reconcile(&mut marks, "YaXd", "", None);
        assert!(marks.is_empty());
    }
}
