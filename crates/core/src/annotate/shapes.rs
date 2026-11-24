//! Polygons, polylines, clouds and measurements (distance, perimeter, area).
//!
//! Written as standard /Polygon, /PolyLine and /Line annotations; measurements carry their
//! intent (/IT /LineDimension, /PolyLineDimension, /PolygonDimension), their value in
//! /Contents and their scale in a /Measure dictionary, and show the value in the appearance.

use std::fmt;

use lopdf::{Dictionary, Document, Object, decode_text_string, dictionary, text_string};

use super::{
    Annotation, Color, Parts, Rect, border_style, check_width, color_array, contents_string, num,
    numbers, read_color, read_rgb, read_width, rgb, standard_font, win_ansi_hex,
};
use crate::fonts::Font;
use crate::pdf::{deref, dict_entry};
use crate::{Error, Result};

/// What a measurement measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasureKind {
    /// Straight distance between two points.
    Distance,
    /// Length of an open path.
    Perimeter,
    /// Area inside a closed path.
    Area,
}

impl MeasureKind {
    /// Intent (/IT) of the annotation.
    fn intent(self) -> &'static str {
        match self {
            Self::Distance => "LineDimension",
            Self::Perimeter => "PolyLineDimension",
            Self::Area => "PolygonDimension",
        }
    }

    fn from_intent(intent: &[u8]) -> Option<Self> {
        [Self::Distance, Self::Perimeter, Self::Area]
            .into_iter()
            .find(|kind| kind.intent().as_bytes() == intent)
    }
}

/// Length units of a [`Scale`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Mm,
    Cm,
    M,
    In,
    Ft,
}

impl Unit {
    pub const ALL: [Self; 5] = [Self::Mm, Self::Cm, Self::M, Self::In, Self::Ft];

    pub fn symbol(self) -> &'static str {
        match self {
            Self::Mm => "mm",
            Self::Cm => "cm",
            Self::M => "m",
            Self::In => "in",
            Self::Ft => "ft",
        }
    }

    /// PDF points in one unit.
    fn points(self) -> f32 {
        match self {
            Self::Mm => 72.0 / 25.4,
            Self::Cm => 720.0 / 25.4,
            Self::M => 72_000.0 / 25.4,
            Self::In => 72.0,
            Self::Ft => 864.0,
        }
    }

    fn from_symbol(symbol: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|unit| unit.symbol().eq_ignore_ascii_case(symbol))
    }
}

/// Drawing scale: `paper` `paper_unit` on the page stand for `real` `real_unit`.
/// The default, 1 in = 1 in, measures the page itself (1 pt = 1/72 in).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scale {
    pub paper: f32,
    pub paper_unit: Unit,
    pub real: f32,
    pub real_unit: Unit,
}

impl Default for Scale {
    fn default() -> Self {
        Self {
            paper: 1.0,
            paper_unit: Unit::In,
            real: 1.0,
            real_unit: Unit::In,
        }
    }
}

impl fmt::Display for Scale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} = {} {}",
            self.paper,
            self.paper_unit.symbol(),
            self.real,
            self.real_unit.symbol()
        )
    }
}

impl Scale {
    /// Read "1 cm = 2 m" (spaces optional, decimal point or comma; mm, cm, m, in, ft).
    pub fn parse(text: &str) -> Option<Self> {
        let side = |side: &str| -> Option<(f32, Unit)> {
            let side = side.trim();
            let split = side.find(|c: char| c.is_alphabetic())?;
            let value: f32 = side[..split].trim().replace(',', ".").parse().ok()?;
            let unit = Unit::from_symbol(side[split..].trim())?;
            (value.is_finite() && value > 0.0).then_some((value, unit))
        };
        let (paper, real) = text.split_once('=')?;
        let ((paper, paper_unit), (real, real_unit)) = (side(paper)?, side(real)?);
        Some(Self {
            paper,
            paper_unit,
            real,
            real_unit,
        })
    }

    /// Real units (`real_unit`) per PDF point.
    pub fn per_point(self) -> f32 {
        self.real / (self.paper * self.paper_unit.points())
    }

