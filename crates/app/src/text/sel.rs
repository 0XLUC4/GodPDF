//! Page text selection: picking characters under
//! the pointer, the line bands of a selected range, and the text to copy. All geometry is in
//! displayed page points; a band is `[x0, y0, x1, y1]`.

use std::ops::RangeInclusive;

use godpdf_render::TextChar;

/// A range of characters of one page between two carets. Caret `i` sits before character
/// `i` of its [`TextChar`] list (`len` is after the last one), so the list order (the reading
/// order of the page) runs from one line to the next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSel {
    pub page: usize,
    /// Where the drag started.
    pub anchor: usize,
    /// Where the pointer is now.
    pub head: usize,
}

impl TextSel {
    /// The selected characters; `None` while both carets meet.
    pub fn range(&self) -> Option<RangeInclusive<usize>> {
        let (start, end) = (self.anchor.min(self.head), self.anchor.max(self.head));
        (end > start).then(|| start..=end - 1)
    }
}

/// Distance from `pos` to a character cell (0 inside). Vertical distance weighs more, so the
/// pointer picks a line first, then a character on it.
fn distance(ch: &TextChar, (x, y): (f32, f32)) -> f32 {
    let dx = (ch.x0 - x).max(x - ch.x1).max(0.0);
    let dy = (ch.y0 - y).max(y - ch.y1).max(0.0);
    dx + 4.0 * dy
}

/// Index of the character closest to `pos`.
pub fn nearest(chars: &[TextChar], pos: (f32, f32)) -> Option<usize> {
    chars
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| distance(a, pos).total_cmp(&distance(b, pos)))
        .map(|(index, _)| index)
}

/// The character at `pos`, if the pointer is on text or within half a line of it.
pub fn char_near(chars: &[TextChar], pos: (f32, f32)) -> Option<usize> {
    let index = nearest(chars, pos)?;
    let ch = &chars[index];
    (distance(ch, pos) <= (ch.y1 - ch.y0) * 0.5).then_some(index)
}

/// The caret nearest `pos`: before or after the closest character, by the half it is on.
pub fn caret(chars: &[TextChar], pos: (f32, f32)) -> Option<usize> {
    let index = nearest(chars, pos)?;
    let ch = &chars[index];
    Some(index + usize::from(pos.0 > (ch.x0 + ch.x1) / 2.0))
}

/// True when the gap before `next` reads as a space the PDF does not write.
fn spaced(prev: &TextChar, next: &TextChar) -> bool {
    next.x0 - prev.x1 > 0.25 * (next.y1 - next.y0)
}

fn in_word(ch: &TextChar) -> bool {
    !ch.text.is_empty()
        && ch
            .text
            .chars()
            .all(|letter| letter.is_alphanumeric() || letter == '_')
}

/// The word `index` is in: the letters and digits around it on its line, up to a space,
/// punctuation or a gap. A character outside any word is a word of its own.
pub fn word_of(chars: &[TextChar], index: usize) -> RangeInclusive<usize> {
    let joined = |at: usize| {
        let (prev, next) = (&chars[at - 1], &chars[at]);
        in_word(prev) && in_word(next) && same_line(prev, next) && !spaced(prev, next)
    };
    let mut first = index.min(chars.len().saturating_sub(1));
    let mut last = first;
    while first > 0 && joined(first) {
        first -= 1;
    }
    while last + 1 < chars.len() && joined(last + 1) {
        last += 1;
    }
    first..=last
}

/// True when `next` continues the line of `prev` (same vertical band, not before it).
fn same_line(prev: &TextChar, next: &TextChar) -> bool {
    let overlap = prev.y1.min(next.y1) - prev.y0.max(next.y0);
    let height = (prev.y1 - prev.y0).min(next.y1 - next.y0);
    overlap > 0.5 * height && next.x0 >= prev.x0
}

/// The characters of the line `index` is on.
pub fn line_of(chars: &[TextChar], index: usize) -> RangeInclusive<usize> {
    let joined = |at: usize| same_line(&chars[at - 1], &chars[at]);
    let mut first = index.min(chars.len().saturating_sub(1));
    let mut last = first;
    while first > 0 && joined(first) {
        first -= 1;
    }
    while last + 1 < chars.len() && joined(last + 1) {
        last += 1;
    }
    first..=last
}

/// One band per line covered by `range`.
pub fn bands(chars: &[TextChar], range: RangeInclusive<usize>) -> Vec<[f32; 4]> {
    let Some(selected) = chars.get(range) else {
        return Vec::new();
    };
    let mut bands: Vec<[f32; 4]> = Vec::new();
    let mut prev: Option<&TextChar> = None;
    for ch in selected {
        match (bands.last_mut(), prev) {
            (Some(band), Some(prev)) if same_line(prev, ch) => {
                *band = [
                    band[0].min(ch.x0),
                    band[1].min(ch.y0),
                    band[2].max(ch.x1),
                    band[3].max(ch.y1),
                ];
            }
            _ => bands.push([ch.x0, ch.y0, ch.x1, ch.y1]),
        }
        prev = Some(ch);
    }
    bands
}

/// The text of `range`, with line breaks and the spaces the PDF only implies by position.
pub fn text(chars: &[TextChar], range: RangeInclusive<usize>) -> String {
    spelled(chars, range)
        .into_iter()
        .map(|(letter, _)| letter)
        .collect()
}

