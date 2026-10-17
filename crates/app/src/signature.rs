//! The handwritten signature of the user: drawn once on a pad, kept between sessions, put
//! on pages as a drawing that can be moved and resized.

use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};

use crate::theme::Palette;
use crate::widgets::{icon, with_icon};

/// Size of the drawing pad on screen.
const PAD: egui::Vec2 = vec2(440.0, 170.0);
/// Width of a signature put on a page, in points.
pub const WIDTH: f32 = 150.0;
/// Ink of a signature: a dark blue, like a pen.
pub const INK: godpdf_core::Color = godpdf_core::Color {
    r: 0.05,
    g: 0.12,
    b: 0.45,
};
pub const PEN: f32 = 1.4;

/// Strokes in a box one unit wide, y pointing down.
#[derive(Clone, Debug, PartialEq)]
pub struct Signature {
    strokes: Vec<Vec<(f32, f32)>>,
}

impl Signature {
    /// From strokes drawn on the pad: framed tight and made one unit wide.
    pub fn from_pad(strokes: &[Vec<Pos2>]) -> Option<Self> {
        let points: Vec<Pos2> = strokes.iter().flatten().copied().collect();
        let bounds = Rect::from_points(&points);
        if points.len() < 2 || bounds.width().max(bounds.height()) < 4.0 {
            return None;
        }
        let scale = 1.0 / bounds.width().max(1.0);
        let strokes = strokes
            .iter()
            .filter(|stroke| !stroke.is_empty())
            .map(|stroke| {
                stroke
                    .iter()
                    .map(|p| ((p.x - bounds.min.x) * scale, (p.y - bounds.min.y) * scale))
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
    pub fn placed(&self, at: Pos2) -> Vec<Vec<Pos2>> {
        self.strokes
            .iter()
            .map(|stroke| {
                stroke
                    .iter()
                    .map(|&(x, y)| at + vec2(x, y) * WIDTH)
                    .collect()
            })
            .collect()
    }
}

/// A pad to sign on with the mouse or a pen.
pub fn pad(ui: &mut Ui, strokes: &mut Vec<Vec<Pos2>>) {
    let palette = Palette::of(ui);
    let (rect, response) = ui.allocate_exact_size(PAD, Sense::drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, Color32::WHITE);
    painter.rect_stroke(
        rect,
        6.0,
        Stroke::new(1.0, palette.border),
        egui::StrokeKind::Inside,
    );
    let line_y = rect.bottom() - 40.0;
    painter.line_segment(
        [
            pos2(rect.left() + 24.0, line_y),
            pos2(rect.right() - 24.0, line_y),
        ],
        Stroke::new(1.0, Color32::from_gray(200)),
    );
    if response.drag_started() {
        strokes.push(Vec::new());
    }
    if response.dragged()
        && let Some(pos) = response.interact_pointer_pos()
        && let Some(stroke) = strokes.last_mut()
    {
        let point = (pos - rect.min).to_pos2();
        if stroke.last() != Some(&point) {
            stroke.push(point);
        }
    }
    let ink = Color32::from_rgb(13, 31, 115);
    for stroke in strokes.iter() {
        let points: Vec<Pos2> = stroke.iter().map(|p| rect.min + p.to_vec2()).collect();
        match points.as_slice() {
            [dot] => {
                painter.circle_filled(*dot, 1.5, ink);
            }
            _ => {
                painter.add(egui::Shape::line(points, Stroke::new(2.5, ink)));
            }
        }
    }
    if strokes.iter().all(Vec::is_empty) {
        painter.text(
            rect.center() - vec2(0.0, 12.0),
            egui::Align2::CENTER_CENTER,
            "Sign here",
            egui::FontId::proportional(16.0),
            Color32::from_gray(170),
        );
    }
}

/// What the signature panel asks the app to do.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PanelAction {
    Draw,
    Place,
    Forget,
}

/// The signature panel: the saved signature and what to do with it.
pub fn panel(ui: &mut Ui, saved: Option<&Signature>) -> Option<PanelAction> {
    let palette = Palette::of(ui);
    let mut action = None;
    let Some(signature) = saved else {
        ui.label(
            RichText::new("Draw your signature once: it is kept for the next time.")
                .color(palette.text_dim),
        );
        ui.add_space(6.0);
        if ui
            .button(with_icon(icon::PENCIL_SIMPLE, "Draw my signature"))
            .clicked()
        {
            action = Some(PanelAction::Draw);
        }
        return action;
    };
    let width = ui.available_width().min(280.0);
    let height = (signature.height() * width).clamp(40.0, 140.0) + 16.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, Color32::WHITE);
    let ink = Color32::from_rgb(13, 31, 115);
    let scale = (width - 16.0).min((height - 16.0) / signature.height().max(0.01));
    for stroke in &signature.strokes {
        let points: Vec<Pos2> = stroke
            .iter()
            .map(|&(x, y)| rect.min + vec2(8.0, 8.0) + vec2(x, y) * scale)
            .collect();
        painter.add(egui::Shape::line(points, Stroke::new(2.0, ink)));
    }
    ui.add_space(8.0);
    let place = egui::Button::new(
        RichText::new(with_icon(icon::SIGNATURE, "Sign the page")).color(palette.on_accent),
    )
    .fill(palette.accent);
    if ui
        .add(place)
        .on_hover_text("Then click where the signature goes")
        .clicked()
    {
        action = Some(PanelAction::Place);
    }
    ui.horizontal(|ui| {
        if ui
            .button(with_icon(icon::PENCIL_SIMPLE, "Draw again"))
            .clicked()
        {
            action = Some(PanelAction::Draw);
        }
        if ui.button(with_icon(icon::TRASH, "Forget it")).clicked() {
            action = Some(PanelAction::Forget);
        }
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signature_is_kept_as_text() {
        let drawn = vec![
            vec![pos2(10.0, 10.0), pos2(110.0, 60.0)],
            vec![pos2(60.0, 20.0)],
        ];
        let signature = Signature::from_pad(&drawn).unwrap();
        assert_eq!(signature.strokes[0], [(0.0, 0.0), (1.0, 0.5)]);
        let back = Signature::from_text(&signature.to_text()).unwrap();
        assert_eq!(back.to_text(), signature.to_text());
        let placed = back.placed(pos2(100.0, 200.0));
        assert_eq!(placed[0][1], pos2(100.0 + WIDTH, 200.0 + WIDTH / 2.0));
    }

    #[test]
    fn nothing_drawn_is_no_signature() {
        assert!(Signature::from_pad(&[]).is_none());
        assert!(Signature::from_pad(&[vec![pos2(1.0, 1.0)]]).is_none());
        assert!(Signature::from_text("").is_none());
        assert!(Signature::from_text("1,a").is_none());
    }
}
