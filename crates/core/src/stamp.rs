//! Marks drawn over the pages themselves: page numbers and watermarks. Also the removal of
//! every link of a document.

use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};

use crate::annotate::{annots_entries, num, rgb, set_annots, win_ansi_hex};
use crate::helvetica::{line_width, win_ansi};
use crate::pdf::{check_page, deref, dict_entry, flatten, load, page_ids, save};
use crate::{Color, Error, Picture, Result};

/// Resource names of what a stamp draws with: unlikely to be taken by the page.
const FONT_NAME: &str = "GodPdfHelv";
const STATE_NAME: &str = "GodPdfAlpha";
const IMAGE_NAME: &str = "GodPdfMark";
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
        let ops = line_at(
            &doc,
            id,
            numbers.place,
            &text,
            numbers.font_size,
            numbers.color,
        );
        wrap(&mut doc, id, "", &ops, &[("Font", FONT_NAME, font)])?;
    }
    save(&mut doc)
}

/// Drawing `text` at `place` of `page`, upright for the reader, `NUMBER_MARGIN` from the
/// edges.
fn line_at(
    doc: &Document,
    page: ObjectId,
    place: Place,
    text: &str,
    font_size: f32,
    color: Color,
) -> String {
    let view = View::of(doc, page);
    let width = line_width(text, font_size);
    let x = match place {
        Place::TopLeft | Place::BottomLeft => NUMBER_MARGIN,
        Place::TopCenter | Place::BottomCenter => (view.width - width) / 2.0,
        Place::TopRight | Place::BottomRight => view.width - NUMBER_MARGIN - width,
    };
    let y = if place.is_top() {
        view.height - NUMBER_MARGIN - font_size * 0.7
    } else {
        NUMBER_MARGIN
    };
    format!(
        "q {} cm BT /{FONT_NAME} {} Tf {} rg {} {} Td <{}> Tj ET Q\n",
        view.matrix(),
        num(font_size),
        rgb(color),
        num(x),
        num(y),
        win_ansi_hex(text)
    )
}

/// Bates numbers: `prefix`, then a count from `start` written with at least `digits` digits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bates {
    pub prefix: String,
    pub start: u64,
    pub digits: usize,
}

impl Default for Bates {
    fn default() -> Self {
        Self {
            prefix: String::new(),
            start: 1,
            digits: 6,
        }
    }
}

impl Bates {
    /// The number of the page `index` places after the first: "ABC000012".
    pub fn number(&self, index: usize) -> String {
        let count = self.start.saturating_add(index as u64);
        format!("{}{count:0width$}", self.prefix, width = self.digits)
    }
}

/// Lines of text along the top and the bottom of pages. In their text, `{n}` is the number of
/// the page, `{total}` the number of the last one, `{date}`, `{file}` and `{bates}` what the
/// fields of the same name say.
#[derive(Debug, Clone, PartialEq)]
pub struct HeaderFooter {
    /// Left, centre and right of the top line; empty for nothing.
    pub header: [String; 3],
    /// Left, centre and right of the bottom line.
    pub footer: [String; 3],
    /// Number of the first of the pages.
    pub first: usize,
    pub date: String,
    pub file: String,
    pub bates: Bates,
    pub font_size: f32,
    pub color: Color,
}

impl Default for HeaderFooter {
    fn default() -> Self {
        Self {
            header: Default::default(),
            footer: Default::default(),
            first: 1,
            date: String::new(),
            file: String::new(),
            bates: Bates::default(),
            font_size: 10.0,
            color: PageNumbers::default().color,
        }
    }
}

impl HeaderFooter {
    /// `template` as written on the page `index` places after the first, of `count`.
    pub fn expand(&self, template: &str, index: usize, count: usize) -> String {
        template
            .replace("{n}", &(self.first + index).to_string())
            .replace("{total}", &(self.first + count - 1).to_string())
            .replace("{date}", &self.date)
            .replace("{file}", &self.file)
            .replace("{bates}", &self.bates.number(index))
    }
}

/// Write the header and footer on each of `pages`, counted in the order given.
pub fn header_footer(pdf: &[u8], pages: &[usize], marks: &HeaderFooter) -> Result<Vec<u8>> {
    check_size(marks.font_size)?;
    let slots: Vec<(Place, &String)> = [Place::TopLeft, Place::TopCenter, Place::TopRight]
        .into_iter()
        .zip(&marks.header)
        .chain(
            [Place::BottomLeft, Place::BottomCenter, Place::BottomRight]
                .into_iter()
                .zip(&marks.footer),
        )
        .filter(|(_, template)| !template.trim().is_empty())
        .collect();
    if slots.is_empty() {
        return Err(Error::InvalidArgument(
            "the header and footer have no text".into(),
        ));
    }
    let mut doc = load(pdf)?;
    let ids = targets(&mut doc, pages)?;
    let font = doc.add_object(helvetica());
    let count = ids.len();
    for (index, id) in ids.into_iter().enumerate() {
        let mut ops = String::new();
        for &(place, template) in &slots {
            let text = marks.expand(template, index, count);
            check_text(&text)?;
            ops += &line_at(&doc, id, place, &text, marks.font_size, marks.color);
        }
        wrap(&mut doc, id, "", &ops, &[("Font", FONT_NAME, font)])?;
    }
    save(&mut doc)
}

/// A picture in the middle of the page.
#[derive(Debug, Clone, PartialEq)]
pub struct PictureWatermark {
    pub picture: Picture,
    /// From 0 (invisible) to 1 (solid).
    pub opacity: f32,
    /// Part of the width and height of the page the picture may take, up to 1.
    pub scale: f32,
}

