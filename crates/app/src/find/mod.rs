//! Find bar, like a web browser's: Ctrl+F opens a small card at the top right of the
//! viewer; typing searches every page in the background (debounced, page by page from
//! the page being read), every match is marked on the pages and the current one stands
//! out; Enter / F3 and Shift+Enter / Shift+F3 step through them and scroll to the word.
//! Esc closes the bar and drops the marks.
//!
//! Per document: [`FindState`] (`DocTab::panels.find`). App-wide: the `FindBar` global
//! (the text field, shared by the tabs, its options and where the focus goes back).

mod matching;

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Duration;

use godpdf_render::{Document, TextChar};
use gpui::component::input::{Input, InputEvent, InputState};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, EntityId, FocusHandle, Focusable, Global, Hsla, KeyBinding,
    MouseButton, Subscription, WeakEntity, Window, actions, div, px,
};

use crate::doc::DocTab;
use crate::theme::{self, Palette};
use crate::{icons, widgets};
use matching::{Options, bands, reveal_delta, scan_order, step};

actions!(godpdf, [Find, FindNext, FindPrevious, FindClose]);

/// Wait after a key before searching, so fast typing searches once.
const DEBOUNCE: Duration = Duration::from_millis(120);
/// Bar distance from the viewer's top and right edges (right: past the scrollbar).
const INSET: (f32, f32) = (12., 26.);
const KEY_CONTEXT: &str = "FindBar";

/// One match: its page and one box per line it covers (displayed points).
struct Match {
    page: usize,
    bands: Vec<[f32; 4]>,
}

/// Find state of one document.
#[derive(Default)]
pub struct FindState {
    open: bool,
    /// What is typed for this document.
    query: String,
    /// Matches found so far, in page order.
    matches: Vec<Match>,
    /// Index of the match stepped to in `matches`.
    current: Option<usize>,
    searching: bool,
    /// Bumped by every new search: a running one stops when it no longer matches.
    token: u64,
    /// Document generation the matches and text positions were read from.
    generation: u64,
    /// Text positions of the pages searched, read once per generation.
    chars: HashMap<usize, Arc<Vec<TextChar>>>,
}

#[derive(Default)]
struct FindBar {
    field: Option<(Entity<InputState>, Subscription)>,
    /// The document whose query the field shows.
    shown: Option<EntityId>,
    /// The document the viewer shows, set while it renders: actions go to it.
    viewing: Option<WeakEntity<DocTab>>,
    /// Focus the field on the next render (actions have no window at hand).
    focus: bool,
    /// Where the focus goes back when the bar closes.
    return_focus: Option<FocusHandle>,
    options: Options,
}

impl Global for FindBar {}

fn bar(cx: &mut App) -> &mut FindBar {
    cx.default_global::<FindBar>()
}

/// Register the find actions and their keys (Enter / Esc in the field outrank its own).
pub fn register(cx: &mut App) {
    cx.set_global(FindBar::default());
    let field = Some("FindBar > Input");
    cx.bind_keys([
        KeyBinding::new(secondary!("f"), Find, None),
        KeyBinding::new("f3", FindNext, None),
        KeyBinding::new("shift-f3", FindPrevious, None),
        KeyBinding::new("enter", FindNext, field),
        KeyBinding::new("shift-enter", FindPrevious, field),
        KeyBinding::new("escape", FindClose, field),
        KeyBinding::new("escape", FindClose, Some(KEY_CONTEXT)),
    ]);
    cx.on_action(|_: &Find, cx| open(cx));
    cx.on_action(|_: &FindNext, cx| next(true, cx));
    cx.on_action(|_: &FindPrevious, cx| next(false, cx));
}

/// The bar is open on the document in front (for the rail button).
pub fn is_open(tab: &Entity<DocTab>, cx: &App) -> bool {
    tab.read(cx).panels.find.open
}

/// Run `update` on the document the viewer shows, if any.
fn with_tab(cx: &mut App, update: impl FnOnce(&mut DocTab, &mut Context<DocTab>)) {
    let Some(tab) = bar(cx).viewing.clone().and_then(|weak| weak.upgrade()) else {
        return;
    };
    tab.update(cx, |tab, cx| {
        update(tab, cx);
        cx.notify();
    });
}

