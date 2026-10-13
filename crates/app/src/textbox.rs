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
use godpdf_core::{
    Annotation, CharStyle, FREE_TEXT_ASCENT, FREE_TEXT_LINE_HEIGHT, FREE_TEXT_PADDING, Font,
    Script, style_at,
};

use crate::geometry::{self, PageGeom};
use crate::tools::CommentStyle;

/// Width of the draggable border ring in screen pixels.
const RING: f32 = 5.0;
/// Box border; the page is always white, so this stays near-black in both themes.
const BORDER: Color32 = Color32::from_gray(0x22);
/// Width of a new box, in points (clamped to the page).
const DEFAULT_WIDTH: f32 = 220.0;
/// Narrowest box, in font sizes.
const MIN_WIDTH_EMS: f32 = 2.0;

pub fn install_font(fonts: &mut egui::FontDefinitions) {
    let proportional = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    for font in Font::ALL {
        for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
            let name = family_name(font, bold, italic);
            if let Some(bytes) = system_font(font, bold, italic) {
                fonts
                    .font_data
                    .insert(name.clone(), egui::FontData::from_owned(bytes).into());
            }
        }
    }
    for font in Font::ALL {
        for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
            let mut family: Vec<String> = [
                family_name(font, bold, italic),
                family_name(font, bold, false),
                family_name(font, false, false),
            ]
            .into_iter()
            .filter(|name| fonts.font_data.contains_key(name))
            .collect();
            family.dedup();
            family.extend(proportional.iter().cloned());
            fonts.families.insert(
                FontFamily::Name(family_name(font, bold, italic).into()),
                family,
            );
        }
    }
    let mut bold = proportional;
    let helvetica_bold = family_name(Font::Helvetica, true, false);
    if fonts.font_data.contains_key(&helvetica_bold) {
        bold.insert(0, helvetica_bold);
    }
    fonts
        .families
        .insert(FontFamily::Name(BOLD_FAMILY.into()), bold);
}

pub const BOLD_FAMILY: &str = "ui-bold";

fn family_name(font: Font, bold: bool, italic: bool) -> String {
    format!("pdf-{}", font.resource(bold, italic))
}

#[cfg(not(target_arch = "wasm32"))]
fn system_font(font: Font, bold: bool, italic: bool) -> Option<Vec<u8>> {
    let variant = match (bold, italic) {
        (false, false) => 0,
        (true, false) => 1,
        (false, true) => 2,
        (true, true) => 3,
    };
    let (windows, mac, unix) = match font {
        Font::Helvetica => (
            ["arial.ttf", "arialbd.ttf", "ariali.ttf", "arialbi.ttf"],
            ["Arial", "Arial Bold", "Arial Italic", "Arial Bold Italic"],
            "LiberationSans",
        ),
        Font::Times => (
            ["times.ttf", "timesbd.ttf", "timesi.ttf", "timesbi.ttf"],
            [
                "Times New Roman",
                "Times New Roman Bold",
                "Times New Roman Italic",
                "Times New Roman Bold Italic",
            ],
            "LiberationSerif",
        ),
        Font::Courier => (
            ["cour.ttf", "courbd.ttf", "couri.ttf", "courbi.ttf"],
            [
                "Courier New",
                "Courier New Bold",
                "Courier New Italic",
                "Courier New Bold Italic",
            ],
            "LiberationMono",
        ),
    };
    let suffix = ["Regular", "Bold", "Italic", "BoldItalic"][variant];
    let unix_file = format!("{unix}-{suffix}.ttf");
    let mac_file = format!("{}.ttf", mac[variant]);
    let windows = std::env::var_os("WINDIR").map(|dir| {
        std::path::PathBuf::from(dir)
            .join("Fonts")
            .join(windows[variant])
    });
    let others = [
        format!("/System/Library/Fonts/Supplemental/{mac_file}"),
        format!("/Library/Fonts/{mac_file}"),
        format!("/usr/share/fonts/truetype/liberation/{unix_file}"),
        format!(
            "/usr/share/fonts/liberation-{}/{unix_file}",
            unix.trim_start_matches("Liberation").to_lowercase()
        ),
        format!("/usr/share/fonts/TTF/{unix_file}"),
    ];
    windows
        .into_iter()
        .chain(others.iter().map(std::path::PathBuf::from))
        .find_map(|path| std::fs::read(path).ok())
}