/// Draw `mark` centred on each of `pages`, over what they show, its proportions kept.
pub fn picture_watermark(pdf: &[u8], pages: &[usize], mark: &PictureWatermark) -> Result<Vec<u8>> {
    if !mark.scale.is_finite() || mark.scale <= 0.0 || mark.scale > 1.0 {
        return Err(Error::InvalidArgument("invalid watermark size".into()));
    }
    let opacity = if mark.opacity.is_finite() {
        mark.opacity.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let mut doc = load(pdf)?;
    let ids = targets(&mut doc, pages)?;
    let image = mark.picture.write(&mut doc);
    let state = doc.add_object(dictionary! {
        "Type" => "ExtGState",
        "ca" => opacity,
        "CA" => opacity,
    });
    let (pw, ph) = (mark.picture.width as f32, mark.picture.height as f32);
    for id in ids {
        let view = View::of(&doc, id);
        let fit = mark.scale * (view.width / pw).min(view.height / ph);
        let (w, h) = (pw * fit, ph * fit);
        let ops = format!(
            "q {} cm /{STATE_NAME} gs {} 0 0 {} {} {} cm /{IMAGE_NAME} Do Q\n",
            view.matrix(),
            num(w),
            num(h),
            num((view.width - w) / 2.0),
            num((view.height - h) / 2.0),
        );
        let resources = [
            ("XObject", IMAGE_NAME, image),
            ("ExtGState", STATE_NAME, state),
        ];
        wrap(&mut doc, id, "", &ops, &resources)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn blank(pages: usize) -> Vec<u8> {
        let mut pdf = crate::blank_document(400.0, 500.0).unwrap();
        for at in 1..pages {
            pdf = crate::insert_blank_page(&pdf, at, 400.0, 500.0).unwrap();
        }
        pdf
    }

    #[test]
    fn bates_numbers_are_padded() {
        let bates = Bates {
            prefix: "ACME-".to_owned(),
            start: 98,
            digits: 4,
        };
        assert_eq!(bates.number(0), "ACME-0098");
        assert_eq!(bates.number(3), "ACME-0101");
        let wide = Bates { digits: 1, ..bates };
        assert_eq!(wide.number(2), "ACME-100");
    }

    #[test]
    fn headers_and_footers_fill_in_their_placeholders() {
        let marks = HeaderFooter {
            header: ["{file}".to_owned(), String::new(), "{date}".to_owned()],
            footer: [
                String::new(),
                "Page {n} of {total}".to_owned(),
                "{bates}".to_owned(),
            ],
            first: 3,
            date: "2026-11-24".to_owned(),
            file: "report.pdf".to_owned(),
            bates: Bates {
                prefix: "X".to_owned(),
                start: 7,
                digits: 3,
            },
            ..HeaderFooter::default()
        };
        let out = header_footer(&blank(3), &[1, 2], &marks).unwrap();
        assert!(crate::extract_text(&out, 0).unwrap().trim().is_empty());
        let second = crate::extract_text(&out, 1).unwrap();
        for words in ["report.pdf", "2026-11-24", "Page 3 of 4", "X007"] {
            assert!(second.contains(words), "{words} missing from {second}");
        }
        let third = crate::extract_text(&out, 2).unwrap();
        assert!(third.contains("Page 4 of 4") && third.contains("X008"));
    }

    #[test]
    fn empty_or_unwritable_headers_are_refused() {
        let pdf = blank(1);
        assert!(header_footer(&pdf, &[0], &HeaderFooter::default()).is_err());
        let marks = HeaderFooter {
            header: ["{file}".to_owned(), String::new(), String::new()],
            file: "文書.pdf".to_owned(),
            ..HeaderFooter::default()
        };
        assert!(header_footer(&pdf, &[0], &marks).is_err());
    }

    #[test]
    fn custom_stamps_are_read_back() {
        use crate::{Annotation, Rect, StampLabel};
        let colour = Color {
            r: 0.0,
            g: 0.5,
            b: 1.0,
        };
        let rect = Rect {
            x0: 50.0,
            y0: 50.0,
            x1: 200.0,
            y1: 108.0,
        };
        let stamp = |text: &str| Annotation::Stamp {
            rect,
            label: StampLabel::Custom {
                text: text.to_owned(),
                color: colour,
            },
            date: Some("2026-11-24".to_owned()),
        };
        let out = crate::add_annotation(&blank(1), 0, &stamp("PAID")).unwrap();
        let placed = crate::annotations(&out, 0).unwrap();
        assert_eq!(placed[0].annotation.as_ref(), Some(&stamp("PAID")));
        for text in ["", "  ", "文"] {
            assert!(crate::add_annotation(&blank(1), 0, &stamp(text)).is_err());
        }
    }

    #[test]
    fn a_picture_watermark_is_centred_and_fits() {
        // 2x1 pixels, half transparent.
        let picture = Picture::rgba(2, 1, &[255, 0, 0, 128, 0, 0, 255, 128]).unwrap();
        let mark = PictureWatermark {
            picture,
            opacity: 0.5,
            scale: 0.5,
        };
        let out = picture_watermark(&blank(2), &[1], &mark).unwrap();
        let doc = load(&out).unwrap();
        let ids = page_ids(&doc);
        let drawn = String::from_utf8_lossy(&doc.get_page_content(ids[1])).into_owned();
        // Half the width of a 400x500 page, 2:1, in its middle.
        assert!(
            drawn.contains("200.000 0 0 100.000 100.000 200.000 cm /GodPdfMark Do"),
            "{drawn}"
        );
        assert!(!String::from_utf8_lossy(&doc.get_page_content(ids[0])).contains("Do"));
        let bad = PictureWatermark { scale: 0.0, ..mark };
        assert!(picture_watermark(&blank(1), &[0], &bad).is_err());
    }
}