/// The text of `range` as [`text`] gives it, letter by letter, each with the middle of the
/// glyph it comes from. What the PDF only implies is where the glyph before it ends.
pub fn spelled(chars: &[TextChar], range: RangeInclusive<usize>) -> Vec<(char, (f32, f32))> {
    let Some(selected) = chars.get(range) else {
        return Vec::new();
    };
    let mut out: Vec<(char, (f32, f32))> = Vec::new();
    let mut prev: Option<&TextChar> = None;
    for ch in selected {
        if let Some(prev) = prev {
            let end = (prev.x1, (prev.y0 + prev.y1) / 2.0);
            if !same_line(prev, ch) {
                out.push(('\n', end));
            } else if spaced(prev, ch)
                && !out.last().is_some_and(|(last, _)| last.is_whitespace())
                && !ch.text.starts_with(char::is_whitespace)
            {
                out.push((' ', end));
            }
        }
        let middle = ((ch.x0 + ch.x1) / 2.0, (ch.y0 + ch.y1) / 2.0);
        out.extend(ch.text.chars().map(|letter| (letter, middle)));
        prev = Some(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two lines: "ab cd" (a gap before "c", no space glyph) and "ef".
    fn chars() -> Vec<TextChar> {
        let ch = |text: &str, x0: f32, y0: f32| TextChar {
            text: text.into(),
            x0,
            y0,
            x1: x0 + 6.0,
            y1: y0 + 12.0,
        };
        vec![
            ch("a", 10.0, 10.0),
            ch("b", 16.0, 10.0),
            ch("c", 28.0, 10.0),
            ch("d", 34.0, 10.0),
            ch("e", 10.0, 30.0),
            ch("f", 16.0, 30.0),
        ]
    }

    #[test]
    fn picks_the_line_first_then_the_character() {
        let chars = chars();
        assert_eq!(nearest(&chars, (17.0, 15.0)), Some(1));
        assert_eq!(nearest(&chars, (300.0, 36.0)), Some(5));
        assert_eq!(char_near(&chars, (12.0, 12.0)), Some(0));
        assert_eq!(char_near(&chars, (300.0, 300.0)), None);
        assert_eq!(nearest(&[], (0.0, 0.0)), None);
    }

    #[test]
    fn a_line_is_found_from_any_of_its_characters() {
        let chars = chars();
        assert_eq!((line_of(&chars, 0), line_of(&chars, 2)), (0..=3, 0..=3));
        assert_eq!((line_of(&chars, 4), line_of(&chars, 5)), (4..=5, 4..=5));
        assert_eq!(text(&chars, line_of(&chars, 3)), "ab cd");
        let middles: Vec<f32> = spelled(&chars, 0..=3).iter().map(|(_, at)| at.0).collect();
        assert_eq!(middles, [13.0, 19.0, 22.0, 31.0, 37.0]);
    }

    #[test]
    fn one_band_per_line() {
        let chars = chars();
        assert_eq!(
            bands(&chars, 1..=4),
            [[16.0, 10.0, 40.0, 22.0], [10.0, 30.0, 16.0, 42.0]]
        );
        assert!(bands(&chars, 4..=99).is_empty());
    }

    #[test]
    fn copied_text_gets_implied_spaces_and_line_breaks() {
        let chars = chars();
        assert_eq!(text(&chars, 0..=5), "ab cd\nef");
        let sel = TextSel {
            page: 0,
            anchor: 4,
            head: 1,
        };
        assert_eq!(text(&chars, sel.range().unwrap()), "b cd");
        assert_eq!(TextSel { head: 4, ..sel }.range(), None);
    }

    #[test]
    fn carets_fall_between_characters() {
        let chars = chars();
        // Left / right half of "b" (16..22).
        assert_eq!(caret(&chars, (17.0, 15.0)), Some(1));
        assert_eq!(caret(&chars, (21.0, 15.0)), Some(2));
        // Past the end of a line, at the end of the text.
        assert_eq!(caret(&chars, (300.0, 15.0)), Some(4));
        assert_eq!(caret(&chars, (300.0, 36.0)), Some(6));
        assert_eq!(caret(&[], (0.0, 0.0)), None);
    }

    #[test]
    fn a_drag_runs_across_lines_in_reading_order() {
        let chars = chars();
        // From the right half of "c" on line one to the left half of "f" on line two.
        let sel = TextSel {
            page: 0,
            anchor: caret(&chars, (33.0, 15.0)).unwrap(),
            head: caret(&chars, (17.0, 35.0)).unwrap(),
        };
        let range = sel.range().unwrap();
        assert_eq!(
            text(&chars, range.clone()),
            "d
e"
        );
        assert_eq!(
            bands(&chars, range),
            [[34.0, 10.0, 40.0, 22.0], [10.0, 30.0, 16.0, 42.0]]
        );
        // Dragged backwards, the same characters.
        let back = TextSel {
            anchor: sel.head,
            head: sel.anchor,
            ..sel
        };
        assert_eq!(back.range(), Some(3..=4));
    }

    #[test]
    fn a_word_ends_at_gaps_spaces_and_punctuation() {
        let ch = |text: &str, x0: f32| TextChar {
            text: text.into(),
            x0,
            y0: 0.0,
            x1: x0 + 6.0,
            y1: 12.0,
        };
        // "ab, cd" written with a space glyph, then "ef" after a gap.
        let chars = vec![
            ch("a", 0.0),
            ch("b", 6.0),
            ch(",", 12.0),
            ch(" ", 18.0),
            ch("c", 24.0),
            ch("d", 30.0),
            ch("e", 48.0),
            ch("f", 54.0),
        ];
        assert_eq!(word_of(&chars, 1), 0..=1);
        assert_eq!(word_of(&chars, 2), 2..=2);
        assert_eq!(word_of(&chars, 4), 4..=5);
        assert_eq!(word_of(&chars, 7), 6..=7);
        assert_eq!(text(&chars, line_of(&chars, 0)), "ab, cd ef");
    }
}
