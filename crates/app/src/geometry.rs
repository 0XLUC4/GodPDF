//! Screen (displayed page) coordinates -> unrotated PDF user space for annotations.
//!
//! Render reports page sizes *after* /Rotate, but annotations live in unrotated space.

use egui::{Pos2, Vec2};
pub use godpdf_core::{PageGeometry as PageGeom, page_geometry};

/// `p` is in displayed points, origin top-left; `displayed` is the post-rotation page size.
pub fn to_pdf(geom: &PageGeom, displayed: Vec2, p: Pos2) -> (f32, f32) {
    let (sx, sy) = (p.x, p.y);
    let (w, h) = if geom.rotation % 180 == 0 {
        (displayed.x, displayed.y)
    } else {
        (displayed.y, displayed.x)
    };
    let (ux, uy) = match geom.rotation {
        90 => (sy, sx),
        180 => (w - sx, sy),
        270 => (w - sy, h - sx),
        _ => (sx, h - sy),
    };
    (ux + geom.origin.0, uy + geom.origin.1)
}

/// Inverse of [`to_pdf`]: unrotated PDF user space -> displayed points (origin top-left).
pub fn from_pdf(geom: &PageGeom, displayed: Vec2, (x, y): (f32, f32)) -> Pos2 {
    let (ux, uy) = (x - geom.origin.0, y - geom.origin.1);
    let (w, h) = if geom.rotation % 180 == 0 {
        (displayed.x, displayed.y)
    } else {
        (displayed.y, displayed.x)
    };
    let (sx, sy) = match geom.rotation {
        90 => (uy, ux),
        180 => (w - ux, uy),
        270 => (h - uy, w - ux),
        _ => (ux, h - uy),
    };
    Pos2::new(sx, sy)
}

/// Displayed-space rectangle of a PDF rectangle.
pub fn from_pdf_rect(geom: &PageGeom, displayed: Vec2, rect: &godpdf_core::Rect) -> egui::Rect {
    egui::Rect::from_two_pos(
        from_pdf(geom, displayed, (rect.x0, rect.y0)),
        from_pdf(geom, displayed, (rect.x1, rect.y1)),
    )
}

pub fn to_pdf_rect(geom: &PageGeom, displayed: Vec2, a: Pos2, b: Pos2) -> godpdf_core::Rect {
    let (ax, ay) = to_pdf(geom, displayed, a);
    let (bx, by) = to_pdf(geom, displayed, b);
    godpdf_core::Rect {
        x0: ax.min(bx),
        y0: ay.min(by),
        x1: ax.max(bx),
        y1: ay.max(by),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2};

    fn geom(rotation: i32) -> PageGeom {
        PageGeom {
            rotation,
            origin: (0.0, 0.0),
        }
    }

    #[test]
    fn unrotated_flips_y() {
        assert_eq!(
            to_pdf(&geom(0), vec2(600.0, 800.0), pos2(10.0, 20.0)),
            (10.0, 780.0)
        );
    }

    #[test]
    fn from_pdf_inverts_to_pdf() {
        for rotation in [0, 90, 180, 270] {
            let shown = if rotation % 180 == 0 {
                vec2(600.0, 800.0)
            } else {
                vec2(800.0, 600.0)
            };
            let geom = PageGeom {
                rotation,
                origin: (5.0, 7.0),
            };
            let p = pos2(123.0, 45.0);
            assert_eq!(from_pdf(&geom, shown, to_pdf(&geom, shown, p)), p);
        }
    }

    #[test]
    fn rotated_corners_map_to_unrotated_corners() {
        // Portrait 600x800 shown landscape 800x600.
        let shown = vec2(800.0, 600.0);
        // 90° clockwise: displayed top-left is the original bottom-left.
        assert_eq!(to_pdf(&geom(90), shown, pos2(0.0, 0.0)), (0.0, 0.0));
        // 270°: displayed bottom-right is the original bottom-left.
        assert_eq!(to_pdf(&geom(270), shown, pos2(800.0, 600.0)), (0.0, 0.0));
        assert_eq!(to_pdf(&geom(270), shown, pos2(0.0, 600.0)), (0.0, 800.0));
        // 180°: displayed top-right is the original bottom-left.
        assert_eq!(
            to_pdf(&geom(180), vec2(600.0, 800.0), pos2(600.0, 0.0)),
            (0.0, 0.0)
        );
    }
}
