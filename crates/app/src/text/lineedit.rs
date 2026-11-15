//! Changing a run of text the page draws (a line, or a cell of a table on it), where it is.
//! Text only: what changed is written again in place, everything else on the page stays as
//! it was (core `replace_run`). Bold / slanted / underlined from the keys or the small bar
//! above the run rewrite it word by word in a standard font (core `replace_styled`).
//!
//! Enter or a click away writes it, Esc drops it.

use godpdf_core::{CharStyle, Font, LineLook, Rect, style_at};
use gpui::component::input::{Escape, Input, InputEvent, InputState};
use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Entity, FontWeight, Hsla, MouseButton, Rgba, SharedString, Subscription,
    Window, div, px, rgb,
};

use super::sel;
use crate::doc::{DocTab, guarded};
use crate::theme::{self, Palette};
use crate::tools::{Format, ToggleBold, ToggleItalic, ToggleUnderline};
use crate::{geometry, icons, toasts, widgets};

/// Line height of the field, in ems of the text: room for accents and descenders.
const FIELD_LINE: f32 = 1.3;

/// Frame of the line being changed (rgb 25 63 EB).
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
    /// A style was changed on the bar or with the keys: the run is written word by word.
    restyled: bool,
    font: Font,
    size: f32,
    colors: Vec<Hsla>,
    /// The page colour around the run: the field's, so light text on a dark band shows.
    backdrop: Hsla,
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
pub(crate) fn reconcile<T: Copy + Default>(
    marks: &mut Vec<T>,
    old: &str,
    new: &str,
    typing: Option<T>,
) {
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

/// The colour the page has around `band` (left, top, right, bottom, in pixels of a `width` x
/// `height` BGRA bitmap): the most common one on a ring 2 to 4 pixels outside it. White when
/// the ring is off the bitmap.
pub(crate) fn backdrop(bgra: &[u8], width: usize, height: usize, band: [f32; 4]) -> [u8; 3] {
    let mut counts: std::collections::HashMap<[u8; 3], usize> = Default::default();
    let mut count = |x: f32, y: f32| {
        let (x, y) = (x.round(), y.round());
        if x < 0.0 || y < 0.0 || x >= width as f32 || y >= height as f32 {
            return;
        }
        let at = (y as usize * width + x as usize) * 4;
        if let Some(&[b, g, r, _]) = bgra.get(at..at + 4).and_then(|px| px.as_array()) {
            *counts.entry([r, g, b]).or_default() += 1;
        }
    };
    let [x0, y0, x1, y1] = band;
    for out in [2.0, 3.0, 4.0] {
        let steps = ((x1 - x0) / 2.0).ceil().max(1.0) as usize;
        for step in 0..=steps {
            let x = x0 + (x1 - x0) * step as f32 / steps as f32;
            count(x, y0 - out);
            count(x, y1 + out);
        }
        let steps = ((y1 - y0) / 2.0).ceil().max(1.0) as usize;
        for step in 0..=steps {
            let y = y0 + (y1 - y0) * step as f32 / steps as f32;
            count(x0 - out, y);
            count(x1 + out, y);
        }
    }
    // Ties go to the lightest, so a page cut evenly stays as it was (white first).
    counts
        .into_iter()
        .max_by_key(|(color, count)| (*count, color.iter().map(|&c| u32::from(c)).sum::<u32>()))
        .map_or([255; 3], |(color, _)| color)
}

/// The colour of the page around `band` (displayed points) as last rendered.
fn page_backdrop(tab: &DocTab, page: usize, band: [f32; 4]) -> Hsla {
    let white = Rgba {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };
    let color = (|| {
        let image = tab.cached_page(page)?;
        let &(page_width, _) = tab.page_sizes.get(page)?;
        let pixels = image.size(0);
        let (width, height) = (pixels.width.0 as usize, pixels.height.0 as usize);
        let scale = width as f32 / page_width.max(1.0);
        let [r, g, b] = backdrop(image.as_bytes(0)?, width, height, band.map(|v| v * scale));
        let channel = |value: u8| f32::from(value) / 255.0;
        Some(Rgba {
            r: channel(r),
            g: channel(g),
            b: channel(b),
            a: 1.0,
        })
    })();
    color.unwrap_or(white).into()
}

fn font_family(font: Font) -> &'static str {
    match font {
        Font::Helvetica => "Arial",
        Font::Times => "Times New Roman",
        Font::Courier => "Courier New",
    }
}

