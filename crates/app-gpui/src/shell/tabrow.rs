//! Tab row (egui `GodApp::tab_row`, `tab_shape`, `doc_tab`): home tab, one tab per document,
//! "+ Open", then updates / about / theme on the right. Fill `surface`, margin 8 / 8 / 4 / 0,
//! no line under it: the active tab merges into the header below.

use gpui::prelude::*;
use gpui::{AnyElement, Context, Div, MouseButton, SharedString, Stateful, div, px};

use super::TAB_HEIGHT;
use crate::app::{GodPdfApp, Open, ToggleTheme};
use crate::theme::Palette;
use crate::widgets::{self, dispatch, icon};
use crate::{fonts, icons};

/// Longest document name shown in a tab before "…".
const TAB_TEXT_MAX: f32 = 180.0;

pub fn render(app: &GodPdfApp, palette: &Palette, cx: &mut Context<GodPdfApp>) -> AnyElement {
    let doc_view = app.doc_view();
    let home = tab_shape("home-tab", !doc_view, palette)
        .w(px(40.))
        .child(
            content()
                .w_full()
                .justify_center()
                .child(icon(icons::HOUSE, 17.0, palette.text)),
        )
        .on_click(cx.listener(|app, _, _, cx| {
            if !app.tabs.is_empty() {
                app.shell.home = true;
                cx.notify();
            }
        }));
    let home = widgets::tooltip(home, "Home");

    let tabs = app.tabs.iter().enumerate().map(|(index, tab)| {
        let tab = tab.read(cx);
        let active = doc_view && index == app.active;
        (index, tab.title().to_owned().into(), tab.is_dirty(), active)
    });
    let tabs: Vec<_> = tabs
        .collect::<Vec<_>>()
        .into_iter()
        .map(|(index, title, dirty, active)| doc_tab(index, title, dirty, active, palette, cx))
        .collect();

    let open = widgets::tooltip(
        widgets::button("open", "", palette)
            .child(widgets::with_icon(icons::PLUS, "Open", 14.0, palette.text))
            .on_click(dispatch(Open)),
        "Open PDF (Ctrl+O)",
    );

    let dark = Palette::is_dark(cx);
    let (glyph, tip) = if dark {
        (icons::SUN, "Light mode")
    } else {
        (icons::MOON, "Dark mode")
    };
    // TODO(tab row): Updates (+ blue dot when one is out, `theme::DOWNLOAD_BLUE`) and About
    // have no GPUI port yet; tab context menu (Close / Close the others / Reopen closed tab /
    // Show in folder / Copy path) is not ported either.
    let right = div()
        .ml_auto()
        .flex()
        .items_center()
        .gap(px(2.))
        .child(widgets::icon_button(
            "updates",
            icons::ARROW_CIRCLE_UP,
            "Updates",
            palette,
        ))
        .child(widgets::icon_button(
            "about",
            icons::QUESTION,
            "About GodPDF",
            palette,
        ))
        .child(widgets::icon_button("theme", glyph, tip, palette).on_click(dispatch(ToggleTheme)));

    div()
        .id("tab-row")
        .flex_none()
        .flex()
        .items_center()
        .gap(px(2.))
        .h(px(TAB_HEIGHT + 4.))
        .pt(px(4.))
        .pl(px(8. + 2.))
        .pr(px(8.))
        .bg(palette.surface)
        .text_size(px(fonts::BODY))
        .child(home)
        .children(tabs)
        .child(div().w(px(6. - 2.)))
        .child(open)
        .child(right)
        .into_any_element()
}

/// Tab-shaped hit area (egui `tab_shape`): active = `bg` with rounded top corners from 4 px
/// below the row top down to the header; hovered = `hover`, radius 8, 5 px in from top and
/// bottom. Add the width and a [`content`] child.
fn tab_shape(id: impl Into<gpui::ElementId>, active: bool, palette: &Palette) -> Stateful<Div> {
    let id = id.into();
    let group: SharedString = format!("{id:?}").into();
    let hover = palette.hover;
    let fill = div().absolute().left_0().right_0();
    let fill = if active {
        fill.top(px(4.)).bottom_0().rounded_t(px(8.)).bg(palette.bg)
    } else {
        fill.top(px(5.))
            .bottom(px(5.))
            .rounded(px(8.))
            .group_hover(group.clone(), move |style| style.bg(hover))
    };
    div()
        .id(id)
        .group(group)
        .relative()
        .flex_none()
        .h(px(TAB_HEIGHT))
        .cursor_pointer()
        .child(fill)
}

/// Content of a tab, 2 px below the middle as egui draws it.
fn content() -> Div {
    div().relative().h_full().pt(px(4.)).flex().items_center()
}

/// One document tab (egui `doc_tab`): FILE_PDF 16 centered 20 px in, name (14) at 36 px,
/// `*` when unsaved, close cross (22 / 13) 8 px from the right edge.
fn doc_tab(
    index: usize,
    title: SharedString,
    dirty: bool,
    active: bool,
    palette: &Palette,
    cx: &mut Context<GodPdfApp>,
) -> Stateful<Div> {
    let color = if active {
        palette.text
    } else {
        palette.text_dim
    };
    let close = widgets::small_icon_button(("close-tab", index), icons::X, "Close", palette)
        .on_click(cx.listener(move |app, _, window, cx| {
            cx.stop_propagation();
            app.request_close(index, window, cx);
        }));
    let label = div()
        .flex()
        .items_center()
        .min_w(px(40.))
        .max_w(px(TAB_TEXT_MAX + if dirty { 14. } else { 0. }))
        .text_size(px(14.))
        .text_color(color)
        .child(
            div()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(title.clone()),
        )
        .when(dirty, |label| {
            label.child(div().pl(px(4.)).text_size(px(15.)).child("*"))
        });
    let tip: SharedString = if dirty {
        format!("{title} (unsaved changes)").into()
    } else {
        title
    };
    let tab = tab_shape(("tab", index), active, palette)
        // Widths of egui: 12 + 16 + 8 + text + 8 + close + 8.
        .child(
            content()
                .pl(px(12.))
                .pr(px(8.))
                .gap(px(8.))
                .child(icon(icons::FILE_PDF, 16.0, color))
                .child(label)
                .child(close),
        )
        .on_click(cx.listener(move |app, _, _, cx| {
            app.active = index;
            app.shell.home = false;
            cx.notify();
        }))
        .on_mouse_down(
            MouseButton::Middle,
            cx.listener(move |app, _, window, cx| app.request_close(index, window, cx)),
        );
    widgets::tooltip(tab, tip)
}
