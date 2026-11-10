//! Pages panel: one thumbnail per page. Click jumps, drag onto another page moves it there,
//! right-click rotates, deletes, inserts or extracts.

use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use gpui::component::menu::ContextMenuExt;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, ObjectFit, RenderImage, SharedString, WeakEntity, Window,
    div, img, px, uniform_list, white,
};

use super::{BOX, menu_entry};
use crate::doc::DocTab;
use crate::theme::Palette;
use crate::{files, icons, toasts, widgets};

const ROW_HEIGHT: f32 = 196.;
/// Thumbnails rendering at once.
const MAX_IN_FLIGHT: usize = 4;
/// Thumbnails kept beyond the visible rows, on each side.
const KEEP_AROUND: usize = 10;

/// Rendered thumbnails of one document.
#[derive(Default)]
pub struct Thumbs {
    /// Page -> image and the document generation it shows (older = stale but shown).
    images: HashMap<usize, (Arc<RenderImage>, u64)>,
    /// Generation and page count the in-flight jobs and errors belong to.
    generation: u64,
    count: usize,
    in_flight: HashSet<usize>,
    failed: HashSet<usize>,
    /// Images dropped from the cache; their GPU textures are freed on the next render.
    evicted: Vec<Arc<RenderImage>>,
}

/// Payload of a thumbnail being dragged.
struct DraggedPage {
    page: usize,
}

/// What follows the pointer while a thumbnail is dragged.
struct DragGhost {
    label: SharedString,
    palette: Palette,
}

impl Render for DragGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(10.))
            .py(px(4.))
            .rounded(px(6.))
            .bg(self.palette.accent)
            .text_color(self.palette.on_accent)
            .text_sm()
            .child(self.label.clone())
    }
}

/// New page order after moving page `from` so it ends at index `to`; `None` when nothing moves.
pub fn move_to(count: usize, from: usize, to: usize) -> Option<Vec<usize>> {
    if from == to || from >= count || to >= count {
        return None;
    }
    let mut order: Vec<usize> = (0..count).filter(|&page| page != from).collect();
    order.insert(to, from);
    Some(order)
}

pub fn render(tab: &Entity<DocTab>, window: &mut Window, cx: &mut App) -> AnyElement {
    let palette = Palette::of(cx);
    let count = tab.update(cx, |tab, _| {
        tab.panels.thumbs.free_evicted(window);
        tab.page_count()
    });
    let entity = tab.clone();
    uniform_list("thumbs", count, move |rows, window, cx| {
        let max_px = (BOX.0.max(BOX.1) * window.scale_factor()).round() as u32;
        let weak = entity.downgrade();
        entity.update(cx, |tab, cx| {
            trim(tab, rows.clone(), PANEL);
            rows.map(|page| {
                let image = thumbnail(tab, page, max_px, cx, PANEL);
                let current = page == tab.view.current_page;
                let size = fit(tab.page_sizes.get(page).copied().unwrap_or((612.0, 792.0)));
                row(
                    &weak,
                    page,
                    tab.page_count(),
                    image.map(|image| (image, size)),
                    current,
                    &palette,
                )
            })
            .collect::<Vec<_>>()
        })
    })
    .flex_1()
    .into_any_element()
}

