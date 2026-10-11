use lopdf::{
    Dictionary, Document, Object, ObjectId, Stream, decode_text_string, dictionary, text_string,
};

use crate::pdf::{check_page, deref, dict_entry, load, page_ids, save};
use crate::{Error, Result};

/// Rectangle in PDF points, unrotated page space (origin bottom-left).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

/// RGB, each 0.0..=1.0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

/// Annotation kinds. Each is written with an appearance stream (/AP) so every viewer shows it.
#[derive(Debug, Clone, PartialEq)]
pub enum Annotation {
    Highlight {
        rect: Rect,
        color: Color,
    },
    Note {
        rect: Rect,
        text: String,
    },
    FreeText {
        rect: Rect,
        text: String,
        font_size: f32,
        color: Color,
    },
    Square {
        rect: Rect,
        color: Color,
        width: f32,
    },
    /// Each stroke is a polyline of (x, y) points.
    Ink {
        strokes: Vec<Vec<(f32, f32)>>,
        color: Color,
        width: f32,
    },
}

impl Rect {
    /// This rectangle moved by (`dx`, `dy`) points.
    pub fn translated(self, dx: f32, dy: f32) -> Self {
        Self {
            x0: self.x0 + dx,
            y0: self.y0 + dy,
            x1: self.x1 + dx,
            y1: self.y1 + dy,
        }
    }
}

impl Annotation {
    /// The same annotation moved by (`dx`, `dy`) points: its rectangle, or every ink point.
    pub fn translated(&self, dx: f32, dy: f32) -> Self {
        let mut moved = self.clone();
        match &mut moved {
            Self::Highlight { rect, .. }
            | Self::Note { rect, .. }
            | Self::FreeText { rect, .. }
            | Self::Square { rect, .. } => *rect = rect.translated(dx, dy),
            Self::Ink { strokes, .. } => {
                for point in strokes.iter_mut().flatten() {
                    *point = (point.0 + dx, point.1 + dy);
                }
            }
        }
        moved
    }

    /// The same annotation stretched from the box `from` (its current /Rect) onto `to`:
    /// rectangles become `to`, ink points are mapped proportionally. A free-text box only
    /// takes the new position and width; its height always follows its wrapped text.
    pub fn fitted(&self, from: Rect, to: Rect) -> Self {
        let mut fitted = self.clone();
        match &mut fitted {
            Self::Highlight { rect, .. } | Self::Note { rect, .. } | Self::Square { rect, .. } => {
                *rect = to;
            }
            Self::FreeText {
                rect,
                text,
                font_size,
                ..
            } => {
                let lines = wrap_free_text(text, *font_size, to.x1 - to.x0).len();
                *rect = Rect {
                    y0: to.y1 - free_text_height(lines, *font_size),
                    ..to
                };
            }
            Self::Ink { strokes, .. } => {
                let scale = |value: f32, from0: f32, from1: f32, to0: f32, to1: f32| {
                    let span = from1 - from0;
                    if span.abs() < f32::EPSILON {
                        (to0 + to1) / 2.0
                    } else {
                        to0 + (value - from0) / span * (to1 - to0)
                    }
                };
                for point in strokes.iter_mut().flatten() {
                    *point = (
                        scale(point.0, from.x0, from.x1, to.x0, to.x1),
                        scale(point.1, from.y0, from.y1, to.y0, to.y1),
                    );
                }
            }
        }
        fitted
    }

    /// Short human name of the kind ("Highlight", "Sticky note", ...).
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Highlight { .. } => "Highlight",
            Self::Note { .. } => "Sticky note",
            Self::FreeText { .. } => "Text",
            Self::Square { .. } => "Rectangle",
            Self::Ink { .. } => "Drawing",
        }
    }
}

