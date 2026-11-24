//! Pure coordinate math: the continuous-scroll page layout, and displayed page points <->
//! unrotated PDF user space.
//!
//! Three spaces:
//! - **view**: logical pixels inside the viewer, origin at its top-left corner.
//! - **content**: view + scroll offset; pages are stacked top to bottom in it.
//! - **displayed page points**: origin top-left of a page, after /Rotate; this is what
//!   `godpdf_render` reports. One point is `zoom` logical pixels.
//! - **PDF user space**: unrotated, origin bottom-left; what `godpdf_core` edits take.

pub use godpdf_core::PageGeometry;

/// Margin around the stack of pages, in logical pixels.
pub const PAD: f32 = 28.0;
/// Space between two pages, in logical pixels.
pub const GAP: f32 = 18.0;
pub const MIN_ZOOM: f32 = 0.1;
pub const MAX_ZOOM: f32 = 6.0;
/// Factor of one Ctrl+/- step or one Ctrl+wheel notch.
pub const ZOOM_STEP: f32 = 1.2;

/// A page rectangle in content space, logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// How the pages sit in rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Spread {
    /// One page per row.
    #[default]
    Single,
    /// Two pages side by side.
    Pairs,
    /// Two pages side by side, the first one alone as a cover.
    Cover,
}

impl Spread {
    /// The pages of each row, top to bottom, for `count` pages.
    pub fn rows(self, count: usize) -> Vec<std::ops::Range<usize>> {
        let first = match self {
            Self::Single => return (0..count).map(|page| page..page + 1).collect(),
            Self::Pairs => 0,
            Self::Cover => count.min(1),
        };
        let cover = (first == 1).then_some(0..1);
        cover
            .into_iter()
            .chain(
                (first..count)
                    .step_by(2)
                    .map(|start| start..(start + 2).min(count)),
            )
            .collect()
    }
}

/// Where every page sits in content space at one zoom and one viewer width.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub zoom: f32,
    pub rects: Vec<PageRect>,
    pub content_width: f32,
    pub content_height: f32,
}

impl Layout {
    /// One page per row: see [`Layout::with_spread`].
    pub fn new(sizes: &[(f32, f32)], zoom: f32, view_width: f32) -> Self {
        Self::with_spread(sizes, zoom, view_width, Spread::Single)
    }

    /// `sizes` are displayed page sizes in points (width, height); rows of pages are
    /// centered in `view_width` (or in the content when it is wider than the view), pages
    /// of a row top-aligned.
    pub fn with_spread(sizes: &[(f32, f32)], zoom: f32, view_width: f32, spread: Spread) -> Self {
        let rows = spread.rows(sizes.len());
        let row_width = |row: &std::ops::Range<usize>| {
            let pages: f32 = sizes[row.clone()].iter().map(|size| size.0 * zoom).sum();
            pages + GAP * row.len().saturating_sub(1) as f32
        };
        let widest = rows.iter().map(row_width).fold(0.0, f32::max);
        let content_width = (widest + 2.0 * PAD).max(view_width);
        let mut rects = Vec::with_capacity(sizes.len());
        let mut y = PAD;
        for row in &rows {
            let mut x = (content_width - row_width(row)) / 2.0;
            let mut tallest: f32 = 0.0;
            for &(width, height) in &sizes[row.clone()] {
                let (width, height) = (width * zoom, height * zoom);
                rects.push(PageRect {
                    x,
                    y,
                    width,
                    height,
                });
                x += width + GAP;
                tallest = tallest.max(height);
            }
            y += tallest + GAP;
        }
        let content_height = if rects.is_empty() { 0.0 } else { y - GAP + PAD };
        Self {
            zoom,
            rects,
            content_width,
            content_height,
        }
    }

    /// Pages that intersect the content band `top..top + height`.
    pub fn visible(&self, top: f32, height: f32) -> std::ops::Range<usize> {
        let bottom = top + height;
        // Not a partition with rows of pages of different heights: scanned.
        let first = self
            .rects
            .iter()
            .position(|rect| rect.y + rect.height >= top)
            .unwrap_or(self.rects.len());
        let end = self.rects.partition_point(|rect| rect.y <= bottom);
        first..end.max(first)
    }

    /// The page "being read": the first page of the last row whose top is above 35% of
    /// the view.
    pub fn current_page(&self, scroll_y: f32, view_height: f32) -> usize {
        let probe = scroll_y + view_height * 0.35;
        let mut page = self
            .rects
            .partition_point(|rect| rect.y <= probe)
            .saturating_sub(1);
        while page > 0 && self.rects[page - 1].y == self.rects[page].y {
            page -= 1;
        }
        page
    }

    /// Page under a content-space point, and that point in displayed page points.
    pub fn hit(&self, x: f32, y: f32) -> Option<(usize, (f32, f32))> {
        let page = self.rects.iter().position(|rect| {
            x >= rect.x && x <= rect.x + rect.width && y >= rect.y && y <= rect.y + rect.height
        })?;
        let rect = self.rects[page];
        Some((page, ((x - rect.x) / self.zoom, (y - rect.y) / self.zoom)))
    }

