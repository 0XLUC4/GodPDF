//! Changing a line the page draws, where it is: each word as bold, as slanted, as underlined
//! and as raised as the page had it, in its colour, and as the keys or the small bar above the
//! line make it.

use std::ops::Range;

use egui::text::LayoutJob;
use egui::{
    Color32, Frame, Id, Key, Margin, Modifiers, Order, Rect, TextEdit, Ui, UiBuilder, vec2,
};
use godpdf_core::{CharStyle, Font, style_at};

use crate::textbox::{Format, Look, common_style, text_format, toggle_all};
use crate::theme::Palette;

pub struct LineEdit {
    pub tab: u64,
    pub page: usize,
    pub band: Rect,
    pub area: godpdf_core::Rect,
    pub text: String,
    /// One per character of `text`.
    styles: Vec<CharStyle>,
    /// The colour of each character of `text`: which of `colors`.
    inks: Vec<usize>,
    /// The words, their styles and their colours when the line was opened.
    original: (String, Vec<CharStyle>, Vec<usize>),
    /// The words `styles` and `inks` were last matched to.
    marked_text: String,
    /// The style and the colour of what is typed next.
    typing: (CharStyle, usize),
    selection: Range<usize>,
    font: Font,
    size: f32,
    /// The colours of the line, as the page has them.
    colors: Vec<Color32>,
    focus_pending: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Commit,
    Cancel,
}

impl LineEdit {
    /// `letters`: each character of the line, its style and its colour in `look.inks`.
    pub fn new(
        tab: u64,
        page: usize,
        band: Rect,
        area: godpdf_core::Rect,
        letters: Vec<(char, CharStyle, usize)>,
        look: &godpdf_core::LineLook,
    ) -> Self {
        let channel = |value: f32| (value * 255.0).round() as u8;
        let colors = look.inks.iter().map(|ink| {
            ink.map_or(Color32::BLACK, |color| {
                Color32::from_rgb(channel(color.r), channel(color.g), channel(color.b))
            })
        });
        let text: String = letters.iter().map(|(letter, ..)| *letter).collect();
        let styles: Vec<CharStyle> = letters.iter().map(|(_, style, _)| *style).collect();
        let inks: Vec<usize> = letters.iter().map(|(.., ink)| *ink).collect();
        let end = letters.len();
        Self {
            tab,
            page,
            band,
            area,
            original: (text.clone(), styles.clone(), inks.clone()),
            marked_text: text.clone(),
            typing: letters
                .last()
                .map_or_else(Default::default, |(_, style, ink)| (*style, *ink)),
            text,
            styles,
            inks,
            selection: end..end,
            font: look.font,
            size: look.size,
            colors: colors.collect(),
            focus_pending: true,
        }
    }

    /// One style per character of the line.
    pub fn styles(&self) -> &[CharStyle] {
        &self.styles
    }

    /// The colour of each character of the line: which of the inks of its look.
    pub fn inks(&self) -> &[usize] {
        &self.inks
    }

    pub fn changed(&self) -> bool {
        (&self.text, &self.styles, &self.inks)
            != (&self.original.0, &self.original.1, &self.original.2)
    }

    /// The style of the selected words, or of what is typed next.
    fn active(&self) -> CharStyle {
        if self.selection.is_empty() {
            self.typing.0
        } else {
            common_style(&self.styles, self.selection.clone())
        }
    }

    fn toggle(&mut self, format: Format) {
        if self.selection.is_empty() {
            let on = !format.is_on(self.typing.0);
            format.set(&mut self.typing.0, on);
        } else {
            let len = self.text.chars().count();
            toggle_all(&mut self.styles, len, self.selection.clone(), format);
            // `toggle_all` empties a list of plain styles: here there is one per character.
            self.styles.resize(len, CharStyle::default());
        }
        self.focus_pending = true;
    }

    fn id(&self) -> Id {
        Id::new(("line-edit", self.tab))
    }
}

/// Have `marks` (one per character of `old`) follow the words to `new`: what was typed is
/// marked `typing`, what stayed keeps its mark.
fn reconcile<T: Copy>(marks: &mut Vec<T>, old: &str, new: &str, typing: T) {
    let old: Vec<char> = old.chars().collect();
    let new: Vec<char> = new.chars().collect();
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    marks.resize(old.len(), typing);
    let kept = marks.split_off(old.len() - suffix);
    marks.truncate(prefix);
    marks.extend(std::iter::repeat_n(typing, new.len() - prefix - suffix));
    marks.extend(kept);
}

/// The line as it is laid out: one run for each stretch of one style and one colour.
fn line_job(
    text: &str,
    styles: &[CharStyle],
    inks: &[usize],
    look: &Look,
    colors: &[Color32],
) -> LayoutJob {
    let mark = |index: usize| {
        let ink = inks.get(index).copied().unwrap_or(0);
        (style_at(styles, index), ink)
    };
    let mut job = LayoutJob::default();
    let mut append = |run: &mut String, (style, ink): (CharStyle, usize)| {
        let mut format = text_format(look, style);
        format.color = colors.get(ink).copied().unwrap_or(Color32::BLACK);
        format.underline.color = format.color;
        job.append(&std::mem::take(run), 0.0, format);
    };
    let mut run = String::new();
    let mut run_mark = mark(0);
    for (index, letter) in text.chars().enumerate() {
        if !run.is_empty() && mark(index) != run_mark {
            append(&mut run, run_mark);
        }
        run_mark = mark(index);
        run.push(letter);
    }
    append(&mut run, run_mark);
    job.wrap.max_width = f32::INFINITY;
    job
}