    /// What `kind` measures over `vertices` (PDF points), in `real_unit` (squared for an area).
    pub fn value(self, kind: MeasureKind, vertices: &[(f32, f32)]) -> f32 {
        let per_point = self.per_point();
        match kind {
            MeasureKind::Distance | MeasureKind::Perimeter => {
                path_length(vertices, false) * per_point
            }
            MeasureKind::Area => polygon_area(vertices) * per_point * per_point,
        }
    }

    /// The value with its unit, as the label and /Contents show it ("12.50 m", "3.20 m²").
    pub fn label(self, kind: MeasureKind, vertices: &[(f32, f32)]) -> String {
        let unit = self.real_unit.symbol();
        let value = self.value(kind, vertices);
        match kind {
            MeasureKind::Area => format!("{value:.2} {unit}²"),
            _ => format!("{value:.2} {unit}"),
        }
    }
}

/// Length of the path through `points`, back to the first one when `closed`.
pub fn path_length(points: &[(f32, f32)], closed: bool) -> f32 {
    let open: f32 = points
        .windows(2)
        .map(|pair| (pair[1].0 - pair[0].0).hypot(pair[1].1 - pair[0].1))
        .sum();
    match (closed, points.first(), points.last()) {
        (true, Some(first), Some(last)) if points.len() > 2 => {
            open + (first.0 - last.0).hypot(first.1 - last.1)
        }
        _ => open,
    }
}

/// Area inside the polygon `points` (shoelace formula), whatever its orientation.
pub fn polygon_area(points: &[(f32, f32)]) -> f32 {
    signed_area(points).abs()
}

/// Positive for counter-clockwise points in a y-up space.
fn signed_area(points: &[(f32, f32)]) -> f32 {
    let Some(&first) = points.first() else {
        return 0.0;
    };
    // Measured from the first point: fewer rounding errors far from the origin.
    let twice: f64 = points
        .windows(2)
        .map(|pair| {
            let (ax, ay) = (pair[0].0 - first.0, pair[0].1 - first.1);
            let (bx, by) = (pair[1].0 - first.0, pair[1].1 - first.1);
            f64::from(ax) * f64::from(by) - f64::from(bx) * f64::from(ay)
        })
        .sum();
    (twice / 2.0) as f32
}

/// One scallop of a cloud: a cubic Bezier from `[0]` to `[3]` through controls `[1]`, `[2]`.
type Arc = [(f32, f32); 4];

/// Radius of the scallops of a cloud drawn `width` wide.
fn scallop_radius(width: f32) -> f32 {
    5.0 + width
}

/// The scallops of a cloud around `vertices`: each edge split into half circles bulging
/// outwards. Works in any space, y-up or y-down.
fn scallops(vertices: &[(f32, f32)], width: f32) -> Vec<Arc> {
    let outward = if signed_area(vertices) >= 0.0 {
        1.0
    } else {
        -1.0
    };
    let diameter = 2.0 * scallop_radius(width);
    let mut arcs = Vec::new();
    for (index, &start) in vertices.iter().enumerate() {
        let end = vertices[(index + 1) % vertices.len()];
        let (dx, dy) = (end.0 - start.0, end.1 - start.1);
        let length = dx.hypot(dy);
        if length < f32::EPSILON {
            continue;
        }
        let count = (length / diameter).round().max(1.0);
        let (step_x, step_y) = (dx / count, dy / count);
        // Half circle as one cubic: controls 4/3 of the radius out.
        let bulge = 4.0 / 3.0 * (length / count) / 2.0;
        let (nx, ny) = (
            dy / length * outward * bulge,
            -dx / length * outward * bulge,
        );
        for step in 0..count as usize {
            let a = (
                start.0 + step_x * step as f32,
                start.1 + step_y * step as f32,
            );
            let b = (a.0 + step_x, a.1 + step_y);
            arcs.push([a, (a.0 + nx, a.1 + ny), (b.0 + nx, b.1 + ny), b]);
        }
    }
    arcs
}

/// The outline of a cloud around `vertices` (`width` its stroke), as points to draw a
/// closed polyline through: what the appearance of a cloud shows.
pub fn cloud_outline(vertices: &[(f32, f32)], width: f32) -> Vec<(f32, f32)> {
    const SAMPLES: usize = 8;
    let mut points = Vec::new();
    for [a, b, c, d] in scallops(vertices, width) {
        for sample in 0..SAMPLES {
            let t = sample as f32 / SAMPLES as f32;
            let u = 1.0 - t;
            let mix = |p: f32, q: f32, r: f32, s: f32| {
                u * u * u * p + 3.0 * u * u * t * q + 3.0 * u * t * t * r + t * t * t * s
            };
            points.push((mix(a.0, b.0, c.0, d.0), mix(a.1, b.1, c.1, d.1)));
        }
    }
    points
}

