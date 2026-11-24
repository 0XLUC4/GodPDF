//! PDF/A-2b, best effort: what a document needs to be archived (ISO 19005-2), as far as
//! it can be added or taken out. What cannot be fixed (fonts not embedded, CMYK colours)
//! is reported instead.

use std::collections::BTreeSet;

use lopdf::content::Content;
use lopdf::{
    Dictionary, Document, Object, ObjectId, Stream, StringFormat, dictionary, text_string,
};

use crate::Result;
use crate::annotate::{annots_entries, pdf_date, set_annots};
use crate::docx::escape;
use crate::info::{info_dict, text_entry};
use crate::pdf::{deref, dict_entry, load, page_ids, save};
use crate::retype::descendant;

/// A document converted to PDF/A-2b, and what could not be made to conform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfA {
    pub bytes: Vec<u8>,
    /// One sentence for each problem left.
    pub warnings: Vec<String>,
}

/// Actions PDF/A-2 allows; `Named` ones only to move between pages.
const ALLOWED_ACTIONS: [&[u8]; 6] = [
    b"GoTo",
    b"GoToR",
    b"GoToE",
    b"Thread",
    b"URI",
    b"SubmitForm",
];
const ALLOWED_NAMED: [&[u8]; 4] = [b"NextPage", b"PrevPage", b"FirstPage", b"LastPage"];
/// Annotations PDF/A-2 forbids (multimedia) or that carry files.
const FORBIDDEN_ANNOTS: [&[u8]; 6] = [
    b"FileAttachment",
    b"Sound",
    b"Movie",
    b"Screen",
    b"3D",
    b"RichMedia",
];
/// Annotation flags: invisible, hidden, no view; print.
const HIDDEN_FLAGS: i64 = 1 | 2 | 32;
const PRINT_FLAG: i64 = 4;
const SRGB: &str = "sRGB IEC61966-2.1";

/// Convert to PDF/A-2b: sRGB output intent with its ICC profile, XMP metadata matching the
/// Info dictionary, a document ID; scripts, forbidden actions, multimedia and attached files
/// are taken out. Fonts are not embedded: those missing are listed in the warnings.
pub fn convert_to_pdfa(pdf: &[u8]) -> Result<PdfA> {
    let mut doc = load(pdf)?;
    doc.version = "1.7".into();
    doc.trailer.remove(b"Encrypt");
    remove_actions(&mut doc);
    let missing_appearances = clean_annotations(&mut doc)?;
    clean_catalog(&mut doc)?;
    clean_streams(&mut doc);
    ensure_id(&mut doc);
    write_xmp(&mut doc)?;
    add_output_intent(&mut doc)?;
    let mut warnings = check(&doc);
    if missing_appearances > 0 {
        warnings.push(format!(
            "{missing_appearances} comment(s) or form field(s) have no appearance of their own"
        ));
    }
    Ok(PdfA {
        bytes: save(&mut doc)?,
        warnings,
    })
}

fn allowed_action(doc: &Document, action: &Object) -> bool {
    // A destination (array or name) instead of an action is fine.
    let Some(dict) = deref(doc, action).and_then(|a| a.as_dict().ok()) else {
        return true;
    };
    match dict.get(b"S").and_then(Object::as_name) {
        Ok(b"Named") => dict
            .get(b"N")
            .and_then(Object::as_name)
            .is_ok_and(|name| ALLOWED_NAMED.contains(&name)),
        Ok(kind) => ALLOWED_ACTIONS.contains(&kind),
        Err(_) => true,
    }
}

fn holder_mut(object: &mut Object) -> Option<&mut Dictionary> {
    match object {
        Object::Dictionary(dict) => Some(dict),
        Object::Stream(stream) => Some(&mut stream.dict),
        _ => None,
    }
}

