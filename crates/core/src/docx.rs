//! Export to Word: a `.docx` package (WordprocessingML in a zip) written by hand, with the
//! text of each page as paragraphs and, when asked, its pictures.

use std::collections::HashSet;
use std::io::Write as _;

use flate2::Compression;
use flate2::write::DeflateEncoder;

use crate::images::{decode, drawn_size, page_images};
use crate::pdf::{load, page_ids};
use crate::text::page_text;
use crate::{Error, Result};

/// EMU (the unit of Word drawings) in a point.
const EMU_PER_POINT: f32 = 12_700.0;
/// Pictures are at most as wide as the text: A4 less 2.5 cm margins, in points.
const MAX_WIDTH: f32 = 595.0 - 2.0 * 72.0;

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="jpeg" ContentType="image/jpeg"/><Default Extension="png" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#;

const PACKAGE_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" w:eastAsia="Calibri" w:cs="Calibri"/><w:sz w:val="22"/><w:szCs w:val="22"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="0" w:line="264" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style></w:styles>"#;

/// The text of every page of `pdf` as a Word document: one paragraph per line, a page break
/// between pages, and with `pictures` the pictures each page draws (those that can be
/// decoded), under its text. Scanned pages without a text layer come out empty.
pub fn export_docx(pdf: &[u8], pictures: bool) -> Result<Vec<u8>> {
    let doc = load(pdf)?;
    let mut body = String::new();
    let mut media: Vec<(String, Vec<u8>)> = Vec::new();
    let pages = page_ids(&doc);
    for (index, &page) in pages.iter().enumerate() {
        if index > 0 {
            body.push_str(r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"#);
        }
        for line in page_text(&doc, index).lines() {
            body.push_str(&paragraph(line));
        }
        if !pictures {
            continue;
        }
        let mut seen = HashSet::new();
        for (id, matrix) in page_images(&doc, page) {
            if !seen.insert(id) {
                continue;
            }
            let Some((extension, bytes)) = picture_file(&doc, id) else {
                continue;
            };
            let (width, height) = drawn_size(matrix);
            if width < 1.0 || height < 1.0 {
                continue;
            }
            let shrink = (MAX_WIDTH / width).min(1.0);
            let number = media.len() + 1;
            body.push_str(&drawing(number, width * shrink, height * shrink));
            media.push((format!("image{number}.{extension}"), bytes));
        }
    }
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><w:body>{body}<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr></w:body></w:document>"#
    );
    let mut rels = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdStyles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#,
    );
    for (index, (name, _)) in media.iter().enumerate() {
        rels.push_str(&format!(
            r#"<Relationship Id="rIdImage{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/{name}"/>"#,
            index + 1
        ));
    }
    rels.push_str("</Relationships>");

    let mut zip = Zip::default();
    zip.add("[Content_Types].xml", CONTENT_TYPES.as_bytes())?;
    zip.add("_rels/.rels", PACKAGE_RELS.as_bytes())?;
    zip.add("word/document.xml", document.as_bytes())?;
    zip.add("word/styles.xml", STYLES.as_bytes())?;
    zip.add("word/_rels/document.xml.rels", rels.as_bytes())?;
    for (name, bytes) in &media {
        zip.add(&format!("word/media/{name}"), bytes)?;
    }
    zip.finish()
}

/// One line of text as a paragraph; an empty line stays as an empty paragraph.
fn paragraph(line: &str) -> String {
    let text = escape(line.trim_end());
    if text.is_empty() {
        return "<w:p/>".to_owned();
    }
    format!(r#"<w:p><w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p>"#)
}

/// A picture in a paragraph of its own, `width` by `height` points.
fn drawing(number: usize, width: f32, height: f32) -> String {
    let (cx, cy) = (
        (width * EMU_PER_POINT).round() as i64,
        (height * EMU_PER_POINT).round() as i64,
    );
    format!(
        r#"<w:p><w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="{cx}" cy="{cy}"/><wp:docPr id="{number}" name="Picture {number}"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="{number}" name="Picture {number}"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rIdImage{number}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#
    )
}

/// Picture `id` as a file: a JPEG as it is stored, anything else decodable as PNG.
fn picture_file(doc: &lopdf::Document, id: lopdf::ObjectId) -> Option<(&'static str, Vec<u8>)> {
    let stream = doc.get_object(id).ok()?.as_stream().ok()?;
    let decoded = decode(doc, stream)?;
    if decoded.jpeg {
        return Some(("jpeg", stream.content.clone()));
    }
    let mut png = std::io::Cursor::new(Vec::new());
    decoded
        .pixels
        .write_to(&mut png, image::ImageFormat::Png)
        .ok()?;
    Some(("png", png.into_inner()))
}

/// `text` as XML character data: markup characters escaped, characters XML forbids dropped.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' => out.push(' '),
            c if c < ' ' || matches!(c, '\u{FFFE}' | '\u{FFFF}') => {}
            c => out.push(c),
        }
    }
    out
}

/// A zip archive being written: deflated entries, no zip64 (parts under 4 GB).
#[derive(Default)]
struct Zip {
    out: Vec<u8>,
    central: Vec<u8>,
    entries: u16,
}

