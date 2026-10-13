//! Page text selection: picking characters under the pointer, the line bands of a selected
//! range, and the text to copy. All geometry is in displayed page points.

use std::ops::RangeInclusive;

use egui::{Pos2, Rect, pos2};
use godpdf_render::TextChar;

/// A range of characters of one page, by index into its [`TextChar`] list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSel {
    pub page: usize,
    /// Where the drag started.
    pub anchor: usize,
    /// Where the pointer is now.
    pub head: usize,
}

impl TextSel {
    pub fn range(&self) -> RangeInclusive<usize> {
        self.anchor.min(self.head)..=self.anchor.max(self.head)
    }
}

fn cell(ch: &TextChar) -> Rect {
    Rect::from_min_max(pos2(ch.x0, ch.y0), pos2(ch.x1, ch.y1))
}

/// Distance from `pos` to a character cell (0 inside). Vertical distance weighs more, so the
/// pointer picks a line first, then a character on it.
fn distance(ch: &TextChar, pos: Pos2) -> f32 {
    let dx = (ch.x0 - pos.x).max(pos.x - ch.x1).max(0.0);
    let dy = (ch.y0 - pos.y).max(pos.y - ch.y1).max(0.0);
    dx + 4.0 * dy
}

/// Index of the character closest to `pos`.
pub fn nearest(chars: &[TextChar], pos: Pos2) -> Option<usize> {
    chars
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| distance(a, pos).total_cmp(&distance(b, pos)))
        .map(|(index, _)| index)
}

/// The character at `pos`, if the pointer is on text or within half a line of it.
pub fn char_near(chars: &[TextChar], pos: Pos2) -> Option<usize> {
    let index = nearest(chars, pos)?;
    let ch = &chars[index];
    (distance(ch, pos) <= (ch.y1 - ch.y0) * 0.5).then_some(index)
}

/// True when `next` continues the line of `prev` (same vertical band, not before it).
fn same_line(prev: &TextChar, next: &TextChar) -> bool {
    let overlap = prev.y1.min(next.y1) - prev.y0.max(next.y0);
    let height = (prev.y1 - prev.y0).min(next.y1 - next.y0);
    overlap > 0.5 * height && next.x0 >= prev.x0
}

/// The characters of the line `index` is on: those before and after it that follow one
/// another on the same line.
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

/// Where `query` is written in `chars`: one range per occurrence, none overlapping.
///
/// Case and white space are ignored on both sides: a PDF often only implies its spaces by
/// position, and breaks a phrase over two lines.
pub fn find(chars: &[TextChar], query: &str) -> Vec<RangeInclusive<usize>> {
    let fold = |text: &str| -> Vec<char> {
        let kept = text.chars().filter(|ch| !ch.is_whitespace());
        kept.flat_map(char::to_lowercase).collect()
    };
    let needle = fold(query);
    // Each folded char of the page, with the index of the glyph it comes from.
    let glyphs = chars.iter().enumerate();
    let hay: Vec<(char, usize)> = glyphs
        .flat_map(|(index, ch)| fold(&ch.text).into_iter().map(move |ch| (ch, index)))
        .collect();
    let mut found = Vec::new();
    let mut at = 0;
    while !needle.is_empty() && at + needle.len() <= hay.len() {
        let here = &hay[at..at + needle.len()];
        if here.iter().map(|(ch, _)| ch).eq(needle.iter()) {
            found.push(here[0].1..=here[needle.len() - 1].1);
            at += needle.len();
        } else {
            at += 1;
        }
    }
    found
}

/// One rectangle per line covered by `range`.
pub fn bands(chars: &[TextChar], range: RangeInclusive<usize>) -> Vec<Rect> {
    let Some(selected) = chars.get(range) else {
        return Vec::new();
    };
    let mut bands: Vec<Rect> = Vec::new();
    let mut prev: Option<&TextChar> = None;
    for ch in selected {
        match (bands.last_mut(), prev) {
            (Some(band), Some(prev)) if same_line(prev, ch) => *band = band.union(cell(ch)),
            _ => bands.push(cell(ch)),
        }
        prev = Some(ch);
    }
    bands
}

