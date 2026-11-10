//! Left sidebar (ported from `crates/app/src/sidebar.rs`, `bookmarks.rs`, `organize.rs`): an
//! icon rail and one open panel: page thumbnails, bookmarks or text search.
//!
//! App-wide state is the [`PanelState`] global (open panel, search field, the tab shown).
//! Per-document state is [`TabPanels`], a field of `DocTab` (thumbnail cache, search hits,
//! bookmark being renamed), so it goes away with its tab.

mod bookmarks;
mod search;
mod thumbs;

use gpui::assets::IconName;
use gpui::component::input::InputState;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, Global, KeyBinding, Subscription, WeakEntity, Window,
    actions, div, px,
};

use crate::doc::DocTab;
use crate::theme::Palette;

actions!(
    godpdf,
    [
        ShowPages,
        ShowBookmarks,
        Find,
        FindNext,
        FindPrevious,
        AddBookmark
    ]
);

const RAIL_WIDTH: f32 = 44.;
const PANEL_WIDTH: f32 = 240.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Pages,
    Bookmarks,
    Search,
}

impl Panel {
    const ALL: [Panel; 3] = [Panel::Pages, Panel::Bookmarks, Panel::Search];

    fn icon(self) -> IconName {
        match self {
            Self::Pages => IconName::Files,
            Self::Bookmarks => IconName::Bookmark,
            Self::Search => IconName::Search,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Pages => "Pages",
            Self::Bookmarks => "Bookmarks",
            Self::Search => "Search",
        }
    }
}

/// App-wide sidebar state.
#[derive(Default)]
pub struct PanelState {
    open: Option<Panel>,
    /// The document the sidebar shows, set while it renders: key actions go to it.
    active: Option<WeakEntity<DocTab>>,
    /// The search field, created the first time the search panel shows (it needs a window).
    search_input: Option<(Entity<InputState>, Subscription)>,
    /// Focus the search field on the next render (Ctrl+F has no window at hand).
    focus_search: bool,
}

impl Global for PanelState {}

/// Sidebar state of one document.
#[derive(Default)]
pub struct TabPanels {
    thumbs: thumbs::Thumbs,
    search: search::Search,
    bookmarks: bookmarks::Marks,
}

fn state(cx: &mut App) -> &mut PanelState {
    cx.default_global::<PanelState>()
}

fn open_panel(cx: &App) -> Option<Panel> {
    cx.try_global::<PanelState>().and_then(|state| state.open)
}

/// Show `panel`, or hide the sidebar when it is already shown.
fn toggle(panel: Panel, cx: &mut App) {
    let state = state(cx);
    state.open = if state.open == Some(panel) {
        None
    } else {
        Some(panel)
    };
    cx.refresh_windows();
}

fn show(panel: Panel, cx: &mut App) {
    state(cx).open = Some(panel);
    cx.refresh_windows();
}

/// Run `update` on the document the sidebar shows, if any.
fn with_active(cx: &mut App, update: impl FnOnce(&mut DocTab, &mut Context<DocTab>)) {
    let Some(tab) = state(cx).active.clone().and_then(|weak| weak.upgrade()) else {
        return;
    };
    tab.update(cx, |tab, cx| {
        update(tab, cx);
        cx.notify();
    });
}

/// Register panel actions and their key bindings (same keys as the egui app, plus F3).
pub fn register(cx: &mut App) {
    cx.set_global(PanelState::default());
    cx.bind_keys([
        KeyBinding::new(secondary!("f"), Find, None),
        KeyBinding::new(secondary!("d"), AddBookmark, None),
        KeyBinding::new("f3", FindNext, None),
        KeyBinding::new("shift-f3", FindPrevious, None),
    ]);
    cx.on_action(|_: &ShowPages, cx| toggle(Panel::Pages, cx));
    cx.on_action(|_: &ShowBookmarks, cx| toggle(Panel::Bookmarks, cx));
    cx.on_action(|_: &Find, cx| {
        state(cx).focus_search = true;
        show(Panel::Search, cx);
    });
    cx.on_action(|_: &FindNext, cx| search::step(true, cx));
    cx.on_action(|_: &FindPrevious, cx| search::step(false, cx));
    cx.on_action(|_: &AddBookmark, cx| {
        with_active(cx, bookmarks::add_for_current_page);
        show(Panel::Bookmarks, cx);
    });
}

/// Left of the viewer: the icon rail, and the open panel next to it. `None` without a document.
pub fn left(tab: Option<&Entity<DocTab>>, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    let tab = tab?.clone();
    state(cx).active = Some(tab.downgrade());
    let palette = Palette::of(cx);
    let open = open_panel(cx);
    let body = open.map(|panel| {
        let content = match panel {
            Panel::Pages => thumbs::render(&tab, window, cx),
            Panel::Bookmarks => bookmarks::render(&tab, window, cx),
            Panel::Search => search::render(&tab, window, cx),
        };
        div()
            .w(px(PANEL_WIDTH))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(palette.panel)
            .border_r_1()
            .border_color(palette.border)
            .child(
                div()
                    .px(px(12.))
                    .pt(px(10.))
                    .pb(px(6.))
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(panel.title()),
            )
            .child(div().flex_1().min_h_0().flex().flex_col().child(content))
    });
    let rail = div()
        .w(px(RAIL_WIDTH))
        .flex_none()
        .h_full()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(4.))
        .pt(px(8.))
        .bg(palette.panel)
        .border_r_1()
        .border_color(palette.border)
        .children(Panel::ALL.map(|panel| rail_button(panel, open == Some(panel), &palette)));
    Some(
        div()
            .flex()
            .flex_none()
            .h_full()
            .child(rail)
            .children(body)
            .into_any_element(),
    )
}

fn rail_button(panel: Panel, active: bool, palette: &Palette) -> impl IntoElement {
    let hover = palette.hover;
    div()
        .id(panel.title())
        .size(px(32.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(7.))
        .text_size(px(16.))
        .cursor_pointer()
        .text_color(if active {
            palette.text
        } else {
            palette.text_dim
        })
        .when(active, |button| button.bg(palette.selected))
        .hover(move |style| style.bg(hover))
        .child(panel.icon())
        .on_click(move |_, _, cx| {
            if panel == Panel::Search && open_panel(cx) != Some(Panel::Search) {
                state(cx).focus_search = true;
            }
            toggle(panel, cx);
        })
}

/// Right of the viewer: nothing yet (comments and properties are another wave).
pub fn right(
    _tab: Option<&Entity<DocTab>>,
    _window: &mut Window,
    _cx: &mut App,
) -> Option<AnyElement> {
    None
}

/// Search hits marked over one page (called by the viewer for every visible page).
pub fn page_overlay(tab: &mut DocTab, page: usize, cx: &mut Context<DocTab>) -> Option<AnyElement> {
    search::page_overlay(tab, page, cx)
}

/// A small text row in the panel, dimmed (hints, counts, errors).
fn note(text: impl Into<gpui::SharedString>, palette: &Palette) -> gpui::Div {
    div()
        .px(px(12.))
        .py(px(6.))
        .text_sm()
        .text_color(palette.text_dim)
        .child(text.into())
}

/// A square icon button in the panel palette. Add `.on_click(..)`.
fn icon_button(
    id: impl Into<gpui::ElementId>,
    icon: IconName,
    palette: &Palette,
) -> gpui::Stateful<gpui::Div> {
    let hover = palette.hover;
    div()
        .id(id)
        .size(px(26.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .text_size(px(14.))
        .text_color(palette.text)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(icon)
}
