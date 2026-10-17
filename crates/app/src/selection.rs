//! Select tool: hover, select, move, resize and open annotations on a page; select text.
//!
//! Coordinates: "displayed" = page points after /Rotate, origin top-left (what the viewer lays
//! out); "screen" = displayed × zoom + page rect origin; PDF = unrotated page space.

use egui::{Color32, CursorIcon, Id, Pos2, Rect, Sense, Stroke, TextureId, Ui, Vec2, pos2};
use godpdf_core::Annotation;

use crate::app::egui_color;
use crate::doc::DocTab;
use crate::geometry::{self, PageGeom};
use crate::quickbar;
use crate::textbox;
use crate::textsel::{self, TextSel};

/// Extra screen pixels around an annotation that still count as hitting it.
const HIT_SLOP: f32 = 3.0;
/// Drags shorter than this (displayed points) are treated as clicks.
const MIN_MOVE: f32 = 0.5;
/// Smallest side of a resized annotation, in points.
const MIN_SIZE: f32 = 8.0;
/// Screen pixels around a corner handle that grab it.
const HANDLE_HIT: f32 = 8.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selected {
    pub page: usize,
    pub id: String,
}

/// An annotation being dragged; `offset` in displayed points.
pub struct Drag {
    pub page: usize,
    pub id: String,
    pub offset: Vec2,
}

/// A corner of the selected annotation being dragged to resize it.
pub struct Resize {
    pub page: usize,
    pub id: String,
    /// 0 top-left, 1 top-right, 2 bottom-right, 3 bottom-left.
    pub corner: usize,
    /// The new displayed rect so far.
    pub rect: Rect,
}

/// Stand-in for an annotation shown somewhere else than in the rendered page: while it is
/// dragged or resized, and after the drop until the page is re-rendered. It is drawn over the
/// page rendered without the annotation, so only the annotation moves. Rects in displayed
/// points.
pub struct Ghost {
    pub page: usize,
    pub id: String,
    /// Where the annotation is in the page texture.
    pub src: Rect,
    /// Where to show it.
    pub dst: Rect,
    /// The annotation as it is being moved or resized.
    pub annotation: Annotation,
    /// Show the pixels of `src` moved to `dst` (a note: exactly the icon the page shows)
    /// instead of drawing the annotation anew.
    pub pixels: bool,
}

/// Opacity of a highlight drawn by the app (previews), like the one written into the PDF.
pub const HIGHLIGHT_OPACITY: f32 = 0.6;

/// What a drag does to the selected annotation, in displayed points.
#[derive(Clone, Copy)]
enum Change {
    Move(Vec2),
    Resize(Rect),
}

#[derive(Default)]
pub struct CanvasState {
    pub selected: Option<Selected>,
    pub drag: Option<Drag>,
    pub resize: Option<Resize>,
    /// Shown between a move / resize and the re-rendered page.
    pub ghost: Option<Ghost>,
    /// Page text selected by dragging over it.
    pub text: Option<TextSel>,
}

/// What the app should do after a Select-tool interaction.
pub enum Action {
    /// Write the annotation `id` moved to `moved`.
    Move {
        page: usize,
        id: String,
        moved: Annotation,
    },
    /// Write the annotation `id` resized to `resized`.
    Resize {
        page: usize,
        id: String,
        resized: Annotation,
    },
    /// Double-click on a FreeText or Note: edit it. `point` in displayed page points.
    Open { page: usize, point: Pos2 },
}

/// Topmost (last drawn) rectangle containing `pos`, allowing `slop` around each.
pub fn hit_test(rects: &[Rect], pos: Pos2, slop: f32) -> Option<usize> {
    rects
        .iter()
        .rposition(|rect| rect.expand(slop).contains(pos))
}