pub fn show(ui: &mut Ui, edit: &mut LineEdit, page_rect: Rect, zoom: f32) -> Option<Event> {
    let id = edit.id();
    let look = Look::new(ui.painter(), edit.font, edit.size, zoom, Color32::BLACK);
    let band = Rect::from_min_size(
        page_rect.min + edit.band.min.to_vec2() * zoom,
        edit.band.size() * zoom,
    );
    let job = line_job(&edit.text, &edit.styles, &edit.inks, &look, &edit.colors);
    let galley = ui.painter().layout_job(job);
    let width = band.width().max(galley.size().x + edit.size * zoom);
    let height = band.height().max(galley.size().y);
    let field = Rect::from_min_size(
        band.left_center() - vec2(0.0, height / 2.0),
        vec2(width, height),
    );
    let cover = field.union(band).expand(2.0);
    ui.painter().rect_filled(cover, 2.0, Color32::WHITE);
    ui.painter().rect_stroke(
        cover,
        2.0,
        egui::Stroke::new(1.0, Color32::from_rgb(0x25, 0x63, 0xEB)),
        egui::StrokeKind::Outside,
    );

    let focused = ui.memory(|memory| memory.has_focus(id));
    let (enter, escape, mut formats) = ui.input_mut(|input| {
        let enter = focused && input.consume_key(Modifiers::NONE, Key::Enter);
        let formats: Vec<Format> = [
            (Key::B, Format::Bold),
            (Key::I, Format::Italic),
            (Key::U, Format::Underline),
        ]
        .into_iter()
        .filter(|(key, _)| focused && input.consume_key(Modifiers::COMMAND, *key))
        .map(|(_, format)| format)
        .collect();
        (enter, input.key_pressed(Key::Escape), formats)
    });
    formats.extend(format_bar(ui, id, cover, edit.active()));
    for format in formats {
        edit.toggle(format);
    }

    let focus_pending = std::mem::take(&mut edit.focus_pending);
    let cursor = {
        let mut child = ui.new_child(UiBuilder::new().max_rect(field).id_salt(id.with("field")));
        let LineEdit {
            text,
            styles,
            inks,
            marked_text,
            typing,
            colors,
            ..
        } = edit;
        let typing = *typing;
        let mut layouter = |ui: &Ui, buffer: &dyn egui::TextBuffer, _wrap: f32| {
            let typed = buffer.as_str();
            reconcile(styles, marked_text, typed, typing.0);
            reconcile(inks, marked_text, typed, typing.1);
            typed.clone_into(marked_text);
            let job = line_job(typed, styles, inks, &look, colors);
            ui.painter().layout_job(job)
        };
        let output = TextEdit::singleline(text)
            .id(id)
            .frame(Frame::NONE)
            .margin(Margin::ZERO)
            .desired_width(width)
            .layouter(&mut layouter)
            .show(&mut child);
        if focus_pending {
            output.response.request_focus();
        }
        output.cursor_range.map(|range| {
            let range = range.as_sorted_char_range();
            range.start.0..range.end.0
        })
    };
    reconcile(
        &mut edit.styles,
        &edit.marked_text,
        &edit.text,
        edit.typing.0,
    );
    reconcile(&mut edit.inks, &edit.marked_text, &edit.text, edit.typing.1);
    edit.text.clone_into(&mut edit.marked_text);
    if let Some(cursor) = cursor {
        // What is typed next is like what is before the caret, or like the start of what is
        // selected, which it takes the place of.
        if cursor != edit.selection {
            let at = if cursor.is_empty() {
                cursor.start.saturating_sub(1)
            } else {
                cursor.start
            };
            let ink = edit.inks.get(at).copied().unwrap_or(0);
            edit.typing = (style_at(&edit.styles, at), ink);
        }
        edit.selection = cursor;
    }

    if escape {
        return Some(Event::Cancel);
    }
    if enter {
        return Some(Event::Commit);
    }
    let clicked_outside = ui.input(|input| {
        input
            .pointer
            .any_click()
            .then(|| input.pointer.interact_pos())
            .flatten()
    });
    clicked_outside
        .is_some_and(|pos| {
            ui.clip_rect().contains(pos)
                && !cover.contains(pos)
                && ui.ctx().layer_id_at(pos) == Some(ui.layer_id())
        })
        .then_some(Event::Commit)
}

/// The small bar above the line: what a click on it turns on or off.
fn format_bar(ui: &Ui, id: Id, cover: Rect, active: CharStyle) -> Option<Format> {
    let palette = Palette::of(ui);
    let height = crate::widgets::ICON_BUTTON.y + 12.0;
    let mut clicked = None;
    egui::Area::new(id.with("formats"))
        .order(Order::Foreground)
        .fixed_pos(cover.left_top() - vec2(0.0, height + 6.0))
        .show(ui.ctx(), |ui| {
            Frame::popup(ui.style())
                .inner_margin(Margin::same(4))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for format in Format::ALL {
                            let on = format.is_on(active);
                            if crate::quickbar::format_toggle(ui, format, on, &palette).clicked() {
                                clicked = Some(format);
                            }
                        }
                    });
                });
        });
    clicked
}

#[cfg(test)]
mod tests {
    use super::reconcile;

    #[test]
    fn marks_follow_the_words() {
        // Typed in the middle: the new letters take the mark of what is typed.
        let mut marks = vec![1, 1, 2, 2];
        reconcile(&mut marks, "abcd", "abXYcd", 9);
        assert_eq!(marks, [1, 1, 9, 9, 2, 2]);
        // Taken out: the others keep theirs.
        reconcile(&mut marks, "abXYcd", "ad", 9);
        assert_eq!(marks, [1, 2]);
        reconcile(&mut marks, "ad", "", 9);
        assert!(marks.is_empty());
    }
}
