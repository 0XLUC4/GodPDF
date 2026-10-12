//! Free-text box typed directly on the page, before it becomes a PDF annotation.
//!
//! The box is laid out like core's FreeText appearance (same padding, line height, first
//! baseline) with a Helvetica-metric font, wraps at its own width, and is anchored by its
//! top-left corner on the page as displayed, so committing changes neither position, size nor
//! weight. The text reads upright on the page as it is turned.

use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat};
use egui::{
    Color32, CursorIcon, FontFamily, FontId, Frame, Galley, Id, Key, Margin, Modifiers, Pos2, Rect,
    Sense, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, Vec2, vec2,
};
use godpdf_core::{Annotation, FREE_TEXT_ASCENT, FREE_TEXT_LINE_HEIGHT, FREE_TEXT_PADDING};

use crate::geometry::{self, PageGeom};

/// Width of the draggable border ring in screen pixels.
const RING: f32 = 5.0;
/// Box border; the page is always white, so this stays near-black in both themes.
const BORDER: Color32 = Color32::from_gray(0x22);
/// Width of a new box, in points (clamped to the page).
const DEFAULT_WIDTH: f32 = 220.0;
/// Narrowest box, in font sizes.
const MIN_WIDTH_EMS: f32 = 2.0;
/// egui font family with Helvetica metrics (see [`install_font`]).
const FONT_FAMILY: &str = "pdf-sans";

/// Register the editor font: a system font metric-compatible with the PDF's Helvetica (Arial,
/// Liberation Sans), so typed text has the width and weight of the committed annotation.
/// Falls back to the UI font when none is installed.
pub fn install_font(fonts: &mut egui::FontDefinitions) {
    let mut family = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    if let Some(bytes) = system_font() {
        fonts.font_data.insert(
            FONT_FAMILY.to_owned(),
            egui::FontData::from_owned(bytes).into(),
        );
        family.insert(0, FONT_FAMILY.to_owned());
    }
    fonts
        .families
        .insert(FontFamily::Name(FONT_FAMILY.into()), family);
}

#[cfg(not(target_arch = "wasm32"))]
fn system_font() -> Option<Vec<u8>> {
    let windows = std::env::var_os("WINDIR").map(|dir| {
        std::path::PathBuf::from(dir)
            .join("Fonts")
            .join("arial.ttf")
    });
    let unix = [
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/Library/Fonts/Arial.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        "/usr/share/fonts/liberation-sans/LiberationSans-Regular.ttf",
        "/usr/share/fonts/TTF/LiberationSans-Regular.ttf",
    ];
    windows
        .into_iter()
        .chain(unix.into_iter().map(std::path::PathBuf::from))
        .find_map(|path| std::fs::read(path).ok())
}

#[cfg(target_arch = "wasm32")]
fn system_font() -> Option<Vec<u8>> {
    None
}

pub struct TextBox {
    pub tab: u64,
    pub page: usize,
    /// Top-left corner in displayed page points.
    anchor: Pos2,
    /// Box width in points, padding included. Text wraps inside it.
    width: f32,
    geom: PageGeom,
    /// Displayed (post-rotation) page size in points.
    displayed: Vec2,
    pub text: String,
    /// `text` as wrapped on screen, one entry per visual line (set when shown).
    lines: Vec<String>,
    /// Annotation being edited (already removed from the document); restored on cancel.
    pub original: Option<Annotation>,
    focus_pending: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Commit,
    Cancel,
}

/// Where a box goes: its page and geometry.
pub struct Placement {
    pub page: usize,
    /// Top-left corner in displayed page points.
    pub anchor: Pos2,
    /// Box width in points; `None` picks the default.
    pub width: Option<f32>,
    pub geom: PageGeom,
    /// Displayed (post-rotation) page size in points.
    pub displayed: Vec2,
}

impl TextBox {
    pub fn new(tab: u64, place: Placement, text: String, original: Option<Annotation>) -> Self {
        Self {
            tab,
            page: place.page,
            anchor: place.anchor,
            width: place.width.unwrap_or(DEFAULT_WIDTH),
            geom: place.geom,
            displayed: place.displayed,
            text,
            lines: Vec::new(),
            original,
            focus_pending: true,
        }
    }

