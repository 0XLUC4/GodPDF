//! Pure geometry of the canvas tools, in displayed page points (origin top-left, after
//! /Rotate).

use crate::geometry::{self, PageGeometry};

/// Drags shorter than this (displayed points) are clicks.
pub const MIN_MOVE: f32 = 0.5;
/// Smallest side of a resized annotation, in points.
pub const MIN_SIZE: f32 = 8.0;
/// Millimetres in a point.
pub const MM: f32 = 25.4 / 72.0;
/// Pixels of a picture at 96 per inch, in points.
pub const POINTS_PER_PIXEL: f32 = 0.75;

/// A rectangle in displayed page points; `x0 <= x1`, `y0 <= y1`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DRect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl DRect {
    /// The rectangle spanned by two corners, in any order.
    pub fn span(a: (f32, f32), b: (f32, f32)) -> Self {
        Self {
            x0: a.0.min(b.0),
            y0: a.1.min(b.1),
            x1: a.0.max(b.0),
            y1: a.1.max(b.1),
        }
    }

    pub fn width(&self) -> f32 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> f32 {
        self.y1 - self.y0
    }

    pub fn contains(&self, (x, y): (f32, f32), slop: f32) -> bool {
        x >= self.x0 - slop && x <= self.x1 + slop && y >= self.y0 - slop && y <= self.y1 + slop
    }

    pub fn translate(&self, (dx, dy): (f32, f32)) -> Self {
        Self {
            x0: self.x0 + dx,
            y0: self.y0 + dy,
            x1: self.x1 + dx,
            y1: self.y1 + dy,
        }
    }

    /// `amount` smaller on every side (larger when negative), never past its middle.
    pub fn inset(&self, amount: f32) -> Self {
        let (dx, dy) = (
            amount.min(self.width() / 2.0),
            amount.min(self.height() / 2.0),
        );
        Self {
            x0: self.x0 + dx,
            y0: self.y0 + dy,
            x1: self.x1 - dx,
            y1: self.y1 - dy,
        }
    }

    /// Corners: 0 top-left, 1 top-right, 2 bottom-right, 3 bottom-left.
    pub fn corners(&self) -> [(f32, f32); 4] {
        [
            (self.x0, self.y0),
            (self.x1, self.y0),
            (self.x1, self.y1),
            (self.x0, self.y1),
        ]
    }

    pub fn from_pdf(geom: &PageGeometry, displayed: (f32, f32), rect: &godpdf_core::Rect) -> Self {
        Self::span(
            geometry::from_pdf(geom, displayed, (rect.x0, rect.y0)),
            geometry::from_pdf(geom, displayed, (rect.x1, rect.y1)),
        )
    }

    pub fn to_pdf(self, geom: &PageGeometry, displayed: (f32, f32)) -> godpdf_core::Rect {
        geometry::to_pdf_rect(geom, displayed, (self.x0, self.y0), (self.x1, self.y1))
    }
}

/// Topmost (last drawn) rectangle containing `point`, allowing `slop` around each.
pub fn hit_test(rects: &[DRect], point: (f32, f32), slop: f32) -> Option<usize> {
    rects.iter().rposition(|rect| rect.contains(point, slop))
}

/// Distance from `point` to the polyline `line` (to its point when it has only one);
/// infinite for no point.
pub fn polyline_distance(line: &[(f32, f32)], point: (f32, f32)) -> f32 {
    let to = |(x, y): (f32, f32)| (point.0 - x).hypot(point.1 - y);
    let segment = |a: (f32, f32), b: (f32, f32)| {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let length = dx * dx + dy * dy;
        if length <= f32::EPSILON {
            return to(a);
        }
        let t = (((point.0 - a.0) * dx + (point.1 - a.1) * dy) / length).clamp(0.0, 1.0);
        to((a.0 + dx * t, a.1 + dy * t))
    };
    match line {
        [] => f32::INFINITY,
        [only] => to(*only),
        _ => line
            .windows(2)
            .map(|pair| segment(pair[0], pair[1]))
            .fold(f32::INFINITY, f32::min),
    }
}

/// A displayed-space offset expressed in PDF space (rotation aware).
pub fn pdf_delta(geom: &PageGeometry, displayed: (f32, f32), offset: (f32, f32)) -> (f32, f32) {
    let (x0, y0) = geometry::to_pdf(geom, displayed, (0.0, 0.0));
    let (x1, y1) = geometry::to_pdf(geom, displayed, offset);
    (x1 - x0, y1 - y0)
}

