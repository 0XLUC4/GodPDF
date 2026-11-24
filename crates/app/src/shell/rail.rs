//! Right icon rail: the view toggles at the top,
//! page box / previous / next / zoom at the bottom. Width 52, fill `bg`, margin 0 × 10,
//! a `border` line on its left. The panel a toggle opens is `panels::right`.

use std::cell::Cell;
use std::rc::Rc;

use gpui::component::input::{Input, InputEvent, InputState};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, DragMoveEvent, EmptyView, Entity, EntityId, Global, Pixels, Point,
    Subscription, WeakEntity, div, px,
};

use super::FitPage;
use crate::app::{FitWidth, GodPdfApp, ZoomIn, ZoomOut};
use crate::doc::DocTab;
use crate::geometry::Spread;
use crate::panels::{self, Panel};
use crate::theme::{self, Palette};
use crate::viewer::{Present, SinglePageView, TwoPageView};
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
    // Search is the find bar over the viewer, not a panel.
    let find = widgets::rail_button(
        "find",
        icons::MAGNIFYING_GLASS,
        "Find (Ctrl+F)",
        crate::find::is_open(tab, cx),
        true,
        palette,
    )
    .on_click(dispatch(panels::Find));

    let doc = tab.read(cx);
    let (current, count, zoom) = (doc.view.current_page, doc.page_count(), doc.view.zoom);
    // Previous and next rows of pages, one page each unless two sit side by side.
    let (before, after) = (doc.neighbour_page(false), doc.neighbour_page(true));
    let two_pages = doc.view.spread != Spread::Single;
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
        before.is_some(),
    )
    .when_some(before, |button, page| button.on_click(goto(page)));
    let next = rail("next-page", icons::CARET_DOWN, "Next page", after.is_some())
        .when_some(after, |button, page| button.on_click(goto(page)));
    let spread = move |_: &gpui::ClickEvent, window: &mut gpui::Window, cx: &mut App| {
        let action: Box<dyn gpui::Action> = if two_pages {
            Box::new(SinglePageView)
        } else {
            Box::new(TwoPageView)
        };
        window.dispatch_action(action, cx);
    };

    let dim = |text: String| {
        div()
            .text_size(px(12.))
            .text_color(palette.text_dim)
            .child(text)
    };
    let page_box = page_box(tab, current, count, palette, cx);
    let bottom = div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(4.))
        .child(page_box)
        .child(dim(format!("/ {count}")))
        .child(previous)
        .child(next)
        // 8 px of space after a 4 px item gap: 4 + 4 + 4.
        .child(div().h(px(4.)))
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
            widgets::rail_button(
                "two-pages",
                icons::COLUMNS,
                "Two pages side by side",
                two_pages,
                true,
                palette,
            )
            .on_click(spread),
        )
        .child(
            rail("present", icons::CORNERS_OUT, "Presentation (F5)", true)
                .on_click(dispatch(Present)),
        )
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

    // Scrolls when the window is too short for both groups.
    div()
        .id("rail")
        .overflow_y_scroll()
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
                .children(views)
                .child(find),
        )
        .child(bottom)
        .into_any_element()
}

/// The page box while it is typed in, with the tab it belongs to.
#[derive(Default)]
struct PageField(Option<(EntityId, Entity<InputState>, Subscription)>);

impl Global for PageField {}

/// Payload of a drag on the page box: where it started.
struct PageDrag {
    from: usize,
    /// Pointer offset in the box at the drag start (set by the drag constructor).
    offset: Rc<Cell<Point<Pixels>>>,
}

/// Page number box, 38 × 26: drag right / up for later pages (one
/// page per point), click to type a number (Enter or leaving the field goes there, Esc
/// cancels).
fn page_box(
    tab: &Entity<DocTab>,
    current: usize,
    count: usize,
    palette: &Palette,
    cx: &mut Context<GodPdfApp>,
) -> AnyElement {
    let size = |element: gpui::Div| element.w(px(38.)).h(px(theme::INTERACT_HEIGHT));
    if let Some((_, field, _)) = cx
        .try_global::<PageField>()
        .and_then(|field| field.0.as_ref())
        .filter(|(owner, _, _)| *owner == tab.entity_id())
    {
        return size(div())
            .on_key_down(|event, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.set_global(PageField::default());
                    cx.refresh_windows();
                }
            })
            .child(Input::new(field).size_full())
            .into_any_element();
    }
    let shown = (current + 1).min(count.max(1));
    let (weak, drag_tab) = (tab.downgrade(), tab.downgrade());
    let hover = palette.hover;
    let element = size(div())
        .id("page-box")
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme::RADIUS))
        .bg(palette.surface)
        .border_1()
        .border_color(palette.border)
        .hover(move |style| style.bg(hover))
        .cursor_ew_resize()
        .text_size(px(fonts::BODY))
        .child(shown.to_string())
        .on_click(move |_, window, cx| edit(&weak, shown, window, cx))
        .on_drag(
            PageDrag {
                from: current,
                offset: Rc::default(),
            },
            |drag, offset, _, cx| {
                drag.offset.set(offset);
                cx.new(|_| EmptyView)
            },
        )
        .on_drag_move(move |event: &DragMoveEvent<PageDrag>, _, cx| {
            let drag = event.drag(cx);
            let start = event.bounds.origin + drag.offset.get();
            let moved = event.event.position - start;
            let delta = f32::from(moved.x) - f32::from(moved.y);
            let page = dragged_page(drag.from, delta, count);
            let _ = drag_tab.update(cx, |tab, cx| {
                if tab.view.current_page != page {
                    tab.scroll_to_page(page);
                    cx.notify();
                }
            });
        });
    widgets::tooltip(element, "Page number (drag or click to type)").into_any_element()
}

/// Page reached by dragging `delta` points from page `from` (one page per point, clamped).
fn dragged_page(from: usize, delta: f32, count: usize) -> usize {
    let page = from as f32 + delta.round();
    page.clamp(0.0, count.saturating_sub(1) as f32) as usize
}

/// Turn the box into a text field holding `shown`, selected and focused.
fn edit(tab: &WeakEntity<DocTab>, shown: usize, window: &mut gpui::Window, cx: &mut App) {
    let field = cx.new(|cx| InputState::new(window, cx).default_value(shown.to_string()));
    field.update(cx, |field, cx| {
        field.focus(window, cx);
        field.select_all(window, cx);
    });
    let owner = tab.entity_id();
    let tab = tab.clone();
    let subscription = cx.subscribe(&field, move |field, event: &InputEvent, cx| {
        if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
            let typed = field.read(cx).value().trim().parse::<usize>().ok();
            cx.set_global(PageField::default());
            let _ = tab.update(cx, |tab, cx| {
                if let Some(page) = typed.filter(|&page| page >= 1) {
                    tab.scroll_to_page((page - 1).min(tab.page_count().saturating_sub(1)));
                }
                cx.notify();
            });
            cx.refresh_windows();
        }
    });
    cx.set_global(PageField(Some((owner, field, subscription))));
    cx.refresh_windows();
}

#[cfg(test)]
mod tests {
    use super::dragged_page;

    #[test]
    fn dragging_moves_one_page_per_point_within_the_document() {
        assert_eq!(dragged_page(3, 2.4, 10), 5);
        assert_eq!(dragged_page(3, -1.6, 10), 1);
        assert_eq!(dragged_page(3, -50.0, 10), 0);
        assert_eq!(dragged_page(3, 50.0, 10), 9);
        assert_eq!(dragged_page(0, 5.0, 0), 0);
    }
}