/// Checks shared by every path: enough finite vertices, a valid width.
fn check_path(vertices: &[(f32, f32)], least: usize, width: f32) -> Result<()> {
    check_width(width)?;
    if vertices.len() < least {
        return Err(Error::InvalidArgument(format!(
            "a path needs at least {least} points"
        )));
    }
    if vertices
        .iter()
        .any(|&(x, y)| !(x.is_finite() && y.is_finite()))
    {
        return Err(Error::InvalidArgument("path point is not finite".into()));
    }
    Ok(())
}

/// The box around `points`, grown by `margin`.
fn bounds(points: &[(f32, f32)], margin: f32) -> Rect {
    let fold = |pick: fn(&(f32, f32)) -> f32, start: f32, f: fn(f32, f32) -> f32| {
        points.iter().map(pick).fold(start, f)
    };
    Rect {
        x0: fold(|p| p.0, f32::INFINITY, f32::min) - margin,
        y0: fold(|p| p.1, f32::INFINITY, f32::min) - margin,
        x1: fold(|p| p.0, f32::NEG_INFINITY, f32::max) + margin,
        y1: fold(|p| p.1, f32::NEG_INFINITY, f32::max) + margin,
    }
}

fn union(a: Rect, b: Rect) -> Rect {
    Rect {
        x0: a.x0.min(b.x0),
        y0: a.y0.min(b.y0),
        x1: a.x1.max(b.x1),
        y1: a.y1.max(b.y1),
    }
}

fn point((x, y): (f32, f32)) -> String {
    format!("{} {}", num(x), num(y))
}

/// `m` / `l` operators through `vertices`, closed with `h` when `closed`.
fn path_ops(vertices: &[(f32, f32)], closed: bool) -> String {
    let mut ops: Vec<String> = vertices
        .iter()
        .enumerate()
        .map(|(index, &at)| format!("{} {}", point(at), if index == 0 { "m" } else { "l" }))
        .collect();
    if closed {
        ops.push("h".into());
    }
    ops.join(" ")
}

/// The scalloped path of a cloud.
fn cloud_ops(vertices: &[(f32, f32)], width: f32) -> String {
    let arcs = scallops(vertices, width);
    let Some(first) = arcs.first() else {
        return path_ops(vertices, true);
    };
    let mut ops = format!("{} m", point(first[0]));
    for [_, b, c, d] in &arcs {
        ops += &format!(" {} {} {} c", point(*b), point(*c), point(*d));
    }
    ops + " h"
}

fn vertices_array(vertices: &[(f32, f32)]) -> Vec<Object> {
    vertices
        .iter()
        .flat_map(|&(x, y)| [x.into(), y.into()])
        .collect()
}

/// A polygon (`closed`) or a polyline through `vertices`: stroked, the polygon painted
/// inside with `fill` first, its edges drawn as a cloud when `cloud`.
pub(super) fn poly(
    vertices: &[(f32, f32)],
    color: Color,
    width: f32,
    fill: Option<Color>,
    closed: bool,
    cloud: bool,
) -> Result<Parts> {
    check_path(vertices, if closed { 3 } else { 2 }, width)?;
    let mut margin = width / 2.0 + 1.0;
    if cloud {
        margin += scallop_radius(width);
    }
    let path = if cloud {
        cloud_ops(vertices, width)
    } else {
        path_ops(vertices, closed)
    };
    let mut extra = dictionary! {
        "Vertices" => vertices_array(vertices),
        "BS" => border_style(width),
    };
    let mut appearance = format!("{} w 1 J 1 j {} RG", num(width), rgb(color));
    let paint = match fill.filter(|_| closed) {
        Some(fill) => {
            extra.set("IC", color_array(fill));
            appearance += &format!(" {} rg", rgb(fill));
            if width > 0.0 { "B" } else { "f" }
        }
        None => "S",
    };
    appearance += &format!(" {path} {paint}");
    if cloud {
        extra.set("BE", dictionary! { "S" => "C", "I" => 1i64 });
    }
    Ok(Parts {
        subtype: if closed { "Polygon" } else { "PolyLine" },
        rect: bounds(vertices, margin),
        color: Some(color),
        extra,
        appearance,
        resources: Dictionary::new(),
    })
}