/// A displayed-space offset expressed in PDF space (rotation aware).
pub fn pdf_delta(geom: &PageGeom, displayed: Vec2, offset: Vec2) -> (f32, f32) {
    let (x0, y0) = geometry::to_pdf(geom, displayed, Pos2::ZERO);
    let (x1, y1) = geometry::to_pdf(geom, displayed, Pos2::ZERO + offset);
    (x1 - x0, y1 - y0)
}

/// Limit `offset` so `item` (displayed rect) stays on a page of size `page`.
pub fn clamp_offset(item: Rect, page: Vec2, offset: Vec2) -> Vec2 {
    let range = |low: f32, high: f32, value: f32| {
        if low > high {
            0.0
        } else {
            value.clamp(low, high)
        }
    };
    Vec2::new(
        range(-item.min.x, page.x - item.max.x, offset.x),
        range(-item.min.y, page.y - item.max.y, offset.y),
    )
}

/// Displayed point -> screen.
pub fn to_screen(page_rect: Rect, zoom: f32, p: Pos2) -> Pos2 {
    page_rect.min + p.to_vec2() * zoom
}

/// Screen -> displayed point.
pub fn to_displayed(page_rect: Rect, zoom: f32, p: Pos2) -> Pos2 {
    ((p - page_rect.min) / zoom).to_pos2()
}

struct Item {
    id: String,
    /// Displayed rect.
    rect: Rect,
    movable: bool,
    editable: bool,
    /// Free text: only its width can be resized; the height follows the text.
    text: bool,
    /// Opaque-on-blank kinds (text, notes, pictures): hide the original while a ghost shows it
    /// elsewhere.
    cover: bool,
    picture: bool,
}

impl Item {
    fn resizable(&self) -> bool {
        self.movable && (self.text || !self.cover)
    }
}

/// Corners in the order top-left, top-right, bottom-right, bottom-left.
fn corners(rect: Rect) -> [Pos2; 4] {
    [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ]
}

/// The displayed rect after dragging `corner` of `original` to `pointer`: the opposite corner
/// stays put, the result stays on the page and keeps a minimum size. Text boxes only change
/// width.
pub fn resized_rect(original: Rect, corner: usize, pointer: Pos2, page: Vec2, text: bool) -> Rect {
    let fixed = corners(original)[(corner + 2) % 4];
    let pointer = pos2(pointer.x.clamp(0.0, page.x), pointer.y.clamp(0.0, page.y));
    // Never flip through the fixed corner, never get smaller than MIN_SIZE.
    let span = |fixed: f32, pointer: f32, grows: bool| {
        if grows {
            (fixed, pointer.max(fixed + MIN_SIZE))
        } else {
            (pointer.min(fixed - MIN_SIZE), fixed)
        }
    };
    let x = span(fixed.x, pointer.x, corner == 1 || corner == 2);
    let y = if text {
        (original.min.y, original.max.y)
    } else {
        span(fixed.y, pointer.y, corner == 2 || corner == 3)
    };
    Rect::from_min_max(pos2(x.0, y.0), pos2(x.1, y.1))
}

/// A page shown at `rect` (screen) with `zoom`, to place PDF-space things on screen.
struct View {
    rect: Rect,
    zoom: f32,
    geom: PageGeom,
    /// Displayed page size in points.
    displayed: Vec2,
}

impl View {
    /// Displayed rect -> screen.
    fn screen(&self, rect: Rect) -> Rect {
        Rect::from_min_max(
            to_screen(self.rect, self.zoom, rect.min),
            to_screen(self.rect, self.zoom, rect.max),
        )
    }

    fn pdf_point(&self, point: (f32, f32)) -> Pos2 {
        let shown = geometry::from_pdf(&self.geom, self.displayed, point);
        to_screen(self.rect, self.zoom, shown)
    }

    fn pdf_rect(&self, rect: &godpdf_core::Rect) -> Rect {
        self.screen(geometry::from_pdf_rect(&self.geom, self.displayed, rect))
    }
}