    /// Largest scroll offsets for a view of `view` (width, height).
    pub fn max_scroll(&self, view: (f32, f32)) -> (f32, f32) {
        (
            (self.content_width - view.0).max(0.0),
            (self.content_height - view.1).max(0.0),
        )
    }

    /// Scroll offset that puts the top of `page` near the top of the view.
    pub fn scroll_to_page(&self, page: usize) -> Option<f32> {
        self.rects
            .get(page)
            .map(|rect| (rect.y - PAD / 2.0).max(0.0))
    }
}

/// Zoom at which a `page` (points) fits whole in a `view` (logical pixels), margins included.
pub fn fit_page_zoom(page: (f32, f32), view: (f32, f32)) -> f32 {
    let fit = |view: f32, page: f32| (view - 2.0 * PAD) / page.max(1.0);
    fit(view.0, page.0)
        .min(fit(view.1, page.1))
        .clamp(MIN_ZOOM, MAX_ZOOM)
}

/// Zoom that fits the widest row of pages in the viewer width.
pub fn fit_width_zoom(sizes: &[(f32, f32)], view_width: f32, spread: Spread) -> f32 {
    let rows = spread.rows(sizes.len());
    let widest = rows
        .iter()
        .map(|row| sizes[row.clone()].iter().map(|size| size.0).sum())
        .fold(1.0, f32::max);
    let gaps = rows.iter().map(|row| row.len() - 1).max().unwrap_or(0) as f32 * GAP;
    ((view_width - 2.0 * PAD - gaps) / widest).clamp(MIN_ZOOM, MAX_ZOOM)
}

/// Size in points of the row of pages `page` is in: widths added, the tallest height.
pub fn row_size(sizes: &[(f32, f32)], spread: Spread, page: usize) -> Option<(f32, f32)> {
    let row = spread
        .rows(sizes.len())
        .into_iter()
        .find(|row| row.contains(&page))?;
    Some(sizes[row].iter().fold((0.0, 0.0), |(width, height), size| {
        (width + size.0, f32::max(height, size.1))
    }))
}

/// New scroll offset after zooming so the content under `anchor` (view space) stays under it.
/// `old` and `new` are the layouts before and after; `scroll` is the old offset.
pub fn anchored_scroll(
    old: &Layout,
    new: &Layout,
    scroll: (f32, f32),
    anchor: (f32, f32),
) -> (f32, f32) {
    let (content_x, content_y) = (scroll.0 + anchor.0, scroll.1 + anchor.1);
    // Vertical: keep the same fraction of the same page under the anchor (gaps don't scale).
    let page = old
        .rects
        .partition_point(|rect| rect.y <= content_y)
        .saturating_sub(1);
    let new_y = match (old.rects.get(page), new.rects.get(page)) {
        (Some(before), Some(after)) => {
            let fraction = (content_y - before.y) / before.height.max(1.0);
            after.y + fraction * after.height
        }
        _ => content_y * new.zoom / old.zoom,
    };
    // Horizontal: pages are centered, so scale around the content center.
    let new_x =
        new.content_width / 2.0 + (content_x - old.content_width / 2.0) * new.zoom / old.zoom;
    (new_x - anchor.0, new_y - anchor.1)
}

/// `p` is in displayed points, origin top-left; `displayed` is the post-rotation page size.
pub fn to_pdf(geom: &PageGeometry, displayed: (f32, f32), (sx, sy): (f32, f32)) -> (f32, f32) {
    let (w, h) = unrotated(geom, displayed);
    let (ux, uy) = match geom.rotation {
        90 => (sy, sx),
        180 => (w - sx, sy),
        270 => (w - sy, h - sx),
        _ => (sx, h - sy),
    };
    (ux + geom.origin.0, uy + geom.origin.1)
}

/// Inverse of [`to_pdf`]: unrotated PDF user space -> displayed points (origin top-left).
pub fn from_pdf(geom: &PageGeometry, displayed: (f32, f32), (x, y): (f32, f32)) -> (f32, f32) {
    let (ux, uy) = (x - geom.origin.0, y - geom.origin.1);
    let (w, h) = unrotated(geom, displayed);
    match geom.rotation {
        90 => (uy, ux),
        180 => (w - ux, uy),
        270 => (h - uy, w - ux),
        _ => (ux, h - uy),
    }
}

/// PDF rectangle spanned by two displayed points.
pub fn to_pdf_rect(
    geom: &PageGeometry,
    displayed: (f32, f32),
    a: (f32, f32),
    b: (f32, f32),
) -> godpdf_core::Rect {
    let (ax, ay) = to_pdf(geom, displayed, a);
    let (bx, by) = to_pdf(geom, displayed, b);
    godpdf_core::Rect {
        x0: ax.min(bx),
        y0: ay.min(by),
        x1: ax.max(bx),
        y1: ay.max(by),
    }
}

