//! Marks drawn over the pages themselves: page numbers and watermarks. Also the removal of
//! every link of a document.

use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};

use crate::annotate::{annots_entries, num, rgb, set_annots, win_ansi_hex};
use crate::helvetica::{line_width, win_ansi};
use crate::pdf::{check_page, deref, dict_entry, flatten, load, page_ids, save};
use crate::{Color, Error, Result};

/// Resource names of what a stamp draws with: unlikely to be taken by the page.
const FONT_NAME: &str = "GodPdfHelv";
const STATE_NAME: &str = "GodPdfAlpha";
/// Distance of a page number from the edges of the page.
const NUMBER_MARGIN: f32 = 28.0;

/// Where a page number goes, on the page as it is displayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    TopLeft,
    TopCenter,
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl Place {
    pub const ALL: [Place; 6] = [
        Place::BottomCenter,
        Place::BottomLeft,
        Place::BottomRight,
        Place::TopCenter,
        Place::TopLeft,
        Place::TopRight,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::TopLeft => "Top left",
            Self::TopCenter => "Top centre",
            Self::TopRight => "Top right",
            Self::BottomLeft => "Bottom left",
            Self::BottomCenter => "Bottom centre",
            Self::BottomRight => "Bottom right",
        }
    }

    fn is_top(self) -> bool {
        matches!(self, Self::TopLeft | Self::TopCenter | Self::TopRight)
    }
}

/// How pages are numbered.
#[derive(Debug, Clone, PartialEq)]
pub struct PageNumbers {
    pub place: Place,
    /// `{n}` is the number of the page, `{total}` the number of the last one.
    pub template: String,
    /// Number of the first of `pages`.
    pub first: usize,
    pub font_size: f32,
    pub color: Color,
}

impl Default for PageNumbers {
    fn default() -> Self {
        Self {
            place: Place::BottomCenter,
            template: "{n}".to_owned(),
            first: 1,
            font_size: 10.0,
            color: Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
            },
        }
    }
}

/// A line of text across the page.
#[derive(Debug, Clone, PartialEq)]
pub struct Watermark {
    pub text: String,
    pub font_size: f32,
    pub color: Color,
    /// From 0 (invisible) to 1 (solid).
    pub opacity: f32,
    /// Counter-clockwise, in degrees.
    pub angle: f32,
    /// From the centre of the page as displayed: right and up, in points.
    pub offset: (f32, f32),
}

impl Default for Watermark {
    fn default() -> Self {
        Self {
            text: String::new(),
            font_size: 64.0,
            color: Color {
                r: 0.5,
                g: 0.5,
                b: 0.5,
            },
            opacity: 0.3,
            angle: 45.0,
            offset: (0.0, 0.0),
        }
    }
}

/// Write its number on each of `pages`, counted in the order given from `numbers.first`.
pub fn number_pages(pdf: &[u8], pages: &[usize], numbers: &PageNumbers) -> Result<Vec<u8>> {
    check_size(numbers.font_size)?;
    let mut doc = load(pdf)?;
    let ids = targets(&mut doc, pages)?;
    let font = doc.add_object(helvetica());
    let total = numbers.first + ids.len() - 1;
    for (index, id) in ids.into_iter().enumerate() {
        let text = numbers
            .template
            .replace("{n}", &(numbers.first + index).to_string())
            .replace("{total}", &total.to_string());
        check_text(&text)?;
        let view = View::of(&doc, id);
        let width = line_width(&text, numbers.font_size);
        let x = match numbers.place {
            Place::TopLeft | Place::BottomLeft => NUMBER_MARGIN,
            Place::TopCenter | Place::BottomCenter => (view.width - width) / 2.0,
            Place::TopRight | Place::BottomRight => view.width - NUMBER_MARGIN - width,
        };
        let y = if numbers.place.is_top() {
            view.height - NUMBER_MARGIN - numbers.font_size * 0.7
        } else {
            NUMBER_MARGIN
        };
        let ops = format!(
            "q {} cm BT /{FONT_NAME} {} Tf {} rg {} {} Td <{}> Tj ET Q\n",
            view.matrix(),
            num(numbers.font_size),
            rgb(numbers.color),
            num(x),
            num(y),
            win_ansi_hex(&text)
        );
        wrap(&mut doc, id, "", &ops, &[("Font", FONT_NAME, font)])?;
    }
    save(&mut doc)
}