/// Ctrl+F: open the bar (searching its text again) and focus the field.
fn open(cx: &mut App) {
    bar(cx).focus = true;
    with_tab(cx, |tab, cx| {
        if !tab.panels.find.open {
            tab.panels.find.open = true;
            start(tab, true, cx);
        }
    });
}

/// F3 / Shift+F3 / Enter: the next or previous match; opens the bar when closed.
fn next(forward: bool, cx: &mut App) {
    let open_now = bar(cx)
        .viewing
        .clone()
        .and_then(|weak| weak.upgrade())
        .is_some_and(|tab| !tab.read(cx).panels.find.open);
    if open_now {
        open(cx);
    } else {
        with_tab(cx, |tab, _| go(tab, forward));
    }
}

fn go(tab: &mut DocTab, forward: bool) {
    let find = &mut tab.panels.find;
    if let Some(at) = step(find.current, find.matches.len(), forward) {
        find.current = Some(at);
        reveal(tab, at);
    }
}

/// Scroll so match `index` is in sight (centred when it is not).
fn reveal(tab: &mut DocTab, index: usize) {
    let Some(found) = tab.panels.find.matches.get(index) else {
        return;
    };
    let layout = tab.layout();
    let (Some(rect), Some(band)) = (layout.rects.get(found.page), found.bands.first()) else {
        return;
    };
    let zoom = layout.zoom;
    let size = tab.view.bounds.size;
    let (scroll_x, scroll_y) = tab.view.scroll;
    let dx = reveal_delta(
        rect.x + band[0] * zoom,
        rect.x + band[2] * zoom,
        scroll_x,
        f32::from(size.width),
    );
    let dy = reveal_delta(
        rect.y + band[1] * zoom,
        rect.y + band[3] * zoom,
        scroll_y,
        f32::from(size.height),
    );
    tab.scroll_by((dx, dy));
}

/// Search the query again from scratch, stopping the search running; `jump` scrolls to
/// the first match found (the first one from the page being read).
fn start(tab: &mut DocTab, jump: bool, cx: &mut Context<DocTab>) {
    let generation = tab.generation();
    let options = bar(cx).options;
    let (count, from, doc) = (tab.page_count(), tab.view.current_page, tab.renderer());
    let find = &mut tab.panels.find;
    find.token += 1;
    let token = find.token;
    find.matches.clear();
    find.current = None;
    if find.generation != generation {
        find.chars.clear();
        find.generation = generation;
    }
    let query = find.query.clone();
    find.searching = find.open && !query.trim().is_empty() && doc.is_some();
    let (true, Some(doc)) = (find.searching, doc) else {
        return;
    };
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(DEBOUNCE).await;
        for page in scan_order(count, from) {
            let cached = this.read_with(cx, |tab, _| {
                let find = &tab.panels.find;
                (find.token == token).then(|| find.chars.get(&page).cloned())
            });
            let Ok(Some(cached)) = cached else {
                return;
            };
            let (doc, query) = (Arc::clone(&doc), query.clone());
            let (chars, found) = cx
                .background_executor()
                .spawn(async move {
                    let chars = cached.unwrap_or_else(|| Arc::new(read_chars(&doc, page)));
                    let found: Vec<Vec<[f32; 4]>> = matching::find(&chars, &query, options)
                        .into_iter()
                        .map(|range| bands(&chars, range))
                        .collect();
                    (chars, found)
                })
                .await;
            let _ = this.update(cx, |tab, cx| {
                if add(tab, token, page, chars, found, jump) {
                    cx.notify();
                }
            });
        }
        let _ = this.update(cx, |tab, cx| {
            if tab.panels.find.token == token {
                tab.panels.find.searching = false;
                cx.notify();
            }
        });
    })
    .detach();
}

