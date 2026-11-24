//! Pure logic of the stamp tool: where a clicked stamp goes, and the date it can carry.

use std::time::{SystemTime, UNIX_EPOCH};

use super::geom::{self, DRect};

/// Size of a new stamp, in points; a dated one is taller for its second line.
const STAMP_SIZE: (f32, f32) = (150.0, 44.0);
const DATED_STAMP_SIZE: (f32, f32) = (150.0, 58.0);
/// Longest side of a picture stamp, in points.
const PICTURE_STAMP: f32 = 150.0;

/// A new stamp centred on `center`, moved back onto a page of size `page` if it overflows.
pub fn stamp_rect(center: (f32, f32), page: (f32, f32), dated: bool) -> DRect {
    let (w, h) = if dated { DATED_STAMP_SIZE } else { STAMP_SIZE };
    let rect = DRect {
        x0: center.0 - w / 2.0,
        y0: center.1 - h / 2.0,
        x1: center.0 + w / 2.0,
        y1: center.1 + h / 2.0,
    };
    rect.translate(geom::clamp_offset(rect, page, (0.0, 0.0)))
}

/// A picture stamp centred on `center`: its `natural` size, made smaller to fit in a square
/// of `PICTURE_STAMP` points, then moved back onto the page like [`stamp_rect`].
pub fn picture_rect(center: (f32, f32), natural: (f32, f32), page: (f32, f32)) -> DRect {
    let fit = (PICTURE_STAMP / natural.0.max(natural.1).max(f32::EPSILON)).min(1.0);
    let (w, h) = (natural.0 * fit, natural.1 * fit);
    let rect = DRect {
        x0: center.0 - w / 2.0,
        y0: center.1 - h / 2.0,
        x1: center.0 + w / 2.0,
        y1: center.1 + h / 2.0,
    };
    rect.translate(geom::clamp_offset(rect, page, (0.0, 0.0)))
}

/// Today as "YYYY-MM-DD".
// shortcut: UTC date (no time zone database), off by one near midnight; use a tz crate if that matters.
pub fn today() -> String {
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() / 86_400);
    let (year, month, day) = civil_date(days as i64);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Calendar date of a day count since 1970-01-01 (Howard Hinnant's `civil_from_days`).
fn civil_date(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_since_epoch_to_dates() {
        assert_eq!(civil_date(0), (1970, 1, 1));
        assert_eq!(civil_date(59), (1970, 3, 1));
        assert_eq!(civil_date(11_016), (2000, 2, 29));
        assert_eq!(civil_date(20_402), (2025, 11, 10));
        assert_eq!(today().len(), 10);
    }

    #[test]
    fn picture_stamps_shrink_to_fit() {
        let rect = picture_rect((300.0, 400.0), (600.0, 300.0), (600.0, 800.0));
        assert_eq!(
            rect,
            DRect {
                x0: 225.0,
                y0: 362.5,
                x1: 375.0,
                y1: 437.5
            }
        );
        let small = picture_rect((10.0, 10.0), (40.0, 20.0), (600.0, 800.0));
        assert_eq!((small.x0, small.y0, small.width()), (0.0, 0.0, 40.0));
    }

    #[test]
    fn stamps_are_centred_and_stay_on_the_page() {
        let page = (600.0, 800.0);
        let rect = stamp_rect((300.0, 400.0), page, false);
        assert_eq!(
            rect,
            DRect {
                x0: 225.0,
                y0: 378.0,
                x1: 375.0,
                y1: 422.0
            }
        );
        let corner = stamp_rect((0.0, 800.0), page, true);
        assert_eq!(
            corner,
            DRect {
                x0: 0.0,
                y0: 742.0,
                x1: 150.0,
                y1: 800.0
            }
        );
    }
}