/// Limit `offset` so `item` stays on a page of size `page`.
pub fn clamp_offset(item: DRect, page: (f32, f32), offset: (f32, f32)) -> (f32, f32) {
    let range = |low: f32, high: f32, value: f32| {
        if low > high {
            0.0
        } else {
            value.clamp(low, high)
        }
    };
    (
        range(-item.x0, page.0 - item.x1, offset.0),
        range(-item.y0, page.1 - item.y1, offset.1),
    )
}

/// `original` after dragging `corner` to `pointer`: the opposite corner stays put, the result
/// stays on the page and keeps a minimum size. Text boxes only change width.
pub fn resized_rect(
    original: DRect,
    corner: usize,
    pointer: (f32, f32),
    page: (f32, f32),
    text: bool,
) -> DRect {
    let fixed = original.corners()[(corner + 2) % 4];
    let pointer = (pointer.0.clamp(0.0, page.0), pointer.1.clamp(0.0, page.1));
    // Never flip through the fixed corner, never get smaller than MIN_SIZE.
    let span = |fixed: f32, pointer: f32, grows: bool| {
        if grows {
            (fixed, pointer.max(fixed + MIN_SIZE))
        } else {
            (pointer.min(fixed - MIN_SIZE), fixed)
        }
    };
    let x = span(fixed.0, pointer.0, corner == 1 || corner == 2);
    let y = if text {
        (original.y0, original.y1)
    } else {
        span(fixed.1, pointer.1, corner == 2 || corner == 3)
    };
    DRect {
        x0: x.0,
        y0: y.0,
        x1: x.1,
        y1: y.1,
    }
}

/// `rect` given the shape of `original`, as big as the larger of its two sides asks but never
/// past the edges of a page of size `page`, the corner opposite `corner` staying put.
pub fn keep_ratio(original: DRect, rect: DRect, corner: usize, page: (f32, f32)) -> DRect {
    let fixed = original.corners()[(corner + 2) % 4];
    let (right, down) = (corner == 1 || corner == 2, corner == 2 || corner == 3);
    let room = (
        if right { page.0 - fixed.0 } else { fixed.0 },
        if down { page.1 - fixed.1 } else { fixed.1 },
    );
    let (width, height) = (original.width().max(1.0), original.height().max(1.0));
    let scale = (rect.width() / width)
        .max(rect.height() / height)
        .min(room.0 / width)
        .min(room.1 / height);
    let (w, h) = (original.width() * scale, original.height() * scale);
    let (x0, x1) = if right {
        (fixed.0, fixed.0 + w)
    } else {
        (fixed.0 - w, fixed.0)
    };
    let (y0, y1) = if down {
        (fixed.1, fixed.1 + h)
    } else {
        (fixed.1 - h, fixed.1)
    };
    DRect { x0, y0, x1, y1 }
}

/// Where a picture of `natural` size (points) goes: fitted, centered, in the dragged box
/// `from`-`to`, or else with its top-left corner at `from` at its own size, smaller when the
/// page has no room.
pub fn place_picture(
    natural: (f32, f32),
    page: (f32, f32),
    from: (f32, f32),
    to: Option<(f32, f32)>,
) -> DRect {
    let natural = (natural.0.max(1.0), natural.1.max(1.0));
    match to {
        Some(to) => {
            let frame = DRect::span(from, to);
            let scale = (frame.width() / natural.0).min(frame.height() / natural.1);
            let (w, h) = (natural.0 * scale, natural.1 * scale);
            let x0 = (frame.x0 + frame.x1 - w) / 2.0;
            let y0 = (frame.y0 + frame.y1 - h) / 2.0;
            DRect {
                x0,
                y0,
                x1: x0 + w,
                y1: y0 + h,
            }
        }
        None => {
            let room = (
                (page.0 - from.0).max(page.0 * 0.2),
                (page.1 - from.1).max(page.1 * 0.2),
            );
            let scale = (room.0 / natural.0).min(room.1 / natural.1).min(1.0);
            DRect {
                x0: from.0,
                y0: from.1,
                x1: from.0 + natural.0 * scale,
                y1: from.1 + natural.1 * scale,
            }
        }
    }
}

