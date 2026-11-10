//! The Organize page grid: a tool bar of page commands, then a card
//! per page. Click selects (Ctrl toggles, Shift selects a run), double-click opens the page
//! in the reader, drag moves the page (or the selection it is in) before or after another,
//! the arrows of a card move it one place.

use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    Action, AnyElement, App, ClickEvent, Context, Entity, ObjectFit, RenderImage, WeakEntity,
    Window, canvas, div, img, px, uniform_list, white,
};

use super::{InsertFile, MoveEarlier, MoveLater, apply_order, move_pages, order};
use crate::dialogs::{
    DeletePage, ExtractPages, InsertBlankPage, RotateAll, RotateLeft, RotateRight,
};
use crate::doc::DocTab;
use crate::theme::Palette;
use crate::{fonts, icons, panels, widgets};

/// A page card and the box its thumbnail is fitted in.
const CARD: (f32, f32) = (168., 218.);
const THUMB_BOX: (f32, f32) = (144., 180.);
/// Space between cards, and before the first one.
const GAP: f32 = 14.;
const INDENT: f32 = 12.;
const PICK: panels::Pick = |tab| &mut tab.grid.thumbs;

/// A tool bar button: the command, its icon, label and description.
type ToolButton = (
    fn() -> Box<dyn Action>,
    &'static str,
    &'static str,
    &'static str,
);

const TOOLBAR: [ToolButton; 9] = [
    (
        || Box::new(RotateLeft),
        icons::ARROW_COUNTER_CLOCKWISE,
        "Rotate left",
        "Turn the current page by 90°",
    ),
    (
        || Box::new(RotateRight),
        icons::ARROW_CLOCKWISE,
        "Rotate right",
        "Turn the current page by 90°",
    ),
    (
        || Box::new(RotateAll),
        icons::ARROWS_CLOCKWISE,
        "Rotate all pages",
        "Turn every page by 90°",
    ),
    (
        || Box::new(MoveEarlier),
        icons::CARET_LEFT,
        "Move earlier",
        "Move the selected pages one place earlier",
    ),
    (
        || Box::new(MoveLater),
        icons::CARET_RIGHT,
        "Move later",
        "Move the selected pages one place later",
    ),
    (
        || Box::new(DeletePage),
        icons::TRASH,
        "Delete pages",
        "Remove pages for good",
    ),
    (
        || Box::new(ExtractPages),
        icons::EXPORT,
        "Extract pages…",
        "Copy pages into a new PDF",
    ),
    (
        || Box::new(InsertBlankPage),
        icons::FILE_PLUS,
        "Insert blank page",
        "Add an empty page",
    ),
    (
        || Box::new(InsertFile),
        icons::FILE_ARROW_DOWN,
        "Insert from file…",
        "Insert another PDF's pages",
    ),
];

/// Cards on a row of a grid `width` wide.
fn columns(width: f32) -> usize {
    (((width - INDENT + GAP) / (CARD.0 + GAP)).floor() as usize).max(1)
}

pub fn render(tab: &Entity<DocTab>, window: &mut Window, cx: &mut App) -> AnyElement {
    let palette = Palette::of(cx);
    let (count, selected, width) = tab.update(cx, |tab, _| {
        tab.grid.thumbs.free_evicted(window);
        (tab.page_count(), tab.grid.selection.len(), tab.grid.width)
    });
    let per_row = columns(width);
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(toolbar(tab, count, selected, &palette))
        .child(
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .bg(palette.bg)
                .child(measure(tab))
                // Not measured yet (first frame): one column would flash before the real ones.
                .when(width > 0., |grid| {
                    grid.child(cards(tab, count, per_row, &palette))
                }),
        )
        .into_any_element()
}