/// The text of `range`, with line breaks and the spaces the PDF only implies by position.
pub fn text(chars: &[TextChar], range: RangeInclusive<usize>) -> String {
    let spelled = spelled(chars, range).into_iter();
    spelled.map(|(letter, _)| letter).collect()
}

/// The text of `range` as [`text`] gives it, letter by letter, each with the middle of the
/// glyph it comes from. What the PDF only implies is where the glyph before it ends.
pub fn spelled(chars: &[TextChar], range: RangeInclusive<usize>) -> Vec<(char, Pos2)> {
    let Some(selected) = chars.get(range) else {
        return Vec::new();
    };
    let mut out: Vec<(char, Pos2)> = Vec::new();
    let mut prev: Option<&TextChar> = None;
    for ch in selected {
        if let Some(prev) = prev {
            let end = pos2(prev.x1, (prev.y0 + prev.y1) / 2.0);
            if !same_line(prev, ch) {
                out.push(('\n', end));
            } else if ch.x0 - prev.x1 > 0.25 * (ch.y1 - ch.y0)
                && !out.last().is_some_and(|(last, _)| last.is_whitespace())
                && !ch.text.starts_with(char::is_whitespace)
            {
                out.push((' ', end));
            }
        }
        let middle = cell(ch).center();
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
        assert_eq!(nearest(&chars, pos2(17.0, 15.0)), Some(1));
        // Far right of line 2: its last character, not something on line 1.
        assert_eq!(nearest(&chars, pos2(300.0, 36.0)), Some(5));
        assert_eq!(char_near(&chars, pos2(12.0, 12.0)), Some(0));
        assert_eq!(char_near(&chars, pos2(300.0, 300.0)), None);
        assert_eq!(nearest(&[], pos2(0.0, 0.0)), None);
    }

    #[test]
    fn a_query_is_found_whatever_its_case_and_spaces() {
        let chars = chars();
        // "ab cd" has no space glyph, and "d e" runs over two lines.
        assert_eq!(find(&chars, "B C"), [1..=2]);
        assert_eq!(find(&chars, "abcd"), [0..=3]);
        assert_eq!(find(&chars, "d e"), [3..=4]);
        assert!(find(&chars, "x").is_empty() && find(&chars, "  ").is_empty());
        let twice: Vec<TextChar> = chars.iter().chain(&chars).cloned().collect();
        assert_eq!(find(&twice, "ef"), [4..=5, 10..=11]);
    }

    #[test]
    fn a_line_is_found_from_any_of_its_characters() {
        let chars = chars();
        assert_eq!((line_of(&chars, 0), line_of(&chars, 2)), (0..=3, 0..=3));
        assert_eq!((line_of(&chars, 4), line_of(&chars, 5)), (4..=5, 4..=5));
        assert_eq!(text(&chars, line_of(&chars, 3)), "ab cd");
        // Each letter is where its glyph is; the implied space, where "b" ends.
        let middles: Vec<f32> = spelled(&chars, 0..=3).iter().map(|(_, at)| at.x).collect();
        assert_eq!(middles, [13.0, 19.0, 22.0, 31.0, 37.0]);
    }

    #[test]
    fn one_band_per_line() {
        let chars = chars();
        let bands = bands(&chars, 1..=4);
        assert_eq!(bands.len(), 2);
        assert_eq!(
            bands[0],
            Rect::from_min_max(pos2(16.0, 10.0), pos2(40.0, 22.0))
        );
        assert_eq!(
            bands[1],
            Rect::from_min_max(pos2(10.0, 30.0), pos2(16.0, 42.0))
        );
        assert!(super::bands(&chars, 4..=99).is_empty());
    }

    #[test]
    fn copied_text_gets_implied_spaces_and_line_breaks() {
        let chars = chars();
        assert_eq!(text(&chars, 0..=5), "ab cd\nef");
        let sel = TextSel {
            page: 0,
            anchor: 3,
            head: 1,
        };
        assert_eq!(text(&chars, sel.range()), "b cd");
    }
}