/// Draw `mark` across each of `pages`, over what they show.
pub fn watermark(pdf: &[u8], pages: &[usize], mark: &Watermark) -> Result<Vec<u8>> {
    check_size(mark.font_size)?;
    if mark.text.trim().is_empty() {
        return Err(Error::InvalidArgument("the watermark has no text".into()));
    }
    check_text(&mark.text)?;
    if !mark.angle.is_finite() || !mark.offset.0.is_finite() || !mark.offset.1.is_finite() {
        return Err(Error::InvalidArgument("invalid watermark position".into()));
    }
    let opacity = if mark.opacity.is_finite() {
        mark.opacity.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let mut doc = load(pdf)?;
    let ids = targets(&mut doc, pages)?;
    let font = doc.add_object(helvetica());
    let state = doc.add_object(dictionary! {
        "Type" => "ExtGState",
        "ca" => opacity,
        "CA" => opacity,
    });
    let (sin, cos) = mark.angle.to_radians().sin_cos();
    let width = line_width(&mark.text, mark.font_size);
    for id in ids {
        let view = View::of(&doc, id);
        let ops = format!(
            "q {} cm /{STATE_NAME} gs {} {} {} {} {} {} cm BT /{FONT_NAME} {} Tf {} rg {} {} Td <{}> Tj ET Q\n",
            view.matrix(),
            num(cos),
            num(sin),
            num(-sin),
            num(cos),
            num(view.width / 2.0 + mark.offset.0),
            num(view.height / 2.0 + mark.offset.1),
            num(mark.font_size),
            rgb(mark.color),
            num(-width / 2.0),
            num(-mark.font_size * 0.35),
            win_ansi_hex(&mark.text)
        );
        let resources = [("Font", FONT_NAME, font), ("ExtGState", STATE_NAME, state)];
        wrap(&mut doc, id, "", &ops, &resources)?;
    }
    save(&mut doc)
}

/// Take every link out of the document. The words and pictures they were on stay.
pub fn remove_links(pdf: &[u8]) -> Result<Vec<u8>> {
    let mut doc = load(pdf)?;
    let mut removed = 0;
    for id in page_ids(&doc) {
        let entries = annots_entries(&doc, id)?;
        let kept: Vec<Object> = entries
            .iter()
            .filter(|entry| !is_link(&doc, entry))
            .cloned()
            .collect();
        if kept.len() != entries.len() {
            removed += entries.len() - kept.len();
            set_annots(&mut doc, id, kept)?;
        }
    }
    if removed == 0 {
        return Err(Error::InvalidArgument("the document has no links".into()));
    }
    save(&mut doc)
}

fn is_link(doc: &Document, annot: &Object) -> bool {
    deref(doc, annot)
        .and_then(|annot| annot.as_dict().ok())
        .and_then(|annot| annot.get(b"Subtype").ok())
        .and_then(|subtype| subtype.as_name().ok())
        == Some(b"Link".as_slice())
}

pub(crate) fn helvetica() -> Dictionary {
    dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    }
}

fn check_size(font_size: f32) -> Result<()> {
    if font_size.is_finite() && (1.0..=1000.0).contains(&font_size) {
        Ok(())
    } else {
        Err(Error::InvalidArgument(format!(
            "invalid text size {font_size}"
        )))
    }
}

/// Helvetica draws Latin letters only: say so, never write a `?` in their place.
pub(crate) fn check_text(text: &str) -> Result<()> {
    match text.chars().find(|&c| c != '?' && win_ansi(c) == b'?') {
        Some(c) => Err(Error::InvalidArgument(format!(
            "the character “{c}” cannot be written in Helvetica"
        ))),
        None => Ok(()),
    }
}

/// The ids of `pages`, each with its own resources and boxes.
pub(crate) fn targets(doc: &mut Document, pages: &[usize]) -> Result<Vec<ObjectId>> {
    if pages.is_empty() {
        return Err(Error::InvalidArgument("no pages selected".into()));
    }
    let (_, ids) = flatten(doc)?;
    pages
        .iter()
        .map(|&page| check_page(page, ids.len()).map(|()| ids[page]))
        .collect()
}