#[cfg(target_arch = "wasm32")]
fn system_font(_font: Font, _bold: bool, _italic: bool) -> Option<Vec<u8>> {
    None
}

pub struct TextBox {
    pub tab: u64,
    pub page: usize,
    anchor: Pos2,
    width: f32,
    geom: PageGeom,
    displayed: Vec2,
    pub text: String,
    pub styles: Vec<CharStyle>,
    styled_text: String,
    typing: CharStyle,
    selection: std::ops::Range<usize>,
    lines: Vec<String>,
    pub original: Option<Annotation>,
    focus_pending: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Commit,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Bold,
    Italic,
    Underline,
    Sub,
    Super,
}

impl Format {
    pub const ALL: [Format; 5] = [
        Format::Bold,
        Format::Italic,
        Format::Underline,
        Format::Sub,
        Format::Super,
    ];

    pub fn is_on(self, style: CharStyle) -> bool {
        match self {
            Self::Bold => style.bold,
            Self::Italic => style.italic,
            Self::Underline => style.underline,
            Self::Sub => style.script == Script::Sub,
            Self::Super => style.script == Script::Super,
        }
    }

    pub fn set(self, style: &mut CharStyle, on: bool) {
        match self {
            Self::Bold => style.bold = on,
            Self::Italic => style.italic = on,
            Self::Underline => style.underline = on,
            Self::Sub | Self::Super if !on => {
                if self.is_on(*style) {
                    style.script = Script::Normal;
                }
            }
            Self::Sub => style.script = Script::Sub,
            Self::Super => style.script = Script::Super,
        }
    }
}

pub fn toggle_all(
    styles: &mut Vec<CharStyle>,
    len: usize,
    range: std::ops::Range<usize>,
    format: Format,
) {
    styles.resize(len, CharStyle::default());
    let range = range.start.min(len)..range.end.min(len);
    let on = !styles[range.clone()]
        .iter()
        .all(|style| format.is_on(*style));
    for style in &mut styles[range] {
        format.set(style, on);
    }
    if styles.iter().all(|style| *style == CharStyle::default()) {
        styles.clear();
    }
}

pub fn common_style(styles: &[CharStyle], range: std::ops::Range<usize>) -> CharStyle {
    let mut common = CharStyle::default();
    if range.is_empty() {
        return common;
    }
    for format in Format::ALL {
        let on = range
            .clone()
            .all(|index| format.is_on(style_at(styles, index)));
        if on {
            format.set(&mut common, true);
        }
    }
    common
}

pub struct Placement {
    pub page: usize,
    pub anchor: Pos2,
    pub width: Option<f32>,
    pub geom: PageGeom,
    pub displayed: Vec2,
}

impl TextBox {
    pub fn new(
        tab: u64,
        place: Placement,
        text: String,
        styles: Vec<CharStyle>,
        original: Option<Annotation>,
    ) -> Self {
        let end = text.chars().count();
        Self {
            tab,
            page: place.page,
            anchor: place.anchor,
            width: place.width.unwrap_or(DEFAULT_WIDTH),
            geom: place.geom,
            displayed: place.displayed,
            styled_text: text.clone(),
            typing: end
                .checked_sub(1)
                .map_or_else(CharStyle::default, |last| style_at(&styles, last)),
            text,
            styles,
            selection: end..end,
            lines: Vec::new(),
            original,
            focus_pending: true,
        }
    }