fn unrotated(geom: &PageGeometry, displayed: (f32, f32)) -> (f32, f32) {
    if geom.rotation % 180 == 0 {
        displayed
    } else {
        (displayed.1, displayed.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LETTER: (f32, f32) = (612.0, 792.0);

    #[test]
    fn fit_page_fits_the_tighter_side() {
        // Tall view: the width decides; wide view: the height.
        assert_eq!(fit_page_zoom((100.0, 100.0), (256.0, 1000.0)), 2.0);
        assert_eq!(fit_page_zoom((100.0, 100.0), (1000.0, 156.0)), 1.0);
    }

    fn geom(rotation: i32) -> PageGeometry {
        PageGeometry {
            rotation,
            origin: (0.0, 0.0),
        }
    }

    #[test]
    fn pages_stack_with_pad_and_gap() {
        let layout = Layout::new(&[LETTER, LETTER], 1.0, 100.0);
        assert_eq!(layout.rects[0].y, PAD);
        assert_eq!(layout.rects[1].y, PAD + 792.0 + GAP);
        assert_eq!(layout.content_height, PAD + 2.0 * 792.0 + GAP + PAD);
        assert_eq!(layout.content_width, 612.0 + 2.0 * PAD);
    }

    #[test]
    fn pages_center_in_a_wide_view() {
        let layout = Layout::new(&[LETTER], 0.5, 1000.0);
        assert_eq!(layout.rects[0].x, (1000.0 - 306.0) / 2.0);
    }

    #[test]
    fn visible_and_current_pages() {
        let layout = Layout::new(&[LETTER; 10], 1.0, 800.0);
        assert_eq!(layout.visible(0.0, 600.0), 0..1);
        let second = layout.rects[1].y;
        assert_eq!(layout.visible(second - GAP - 10.0, 600.0), 0..2);
        assert_eq!(layout.current_page(second, 600.0), 1);
        assert_eq!(layout.visible(1.0e9, 600.0), 10..10);
    }

    #[test]
    fn hit_maps_to_page_points() {
        let layout = Layout::new(&[LETTER], 2.0, 0.0);
        let rect = layout.rects[0];
        assert_eq!(
            layout.hit(rect.x + 20.0, rect.y + 40.0),
            Some((0, (10.0, 20.0)))
        );
        assert_eq!(layout.hit(0.0, 0.0), None);
    }

    #[test]
    fn zoom_keeps_anchor_on_same_spot() {
        let sizes = [LETTER; 5];
        let old = Layout::new(&sizes, 1.0, 800.0);
        let new = Layout::new(&sizes, 2.0, 800.0);
        let scroll = (0.0, 1000.0);
        let anchor = (400.0, 300.0);
        let (page, point) = old.hit(scroll.0 + anchor.0, scroll.1 + anchor.1).unwrap();
        let after = anchored_scroll(&old, &new, scroll, anchor);
        let (new_page, new_point) = new.hit(after.0 + anchor.0, after.1 + anchor.1).unwrap();
        assert_eq!(page, new_page);
        assert!((point.0 - new_point.0).abs() < 0.01 && (point.1 - new_point.1).abs() < 0.01);
    }

    #[test]
    fn spreads_put_pages_in_rows() {
        assert_eq!(Spread::Single.rows(2), [0..1, 1..2]);
        assert_eq!(Spread::Pairs.rows(5), [0..2, 2..4, 4..5]);
        assert_eq!(Spread::Cover.rows(4), [0..1, 1..3, 3..4]);
        assert!(Spread::Cover.rows(0).is_empty());
        let layout = Layout::with_spread(&[LETTER; 3], 1.0, 0.0, Spread::Cover);
        let [cover, left, right] = layout.rects[..] else {
            panic!("three pages");
        };
        assert_eq!(layout.content_width, 2.0 * 612.0 + GAP + 2.0 * PAD);
        assert_eq!(cover.x, (layout.content_width - 612.0) / 2.0);
        assert_eq!((left.y, right.y), (PAD + 792.0 + GAP, PAD + 792.0 + GAP));
        assert_eq!(right.x, left.x + 612.0 + GAP);
        // The row's first page is the one being read; both pages of it show.
        assert_eq!(layout.current_page(left.y, 600.0), 1);
        assert_eq!(layout.visible(left.y, 10.0), 1..3);
        assert_eq!(
            row_size(&[LETTER; 3], Spread::Cover, 2),
            Some((1224.0, 792.0))
        );
        assert_eq!(
            fit_width_zoom(&[LETTER; 2], 1224.0 + 2.0 * PAD + GAP, Spread::Pairs),
            1.0
        );
    }

    #[test]
    fn unrotated_flips_y() {
        assert_eq!(
            to_pdf(&geom(0), (600.0, 800.0), (10.0, 20.0)),
            (10.0, 780.0)
        );
    }

    #[test]
    fn from_pdf_inverts_to_pdf() {
        for rotation in [0, 90, 180, 270] {
            let shown = if rotation % 180 == 0 {
                (600.0, 800.0)
            } else {
                (800.0, 600.0)
            };
            let geom = PageGeometry {
                rotation,
                origin: (5.0, 7.0),
            };
            let p = (123.0, 45.0);
            assert_eq!(from_pdf(&geom, shown, to_pdf(&geom, shown, p)), p);
        }
    }
}