/// Learn the grid width after layout, for the cards per row.
fn measure(tab: &Entity<DocTab>) -> impl IntoElement {
    let tab = tab.downgrade();
    canvas(
        move |bounds, _, cx| {
            let width = f32::from(bounds.size.width);
            let _ = tab.update(cx, |tab, cx| {
                if (tab.grid.width - width).abs() > 0.5 {
                    tab.grid.width = width;
                    cx.notify();
                }
            });
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_full()
}

/// The tool bar: panel fill, margin 16 × 10, wrapped framed buttons (glyph + label, 28 high), a
/// separator, Select all / Clear selection, then "k of n selected".
fn toolbar(tab: &Entity<DocTab>, count: usize, selected: usize, palette: &Palette) -> AnyElement {
    let buttons = TOOLBAR
        .iter()
        .enumerate()
        .map(|(index, &(action, glyph, label, tip))| {
            let content = widgets::with_icon(glyph, label, fonts::BODY, palette.text);
            let button = widgets::framed_button(("organize-tool", index), content, true, palette)
                .h(px(widgets::ICON_BUTTON))
                .on_click(move |_, window, cx| window.dispatch_action(action(), cx));
            widgets::tooltip(button, tip)
        });
    let (all, none) = (tab.downgrade(), tab.downgrade());
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(crate::theme::ITEM_SPACING.0))
        .px(px(16.))
        .py(px(10.))
        .bg(palette.panel)
        .border_b_1()
        .border_color(palette.border)
        .children(buttons)
        .child(div().w(px(1.)).h(px(20.)).bg(palette.border))
        .child(
            widgets::icon_button("select-all", icons::SELECTION_ALL, "Select all", palette)
                .on_click(move |_, _, cx| {
                    update(&all, cx, |tab| {
                        tab.grid.selection = (0..tab.page_count()).collect();
                    });
                }),
        )
        .child(
            widgets::icon_button(
                "clear-selection",
                icons::SELECTION_SLASH,
                "Clear selection",
                palette,
            )
            .on_click(move |_, _, cx| update(&none, cx, |tab| tab.grid.selection.clear())),
        )
        .child(
            div()
                .text_color(palette.text_dim)
                .child(format!("{selected} of {count} selected")),
        )
        .into_any_element()
}

fn update(tab: &WeakEntity<DocTab>, cx: &mut App, change: impl FnOnce(&mut DocTab)) {
    // The tab may be closed meanwhile: nothing to do then.
    let _ = tab.update(cx, |tab, cx| {
        change(tab);
        cx.notify();
    });
}

/// Rows of cards; only the rows on screen are built (and their thumbnails rendered).
fn cards(tab: &Entity<DocTab>, count: usize, per_row: usize, palette: &Palette) -> AnyElement {
    let palette = *palette;
    let entity = tab.clone();
    uniform_list(
        "organize-rows",
        count.div_ceil(per_row),
        move |rows, window, cx| {
            let max_px = (THUMB_BOX.0.max(THUMB_BOX.1) * window.scale_factor()).round() as u32;
            let weak = entity.downgrade();
            entity.update(cx, |tab, cx| {
                let pages = rows.start * per_row..(rows.end * per_row).min(count);
                panels::trim(tab, pages, PICK);
                rows.map(|row| {
                    let first = row * per_row;
                    let cards = (first..(first + per_row).min(count))
                        .map(|page| card(&weak, tab, page, max_px, &palette, cx))
                        .collect::<Vec<_>>();
                    div()
                        .h(px(CARD.1 + GAP))
                        .pl(px(INDENT))
                        .flex()
                        .gap(px(GAP))
                        .children(cards)
                })
                .collect::<Vec<_>>()
            })
        },
    )
    .size_full()
    .pt(px(16.))
    .pb(px(24.))
    .into_any_element()
}

/// Payload of a card being dragged.
struct DraggedCard {
    page: usize,
}

/// A page card: 168 × 218, radius 10, surface fill (hover fill when hovered), 1 px border or
/// 2 px accent when selected; the page fitted in 144 × 180 at 12 from the top, white; its
/// number (13) 14 above the bottom; earlier / later arrows in the bottom corners.
fn card(
    weak: &WeakEntity<DocTab>,
    tab: &mut DocTab,
    page: usize,
    max_px: u32,
    palette: &Palette,
    cx: &mut Context<DocTab>,
) -> AnyElement {
    let palette = *palette;
    let count = tab.page_count();
    let selected = tab.grid.selection.contains(&page);
    let size = tab.page_sizes.get(page).copied().unwrap_or((612., 792.));
    let image = panels::thumbnail(tab, page, max_px, cx, PICK);
    let moving = if selected {
        tab.grid.selection.len()
    } else {
        1
    };
    let hover = palette.hover;
    let thumb = thumb(image, size, &palette);
    let arrow = |shown: bool, glyph, tip, delta: isize| {
        let weak = weak.clone();
        shown.then(|| {
            let button = widgets::small_icon_button(
                ("organize-move", page * 2 + usize::from(delta > 0)),
                glyph,
                tip,
                &palette,
            )
            .absolute()
            .bottom(px(8.))
            .when(delta < 0, |button| button.left(px(8.)))
            .when(delta > 0, |button| button.right(px(8.)))
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                if let Some(tab) = weak.upgrade() {
                    move_pages(&tab, &[page], delta, cx);
                }
            });
            if selected {
                button
            } else {
                button
                    .invisible()
                    .group_hover("organize-card", |style| style.visible())
            }
        })
    };
    let (click, drop_before, drop_after) = (weak.clone(), weak.clone(), weak.clone());
    div()
        .id(("organize-card", page))
        .group("organize-card")
        .relative()
        .flex_none()
        .w(px(CARD.0))
        .h(px(CARD.1))
        .rounded(px(10.))
        .bg(palette.surface)
        .hover(move |style| style.bg(hover))
        .border_1()
        .border_color(palette.border)
        // Selection: a 2 px outline over the hairline, drawn on top so nothing inside moves.
        .when(selected, |card| {
            card.child(
                div()
                    .absolute()
                    .top(px(-1.))
                    .left(px(-1.))
                    .right(px(-1.))
                    .bottom(px(-1.))
                    .rounded(px(10.))
                    .border_2()
                    .border_color(palette.accent),
            )
        })
        .cursor_pointer()
        .child(
            div()
                .absolute()
                .top(px(12.))
                .left_0()
                .right_0()
                .h(px(THUMB_BOX.1))
                .flex()
                .items_center()
                .justify_center()
                .child(thumb),
        )
        .child(
            div()
                .absolute()
                .bottom_0()
                .left_0()
                .right_0()
                .h(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(13.))
                .text_color(if selected {
                    palette.accent_text
                } else {
                    palette.text_dim
                })
                .child(format!("{}", page + 1)),
        )
        .child(drop_zone(page, false, drop_before, palette))
        .child(drop_zone(page, true, drop_after, palette))
        .children(arrow(page > 0, icons::CARET_LEFT, "Move earlier", -1))
        .children(arrow(page + 1 < count, icons::CARET_RIGHT, "Move later", 1))
        .on_click(move |event: &ClickEvent, _, cx| clicked(&click, page, event, cx))
        .on_drag(DraggedCard { page }, move |_, _, _, cx| {
            let label = if moving > 1 {
                format!("{moving} pages")
            } else {
                format!("Page {}", page + 1)
            };
            cx.new(|_| widgets::DragGhost {
                label: label.into(),
                palette,
            })
        })
        .into_any_element()
}