    /// The annotation /Rect: the box as shown.
    pub fn pdf_rect(&self, font_size: f32) -> godpdf_core::Rect {
        let shown = self.displayed_rect(font_size);
        geometry::to_pdf_rect(&self.geom, self.displayed, shown.min, shown.max)
    }

    pub fn rotation(&self) -> i32 {
        self.geom.rotation
    }

    fn height(&self, font_size: f32) -> f32 {
        let lines = self.lines.len().max(1) as f32;
        lines * font_size * FREE_TEXT_LINE_HEIGHT + 2.0 * FREE_TEXT_PADDING
    }

    /// Keep the whole box on the page: narrow it, then slide it back inside.
    fn fit(&mut self, font_size: f32) {
        let page = self.displayed;
        let min_width = (MIN_WIDTH_EMS * font_size).min(page.x);
        self.width = self.width.clamp(min_width, page.x);
        self.anchor.x = self.anchor.x.clamp(0.0, page.x - self.width);
        let highest_top = (page.y - self.height(font_size)).max(0.0);
        self.anchor.y = self.anchor.y.clamp(0.0, highest_top);
    }

    /// Box in displayed page points.
    fn displayed_rect(&self, font_size: f32) -> Rect {
        Rect::from_min_size(self.anchor, vec2(self.width, self.height(font_size)))
    }

    fn id(&self) -> Id {
        Id::new(("text-box", self.tab))
    }
}

/// egui layout of `text` with the PDF line height, wrapped at `wrap_width`.
fn layout(
    painter: &egui::Painter,
    text: &str,
    font: &FontId,
    line_height: f32,
    color: Color32,
    wrap_width: f32,
) -> Arc<Galley> {
    let mut job = LayoutJob::single_section(
        text.to_owned(),
        TextFormat {
            font_id: font.clone(),
            line_height: Some(line_height),
            color,
            ..Default::default()
        },
    );
    job.wrap.max_width = wrap_width;
    painter.layout_job(job)
}

fn editor_font(font_size: f32, zoom: f32) -> FontId {
    FontId::new(
        (font_size * zoom).max(1.0),
        FontFamily::Name(FONT_FAMILY.into()),
    )
}

/// Where the text starts in a box whose top-left corner is `frame_min` (screen), so that its
/// first baseline is where the PDF appearance puts it.
fn text_origin(
    painter: &egui::Painter,
    frame_min: Pos2,
    font: &FontId,
    font_size: f32,
    zoom: f32,
) -> Pos2 {
    let line_height = font_size * FREE_TEXT_LINE_HEIGHT * zoom;
    let probe = layout(
        painter,
        "Hg",
        font,
        line_height,
        Color32::BLACK,
        f32::INFINITY,
    );
    let egui_baseline = probe
        .rows
        .first()
        .and_then(|row| row.glyphs.first())
        .map_or(font.size * 0.8, |glyph| glyph.pos.y);
    let pad = FREE_TEXT_PADDING * zoom;
    frame_min
        + vec2(
            pad,
            pad + FREE_TEXT_ASCENT * font_size * zoom - egui_baseline,
        )
}

/// Paint `text` as the free-text annotation with the box `frame` (screen) will show it:
/// same wrapping, font and baselines. `turn` is how far the text is turned clockwise on
/// screen (0, 90, 180 or 270). Used while such a box is resized.
pub fn paint(
    painter: &egui::Painter,
    frame: Rect,
    turn: i32,
    text: &str,
    font_size: f32,
    zoom: f32,
    color: Color32,
) {
    let turn = turn.rem_euclid(360);
    let upright_width = if turn % 180 == 0 {
        frame.width()
    } else {
        frame.height()
    };
    let corner = match turn {
        90 => frame.right_top(),
        180 => frame.right_bottom(),
        270 => frame.left_bottom(),
        _ => frame.left_top(),
    };
    let font = editor_font(font_size, zoom);
    let lines = godpdf_core::wrap_free_text(text, font_size, upright_width / zoom);
    let galley = layout(
        painter,
        &lines.join("\n"),
        &font,
        font_size * FREE_TEXT_LINE_HEIGHT * zoom,
        color,
        f32::INFINITY,
    );
    let offset = text_origin(painter, Pos2::ZERO, &font, font_size, zoom).to_vec2();
    let angle = (turn as f32).to_radians();
    let rotation = egui::emath::Rot2::from_angle(angle);
    let shape =
        egui::epaint::TextShape::new(corner + rotation * offset, galley, color).with_angle(angle);
    painter.add(shape);
}