    pub fn pdf_rect(&self, font_size: f32) -> godpdf_core::Rect {
        let shown = self.displayed_rect(font_size);
        geometry::to_pdf_rect(&self.geom, self.displayed, shown.min, shown.max)
    }

    pub fn rotation(&self) -> i32 {
        self.geom.rotation
    }

    pub fn committed_styles(&self, len: usize) -> Vec<CharStyle> {
        let styles: Vec<CharStyle> = (0..len)
            .map(|index| style_at(&self.styles, index))
            .collect();
        if styles.iter().all(|style| *style == CharStyle::default()) {
            return Vec::new();
        }
        styles
    }

    pub fn active(&self) -> CharStyle {
        if self.selection.is_empty() {
            self.typing
        } else {
            common_style(&self.styles, self.selection.clone())
        }
    }

    pub fn toggle(&mut self, format: Format) {
        if self.selection.is_empty() {
            let on = !format.is_on(self.typing);
            format.set(&mut self.typing, on);
        } else {
            let len = self.text.chars().count();
            toggle_all(&mut self.styles, len, self.selection.clone(), format);
        }
        self.focus_pending = true;
    }

    pub fn refocus(&mut self) {
        self.focus_pending = true;
    }

    fn height(&self, font_size: f32) -> f32 {
        let lines = self.lines.len().max(1) as f32;
        lines * font_size * FREE_TEXT_LINE_HEIGHT + 2.0 * FREE_TEXT_PADDING
    }

    fn fit(&mut self, font_size: f32) {
        let page = self.displayed;
        let min_width = (MIN_WIDTH_EMS * font_size).min(page.x);
        self.width = self.width.clamp(min_width, page.x);
        self.anchor.x = self.anchor.x.clamp(0.0, page.x - self.width);
        let highest_top = (page.y - self.height(font_size)).max(0.0);
        self.anchor.y = self.anchor.y.clamp(0.0, highest_top);
    }

    fn displayed_rect(&self, font_size: f32) -> Rect {
        Rect::from_min_size(self.anchor, vec2(self.width, self.height(font_size)))
    }

    fn id(&self) -> Id {
        Id::new(("text-box", self.tab))
    }
}

fn reconcile(styles: &mut Vec<CharStyle>, known: &mut String, typing: CharStyle, text: &str) {
    if known == text {
        return;
    }
    let old: Vec<char> = known.chars().collect();
    let new: Vec<char> = text.chars().collect();
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let mut next: Vec<CharStyle> = (0..prefix).map(|index| style_at(styles, index)).collect();
    next.extend(std::iter::repeat_n(typing, new.len() - prefix - suffix));
    next.extend((old.len() - suffix..old.len()).map(|index| style_at(styles, index)));
    if next.iter().all(|style| *style == CharStyle::default()) {
        next.clear();
    }
    *styles = next;
    *known = text.to_owned();
}

fn editor_font(font: Font, style: CharStyle, font_size: f32, zoom: f32) -> FontId {
    FontId::new(
        (style.size(font_size) * zoom).max(1.0),
        FontFamily::Name(family_name(font, style.bold, style.italic).into()),
    )
}

struct Look {
    font: Font,
    font_size: f32,
    zoom: f32,
    color: Color32,
}

fn text_format(look: &Look, style: CharStyle) -> TextFormat {
    let valign = if style.script == Script::Super {
        egui::Align::TOP
    } else {
        egui::Align::BOTTOM
    };
    let underline = if style.underline {
        Stroke::new((look.font_size * 0.06 * look.zoom).max(1.0), look.color)
    } else {
        Stroke::NONE
    };
    TextFormat {
        font_id: editor_font(look.font, style, look.font_size, look.zoom),
        line_height: Some(look.font_size * FREE_TEXT_LINE_HEIGHT * look.zoom),
        color: look.color,
        underline,
        valign,
        ..Default::default()
    }
}