/// One row as egui `sidebar::pages` draws it: 196 high; hover / current fill 2 px in,
/// radius 8 (current = accent at 18 %); the page fitted in a box 32 px narrower than the row
/// and 156 high, centered 84 px from the top, white, 2 px accent outline when current; the
/// number 13 centered 16 px above the bottom; Move up / down arrows on hover.
fn row(
    tab: &WeakEntity<DocTab>,
    page: usize,
    count: usize,
    image: Option<(Arc<RenderImage>, (f32, f32))>,
    current: bool,
    palette: &Palette,
) -> impl IntoElement + use<> {
    let palette = *palette;
    let thumb = match image {
        Some((image, (width, height))) => div()
            .bg(white())
            .when(current, |thumb| {
                // egui strokes outside the picture.
                thumb.m(px(-2.)).border_2().border_color(palette.accent)
            })
            .child(
                img(image)
                    .w(px(width))
                    .h(px(height))
                    .object_fit(ObjectFit::Fill),
            ),
        None => div()
            .w(px(BOX.0 - 48.))
            .h(px(BOX.1 - 48.))
            .rounded(px(2.))
            .bg(palette.canvas),
    };
    let arrow = |shown: bool, top: f32, glyph, tip, delta: isize| {
        let tab = tab.clone();
        shown.then(|| {
            widgets::small_icon_button(
                ("thumb-move", page * 2 + (delta > 0) as usize),
                glyph,
                tip,
                &palette,
            )
            .absolute()
            .top(px(top - 2.))
            .right(px(10. - 2.))
            .invisible()
            .group_hover("thumb-row", |style| style.visible())
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                let to = page.saturating_add_signed(delta);
                reorder(&tab, page, to, cx);
            })
        })
    };
    let fill = if current {
        palette.accent.opacity(0.18)
    } else {
        gpui::transparent_black()
    };
    let jump_to = tab.clone();
    let drop_on = tab.clone();
    let menu_of = tab.clone();
    let inner = div()
        .relative()
        .size_full()
        .rounded(px(8.))
        .bg(fill)
        .when(!current, |inner| {
            inner.group_hover("thumb-row", move |style| style.bg(palette.hover))
        })
        .child(
            div()
                .absolute()
                .top(px(84. - BOX.1 / 2. - 2.))
                .left_0()
                .right_0()
                .h(px(BOX.1))
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
                .text_color(if current {
                    palette.accent_text
                } else {
                    palette.text_dim
                })
                .child(format!("{}", page + 1)),
        )
        .children(arrow(page > 0, 8., icons::CARET_UP, "Move up", -1))
        .children(arrow(
            page + 1 < count,
            12. + widgets::SMALL_ICON_BUTTON,
            icons::CARET_DOWN,
            "Move down",
            1,
        ));
    div()
        .id(("thumb", page))
        .group("thumb-row")
        .h(px(ROW_HEIGHT))
        .p(px(2.))
        .cursor_pointer()
        .child(inner)
        .on_click(move |_, _, cx| {
            update(&jump_to, cx, |tab, _| tab.scroll_to_page(page));
        })
        .on_drag(DraggedPage { page }, move |dragged, _, _, cx| {
            let label = format!("Page {}", dragged.page + 1).into();
            cx.new(|_| DragGhost { label, palette })
        })
        .drag_over::<DraggedPage>(move |style, _, _, _| {
            style.border_2().border_color(palette.accent)
        })
        .on_drop(move |dragged: &DraggedPage, _, cx| {
            reorder(&drop_on, dragged.page, page, cx);
        })
        .context_menu(move |menu, _, _| page_menu(menu, &menu_of, page, count, &palette))
}

fn page_menu(
    menu: gpui::component::menu::PopupMenu,
    tab: &WeakEntity<DocTab>,
    page: usize,
    count: usize,
    palette: &Palette,
) -> gpui::component::menu::PopupMenu {
    let item = |glyph: &'static str,
                label: &'static str,
                enabled: bool,
                run: fn(&WeakEntity<DocTab>, usize, &mut App)| {
        let tab = tab.clone();
        menu_entry(glyph, label, enabled, palette).on_click(move |_, _, cx| run(&tab, page, cx))
    };
    menu.item(item(
        icons::ARROW_COUNTER_CLOCKWISE,
        "Rotate left",
        true,
        |tab, page, cx| rotate(tab, page, -90, cx),
    ))
    .item(item(
        icons::ARROW_CLOCKWISE,
        "Rotate right",
        true,
        |tab, page, cx| rotate(tab, page, 90, cx),
    ))
    .separator()
    .item(item(
        icons::FILE_PLUS,
        "Insert blank page after",
        true,
        insert_blank,
    ))
    .item(item(
        icons::FILE_ARROW_DOWN,
        "Insert PDF after…",
        true,
        insert_file,
    ))
    .item(item(icons::EXPORT, "Extract page…", true, extract))
    .separator()
    .item(item(icons::TRASH, "Delete page", count > 1, delete))
}