/// Draw `annotation` the way the PDF appearance looks.
fn paint_shape(painter: &egui::Painter, view: &View, annotation: &Annotation) {
    let zoom = view.zoom;
    match annotation {
        Annotation::Highlight { rect, color } => {
            let fill = egui_color(*color).gamma_multiply(HIGHLIGHT_OPACITY);
            painter.rect_filled(view.pdf_rect(rect), 0.0, fill);
        }
        Annotation::Square { rect, color, width } => {
            painter.rect_stroke(
                view.pdf_rect(rect),
                0.0,
                Stroke::new(width * zoom, egui_color(*color)),
                egui::StrokeKind::Inside,
            );
        }
        Annotation::Ink {
            strokes,
            color,
            width,
        } => {
            let color = egui_color(*color);
            for stroke in strokes {
                let points: Vec<Pos2> = stroke.iter().map(|point| view.pdf_point(*point)).collect();
                match points.as_slice() {
                    [dot] => {
                        painter.circle_filled(*dot, width * zoom / 2.0, color);
                    }
                    _ => {
                        painter.line(points, Stroke::new(width * zoom, color));
                    }
                }
            }
        }
        Annotation::FreeText {
            rect,
            text,
            font_size,
            color,
            rotation,
            font,
            styles,
        } => {
            let frame = view.pdf_rect(rect);
            let turn = view.geom.rotation - rotation;
            let shape = textbox::Shape {
                text,
                styles,
                font: *font,
                font_size: *font_size,
                color: egui_color(*color),
            };
            textbox::paint(painter, frame, turn, &shape, zoom);
        }
        // Shown with the pixels of the page (see `Ghost::pixels`).
        Annotation::Note { .. } | Annotation::Image { .. } => {}
    }
}

/// Paint `ghost` over its page: the page without the annotation (`base`, when it is ready),
/// then the annotation where it is now. `texture` is the page as rendered.
fn paint_ghost(
    painter: &egui::Painter,
    view: &View,
    texture: Option<TextureId>,
    base: Option<TextureId>,
    ghost: &Ghost,
) {
    let page_rect = view.rect;
    let src = view.screen(ghost.src);
    let whole = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    if let Some(base) = base {
        painter.image(base, page_rect, whole, Color32::WHITE);
    } else if ghost.pixels {
        // Until the base arrives: a note sits on blank paper as a rule, hide it with white.
        painter.rect_filled(src.expand(1.0), 0.0, Color32::WHITE);
    }
    if !ghost.pixels {
        paint_shape(painter, view, &ghost.annotation);
    } else if let Some(texture) = texture {
        let uv = Rect::from_min_max(
            ((src.min - page_rect.min) / page_rect.size()).to_pos2(),
            ((src.max - page_rect.min) / page_rect.size()).to_pos2(),
        );
        painter.image(texture, view.screen(ghost.dst), uv, Color32::WHITE);
    }
}

