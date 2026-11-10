//! Per-page interactions: select / move / resize / delete annotations, draw new ones, erase,
//! pan, place pictures. State lives in [`Canvas`], a field of each `DocTab`.

use std::collections::HashMap;
use std::rc::Rc;

use godpdf_core::{Annotation, PageGeometry, Picture, PlacedAnnotation, Rect};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, CursorStyle, Div, Hsla, MouseButton, MouseDownEvent, MouseMoveEvent,
    PathBuilder, Pixels, Point, Window, black, canvas, div, point, px, rgb, white,
};

use super::geom::{self, DRect};
use super::{Tool, current_tool, editor, fields, hsla, sign, stamp, tool_state};
use crate::doc::{DocTab, guarded};
use crate::geometry::PageRect;
use crate::toasts;

/// Screen pixels around an annotation that still count as hitting it.
const HIT_SLOP: f32 = 3.0;
/// Screen pixels around a corner handle that grab it.
const HANDLE_HIT: f32 = 8.0;
const HANDLE_SIZE: f32 = 7.0;
/// Opacity of the highlight preview, as the egui app shows it.
const PREVIEW_OPACITY: f32 = 0.4;
/// Drags with a side shorter than this (points) are clicks.
const TINY: f32 = 3.0;
/// Outline of an area marked for redaction.
const REDACT_RED: u32 = 0xD92D20;

/// The annotation selected on a page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selected {
    pub page: usize,
    pub id: String,
}

/// A mouse gesture in progress. Points are displayed page points.
enum Gesture {
    /// A comment being drawn (first point, then the last one; every point for ink).
    Draw {
        page: usize,
        points: Vec<(f32, f32)>,
    },
    Move {
        page: usize,
        id: String,
        original: DRect,
        start: (f32, f32),
        offset: (f32, f32),
    },
    Resize {
        page: usize,
        id: String,
        /// 0 top-left, 1 top-right, 2 bottom-right, 3 bottom-left.
        corner: usize,
        original: DRect,
        rect: DRect,
        text: bool,
        picture: bool,
    },
    /// Hand tool; `last` in window coordinates.
    Pan { last: Point<Pixels> },
    /// Eraser drag; after the first removal the next ones fold into the same undo step.
    Erase { erased: bool },
}

impl Gesture {
    fn page(&self) -> Option<usize> {
        match self {
            Self::Draw { page, .. } | Self::Move { page, .. } | Self::Resize { page, .. } => {
                Some(*page)
            }
            Self::Pan { .. } | Self::Erase { .. } => None,
        }
    }
}

/// Tool state of one document.
#[derive(Default)]
pub struct Canvas {
    /// Generation of the document `annotations` were read from.
    generation: u64,
    annotations: HashMap<usize, Rc<Vec<PlacedAnnotation>>>,
    pub selected: Option<Selected>,
    gesture: Option<Gesture>,
    pub(super) editor: Option<editor::Editor>,
    /// Areas marked for redaction (page, PDF space), removed only by "Apply redactions".
    // shortcut: marks are not remapped when pages are moved or deleted; clear them there once page tools land.
    pub(super) redactions: Vec<(usize, Rect)>,
    /// Form field boxes and the text field being typed in.
    pub(super) fields: fields::Fields,
}

/// An annotation of a page as the tools see it.
pub(super) struct Item {
    pub id: String,
    pub rect: DRect,
    pub annotation: Option<Annotation>,
}

impl Item {
    /// Only our kinds can be rewritten; links, form fields... can only be erased.
    fn movable(&self) -> bool {
        self.annotation.is_some()
    }

    fn is_text(&self) -> bool {
        matches!(self.annotation, Some(Annotation::FreeText { .. }))
    }

    fn is_note(&self) -> bool {
        matches!(self.annotation, Some(Annotation::Note { .. }))
    }

    fn is_picture(&self) -> bool {
        matches!(self.annotation, Some(Annotation::Image { .. }))
    }

    /// A note is a fixed-size icon.
    fn resizable(&self) -> bool {
        self.movable() && !self.is_note()
    }
}