/// Run `update` on `tab` if it is still open, then redraw it.
fn update(
    tab: &WeakEntity<DocTab>,
    cx: &mut App,
    update: impl FnOnce(&mut DocTab, &mut Context<DocTab>),
) {
    // The tab may be closed meanwhile: nothing to do then.
    let _ = tab.update(cx, |tab, cx| {
        update(tab, cx);
        cx.notify();
    });
}

/// Run a core edit on `tab`; errors become toasts.
fn edit(
    tab: &WeakEntity<DocTab>,
    cx: &mut App,
    label: &str,
    edit: impl FnOnce(&[u8]) -> godpdf_core::Result<Vec<u8>>,
) -> bool {
    let mut done = false;
    update(tab, cx, |tab, cx| match tab.apply(label, edit) {
        Ok(()) => done = true,
        Err(err) => toasts::error(cx, err),
    });
    done
}

fn reorder(tab: &WeakEntity<DocTab>, from: usize, to: usize, cx: &mut App) {
    let Some(count) = tab.upgrade().map(|tab| tab.read(cx).page_count()) else {
        return;
    };
    let Some(order) = move_to(count, from, to) else {
        return;
    };
    if edit(tab, cx, "Move page", |pdf| {
        godpdf_core::reorder_pages(pdf, &order)
    }) {
        update(tab, cx, |tab, _| tab.scroll_to_page(to));
    }
}

fn rotate(tab: &WeakEntity<DocTab>, page: usize, degrees: i32, cx: &mut App) {
    let label = if degrees < 0 {
        "Rotate left"
    } else {
        "Rotate right"
    };
    edit(tab, cx, label, |pdf| {
        godpdf_core::rotate_pages(pdf, &[page], degrees)
    });
}

fn delete(tab: &WeakEntity<DocTab>, page: usize, cx: &mut App) {
    edit(tab, cx, "Delete 1 page(s)", |pdf| {
        godpdf_core::delete_pages(pdf, &[page])
    });
}

fn insert_blank(tab: &WeakEntity<DocTab>, page: usize, cx: &mut App) {
    let Some(size) = tab
        .upgrade()
        .and_then(|tab| tab.read(cx).page_sizes.get(page).copied())
    else {
        return;
    };
    edit(tab, cx, "Insert blank page", |pdf| {
        godpdf_core::insert_blank_page(pdf, page + 1, size.0, size.1)
    });
}

fn insert_file(tab: &WeakEntity<DocTab>, page: usize, cx: &mut App) {
    crate::organize::insert_files(tab, page + 1, cx);
}

fn extract(tab: &WeakEntity<DocTab>, page: usize, cx: &mut App) {
    let Some(tab) = tab.upgrade() else { return };
    let tab = tab.read(cx);
    let stem = tab
        .title()
        .trim_end_matches(".pdf")
        .trim_end_matches(".PDF");
    let name = format!("{stem} - page {}.pdf", page + 1);
    let extracted =
        crate::doc::guarded(|| godpdf_core::extract_pages(tab.session.bytes(), &[page]))
            .map_err(|err| err.to_string())
            .and_then(|bytes| files::save_pdf(&name, None, &bytes));
    match extracted {
        Ok(Some(path)) => toasts::success(cx, format!("Page saved to {}", path.display())),
        Ok(None) => {}
        Err(err) => toasts::error(cx, err),
    }
}

/// Logical size of the thumbnail of a page of `size` points, fitted in [`BOX`].
fn fit(size: (f32, f32)) -> (f32, f32) {
    let scale = (BOX.0 / size.0.max(1.0)).min(BOX.1 / size.1.max(1.0));
    (size.0 * scale, size.1 * scale)
}

/// Which thumbnail cache of a document: the panel's or the Organize grid's.
pub type Pick = fn(&mut DocTab) -> &mut Thumbs;

