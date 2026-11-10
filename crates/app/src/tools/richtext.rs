//! Free text drawn the way core writes its appearance: wrapped by `wrap_styled`, each run in
//! its own font, sub / superscripts smaller and moved off the baseline, underlines where core
//! puts them. Used by the text box being typed and by a text box being moved or resized.

use std::ops::Range;
use std::rc::Rc;

use godpdf_core::{
    CharStyle, FREE_TEXT_ASCENT, FREE_TEXT_LINE_HEIGHT, FREE_TEXT_PADDING, Font, style_at,
    wrap_styled,
};
use gpui::prelude::*;
use gpui::{
    AnyElement, Bounds, FontStyle, FontWeight, Hsla, TextAlign, TextRun, canvas, fill, point, px,
    size,
};

/// Characters of one style on one line. Points from the box's top-left corner.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub chars: Range<usize>,
    pub style: CharStyle,
    pub line: usize,
    pub x: f32,
    pub width: f32,
    /// Where its glyphs sit: the line's baseline, raised or lowered for a script.
    pub baseline: f32,
}

/// A free text laid out in a box `width` points wide.
pub struct Layout {
    pub chars: Vec<char>,
    pub lines: Vec<Range<usize>>,
    pub runs: Vec<Run>,
    advances: Vec<f32>,
    font_size: f32,
}

impl Layout {
    pub fn new(text: &str, styles: &[CharStyle], font: Font, font_size: f32, width: f32) -> Self {
        let chars: Vec<char> = text.chars().collect();
        let advances: Vec<f32> = chars
            .iter()
            .enumerate()
            .map(|(index, &c)| font.char_width(c, style_at(styles, index), font_size))
            .collect();
        let lines = wrap_styled(text, styles, font, font_size, width);
        let mut runs = Vec::new();
        for (line, range) in lines.iter().enumerate() {
            let baseline = baseline(line, font_size);
            let mut x = FREE_TEXT_PADDING;
            let mut at = range.start;
            while at < range.end {
                let style = style_at(styles, at);
                let end = (at..range.end)
                    .find(|&index| style_at(styles, index) != style)
                    .unwrap_or(range.end);
                let width: f32 = advances[at..end].iter().sum();
                runs.push(Run {
                    chars: at..end,
                    style,
                    line,
                    x,
                    width,
                    baseline: baseline - style.rise(font_size),
                });
                x += width;
                at = end;
            }
        }
        Self {
            chars,
            lines,
            runs,
            advances,
            font_size,
        }
    }

    /// Height of the box: its lines and the padding.
    pub fn height(&self) -> f32 {
        self.lines.len().max(1) as f32 * self.line_height() + 2.0 * FREE_TEXT_PADDING
    }

    fn line_height(&self) -> f32 {
        self.font_size * FREE_TEXT_LINE_HEIGHT
    }

    /// Where the caret before character `index` goes: its x and its line.
    pub fn caret(&self, index: usize) -> (f32, usize) {
        let line = self
            .lines
            .iter()
            .position(|range| index <= range.end)
            .unwrap_or(self.lines.len().saturating_sub(1));
        let Some(range) = self.lines.get(line) else {
            return (FREE_TEXT_PADDING, 0);
        };
        let end = index.clamp(range.start, range.end);
        let x = FREE_TEXT_PADDING + self.advances[range.start..end].iter().sum::<f32>();
        (x, line)
    }

    fn line_top(&self, line: usize) -> f32 {
        FREE_TEXT_PADDING + line as f32 * self.line_height()
    }
}

/// Baseline of `line` below the box's top edge (core: first baseline under the padding).
fn baseline(line: usize, font_size: f32) -> f32 {
    FREE_TEXT_PADDING
        + FREE_TEXT_ASCENT * font_size
        + line as f32 * font_size * FREE_TEXT_LINE_HEIGHT
}

/// How the text is drawn: font family on screen, colour, and for the box being typed the
/// selected characters (an empty range is the caret).
pub struct Look {
    pub family: &'static str,
    pub color: Hsla,
    pub selection: Option<Range<usize>>,
}

/// Selected text fill, the caret colour.
const SELECTION: u32 = 0x2563EB;