/// Size of the label of a measurement, in points.
const LABEL_SIZE: f32 = 9.0;
/// Space between the label's text and its white backdrop.
const LABEL_PAD: f32 = 2.0;

/// A measurement of `vertices` with `scale`: the line or path, and its value in a label.
pub(super) fn measure(
    kind: MeasureKind,
    vertices: &[(f32, f32)],
    color: Color,
    width: f32,
    scale: Scale,
) -> Result<Parts> {
    let (least, closed) = match kind {
        MeasureKind::Distance => (2, false),
        MeasureKind::Perimeter => (2, false),
        MeasureKind::Area => (3, true),
    };
    check_path(vertices, least, width)?;
    if kind == MeasureKind::Distance && vertices.len() != 2 {
        return Err(Error::InvalidArgument(
            "a distance has exactly two points".into(),
        ));
    }
    let label = scale.label(kind, vertices);
    let mut appearance = format!(
        "{} w 1 J 1 j {} RG {} S",
        num(width),
        rgb(color),
        path_ops(vertices, closed)
    );
    let mut extra = dictionary! {
        "IT" => kind.intent(),
        "Contents" => contents_string(&label),
        "Measure" => measure_dict(scale),
        "BS" => border_style(width),
    };
    let tick = 4.0 + width;
    if kind == MeasureKind::Distance {
        // Short bars across both ends, as on a drawing's dimension line.
        let (start, end) = (vertices[0], vertices[1]);
        for [a, b] in end_ticks(start, end, tick) {
            appearance += &format!(" {} m {} l S", point(a), point(b));
        }
        extra.set("L", vertices_array(vertices));
        extra.set("LE", vec![Object::from("Butt"), Object::from("Butt")]);
        extra.set("Cap", true);
    } else {
        extra.set("Vertices", vertices_array(vertices));
    }
    let (backdrop, text) = label_layout(kind, vertices, &label, width);
    let font = Font::Helvetica.resource(false, false);
    appearance += &format!(
        " 1 1 1 rg {} {} {} {} re f BT /{font} {} Tf {} rg {} Td <{}> Tj ET",
        num(backdrop.x0),
        num(backdrop.y0),
        num(backdrop.x1 - backdrop.x0),
        num(backdrop.y1 - backdrop.y0),
        num(LABEL_SIZE),
        rgb(color),
        point(text),
        win_ansi_hex(&label),
    );
    let margin = width / 2.0
        + 1.0
        + if kind == MeasureKind::Distance {
            tick
        } else {
            0.0
        };
    Ok(Parts {
        subtype: match kind {
            MeasureKind::Distance => "Line",
            MeasureKind::Perimeter => "PolyLine",
            MeasureKind::Area => "Polygon",
        },
        rect: union(bounds(vertices, margin), backdrop),
        color: Some(color),
        extra,
        appearance,
        resources: dictionary! {
            "Font" => dictionary! { font => standard_font(Font::Helvetica.base_font(false, false)) },
        },
    })
}

/// The bars across both ends of a distance from `start` to `end`, `half` long each way.
fn end_ticks(start: (f32, f32), end: (f32, f32), half: f32) -> Vec<[(f32, f32); 2]> {
    let (dx, dy) = (end.0 - start.0, end.1 - start.1);
    let length = dx.hypot(dy);
    if length < f32::EPSILON {
        return Vec::new();
    }
    let (nx, ny) = (-dy / length * half, dx / length * half);
    [start, end]
        .into_iter()
        .map(|at| [(at.0 - nx, at.1 - ny), (at.0 + nx, at.1 + ny)])
        .collect()
}