const PANEL: Pick = |tab| &mut tab.panels.thumbs;

impl Thumbs {
    /// Free the GPU textures of the thumbnails dropped from the cache.
    pub fn free_evicted(&mut self, window: &mut Window) {
        for image in self.evicted.drain(..) {
            // Fails only for an image never painted: nothing to free then.
            let _ = window.drop_image(image);
        }
    }
}

/// Drop thumbnails far from the visible `rows`, and forget everything on a page count change.
pub fn trim(tab: &mut DocTab, rows: std::ops::Range<usize>, pick: Pick) {
    let generation = tab.generation();
    let count = tab.page_count();
    let thumbs = pick(tab);
    if thumbs.generation != generation {
        thumbs.generation = generation;
        thumbs.in_flight.clear();
        thumbs.failed.clear();
    }
    let keep = rows.start.saturating_sub(KEEP_AROUND)..rows.end + KEEP_AROUND;
    let far: Vec<usize> = thumbs
        .images
        .keys()
        .copied()
        .filter(|page| !keep.contains(page) || thumbs.count != count)
        .collect();
    for page in far {
        if let Some((image, _)) = thumbs.images.remove(&page) {
            thumbs.evicted.push(image);
        }
    }
    thumbs.count = count;
}

/// Thumbnail of `page`, rendering it when missing or stale (the stale one shows meanwhile).
pub fn thumbnail(
    tab: &mut DocTab,
    page: usize,
    max_px: u32,
    cx: &mut Context<DocTab>,
    pick: Pick,
) -> Option<Arc<RenderImage>> {
    let generation = tab.generation();
    let renderer = tab.renderer();
    let thumbs = pick(tab);
    let fresh = thumbs
        .images
        .get(&page)
        .is_some_and(|(_, made)| *made == generation);
    if !fresh
        && !thumbs.failed.contains(&page)
        && thumbs.in_flight.len() < MAX_IN_FLIGHT
        && let Some(doc) = renderer
        && thumbs.in_flight.insert(page)
    {
        let rendered = cx.background_executor().spawn(async move {
            catch_unwind(AssertUnwindSafe(|| doc.thumbnail(page, max_px)))
                .map_err(|_| "renderer crashed".to_owned())
                .and_then(|bitmap| bitmap.map_err(|err| err.to_string()))
                .and_then(crate::doc::to_render_image)
        });
        cx.spawn(async move |this, cx| {
            let result = rendered.await;
            let _ = this.update(cx, |tab, cx| {
                if tab.generation() != generation {
                    return;
                }
                let thumbs = pick(tab);
                thumbs.in_flight.remove(&page);
                match result {
                    Ok(image) => {
                        if let Some((old, _)) =
                            thumbs.images.insert(page, (Arc::new(image), generation))
                        {
                            thumbs.evicted.push(old);
                        }
                    }
                    // The thumbnail stays a grey box; the viewer shows the page's error.
                    Err(_) => {
                        thumbs.failed.insert(page);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    pick(tab)
        .images
        .get(&page)
        .map(|(image, _)| Arc::clone(image))
}

#[cfg(test)]
mod tests {
    use super::{fit, move_to};

    #[test]
    fn thumbnails_fit_their_box() {
        let (width, height) = super::BOX;
        assert_eq!(fit((612.0, 792.0)).1, height);
        assert_eq!(fit((800.0, 400.0)), (width, width / 2.0));
    }

    #[test]
    fn a_dropped_page_lands_at_the_target_index() {
        assert_eq!(move_to(4, 0, 2), Some(vec![1, 2, 0, 3]));
        assert_eq!(move_to(4, 3, 0), Some(vec![3, 0, 1, 2]));
        assert_eq!(move_to(4, 1, 3), Some(vec![0, 2, 3, 1]));
    }

    #[test]
    fn no_move_gives_none() {
        assert_eq!(move_to(4, 2, 2), None);
        assert_eq!(move_to(4, 4, 0), None);
        assert_eq!(move_to(4, 0, 4), None);
    }
}
