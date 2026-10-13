//! New documents made from what is not a PDF: a picture or plain text.

use lopdf::{Dictionary, Document, Object, Stream, dictionary};

use crate::annotate::{FREE_TEXT_PADDING, num, win_ansi_hex, wrap_free_text};
use crate::pdf::save;
use crate::stamp::check_text;
use crate::{Error, Result};

/// A4, in points.
const A4: (f32, f32) = (595.0, 842.0);
/// Screen pixels to points: 96 pixels to the inch.
const PIXEL: f32 = 0.75;
const TEXT_MARGIN: f32 = 56.0;
const TEXT_SIZE: f32 = 11.0;
const TEXT_LEADING: f32 = 15.0;

/// One page showing a JPEG, as it is in the file: it is not decoded nor compressed again.
/// A photo taken with the camera turned is shown upright.
pub fn jpeg_document(jpeg: &[u8]) -> Result<Vec<u8>> {
    let info =
        jpeg_info(jpeg).ok_or_else(|| Error::InvalidArgument("not a readable JPEG".into()))?;
    let mut image = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Image",
        "Width" => i64::from(info.width),
        "Height" => i64::from(info.height),
        "BitsPerComponent" => 8,
        "Filter" => "DCTDecode",
    };
    match info.components {
        1 => image.set("ColorSpace", "DeviceGray"),
        3 => image.set("ColorSpace", "DeviceRGB"),
        4 => {
            // The four-colour JPEGs of Adobe programs hold their inks inverted.
            image.set("ColorSpace", "DeviceCMYK");
            image.set(
                "Decode",
                [1, 0, 1, 0, 1, 0, 1, 0].map(Object::Integer).to_vec(),
            );
        }
        other => {
            return Err(Error::InvalidArgument(format!(
                "a JPEG of {other} colour channels cannot be shown"
            )));
        }
    }
    image_document(
        Stream::new(image, jpeg.to_vec()),
        info.width,
        info.height,
        info.rotation,
    )
}

/// One page showing a picture given as rows of red, green and blue bytes, top row first.
pub fn rgb_document(width: u32, height: u32, rgb: &[u8]) -> Result<Vec<u8>> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(3));
    if width == 0 || height == 0 || expected != Some(rgb.len()) {
        return Err(Error::InvalidArgument(format!(
            "{} bytes are not a picture of {width}x{height}",
            rgb.len()
        )));
    }
    let image = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Image",
        "Width" => i64::from(width),
        "Height" => i64::from(height),
        "BitsPerComponent" => 8,
        "ColorSpace" => "DeviceRGB",
    };
    image_document(Stream::new(image, rgb.to_vec()), width, height, 0)
}

/// Plain text on A4 pages, in Helvetica: long lines go on the next one.
pub fn text_document(text: &str) -> Result<Vec<u8>> {
    let text = text.replace("\r\n", "\n").replace('\t', "    ");
    let text = text.trim_start_matches('\u{FEFF}').trim_end();
    check_text(&text.replace('\n', " "))?;
    let width = A4.0 - 2.0 * TEXT_MARGIN + 2.0 * FREE_TEXT_PADDING;
    let lines = wrap_free_text(text, TEXT_SIZE, width);
    let per_page = ((A4.1 - 2.0 * TEXT_MARGIN) / TEXT_LEADING) as usize;
    let mut builder = Builder::new();
    let font = builder.doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    // An empty file still makes a page.
    let chunks: Vec<&[String]> = if lines.is_empty() {
        vec![&[]]
    } else {
        lines.chunks(per_page).collect()
    };
    for chunk in chunks {
        let mut ops = format!(
            "BT /F1 {} Tf {} TL {} {} Td\n",
            num(TEXT_SIZE),
            num(TEXT_LEADING),
            num(TEXT_MARGIN),
            num(A4.1 - TEXT_MARGIN - TEXT_SIZE)
        );
        for line in chunk {
            ops.push_str(&format!("<{}> Tj T*\n", win_ansi_hex(line)));
        }
        ops.push_str("ET\n");
        let resources = dictionary! { "Font" => dictionary! { "F1" => font } };
        builder.page(A4, 0, resources, ops.into_bytes());
    }
    builder.finish()
}

/// A page the size of the picture, or A4 when the picture is larger than that.
fn image_document(image: Stream, width: u32, height: u32, rotation: i64) -> Result<Vec<u8>> {
    let (mut w, mut h) = (width as f32 * PIXEL, height as f32 * PIXEL);
    let (max_w, max_h) = if w > h { (A4.1, A4.0) } else { A4 };
    let shrink = (max_w / w).min(max_h / h).min(1.0);
    w *= shrink;
    h *= shrink;
    let mut builder = Builder::new();
    let image = builder.doc.add_object(image);
    let resources = dictionary! { "XObject" => dictionary! { "Im0" => image } };
    let ops = format!("q {} 0 0 {} 0 0 cm /Im0 Do Q\n", num(w), num(h));
    builder.page((w, h), rotation, resources, ops.into_bytes());
    builder.finish()
}

/// A document being made, page after page.
struct Builder {
    doc: Document,
    root: lopdf::ObjectId,
    pages: Vec<Object>,
}

impl Builder {
    fn new() -> Self {
        let mut doc = Document::with_version("1.7");
        let root = doc.new_object_id();
        Self {
            doc,
            root,
            pages: Vec::new(),
        }
    }