/// Where the label of a measurement goes (PDF space): its white backdrop and the start of
/// its baseline. A distance is labelled above its middle, a perimeter above the middle of
/// its last segment, an area in the middle of its points.
pub fn label_layout(
    kind: MeasureKind,
    vertices: &[(f32, f32)],
    label: &str,
    width: f32,
) -> (Rect, (f32, f32)) {
    let text_width = crate::helvetica::line_width(label, LABEL_SIZE);
    let (box_width, box_height) = (text_width + 2.0 * LABEL_PAD, LABEL_SIZE * 1.2);
    let middle = |a: (f32, f32), b: (f32, f32)| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    let (x, y0) = match (kind, vertices) {
        (MeasureKind::Area, _) | (_, [] | [_]) => {
            let count = vertices.len().max(1) as f32;
            let (sx, sy) = vertices
                .iter()
                .fold((0.0, 0.0), |sum, p| (sum.0 + p.0, sum.1 + p.1));
            (sx / count, sy / count - box_height / 2.0)
        }
        (_, [.., a, b]) => {
            let (x, y) = middle(*a, *b);
            (x, y + width / 2.0 + 2.0)
        }
    };
    let backdrop = Rect {
        x0: x - box_width / 2.0,
        y0,
        x1: x + box_width / 2.0,
        y1: y0 + box_height,
    };
    // Helvetica's descender is about 0.21 em: the baseline sits that far over the bottom.
    let baseline = (
        backdrop.x0 + LABEL_PAD,
        y0 + 0.1 * LABEL_SIZE + 0.21 * LABEL_SIZE,
    );
    (backdrop, baseline)
}

/// Rectilinear /Measure dictionary of `scale`, so other readers measure the same way.
fn measure_dict(scale: Scale) -> Dictionary {
    let unit = scale.real_unit.symbol();
    let format = |unit: String, factor: f32| -> Object {
        dictionary! {
            "Type" => "NumberFormat",
            "U" => text_string(&unit),
            "C" => factor,
            "D" => 100i64,
        }
        .into()
    };
    dictionary! {
        "Type" => "Measure",
        "Subtype" => "RL",
        "R" => text_string(&scale.to_string()),
        "X" => vec![format(unit.to_owned(), scale.per_point())],
        "D" => vec![format(unit.to_owned(), 1.0)],
        "A" => vec![format(format!("sq {unit}"), 1.0)],
    }
}

/// The measurement intent of `dict`, if it is one of ours.
pub(super) fn measure_kind(doc: &Document, dict: &Dictionary) -> Option<MeasureKind> {
    let intent = deref(doc, dict.get(b"IT").ok()?)?.as_name().ok()?;
    MeasureKind::from_intent(intent)
}

/// The scale written in /Measure /R, else the default one.
fn read_scale(doc: &Document, dict: &Dictionary) -> Scale {
    dict_entry(doc, dict, b"Measure")
        .and_then(|measure| deref(doc, measure.get(b"R").ok()?))
        .and_then(|ratio| decode_text_string(ratio).ok())
        .and_then(|ratio| Scale::parse(&ratio))
        .unwrap_or_default()
}

/// /Vertices (or /L for a line) as points.
fn read_vertices(doc: &Document, dict: &Dictionary, key: &[u8]) -> Option<Vec<(f32, f32)>> {
    let values = numbers(doc, dict, key)?;
    let points: Vec<(f32, f32)> = values
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&[x, y]| (x, y))
        .collect();
    (points.len() >= 2).then_some(points)
}