/// Written into /T (author) of every annotation.
const AUTHOR: &str = "GodPDF";
/// /F bit 3: print the annotation.
const PRINT_FLAG: i64 = 4;
const NOTE_COLOR: Color = Color {
    r: 1.0,
    g: 0.85,
    b: 0.2,
};
const HIGHLIGHT_OPACITY: f32 = 0.4;
/// Free-text layout, shared with the app's editor so typed text never moves on commit.
/// Inner padding between the /Rect edge and the text, in points.
pub const FREE_TEXT_PADDING: f32 = 2.0;
/// Baseline-to-baseline distance, in font sizes.
pub const FREE_TEXT_LINE_HEIGHT: f32 = 1.2;
/// First baseline below the top padding, in font sizes. Helvetica's ascender is 0.718 em and
/// its descender 0.207 em; 0.85 centres that 0.925 em ink box in the 1.2 em line.
pub const FREE_TEXT_ASCENT: f32 = 0.85;

/// /Rect of a free-text annotation whose top-left corner is `top_left` (PDF space, points):
/// as wide as the longest line in Helvetica plus padding, one line height per line.
pub fn free_text_rect(top_left: (f32, f32), text: &str, font_size: f32) -> Rect {
    let lines = text.split('\n');
    let count = lines.clone().count().max(1) as f32;
    let width = lines
        .map(|line| crate::helvetica::line_width(line, font_size))
        .fold(0.0, f32::max);
    let (x0, y1) = top_left;
    Rect {
        x0,
        y0: y1 - count * font_size * FREE_TEXT_LINE_HEIGHT - 2.0 * FREE_TEXT_PADDING,
        x1: x0 + width + 2.0 * FREE_TEXT_PADDING,
        y1,
    }
}

/// Break `text` into the lines a free-text box `width` points wide shows (padding included):
/// explicit line breaks are kept, long lines wrap at spaces, and a word wider than the box is
/// split. Measured in Helvetica, the font the annotation is drawn with.
pub fn wrap_free_text(text: &str, font_size: f32, width: f32) -> Vec<String> {
    // Tolerance so a line measured to fit exactly is not pushed over by rounding.
    let room = (width - 2.0 * FREE_TEXT_PADDING).max(0.0) + 0.01;
    let fits = |line: &str| crate::helvetica::line_width(line, font_size) <= room;
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split(' ') {
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if fits(&candidate) {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            // A single word wider than the box: split it by characters.
            for ch in word.chars() {
                let mut longer = line.clone();
                longer.push(ch);
                if !line.is_empty() && !fits(&longer) {
                    lines.push(std::mem::replace(&mut line, ch.to_string()));
                } else {
                    line = longer;
                }
            }
        }
        lines.push(line);
    }
    lines
}

/// Height of a free-text box showing `lines` lines.
fn free_text_height(lines: usize, font_size: f32) -> f32 {
    lines.max(1) as f32 * font_size * FREE_TEXT_LINE_HEIGHT + 2.0 * FREE_TEXT_PADDING
}

/// Append `annotation` to the /Annots of `page`.
pub fn add_annotation(pdf: &[u8], page: usize, annotation: &Annotation) -> Result<Vec<u8>> {
    add_annotations(pdf, page, std::slice::from_ref(annotation))
}

/// Append several annotations to `page` in one pass (e.g. one highlight per text line).
pub fn add_annotations(pdf: &[u8], page: usize, annotations: &[Annotation]) -> Result<Vec<u8>> {
    let mut doc = load(pdf)?;
    let page_id = page_id(&doc, page)?;
    for annotation in annotations {
        let parts = annotation_parts(annotation)?;
        let annot_id = doc.new_object_id();
        let dict = annotation_dict(&mut doc, page_id, annot_id, parts);
        doc.objects.insert(annot_id, Object::Dictionary(dict));
        append_to_annots(&mut doc, page_id, annot_id)?;
    }
    save(&mut doc)
}