/// Annotations of `page`, parsed once per document revision.
pub(super) fn annotations(
    tab: &mut DocTab,
    page: usize,
    cx: &mut App,
) -> Rc<Vec<PlacedAnnotation>> {
    if tab.canvas.generation != tab.generation() {
        tab.canvas.annotations.clear();
        tab.canvas.generation = tab.generation();
    }
    if let Some(list) = tab.canvas.annotations.get(&page) {
        return Rc::clone(list);
    }
    let list =
        guarded(|| godpdf_core::annotations(tab.session.bytes(), page)).unwrap_or_else(|err| {
            toasts::error(
                cx,
                format!("Page {}: cannot read comments: {err}", page + 1),
            );
            Vec::new()
        });
    let list = Rc::new(list);
    tab.canvas.annotations.insert(page, Rc::clone(&list));
    list
}

/// Rotation / origin and displayed size of `page`.
pub(super) fn page_frame(tab: &mut DocTab, page: usize) -> Option<(PageGeometry, (f32, f32))> {
    let displayed = *tab.page_sizes.get(page)?;
    let geom = tab.page_geom(page).ok()?;
    Some((geom, displayed))
}

pub(super) fn items(tab: &mut DocTab, page: usize, cx: &mut App) -> Option<Vec<Item>> {
    let (geom, displayed) = page_frame(tab, page)?;
    let list = annotations(tab, page, cx);
    Some(
        list.iter()
            .map(|placed| Item {
                id: placed.id.clone(),
                rect: DRect::from_pdf(&geom, displayed, &placed.rect),
                annotation: placed.annotation.clone(),
            })
            .collect(),
    )
}

/// The selected annotation, as it is in the document.
pub(super) fn selected_item(tab: &mut DocTab, cx: &mut App) -> Option<(usize, Item)> {
    let selected = tab.canvas.selected.clone()?;
    let item = items(tab, selected.page, cx)?
        .into_iter()
        .find(|item| item.id == selected.id)?;
    Some((selected.page, item))
}

/// A window position as displayed page points of `page`.
fn page_point(tab: &DocTab, page: usize, position: Point<Pixels>) -> Option<(f32, f32)> {
    let rect = *tab.layout().rects.get(page)?;
    let zoom = tab.view.zoom;
    let origin = tab.view.bounds.origin;
    let x = f32::from(position.x - origin.x) + tab.view.scroll.0 - rect.x;
    let y = f32::from(position.y - origin.y) + tab.view.scroll.1 - rect.y;
    Some((x / zoom, y / zoom))
}

pub(super) fn report(result: Result<(), String>, cx: &mut App) {
    if let Err(err) = result {
        toasts::error(cx, err);
    }
}

/// Leave whatever is going on: finish the text being typed, drop gesture and selection.
pub fn reset(tab: &mut DocTab, cx: &mut Context<DocTab>) {
    editor::commit(tab, cx);
    tab.canvas.gesture = None;
    tab.canvas.selected = None;
}

pub fn delete_selected(tab: &mut DocTab, cx: &mut Context<DocTab>) {
    let Some(selected) = tab.canvas.selected.take() else {
        return;
    };
    let result = tab.apply("Delete comment", |pdf| {
        godpdf_core::remove_annotation(pdf, selected.page, &selected.id)
    });
    report(result, cx);
}

/// Rewrite annotation `id` of `page` with what `change` makes of it. `verb` names the undo step.
pub(super) fn rewrite(
    tab: &mut DocTab,
    page: usize,
    id: &str,
    verb: &str,
    change: impl FnOnce(&Annotation, &PlacedAnnotation, &PageGeometry, (f32, f32)) -> Annotation,
    cx: &mut App,
) {
    let Some((geom, displayed)) = page_frame(tab, page) else {
        return;
    };
    let list = annotations(tab, page, cx);
    let Some(placed) = list.iter().find(|placed| placed.id == id) else {
        return;
    };
    let Some(annotation) = &placed.annotation else {
        return;
    };
    let changed = change(annotation, placed, &geom, displayed);
    let label = format!("{verb} {}", changed.kind_name().to_lowercase());
    let result = tab.apply(&label, |pdf| {
        godpdf_core::replace_annotation(pdf, page, id, &changed)
    });
    report(result, cx);
}