/// Additional actions (`AA`) go everywhere; `A` and `OpenAction` go when not allowed.
fn remove_actions(doc: &mut Document) {
    let mut forbidden: Vec<(ObjectId, &[u8])> = Vec::new();
    for (&id, object) in &doc.objects {
        let dict = match object {
            Object::Dictionary(dict) => dict,
            Object::Stream(stream) => &stream.dict,
            _ => continue,
        };
        for key in [b"A".as_slice(), b"OpenAction"] {
            if dict
                .get(key)
                .is_ok_and(|action| !allowed_action(doc, action))
            {
                forbidden.push((id, key));
            }
        }
    }
    for object in doc.objects.values_mut() {
        if let Some(dict) = holder_mut(object) {
            dict.remove(b"AA");
        }
    }
    for (id, key) in forbidden {
        if let Some(dict) = doc.objects.get_mut(&id).and_then(holder_mut) {
            dict.remove(key);
        }
    }
}

/// Take out forbidden and hidden annotations, make the others printable. Returns how many
/// lack an appearance stream.
fn clean_annotations(doc: &mut Document) -> Result<usize> {
    let mut missing = 0;
    for page in page_ids(doc) {
        let entries = annots_entries(doc, page)?;
        let kept: Vec<Object> = entries
            .iter()
            .filter(|annot| {
                let Some(dict) = deref(doc, annot).and_then(|a| a.as_dict().ok()) else {
                    return false;
                };
                let subtype = dict
                    .get(b"Subtype")
                    .and_then(Object::as_name)
                    .unwrap_or(b"");
                let flags = dict.get(b"F").and_then(Object::as_i64).unwrap_or(0);
                !FORBIDDEN_ANNOTS.contains(&subtype) && flags & HIDDEN_FLAGS == 0
            })
            .cloned()
            .collect();
        if kept.len() != entries.len() {
            set_annots(doc, page, kept.clone())?;
        }
        for annot in kept {
            let Object::Reference(id) = annot else {
                continue;
            };
            let Ok(dict) = doc.get_dictionary_mut(id) else {
                continue;
            };
            let subtype = dict
                .get(b"Subtype")
                .and_then(Object::as_name)
                .unwrap_or(b"");
            if subtype == b"Popup" {
                continue;
            }
            if subtype != b"Link" && !dict.has(b"AP") {
                missing += 1;
            }
            let flags = dict.get(b"F").and_then(Object::as_i64).unwrap_or(0);
            dict.set("F", flags | PRINT_FLAG);
        }
    }
    Ok(missing)
}

/// Scripts and attached files of the name tree, the XFA form and its "draw the fields
/// yourself" flag.
fn clean_catalog(doc: &mut Document) -> Result<()> {
    for key in [b"Names".as_slice(), b"AcroForm"] {
        let id = match doc.catalog_mut()?.get_mut(key) {
            Ok(Object::Reference(id)) => Some(*id),
            Ok(Object::Dictionary(dict)) => {
                strip(dict);
                None
            }
            _ => None,
        };
        if let Some(id) = id
            && let Ok(dict) = doc.get_dictionary_mut(id)
        {
            strip(dict);
        }
    }
    Ok(())
}

fn strip(dict: &mut Dictionary) {
    for key in [
        b"JavaScript".as_slice(),
        b"EmbeddedFiles",
        b"XFA",
        b"NeedAppearances",
    ] {
        dict.remove(key);
    }
}

/// LZW streams are decoded (they are compressed again with Flate on saving); pictures
/// lose what PDF/A forbids: interpolation, alternates, OPI; graphic states their transfer
/// functions.
fn clean_streams(doc: &mut Document) {
    for object in doc.objects.values_mut() {
        if let Object::Stream(stream) = object
            && stream
                .filters()
                .is_ok_and(|filters| filters.contains(&b"LZWDecode".as_slice()))
        {
            // Best effort: a stream that cannot be decoded is left as it is.
            let _ = stream.decompress();
        }
        let Some(dict) = holder_mut(object) else {
            continue;
        };
        match dict.get(b"Subtype").and_then(Object::as_name) {
            Ok(b"Image") => {
                for key in [b"Interpolate".as_slice(), b"Alternates", b"OPI"] {
                    dict.remove(key);
                }
            }
            Ok(b"Form") => {
                dict.remove(b"OPI");
            }
            _ => {}
        }
        if matches!(
            dict.get(b"Type").and_then(Object::as_name),
            Ok(b"ExtGState")
        ) {
            dict.remove(b"TR");
        }
    }
}