/// Kind-specific pieces of an annotation.
struct Parts {
    subtype: &'static str,
    rect: Rect,
    color: Option<Color>,
    extra: Dictionary,
    /// Appearance content stream, drawn in page space (BBox = Rect).
    appearance: String,
    resources: Dictionary,
}

fn annotation_parts(annotation: &Annotation) -> Result<Parts> {
    match annotation {
        Annotation::Highlight { rect, color } => Ok(highlight(normalized(*rect)?, *color)),
        Annotation::Note { rect, text } => Ok(note(normalized(*rect)?, text)),
        Annotation::FreeText {
            rect,
            text,
            font_size,
            color,
        } => free_text(normalized(*rect)?, text, *font_size, *color),
        Annotation::Square { rect, color, width } => square(normalized(*rect)?, *color, *width),
        Annotation::Ink {
            strokes,
            color,
            width,
        } => ink(strokes, *color, *width),
    }
}

fn annotation_dict(
    doc: &mut Document,
    page_id: ObjectId,
    annot_id: ObjectId,
    parts: Parts,
) -> Dictionary {
    let mut dict = dictionary! {
        "Type" => "Annot",
        "Subtype" => parts.subtype,
        "Rect" => rect_array(parts.rect),
        "F" => PRINT_FLAG,
        "NM" => Object::string_literal(unique_name(annot_id)),
        "T" => text_string(AUTHOR),
        "P" => page_id,
    };
    if let Some(color) = parts.color {
        dict.set("C", color_array(color));
    }
    if let Some(date) = pdf_date() {
        dict.set("M", Object::string_literal(date));
    }
    dict.extend(&parts.extra);
    let form = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Form",
        "BBox" => rect_array(parts.rect),
        "Resources" => parts.resources,
    };
    let appearance = doc.add_object(Stream::new(form, parts.appearance.into_bytes()));
    dict.set("AP", dictionary! { "N" => appearance });
    dict
}

/// Translucent filled rectangle, multiplied onto the page.
fn highlight(rect: Rect, color: Color) -> Parts {
    let Rect { x0, y0, x1, y1 } = rect;
    let quad: Vec<Object> = [x0, y1, x1, y1, x0, y0, x1, y0]
        .into_iter()
        .map(Object::from)
        .collect();
    let gstate = dictionary! {
        "Type" => "ExtGState",
        "BM" => "Multiply",
        "ca" => HIGHLIGHT_OPACITY,
        "CA" => HIGHLIGHT_OPACITY,
    };
    Parts {
        subtype: "Highlight",
        rect,
        color: Some(color),
        extra: dictionary! { "QuadPoints" => quad },
        appearance: format!("/GS0 gs {} rg {} re f", rgb(color), rect_ops(rect)),
        resources: dictionary! { "ExtGState" => dictionary! { "GS0" => gstate } },
    }
}

/// Sticky note: yellow sheet with three text lines.
fn note(rect: Rect, text: &str) -> Parts {
    let (width, height) = (rect.x1 - rect.x0, rect.y1 - rect.y0);
    let mut appearance = format!(
        "1 w 0.25 0.25 0.25 RG {} rg {} re B",
        rgb(NOTE_COLOR),
        rect_ops(inset(rect, 0.5))
    );
    for line in 1..=3u8 {
        let y = rect.y1 - height * f32::from(line) / 4.0;
        let (left, right) = (rect.x0 + width * 0.2, rect.x1 - width * 0.2);
        appearance += &format!(" {} {} m {} {} l S", num(left), num(y), num(right), num(y));
    }
    Parts {
        subtype: "Text",
        rect,
        color: Some(NOTE_COLOR),
        extra: dictionary! { "Contents" => contents_string(text), "Name" => "Comment", "Open" => false },
        appearance,
        resources: Dictionary::new(),
    }
}