fn on_down(
    tab: &mut DocTab,
    page: usize,
    event: &MouseDownEvent,
    window: &mut Window,
    cx: &mut Context<DocTab>,
) {
    let Some(point) = page_point(tab, page, event.position) else {
        return;
    };
    let tool = current_tool(cx);
    // A click outside the text being typed finishes it, and does nothing else for text tools.
    if tab.canvas.editor.is_some() {
        editor::commit(tab, cx);
        if matches!(tool, Tool::Text | Tool::Note) {
            cx.notify();
            return;
        }
    }
    match tool {
        Tool::Select => select_down(tab, page, point, event.click_count, window, cx),
        Tool::Hand => {
            tab.canvas.gesture = Some(Gesture::Pan {
                last: event.position,
            })
        }
        Tool::Text => editor::open_text(tab, page, point, true, window, cx),
        Tool::Note => editor::open_note(tab, page, point, None, window, cx),
        Tool::Eraser => {
            let erased = erase_at(tab, page, point, false, cx);
            tab.canvas.gesture = Some(Gesture::Erase { erased });
        }
        Tool::Stamp => place_stamp(tab, page, point, cx),
        Tool::Sign => sign::click(tab, page, point, cx),
        Tool::Redact if unmark_at(tab, page, point) => {}
        Tool::Highlight
        | Tool::Underline
        | Tool::StrikeOut
        | Tool::Ink
        | Tool::Rectangle
        | Tool::Ellipse
        | Tool::Line
        | Tool::Arrow
        | Tool::Redact
        | Tool::Picture
        | Tool::Field => {
            tab.canvas.gesture = Some(Gesture::Draw {
                page,
                points: vec![point],
            });
        }
    }
    cx.notify();
}

fn select_down(
    tab: &mut DocTab,
    page: usize,
    point: (f32, f32),
    clicks: usize,
    window: &mut Window,
    cx: &mut Context<DocTab>,
) {
    let Some(items) = items(tab, page, cx) else {
        return;
    };
    let zoom = tab.view.zoom;
    // A corner of the selected annotation resizes it.
    let selected = tab
        .canvas
        .selected
        .as_ref()
        .filter(|selected| selected.page == page)
        .and_then(|selected| items.iter().find(|item| item.id == selected.id))
        .filter(|item| item.resizable());
    if let Some(item) = selected {
        let grab = HANDLE_HIT / zoom;
        let corner = item.rect.corners().iter().position(|corner| {
            (corner.0 - point.0).abs() <= grab && (corner.1 - point.1).abs() <= grab
        });
        if let Some(corner) = corner {
            tab.canvas.gesture = Some(Gesture::Resize {
                page,
                id: item.id.clone(),
                corner,
                original: item.rect,
                rect: item.rect,
                text: item.is_text(),
                picture: item.is_picture(),
            });
            return;
        }
    }
    let rects: Vec<DRect> = items.iter().map(|item| item.rect).collect();
    let hit = geom::hit_test(&rects, point, HIT_SLOP / zoom).map(|index| &items[index]);
    tab.canvas.selected = hit.map(|item| Selected {
        page,
        id: item.id.clone(),
    });
    let Some(item) = hit else {
        return;
    };
    if clicks >= 2 && item.is_text() {
        editor::open_text(tab, page, point, false, window, cx);
    } else if clicks >= 2 && item.is_note() {
        editor::open_note(
            tab,
            page,
            point,
            Some((item.id.clone(), item.rect)),
            window,
            cx,
        );
    } else if item.movable() {
        tab.canvas.gesture = Some(Gesture::Move {
            page,
            id: item.id.clone(),
            original: item.rect,
            start: point,
            offset: (0.0, 0.0),
        });
    }
}

