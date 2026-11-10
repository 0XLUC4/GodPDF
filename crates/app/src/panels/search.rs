//! Search panel: find text in every page (in the background), list the pages with hits,
//! mark the hits on the pages, step through them with Enter / F3 / Shift+F3.

use std::collections::HashMap;
use std::ops::RangeInclusive;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use godpdf_core::SearchHit;
use godpdf_render::TextChar;
use gpui::component::input::{Input, InputEvent, InputState};
use gpui::component::spinner::Spinner;
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, Hsla, Rgba, Window, div, px};

use super::{Panel, label, open_panel, state, with_active};
use crate::doc::{DocTab, guarded};
use crate::theme::{self, Palette};
use crate::{icons, widgets};

/// Search of one document: the text of its field and the results.
#[derive(Default)]
pub struct Search {
    /// What is typed in the search field for this document (the field is shared by tabs).
    query: String,
    /// Query the hits belong to; empty before the first search.
    searched: String,
    hits: Vec<SearchHit>,
    error: Option<String>,
    busy: bool,
    /// The match stepped to: its page and its index among those [`find`] gives there.
    at: Option<(usize, usize)>,
    /// Document generation the hits and text positions were read from.
    generation: u64,
    /// Text positions of pages with hits, read once per generation.
    chars: HashMap<usize, Arc<Vec<TextChar>>>,
}

impl Search {
    /// Forget results read from older bytes (an edit may have moved the text).
    fn sync(&mut self, generation: u64) {
        if self.generation != generation {
            *self = Self {
                generation,
                query: std::mem::take(&mut self.query),
                ..Self::default()
            };
        }
    }
}

/// The search field, created on first use. What is typed goes to the document shown.
fn input(window: &mut Window, cx: &mut App) -> Entity<InputState> {
    if let Some((input, _)) = &state(cx).search_input {
        return input.clone();
    }
    let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search text…"));
    let subscription = cx.subscribe(&input, |input, event: &InputEvent, cx| match event {
        InputEvent::PressEnter { shift, .. } => run(!*shift, cx),
        InputEvent::Change => {
            let typed = input.read(cx).value().to_string();
            with_active(cx, |tab, _| tab.panels.search.query = typed);
        }
        _ => {}
    });
    state(cx).search_input = Some((input.clone(), subscription));
    input
}