/// Two random byte strings when the trailer has no ID.
fn ensure_id(doc: &mut Document) {
    let valid = doc
        .trailer
        .get(b"ID")
        .and_then(Object::as_array)
        .is_ok_and(|id| id.len() == 2 && id.iter().all(|part| part.as_str().is_ok()));
    if valid {
        return;
    }
    let mut id = [0u8; 16];
    if getrandom::fill(&mut id).is_err() {
        // No randomness: the content still tells documents apart.
        let len = doc.objects.len() as u64;
        id[..8].copy_from_slice(&len.to_be_bytes());
    }
    let part = Object::String(id.to_vec(), StringFormat::Hexadecimal);
    doc.trailer.set("ID", vec![part.clone(), part]);
}

/// Info dictionary and XMP packet saying the same, with the PDF/A identification.
fn write_xmp(doc: &mut Document) -> Result<()> {
    let mut info = info_dict(doc).cloned().unwrap_or_default();
    let field = |key: &[u8]| text_entry(doc, &info, key).filter(|text| !text.trim().is_empty());
    let [title, author, subject, keywords, creator, producer] = [
        b"Title".as_slice(),
        b"Author",
        b"Subject",
        b"Keywords",
        b"Creator",
        b"Producer",
    ]
    .map(field);
    let now = pdf_date()
        .and_then(|date| PdfDate::parse(&date))
        .unwrap_or_default();
    let created = text_entry(doc, &info, b"CreationDate")
        .and_then(|date| PdfDate::parse(&date))
        .unwrap_or(now);
    let producer = producer.unwrap_or_else(|| "GodPDF".to_owned());
    let texts = [
        ("Title", &title),
        ("Author", &author),
        ("Subject", &subject),
        ("Keywords", &keywords),
        ("Creator", &creator),
    ];
    for (key, value) in texts {
        match value {
            Some(text) => info.set(key, text_string(text)),
            None => {
                info.remove(key.as_bytes());
            }
        }
    }
    info.set("Producer", text_string(&producer));
    info.set("CreationDate", Object::string_literal(created.pdf()));
    info.set("ModDate", Object::string_literal(now.pdf()));
    // Would need its own XMP property.
    info.remove(b"Trapped");

    let mut xmp = String::new();
    let mut property = |tag: &str, value: &Option<String>, wrap: Option<(&str, bool)>| {
        let Some(value) = value else { return };
        let value = escape(value);
        match wrap {
            None => xmp += &format!("   <{tag}>{value}</{tag}>\n"),
            Some((list, lang)) => {
                let lang = if lang { r#" xml:lang="x-default""# } else { "" };
                xmp += &format!(
                    "   <{tag}><rdf:{list}><rdf:li{lang}>{value}</rdf:li></rdf:{list}></{tag}>\n"
                );
            }
        }
    };
    property("dc:title", &title, Some(("Alt", true)));
    property("dc:creator", &author, Some(("Seq", false)));
    property("dc:description", &subject, Some(("Alt", true)));
    property("pdf:Keywords", &keywords, None);
    property("pdf:Producer", &Some(producer), None);
    property("xmp:CreatorTool", &creator, None);
    property("xmp:CreateDate", &Some(created.xmp()), None);
    property("xmp:ModifyDate", &Some(now.xmp()), None);
    property("xmp:MetadataDate", &Some(now.xmp()), None);
    let packet = format!(
        "<?xpacket begin=\"\u{FEFF}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n\
 <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
  <rdf:Description rdf:about=\"\"\n\
    xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\"\n\
    xmlns:dc=\"http://purl.org/dc/elements/1.1/\"\n\
    xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\"\n\
    xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\">\n\
   <pdfaid:part>2</pdfaid:part>\n\
   <pdfaid:conformance>B</pdfaid:conformance>\n\
   <dc:format>application/pdf</dc:format>\n\
{xmp}  </rdf:Description>\n\
 </rdf:RDF>\n\
</x:xmpmeta>\n\
<?xpacket end=\"w\"?>"
    );
    let info_id = doc.add_object(info);
    doc.trailer.set("Info", info_id);
    // PDF/A: the metadata stream is not compressed.
    let metadata = Stream::new(
        dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
        packet.into_bytes(),
    )
    .with_compression(false);
    let metadata = doc.add_object(metadata);
    doc.catalog_mut()?.set("Metadata", metadata);
    Ok(())
}

fn add_output_intent(doc: &mut Document) -> Result<()> {
    let profile = doc.add_object(Stream::new(dictionary! { "N" => 3 }, srgb_profile()));
    let intent = dictionary! {
        "Type" => "OutputIntent",
        "S" => "GTS_PDFA1",
        "OutputConditionIdentifier" => Object::string_literal(SRGB),
        "Info" => Object::string_literal(SRGB),
        "DestOutputProfile" => profile,
    };
    doc.catalog_mut()?
        .set("OutputIntents", vec![Object::Dictionary(intent)]);
    Ok(())
}

/// What still keeps `doc` from conforming.
fn check(doc: &Document) -> Vec<String> {
    let mut warnings = Vec::new();
    let fonts = fonts_not_embedded(doc);
    if !fonts.is_empty() {
        let names: Vec<&str> = fonts.iter().map(String::as_str).collect();
        warnings.push(format!("Fonts not embedded: {}", names.join(", ")));
    }
    if uses_cmyk(doc) {
        warnings.push("CMYK colours are used: the sRGB profile does not cover them".to_owned());
    }
    warnings
}

/// Names of the fonts whose glyphs are not in the file (Type 3 fonts are drawings: they
/// always are).
fn fonts_not_embedded(doc: &Document) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for object in doc.objects.values() {
        let Ok(font) = object.as_dict() else { continue };
        if !matches!(font.get(b"Type").and_then(Object::as_name), Ok(b"Font")) {
            continue;
        }
        let glyphs = match font.get(b"Subtype").and_then(Object::as_name) {
            Ok(b"Type3") => continue,
            Ok(b"Type0") => descendant(doc, font),
            Ok(_) => Some(font),
            Err(_) => continue,
        };
        let embedded = glyphs
            .and_then(|glyphs| dict_entry(doc, glyphs, b"FontDescriptor"))
            .is_some_and(|descriptor| {
                [b"FontFile".as_slice(), b"FontFile2", b"FontFile3"]
                    .iter()
                    .any(|key| descriptor.has(key))
            });
        if !embedded {
            let name = font
                .get(b"BaseFont")
                .and_then(Object::as_name)
                .map(|name| String::from_utf8_lossy(name).into_owned())
                .unwrap_or_else(|_| "unnamed".to_owned());
            // A subset is named "ABCDEF+Name".
            let name = match name.split_once('+') {
                Some((prefix, rest)) if prefix.len() == 6 => rest.to_owned(),
                _ => name,
            };
            names.insert(name);
        }
    }
    names
}