/// Select-tool input and overlays for one page shown at `page_rect` (screen).
pub fn page(
    ui: &mut Ui,
    tab: &mut DocTab,
    page: usize,
    page_rect: Rect,
    texture: Option<TextureId>,
) -> Option<Action> {
    let zoom = tab.zoom;
    let displayed = *tab.page_sizes.get(page)?;
    let geom = tab.page_geom(page).ok()?;
    let view = View {
        rect: page_rect,
        zoom,
        geom,
        displayed,
    };
    let chars = tab.text_chars(page);
    let fresh = tab.page_fresh(page);
    let items: Vec<Item> = tab
        .annotations(page)
        .ok()?
        .iter()
        .map(|placed| {
            let text = matches!(placed.annotation, Some(Annotation::FreeText { .. }));
            let note = matches!(placed.annotation, Some(Annotation::Note { .. }));
            let picture = matches!(placed.annotation, Some(Annotation::Image { .. }));
            Item {
                id: placed.id.clone(),
                rect: geometry::from_pdf_rect(&geom, displayed, &placed.rect),
                movable: placed.annotation.is_some(),
                editable: text || note,
                text,
                cover: text || note || picture,
                picture,
            }
        })
        .collect();
    let state = &mut tab.canvas;
    // Forget a selection whose annotation is gone (undo, page edits).
    if state
        .selected
        .as_ref()
        .is_some_and(|sel| sel.page == page && !items.iter().any(|item| item.id == sel.id))
    {
        state.selected = None;
    }
    // The re-rendered page has arrived: the stand-in pixels are no longer needed.
    if fresh && state.ghost.as_ref().is_some_and(|ghost| ghost.page == page) {
        state.ghost = None;
    }

    let screen_rects: Vec<Rect> = items.iter().map(|item| view.screen(item.rect)).collect();
    let response = ui.interact(
        page_rect,
        Id::new(("page-select", tab.id, page)),
        Sense::click_and_drag(),
    );
    let hit_at = |pos: Option<Pos2>| pos.and_then(|pos| hit_test(&screen_rects, pos, HIT_SLOP));
    let selected_index = |state: &CanvasState| {
        state
            .selected
            .as_ref()
            .filter(|sel| sel.page == page)
            .and_then(|sel| items.iter().position(|item| item.id == sel.id))
    };
    // Corner handle of the selected annotation under `pos`.
    let handle_at = |state: &CanvasState, pos: Option<Pos2>| {
        let index = selected_index(state).filter(|&index| items[index].resizable())?;
        let pos = pos?;
        corners(screen_rects[index].expand(2.0))
            .iter()
            .position(|corner| corner.distance(pos) <= HANDLE_HIT)
            .map(|corner| (index, corner))
    };
    let mut action = None;

    if response.drag_started() {
        let origin = ui.input(|input| input.pointer.press_origin());
        state.drag = None;
        state.resize = None;
        state.text = None;
        if let Some((index, corner)) = handle_at(state, origin) {
            state.resize = Some(Resize {
                page,
                id: items[index].id.clone(),
                corner,
                rect: items[index].rect,
            });
        } else {
            state.selected = hit_at(origin).map(|index| {
                if items[index].movable {
                    state.drag = Some(Drag {
                        page,
                        id: items[index].id.clone(),
                        offset: Vec2::ZERO,
                    });
                }
                Selected {
                    page,
                    id: items[index].id.clone(),
                }
            });
            // Not on a comment: a drag starting on text selects text.
            state.text = origin
                .filter(|_| state.selected.is_none())
                .and_then(|pos| textsel::char_near(&chars, to_displayed(page_rect, zoom, pos)))
                .map(|index| TextSel {
                    page,
                    anchor: index,
                    head: index,
                });
        }
    }
    if response.dragged()
        && let Some(sel) = state.text.as_mut().filter(|sel| sel.page == page)
        && let Some(pos) = response.interact_pointer_pos()
        && let Some(head) = textsel::nearest(&chars, to_displayed(page_rect, zoom, pos))
    {
        sel.head = head;
    }

    // A finished move / resize: (index of the annotation, what changed).
    let mut dropped: Option<(usize, Change)> = None;
    // Resize: the dragged corner follows the pointer.
    if let Some(resize) = state.resize.as_mut().filter(|resize| resize.page == page)
        && let Some(index) = items.iter().position(|item| item.id == resize.id)
    {
        let item = &items[index];
        if response.dragged()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let pointer = to_displayed(page_rect, zoom, pos);
            resize.rect = resized_rect(item.rect, resize.corner, pointer, displayed, item.text);
        }
        if response.drag_stopped() {
            if resize.rect != item.rect {
                dropped = Some((index, Change::Resize(resize.rect)));
            }
            state.resize = None;
        }
    }
    // Move: the whole annotation follows the pointer.
    if let Some(drag) = state.drag.as_mut().filter(|drag| drag.page == page)
        && let Some(index) = items.iter().position(|item| item.id == drag.id)
    {
        let item = &items[index];
        if response.dragged() {
            let wanted = drag.offset + response.drag_delta() / zoom;
            drag.offset = clamp_offset(item.rect, displayed, wanted);
        }
        if response.drag_stopped() {
            if drag.offset.length() >= MIN_MOVE {
                dropped = Some((index, Change::Move(drag.offset)));
            }
            state.drag = None;
        }
    }
    if response.clicked() {
        state.text = None;
        state.selected = hit_at(response.interact_pointer_pos()).map(|index| Selected {
            page,
            id: items[index].id.clone(),
        });
    }
    if response.double_clicked()
        && let Some(pos) = response.interact_pointer_pos()
        && let Some(index) = hit_at(Some(pos))
        && items[index].editable
    {
        action = Some(Action::Open {
            page,
            point: to_displayed(page_rect, zoom, pos),
        });
    }

    // Cursor.
    let hovered = hit_at(response.hover_pos());
    let selected = selected_index(state);
    let resizing = state.resize.as_ref().filter(|resize| resize.page == page);
    let dragging = state.drag.as_ref().filter(|drag| drag.page == page);
    let corner_cursor = |index: usize, corner: usize| {
        if items[index].text {
            CursorIcon::ResizeHorizontal
        } else if corner.is_multiple_of(2) {
            CursorIcon::ResizeNwSe
        } else {
            CursorIcon::ResizeNeSw
        }
    };
    if let (Some(resize), Some(index)) = (resizing, selected) {
        ui.ctx()
            .set_cursor_icon(corner_cursor(index, resize.corner));
    } else if dragging.is_some() {
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    } else if let Some((index, corner)) = handle_at(state, response.hover_pos()) {
        ui.ctx().set_cursor_icon(corner_cursor(index, corner));
    } else if let Some(index) = hovered {
        ui.ctx().set_cursor_icon(if items[index].movable {
            CursorIcon::Move
        } else {
            CursorIcon::PointingHand
        });
    } else if response
        .hover_pos()
        .is_some_and(|pos| textsel::char_near(&chars, to_displayed(page_rect, zoom, pos)).is_some())
    {
        ui.ctx().set_cursor_icon(CursorIcon::Text);
    }

    // What the drag in progress does to the selected annotation.
    let changing = selected.and_then(|index| {
        if let Some(resize) = resizing {
            return Some((index, Change::Resize(resize.rect)));
        }
        let offset = dragging
            .map(|drag| drag.offset)
            .filter(|offset| *offset != Vec2::ZERO)?;
        Some((index, Change::Move(offset)))
    });

    // The annotation of `items[index]` after `change`, and where it shows.
    let ghost_of = |tab: &mut DocTab, index: usize, change: Change| {
        let item = &items[index];
        let list = tab.annotations(page).ok()?;
        let placed = list.iter().find(|placed| placed.id == item.id)?;
        let annotation = placed.annotation.as_ref()?;
        let (annotation, dst) = match change {
            Change::Move(offset) => {
                let (dx, dy) = pdf_delta(&geom, displayed, offset);
                (annotation.translated(dx, dy), item.rect.translate(offset))
            }
            Change::Resize(rect) => {
                let to = geometry::to_pdf_rect(&geom, displayed, rect.min, rect.max);
                let fitted = annotation.fitted(placed.rect, to);
                // A text box settles on the height of its re-wrapped text.
                let dst = match &fitted {
                    Annotation::FreeText { rect, .. } => {
                        geometry::from_pdf_rect(&geom, displayed, rect)
                    }
                    _ => rect,
                };
                (fitted, dst)
            }
        };
        Some(Ghost {
            page,
            id: item.id.clone(),
            src: item.rect,
            dst,
            annotation,
            // Text is drawn anew: its pixels would carry along what the page shows behind
            // it (an image, a coloured background). A note is only its small icon.
            // A picture is its pixels, stretched when resized.
            pixels: item.picture || (item.cover && !item.text && matches!(change, Change::Move(_))),
        })
    };
    let live = changing.and_then(|(index, change)| ghost_of(tab, index, change));
    // Dropped: write the edit, and keep showing the result until the page is re-rendered.
    if let Some((index, change)) = dropped
        && let Some(ghost) = ghost_of(tab, index, change)
    {
        let (id, annotation) = (ghost.id.clone(), ghost.annotation.clone());
        action = Some(match change {
            Change::Move(_) => Action::Move {
                page,
                id,
                moved: annotation,
            },
            Change::Resize(_) => Action::Resize {
                page,
                id,
                resized: annotation,
            },
        });
        tab.canvas.ghost = Some(ghost);
    }
    // Have the page without the annotation ready before it is grabbed: the one under the
    // pointer, else the selected one.
    let settling = tab
        .canvas
        .ghost
        .as_ref()
        .is_some_and(|ghost| ghost.page == page);
    let grabbable = if live.is_some() {
        selected
    } else {
        hovered.filter(|&index| items[index].movable).or(selected)
    };
    if !settling && let Some(index) = grabbable {
        tab.lift(ui.ctx(), page, &items[index].id);
    }

    // Overlays.
    let painter = ui.painter_at(page_rect);
    let settled = tab.canvas.ghost.as_ref().filter(|ghost| ghost.page == page);
    let ghost = live.as_ref().or(settled);
    if let Some(ghost) = ghost {
        let base = tab.lifted(page, &ghost.id);
        paint_ghost(&painter, &view, texture, base, ghost);
    }
    if let Some(sel) = tab.canvas.text.as_ref().filter(|sel| sel.page == page) {
        paint_bands(&painter, &chars, sel, page_rect, zoom, TEXT_SELECTION);
        // Ctrl+C (egui turns it into a Copy event) copies the selected text.
        if ui.input(|input| input.events.contains(&egui::Event::Copy)) {
            ui.ctx().copy_text(textsel::text(&chars, sel.range()));
        }
    }
    if let Some(index) = hovered
        && Some(index) != selected
        && live.is_none()
    {
        painter.rect_stroke(
            screen_rects[index].expand(1.0),
            0.0,
            Stroke::new(1.0, Color32::from_gray(0x77)),
            egui::StrokeKind::Middle,
        );
    }
    if let Some(index) = selected {
        let item = &items[index];
        let rect = ghost
            .filter(|ghost| ghost.id == item.id)
            .map_or(item.rect, |ghost| ghost.dst);
        let outline = view.screen(rect).expand(2.0);
        quickbar::dashed_rect(&painter, outline, Stroke::new(1.0, Color32::BLACK));
        if item.resizable() {
            quickbar::handles(&painter, outline);
        }
    }
    action
}

