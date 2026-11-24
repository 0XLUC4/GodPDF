//! Paths built click by click: polygons, polylines, clouds and measurements (distance,
//! perimeter, area). A click adds a point, a double click or Enter finishes, Backspace takes
//! the last point back, Esc drops the path. A distance is done at its second point.

use godpdf_core::{Annotation, Font, MeasureKind, PageGeometry};
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Pixels, Point, div, px, white};

use super::canvas::{self, boxed, page_frame, page_point};
use super::geom::DRect;
use super::{Tool, current_tool, editor, hsla, tool_state};
use crate::doc::DocTab;

/// The path being built, in displayed page points.
pub(super) struct Path {
    page: usize,
    vertices: Vec<(f32, f32)>,
    /// Where the next point would go: the pointer, while it is over the page.
    pointer: Option<(f32, f32)>,
}

/// Points closer than this (displayed points) to the last one are the same point.
const SAME_POINT: f32 = 2.0;

pub(super) fn is_path_tool(tool: Tool) -> bool {
    matches!(
        tool,
        Tool::Polygon
            | Tool::PolyLine
            | Tool::Cloud
            | Tool::Distance
            | Tool::Perimeter
            | Tool::Area
    )
}

/// Fewest points the path of `tool` needs.
fn least_points(tool: Tool) -> usize {
    match tool {
        Tool::Polygon | Tool::Cloud | Tool::Area => 3,
        _ => 2,
    }
}

/// A click of a path tool on `page`: add `point`, or finish on a double click.
pub(super) fn click(
    tab: &mut DocTab,
    page: usize,
    point: (f32, f32),
    clicks: usize,
    cx: &mut Context<DocTab>,
) {
    if clicks >= 2 {
        finish(tab, cx);
        return;
    }
    // A click on another page starts over there.
    let path = match &mut tab.canvas.path {
        Some(path) if path.page == page => path,
        slot => slot.insert(Path {
            page,
            vertices: Vec::new(),
            pointer: None,
        }),
    };
    let near = |last: &(f32, f32)| (last.0 - point.0).hypot(last.1 - point.1) < SAME_POINT;
    if !path.vertices.last().is_some_and(near) {
        path.vertices.push(point);
    }
    if current_tool(cx) == Tool::Distance && path.vertices.len() == 2 {
        finish(tab, cx);
    }
}

/// Follow the pointer over `page` to show the next segment; true when something changed.
pub(super) fn track(tab: &mut DocTab, page: usize, position: Point<Pixels>) -> bool {
    let point = page_point(tab, page, position);
    match &mut tab.canvas.path {
        Some(path) if path.page == page && path.pointer != point => {
            path.pointer = point;
            true
        }
        _ => false,
    }
}

/// Backspace: take the last point back (the whole path with it when it was the only one).
/// False when no path is being built.
pub(super) fn remove_last(tab: &mut DocTab) -> bool {
    let Some(path) = &mut tab.canvas.path else {
        return false;
    };
    path.vertices.pop();
    if path.vertices.is_empty() {
        tab.canvas.path = None;
    }
    true
}

/// Write the path as the annotation of the current tool (too few points: dropped).
pub(super) fn finish(tab: &mut DocTab, cx: &mut Context<DocTab>) {
    let Some(path) = tab.canvas.path.take() else {
        return;
    };
    let tool = current_tool(cx);
    if path.vertices.len() < least_points(tool) {
        return;
    }
    let Some((geom, displayed)) = page_frame(tab, path.page) else {
        return;
    };
    let Some(annotation) = annotation(tab, tool, &path.vertices, &geom, displayed, cx) else {
        return;
    };
    let label = annotation.kind_name();
    let page = path.page;
    let result = tab.apply(label, |pdf| {
        godpdf_core::add_annotation(pdf, page, &annotation)
    });
    canvas::report(result, cx);
    cx.notify();
}