    fn page(&mut self, size: (f32, f32), rotation: i64, resources: Dictionary, ops: Vec<u8>) {
        let contents = self.doc.add_object(Stream::new(Dictionary::new(), ops));
        let mut page = dictionary! {
            "Type" => "Page",
            "Parent" => self.root,
            "MediaBox" => vec![0.into(), 0.into(), size.0.into(), size.1.into()],
            "Contents" => contents,
            "Resources" => resources,
        };
        if rotation != 0 {
            page.set("Rotate", rotation);
        }
        self.pages
            .push(Object::Reference(self.doc.add_object(page)));
    }

    fn finish(mut self) -> Result<Vec<u8>> {
        let count = self.pages.len() as i64;
        let pages = dictionary! { "Type" => "Pages", "Kids" => self.pages, "Count" => count };
        self.doc
            .objects
            .insert(self.root, Object::Dictionary(pages));
        let catalog = self
            .doc
            .add_object(dictionary! { "Type" => "Catalog", "Pages" => self.root });
        self.doc.trailer.set("Root", catalog);
        save(&mut self.doc)
    }
}

struct JpegInfo {
    width: u32,
    height: u32,
    components: u8,
    /// Quarter turns, clockwise, that show the photo upright: 0, 90, 180 or 270.
    rotation: i64,
}

/// Size and channels from the frame header of a JPEG, and the turn its EXIF data asks for.
fn jpeg_info(jpeg: &[u8]) -> Option<JpegInfo> {
    if !jpeg.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut rotation = 0;
    let mut at = 2;
    loop {
        let marker = *jpeg.get(at + 1)?;
        if *jpeg.get(at)? != 0xFF {
            return None;
        }
        // Fill bytes and markers without a length.
        if marker == 0xFF {
            at += 1;
            continue;
        }
        if matches!(marker, 0x01 | 0xD0..=0xD8) {
            at += 2;
            continue;
        }
        let length = usize::from(u16::from_be_bytes([*jpeg.get(at + 2)?, *jpeg.get(at + 3)?]));
        let segment = jpeg.get(at + 4..at + 2 + length)?;
        match marker {
            // Start of frame, of any kind but the tables that share the range.
            0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => {
                let height = u16::from_be_bytes([*segment.get(1)?, *segment.get(2)?]);
                let width = u16::from_be_bytes([*segment.get(3)?, *segment.get(4)?]);
                if width == 0 || height == 0 {
                    return None;
                }
                return Some(JpegInfo {
                    width: u32::from(width),
                    height: u32::from(height),
                    components: *segment.get(5)?,
                    rotation,
                });
            }
            0xE1 => rotation = exif_rotation(segment).unwrap_or(rotation),
            // Start of scan: the frame header comes before it or not at all.
            0xDA => return None,
            _ => {}
        }
        at += 2 + length;
    }
}

/// The orientation tag of an EXIF segment, as the turn that undoes it.
fn exif_rotation(segment: &[u8]) -> Option<i64> {
    let tiff = segment.strip_prefix(b"Exif\0\0")?;
    let big = match tiff.get(..2)? {
        b"MM" => true,
        b"II" => false,
        _ => return None,
    };
    let u16_at = |at: usize| {
        let bytes = [*tiff.get(at)?, *tiff.get(at + 1)?];
        Some(if big {
            u16::from_be_bytes(bytes)
        } else {
            u16::from_le_bytes(bytes)
        })
    };
    let u32_at = |at: usize| {
        let bytes: [u8; 4] = tiff.get(at..at + 4)?.try_into().ok()?;
        Some(if big {
            u32::from_be_bytes(bytes)
        } else {
            u32::from_le_bytes(bytes)
        })
    };
    let directory = u32_at(4)? as usize;
    for entry in 0..usize::from(u16_at(directory)?) {
        let at = directory + 2 + entry * 12;
        if u16_at(at)? == 0x0112 {
            return Some(match u16_at(at + 8)? {
                3 => 180,
                6 => 90,
                8 => 270,
                _ => 0,
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The headers of a JPEG of `width`x`height`, with an EXIF orientation when given.
    fn jpeg(width: u16, height: u16, orientation: Option<u16>) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xD8];
        if let Some(orientation) = orientation {
            let mut exif = b"Exif\0\0MM\0\x2A\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01".to_vec();
            exif.extend(orientation.to_be_bytes());
            exif.extend([0, 0, 0, 0, 0, 0]);
            bytes.extend([0xFF, 0xE1]);
            bytes.extend((exif.len() as u16 + 2).to_be_bytes());
            bytes.extend(exif);
        }
        bytes.extend([0xFF, 0xC0, 0, 11, 8]);
        bytes.extend(height.to_be_bytes());
        bytes.extend(width.to_be_bytes());
        bytes.extend([3, 0, 0, 0]);
        bytes
    }

    #[test]
    fn the_frame_header_gives_the_size() {
        let info = jpeg_info(&jpeg(640, 480, None)).unwrap();
        assert_eq!((info.width, info.height, info.components), (640, 480, 3));
        assert_eq!(info.rotation, 0);
    }

    #[test]
    fn a_turned_photo_is_shown_upright() {
        assert_eq!(jpeg_info(&jpeg(640, 480, Some(6))).unwrap().rotation, 90);
        assert_eq!(jpeg_info(&jpeg(640, 480, Some(8))).unwrap().rotation, 270);
        assert_eq!(jpeg_info(&jpeg(640, 480, Some(1))).unwrap().rotation, 0);
    }

    #[test]
    fn what_is_not_a_jpeg_is_refused() {
        assert!(jpeg_info(b"\x89PNG").is_none());
        assert!(jpeg_info(&[0xFF, 0xD8, 0xFF]).is_none());
        assert!(jpeg_document(b"hello").is_err());
    }
}