/// Fill of selected page text (grey: the UI stays monochrome).
const TEXT_SELECTION: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 56);

fn paint_bands(
    painter: &egui::Painter,
    chars: &[godpdf_render::TextChar],
    sel: &TextSel,
    page_rect: Rect,
    zoom: f32,
    fill: Color32,
) {
    for band in textsel::bands(chars, sel.range()) {
        let screen = Rect::from_min_max(
            to_screen(page_rect, zoom, band.min),
            to_screen(page_rect, zoom, band.max),
        );
        painter.rect_filled(screen, 0.0, fill);
    }
}

/// What a Highlight-tool drag over a page did.
pub enum TextDrag {
    /// The pointer is not on text: the caller falls back to the free rectangle.
    Off,
    /// Text is being selected.
    Active,
    /// Released: highlight these line rectangles (displayed points).
    Done(Vec<Rect>),
}

/// Highlight tool over text: drag selects text (previewed in `color`), release highlights it.
pub fn highlight_text(
    ui: &mut Ui,
    tab: &mut DocTab,
    page: usize,
    page_rect: Rect,
    color: Color32,
) -> TextDrag {
    let zoom = tab.zoom;
    let chars = tab.text_chars(page);
    let at = |pos: Pos2| to_displayed(page_rect, zoom, pos);
    let active = tab.canvas.text.as_ref().is_some_and(|sel| sel.page == page);
    let on_text = ui
        .input(|input| input.pointer.hover_pos())
        .filter(|pos| page_rect.contains(*pos))
        .is_some_and(|pos| textsel::char_near(&chars, at(pos)).is_some());
    if tab.gesture.is_some() || !(active || on_text) {
        return TextDrag::Off;
    }
    let response = ui
        .interact(
            page_rect,
            Id::new(("page-highlight-text", tab.id, page)),
            Sense::drag(),
        )
        .on_hover_cursor(CursorIcon::Text);
    let state = &mut tab.canvas;
    if response.drag_started() {
        state.text = ui
            .input(|input| input.pointer.press_origin())
            .and_then(|pos| textsel::char_near(&chars, at(pos)))
            .map(|index| TextSel {
                page,
                anchor: index,
                head: index,
            });
    }
    let Some(sel) = state.text.as_mut().filter(|sel| sel.page == page) else {
        return TextDrag::Active;
    };
    if response.dragged()
        && let Some(pos) = response.interact_pointer_pos()
        && let Some(head) = textsel::nearest(&chars, at(pos))
    {
        sel.head = head;
    }
    paint_bands(
        &ui.painter_at(page_rect),
        &chars,
        sel,
        page_rect,
        zoom,
        color.gamma_multiply(0.4),
    );
    if response.drag_stopped() {
        let bands = textsel::bands(&chars, sel.range());
        state.text = None;
        return TextDrag::Done(bands);
    }
    TextDrag::Active
}