/// Enter: search for the field's text, or go to the next match when it is already searched.
fn run(forward: bool, cx: &mut App) {
    with_active(cx, |tab, cx| {
        let query = tab.panels.search.query.trim().to_owned();
        if query.is_empty() {
            return;
        }
        let generation = tab.generation();
        let search = &mut tab.panels.search;
        search.sync(generation);
        if search.busy {
            return;
        }
        if search.searched == query && search.error.is_none() {
            go(tab, forward);
            return;
        }
        search.busy = true;
        let bytes = tab.session.bytes().to_vec();
        let found = crate::services::jobs::background(cx, async move {
            guarded(|| godpdf_core::search(&bytes, &query)).map(|hits| (query, hits))
        });
        cx.spawn(async move |this, cx| {
            let result = found.await;
            let _ = this.update(cx, |tab, cx| {
                let search = &mut tab.panels.search;
                if search.generation != generation {
                    return;
                }
                search.busy = false;
                search.at = None;
                match result {
                    Ok((query, hits)) => {
                        search.searched = query;
                        search.hits = hits;
                        search.error = None;
                        go(tab, true);
                    }
                    Err(err) => {
                        search.hits.clear();
                        search.error = Some(err.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    });
}

/// F3 / Shift+F3: next or previous match; searches first when needed.
pub fn step(forward: bool, cx: &mut App) {
    if open_panel(cx) != Some(Panel::Search) {
        super::show(Panel::Search, cx);
    }
    run(forward, cx);
}

/// Text positions of `page`, read once per generation.
fn page_chars(tab: &mut DocTab, page: usize) -> Arc<Vec<TextChar>> {
    if let Some(chars) = tab.panels.search.chars.get(&page) {
        return Arc::clone(chars);
    }
    let chars = Arc::new(read_chars(tab, page));
    tab.panels.search.chars.insert(page, Arc::clone(&chars));
    chars
}

/// Step to the next (or previous) match and scroll it into view.
fn go(tab: &mut DocTab, forward: bool) {
    let search = &tab.panels.search;
    let (pages, at, query) = (hit_pages(&search.hits), search.at, search.searched.clone());
    // shortcut: steps by core's count per page; a page whose glyphs read otherwise may get
    // a stop without a mark. Count with `find` if that shows up.
    let counts: HashMap<usize, usize> = search
        .hits
        .iter()
        .map(|hit| (hit.page, hit.count))
        .collect();
    let count = |page: usize| counts.get(&page).copied().unwrap_or(1);
    let Some((page, index)) = next_match(&pages, count, at, tab.view.current_page, forward) else {
        return;
    };
    tab.panels.search.at = Some((page, index));
    let chars = page_chars(tab, page);
    let band = find(&chars, &query)
        .get(index)
        .and_then(|found| bands(&chars, found.clone()).first().copied());
    let layout = tab.layout();
    let (Some(band), Some(rect)) = (band, layout.rects.get(page)) else {
        tab.scroll_to_page(page);
        return;
    };
    // Shown a third of the way down when it is out of sight.
    let view_height = f32::from(tab.view.bounds.size.height);
    let (top, bottom) = (
        rect.y + band[1] * layout.zoom,
        rect.y + band[3] * layout.zoom,
    );
    let scroll = tab.view.scroll.1;
    if top < scroll || bottom > scroll + view_height {
        tab.scroll_by((0.0, top - view_height / 3.0 - scroll));
    }
}

fn hit_pages(hits: &[SearchHit]) -> Vec<usize> {
    hits.iter().map(|hit| hit.page).collect()
}

/// The match after `at` (before it when `!forward`), wrapping around the document. Without
/// `at`, the first match from the page being read on (the last one before it backwards).
/// `pages` is ascending; `count` gives the number of matches of a page.
pub fn next_match(
    pages: &[usize],
    count: impl Fn(usize) -> usize,
    at: Option<(usize, usize)>,
    current: usize,
    forward: bool,
) -> Option<(usize, usize)> {
    let last = |page: usize| count(page).max(1) - 1;
    match at.filter(|(page, _)| pages.contains(page)) {
        Some((page, index)) if forward && index < last(page) => Some((page, index + 1)),
        Some((page, index)) if !forward && index > 0 => Some((page, index.min(last(page) + 1) - 1)),
        Some((page, _)) => {
            let next = next_page(pages, page, forward)?;
            Some((next, if forward { 0 } else { last(next) }))
        }
        None if forward => {
            let page = pages
                .iter()
                .copied()
                .find(|&page| page >= current)
                .or_else(|| pages.first().copied())?;
            Some((page, 0))
        }
        None => {
            let page = next_page(pages, current, false)?;
            Some((page, last(page)))
        }
    }
}

/// The page with hits after `current` (before it when `!forward`), wrapping around.
/// `pages` is ascending.
pub fn next_page(pages: &[usize], current: usize, forward: bool) -> Option<usize> {
    if forward {
        pages
            .iter()
            .copied()
            .find(|&page| page > current)
            .or_else(|| pages.first().copied())
    } else {
        pages
            .iter()
            .rev()
            .copied()
            .find(|&page| page < current)
            .or_else(|| pages.last().copied())
    }
}

pub fn render(tab: &Entity<DocTab>, window: &mut Window, cx: &mut App) -> AnyElement {
    let palette = Palette::of(cx);
    let field = input(window, cx);
    // The field shows the text typed for the document in front.
    let shown = tab.entity_id();
    if state(cx).search_tab != Some(shown) {
        state(cx).search_tab = Some(shown);
        let text = tab.read(cx).panels.search.query.clone();
        field.update(cx, |field, cx| field.set_value(text, window, cx));
    }
    if std::mem::take(&mut state(cx).focus_search) {
        field.update(cx, |field, cx| field.focus(window, cx));
    }
    let tab_ref = tab.read(cx);
    let typed = tab_ref.panels.search.query.trim().to_owned();
    let generation = tab_ref.generation();
    let current = tab_ref.view.current_page;
    let search = &tab_ref.panels.search;
    let fresh = search.generation == generation;

    let total: usize = search.hits.iter().map(|hit| hit.count).sum();
    let status = if search.busy && fresh {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(Spinner::new().color(palette.text_dim))
            .child(label("Searching…", palette.text_dim))
    } else if let Some(err) = search.error.as_ref().filter(|_| fresh) {
        label(err.clone(), palette.error)
    } else if !fresh || search.searched.is_empty() || search.searched != typed {
        label("Press Enter to search all pages.", palette.text_dim)
    } else {
        label(
            format!(
                "{total} matches on {} pages for “{}”",
                search.hits.len(),
                search.searched
            ),
            palette.text_dim,
        )
    };
    let show_hits = fresh && !search.busy && search.searched == typed && !typed.is_empty();
    let rows: Vec<_> = if show_hits {
        search
            .hits
            .iter()
            .map(|hit| hit_row(tab, hit, hit.page == current, &palette))
            .collect()
    } else {
        Vec::new()
    };
    // egui: field `available - 40` wide + 28 px button, 8 apart; then label, 4 px, the hits.
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .pr(px(4.))
                .child(div().flex_1().child(Input::new(&field).h(px(26.))))
                .child(
                    widgets::icon_button("search-run", icons::MAGNIFYING_GLASS, "Search", &palette)
                        .on_click(|_, _, cx| run(true, cx)),
                ),
        )
        .child(status)
        .child(
            div()
                .id("search-hits")
                .mt(px(4.))
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(px(6.))
                .children(rows),
        )
        .into_any_element()
}

/// egui `Button::selectable` sized `[width, 26]`: centered text, `selected` fill when on.
fn hit_row(
    tab: &Entity<DocTab>,
    hit: &SearchHit,
    current: bool,
    palette: &Palette,
) -> impl IntoElement {
    let tab = tab.downgrade();
    let page = hit.page;
    let hover = palette.hover;
    div()
        .id(("hit", page))
        .flex_none()
        .h(px(26.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme::RADIUS))
        .cursor_pointer()
        .when(current, |row| row.bg(palette.selected))
        .hover(move |style| style.bg(hover))
        .child(format!("Page {}   ·   {} ×", page + 1, hit.count))
        .on_click(move |_, _, cx| {
            let _ = tab.update(cx, |tab, cx| {
                tab.scroll_to_page(page);
                cx.notify();
            });
        })
}

/// Yellow marks over the hits on `page`, while the search panel shows them.
pub fn page_overlay(tab: &mut DocTab, page: usize, cx: &mut Context<DocTab>) -> Option<AnyElement> {
    if open_panel(cx) != Some(Panel::Search) {
        return None;
    }
    let typed = tab.panels.search.query.trim().to_owned();
    let generation = tab.generation();
    let search = &tab.panels.search;
    let marked = search.generation == generation
        && !search.busy
        && !typed.is_empty()
        && search.searched == typed
        && search.hits.iter().any(|hit| hit.page == page);
    if !marked {
        return None;
    }
    let chars = page_chars(tab, page);
    let zoom = tab.view.zoom;
    let mark = |a: f32, g: f32| -> Hsla {
        Rgba {
            r: 1.0,
            g,
            b: 0.0,
            a,
        }
        .into()
    };
    let current = tab
        .panels
        .search
        .at
        .filter(|(at, _)| *at == page)
        .map(|(_, index)| index);
    let marks = find(&chars, &typed)
        .into_iter()
        .enumerate()
        .flat_map(|(index, found)| {
            // The match stepped to stands out in orange.
            let fill = if current == Some(index) {
                mark(0.6, 0.45)
            } else {
                mark(0.38, 0.77)
            };
            bands(&chars, found)
                .into_iter()
                .map(move |band| (band, fill))
        })
        .map(|([x0, y0, x1, y1], fill)| {
            div()
                .absolute()
                .left(px(x0 * zoom - 1.))
                .top(px(y0 * zoom - 1.))
                .w(px((x1 - x0) * zoom + 2.))
                .h(px((y1 - y0) * zoom + 2.))
                .rounded(px(2.))
                .bg(fill)
        });
    Some(
        div()
            .absolute()
            .inset_0()
            .children(marks)
            .into_any_element(),
    )
}

/// Text positions of `page`; empty when it has none or cannot be read.
fn read_chars(tab: &DocTab, page: usize) -> Vec<TextChar> {
    tab.renderer()
        .and_then(|doc| catch_unwind(AssertUnwindSafe(|| doc.text_chars(page).ok())).ok()?)
        .unwrap_or_default()
}

/// Glyph ranges where `query` is written (ported from the egui `src/textsel.rs`).
/// Case and white space are ignored on both sides: a PDF often only implies its spaces by
/// position, and breaks a phrase over two lines.
pub fn find(chars: &[TextChar], query: &str) -> Vec<RangeInclusive<usize>> {
    let fold = |text: &str| -> Vec<char> {
        let kept = text.chars().filter(|ch| !ch.is_whitespace());
        kept.flat_map(char::to_lowercase).collect()
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
        if here.iter().map(|(ch, _)| ch).eq(needle.iter()) {
            found.push(here[0].1..=here[needle.len() - 1].1);
            at += needle.len();
        } else {
            at += 1;
        }
    }
    found
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

    #[test]
    fn finds_ignoring_case_and_spaces() {
        let chars = line("Hello World hello", 0, 0);
        assert_eq!(find(&chars, "HELLO"), vec![0..=4, 12..=16]);
        assert_eq!(find(&chars, "loWor"), vec![3..=8]);
        assert!(find(&chars, "   ").is_empty());
        assert!(find(&chars, "absent").is_empty());
    }

    #[test]
    fn a_hit_over_two_lines_gives_two_bands() {
        let mut chars = line("ab", 0, 5);
        chars.extend(line("cd", 1, 0));
        let found = find(&chars, "abcd");
        assert_eq!(found, vec![0..=3]);
        assert_eq!(
            bands(&chars, found[0].clone()),
            vec![[50.0, 0.0, 70.0, 12.0], [0.0, 20.0, 20.0, 32.0]]
        );
    }

    #[test]
    fn steps_match_by_match_across_pages() {
        let pages = [2, 5];
        let count = |page: usize| if page == 2 { 2 } else { 1 };
        let next = |at, forward| next_match(&pages, count, at, 3, forward);
        assert_eq!(next(None, true), Some((5, 0)));
        assert_eq!(next(None, false), Some((2, 1)));
        assert_eq!(next(Some((2, 0)), true), Some((2, 1)));
        assert_eq!(next(Some((2, 1)), true), Some((5, 0)));
        assert_eq!(next(Some((5, 0)), true), Some((2, 0)));
        assert_eq!(next(Some((2, 0)), false), Some((5, 0)));
        assert_eq!(next(Some((5, 0)), false), Some((2, 1)));
        assert_eq!(next_match(&[], count, None, 0, true), None);
    }

    #[test]
    fn steps_through_pages_and_wraps() {
        let pages = [2, 5, 9];
        assert_eq!(next_page(&pages, 0, true), Some(2));
        assert_eq!(next_page(&pages, 5, true), Some(9));
        assert_eq!(next_page(&pages, 9, true), Some(2));
        assert_eq!(next_page(&pages, 5, false), Some(2));
        assert_eq!(next_page(&pages, 2, false), Some(9));
        assert_eq!(next_page(&[], 3, true), None);
    }
}