fn on_move(tab: &mut DocTab, page: usize, event: &MouseMoveEvent, cx: &mut Context<DocTab>) {
    let Some(gesture) = &tab.canvas.gesture else {
        return;
    };
    // The button went up somewhere we did not hear about.
    if !event.dragging() {
        finish(tab, cx);
        return;
    }
    let gesture_page = gesture.page();
    let point = gesture_page.and_then(|page| page_point(tab, page, event.position));
    let size = gesture_page
        .and_then(|page| tab.page_sizes.get(page).copied())
        .unwrap_or((0.0, 0.0));
    let mut pan = None;
    let mut erase = None;
    match tab.canvas.gesture.as_mut() {
        Some(Gesture::Draw { points, .. }) => {
            let Some(point) = point.map(|p| (p.0.clamp(0.0, size.0), p.1.clamp(0.0, size.1)))
            else {
                return;
            };
            if current_tool(cx) == Tool::Ink {
                if points.last() != Some(&point) {
                    points.push(point);
                }
            } else {
                points.truncate(1);
                points.push(point);
            }
        }
        Some(Gesture::Move {
            original,
            start,
            offset,
            ..
        }) => {
            let Some(point) = point else { return };
            let wanted = (point.0 - start.0, point.1 - start.1);
            *offset = geom::clamp_offset(*original, size, wanted);
        }
        Some(Gesture::Resize {
            corner,
            original,
            rect,
            text,
            picture,
            ..
        }) => {
            let Some(point) = point else { return };
            *rect = geom::resized_rect(*original, *corner, point, size, *text);
            // A picture keeps its proportions; Shift stretches it freely.
            if *picture && !event.modifiers.shift {
                *rect = geom::keep_ratio(*original, *rect, *corner);
            }
        }
        Some(Gesture::Pan { last }) => {
            pan = Some((
                f32::from(last.x - event.position.x),
                f32::from(last.y - event.position.y),
            ));
            *last = event.position;
        }
        Some(Gesture::Erase { erased }) => erase = Some(*erased),
        None => return,
    }
    if let Some(delta) = pan {
        tab.scroll_by(delta);
    }
    if let Some(erased) = erase
        && let Some(point) = page_point(tab, page, event.position)
        && erase_at(tab, page, point, erased, cx)
    {
        tab.canvas.gesture = Some(Gesture::Erase { erased: true });
    }
    cx.notify();
}

/// The button is up: write what the gesture made.
fn finish(tab: &mut DocTab, cx: &mut Context<DocTab>) {
    let Some(gesture) = tab.canvas.gesture.take() else {
        return;
    };
    match gesture {
        Gesture::Draw { page, points } => commit_draw(tab, page, &points, cx),
        Gesture::Move {
            page, id, offset, ..
        } if offset.0.hypot(offset.1) >= geom::MIN_MOVE => {
            let change =
                |annotation: &Annotation, _: &PlacedAnnotation, geom: &PageGeometry, displayed| {
                    let (dx, dy) = geom::pdf_delta(geom, displayed, offset);
                    annotation.translated(dx, dy)
                };
            rewrite(tab, page, &id, "Move", change, cx);
        }
        Gesture::Resize {
            page,
            id,
            original,
            rect,
            ..
        } if rect != original => {
            let change = |annotation: &Annotation,
                          placed: &PlacedAnnotation,
                          geom: &PageGeometry,
                          displayed| {
                annotation.fitted(placed.rect, rect.to_pdf(geom, displayed))
            };
            rewrite(tab, page, &id, "Resize", change, cx);
        }
        _ => {}
    }
    cx.notify();
}