/// Draw and edit the box over its page (`page_rect` on screen). Returns how it ended, if it did.
pub fn show(
    ui: &mut Ui,
    text_box: &mut TextBox,
    page_rect: Rect,
    zoom: f32,
    font_size: f32,
    color: Color32,
) -> Option<Event> {
    let id = text_box.id();
    let font = editor_font(font_size, zoom);
    let line_height = font_size * FREE_TEXT_LINE_HEIGHT * zoom;
    let focused = ui.memory(|memory| memory.has_focus(id));
    let (finish, escape) = ui.input_mut(|input| {
        let finish = focused && input.consume_key(Modifiers::COMMAND, Key::Enter);
        (finish, input.key_pressed(Key::Escape))
    });

    text_box.fit(font_size);
    let shown = text_box.displayed_rect(font_size);
    let frame_rect = Rect::from_min_max(
        page_rect.min + shown.min.to_vec2() * zoom,
        page_rect.min + shown.max.to_vec2() * zoom,
    );
    let pad = FREE_TEXT_PADDING * zoom;
    let wrap_width = (frame_rect.width() - 2.0 * pad).max(1.0);

    let text_origin = text_origin(ui.painter(), frame_rect.min, &font, font_size, zoom);
    let text_rect = Rect::from_min_size(
        text_origin,
        vec2(
            wrap_width,
            (frame_rect.height() - 2.0 * pad).max(line_height),
        ),
    );

    // Border ring first so the text field and the resize edge (added after) win over it.
    let ring = ui
        .interact(frame_rect.expand(RING), id.with("ring"), Sense::drag())
        .on_hover_cursor(CursorIcon::Move);
    let edge_rect = Rect::from_min_max(
        frame_rect.right_top() - vec2(RING, 0.0),
        frame_rect.right_bottom() + vec2(RING, 0.0),
    );
    let edge = ui
        .interact(edge_rect, id.with("resize"), Sense::drag())
        .on_hover_cursor(CursorIcon::ResizeHorizontal)
        .on_hover_text("Drag to resize");
    let painter = ui.painter();
    painter.rect(
        frame_rect,
        0.0,
        Color32::from_white_alpha(170),
        Stroke::new(1.0, BORDER),
        StrokeKind::Outside,
    );
    // Thick right border with a grip: the box can be widened from here.
    painter.vline(
        frame_rect.right() + 1.0,
        frame_rect.y_range(),
        Stroke::new(3.0, BORDER),
    );
    let grip = Rect::from_center_size(
        frame_rect.right_center() + vec2(1.0, 0.0),
        vec2(7.0, 18.0f32.min(frame_rect.height())),
    );
    painter.rect(
        grip,
        2.0,
        Color32::WHITE,
        Stroke::new(1.5, BORDER),
        StrokeKind::Inside,
    );

    let mut wrapped: Option<Arc<Galley>> = None;
    {
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(text_rect)
                .id_salt(id.with("edit")),
        );
        let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, _wrap: f32| {
            let galley = layout(
                ui.painter(),
                text.as_str(),
                &font,
                line_height,
                color,
                wrap_width,
            );
            wrapped = Some(Arc::clone(&galley));
            galley
        };
        let edit = TextEdit::multiline(&mut text_box.text)
            .id(id)
            .frame(Frame::NONE)
            .margin(Margin::ZERO)
            .desired_width(wrap_width)
            .desired_rows(1)
            .layouter(&mut layouter);
        let response = child.add(edit);
        if std::mem::take(&mut text_box.focus_pending) || ring.drag_stopped() || edge.drag_stopped()
        {
            response.request_focus();
        }
    }
    if let Some(galley) = wrapped {
        text_box.lines = galley
            .rows
            .iter()
            .map(|row| row.row.text().trim_end().to_owned())
            .collect();
    }

    if edge.dragged() {
        text_box.width += edge.drag_delta().x / zoom;
    }
    if ring.dragged() {
        move_by(text_box, ring.drag_delta() / zoom);
    }
    text_box.fit(font_size);

    if escape {
        return Some(Event::Cancel);
    }
    if finish {
        return Some(Event::Commit);
    }
    clicked_outside(ui, frame_rect.expand(RING)).then_some(Event::Commit)
}