/// Eraser tool on one page: the id of the annotation under a click or drag, if any.
pub fn erase(ui: &mut Ui, tab: &mut DocTab, page: usize, page_rect: Rect) -> Option<String> {
    let zoom = tab.zoom;
    let displayed = *tab.page_sizes.get(page)?;
    let geom = tab.page_geom(page).ok()?;
    let (ids, rects): (Vec<String>, Vec<Rect>) = tab
        .annotations(page)
        .ok()?
        .iter()
        .map(|placed| {
            let shown = geometry::from_pdf_rect(&geom, displayed, &placed.rect);
            let screen = Rect::from_min_max(
                to_screen(page_rect, zoom, shown.min),
                to_screen(page_rect, zoom, shown.max),
            );
            (placed.id.clone(), screen)
        })
        .unzip();
    let response = ui
        .interact(
            page_rect,
            Id::new(("page-erase", tab.id, page)),
            Sense::click_and_drag(),
        )
        .on_hover_cursor(CursorIcon::Crosshair);
    if let Some(index) = response
        .hover_pos()
        .and_then(|pos| hit_test(&rects, pos, HIT_SLOP))
    {
        quickbar::dashed_rect(
            &ui.painter_at(page_rect),
            rects[index].expand(2.0),
            Stroke::new(1.0, Color32::BLACK),
        );
    }
    let erasing = response.clicked() || response.dragged();
    let pos = response.interact_pointer_pos().filter(|_| erasing)?;
    hit_test(&rects, pos, HIT_SLOP).map(|index| ids[index].clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;

    fn geom(rotation: i32) -> PageGeom {
        PageGeom {
            rotation,
            origin: (0.0, 0.0),
        }
    }

    #[test]
    fn hit_test_prefers_topmost() {
        let rects = [
            Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 100.0)),
            Rect::from_min_size(pos2(50.0, 50.0), vec2(20.0, 20.0)),
        ];
        assert_eq!(hit_test(&rects, pos2(60.0, 60.0), 0.0), Some(1));
        assert_eq!(hit_test(&rects, pos2(10.0, 10.0), 0.0), Some(0));
        assert_eq!(hit_test(&rects, pos2(102.0, 10.0), 3.0), Some(0));
        assert_eq!(hit_test(&rects, pos2(200.0, 10.0), 3.0), None);
    }

    #[test]
    fn delta_flips_y_and_follows_rotation() {
        let shown = vec2(600.0, 800.0);
        assert_eq!(pdf_delta(&geom(0), shown, vec2(10.0, 20.0)), (10.0, -20.0));
        assert_eq!(
            pdf_delta(&geom(180), shown, vec2(10.0, 20.0)),
            (-10.0, 20.0)
        );
        let landscape = vec2(800.0, 600.0);
        assert_eq!(
            pdf_delta(&geom(90), landscape, vec2(10.0, 20.0)),
            (20.0, 10.0)
        );
        assert_eq!(
            pdf_delta(&geom(270), landscape, vec2(10.0, 20.0)),
            (-20.0, -10.0)
        );
    }

    #[test]
    fn moved_annotation_lands_under_the_pointer() {
        // A rect at displayed (100, 100)-(150, 120) on a 600x800 page dragged by (30, 40).
        let shown = vec2(600.0, 800.0);
        let g = geom(0);
        let rect = geometry::to_pdf_rect(&g, shown, pos2(100.0, 100.0), pos2(150.0, 120.0));
        let (dx, dy) = pdf_delta(&g, shown, vec2(30.0, 40.0));
        let moved = rect.translated(dx, dy);
        let back = geometry::from_pdf_rect(&g, shown, &moved);
        assert_eq!(back.min, pos2(130.0, 140.0));
        assert_eq!(back.max, pos2(180.0, 160.0));
    }

    #[test]
    fn offset_is_clamped_to_the_page() {
        let item = Rect::from_min_size(pos2(10.0, 10.0), vec2(20.0, 20.0));
        let page = vec2(100.0, 100.0);
        assert_eq!(
            clamp_offset(item, page, vec2(-50.0, 500.0)),
            vec2(-10.0, 70.0)
        );
        assert_eq!(clamp_offset(item, page, vec2(5.0, 5.0)), vec2(5.0, 5.0));
    }

    #[test]
    fn resizing_keeps_the_opposite_corner_and_a_minimum_size() {
        let original = Rect::from_min_max(pos2(100.0, 100.0), pos2(200.0, 150.0));
        let page = vec2(600.0, 800.0);
        // Bottom-right corner dragged out.
        let grown = resized_rect(original, 2, pos2(300.0, 400.0), page, false);
        assert_eq!(
            grown,
            Rect::from_min_max(pos2(100.0, 100.0), pos2(300.0, 400.0))
        );
        // Top-left corner dragged past the fixed corner: clamped to the minimum size.
        let tiny = resized_rect(original, 0, pos2(500.0, 500.0), page, false);
        assert_eq!(
            tiny,
            Rect::from_min_max(pos2(192.0, 142.0), pos2(200.0, 150.0))
        );
        // Never leaves the page.
        let edge = resized_rect(original, 2, pos2(5000.0, 5000.0), page, false);
        assert_eq!(edge.max, pos2(600.0, 800.0));
        // Text boxes only change width.
        let text = resized_rect(original, 1, pos2(400.0, 0.0), page, true);
        assert_eq!(
            text,
            Rect::from_min_max(pos2(100.0, 100.0), pos2(400.0, 150.0))
        );
    }

    #[test]
    fn screen_mapping_round_trips() {
        let page = Rect::from_min_size(pos2(40.5, 12.25), vec2(612.0, 792.0));
        let p = pos2(123.0, 456.0);
        let screen = to_screen(page, 1.5, p);
        assert_eq!(to_displayed(page, 1.5, screen), p);
    }
}
