//! Right panel, opened from the icon rail (`shell/rail.rs`): comments, page
//! thumbnails, bookmarks, the form or the signature. Text search is the find bar (`find/`).
//!
//! App-wide state is the [`PanelState`] global (open panel, the tab shown).
//! Per-document state is [`TabPanels`], a field of `DocTab` (thumbnail cache, find state,
//! bookmark being renamed), so it goes away with its tab.

mod bookmarks;
mod comments;
mod form;
mod sign;
mod thumbs;

pub use crate::find::Find;
pub use thumbs::{Pick, Thumbs, thumbnail, trim};

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, Global, Hsla, KeyBinding, SharedString, WeakEntity, Window,
    actions, div, px,
};

use crate::doc::DocTab;
use crate::theme::Palette;
use crate::{icons, widgets};

actions!(
    godpdf,
    [
        ShowPages,
        ShowBookmarks,
        AddBookmark,
        FillForm,
        Sign,
        ToggleSidebar
    ]
);

/// Right panel: default width, inner margin.
/// 320 at first, dragged 240..=520 by its left edge.
const PANEL_WIDTH: f32 = 320.;
const WIDTH_RANGE: (f32, f32) = (240., 520.);
const PANEL_MARGIN: (f32, f32) = (14., 6.);
/// Largest box a page thumbnail is fitted in (row width - 32 by 156 at the first panel
/// width); a narrower panel narrows it (`thumbs::fit`).
const BOX: (f32, f32) = (PANEL_WIDTH - 2. * PANEL_MARGIN.0 - 32., 156.);

/// The views of the right rail, in rail order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Comments,
    Pages,
    Bookmarks,
    Form,
    Sign,
}

impl Panel {
    pub const ALL: [Panel; 5] = [
        Panel::Comments,
        Panel::Pages,
        Panel::Bookmarks,
        Panel::Form,
        Panel::Sign,
    ];

    pub fn icon(self) -> &'static str {
        match self {
            Self::Comments => icons::CHAT_TEXT,
            Self::Pages => icons::FILES,
            Self::Bookmarks => icons::BOOKMARK_SIMPLE,
            Self::Form => icons::LIST_CHECKS,
            Self::Sign => icons::SIGNATURE,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Comments => "Comments",
            Self::Pages => "Pages",
            Self::Bookmarks => "Bookmarks",
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
    /// Panel width, dragged by its left edge.
    width: f32,
    /// Panel brought back by Toggle sidebar.
    last: Panel,
}

impl Default for PanelState {
    fn default() -> Self {
        Self {
            open: Some(Panel::Pages),
            active: None,
            width: PANEL_WIDTH,
            last: Panel::Pages,
        }
    }
}

impl Global for PanelState {}

/// Sidebar state of one document.
#[derive(Default)]
pub struct TabPanels {
    comments: comments::Comments,
    pub(crate) thumbs: thumbs::Thumbs,
    pub find: crate::find::FindState,
    bookmarks: bookmarks::Marks,
    form: form::FormState,
}

/// Width the content of the right panel can take: less its margins and its 1 px border.
fn content_width(cx: &mut App) -> f32 {
    (state(cx).width - 2. * PANEL_MARGIN.0 - 1.).floor()
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
    // Toggle sidebar brings back the panel used last, however it was closed.
    state.last = panel;
    cx.refresh_windows();
}

pub fn show(panel: Panel, cx: &mut App) {
    let state = state(cx);
    state.open = Some(panel);
    state.last = panel;
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

/// Register panel actions and their key bindings.
pub fn register(cx: &mut App) {
    cx.set_global(PanelState::default());
    cx.bind_keys([KeyBinding::new(secondary!("d"), AddBookmark, None)]);
    cx.on_action(|_: &ShowPages, cx| toggle(Panel::Pages, cx));
    cx.on_action(|_: &ShowBookmarks, cx| toggle(Panel::Bookmarks, cx));
    cx.on_action(|_: &FillForm, cx| show(Panel::Form, cx));
    cx.on_action(|_: &Sign, cx| show(Panel::Sign, cx));
    // Hide the right panel, or bring back the last one.
    cx.on_action(|_: &ToggleSidebar, cx| {
        let state = state(cx);
        if state.open.take().is_none() {
            state.open = Some(state.last);
        }
        cx.refresh_windows();
    });
    cx.on_action(|_: &AddBookmark, cx| {
        with_active(cx, bookmarks::add_for_current_page);
        show(Panel::Bookmarks, cx);
    });
}

/// Rail button clicked: toggle `panel`.
pub fn rail_click(panel: Panel, cx: &mut App) {
    toggle(panel, cx);
}

/// Hide the right panel.
pub fn close(cx: &mut App) {
    state(cx).open = None;
    cx.refresh_windows();
}

/// The right panel left of the rail: title (17) + close, then the view.
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
        // The panel less its margins and its 1 px border.
        Panel::Pages => thumbs::render(&tab, content_width(cx) - 32., window, cx),
        Panel::Bookmarks => bookmarks::render(&tab, window, cx),
        Panel::Form => form::render(&tab, window, cx),
        Panel::Sign => sign::render(content_width(cx), cx),
    };
    Some(
        div()
            .relative()
            .w(px(state(cx).width))
            .flex_none()
            .on_drag_move(crate::resize::on_move(
                crate::resize::Edge::Right,
                WIDTH_RANGE,
                |width, cx| state(cx).width = width,
            ))
            .child(crate::resize::handle(crate::resize::Edge::Right))
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

/// A plain label of the panel in a colour: body size, no padding.
fn label(text: impl Into<SharedString>, color: Hsla) -> gpui::Div {
    div().text_color(color).child(text.into())
}

/// A context menu entry: Phosphor glyph,
/// half a glyph gap, label; dimmed when disabled. Add `.on_click(..)`.
pub(crate) fn menu_entry(
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