/// What `tool` makes of `vertices` (displayed points), in the style of new comments.
fn annotation(
    tab: &DocTab,
    tool: Tool,
    vertices: &[(f32, f32)],
    geom: &PageGeometry,
    displayed: (f32, f32),
    cx: &App,
) -> Option<Annotation> {
    let style = cx.try_global::<super::ToolState>()?.styles.of(tool);
    let vertices: Vec<(f32, f32)> = vertices
        .iter()
        .map(|&point| crate::geometry::to_pdf(geom, displayed, point))
        .collect();
    let (color, width) = (style.color, style.width);
    let measure = |kind| Annotation::Measure {
        kind,
        vertices: vertices.clone(),
        color,
        width,
        scale: tab.canvas.scale,
    };
    let annotation = match tool {
        Tool::Polygon | Tool::Cloud => Annotation::Polygon {
            vertices: vertices.clone(),
            color,
            width,
            fill: style.fill,
            cloud: tool == Tool::Cloud,
        },
        Tool::PolyLine => Annotation::PolyLine {
            vertices: vertices.clone(),
            color,
            width,
        },
        Tool::Distance => measure(MeasureKind::Distance),
        Tool::Perimeter => measure(MeasureKind::Perimeter),
        Tool::Area => measure(MeasureKind::Area),
        _ => return None,
    };
    Some(annotation)
}

/// The path being built on `page`, up to the pointer, as it will be written.
pub(super) fn preview(
    tab: &mut DocTab,
    page: usize,
    zoom: f32,
    cx: &mut Context<DocTab>,
) -> Option<AnyElement> {
    let tool = current_tool(cx);
    let path = tab.canvas.path.as_ref().filter(|path| path.page == page)?;
    let mut vertices = path.vertices.clone();
    vertices.extend(path.pointer);
    if !is_path_tool(tool) || vertices.len() < 2 {
        return None;
    }
    let (geom, displayed) = page_frame(tab, page)?;
    let shown = annotation(tab, tool, &vertices, &geom, displayed, cx)?;
    element(&shown, &geom, displayed, zoom)
}

/// `annotation` (a polygon, polyline or measurement) drawn over its page as core writes it:
/// for the path being built and while it is moved or resized.
pub(super) fn element(
    annotation: &Annotation,
    geom: &PageGeometry,
    displayed: (f32, f32),
    zoom: f32,
) -> Option<AnyElement> {
    let shown = |point: (f32, f32)| crate::geometry::from_pdf(geom, displayed, point);
    let (outline, closed, color, width, fill) = match annotation {
        Annotation::Polygon {
            vertices,
            color,
            width,
            fill,
            cloud,
        } => {
            let outline = if *cloud && vertices.len() >= 3 {
                godpdf_core::cloud_outline(vertices, *width)
            } else {
                vertices.clone()
            };
            (outline, true, *color, *width, *fill)
        }
        Annotation::PolyLine {
            vertices,
            color,
            width,
        } => (vertices.clone(), false, *color, *width, None),
        Annotation::Measure {
            kind,
            vertices,
            color,
            width,
            ..
        } => (
            vertices.clone(),
            *kind == MeasureKind::Area,
            *color,
            *width,
            None,
        ),
        _ => return None,
    };
    let mut line: Vec<(f32, f32)> = outline.iter().map(|&point| shown(point)).collect();
    if closed && let Some(&first) = line.first() {
        line.push(first);
    }
    let stroke = canvas::strokes(vec![line], width, hsla(color), fill.map(hsla), zoom);
    let label = match annotation {
        Annotation::Measure {
            kind,
            vertices,
            width,
            scale,
            ..
        } => {
            let text = scale.label(*kind, vertices);
            let (backdrop, _) = godpdf_core::label_layout(*kind, vertices, &text, *width);
            let rect = DRect::from_pdf(geom, displayed, &backdrop);
            // The backdrop is 1.2 font sizes high.
            let size = (backdrop.y1 - backdrop.y0) / 1.2;
            Some(
                boxed(rect, zoom, 0.0)
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(white())
                    .text_color(hsla(color))
                    .font_family(editor::font_family(Font::Helvetica))
                    .text_size(px(size * zoom))
                    .whitespace_nowrap()
                    .child(text),
            )
        }
        _ => None,
    };
    Some(
        div()
            .absolute()
            .inset_0()
            .child(stroke)
            .children(label)
            .into_any_element(),
    )
}

/// Enter: finish the path of the document shown.
pub(super) fn finish_active(cx: &mut App) {
    if is_path_tool(tool_state(cx).tool) {
        super::with_active(cx, finish);
    }
}