fn commit_draw(tab: &mut DocTab, page: usize, points: &[(f32, f32)], cx: &mut Context<DocTab>) {
    let tool = current_tool(cx);
    let style = tool_state(cx).styles.of(tool);
    let (Some(&first), Some(&last)) = (points.first(), points.last()) else {
        return;
    };
    let Some((geom, displayed)) = page_frame(tab, page) else {
        return;
    };
    let tiny = (last.0 - first.0).abs().min((last.1 - first.1).abs()) < TINY;
    let short = (last.0 - first.0).hypot(last.1 - first.1) < TINY;
    let rect = DRect::span(first, last).to_pdf(&geom, displayed);
    let color = style.color;
    let width = style.width;
    let line = |arrow| Annotation::Line {
        start: crate::geometry::to_pdf(&geom, displayed, first),
        end: crate::geometry::to_pdf(&geom, displayed, last),
        color,
        width,
        arrow,
    };
    let (label, annotation) = match tool {
        Tool::Line | Tool::Arrow if short => return,
        Tool::Highlight
        | Tool::Underline
        | Tool::StrikeOut
        | Tool::Rectangle
        | Tool::Ellipse
        | Tool::Redact
            if tiny =>
        {
            return;
        }
        Tool::Redact => {
            tab.canvas.redactions.push((page, rect));
            return;
        }
        Tool::Highlight => ("Highlight", Annotation::Highlight { rect, color }),
        Tool::Underline => ("Underline", Annotation::Underline { rect, color }),
        Tool::StrikeOut => ("Strikethrough", Annotation::StrikeOut { rect, color }),
        Tool::Rectangle => ("Rectangle", Annotation::Square { rect, color, width }),
        Tool::Ellipse => ("Ellipse", Annotation::Circle { rect, color, width }),
        Tool::Line => ("Line", line(false)),
        Tool::Arrow => ("Arrow", line(true)),
        Tool::Ink if points.len() >= 2 => {
            let stroke = points
                .iter()
                .map(|&point| crate::geometry::to_pdf(&geom, displayed, point))
                .collect();
            (
                "Ink",
                Annotation::Ink {
                    strokes: vec![stroke],
                    color,
                    width,
                },
            )
        }
        Tool::Picture => {
            place_picture(tab, page, first, (!tiny).then_some(last), cx);
            return;
        }
        Tool::Field => {
            fields::new_field(tab, page, first, last, tiny, cx);
            return;
        }
        _ => return,
    };
    let result = tab.apply(label, |pdf| {
        godpdf_core::add_annotation(pdf, page, &annotation)
    });
    report(result, cx);
}

/// Ask for a PNG / JPEG and put it on `page`: at its own size from `from`, or fitted in the
/// dragged box `from`-`to`. It is then selected, to be moved or resized.
fn place_picture(
    tab: &mut DocTab,
    page: usize,
    from: (f32, f32),
    to: Option<(f32, f32)>,
    cx: &mut Context<DocTab>,
) {
    // Blocks until the dialog closes, as opening a PDF does.
    let Some(path) = rfd::FileDialog::new()
        .add_filter("Picture", &["png", "jpg", "jpeg"])
        .pick_file()
    else {
        return;
    };
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let picture = std::fs::read(&path)
        .map_err(|err| err.to_string())
        .and_then(|bytes| decode_picture(&bytes));
    let picture = match picture {
        Ok(picture) => picture,
        Err(err) => {
            toasts::error(cx, format!("Cannot use “{name}”: {err}"));
            return;
        }
    };
    let Some((geom, displayed)) = page_frame(tab, page) else {
        return;
    };
    let natural = natural_size(&picture);
    let rect = geom::place_picture(natural, displayed, from, to).to_pdf(&geom, displayed);
    let annotation = Annotation::Image { rect, picture };
    let result = tab.apply("Picture", |pdf| {
        godpdf_core::add_annotation(pdf, page, &annotation)
    });
    if let Err(err) = result {
        toasts::error(cx, err);
        return;
    }
    if let Some(placed) = annotations(tab, page, cx).last() {
        tab.canvas.selected = Some(Selected {
            page,
            id: placed.id.clone(),
        });
    }
    tool_state(cx).tool = Tool::Select;
    cx.refresh_windows();
    toasts::success(
        cx,
        "Picture added: drag it to move it, its corners to resize it",
    );
}

/// Put the stamp picked in the style card centred on `point`, then select it to be moved or
/// resized.
fn place_stamp(tab: &mut DocTab, page: usize, point: (f32, f32), cx: &mut Context<DocTab>) {
    let Some((geom, displayed)) = page_frame(tab, page) else {
        return;
    };
    let state = tool_state(cx);
    let (label, dated) = (state.stamp, state.stamp_date);
    let rect = stamp::stamp_rect(point, displayed, dated).to_pdf(&geom, displayed);
    let annotation = Annotation::Stamp {
        rect,
        label,
        date: dated.then(stamp::today),
    };
    let result = tab.apply("Stamp", |pdf| {
        godpdf_core::add_annotation(pdf, page, &annotation)
    });
    if let Err(err) = result {
        toasts::error(cx, err);
        return;
    }
    if let Some(placed) = annotations(tab, page, cx).last() {
        tab.canvas.selected = Some(Selected {
            page,
            id: placed.id.clone(),
        });
    }
    // As after a picture: the stamp is ready to be moved or resized.
    tool_state(cx).tool = Tool::Select;
    cx.refresh_windows();
}

