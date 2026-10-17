//! Text that cannot be seen, laid over the words of a scanned page: the page can then be
//! searched and its words selected and copied.

use std::collections::BTreeMap;

use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};

use crate::annotate::num;
use crate::pdf::{load, save};
use crate::stamp::{View, targets, wrap};
use crate::{Error, Result};

/// Width of every character of the layer, in thousandths of the font size.
const ADVANCE: f32 = 500.0;
/// `bfchar` entries per block of a ToUnicode map (the format allows at most 100).
const CMAP_BLOCK: usize = 100;

/// A word found on a page: its text and its box (left, top, right, bottom) in points of the
/// page as displayed, origin top left.
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub text: String,
    pub rect: [f32; 4],
}

/// Lay invisible `words` over each `(page, words)`. Any language: the words are written
/// with a font that draws nothing and maps each code to its Unicode character.
pub fn add_text_layer(pdf: &[u8], pages: &[(usize, Vec<Word>)]) -> Result<Vec<u8>> {
    if pages.iter().all(|(_, words)| words.is_empty()) {
        return Err(Error::InvalidArgument("no text was found".into()));
    }
    let mut codes = BTreeMap::new();
    for (_, words) in pages {
        for c in words.iter().flat_map(|word| word.text.chars()).chain([' ']) {
            let next = codes.len() + 1;
            codes.entry(c).or_insert(next as u16);
        }
    }
    if codes.len() >= usize::from(u16::MAX) {
        return Err(Error::InvalidArgument(
            "too many different characters".into(),
        ));
    }
    let mut doc = load(pdf)?;
    let indices: Vec<usize> = pages.iter().map(|(page, _)| *page).collect();
    let ids = targets(&mut doc, &indices)?;
    let font = unicode_font(&mut doc, &codes);
    for (id, (_, words)) in ids.into_iter().zip(pages) {
        let ops = layer_ops(&doc, id, words, &codes);
        if !ops.is_empty() {
            wrap(&mut doc, id, "", &ops, &[("Font", "GodPdfOcr", font)])?;
        }
    }
    save(&mut doc)
}

/// Text in render mode 3 (neither filled nor stroked), each word stretched over its box.
fn layer_ops(
    doc: &Document,
    page: ObjectId,
    words: &[Word],
    codes: &BTreeMap<char, u16>,
) -> String {
    let view = View::of(doc, page);
    let mut ops = format!(
        "q {} cm BT 3 Tr
",
        view.matrix()
    );
    let mut any = false;
    for word in words {
        let text = word.text.trim();
        let [left, top, right, bottom] = word.rect;
        let (width, height) = (right - left, bottom - top);
        if text.is_empty() || width <= 0.5 || height <= 0.5 {
            continue;
        }
        let size = height * 0.9;
        let natural = (text.chars().count() as f32 * ADVANCE / 1000.0 * size).max(0.1);
        let stretch = (width / natural * 100.0).clamp(1.0, 1000.0);
        // Baseline a fifth of the box above its bottom edge, y measured upwards.
        let baseline = view.height - bottom + height * 0.2;
        // The space keeps the words apart when the text is copied.
        let hex: String = text
            .chars()
            .chain([' '])
            .map(|c| format!("{:04X}", codes[&c]))
            .collect();
        ops += &format!(
            "/GodPdfOcr {} Tf {} Tz 1 0 0 1 {} {} Tm <{hex}> Tj
",
            num(size),
            num(stretch),
            num(left),
            num(baseline),
        );
        any = true;
    }
    if !any {
        return String::new();
    }
    ops + "ET Q
"
}

/// A two-byte font whose codes are `codes` and that draws nothing of its own.
fn unicode_font(doc: &mut Document, codes: &BTreeMap<char, u16>) -> ObjectId {
    let mut entries: Vec<(u16, char)> = codes.iter().map(|(&c, &code)| (code, c)).collect();
    entries.sort_unstable();
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def
         /CMapName /Adobe-Identity-UCS def
/CMapType 2 def
         1 begincodespacerange
<0000> <FFFF>
endcodespacerange
",
    );
    for block in entries.chunks(CMAP_BLOCK) {
        cmap += &format!(
            "{} beginbfchar
",
            block.len()
        );
        for &(code, c) in block {
            let mut units = [0u16; 2];
            let utf16: String = c
                .encode_utf16(&mut units)
                .iter()
                .map(|unit| format!("{unit:04X}"))
                .collect();
            cmap += &format!(
                "<{code:04X}> <{utf16}>
"
            );
        }
        cmap += "endbfchar
";
    }
    cmap += "endcmap
CMapName currentdict /CMap defineresource pop
end
end
";
    let to_unicode = doc.add_object(Stream::new(Dictionary::new(), cmap.into_bytes()));
    let descriptor = doc.add_object(dictionary! {
        "Type" => "FontDescriptor",
        "FontName" => "GodPdfOcrText",
        "Flags" => 4,
        "FontBBox" => vec![0.into(), Object::Integer(-200), 1000.into(), 800.into()],
        "ItalicAngle" => 0,
        "Ascent" => 800,
        "Descent" => -200,
        "CapHeight" => 700,
        "StemV" => 80,
    });
    let descendant = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "CIDFontType2",
        "BaseFont" => "GodPdfOcrText",
        "CIDSystemInfo" => dictionary! {
            "Registry" => Object::string_literal("Adobe"),
            "Ordering" => Object::string_literal("Identity"),
            "Supplement" => 0,
        },
        "FontDescriptor" => descriptor,
        "DW" => ADVANCE as i64,
        "CIDToGIDMap" => "Identity",
    });
    doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type0",
        "BaseFont" => "GodPdfOcrText",
        "Encoding" => "Identity-H",
        "DescendantFonts" => vec![Object::Reference(descendant)],
        "ToUnicode" => to_unicode,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_layer_is_refused() {
        let pdf = crate::blank_document(200.0, 200.0).unwrap();
        assert!(add_text_layer(&pdf, &[(0, Vec::new())]).is_err());
    }

    #[test]
    fn words_become_searchable_text() {
        let pdf = crate::blank_document(400.0, 300.0).unwrap();
        let words = vec![
            Word {
                text: "Été".into(),
                rect: [150.0, 20.0, 190.0, 40.0],
            },
            Word {
                text: "Привет".into(),
                rect: [200.0, 20.0, 260.0, 40.0],
            },
            Word {
                text: "Hello".into(),
                rect: [20.0, 20.0, 80.0, 40.0],
            },
            Word {
                text: "scan".into(),
                rect: [90.0, 20.0, 140.0, 40.0],
            },
        ];
        let out = add_text_layer(&pdf, &[(0, words)]).unwrap();
        let text = crate::extract_text(&out, 0).unwrap();
        assert!(text.contains("Hello") && text.contains("scan"), "{text}");
        assert!(text.contains("Été") && text.contains("Привет"), "{text}");
        assert_eq!(crate::search(&out, "scan").unwrap().len(), 1);
    }
}
