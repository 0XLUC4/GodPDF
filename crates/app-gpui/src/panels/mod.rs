//! Right panel (ported from `crates/app/src/sidebar.rs`, `comments.rs`, `bookmarks.rs` and
//! `GodApp::right_panel`), opened from the icon rail (`shell/rail.rs`): comments, page
//! thumbnails, bookmarks or text search.
//!
//! App-wide state is the [`PanelState`] global (open panel, search field, the tab shown).
//! Per-document state is [`TabPanels`], a field of `DocTab` (thumbnail cache, search hits,
//! bookmark being renamed), so it goes away with its tab.

mod bookmarks;
mod comments;
mod search;
mod thumbs;

use gpui::component::input::InputState;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, Global, Hsla, KeyBinding, SharedString, Subscription,
    WeakEntity, Window, actions, div, px,
};

use crate::doc::DocTab;
use crate::theme::Palette;
use crate::{icons, widgets};

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

/// Right panel (egui `right_panel`): default width, inner margin.
// shortcut: fixed width, egui lets it be dragged 240..=520; add a drag handle if wanted.
const PANEL_WIDTH: f32 = 320.;
const PANEL_MARGIN: (f32, f32) = (14., 6.);
/// Box a page thumbnail is fitted in (egui `sidebar::pages`: row width - 32 by 156).
const BOX: (f32, f32) = (PANEL_WIDTH - 2. * PANEL_MARGIN.0 - 32., 156.);

/// The views of the right rail (egui `sidebar::Tab`), in rail order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Comments,
    Pages,
    Bookmarks,
    Search,
    Form,
    Sign,
}

impl Panel {
    pub const ALL: [Panel; 6] = [
        Panel::Comments,
        Panel::Pages,
        Panel::Bookmarks,
        Panel::Search,
        Panel::Form,
        Panel::Sign,
    ];

    pub fn icon(self) -> &'static str {
        match self {
            Self::Comments => icons::CHAT_TEXT,
            Self::Pages => icons::FILES,
            Self::Bookmarks => icons::BOOKMARK_SIMPLE,
            Self::Search => icons::MAGNIFYING_GLASS,
            Self::Form => icons::LIST_CHECKS,
            Self::Sign => icons::SIGNATURE,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Comments => "Comments",
            Self::Pages => "Pages",
            Self::Bookmarks => "Bookmarks",
            Self::Search => "Search",
            Self::Form => "Form",
            Self::Sign => "Signature",
        }
    }
}

/// App-wide sidebar state.
pub struct PanelState {
    open: Option<Panel>,
    /// The document the sidebar shows, set while it renders: key actions go to it.
    active: Option<WeakEntity<DocTab>>,
    /// The search field, created the first time the search panel shows (it needs a window).
    search_input: Option<(Entity<InputState>, Subscription)>,
    /// Focus the search field on the next render (Ctrl+F has no window at hand).
    focus_search: bool,
}

impl Default for PanelState {
    fn default() -> Self {
        Self {
            open: Some(Panel::Pages),
            active: None,
            search_input: None,
            focus_search: false,
        }
    }
}

impl Global for PanelState {}

/// Sidebar state of one document.
#[derive(Default)]
pub struct TabPanels {
    comments: comments::Comments,
    thumbs: thumbs::Thumbs,
    search: search::Search,
    bookmarks: bookmarks::Marks,
}

fn state(cx: &mut App) -> &mut PanelState {
    cx.default_global::<PanelState>()
}

pub fn open_panel(cx: &App) -> Option<Panel> {
    cx.try_global::<PanelState>().and_then(|state| state.open)
}

/// Show `panel`, or hide the sidebar when it is already shown.
pub fn toggle(panel: Panel, cx: &mut App) {
    let state = state(cx);
    state.open = if state.open == Some(panel) {
        None
    } else {
        Some(panel)
    };
    cx.refresh_windows();
}

pub fn show(panel: Panel, cx: &mut App) {
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

/// Rail button clicked (egui `rail`): toggle `panel`, focusing the search field on open.
pub fn rail_click(panel: Panel, cx: &mut App) {
    if panel == Panel::Search && open_panel(cx) != Some(Panel::Search) {
        state(cx).focus_search = true;
    }
    toggle(panel, cx);
}

/// Hide the right panel.
pub fn close(cx: &mut App) {
    state(cx).open = None;
    cx.refresh_windows();
}

/// The right panel (egui `right_panel`) left of the rail: title (17) + close, then the view.
/// `None` without a document or when closed.
pub fn right(
    tab: Option<&Entity<DocTab>>,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    let tab = tab?.clone();
    state(cx).active = Some(tab.downgrade());
    let panel = open_panel(cx)?;
    let palette = Palette::of(cx);
    let comment_count = (panel == Panel::Comments)
        .then(|| comments::count(&tab, cx))
        .flatten();
    let content = match panel {
        Panel::Comments => comments::render(&tab, cx),
        Panel::Pages => thumbs::render(&tab, window, cx),
        Panel::Bookmarks => bookmarks::render(&tab, window, cx),
        Panel::Search => search::render(&tab, window, cx),
        // TODO(right panel): port `formfill.rs`, `signature::panel`.
        Panel::Form | Panel::Sign => {
            label("Not available yet in this version.", palette.text_dim).into_any_element()
        }
    };
    Some(
        div()
            .w(px(PANEL_WIDTH))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .px(px(PANEL_MARGIN.0))
            .py(px(PANEL_MARGIN.1))
            .bg(palette.bg)
            .border_l_1()
            .border_color(palette.border)
            .child(
                div()
                    .mt(px(10.))
                    .mb(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().text_size(px(17.)).child(panel.title()))
                    .children(comment_count.map(|count| {
                        div()
                            .text_size(px(15.))
                            .text_color(palette.text_dim)
                            .child(count.to_string())
                    }))
                    .child(div().flex_1())
                    .child(
                        widgets::icon_button(
                            "close-right-panel",
                            icons::X,
                            "Close panel",
                            &palette,
                        )
                        .on_click(|_, _, cx| close(cx)),
                    ),
            )
            .child(div().flex_1().min_h_0().flex().flex_col().child(content))
            .into_any_element(),
    )
}

/// Search hits marked over one page (called by the viewer for every visible page).
pub fn page_overlay(tab: &mut DocTab, page: usize, cx: &mut Context<DocTab>) -> Option<AnyElement> {
    search::page_overlay(tab, page, cx)
}

/// A plain label of the panel (egui `ui.label` in a colour): body size, no padding.
fn label(text: impl Into<SharedString>, color: Hsla) -> gpui::Div {
    div().text_color(color).child(text.into())
}

/// A context menu entry like egui's `Button::new(with_icon(glyph, label))`: Phosphor glyph,
/// half a glyph gap, label; dimmed when disabled. Add `.on_click(..)`.
fn menu_entry(
    glyph: &'static str,
    text: impl Into<SharedString>,
    enabled: bool,
    palette: &Palette,
) -> gpui::component::menu::PopupMenuItem {
    let text = text.into();
    let color = if enabled {
        palette.text
    } else {
        palette.disabled()
    };
    gpui::component::menu::PopupMenuItem::element(move |_, _| {
        widgets::with_icon(glyph, text.clone(), crate::fonts::BODY, color)
    })
    .disabled(!enabled)
}
