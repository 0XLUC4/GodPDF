//! The handwritten signature of the user (ported from the egui `src/signature.rs`): drawn
//! once on a pad, kept on disk between sessions, put on pages as an ink drawing that can be
//! moved and resized. The pad dialog is `dialogs/signpad.rs`, the panel `panels/sign.rs`,
//! placing it `tools/sign.rs`.

use std::path::PathBuf;

use gpui::{App, Bounds, Global, PathBuilder, Pixels, Point, Window, point, px, rgb, size};

/// Width of a signature put on a page, in points.
pub const WIDTH: f32 = 150.0;
/// Ink of a signature: a dark blue, like a pen.
pub const INK: godpdf_core::Color = godpdf_core::Color {
    r: 0.05,
    g: 0.12,
    b: 0.45,
};
pub const PEN: f32 = 1.4;
/// The ink as the pad and the panel show it.
pub const SCREEN_INK: u32 = 0x0D1F73;

/// Strokes in a box one unit wide, y pointing down.
#[derive(Clone, Debug, PartialEq)]
pub struct Signature {
    pub strokes: Vec<Vec<(f32, f32)>>,
}

impl Signature {
    /// From strokes drawn on the pad: framed tight and made one unit wide.
    pub fn from_pad(strokes: &[Vec<(f32, f32)>]) -> Option<Self> {
        let points: Vec<(f32, f32)> = strokes.iter().flatten().copied().collect();
        let (min_x, min_y, max_x, max_y) = points.iter().fold(
            (f32::MAX, f32::MAX, f32::MIN, f32::MIN),
            |(x0, y0, x1, y1), &(x, y)| (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
        );
        if points.len() < 2 || (max_x - min_x).max(max_y - min_y) < 4.0 {
            return None;
        }
        let scale = 1.0 / (max_x - min_x).max(1.0);
        let strokes = strokes
            .iter()
            .filter(|stroke| !stroke.is_empty())
            .map(|stroke| {
                stroke
                    .iter()
                    .map(|&(x, y)| ((x - min_x) * scale, (y - min_y) * scale))
                    .collect()
            })
            .collect();
        Some(Self { strokes })
    }

    /// Read back what [`Signature::to_text`] wrote.
    pub fn from_text(text: &str) -> Option<Self> {
        let strokes: Option<Vec<Vec<(f32, f32)>>> = text
            .split(';')
            .filter(|stroke| !stroke.trim().is_empty())
            .map(|stroke| {
                stroke
                    .split_whitespace()
                    .map(|point| {
                        let (x, y) = point.split_once(',')?;
                        Some((x.parse().ok()?, y.parse().ok()?))
                    })
                    .collect()
            })
            .collect();
        let strokes = strokes?;
        (!strokes.is_empty()).then_some(Self { strokes })
    }

    /// Points as `x,y` separated by spaces, strokes by `;`.
    pub fn to_text(&self) -> String {
        self.strokes
            .iter()
            .map(|stroke| {
                stroke
                    .iter()
                    .map(|(x, y)| format!("{x:.4},{y:.4}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
            .join(";")
    }

    /// Height for a width of one.
    pub fn height(&self) -> f32 {
        self.strokes
            .iter()
            .flatten()
            .map(|p| p.1)
            .fold(0.0, f32::max)
    }

    /// The strokes `WIDTH` points wide, top left corner at `at` (displayed points).
    pub fn placed(&self, at: (f32, f32)) -> Vec<Vec<(f32, f32)>> {
        self.strokes
            .iter()
            .map(|stroke| {
                stroke
                    .iter()
                    .map(|&(x, y)| (at.0 + x * WIDTH, at.1 + y * WIDTH))
                    .collect()
            })
            .collect()
    }
}

/// The saved signature, read from disk once at start.
struct Saved(Option<Signature>);

impl Global for Saved {}

/// Where it is kept (the egui app keeps it in its own settings file).
// shortcut: own text file, not shared with the egui app's settings; move to the app settings once GPUI has them.
fn file() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    };
    Some(base?.join("GodPDF").join("signature.txt"))
}

pub fn register(cx: &mut App) {
    let saved = file()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| Signature::from_text(&text));
    cx.set_global(Saved(saved));
}

pub fn saved(cx: &App) -> Option<Signature> {
    cx.try_global::<Saved>()?.0.clone()
}

/// Keep `signature` (or forget it) for this and the next sessions.
pub fn set(signature: Option<Signature>, cx: &mut App) {
    let text = signature
        .as_ref()
        .map(Signature::to_text)
        .unwrap_or_default();
    cx.set_global(Saved(signature));
    let written = file()
        .ok_or_else(|| "no settings folder".to_owned())
        .and_then(|path| {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|err| err.to_string())?;
            }
            crate::files::write_atomic(&path, text.as_bytes())
        });
    if let Err(err) = written {
        crate::toasts::error(cx, format!("Cannot keep the signature: {err}"));
    }
    cx.refresh_windows();
}

/// Paint `strokes` (pixels from `origin`) in the screen ink, `width` px wide; a lone point is a dot.
pub fn paint_strokes(
    window: &mut Window,
    origin: Point<Pixels>,
    strokes: &[Vec<(f32, f32)>],
    width: f32,
) {
    let ink = rgb(SCREEN_INK);
    let at = |&(x, y): &(f32, f32)| origin + point(px(x), px(y));
    for stroke in strokes {
        if let [dot] = stroke.as_slice() {
            let dot = Bounds::centered_at(at(dot), size(px(3.), px(3.)));
            window.paint_quad(gpui::fill(dot, ink).corner_radii(px(1.5)));
            continue;
        }
        let mut path = PathBuilder::stroke(px(width));
        for (index, p) in stroke.iter().enumerate() {
            if index == 0 {
                path.move_to(at(p));
            } else {
                path.line_to(at(p));
            }
        }
        // An empty stroke makes no path.
        if let Ok(path) = path.build() {
            window.paint_path(path, ink);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signature_is_kept_as_text() {
        let drawn = vec![vec![(10.0, 10.0), (110.0, 60.0)], vec![(60.0, 20.0)]];
        let signature = Signature::from_pad(&drawn).unwrap();
        assert_eq!(signature.strokes[0], [(0.0, 0.0), (1.0, 0.5)]);
        let back = Signature::from_text(&signature.to_text()).unwrap();
        assert_eq!(back.to_text(), signature.to_text());
        let placed = back.placed((100.0, 200.0));
        assert_eq!(placed[0][1], (100.0 + WIDTH, 200.0 + WIDTH / 2.0));
    }

    #[test]
    fn nothing_drawn_is_no_signature() {
        assert!(Signature::from_pad(&[]).is_none());
        assert!(Signature::from_pad(&[vec![(1.0, 1.0)]]).is_none());
        assert!(Signature::from_text("").is_none());
        assert!(Signature::from_text("1,a").is_none());
    }
}