/// Helvetica text wrapped to the rectangle's width. The rectangle keeps its top edge and
/// width; its height is set to fit the wrapped lines, so the text never leaves it.
fn free_text(rect: Rect, text: &str, font_size: f32, color: Color) -> Result<Parts> {
    if !(font_size.is_finite() && font_size > 0.0) {
        return Err(Error::InvalidArgument(format!(
            "invalid font size {font_size}"
        )));
    }
    let lines = wrap_free_text(text, font_size, rect.x1 - rect.x0);
    let rect = Rect {
        y0: rect.y1 - free_text_height(lines.len(), font_size),
        ..rect
    };
    let size = num(font_size);
    let mut appearance = format!(
        "q {} re W n BT /Helv {size} Tf {} rg {} TL {} {} Td",
        rect_ops(rect),
        rgb(color),
        num(font_size * FREE_TEXT_LINE_HEIGHT),
        num(rect.x0 + FREE_TEXT_PADDING),
        num(rect.y1 - FREE_TEXT_PADDING - FREE_TEXT_ASCENT * font_size),
    );
    for (index, line) in lines.iter().enumerate() {
        let advance = if index == 0 { "" } else { " T*" };
        appearance += &format!("{advance} <{}> Tj", win_ansi_hex(line));
    }
    appearance += " ET Q";
    let font = dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    };
    Ok(Parts {
        subtype: "FreeText",
        rect,
        // /C would paint a background in some viewers; the text colour lives in /DA.
        color: None,
        extra: dictionary! {
            "Contents" => contents_string(text),
            "DA" => Object::string_literal(format!("/Helv {size} Tf {} rg", rgb(color))),
        },
        appearance,
        resources: dictionary! { "Font" => dictionary! { "Helv" => font } },
    })
}

/// Stroked rectangle, kept inside `rect`.
fn square(rect: Rect, color: Color, width: f32) -> Result<Parts> {
    check_width(width)?;
    Ok(Parts {
        subtype: "Square",
        rect,
        color: Some(color),
        extra: dictionary! { "BS" => border_style(width) },
        appearance: format!(
            "{} w {} RG {} re S",
            num(width),
            rgb(color),
            rect_ops(inset(rect, width / 2.0))
        ),
        resources: Dictionary::new(),
    })
}

/// Stroked polylines with round caps and joins.
fn ink(strokes: &[Vec<(f32, f32)>], color: Color, width: f32) -> Result<Parts> {
    check_width(width)?;
    if strokes.is_empty() || strokes.iter().any(Vec::is_empty) {
        return Err(Error::InvalidArgument(
            "ink needs at least one point per stroke".into(),
        ));
    }
    let points = || strokes.iter().flatten();
    if points().any(|&(x, y)| !(x.is_finite() && y.is_finite())) {
        return Err(Error::InvalidArgument("ink point is not finite".into()));
    }
    let margin = width / 2.0 + 1.0;
    let rect = Rect {
        x0: points().map(|p| p.0).fold(f32::INFINITY, f32::min) - margin,
        y0: points().map(|p| p.1).fold(f32::INFINITY, f32::min) - margin,
        x1: points().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max) + margin,
        y1: points().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max) + margin,
    };
    let mut appearance = format!("{} w 1 J 1 j {} RG", num(width), rgb(color));
    for stroke in strokes {
        for (index, &(x, y)) in stroke.iter().enumerate() {
            appearance += &format!(
                " {} {} {}",
                num(x),
                num(y),
                if index == 0 { "m" } else { "l" }
            );
        }
        if let [(x, y)] = stroke.as_slice() {
            // A lone point still shows as a dot thanks to the round cap.
            appearance += &format!(" {} {} l", num(*x), num(*y));
        }
        appearance += " S";
    }
    let ink_list: Vec<Object> = strokes
        .iter()
        .map(|stroke| {
            Object::Array(
                stroke
                    .iter()
                    .flat_map(|&(x, y)| [x.into(), y.into()])
                    .collect(),
            )
        })
        .collect();
    Ok(Parts {
        subtype: "Ink",
        rect,
        color: Some(color),
        extra: dictionary! { "InkList" => ink_list, "BS" => border_style(width) },
        appearance,
        resources: Dictionary::new(),
    })
}

