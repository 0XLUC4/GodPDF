//! Pure find logic: matches in a page's glyphs, their boxes, stepping and scrolling to them.

use std::ops::RangeInclusive;

use godpdf_render::TextChar;

/// Find bar options.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// Match case.
    pub case: bool,
    /// Whole words only.
    pub whole: bool,
}

/// Glyph ranges where `query` is written.
/// White space is ignored on both sides (a PDF often only implies its spaces by position,
/// and breaks a phrase over two lines); case too unless `options.case`.
pub fn find(chars: &[TextChar], query: &str, options: Options) -> Vec<RangeInclusive<usize>> {
    let fold = |text: &str| -> Vec<char> {
        let kept = text.chars().filter(|ch| !ch.is_whitespace());
        if options.case {
            kept.collect()
        } else {
            kept.flat_map(char::to_lowercase).collect()
        }
    };
    let needle = fold(query);
    // Each folded char of the page, with the index of the glyph it comes from.
    let hay: Vec<(char, usize)> = chars
        .iter()
        .enumerate()
        .flat_map(|(index, ch)| fold(&ch.text).into_iter().map(move |ch| (ch, index)))
        .collect();
    let mut found = Vec::new();
    let mut at = 0;
    while !needle.is_empty() && at + needle.len() <= hay.len() {
        let here = &hay[at..at + needle.len()];
        let (first, last) = (here[0].1, here[needle.len() - 1].1);
        if here.iter().map(|(ch, _)| ch).eq(needle.iter())
            && (!options.whole || whole_word(chars, first, last))
        {
            found.push(first..=last);
            at += needle.len();
        } else {
            at += 1;
        }
    }
    found
}

/// Glyphs `first..=last` stand as a word: a word break before and after them.
fn whole_word(chars: &[TextChar], first: usize, last: usize) -> bool {
    let before = first == 0 || word_break(&chars[first - 1], &chars[first]);
    let after = last + 1 >= chars.len() || word_break(&chars[last], &chars[last + 1]);
    before && after
}

/// A word ends between `prev` and `next`: a non-letter, another line, or a gap wider than
/// a letter spacing (spaces are often only implied by position).
fn word_break(prev: &TextChar, next: &TextChar) -> bool {
    let letter = |ch: Option<char>| ch.is_some_and(char::is_alphanumeric);
    !letter(prev.text.chars().last())
        || !letter(next.text.chars().next())
        || !same_line(prev, next)
        || next.x0 - prev.x1 > 0.15 * (next.y1 - next.y0)
}

fn same_line(prev: &TextChar, next: &TextChar) -> bool {
    let overlap = prev.y1.min(next.y1) - prev.y0.max(next.y0);
    let height = (prev.y1 - prev.y0).min(next.y1 - next.y0);
    overlap > 0.5 * height && next.x0 >= prev.x0
}

/// One `[x0, y0, x1, y1]` box (displayed points) per line covered by `range`.
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

/// Pages in search order: from `start` to the end, then from the first page. The first
/// match found is so the first one at or after the page being read.
pub fn scan_order(count: usize, start: usize) -> impl Iterator<Item = usize> {
    let start = start.min(count);
    (start..count).chain(0..start)
}

/// The match after `current` (before it when `!forward`) of `len`, wrapping around.
pub fn step(current: Option<usize>, len: usize, forward: bool) -> Option<usize> {
    if len == 0 {
        return None;
    }
    Some(match (current, forward) {
        (Some(at), true) => (at + 1) % len,
        (Some(at), false) => (at + len - 1) % len,
        (None, true) => 0,
        (None, false) => len - 1,
    })
}

/// Scroll change that shows `start..end` in a view `size` long scrolled to `scroll`:
/// none when it is already in sight, else it is centred.
pub fn reveal_delta(start: f32, end: f32, scroll: f32, size: f32) -> f32 {
    if start >= scroll && end <= scroll + size {
        0.0
    } else {
        (start + end) / 2.0 - size / 2.0 - scroll
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One glyph per char, 10 points wide, on line `line` (12 points high).
    fn line(text: &str, line: usize, start: usize) -> Vec<TextChar> {
        let top = line as f32 * 20.0;
        text.chars()
            .enumerate()
            .map(|(index, ch)| {
                let x0 = (start + index) as f32 * 10.0;
                TextChar {
                    text: ch.to_string(),
                    x0,
                    y0: top,
                    x1: x0 + 10.0,
                    y1: top + 12.0,
                }
            })
            .collect()
    }

    const ANY: Options = Options {
        case: false,
        whole: false,
    };

    #[test]
    fn finds_ignoring_case_and_spaces() {
        let chars = line("Hello World hello", 0, 0);
        assert_eq!(find(&chars, "HELLO", ANY), vec![0..=4, 12..=16]);
        assert_eq!(find(&chars, "loWor", ANY), vec![3..=8]);
        assert!(find(&chars, "   ", ANY).is_empty());
        assert!(find(&chars, "absent", ANY).is_empty());
    }

    #[test]
    fn match_case_and_whole_word() {
        let chars = line("Hello hello helloed", 0, 0);
        let case = Options { case: true, ..ANY };
        assert_eq!(find(&chars, "hello", case), vec![6..=10, 12..=16]);
        let whole = Options { whole: true, ..ANY };
        assert_eq!(find(&chars, "hello", whole), vec![0..=4, 6..=10]);
        // A space implied by a gap between glyphs also breaks words.
        let mut gapped = line("cat", 0, 0);
        gapped.extend(line("dog", 0, 4));
        assert_eq!(find(&gapped, "dog", whole), vec![3..=5]);
        assert!(find(&line("catdog", 0, 0), "dog", whole).is_empty());
    }

    #[test]
    fn a_hit_over_two_lines_gives_two_bands() {
        let mut chars = line("ab", 0, 5);
        chars.extend(line("cd", 1, 0));
        let found = find(&chars, "abcd", ANY);
        assert_eq!(found, vec![0..=3]);
        assert_eq!(
            bands(&chars, found[0].clone()),
            vec![[50.0, 0.0, 70.0, 12.0], [0.0, 20.0, 20.0, 32.0]]
        );
    }

    #[test]
    fn scans_from_the_current_page_then_wraps() {
        assert_eq!(scan_order(5, 3).collect::<Vec<_>>(), vec![3, 4, 0, 1, 2]);
        assert_eq!(scan_order(3, 0).collect::<Vec<_>>(), vec![0, 1, 2]);
        assert_eq!(scan_order(3, 9).collect::<Vec<_>>(), vec![0, 1, 2]);
        assert_eq!(scan_order(0, 0).count(), 0);
    }

    #[test]
    fn steps_wrap_around() {
        assert_eq!(step(Some(1), 3, true), Some(2));
        assert_eq!(step(Some(2), 3, true), Some(0));
        assert_eq!(step(Some(0), 3, false), Some(2));
        assert_eq!(step(None, 3, true), Some(0));
        assert_eq!(step(None, 3, false), Some(2));
        assert_eq!(step(Some(0), 0, true), None);
    }

    #[test]
    fn reveal_scrolls_only_when_out_of_sight() {
        assert_eq!(reveal_delta(120.0, 140.0, 100.0, 500.0), 0.0);
        // Below the view: its middle (1010) goes to the view middle (scroll + 250).
        assert_eq!(reveal_delta(1000.0, 1020.0, 100.0, 500.0), 660.0);
        // Partly cut off at the top: centred too.
        assert_eq!(reveal_delta(90.0, 110.0, 100.0, 500.0), -250.0);
    }
}