/// `layout` drawn over the whole parent (the box), `zoom` logical pixels per point.
pub fn element(layout: Rc<Layout>, look: Look, zoom: f32) -> AnyElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, cx| {
            let at = |x: f32, y: f32| bounds.origin + point(px(x * zoom), px(y * zoom));
            let line_height = layout.line_height();
            if let Some(selection) = &look.selection {
                let blue: Hsla = gpui::rgb(SELECTION).into();
                if selection.is_empty() {
                    let (x, line) = layout.caret(selection.start);
                    let top = at(x, layout.line_top(line));
                    let caret = Bounds::new(top, size(px(1.5), px(line_height * zoom)));
                    window.paint_quad(fill(caret, look.color));
                }
                for (line, range) in layout.lines.iter().enumerate() {
                    let start = selection.start.max(range.start);
                    let end = selection.end.min(range.end);
                    if start >= end {
                        continue;
                    }
                    let (x0, _) = layout.caret(start);
                    let (x1, _) = layout.caret(end);
                    let band = Bounds::new(
                        at(x0, layout.line_top(line)),
                        size(px((x1 - x0) * zoom), px(line_height * zoom)),
                    );
                    window.paint_quad(fill(band, Hsla { a: 0.3, ..blue }));
                }
            }
            for run in &layout.runs {
                let text: String = layout.chars[run.chars.clone()].iter().collect();
                let font = gpui::Font {
                    weight: if run.style.bold {
                        FontWeight::BOLD
                    } else {
                        FontWeight::NORMAL
                    },
                    style: if run.style.italic {
                        FontStyle::Italic
                    } else {
                        FontStyle::Normal
                    },
                    ..gpui::font(look.family)
                };
                let text_run = TextRun {
                    len: text.len(),
                    font,
                    color: look.color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let font_px = px(run.style.size(layout.font_size) * zoom);
                let shaped =
                    window
                        .text_system()
                        .shape_line(text.into(), font_px, &[text_run], None);
                // gpui centres the glyphs in `line_height`: with ascent + descent the
                // baseline is exactly `ascent` below the origin.
                let origin = at(run.x, run.baseline) - point(px(0.0), shaped.ascent);
                let glyphs = shaped.ascent + shaped.descent;
                let _ = shaped.paint(origin, glyphs, TextAlign::Left, None, window, cx);
                if run.style.underline {
                    let y = baseline(run.line, layout.font_size) + 0.1 * layout.font_size;
                    let thick = (0.06 * layout.font_size * zoom).max(1.0);
                    let bar = Bounds::new(at(run.x, y), size(px(run.width * zoom), px(thick)));
                    window.paint_quad(fill(bar, look.color));
                }
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use godpdf_core::Script;

    #[test]
    fn scripts_are_smaller_and_off_the_baseline() {
        let sub = CharStyle {
            script: Script::Sub,
            ..CharStyle::default()
        };
        let sup = CharStyle {
            script: Script::Super,
            ..CharStyle::default()
        };
        let plain = CharStyle::default();
        let layout = Layout::new(
            "H2O x2",
            &[plain, sub, plain, plain, plain, sup],
            Font::Helvetica,
            10.0,
            500.0,
        );
        let runs = &layout.runs;
        assert_eq!(runs.len(), 4);
        let line = baseline(0, 10.0);
        assert_eq!(runs[0].baseline, line);
        // Subscript: 0.15 em lower, superscript 0.35 em higher (y grows downwards).
        assert!((runs[1].baseline - (line + 1.5)).abs() < 1e-4);
        assert!((runs[3].baseline - (line - 3.5)).abs() < 1e-4);
        // The subscript "2" takes 0.65 of a normal "2".
        let two = Font::Helvetica.char_width('2', plain, 10.0);
        assert!((runs[1].width - two * 0.65).abs() < 1e-4);
        assert!((runs[2].x - (runs[1].x + runs[1].width)).abs() < 1e-4);
    }

    #[test]
    fn caret_follows_the_wrapped_lines() {
        // 30 points minus padding: "aaa bbb" wraps after the space.
        let layout = Layout::new("aaa bbb", &[], Font::Courier, 10.0, 22.0);
        assert_eq!(layout.lines, [0..3, 4..7]);
        assert_eq!(layout.caret(0), (FREE_TEXT_PADDING, 0));
        // The space the line broke at belongs to the end of the first line.
        assert_eq!(layout.caret(3), (FREE_TEXT_PADDING + 18.0, 0));
        assert_eq!(layout.caret(4), (FREE_TEXT_PADDING, 1));
        assert_eq!(layout.caret(7), (FREE_TEXT_PADDING + 18.0, 1));
        let height = 2.0 * 10.0 * FREE_TEXT_LINE_HEIGHT + 2.0 * FREE_TEXT_PADDING;
        assert!((layout.height() - height).abs() < 1e-4);
        // Empty text: one line, caret at the padding.
        let empty = Layout::new("", &[], Font::Helvetica, 10.0, 100.0);
        assert_eq!(
            (empty.lines.len(), empty.caret(0)),
            (1, (FREE_TEXT_PADDING, 0))
        );
    }
}