impl Zip {
    fn add(&mut self, name: &str, bytes: &[u8]) -> Result<()> {
        let too_large = || Error::InvalidArgument("the document is too large for Word".into());
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(bytes)
            .map_err(|err| Error::Pdf(err.to_string()))?;
        let packed = encoder
            .finish()
            .map_err(|err| Error::Pdf(err.to_string()))?;
        let mut crc = flate2::Crc::new();
        crc.update(bytes);
        let offset = u32::try_from(self.out.len()).map_err(|_| too_large())?;
        let size = u32::try_from(bytes.len()).map_err(|_| too_large())?;
        let packed_size = u32::try_from(packed.len()).map_err(|_| too_large())?;
        let name_len = name.len() as u16;
        // Version 2.0, no flags, deflate, 1980-01-01 00:00.
        let common = |header: &mut Vec<u8>| {
            for half in [20u16, 0, 8, 0, 0x21] {
                header.extend(half.to_le_bytes());
            }
            for word in [crc.sum(), packed_size, size] {
                header.extend(word.to_le_bytes());
            }
            header.extend(name_len.to_le_bytes());
            header.extend(0u16.to_le_bytes());
        };
        self.out.extend(0x0403_4b50u32.to_le_bytes());
        common(&mut self.out);
        self.out.extend(name.as_bytes());
        self.out.extend(&packed);

        self.central.extend(0x0201_4b50u32.to_le_bytes());
        self.central.extend(20u16.to_le_bytes());
        common(&mut self.central);
        // Comment length, disk, internal and external attributes, then the offset.
        self.central.extend([0; 2 + 2 + 2 + 4]);
        self.central.extend(offset.to_le_bytes());
        self.central.extend(name.as_bytes());
        self.entries = self.entries.checked_add(1).ok_or_else(too_large)?;
        Ok(())
    }

    fn finish(mut self) -> Result<Vec<u8>> {
        let too_large = || Error::InvalidArgument("the document is too large for Word".into());
        let start = u32::try_from(self.out.len()).map_err(|_| too_large())?;
        let size = u32::try_from(self.central.len()).map_err(|_| too_large())?;
        self.out.extend(&self.central);
        self.out.extend(0x0605_4b50u32.to_le_bytes());
        self.out.extend([0; 4]);
        self.out.extend(self.entries.to_le_bytes());
        self.out.extend(self.entries.to_le_bytes());
        self.out.extend(size.to_le_bytes());
        self.out.extend(start.to_le_bytes());
        self.out.extend(0u16.to_le_bytes());
        Ok(self.out)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::Read as _;

    use super::*;

    /// Name and content of every entry, read from the central directory.
    fn unzip(zip: &[u8]) -> HashMap<String, Vec<u8>> {
        let u16_at = |at: usize| usize::from(u16::from_le_bytes([zip[at], zip[at + 1]]));
        let u32_at = |at: usize| u32::from_le_bytes(zip[at..at + 4].try_into().unwrap()) as usize;
        let end = zip.len() - 22;
        assert_eq!(u32_at(end), 0x0605_4b50);
        let mut at = u32_at(end + 16);
        let mut files = HashMap::new();
        for _ in 0..u16_at(end + 10) {
            assert_eq!(u32_at(at), 0x0201_4b50);
            let (crc, packed, size) = (u32_at(at + 16), u32_at(at + 20), u32_at(at + 24));
            let name_len = u16_at(at + 28);
            let name = String::from_utf8(zip[at + 46..at + 46 + name_len].to_vec()).unwrap();
            let local = u32_at(at + 42);
            let data = local + 30 + u16_at(local + 26) + u16_at(local + 28);
            let mut bytes = Vec::new();
            flate2::read::DeflateDecoder::new(&zip[data..data + packed])
                .read_to_end(&mut bytes)
                .unwrap();
            assert_eq!(bytes.len(), size);
            let mut check = flate2::Crc::new();
            check.update(&bytes);
            assert_eq!(check.sum() as usize, crc);
            files.insert(name, bytes);
            at += 46 + name_len;
        }
        files
    }

    #[test]
    fn the_package_holds_its_parts_and_well_formed_escaped_xml() {
        let pdf = crate::text_document("Fish & chips <cheap>\n\nSecond line").unwrap();
        let docx = export_docx(&pdf, true).unwrap();
        let files = unzip(&docx);
        for part in [
            "[Content_Types].xml",
            "_rels/.rels",
            "word/document.xml",
            "word/styles.xml",
            "word/_rels/document.xml.rels",
        ] {
            let xml = std::str::from_utf8(&files[part]).unwrap();
            roxmltree::Document::parse(xml).unwrap_or_else(|err| panic!("{part}: {err}"));
        }
        let document = std::str::from_utf8(&files["word/document.xml"]).unwrap();
        assert!(
            document.contains("Fish &amp; chips &lt;cheap&gt;"),
            "{document}"
        );
        let tree = roxmltree::Document::parse(document).unwrap();
        let texts: Vec<&str> = tree
            .descendants()
            .filter(|node| node.tag_name().name() == "t")
            .filter_map(|node| node.text())
            .collect();
        assert_eq!(texts, ["Fish & chips <cheap>", "Second line"]);
    }

    #[test]
    fn pages_are_separated_and_pictures_embedded() {
        let pdf = crate::rgb_document(2, 2, &[255; 12]).unwrap();
        let two = crate::merge(&[pdf.as_slice(), pdf.as_slice()]).unwrap();
        let files = unzip(&export_docx(&two, true).unwrap());
        let document = std::str::from_utf8(&files["word/document.xml"]).unwrap();
        assert_eq!(document.matches(r#"w:type="page""#).count(), 1);
        assert!(files.contains_key("word/media/image1.png"));
        assert!(files.contains_key("word/media/image2.png"));
        let rels = std::str::from_utf8(&files["word/_rels/document.xml.rels"]).unwrap();
        assert!(rels.contains(r#"Id="rIdImage2""#));

        let without = unzip(&export_docx(&two, false).unwrap());
        assert!(!without.keys().any(|name| name.starts_with("word/media")));
    }

    #[test]
    fn control_characters_are_dropped() {
        assert_eq!(escape("a\u{1}b\t'\""), "ab &apos;&quot;");
    }
}