/// CMYK pictures, or pages painting in CMYK.
fn uses_cmyk(doc: &Document) -> bool {
    let cmyk_picture = doc.objects.values().any(|object| {
        object.as_stream().is_ok_and(|stream| {
            matches!(
                stream.dict.get(b"ColorSpace").and_then(Object::as_name),
                Ok(b"DeviceCMYK")
            )
        })
    });
    let paints_cmyk = |operation: &lopdf::content::Operation| match operation.operator.as_str() {
        "k" | "K" => true,
        "cs" | "CS" => matches!(
            operation.operands.first().map(Object::as_name),
            Some(Ok(b"DeviceCMYK"))
        ),
        _ => false,
    };
    cmyk_picture
        || page_ids(doc).into_iter().any(|page| {
            Content::decode(&doc.get_page_content(page))
                .is_ok_and(|content| content.operations.iter().any(paints_cmyk))
        })
}

/// A date, from the PDF form `D:YYYYMMDDHHmmSS+HH'mm'` (all but the year optional).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PdfDate {
    /// Year, month, day, hour, minute, second.
    parts: [u32; 6],
    /// `Z`, or the sign, hours and minutes of the offset from UTC.
    zone: Option<(char, u32, u32)>,
}

impl Default for PdfDate {
    fn default() -> Self {
        Self {
            parts: [2000, 1, 1, 0, 0, 0],
            zone: Some(('Z', 0, 0)),
        }
    }
}