/// Take in the matches of `page` from search `token`; true when some were added. The
/// first match found becomes the current one.
fn add(
    tab: &mut DocTab,
    token: u64,
    page: usize,
    chars: Arc<Vec<TextChar>>,
    found: Vec<Vec<[f32; 4]>>,
    jump: bool,
) -> bool {
    let find = &mut tab.panels.find;
    if find.token != token {
        return false;
    }
    find.chars.insert(page, chars);
    if found.is_empty() {
        return false;
    }
    let at = find.matches.partition_point(|found| found.page < page);
    let added = found.len();
    find.matches
        .splice(at..at, found.into_iter().map(|bands| Match { page, bands }));
    match find.current {
        Some(current) if current >= at => find.current = Some(current + added),
        Some(_) => {}
        None => {
            find.current = Some(at);
            if jump {
                reveal(tab, at);
            }
        }
    }
    true
}

/// Close the bar: stop searching, drop the marks, give the focus back.
fn close(tab: &mut DocTab, window: &mut Window, cx: &mut App) {
    let find = &mut tab.panels.find;
    find.open = false;
    find.token += 1;
    find.searching = false;
    find.matches.clear();
    find.current = None;
    if let Some(focus) = bar(cx).return_focus.take() {
        window.focus(&focus, cx);
    }
}