/// The redaction marks of `page` in displayed points, with their index in `redactions`.
fn marks(tab: &mut DocTab, page: usize) -> Vec<(usize, DRect)> {
    let Some((geom, displayed)) = page_frame(tab, page) else {
        return Vec::new();
    };
    tab.canvas
        .redactions
        .iter()
        .enumerate()
        .filter(|(_, (on, _))| *on == page)
        .map(|(index, (_, rect))| (index, DRect::from_pdf(&geom, displayed, rect)))
        .collect()
}

/// Drop the redaction mark under `point`; true when there was one.
fn unmark_at(tab: &mut DocTab, page: usize, point: (f32, f32)) -> bool {
    let marks = marks(tab, page);
    let rects: Vec<DRect> = marks.iter().map(|(_, rect)| *rect).collect();
    let Some(hit) = geom::hit_test(&rects, point, HIT_SLOP / tab.view.zoom) else {
        return false;
    };
    tab.canvas.redactions.remove(marks[hit].0);
    true
}

/// Black out every marked area for good (core `redact`), as one undo step.
pub(super) fn apply_redactions(tab: &mut DocTab, cx: &mut Context<DocTab>) {
    let mut pages: Vec<usize> = tab
        .canvas
        .redactions
        .iter()
        .map(|(page, _)| *page)
        .collect();
    pages.sort_unstable();
    pages.dedup();
    if pages.is_empty() {
        return;
    }
    let marks = tab.canvas.redactions.clone();
    let result = tab.apply("Redact", |pdf| {
        let mut bytes = pdf.to_vec();
        for page in pages {
            let areas: Vec<Rect> = marks
                .iter()
                .filter(|(on, _)| *on == page)
                .map(|(_, rect)| *rect)
                .collect();
            bytes = godpdf_core::redact(&bytes, page, &areas)?;
        }
        Ok(bytes)
    });
    match result {
        Ok(()) => {
            tab.canvas.redactions.clear();
            tab.canvas.selected = None;
            toasts::success(cx, format!("{} area(s) redacted", marks.len()));
        }
        Err(err) => toasts::error(cx, format!("Cannot redact: {err}")),
    }
}

/// Size of a picture's pixels at 96 per inch, in points.
pub(super) fn natural_size(picture: &Picture) -> (f32, f32) {
    (
        picture.width as f32 * geom::POINTS_PER_PIXEL,
        picture.height as f32 * geom::POINTS_PER_PIXEL,
    )
}

/// A PNG or JPEG file as a picture to put on a page. A JPEG is kept as it is.
fn decode_picture(bytes: &[u8]) -> Result<Picture, String> {
    if bytes.starts_with(&[0xFF, 0xD8])
        && let Ok(picture) = guarded(|| Picture::jpeg(bytes))
    {
        return Ok(picture);
    }
    let image = image::load_from_memory(bytes)
        .map_err(|err| err.to_string())?
        .into_rgba8();
    let (width, height) = image.dimensions();
    guarded(|| Picture::rgba(width, height, image.as_raw())).map_err(|err| err.to_string())
}

/// Remove the annotation under `point`; true when one was. `amend` folds it into the last step.
fn erase_at(
    tab: &mut DocTab,
    page: usize,
    point: (f32, f32),
    amend: bool,
    cx: &mut Context<DocTab>,
) -> bool {
    if unmark_at(tab, page, point) {
        return false;
    }
    let Some(items) = items(tab, page, cx) else {
        return false;
    };
    let rects: Vec<DRect> = items.iter().map(|item| item.rect).collect();
    let Some(index) = geom::hit_test(&rects, point, HIT_SLOP / tab.view.zoom) else {
        return false;
    };
    let id = &items[index].id;
    let edit = |pdf: &[u8]| godpdf_core::remove_annotation(pdf, page, id);
    let result = if amend {
        tab.amend("Erase", edit)
    } else {
        tab.apply("Erase", edit)
    };
    let erased = result.is_ok();
    report(result, cx);
    erased
}