impl PdfDate {
    fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let text = text.strip_prefix("D:").unwrap_or(text);
        let digits = text.chars().take_while(char::is_ascii_digit).count();
        if !(4..=14).contains(&digits) || digits % 2 != 0 {
            return None;
        }
        let mut parts = [0, 1, 1, 0, 0, 0];
        parts[0] = text[..4].parse().ok()?;
        for (index, at) in (4..digits).step_by(2).enumerate() {
            parts[index + 1] = text[at..at + 2].parse().ok()?;
        }
        let [_, month, day, hour, minute, second] = parts;
        if !(1..=12).contains(&month)
            || !(1..=31).contains(&day)
            || hour > 23
            || minute > 59
            || second > 59
        {
            return None;
        }
        let rest = &text[digits..];
        let zone = match rest.chars().next() {
            None => None,
            Some('Z') => Some(('Z', 0, 0)),
            Some(sign @ ('+' | '-')) => {
                let numbers: String = rest[1..].chars().filter(char::is_ascii_digit).collect();
                let hours = numbers.get(..2)?.parse().ok()?;
                let minutes = numbers.get(2..4).and_then(|m| m.parse().ok()).unwrap_or(0);
                Some((sign, hours, minutes))
            }
            Some(_) => return None,
        };
        Some(Self { parts, zone })
    }

    fn pdf(self) -> String {
        let [y, mo, d, h, mi, s] = self.parts;
        let zone = match self.zone {
            None => String::new(),
            Some(('Z', ..)) => "Z".to_owned(),
            Some((sign, hours, minutes)) => format!("{sign}{hours:02}'{minutes:02}'"),
        };
        format!("D:{y:04}{mo:02}{d:02}{h:02}{mi:02}{s:02}{zone}")
    }

    fn xmp(self) -> String {
        let [y, mo, d, h, mi, s] = self.parts;
        let zone = match self.zone {
            None => String::new(),
            Some(('Z', ..)) => "Z".to_owned(),
            Some((sign, hours, minutes)) => format!("{sign}{hours:02}:{minutes:02}"),
        };
        format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}{zone}")
    }
}