/// The page fitted in [`THUMB_BOX`] on whole pixels on white, or a grey box of the same
/// size until it is rendered.
fn thumb(image: Option<Arc<RenderImage>>, size: (f32, f32), palette: &Palette) -> gpui::Div {
    let scale = (THUMB_BOX.0 / size.0.max(1.)).min(THUMB_BOX.1 / size.1.max(1.));
    let (width, height) = ((size.0 * scale).round(), (size.1 * scale).round());
    match image {
        Some(image) => div().bg(white()).child(
            img(image)
                .w(px(width))
                .h(px(height))
                .object_fit(ObjectFit::Fill),
        ),
        None => div()
            .w(px(width))
            .h(px(height))
            .rounded(px(2.))
            .bg(palette.canvas),
    }
}

/// Half of a card (and of the gap beside it) a dragged card drops on: before the page on the
/// left half, after it on the right; a 2 px accent line in the gap shows where.
fn drop_zone(
    page: usize,
    after: bool,
    tab: WeakEntity<DocTab>,
    palette: Palette,
) -> impl IntoElement {
    let accent = palette.accent;
    div()
        .absolute()
        .top_0()
        .bottom_0()
        // Whole pixels: the 2 px line sits centred in the gap, sharp.
        .w(px(CARD.0 / 2. + GAP / 2. + 1.))
        .when(after, |zone| zone.right(px(-GAP / 2. - 1.)))
        .when(!after, |zone| zone.left(px(-GAP / 2. - 1.)))
        .drag_over::<DraggedCard>(move |style, _, _, _| {
            if after {
                style.border_r_2().border_color(accent)
            } else {
                style.border_l_2().border_color(accent)
            }
        })
        .on_drop(move |dragged: &DraggedCard, _, cx| {
            let Some(tab) = tab.upgrade() else { return };
            let doc = tab.read(cx);
            if let Some((order, moved)) = order::reorder(
                doc.page_count(),
                &doc.grid.selection,
                dragged.page,
                page,
                after,
            ) {
                apply_order(&tab, "Reorder pages", &order, moved, cx);
            }
        })
}

/// Click: select (Ctrl toggles, Shift selects from the anchor); double-click: read the page.
fn clicked(tab: &WeakEntity<DocTab>, page: usize, event: &ClickEvent, cx: &mut App) {
    let modifiers = event.modifiers();
    let toggle = modifiers.secondary(cx);
    update(tab, cx, |tab| {
        if event.click_count() >= 2 {
            tab.grid.on = false;
            tab.view.current_page = page;
            tab.scroll_to_page(page);
            return;
        }
        let grid = &mut tab.grid;
        if modifiers.shift {
            let anchor = grid.anchor.unwrap_or(page);
            grid.selection = (anchor.min(page)..=anchor.max(page)).collect();
            return;
        }
        if toggle {
            if !grid.selection.remove(&page) {
                grid.selection.insert(page);
            }
        } else {
            grid.selection.clear();
            grid.selection.insert(page);
        }
        grid.anchor = Some(page);
        tab.view.current_page = page;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_fill_the_row() {
        assert_eq!(columns(0.), 1);
        assert_eq!(columns(INDENT + CARD.0), 1);
        assert_eq!(columns(INDENT + 2. * CARD.0 + GAP), 2);
        assert_eq!(columns(INDENT + 2. * CARD.0 + GAP - 1.), 1);
    }
}