/// `rect`'s inscribed ellipse as a closed polyline of `steps` segments (preview drawing).
pub fn ellipse_points(rect: DRect, steps: usize) -> Vec<(f32, f32)> {
    let center = ((rect.x0 + rect.x1) / 2.0, (rect.y0 + rect.y1) / 2.0);
    let radius = (rect.width() / 2.0, rect.height() / 2.0);
    (0..=steps)
        .map(|step| {
            let angle = step as f32 / steps as f32 * std::f32::consts::TAU;
            (
                center.0 + radius.0 * angle.cos(),
                center.1 + radius.1 * angle.sin(),
            )
        })
        .collect()
}

/// Ends of the two strokes of an open arrow head at `end`, `length` long, 30° off the line;
/// `None` for a line of no length.
pub fn arrow_wings(start: (f32, f32), end: (f32, f32), length: f32) -> Option<[(f32, f32); 2]> {
    let (dx, dy) = (end.0 - start.0, end.1 - start.1);
    let norm = dx.hypot(dy);
    if norm < f32::EPSILON {
        return None;
    }
    let (ux, uy) = (dx / norm, dy / norm);
    let (sin, cos) = std::f32::consts::FRAC_PI_6.sin_cos();
    let wing = |side: f32| {
        let (rx, ry) = (ux * cos - side * uy * sin, side * ux * sin + uy * cos);
        (end.0 - rx * length, end.1 - ry * length)
    };
    Some([wing(1.0), wing(-1.0)])
}

/// A field of the picture property card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PictureField {
    Width,
    Height,
    X,
    Y,
}

impl PictureField {
    pub const ALL: [PictureField; 4] = [Self::Width, Self::Height, Self::X, Self::Y];

    pub fn label(self) -> &'static str {
        match self {
            Self::Width => "Width",
            Self::Height => "Height",
            Self::X => "X",
            Self::Y => "Y",
        }
    }

    /// The value shown for `rect`, in millimetres (X / Y from the page's top-left corner).
    pub fn value_mm(self, rect: DRect) -> f32 {
        let points = match self {
            Self::Width => rect.width(),
            Self::Height => rect.height(),
            Self::X => rect.x0,
            Self::Y => rect.y0,
        };
        points * MM
    }
}

/// `rect` with `field` set to `mm` millimetres. Width and height keep the top-left corner and,
/// with `keep`, the proportions; sizes stay between 2 mm and the page.
pub fn edit_picture(rect: DRect, field: PictureField, mm: f32, keep: bool) -> DRect {
    let points = mm / MM;
    let min = 2.0 / MM;
    let ratio = rect.height() / rect.width().max(0.01);
    let (w, h) = match field {
        PictureField::Width => {
            let w = points.max(min);
            (w, if keep { w * ratio } else { rect.height() })
        }
        PictureField::Height => {
            let h = points.max(min);
            (
                if keep {
                    h / ratio.max(0.0001)
                } else {
                    rect.width()
                },
                h,
            )
        }
        PictureField::X => return rect.translate((points - rect.x0, 0.0)),
        PictureField::Y => return rect.translate((0.0, points - rect.y0)),
    };
    DRect {
        x0: rect.x0,
        y0: rect.y0,
        x1: rect.x0 + w,
        y1: rect.y0 + h,
    }
}

/// A number typed by the user: "12.5", "12,5", "12.5 mm".
pub fn parse_mm(text: &str) -> Option<f32> {
    let text = text.trim().trim_end_matches("mm").trim().replace(',', ".");
    text.parse::<f32>().ok().filter(|value| value.is_finite())
}

/// Where a whole page of `page` points goes, relative to `dst`, so that its part `src`
/// fills `dst`: the page picture under a moved or resized annotation, cut to it.
pub fn crop(src: DRect, dst: DRect, page: (f32, f32)) -> DRect {
    let sx = dst.width() / src.width().max(0.01);
    let sy = dst.height() / src.height().max(0.01);
    DRect {
        x0: -src.x0 * sx,
        y0: -src.y0 * sy,
        x1: (page.0 - src.x0) * sx,
        y1: (page.1 - src.y0) * sy,
    }
}

/// Horizontal bands one round dab is made of: the rectangles core takes out approximate
/// the circle the eraser shows.
const DAB_BANDS: usize = 8;

