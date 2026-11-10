//! Right icon rail (egui `doc_panels` "rail" + `GodApp::rail`): the view toggles at the top,
//! page box / previous / next / zoom at the bottom. Width 52, fill `bg`, margin 0 × 10,
//! a `border` line on its left. The panel a toggle opens is `panels::right`.

use gpui::prelude::*;
use gpui::{AnyElement, Context, Entity, div, px};

use super::FitPage;
use crate::app::{FitWidth, GodPdfApp, ZoomIn, ZoomOut};
use crate::dialogs::GoToPage;
use crate::doc::DocTab;
use crate::panels::{self, Panel};
use crate::theme::{self, Palette};
use crate::widgets::{self, dispatch};
use crate::{fonts, icons};

const WIDTH: f32 = 52.0;

pub fn render(tab: &Entity<DocTab>, palette: &Palette, cx: &mut Context<GodPdfApp>) -> AnyElement {
    let open = panels::open_panel(cx);
    let views = Panel::ALL.map(|view| {
        widgets::rail_button(
            view.title(),
            view.icon(),
            view.title(),
            open == Some(view),
            true,
            palette,
        )
        .on_click(move |_, _, cx| panels::rail_click(view, cx))
    });

    let doc = tab.read(cx);
    let (current, count, zoom) = (doc.view.current_page, doc.page_count(), doc.view.zoom);
    let goto = |page: usize| {
        let tab = tab.downgrade();
        move |_: &gpui::ClickEvent, _: &mut gpui::Window, cx: &mut gpui::App| {
            let _ = tab.update(cx, |tab, cx| {
                tab.scroll_to_page(page);
                cx.notify();
            });
        }
    };
    let rail =
        |id, glyph, tip, enabled| widgets::rail_button(id, glyph, tip, false, enabled, palette);
    let previous = rail(
        "previous-page",
        icons::CARET_UP,
        "Previous page",
        current > 0,
    )
    .when(current > 0, |button| {
        button.on_click(goto(current.saturating_sub(1)))
    });
    let next = rail(
        "next-page",
        icons::CARET_DOWN,
        "Next page",
        current + 1 < count,
    )
    .when(current + 1 < count, |button| {
        button.on_click(goto(current + 1))
    });

    let dim = |text: String| {
        div()
            .text_size(px(12.))
            .text_color(palette.text_dim)
            .child(text)
    };
    // TODO(rail): egui's page box is a DragValue (drag or click to type); this one opens Go
    // to page (Ctrl+G).
    let page_box = widgets::tooltip(
        div()
            .id("page-box")
            .w(px(38.))
            .h(px(theme::INTERACT_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(theme::RADIUS))
            .bg(palette.surface)
            .border_1()
            .border_color(palette.border)
            .hover({
                let hover = palette.hover;
                move |style| style.bg(hover)
            })
            .cursor_pointer()
            .text_size(px(fonts::BODY))
            .child((current + 1).min(count.max(1)).to_string())
            .on_click(dispatch(GoToPage)),
        "Page number (drag or click to type)",
    );

    let bottom = div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(4.))
        .child(page_box)
        .child(dim(format!("/ {count}")))
        .child(previous)
        .child(next)
        .child(div().h(px(8.)))
        .child(dim(format!("{:.0}%", zoom * 100.0)))
        .child(
            rail(
                "fit-width",
                icons::ARROWS_OUT_LINE_HORIZONTAL,
                "Fit width (Ctrl+0)",
                true,
            )
            .on_click(dispatch(FitWidth)),
        )
        .child(rail("fit-page", icons::FRAME_CORNERS, "Fit page", true).on_click(dispatch(FitPage)))
        .child(
            rail(
                "zoom-in",
                icons::MAGNIFYING_GLASS_PLUS,
                "Zoom in (Ctrl++)",
                true,
            )
            .on_click(dispatch(ZoomIn)),
        )
        .child(
            rail(
                "zoom-out",
                icons::MAGNIFYING_GLASS_MINUS,
                "Zoom out (Ctrl+-)",
                true,
            )
            .on_click(dispatch(ZoomOut)),
        );

    div()
        .w(px(WIDTH))
        .flex_none()
        .h_full()
        .py(px(10.))
        .flex()
        .flex_col()
        .items_center()
        .justify_between()
        .bg(palette.bg)
        .border_l_1()
        .border_color(palette.border)
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(6.))
                .children(views),
        )
        .child(bottom)
        .into_any_element()
}