/// An absolutely placed box over `rect` (displayed points) at `zoom`, `grow` pixels larger.
pub(super) fn boxed(rect: DRect, zoom: f32, grow: f32) -> Div {
    div()
        .absolute()
        .left(px(rect.x0 * zoom - grow))
        .top(px(rect.y0 * zoom - grow))
        .w(px(rect.width() * zoom + 2.0 * grow))
        .h(px(rect.height() * zoom + 2.0 * grow))
}

fn with_alpha(color: Hsla, alpha: f32) -> Hsla {
    Hsla { a: alpha, ..color }
}

fn cursor(tool: Tool, gesture: Option<&Gesture>) -> CursorStyle {
    match (tool, gesture) {
        (_, Some(Gesture::Pan { .. })) => CursorStyle::ClosedHand,
        (Tool::Hand, _) => CursorStyle::OpenHand,
        (Tool::Text, _) => CursorStyle::IBeam,
        (Tool::Select, _) => CursorStyle::Arrow,
        _ => CursorStyle::Crosshair,
    }
}

fn corner_cursor(corner: usize, text: bool) -> CursorStyle {
    if text {
        CursorStyle::ResizeLeftRight
    } else if corner.is_multiple_of(2) {
        CursorStyle::ResizeUpLeftDownRight
    } else {
        CursorStyle::ResizeUpRightDownLeft
    }
}

pub fn page_overlay(
    tab: &mut DocTab,
    page: usize,
    _rect: PageRect,
    window: &mut Window,
    cx: &mut Context<DocTab>,
) -> Option<AnyElement> {
    let tool = current_tool(cx);
    let zoom = tab.view.zoom;
    let items = items(tab, page, cx)?;
    // Forget a selection whose annotation is gone (undo, page edits).
    if tab
        .canvas
        .selected
        .as_ref()
        .is_some_and(|sel| sel.page == page && !items.iter().any(|item| item.id == sel.id))
    {
        tab.canvas.selected = None;
    }
    let redaction_marks = marks(tab, page);
    let gesture = tab.canvas.gesture.as_ref();
    let mut layer = div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .cursor(cursor(tool, gesture))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |tab, event, window, cx| on_down(tab, page, event, window, cx)),
        )
        .on_mouse_move(cx.listener(move |tab, event, _, cx| on_move(tab, page, event, cx)))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|tab, _, _, cx| finish(tab, cx)),
        )
        .on_mouse_up_out(
            MouseButton::Left,
            cx.listener(|tab, _, _, cx| finish(tab, cx)),
        );

    let red: Hsla = rgb(REDACT_RED).into();
    for (_, mark) in redaction_marks {
        layer = layer.child(
            boxed(mark, zoom, 0.0)
                .bg(with_alpha(red, 0.12))
                .border_2()
                .border_color(red),
        );
    }

    let hover_grey = with_alpha(black(), 0.45);
    // Hover feedback and cursors come from boxes over each annotation.
    if gesture.is_none() && matches!(tool, Tool::Select | Tool::Eraser) {
        let selected_id = tab
            .canvas
            .selected
            .as_ref()
            .filter(|sel| sel.page == page)
            .map(|sel| sel.id.clone());
        for item in &items {
            if Some(&item.id) == selected_id.as_ref() {
                continue;
            }
            let hover = boxed(item.rect, zoom, 1.0);
            layer = layer.child(if tool == Tool::Eraser {
                hover.hover(|style| style.border_1().border_dashed().border_color(black()))
            } else {
                hover
                    .cursor(if item.movable() {
                        CursorStyle::OpenHand
                    } else {
                        CursorStyle::PointingHand
                    })
                    .hover(move |style| style.border_1().border_color(hover_grey))
            });
        }
    }

    // The selection, where the gesture in progress puts it.
    let selected = tab
        .canvas
        .selected
        .as_ref()
        .filter(|sel| sel.page == page)
        .and_then(|sel| items.iter().find(|item| item.id == sel.id));
    if let Some(item) = selected {
        let rect = match gesture {
            Some(Gesture::Move {
                id,
                original,
                offset,
                ..
            }) if *id == item.id => original.translate(*offset),
            Some(Gesture::Resize { id, rect, .. }) if *id == item.id => *rect,
            _ => item.rect,
        };
        layer = layer.child(
            boxed(rect, zoom, 2.0)
                .border_1()
                .border_dashed()
                .border_color(black())
                .when(gesture.is_none() && item.movable(), |outline| {
                    outline.cursor(CursorStyle::ClosedHand)
                }),
        );
        if item.resizable() {
            let text = item.is_text();
            for (corner, (x, y)) in rect.corners().into_iter().enumerate() {
                let half = HANDLE_SIZE / 2.0;
                layer = layer.child(
                    div()
                        .absolute()
                        .left(px(x * zoom - half))
                        .top(px(y * zoom - half))
                        .size(px(HANDLE_SIZE))
                        .bg(white())
                        .border_1()
                        .border_color(black())
                        .cursor(corner_cursor(corner, text)),
                );
            }
        }
    }

    if let Some(Gesture::Draw {
        page: drawn,
        points,
    }) = gesture
        && *drawn == page
    {
        layer = layer.children(draw_preview(tool, points, zoom, cx));
    }
    // Over the comments: a click on a form field fills it in.
    if tool == Tool::Select && tab.canvas.editor.is_none() {
        layer = layer.children(fields::render(tab, page, zoom, cx));
    }
    layer = layer.children(fields::editor(tab, page, zoom, cx));
    if tab
        .canvas
        .editor
        .as_ref()
        .is_some_and(|editor| editor.page == page)
    {
        layer = layer.children(editor::render(tab, zoom, window, cx));
    }
    Some(layer.into_any_element())
}