/// Push onto /Annots, whether it is inline, indirect or missing.
fn append_to_annots(doc: &mut Document, page_id: ObjectId, annot_id: ObjectId) -> Result<()> {
    let existing = doc.get_dictionary(page_id)?.get(b"Annots").ok().cloned();
    match existing {
        Some(Object::Reference(array_id))
            if doc.get_object(array_id).is_ok_and(|o| o.as_array().is_ok()) =>
        {
            doc.get_object_mut(array_id)?
                .as_array_mut()?
                .push(annot_id.into());
        }
        Some(Object::Array(mut items)) => {
            items.push(annot_id.into());
            doc.get_dictionary_mut(page_id)?.set("Annots", items);
        }
        _ => doc
            .get_dictionary_mut(page_id)?
            .set("Annots", vec![Object::from(annot_id)]),
    }
    Ok(())
}

/// Ordered corners (x0 <= x1, y0 <= y1); rejects non-finite values.
fn normalized(rect: Rect) -> Result<Rect> {
    if ![rect.x0, rect.y0, rect.x1, rect.y1]
        .iter()
        .all(|v| v.is_finite())
    {
        return Err(Error::InvalidArgument("rectangle is not finite".into()));
    }
    Ok(Rect {
        x0: rect.x0.min(rect.x1),
        y0: rect.y0.min(rect.y1),
        x1: rect.x0.max(rect.x1),
        y1: rect.y0.max(rect.y1),
    })
}

fn check_width(width: f32) -> Result<()> {
    if width.is_finite() && width >= 0.0 {
        Ok(())
    } else {
        Err(Error::InvalidArgument(format!(
            "invalid line width {width}"
        )))
    }
}

fn inset(rect: Rect, by: f32) -> Rect {
    Rect {
        x0: rect.x0 + by,
        y0: rect.y0 + by,
        x1: rect.x1 - by,
        y1: rect.y1 - by,
    }
}

fn border_style(width: f32) -> Dictionary {
    dictionary! { "Type" => "Border", "W" => width, "S" => "S" }
}

fn rect_array(rect: Rect) -> Vec<Object> {
    vec![
        rect.x0.into(),
        rect.y0.into(),
        rect.x1.into(),
        rect.y1.into(),
    ]
}

fn color_array(color: Color) -> Vec<Object> {
    vec![color.r.into(), color.g.into(), color.b.into()]
}

/// `x y w h` operands for the `re` operator.
fn rect_ops(rect: Rect) -> String {
    format!(
        "{} {} {} {}",
        num(rect.x0),
        num(rect.y0),
        num(rect.x1 - rect.x0),
        num(rect.y1 - rect.y0)
    )
}

fn rgb(color: Color) -> String {
    format!("{} {} {}", num(color.r), num(color.g), num(color.b))
}

fn num(value: f32) -> String {
    format!("{value:.3}")
}

/// Hex string body in WinAnsi; unmappable characters become '?'.
fn win_ansi_hex(text: &str) -> String {
    text.chars()
        .map(|c| format!("{:02X}", crate::helvetica::win_ansi(c)))
        .collect()
}

fn unique_name(annot_id: ObjectId) -> String {
    let salt = getrandom::u64().unwrap_or(0);
    format!("godpdf-{}-{salt:016x}", annot_id.0)
}

/// Current UTC time as a PDF date string.
#[cfg(not(target_arch = "wasm32"))]
fn pdf_date() -> Option<String> {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let secs = i64::try_from(secs).ok()?;
    let (year, month, day) = civil_from_days(secs.div_euclid(86_400));
    let time = secs.rem_euclid(86_400);
    let (hour, minute, second) = (time / 3600, time % 3600 / 60, time % 60);
    Some(format!(
        "D:{year:04}{month:02}{day:02}{hour:02}{minute:02}{second:02}Z"
    ))
}