/// Text positions of `page`; empty when it has none or cannot be read.
fn read_chars(doc: &Document, page: usize) -> Vec<TextChar> {
    catch_unwind(AssertUnwindSafe(|| doc.text_chars(page).ok()))
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// The field, created on first use. What is typed searches the document shown.
fn field(window: &mut Window, cx: &mut App) -> Entity<InputState> {
    if let Some((field, _)) = &bar(cx).field {
        return field.clone();
    }
    let field = cx.new(|cx| InputState::new(window, cx).placeholder("Find in document"));
    let subscription = cx.subscribe(&field, |field, event: &InputEvent, cx| {
        if let InputEvent::Change = event {
            let typed = field.read(cx).value().to_string();
            with_tab(cx, |tab, cx| {
                // Also sent when the field is set to another tab's query: nothing new then.
                if tab.panels.find.query != typed {
                    tab.panels.find.query = typed;
                    start(tab, true, cx);
                }
            });
        }
    });
    bar(cx).field = Some((field.clone(), subscription));
    field
}

/// Soft yellow marks over every match on `page`, the current one in orange.
pub fn page_overlay(tab: &DocTab, page: usize) -> Option<AnyElement> {
    let find = &tab.panels.find;
    if !find.open || find.generation != tab.generation() {
        return None;
    }
    let from = find.matches.partition_point(|found| found.page < page);
    let to = find.matches.partition_point(|found| found.page <= page);
    if from == to {
        return None;
    }
    let (other, current): (Hsla, Hsla) =
        (gpui::rgba(0xFFD4005C).into(), gpui::rgba(0xFF8C00A0).into());
    let zoom = tab.view.zoom;
    let marks = find.matches[from..to]
        .iter()
        .enumerate()
        .flat_map(|(offset, found)| {
            let fill = if find.current == Some(from + offset) {
                current
            } else {
                other
            };
            found.bands.iter().map(move |band| (*band, fill))
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

/// The bar over the viewer's top-right corner, `None` when closed.
pub fn viewer_overlay(
    tab: &mut DocTab,
    window: &mut Window,
    cx: &mut Context<DocTab>,
) -> Option<AnyElement> {
    bar(cx).viewing = Some(cx.weak_entity());
    if !tab.panels.find.open {
        return None;
    }
    // An edit may have moved the text: search the new bytes, staying where we are.
    if tab.panels.find.generation != tab.generation() {
        start(tab, false, cx);
    }
    let field = field(window, cx);
    let shown = cx.entity_id();
    if bar(cx).shown != Some(shown) {
        bar(cx).shown = Some(shown);
        let query = tab.panels.find.query.clone();
        field.update(cx, |field, cx| field.set_value(query, window, cx));
    }
    if std::mem::take(&mut bar(cx).focus) {
        if !field.read(cx).focus_handle(cx).is_focused(window) {
            bar(cx).return_focus = window.focused(cx);
        }
        field.update(cx, |field, cx| {
            field.focus(window, cx);
            field.select_all(window, cx);
        });
    }

    let palette = Palette::of(cx);
    let options = bar(cx).options;
    let find = &tab.panels.find;
    let total = find.matches.len();
    let (status, color) = if find.query.trim().is_empty() {
        (String::new(), palette.text_dim)
    } else if total > 0 {
        let at = find.current.map_or(0, |at| at + 1);
        (format!("{at} / {total}"), palette.text_dim)
    } else if find.searching {
        ("…".to_owned(), palette.text_dim)
    } else {
        ("No results".to_owned(), palette.error)
    };

    let toggle = |id: &'static str, label: &'static str, tip: &'static str, on: bool| {
        let hover = palette.hover;
        let button = div()
            .id(id)
            .flex_none()
            .size(px(widgets::SMALL_ICON_BUTTON))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(theme::RADIUS))
            .cursor_pointer()
            .text_size(px(12.))
            .text_color(if on { palette.text } else { palette.text_dim })
            .when(on, |button| button.bg(palette.selected))
            .hover(move |style| style.bg(hover))
            .child(label);
        widgets::tooltip(button, tip)
    };
    let set = |change: fn(&mut Options)| {
        cx.listener(move |tab: &mut DocTab, _: &gpui::ClickEvent, _, cx| {
            change(&mut bar(cx).options);
            start(tab, true, cx);
            cx.notify();
        })
    };
    let step_to = |forward: bool| {
        cx.listener(move |tab: &mut DocTab, _: &gpui::ClickEvent, _, cx| {
            go(tab, forward);
            cx.notify();
        })
    };
    let small = |id: &'static str, glyph, tip: &'static str| {
        widgets::small_icon_button(id, glyph, tip, &palette)
    };
    // Clicks stay in the bar: the tools under it and the window's focus do not see them.
    let keep_focus = |_: &gpui::MouseDownEvent, window: &mut Window, cx: &mut App| {
        window.prevent_default();
        cx.stop_propagation();
    };

    Some(
        div()
            .id("find-bar")
            .key_context(KEY_CONTEXT)
            .absolute()
            .top(px(INSET.0))
            .right(px(INSET.1))
            .h(px(38.))
            .pl(px(10.))
            .pr(px(5.))
            .flex()
            .items_center()
            .gap(px(4.))
            .rounded(px(theme::MENU_RADIUS))
            .bg(palette.panel)
            .border_1()
            .border_color(palette.border)
            .shadow(widgets::popup_shadow(&palette))
            .block_mouse_except_scroll()
            .on_mouse_down(MouseButton::Left, keep_focus)
            .on_action(cx.listener(|tab, _: &FindClose, window, cx| {
                close(tab, window, cx);
                cx.notify();
            }))
            .child(widgets::icon(
                icons::MAGNIFYING_GLASS,
                14.,
                palette.text_dim,
            ))
            .child(
                div()
                    .w(px(180.))
                    .child(Input::new(&field).appearance(false).h(px(26.))),
            )
            .child(
                div()
                    .min_w(px(64.))
                    .flex()
                    .justify_end()
                    .text_size(px(12.))
                    .text_color(color)
                    .child(status),
            )
            .child(div().w(px(1.)).h(px(18.)).mx(px(2.)).bg(palette.border))
            .child(
                toggle("find-case", "Aa", "Match case", options.case)
                    .on_click(set(|options| options.case = !options.case)),
            )
            .child(
                toggle("find-whole", "ab", "Whole word", options.whole)
                    .on_click(set(|options| options.whole = !options.whole)),
            )
            .child(
                small("find-previous", icons::CARET_UP, "Previous (Shift+Enter)")
                    .on_click(step_to(false)),
            )
            .child(small("find-next", icons::CARET_DOWN, "Next (Enter)").on_click(step_to(true)))
            .child(
                small("find-close", icons::X, "Close (Esc)").on_click(cx.listener(
                    |tab, _: &gpui::ClickEvent, window, cx| {
                        close(tab, window, cx);
                        cx.notify();
                    },
                )),
            )
            .into_any_element(),
    )
}