/// What the drag in progress will draw.
fn draw_preview(tool: Tool, points: &[(f32, f32)], zoom: f32, cx: &App) -> Option<AnyElement> {
    let style = cx.try_global::<super::ToolState>()?.styles.of(tool);
    let color = hsla(style.color);
    let (&first, &last) = (points.first()?, points.last()?);
    let rect = DRect::span(first, last);
    let element = match tool {
        Tool::Highlight => boxed(rect, zoom, 0.0)
            .bg(with_alpha(color, PREVIEW_OPACITY))
            .into_any_element(),
        // A bar along the bottom or through the middle of the dragged box.
        Tool::Underline | Tool::StrikeOut => {
            let bar = (rect.height() * 0.08).max(1.0);
            let y = if tool == Tool::Underline {
                rect.y1 - bar
            } else {
                (rect.y0 + rect.y1 - bar) / 2.0
            };
            boxed(
                DRect {
                    y0: y,
                    y1: y + bar,
                    ..rect
                },
                zoom,
                0.0,
            )
            .bg(color)
            .into_any_element()
        }
        Tool::Rectangle => boxed(rect, zoom, 0.0)
            .border(px(style.width * zoom))
            .border_color(color)
            .into_any_element(),
        Tool::Redact => boxed(rect, zoom, 0.0)
            .border_2()
            .border_color(rgb(REDACT_RED))
            .into_any_element(),
        Tool::Picture | Tool::Field => boxed(rect, zoom, 0.0)
            .border_1()
            .border_dashed()
            .border_color(black())
            .into_any_element(),
        Tool::Ellipse => strokes(
            vec![geom::ellipse_points(rect, 64)],
            style.width,
            color,
            zoom,
        ),
        Tool::Line => strokes(vec![vec![first, last]], style.width, color, zoom),
        Tool::Arrow => {
            let mut lines = vec![vec![first, last]];
            if let Some([a, b]) = geom::arrow_wings(first, last, (style.width * 4.0).max(8.0)) {
                lines.push(vec![a, last, b]);
            }
            strokes(lines, style.width, color, zoom)
        }
        Tool::Ink => strokes(vec![points.to_vec()], style.width, color, zoom),
        _ => return None,
    };
    Some(element)
}

/// Polylines (displayed points) stroked `width` points wide, over the whole page.
fn strokes(lines: Vec<Vec<(f32, f32)>>, width: f32, color: Hsla, zoom: f32) -> AnyElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let mut path = PathBuilder::stroke(px(width * zoom));
            for line in &lines {
                for (index, &(x, y)) in line.iter().enumerate() {
                    let at = bounds.origin + point(px(x * zoom), px(y * zoom));
                    if index == 0 {
                        path.move_to(at);
                    } else {
                        path.line_to(at);
                    }
                }
            }
            // A single point makes no path: nothing to show yet.
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
    .into_any_element()
}