/// wasm32-unknown-unknown has no clock without JS glue, so /M is omitted there.
#[cfg(target_arch = "wasm32")]
fn pdf_date() -> Option<String> {
    None
}

/// Days since 1970-01-01 to (year, month, day). Howard Hinnant's algorithm.
#[cfg(not(target_arch = "wasm32"))]
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// An annotation already on a page.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedAnnotation {
    /// Stable id: the /NM entry, or `"#<index in /Annots>"` when absent.
    pub id: String,
    /// Bounding /Rect, unrotated page space.
    pub rect: Rect,
    /// Parsed back when it is one of our kinds (any Highlight/Text/FreeText/Square/Ink
    /// with enough data); `None` for other subtypes (Link, Widget, ...).
    pub annotation: Option<Annotation>,
}

/// Annotations of `page`, in /Annots order.
pub fn annotations(pdf: &[u8], page: usize) -> Result<Vec<PlacedAnnotation>> {
    let doc = load(pdf)?;
    let page_id = page_id(&doc, page)?;
    let placed = annots_entries(&doc, page_id)?
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let dict = deref(&doc, entry)?.as_dict().ok()?;
            Some(PlacedAnnotation {
                id: annotation_id(dict, index),
                rect: read_rect(&doc, dict).unwrap_or(Rect {
                    x0: 0.0,
                    y0: 0.0,
                    x1: 0.0,
                    y1: 0.0,
                }),
                annotation: parse_annotation(&doc, dict),
            })
        })
        .collect();
    Ok(placed)
}

/// Remove the annotation `id` from `page`. `Error::InvalidArgument` if no such id.
pub fn remove_annotation(pdf: &[u8], page: usize, id: &str) -> Result<Vec<u8>> {
    let mut doc = load(pdf)?;
    let page_id = page_id(&doc, page)?;
    let mut entries = annots_entries(&doc, page_id)?;
    let index = find_annotation(&doc, &entries, id)?;
    let removed = entries.remove(index);
    // Its popup would otherwise keep it alive through /Parent.
    let popup = deref(&doc, &removed)
        .and_then(|o| o.as_dict().ok())
        .and_then(|d| d.get(b"Popup").ok())
        .and_then(|p| p.as_reference().ok());
    if let Some(popup) = popup {
        entries.retain(|e| e.as_reference().ok() != Some(popup));
    }
    set_annots(&mut doc, page_id, entries)?;
    save(&mut doc)
}

/// Replace annotation `id` on `page` with `annotation` (new /AP), keeping its /Annots position
/// and its id. `Error::InvalidArgument` if no such id.
pub fn replace_annotation(
    pdf: &[u8],
    page: usize,
    id: &str,
    annotation: &Annotation,
) -> Result<Vec<u8>> {
    let mut doc = load(pdf)?;
    let page_id = page_id(&doc, page)?;
    let mut entries = annots_entries(&doc, page_id)?;
    let index = find_annotation(&doc, &entries, id)?;
    let parts = annotation_parts(annotation)?;
    let old = deref(&doc, &entries[index])
        .and_then(|o| o.as_dict().ok())
        .cloned()
        .unwrap_or_default();
    // Reuse the object number so a popup's /Parent keeps pointing at us.
    let annot_id = entries[index]
        .as_reference()
        .unwrap_or_else(|_| doc.new_object_id());
    let mut dict = annotation_dict(&mut doc, page_id, annot_id, parts);
    match old.get(b"NM") {
        Ok(name) => dict.set("NM", name.clone()),
        // A synthetic `#<index>` id stays valid only without /NM.
        Err(_) => {
            dict.remove(b"NM");
        }
    }
    if let Ok(popup) = old.get(b"Popup") {
        dict.set("Popup", popup.clone());
    }
    doc.objects.insert(annot_id, Object::Dictionary(dict));
    entries[index] = annot_id.into();
    set_annots(&mut doc, page_id, entries)?;
    save(&mut doc)
}