/// A small ICC v2 display profile of sRGB: D50 white point, the sRGB primaries adapted to
/// D50 (Bradford) and the sRGB tone curve sampled 1024 times.
pub(crate) fn srgb_profile() -> Vec<u8> {
    let fixed = |value: f64| ((value * 65536.0).round() as i32).to_be_bytes();
    let xyz = |[x, y, z]: [f64; 3]| {
        let mut tag = b"XYZ \0\0\0\0".to_vec();
        for value in [x, y, z] {
            tag.extend(fixed(value));
        }
        tag
    };
    let mut desc = b"desc\0\0\0\0".to_vec();
    desc.extend((SRGB.len() as u32 + 1).to_be_bytes());
    desc.extend(SRGB.as_bytes());
    // Null end, then empty Unicode and ScriptCode descriptions.
    desc.extend([0; 1 + 4 + 4 + 2 + 1 + 67]);
    let mut text = b"text\0\0\0\0".to_vec();
    text.extend(b"No copyright, use freely\0");
    let mut curve = b"curv\0\0\0\0".to_vec();
    curve.extend(1024u32.to_be_bytes());
    for index in 0..1024 {
        let x = f64::from(index) / 1023.0;
        let y = if x <= 0.04045 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        };
        curve.extend(((y * 65535.0).round() as u16).to_be_bytes());
    }
    let blocks = [
        desc,
        text,
        xyz([0.9642, 1.0, 0.8249]),
        xyz([0.436_074_7, 0.222_504_5, 0.013_932_2]),
        xyz([0.385_064_9, 0.716_878_6, 0.097_104_5]),
        xyz([0.143_080_4, 0.060_616_9, 0.714_173_3]),
        curve,
    ];
    // Each tag and the block it points at: the three tone curves share theirs.
    let table: [(&[u8; 4], usize); 9] = [
        (b"desc", 0),
        (b"cprt", 1),
        (b"wtpt", 2),
        (b"rXYZ", 3),
        (b"gXYZ", 4),
        (b"bXYZ", 5),
        (b"rTRC", 6),
        (b"gTRC", 6),
        (b"bTRC", 6),
    ];
    let start = 128 + 4 + 12 * table.len();
    let mut data = Vec::new();
    let mut places = Vec::new();
    for block in &blocks {
        places.push((start + data.len(), block.len()));
        data.extend(block);
        while data.len() % 4 != 0 {
            data.push(0);
        }
    }
    let size = start + data.len();
    let mut profile = Vec::with_capacity(size);
    profile.extend((size as u32).to_be_bytes());
    profile.extend([0; 4]);
    profile.extend([0x02, 0x10, 0, 0]);
    profile.extend(b"mntrRGB XYZ ");
    // Created 2000-01-01 00:00:00.
    for part in [2000u16, 1, 1, 0, 0, 0] {
        profile.extend(part.to_be_bytes());
    }
    profile.extend(b"acsp");
    profile.extend([0; 24]);
    // Perceptual rendering intent, then the D50 illuminant of the connection space.
    profile.extend([0; 4]);
    for value in [0.9642, 1.0, 0.8249] {
        profile.extend(fixed(value));
    }
    profile.resize(128, 0);
    profile.extend((table.len() as u32).to_be_bytes());
    for (signature, block) in table {
        let (offset, length) = places[block];
        profile.extend(signature);
        profile.extend((offset as u32).to_be_bytes());
        profile.extend((length as u32).to_be_bytes());
    }
    profile.extend(data);
    profile
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        let mut doc = Document::with_version("1.4");
        let pages = doc.new_object_id();
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let contents = doc.add_object(Stream::new(
            Dictionary::new(),
            b"BT /F1 12 Tf 72 720 Td (Hi) Tj ET".to_vec(),
        ));
        let script = doc.add_object(
            dictionary! { "S" => "JavaScript", "JS" => Object::string_literal("app.alert(1)") },
        );
        let page = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages,
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            "Contents" => contents,
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
            "AA" => dictionary! { "O" => script },
        });
        doc.objects.insert(
            pages,
            Object::Dictionary(
                dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 },
            ),
        );
        let catalog = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages,
            "OpenAction" => script,
            "Names" => dictionary! { "JavaScript" => dictionary! { "Names" => vec![] } },
        });
        doc.trailer.set("Root", catalog);
        let info = doc.add_object(dictionary! {
            "Title" => Object::string_literal("Report <draft> & notes"),
            "Author" => Object::string_literal("Ada"),
            "CreationDate" => Object::string_literal("D:20240315093000+01'00'"),
        });
        doc.trailer.set("Info", info);
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    #[test]
    fn the_structure_of_pdfa_is_written() {
        let converted = convert_to_pdfa(&sample()).unwrap();
        assert!(converted.bytes.starts_with(b"%PDF-1.7"));
        let doc = Document::load_mem(&converted.bytes).unwrap();
        let catalog = doc.catalog().unwrap();
        assert!(!catalog.has(b"OpenAction"));
        assert!(
            !dict_entry(&doc, catalog, b"Names")
                .unwrap()
                .has(b"JavaScript")
        );
        let page = page_ids(&doc)[0];
        assert!(!doc.get_dictionary(page).unwrap().has(b"AA"));

        let intents = catalog.get(b"OutputIntents").unwrap().as_array().unwrap();
        let intent = deref(&doc, &intents[0]).unwrap().as_dict().unwrap();
        assert_eq!(intent.get(b"S").unwrap().as_name().unwrap(), b"GTS_PDFA1");
        let profile = deref(&doc, intent.get(b"DestOutputProfile").unwrap())
            .unwrap()
            .as_stream()
            .unwrap()
            .decompressed_content()
            .unwrap();
        assert_eq!(profile, srgb_profile());

        let ids = doc.trailer.get(b"ID").unwrap().as_array().unwrap();
        assert_eq!(ids.len(), 2);

        let metadata = deref(&doc, catalog.get(b"Metadata").unwrap())
            .unwrap()
            .as_stream()
            .unwrap();
        assert!(
            metadata.dict.get(b"Filter").is_err(),
            "XMP must not be compressed"
        );
        let xmp = String::from_utf8(metadata.content.clone()).unwrap();
        let start = xmp.find("<x:xmpmeta").unwrap();
        let end = xmp.find("<?xpacket end").unwrap();
        let tree = roxmltree::Document::parse(&xmp[start..end]).unwrap();
        let text = |name: &str| {
            tree.descendants()
                .find(|node| node.tag_name().name() == name)
                .and_then(|node| {
                    node.descendants()
                        .find_map(|n| n.text().filter(|t| !t.trim().is_empty()))
                })
                .map(str::to_owned)
        };
        assert_eq!(text("part").as_deref(), Some("2"));
        assert_eq!(text("conformance").as_deref(), Some("B"));
        assert_eq!(text("title").as_deref(), Some("Report <draft> & notes"));
        assert_eq!(text("creator").as_deref(), Some("Ada"));
        assert_eq!(
            text("CreateDate").as_deref(),
            Some("2024-03-15T09:30:00+01:00")
        );

        let info = info_dict(&doc).unwrap();
        assert_eq!(
            text_entry(&doc, info, b"CreationDate").as_deref(),
            Some("D:20240315093000+01'00'")
        );
        let modified = text_entry(&doc, info, b"ModDate").unwrap();
        assert_eq!(
            text("ModifyDate"),
            PdfDate::parse(&modified).map(PdfDate::xmp)
        );
    }

    #[test]
    fn fonts_not_embedded_are_reported() {
        let converted = convert_to_pdfa(&sample()).unwrap();
        assert_eq!(converted.warnings, ["Fonts not embedded: Helvetica"]);
    }

    #[test]
    fn the_profile_is_a_well_formed_icc() {
        let profile = srgb_profile();
        let size = u32::from_be_bytes(profile[..4].try_into().unwrap()) as usize;
        assert_eq!(size, profile.len());
        assert_eq!(&profile[36..40], b"acsp");
        assert_eq!(&profile[12..24], b"mntrRGB XYZ ");
        let count = u32::from_be_bytes(profile[128..132].try_into().unwrap()) as usize;
        assert_eq!(count, 9);
        for entry in 0..count {
            let at = 132 + entry * 12;
            let offset = u32::from_be_bytes(profile[at + 4..at + 8].try_into().unwrap()) as usize;
            let length = u32::from_be_bytes(profile[at + 8..at + 12].try_into().unwrap()) as usize;
            assert_eq!(offset % 4, 0);
            assert!(offset + length <= size);
        }
    }

    #[test]
    fn dates_read_both_ways() {
        let date = PdfDate::parse("D:20240315093000+01'00'").unwrap();
        assert_eq!(date.xmp(), "2024-03-15T09:30:00+01:00");
        assert_eq!(PdfDate::parse("D:2024").unwrap().pdf(), "D:20240101000000");
        assert_eq!(PdfDate::parse("D:20241301").map(PdfDate::pdf), None);
        assert_eq!(
            PdfDate::parse("D:20240315093000Z").unwrap().xmp(),
            "2024-03-15T09:30:00Z"
        );
    }
}