/// A /Polygon, /PolyLine or measuring /Line read back.
pub(super) fn parse(doc: &Document, dict: &Dictionary) -> Option<Annotation> {
    let subtype = dict.get(b"Subtype").ok()?.as_name().ok()?;
    let key: &[u8] = if subtype == b"Line" {
        b"L"
    } else {
        b"Vertices"
    };
    let vertices = read_vertices(doc, dict, key)?;
    let color = read_color(doc, dict)?;
    let width = read_width(doc, dict);
    if let Some(kind) = measure_kind(doc, dict) {
        return Some(Annotation::Measure {
            kind,
            vertices,
            color,
            width,
            scale: read_scale(doc, dict),
        });
    }
    match subtype {
        b"Polygon" => Some(Annotation::Polygon {
            vertices,
            color,
            width,
            fill: read_rgb(doc, dict, b"IC"),
            cloud: dict_entry(doc, dict, b"BE")
                .and_then(|effect| effect.get(b"S").ok()?.as_name().ok())
                .is_some_and(|style| style == b"C"),
        }),
        b"PolyLine" => Some(Annotation::PolyLine {
            vertices,
            color,
            width,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() <= 1e-3 * b.abs().max(1.0)
    }

    #[test]
    fn scales_parse_and_print() {
        let scale = Scale::parse(" 1 cm = 2,5 m ").unwrap();
        assert_eq!(
            scale,
            Scale {
                paper: 1.0,
                paper_unit: Unit::Cm,
                real: 2.5,
                real_unit: Unit::M
            }
        );
        assert_eq!(scale.to_string(), "1 cm = 2.5 m");
        assert_eq!(Scale::parse(&scale.to_string()), Some(scale));
        assert_eq!(Scale::parse("1in=10FT").unwrap().real_unit, Unit::Ft);
        for bad in [
            "",
            "1 cm",
            "1 cm = m",
            "0 cm = 1 m",
            "1 km = 1 m",
            "-1 mm = 1 m",
        ] {
            assert_eq!(Scale::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn default_scale_measures_the_page_in_inches() {
        let scale = Scale::default();
        assert!(close(scale.per_point(), 1.0 / 72.0));
        let inch = [(0.0, 0.0), (72.0, 0.0)];
        assert_eq!(scale.label(MeasureKind::Distance, &inch), "1.00 in");
        // 1 cm on paper is 2 m: 10 cm of page is 20 m.
        let scale = Scale::parse("1 cm = 2 m").unwrap();
        let ten_cm = 720.0 / 2.54;
        assert!(close(
            scale.value(MeasureKind::Distance, &[(0.0, 0.0), (0.0, ten_cm)]),
            20.0
        ));
        let square = [(0.0, 0.0), (ten_cm, 0.0), (ten_cm, ten_cm), (0.0, ten_cm)];
        assert!(close(scale.value(MeasureKind::Area, &square), 400.0));
        assert_eq!(scale.label(MeasureKind::Area, &square), "400.00 m²");
        assert!(close(scale.value(MeasureKind::Perimeter, &square), 60.0));
    }

    #[test]
    fn shoelace_area_and_length() {
        // An L shape: 3x1 + 1x1.
        let l_shape = [
            (0.0, 0.0),
            (3.0, 0.0),
            (3.0, 1.0),
            (1.0, 1.0),
            (1.0, 2.0),
            (0.0, 2.0),
        ];
        assert!(close(polygon_area(&l_shape), 4.0));
        let reversed: Vec<_> = l_shape.iter().rev().copied().collect();
        assert!(close(polygon_area(&reversed), 4.0));
        assert!(close(path_length(&l_shape, true), 10.0));
        assert!(close(path_length(&l_shape, false), 8.0));
        assert_eq!(polygon_area(&l_shape[..2]), 0.0);
        assert_eq!(path_length(&[], true), 0.0);
    }

    #[test]
    fn measures_do_not_depend_on_rotation_or_position() {
        let triangle = [(10.0, 10.0), (110.0, 30.0), (40.0, 90.0)];
        let (area, length) = (polygon_area(&triangle), path_length(&triangle, true));
        for degrees in [17.0f32, 90.0, 180.0, 233.0] {
            let (sin, cos) = degrees.to_radians().sin_cos();
            let turned: Vec<(f32, f32)> = triangle
                .iter()
                .map(|&(x, y)| (x * cos - y * sin + 500.0, x * sin + y * cos - 300.0))
                .collect();
            assert!(close(polygon_area(&turned), area), "{degrees}");
            assert!(close(path_length(&turned, true), length), "{degrees}");
        }
    }

    #[test]
    fn clouds_bulge_outwards_either_way_round() {
        let square = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)];
        let reversed: Vec<_> = square.iter().rev().copied().collect();
        for vertices in [&square[..], &reversed] {
            let outline = cloud_outline(vertices, 1.0);
            assert!(!outline.is_empty());
            // Every sample is on or outside the square, and some are well outside.
            let inside = |&(x, y): &(f32, f32)| x > 0.5 && x < 99.5 && y > 0.5 && y < 99.5;
            assert!(!outline.iter().any(inside));
            let reach = outline
                .iter()
                .map(|&(x, y)| (-x).max(-y).max(x - 100.0).max(y - 100.0))
                .fold(0.0f32, f32::max);
            assert!(reach > 3.0 && reach < 10.0, "{reach}");
        }
    }
}