fn page_id(doc: &Document, page: usize) -> Result<ObjectId> {
    let pages = page_ids(doc);
    check_page(page, pages.len())?;
    Ok(pages[page])
}

/// The /Annots items of `page_id` (empty when absent or malformed).
fn annots_entries(doc: &Document, page_id: ObjectId) -> Result<Vec<Object>> {
    let page = doc.get_dictionary(page_id)?;
    Ok(page
        .get(b"Annots")
        .ok()
        .and_then(|annots| deref(doc, annots))
        .and_then(|annots| annots.as_array().ok())
        .cloned()
        .unwrap_or_default())
}

/// Write back /Annots, into the indirect array when there is one.
fn set_annots(doc: &mut Document, page_id: ObjectId, entries: Vec<Object>) -> Result<()> {
    let existing = doc.get_dictionary(page_id)?.get(b"Annots").ok().cloned();
    match existing {
        Some(Object::Reference(array_id))
            if doc.get_object(array_id).is_ok_and(|o| o.as_array().is_ok()) =>
        {
            *doc.get_object_mut(array_id)?.as_array_mut()? = entries;
        }
        _ => doc.get_dictionary_mut(page_id)?.set("Annots", entries),
    }
    Ok(())
}

fn find_annotation(doc: &Document, entries: &[Object], id: &str) -> Result<usize> {
    entries
        .iter()
        .enumerate()
        .position(|(index, entry)| {
            deref(doc, entry)
                .and_then(|o| o.as_dict().ok())
                .is_some_and(|dict| annotation_id(dict, index) == id)
        })
        .ok_or_else(|| Error::InvalidArgument(format!("no annotation {id}")))
}

/// /NM as text, else `#<index>`.
fn annotation_id(dict: &Dictionary, index: usize) -> String {
    dict.get(b"NM")
        .ok()
        .and_then(|name| decode_text_string(name).ok())
        .unwrap_or_else(|| format!("#{index}"))
}

fn number(doc: &Document, object: &Object) -> Option<f32> {
    deref(doc, object)?
        .as_float()
        .ok()
        .filter(|v| v.is_finite())
}

fn numbers_of(doc: &Document, object: &Object) -> Option<Vec<f32>> {
    deref(doc, object)?
        .as_array()
        .ok()?
        .iter()
        .map(|item| number(doc, item))
        .collect()
}

/// Numbers of the array at `key`.
fn numbers(doc: &Document, dict: &Dictionary, key: &[u8]) -> Option<Vec<f32>> {
    numbers_of(doc, dict.get(key).ok()?)
}

fn read_rect(doc: &Document, dict: &Dictionary) -> Option<Rect> {
    match numbers(doc, dict, b"Rect")?.as_slice() {
        &[x0, y0, x1, y1] => normalized(Rect { x0, y0, x1, y1 }).ok(),
        _ => None,
    }
}

fn read_color(doc: &Document, dict: &Dictionary) -> Option<Color> {
    match numbers(doc, dict, b"C")?.as_slice() {
        &[r, g, b] => Some(Color { r, g, b }),
        _ => None,
    }
}

/// /BS /W, else the third /Border entry, else 1.
fn read_width(doc: &Document, dict: &Dictionary) -> f32 {
    let from_bs = dict_entry(doc, dict, b"BS")
        .and_then(|bs| bs.get(b"W").ok())
        .and_then(|w| number(doc, w));
    let from_border = || numbers(doc, dict, b"Border")?.get(2).copied();
    from_bs.or_else(from_border).unwrap_or(1.0)
}