fn styled_job(
    chars: impl Iterator<Item = (char, CharStyle)>,
    look: &Look,
    wrap_width: f32,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mut run = String::new();
    let mut run_style = CharStyle::default();
    for (c, style) in chars {
        if !run.is_empty() && style != run_style {
            job.append(&std::mem::take(&mut run), 0.0, text_format(look, run_style));
        }
        run_style = style;
        run.push(c);
    }
    job.append(&run, 0.0, text_format(look, run_style));
    job.wrap.max_width = wrap_width;
    job
}

fn text_origin(painter: &egui::Painter, frame_min: Pos2, look: &Look) -> Pos2 {
    let probe = styled_job(
        "Hg".chars().map(|c| (c, CharStyle::default())),
        look,
        f32::INFINITY,
    );
    let probe = painter.layout_job(probe);
    let font_size = look.font_size * look.zoom;
    let egui_baseline = probe
        .rows
        .first()
        .and_then(|row| row.glyphs.first())
        .map_or(font_size * 0.8, |glyph| glyph.pos.y);
    let pad = FREE_TEXT_PADDING * look.zoom;
    frame_min + vec2(pad, pad + FREE_TEXT_ASCENT * font_size - egui_baseline)
}

pub struct Shape<'a> {
    pub text: &'a str,
    pub styles: &'a [CharStyle],
    pub font: Font,
    pub font_size: f32,
    pub color: Color32,
}

pub fn paint(painter: &egui::Painter, frame: Rect, turn: i32, shape: &Shape, zoom: f32) {
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
    let look = Look {
        font: shape.font,
        font_size: shape.font_size,
        zoom,
        color: shape.color,
    };
    let chars: Vec<char> = shape.text.chars().collect();
    let lines = godpdf_core::wrap_styled(
        shape.text,
        shape.styles,
        shape.font,
        shape.font_size,
        upright_width / zoom,
    );
    let mut sequence = Vec::new();
    for (index, line) in lines.into_iter().enumerate() {
        if index > 0 {
            sequence.push(('\n', CharStyle::default()));
        }
        sequence.extend(line.map(|at| (chars[at], style_at(shape.styles, at))));
    }
    let galley = painter.layout_job(styled_job(sequence.into_iter(), &look, f32::INFINITY));
    let offset = text_origin(painter, Pos2::ZERO, &look).to_vec2();
    let angle = (turn as f32).to_radians();
    let rotation = egui::emath::Rot2::from_angle(angle);
    let text = egui::epaint::TextShape::new(corner + rotation * offset, galley, shape.color)
        .with_angle(angle);
    painter.add(text);
}