/// Move the anchor by `delta` displayed points ([`TextBox::fit`] keeps it on the page).
fn move_by(text_box: &mut TextBox, delta: Vec2) {
    text_box.anchor += delta;
}

/// A click on this (viewer) layer, outside `keep`. Clicks on popups or other panels don't count.
fn clicked_outside(ui: &Ui, keep: Rect) -> bool {
    let clip = ui.clip_rect();
    let layer = ui.layer_id();
    let click = ui.input(|input| {
        input
            .pointer
            .any_click()
            .then(|| input.pointer.interact_pos())
            .flatten()
    });
    click.is_some_and(|pos| {
        clip.contains(pos) && !keep.contains(pos) && ui.ctx().layer_id_at(pos) == Some(layer)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_box(anchor: (f32, f32), width: Option<f32>) -> TextBox {
        let place = Placement {
            page: 0,
            anchor: egui::pos2(anchor.0, 792.0 - anchor.1),
            width,
            geom: PageGeom {
                rotation: 0,
                origin: (0.0, 0.0),
            },
            displayed: vec2(612.0, 792.0),
        };
        TextBox::new(1, place, "Hello".into(), None)
    }

    fn pdf_anchor(tb: &TextBox) -> (f32, f32) {
        let rect = tb.pdf_rect(12.0);
        (rect.x0, rect.y1)
    }

    #[test]
    fn rect_hangs_from_the_anchor() {
        let mut tb = text_box((100.0, 700.0), Some(150.0));
        tb.lines = vec!["Hello".into(), "world".into()];
        let rect = tb.pdf_rect(12.0);
        assert_eq!((rect.x0, rect.y1, rect.x1), (100.0, 700.0, 250.0));
        let height = 2.0 * 12.0 * FREE_TEXT_LINE_HEIGHT + 2.0 * FREE_TEXT_PADDING;
        assert!((rect.y1 - rect.y0 - height).abs() < 0.001);
        assert_eq!(tb.displayed_rect(12.0).min, egui::pos2(100.0, 92.0));
    }

    #[test]
    fn dragging_moves_the_anchor_but_never_off_the_page() {
        let mut tb = text_box((100.0, 700.0), Some(150.0));
        move_by(&mut tb, vec2(10.0, 20.0));
        tb.fit(12.0);
        assert_eq!(pdf_anchor(&tb), (110.0, 680.0));
        move_by(&mut tb, vec2(-1000.0, -1000.0));
        tb.fit(12.0);
        assert_eq!(pdf_anchor(&tb), (0.0, 792.0));
        move_by(&mut tb, vec2(5000.0, 5000.0));
        tb.fit(12.0);
        let rect = tb.pdf_rect(12.0);
        assert_eq!(rect.x1, 612.0);
        assert!(rect.y0.abs() < 0.001);
    }

    #[test]
    fn box_is_narrowed_to_the_page_and_grows_upwards_at_the_bottom() {
        // Wider than the page: clamped to the page width.
        let mut tb = text_box((500.0, 700.0), Some(5000.0));
        tb.fit(12.0);
        assert_eq!((pdf_anchor(&tb).0, tb.width), (0.0, 612.0));
        // Near the bottom edge: more lines push the box up, not off the page.
        let mut tb = text_box((100.0, 20.0), None);
        tb.lines = vec![String::new(); 5];
        tb.fit(12.0);
        assert!(tb.pdf_rect(12.0).y0.abs() < 0.001);
    }

    #[test]
    fn box_on_a_turned_page_runs_along_the_screen() {
        let place = Placement {
            page: 0,
            anchor: egui::pos2(100.0, 50.0),
            width: Some(150.0),
            geom: PageGeom {
                rotation: 90,
                origin: (0.0, 0.0),
            },
            displayed: vec2(792.0, 612.0),
        };
        let mut tb = TextBox::new(1, place, "Hello".into(), None);
        tb.lines = vec!["Hello".into()];
        tb.fit(12.0);
        let rect = tb.pdf_rect(12.0);
        assert_eq!(tb.rotation(), 90);
        assert!((rect.y1 - rect.y0 - 150.0).abs() < 0.001);
        assert_eq!((rect.x0, rect.y0), (50.0, 100.0));
    }
}