/// /Contents text string. Text with line breaks or tabs is written as UTF-16BE: readers
/// (lopdf included) drop control characters from PDFDocEncoding strings.
fn contents_string(text: &str) -> Object {
    if !text.contains(|c: char| c.is_control()) {
        return text_string(text);
    }
    let mut bytes = vec![0xFE, 0xFF];
    bytes.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
    Object::String(bytes, lopdf::StringFormat::Hexadecimal)
}

fn read_text(doc: &Document, dict: &Dictionary) -> String {
    dict.get(b"Contents")
        .ok()
        .and_then(|contents| deref(doc, contents))
        .and_then(|contents| decode_text_string(contents).ok())
        .unwrap_or_default()
}

fn read_strokes(doc: &Document, dict: &Dictionary) -> Option<Vec<Vec<(f32, f32)>>> {
    let list = deref(doc, dict.get(b"InkList").ok()?)?.as_array().ok()?;
    let strokes = list
        .iter()
        .map(|stroke| {
            let values = numbers_of(doc, stroke)?;
            let points: Vec<(f32, f32)> = values.chunks_exact(2).map(|p| (p[0], p[1])).collect();
            (!points.is_empty()).then_some(points)
        })
        .collect::<Option<Vec<_>>>()?;
    (!strokes.is_empty()).then_some(strokes)
}

fn parse_annotation(doc: &Document, dict: &Dictionary) -> Option<Annotation> {
    match dict.get(b"Subtype").ok()?.as_name().ok()? {
        b"Highlight" => Some(Annotation::Highlight {
            rect: read_rect(doc, dict)?,
            color: read_color(doc, dict)?,
        }),
        b"Text" => Some(Annotation::Note {
            rect: read_rect(doc, dict)?,
            text: read_text(doc, dict),
        }),
        b"FreeText" => {
            let (font_size, color) = parse_da(doc, dict);
            Some(Annotation::FreeText {
                rect: read_rect(doc, dict)?,
                text: read_text(doc, dict),
                font_size,
                color,
            })
        }
        b"Square" => Some(Annotation::Square {
            rect: read_rect(doc, dict)?,
            color: read_color(doc, dict)?,
            width: read_width(doc, dict),
        }),
        b"Ink" => Some(Annotation::Ink {
            strokes: read_strokes(doc, dict)?,
            color: read_color(doc, dict)?,
            width: read_width(doc, dict),
        }),
        _ => None,
    }
}

const DEFAULT_FONT_SIZE: f32 = 12.0;
const BLACK: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
};

/// Font size (`Tf`) and fill colour (`rg` or `g`) from /DA; 12 pt black when absent.
fn parse_da(doc: &Document, dict: &Dictionary) -> (f32, Color) {
    let da = dict
        .get(b"DA")
        .ok()
        .and_then(|da| deref(doc, da))
        .and_then(|da| da.as_str().ok())
        .map(|da| String::from_utf8_lossy(da).into_owned())
        .unwrap_or_default();
    let tokens: Vec<&str> = da.split_whitespace().collect();
    let operand = |at: usize, back: usize| -> Option<f32> {
        tokens
            .get(at.checked_sub(back)?)?
            .parse::<f32>()
            .ok()
            .filter(|v| v.is_finite())
    };
    let (mut size, mut color) = (DEFAULT_FONT_SIZE, BLACK);
    for (at, token) in tokens.iter().enumerate() {
        match *token {
            "Tf" => {
                if let Some(value) = operand(at, 1).filter(|v| *v > 0.0) {
                    size = value;
                }
            }
            "rg" => {
                if let (Some(r), Some(g), Some(b)) =
                    (operand(at, 3), operand(at, 2), operand(at, 1))
                {
                    color = Color { r, g, b };
                }
            }
            "g" => {
                if let Some(gray) = operand(at, 1) {
                    color = Color {
                        r: gray,
                        g: gray,
                        b: gray,
                    };
                }
            }
            _ => {}
        }
    }
    (size, color)
}