/// A run of `page` was clicked: open it for typing. `band` in displayed points.
pub fn open(
    tab: &mut DocTab,
    page: usize,
    band: [f32; 4],
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
    // The letters as the fonts of the page say them (the viewer may only guess some).
    let letters = match guarded(|| godpdf_core::run_text(tab.session.bytes(), page, area)) {
        Ok(letters) => letters,
        Err(err) => return fail(err.to_string(), cx),
    };
    // Only to show the field like the page; a run whose look cannot be read still opens.
    let look: Option<LineLook> =
        guarded(|| godpdf_core::line_look(tab.session.bytes(), page, area)).ok();
    // Each letter is as the page draws it where it is.
    let marks: Vec<(char, CharStyle, usize)> = letters
        .into_iter()
        .map(|(letter, x)| {
            let (style, ink) = look
                .as_ref()
                .map_or_else(Default::default, |look| (look.style_at(x), look.ink_at(x)));
            (letter, style, ink)
        })
        .collect();
    let text: String = marks.iter().map(|(letter, ..)| *letter).collect();
    let styles: Vec<CharStyle> = marks.iter().map(|(_, style, _)| *style).collect();
    let inks: Vec<usize> = marks.iter().map(|(.., ink)| *ink).collect();
    let inks_of_look = look.as_ref().map_or(&[None][..], |look| &look.inks[..]);
    let colors = inks_of_look
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
        restyled: false,
        font: look.as_ref().map_or(Font::Times, |look| look.font),
        size: look.as_ref().map_or(band[3] - band[1], |look| look.size),
        colors,
        backdrop: page_backdrop(tab, page, band),
        _events,
    });
}

/// Write the run when it changed: nothing (and no undo step) when it did not.
pub fn commit(tab: &mut DocTab, cx: &mut Context<DocTab>) {
    let Some(edit) = tab.text.line.take().filter(LineEdit::changed) else {
        return;
    };
    let result = tab.apply("Edit text", |pdf| {
        if edit.restyled {
            godpdf_core::replace_styled(
                pdf,
                edit.page,
                edit.area,
                &edit.marked,
                &edit.styles,
                &edit.inks,
            )
        } else {
            godpdf_core::replace_run(pdf, edit.page, edit.area, &edit.marked)
        }
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
    line.restyled = true;
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
    // shortcut: one font style and colour for the whole field (not each word's own);
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
    // One line of the font, ascenders to descenders: the field never cuts its letters.
    let line_px = font_px * FIELD_LINE;
    let height = ((y1 - y0) * zoom).max(line_px);
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
        .bg(line.backdrop)
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
                // The input's own sizes (a 20 px line, 32 px high, 12 px in) are for UI
                // text: at the size of the page they would cut big letters and shift them.
                .h(px(line_px))
                .px(px(0.0))
                .py(px(0.0))
                .line_height(px(line_px))
                .text_size(px(font_px))
                .font_family(font_family(line.font))
                .text_color(color)
                .when(most(Format::Bold), |text| {
                    text.font_weight(FontWeight::BOLD)
                })
                .when(most(Format::Italic), |text| text.italic())
                .when(most(Format::Underline), |text| text.underline()),
        );

    // A popup frame (margin 4) 6 px above the line, its three toggles 2 px apart.
    let active = line.active(cx);
    let bar_height = widgets::ICON_BUTTON + 8.0 + 2.0;
    let toggles = [
        (Format::Bold, icons::TEXT_B, "Bold (Ctrl+B)"),
        (Format::Italic, icons::TEXT_ITALIC, "Italic (Ctrl+I)"),
        (
            Format::Underline,
            icons::TEXT_UNDERLINE,
            "Underline (Ctrl+U)",
        ),
    ]
    .into_iter()
    .map(|(format, glyph, tip)| {
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

/// The band (displayed points) of the run of text under the pointer at `point`.
pub fn line_at(chars: &[godpdf_render::TextChar], point: (f32, f32)) -> Option<[f32; 4]> {
    let run = sel::run_of(chars, sel::char_near(chars, point)?);
    sel::bands(chars, run).into_iter().reduce(|all, band| {
        [
            all[0].min(band[0]),
            all[1].min(band[1]),
            all[2].max(band[2]),
            all[3].max(band[3]),
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::{backdrop, reconcile};

    /// A `width` x `height` BGRA bitmap of `paper`, with `ink` from `from` to `to` (pixels).
    fn bitmap(width: usize, height: usize, paper: [u8; 3], ink: ([u8; 3], [usize; 4])) -> Vec<u8> {
        let (color, [x0, y0, x1, y1]) = ink;
        let mut bgra = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for x in 0..width {
                let [r, g, b] = if (x0..x1).contains(&x) && (y0..y1).contains(&y) {
                    color
                } else {
                    paper
                };
                bgra.extend([b, g, r, 255]);
            }
        }
        bgra
    }

    #[test]
    fn the_field_takes_the_colour_around_the_run() {
        let navy = [0x16, 0x21, 0x3E];
        // White text on a dark band: the band is around the run, the page white further out.
        let bgra = bitmap(100, 60, [255; 3], (navy, [10, 10, 90, 50]));
        assert_eq!(backdrop(&bgra, 100, 60, [20.0, 20.0, 80.0, 40.0]), navy);
        // Black text on a white page.
        let bgra = bitmap(100, 60, [255; 3], ([0; 3], [20, 20, 80, 40]));
        assert_eq!(backdrop(&bgra, 100, 60, [20.0, 20.0, 80.0, 40.0]), [255; 3]);
        // Off the bitmap: white.
        assert_eq!(
            backdrop(&bgra, 100, 60, [200.0, 200.0, 300.0, 300.0]),
            [255; 3]
        );
    }

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