/// The visible box of a page and its rotation: where "bottom left" is for the reader.
pub(crate) struct View {
    /// Box in page space: x0, y0, x1, y1.
    pub(crate) corners: [f32; 4],
    rotation: i64,
    /// Size as displayed.
    pub(crate) width: f32,
    pub(crate) height: f32,
}

impl View {
    pub(crate) fn of(doc: &Document, page: ObjectId) -> Self {
        let dict = doc.get_dictionary(page).ok();
        let read = |key: &[u8]| {
            let numbers: Vec<f32> = deref(doc, dict?.get(key).ok()?)?
                .as_array()
                .ok()?
                .iter()
                .filter_map(|n| deref(doc, n)?.as_float().ok())
                .collect();
            match numbers[..] {
                [a, b, c, d] => Some([a.min(c), b.min(d), a.max(c), b.max(d)]),
                _ => None,
            }
        };
        let media = read(b"MediaBox").unwrap_or([0.0, 0.0, 612.0, 792.0]);
        // Viewers show the crop box, within the media box.
        let corners = read(b"CropBox").map_or(media, |crop| {
            [
                crop[0].max(media[0]),
                crop[1].max(media[1]),
                crop[2].min(media[2]),
                crop[3].min(media[3]),
            ]
        });
        let rotation = dict
            .and_then(|dict| dict.get(b"Rotate").ok())
            .and_then(|rotate| deref(doc, rotate)?.as_i64().ok())
            .map_or(0, |degrees| degrees.rem_euclid(360) / 90 * 90);
        let (w, h) = (corners[2] - corners[0], corners[3] - corners[1]);
        let (width, height) = if rotation % 180 == 0 { (w, h) } else { (h, w) };
        Self {
            corners,
            rotation,
            width,
            height,
        }
    }

    /// From the page as displayed (origin bottom left, y up) to page space.
    pub(crate) fn to_page(&self) -> [f32; 6] {
        let [x0, y0, x1, y1] = self.corners;
        match self.rotation {
            90 => [0.0, 1.0, -1.0, 0.0, x1, y0],
            180 => [-1.0, 0.0, 0.0, -1.0, x1, y1],
            270 => [0.0, -1.0, 1.0, 0.0, x0, y1],
            _ => [1.0, 0.0, 0.0, 1.0, x0, y0],
        }
    }

    /// [`Self::to_page`] as `cm` operands.
    pub(crate) fn matrix(&self) -> String {
        let [a, b, c, d, e, f] = self.to_page();
        format!("{a} {b} {c} {d} {} {}", num(e), num(f))
    }
}

/// Draw `under` beneath what the page shows and `over` above it, adding `resources` (kind,
/// name, object) to the page. What the page draws is closed in `q … Q`, so whatever state it
/// leaves behind does not move the marks.
pub(crate) fn wrap(
    doc: &mut Document,
    page: ObjectId,
    under: &str,
    over: &str,
    resources: &[(&str, &str, ObjectId)],
) -> Result<()> {
    let dict = doc.get_dictionary(page)?;
    let mut page_resources = dict_entry(doc, dict, b"Resources")
        .cloned()
        .unwrap_or_default();
    for &(kind, name, id) in resources {
        let mut entries = dict_entry(doc, &page_resources, kind.as_bytes())
            .cloned()
            .unwrap_or_default();
        entries.set(name, id);
        page_resources.set(kind, entries);
    }
    let existing: Vec<Object> = match dict.get(b"Contents").ok() {
        Some(Object::Array(items)) => items.clone(),
        Some(Object::Reference(id)) => match doc.get_object(*id) {
            Ok(Object::Array(items)) => items.clone(),
            _ => vec![Object::Reference(*id)],
        },
        _ => Vec::new(),
    };
    let open = doc.add_object(Stream::new(
        Dictionary::new(),
        format!(
            "{under}q
"
        )
        .into_bytes(),
    ));
    let close = doc.add_object(Stream::new(
        Dictionary::new(),
        format!(
            "
Q
{over}"
        )
        .into_bytes(),
    ));
    let mut contents = vec![Object::Reference(open)];
    contents.extend(existing);
    contents.push(Object::Reference(close));
    let dict = doc.get_dictionary_mut(page)?;
    dict.set("Resources", page_resources);
    dict.set("Contents", contents);
    Ok(())
}