pub fn show(
    ui: &mut Ui,
    text_box: &mut TextBox,
    page_rect: Rect,
    zoom: f32,
    style: &CommentStyle,
) -> Option<Event> {
    let id = text_box.id();
    let font_size = style.font_size;
    let look = Look {
        font: style.font,
        font_size,
        zoom,
        color: style.color,
    };
    let line_height = font_size * FREE_TEXT_LINE_HEIGHT * zoom;
    let focused = ui.memory(|memory| memory.has_focus(id));
    let (finish, escape, formats) = ui.input_mut(|input| {
        let finish = focused && input.consume_key(Modifiers::COMMAND, Key::Enter);
        let formats: Vec<Format> = [
            (Key::B, Format::Bold),
            (Key::I, Format::Italic),
            (Key::U, Format::Underline),
        ]
        .into_iter()
        .filter(|(key, _)| focused && input.consume_key(Modifiers::COMMAND, *key))
        .map(|(_, format)| format)
        .collect();
        (finish, input.key_pressed(Key::Escape), formats)
    });
    for format in formats {
        text_box.toggle(format);
    }

    text_box.fit(font_size);
    let shown = text_box.displayed_rect(font_size);
    let frame_rect = Rect::from_min_max(
        page_rect.min + shown.min.to_vec2() * zoom,
        page_rect.min + shown.max.to_vec2() * zoom,
    );
    let pad = FREE_TEXT_PADDING * zoom;
    let wrap_width = (frame_rect.width() - 2.0 * pad).max(1.0);

    let text_origin = text_origin(ui.painter(), frame_rect.min, &look);
    let text_rect = Rect::from_min_size(
        text_origin,
        vec2(
            wrap_width,
            (frame_rect.height() - 2.0 * pad).max(line_height),
        ),
    );

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

    let before = text_box.text.clone();
    let mut wrapped: Option<Arc<Galley>> = None;
    let focus_pending = std::mem::take(&mut text_box.focus_pending);
    let cursor = {
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(text_rect)
                .id_salt(id.with("edit")),
        );
        let TextBox {
            text,
            styles,
            styled_text,
            typing,
            ..
        } = text_box;
        let typing = *typing;
        let mut layouter = |ui: &Ui, buffer: &dyn egui::TextBuffer, _wrap: f32| {
            reconcile(styles, styled_text, typing, buffer.as_str());
            let chars = buffer
                .as_str()
                .chars()
                .enumerate()
                .map(|(index, c)| (c, style_at(styles, index)));
            let galley = ui
                .painter()
                .layout_job(styled_job(chars, &look, wrap_width));
            wrapped = Some(Arc::clone(&galley));
            galley
        };
        let output = TextEdit::multiline(text)
            .id(id)
            .frame(Frame::NONE)
            .margin(Margin::ZERO)
            .desired_width(wrap_width)
            .desired_rows(1)
            .layouter(&mut layouter)
            .show(&mut child);
        if focus_pending || ring.drag_stopped() || edge.drag_stopped() {
            output.response.request_focus();
        }
        output.cursor_range.map(|range| {
            let range = range.as_sorted_char_range();
            range.start.0..range.end.0
        })
    };
    let text = text_box.text.clone();
    reconcile(
        &mut text_box.styles,
        &mut text_box.styled_text,
        text_box.typing,
        &text,
    );
    if let Some(cursor) = cursor {
        let moved = cursor != text_box.selection;
        if moved && before == text_box.text && cursor.is_empty() {
            let at = cursor.start.saturating_sub(1);
            text_box.typing = style_at(&text_box.styles, at);
        }
        text_box.selection = cursor;
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
        TextBox::new(1, place, "Hello".into(), Vec::new(), None)
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
        let mut tb = TextBox::new(1, place, "Hello".into(), Vec::new(), None);
        tb.lines = vec!["Hello".into()];
        tb.fit(12.0);
        let rect = tb.pdf_rect(12.0);
        assert_eq!(tb.rotation(), 90);
        assert!((rect.y1 - rect.y0 - 150.0).abs() < 0.001);
        assert_eq!((rect.x0, rect.y0), (50.0, 100.0));
    }

    #[test]
    fn typed_characters_take_the_typing_style() {
        let bold = CharStyle {
            bold: true,
            ..CharStyle::default()
        };
        let mut styles = Vec::new();
        let mut known = String::from("ac");
        reconcile(&mut styles, &mut known, bold, "abc");
        assert_eq!(styles, [CharStyle::default(), bold, CharStyle::default()]);
        reconcile(&mut styles, &mut known, CharStyle::default(), "ac");
        assert!(styles.is_empty());
    }

    #[test]
    fn toggling_a_selection_flips_it_and_sub_replaces_super() {
        let mut styles = Vec::new();
        toggle_all(&mut styles, 4, 1..3, Format::Super);
        assert_eq!(common_style(&styles, 1..3).script, Script::Super);
        toggle_all(&mut styles, 4, 1..3, Format::Sub);
        assert_eq!(common_style(&styles, 1..3).script, Script::Sub);
        toggle_all(&mut styles, 4, 1..3, Format::Sub);
        assert!(styles.is_empty());
    }
}