/// Round dabs `size` points wide along the brush path `points`, close enough to cover it
/// (centres at most half a width apart, none closer than a quarter width), each one
/// [`DAB_BANDS`] rectangles whose width is the circle's at the band's middle.
pub fn brush_dabs(points: &[(f32, f32)], size: f32) -> Vec<DRect> {
    let half = size / 2.0;
    let mut centres: Vec<(f32, f32)> = points.first().copied().into_iter().collect();
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let steps = ((b.0 - a.0).hypot(b.1 - a.1) / half).ceil().max(1.0) as usize;
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            let centre = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            let last = centres[centres.len() - 1];
            if (centre.0 - last.0).hypot(centre.1 - last.1) >= half / 2.0 {
                centres.push(centre);
            }
        }
    }
    let band = size / DAB_BANDS as f32;
    centres
        .into_iter()
        .flat_map(|(x, y)| {
            (0..DAB_BANDS).map(move |index| {
                let top = -half + index as f32 * band;
                let middle = top + band / 2.0;
                let reach = (half * half - middle * middle).max(0.0).sqrt();
                DRect {
                    x0: x - reach,
                    y0: y + top,
                    x1: x + reach,
                    y1: y + top + band,
                }
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brush_dabs_follow_the_path() {
        assert!(brush_dabs(&[], 4.0).is_empty());
        // A click is one dab around it.
        assert_eq!(brush_dabs(&[(10.0, 10.0)], 4.0).len(), DAB_BANDS);
        // 20 points right with a 4 point brush: centres every 2 points, 11 dabs.
        let dabs = brush_dabs(&[(0.0, 0.0), (20.0, 0.0)], 4.0);
        assert_eq!(dabs.len(), 11 * DAB_BANDS);
        // The last dab's widest bands reach its right edge.
        assert!(dabs.iter().any(|band| (band.x1 - 22.0).abs() < 0.1));
        // Jitter under a quarter width adds nothing.
        assert_eq!(
            brush_dabs(&[(0.0, 0.0), (0.5, 0.0), (0.0, 0.5)], 4.0).len(),
            DAB_BANDS
        );
    }

    #[test]
    fn a_dab_covers_its_circle() {
        let (radius, centre) = (10.0_f32, (50.0, 50.0));
        let bands = brush_dabs(&[centre], 2.0 * radius);
        // Stacked top to bottom, no gap, spanning the diameter.
        assert!((bands[0].y0 - (centre.1 - radius)).abs() < 1e-3);
        assert!((bands[DAB_BANDS - 1].y1 - (centre.1 + radius)).abs() < 1e-3);
        for pair in bands.windows(2) {
            assert!((pair[0].y1 - pair[1].y0).abs() < 1e-3);
        }
        // Same area as the circle, within 2%.
        let area: f32 = bands.iter().map(|band| band.width() * band.height()).sum();
        let circle = std::f32::consts::PI * radius * radius;
        assert!((area - circle).abs() / circle < 0.02, "{area} vs {circle}");
        // Sampled on a grid: what the bands take and the circle agree almost everywhere.
        let (mut differ, mut samples) = (0, 0);
        for i in 0..200 {
            for j in 0..200 {
                let (x, y) = (40.0 + i as f32 * 0.1, 40.0 + j as f32 * 0.1);
                let inside = (x - centre.0).hypot(y - centre.1) <= radius;
                let taken = bands
                    .iter()
                    .any(|b| x >= b.x0 && x <= b.x1 && y >= b.y0 && y <= b.y1);
                differ += usize::from(inside != taken);
                samples += 1;
            }
        }
        // Each sample stands for 0.01 square points; the steps at the rim stay under 8%.
        let mismatch = differ as f32 * 0.01;
        assert!(mismatch < circle * 0.08, "{mismatch} of {circle} differ");
        assert_eq!(samples, 40_000);
        // Nothing far outside the circle is taken.
        for band in &bands {
            for (x, y) in band.corners() {
                assert!((x - centre.0).hypot(y - centre.1) < radius * 1.12);
            }
        }
    }

    fn geom(rotation: i32) -> PageGeometry {
        PageGeometry {
            rotation,
            origin: (0.0, 0.0),
        }
    }

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> DRect {
        DRect { x0, y0, x1, y1 }
    }

    #[test]
    fn the_page_picture_is_cut_to_the_moved_annotation() {
        let page = (600.0, 800.0);
        let src = rect(100.0, 200.0, 150.0, 220.0);
        // Moved: same scale, the page shifted so `src` sits at the box's corner.
        let moved = crop(src, src.translate((30.0, 40.0)), page);
        assert_eq!(moved, rect(-100.0, -200.0, 500.0, 600.0));
        // Twice as wide: the page is stretched twice as wide too.
        let wide = crop(src, rect(0.0, 0.0, 100.0, 20.0), page);
        assert_eq!(wide, rect(-200.0, -200.0, 1000.0, 600.0));
    }

    #[test]
    fn a_dragged_annotation_keeps_the_grab_point_under_the_pointer() {
        // Grabbed at (110, 205), 10 / 5 points into the box; pointer now at (300, 405).
        let original = rect(100.0, 200.0, 150.0, 220.0);
        let (grab, pointer) = ((110.0, 205.0), (300.0, 405.0));
        let offset = clamp_offset(
            original,
            (600.0, 800.0),
            (pointer.0 - grab.0, pointer.1 - grab.1),
        );
        let moved = original.translate(offset);
        assert_eq!((moved.x0 + 10.0, moved.y0 + 5.0), pointer);
    }

    #[test]
    fn hit_test_prefers_topmost() {
        let rects = [rect(0.0, 0.0, 100.0, 100.0), rect(50.0, 50.0, 70.0, 70.0)];
        assert_eq!(hit_test(&rects, (60.0, 60.0), 0.0), Some(1));
        assert_eq!(hit_test(&rects, (10.0, 10.0), 0.0), Some(0));
        assert_eq!(hit_test(&rects, (102.0, 10.0), 3.0), Some(0));
        assert_eq!(hit_test(&rects, (200.0, 10.0), 3.0), None);
    }

    #[test]
    fn distance_to_a_stroke() {
        let line = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)];
        assert_eq!(polyline_distance(&line, (5.0, 3.0)), 3.0);
        assert_eq!(polyline_distance(&line, (13.0, 5.0)), 3.0);
        // Past an end: to the end point.
        assert_eq!(polyline_distance(&line, (-3.0, -4.0)), 5.0);
        // The far corner of the bounding box is far from the stroke.
        assert!(polyline_distance(&line, (0.0, 10.0)) > 7.0);
        assert_eq!(polyline_distance(&[(1.0, 1.0)], (4.0, 5.0)), 5.0);
        assert_eq!(polyline_distance(&[], (0.0, 0.0)), f32::INFINITY);
    }

    #[test]
    fn delta_flips_y_and_follows_rotation() {
        let shown = (600.0, 800.0);
        assert_eq!(pdf_delta(&geom(0), shown, (10.0, 20.0)), (10.0, -20.0));
        assert_eq!(pdf_delta(&geom(180), shown, (10.0, 20.0)), (-10.0, 20.0));
        let landscape = (800.0, 600.0);
        assert_eq!(pdf_delta(&geom(90), landscape, (10.0, 20.0)), (20.0, 10.0));
    }

    #[test]
    fn pdf_round_trip() {
        let shown = (600.0, 800.0);
        let original = rect(100.0, 100.0, 150.0, 120.0);
        let back = DRect::from_pdf(&geom(0), shown, &original.to_pdf(&geom(0), shown));
        assert_eq!(back, original);
    }

    #[test]
    fn offset_is_clamped_to_the_page() {
        let item = rect(10.0, 10.0, 30.0, 30.0);
        assert_eq!(
            clamp_offset(item, (100.0, 100.0), (-50.0, 500.0)),
            (-10.0, 70.0)
        );
        assert_eq!(clamp_offset(item, (100.0, 100.0), (5.0, 5.0)), (5.0, 5.0));
    }

    #[test]
    fn resizing_keeps_the_opposite_corner_and_a_minimum_size() {
        let original = rect(100.0, 100.0, 200.0, 150.0);
        let page = (600.0, 800.0);
        assert_eq!(
            resized_rect(original, 2, (300.0, 400.0), page, false),
            rect(100.0, 100.0, 300.0, 400.0)
        );
        assert_eq!(
            resized_rect(original, 0, (500.0, 500.0), page, false),
            rect(192.0, 142.0, 200.0, 150.0)
        );
        assert_eq!(
            resized_rect(original, 2, (5000.0, 5000.0), page, false).x1,
            600.0
        );
        assert_eq!(
            resized_rect(original, 1, (400.0, 0.0), page, true),
            rect(100.0, 100.0, 400.0, 150.0)
        );
    }

    #[test]
    fn a_picture_keeps_its_proportions() {
        let original = rect(100.0, 100.0, 200.0, 150.0);
        let page = (600.0, 800.0);
        assert_eq!(
            keep_ratio(original, rect(100.0, 100.0, 300.0, 160.0), 2, page),
            rect(100.0, 100.0, 300.0, 200.0)
        );
        assert_eq!(
            keep_ratio(original, rect(150.0, 140.0, 200.0, 150.0), 0, page),
            rect(150.0, 125.0, 200.0, 150.0)
        );
        // Pulled down along the right edge: as wide as the page lets it, no wider.
        assert_eq!(
            keep_ratio(original, rect(100.0, 100.0, 600.0, 500.0), 2, page),
            rect(100.0, 100.0, 600.0, 350.0)
        );
    }

    #[test]
    fn pictures_are_placed_at_their_size_or_fitted() {
        let page = (600.0, 800.0);
        assert_eq!(
            place_picture((100.0, 50.0), page, (10.0, 20.0), None),
            rect(10.0, 20.0, 110.0, 70.0)
        );
        // Too big for the room left: shrunk, proportions kept.
        let big = place_picture((1200.0, 600.0), page, (0.0, 0.0), None);
        assert_eq!(big, rect(0.0, 0.0, 600.0, 300.0));
        // Fitted in a dragged box and centered in it.
        assert_eq!(
            place_picture((100.0, 50.0), page, (0.0, 0.0), Some((200.0, 200.0))),
            rect(0.0, 50.0, 200.0, 150.0)
        );
    }

    #[test]
    fn picture_fields_in_millimetres() {
        let original = rect(72.0, 144.0, 144.0, 180.0);
        assert!((PictureField::X.value_mm(original) - 25.4).abs() < 1e-4);
        assert!((PictureField::Height.value_mm(original) - 12.7).abs() < 1e-4);
        let wider = edit_picture(original, PictureField::Width, 50.8, true);
        assert!((wider.width() - 144.0).abs() < 1e-3 && (wider.height() - 72.0).abs() < 1e-3);
        assert_eq!((wider.x0, wider.y0), (72.0, 144.0));
        let free = edit_picture(original, PictureField::Height, 25.4, false);
        assert!((free.width() - 72.0).abs() < 1e-3 && (free.height() - 72.0).abs() < 1e-3);
        let moved = edit_picture(original, PictureField::Y, 0.0, true);
        assert_eq!(moved, rect(72.0, 0.0, 144.0, 36.0));
        // Never below 2 mm.
        assert!(edit_picture(original, PictureField::Width, 0.0, false).width() > 5.6);
    }

    #[test]
    fn ellipse_and_arrow_previews() {
        let points = ellipse_points(rect(0.0, 0.0, 100.0, 50.0), 4);
        assert_eq!(points.len(), 5);
        assert!((points[0].0 - 100.0).abs() < 1e-3 && (points[0].1 - 25.0).abs() < 1e-3);
        assert!((points[1].0 - 50.0).abs() < 1e-3 && (points[1].1 - 50.0).abs() < 1e-3);
        // Pointing right: both wings behind the tip, one above and one below.
        let [a, b] = arrow_wings((0.0, 0.0), (10.0, 0.0), 2.0).unwrap();
        assert!(a.0 < 10.0 && b.0 < 10.0 && a.1 * b.1 < 0.0);
        assert!(((a.0 - 10.0).hypot(a.1) - 2.0).abs() < 1e-4);
        assert_eq!(arrow_wings((1.0, 1.0), (1.0, 1.0), 2.0), None);
    }

    #[test]
    fn typed_millimetres() {
        assert_eq!(parse_mm(" 12,5 mm"), Some(12.5));
        assert_eq!(parse_mm("40"), Some(40.0));
        assert_eq!(parse_mm("abc"), None);
        assert_eq!(parse_mm("inf"), None);
    }
}
