//! Whole-document changes: colours of the pages, comments taken out or drawn into the
//! page, hidden data removed, size reduced.

use std::collections::HashMap;

use lopdf::{Document, Object, ObjectId, SaveOptions, dictionary};

use crate::annotate::{annots_entries, num, rgb, set_annots};
use crate::pdf::{deref, dict_entry, load, page_ids, save};
use crate::stamp::{View, targets, wrap};
use crate::{Color, Error, Result};

/// A colour change of whole pages.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tint {
    /// Black becomes white and white black: for reading at night.
    Invert,
    Greyscale,
    /// White paper takes this colour; black stays black.
    Paper(Color),
}

/// Change the colours of `pages`. A layer drawn over each page blends with it: nothing of
/// the page is rewritten.
pub fn tint_pages(pdf: &[u8], pages: &[usize], tint: Tint) -> Result<Vec<u8>> {
    let mut doc = load(pdf)?;
    let ids = targets(&mut doc, pages)?;
    let (mode, color) = match tint {
        Tint::Invert => ("Difference", "1 1 1".to_owned()),
        Tint::Greyscale => ("Saturation", "0.5 0.5 0.5".to_owned()),
        Tint::Paper(color) => ("Multiply", rgb(color)),
    };
    let state = doc.add_object(dictionary! { "Type" => "ExtGState", "BM" => mode });
    for id in ids {
        let [x0, y0, x1, y1] = View::of(&doc, id).corners;
        let ops = format!(
            "q /GodPdfTint gs {color} rg {} {} {} {} re f Q\n",
            num(x0),
            num(y0),
            num(x1 - x0),
            num(y1 - y0)
        );
        wrap(
            &mut doc,
            id,
            "",
            &ops,
            &[("ExtGState", "GodPdfTint", state)],
        )?;
    }
    save(&mut doc)
}

/// What kind of annotation `annot` is.
fn subtype<'a>(doc: &'a Document, annot: &'a Object) -> Option<&'a [u8]> {
    deref(doc, annot)?
        .as_dict()
        .ok()?
        .get(b"Subtype")
        .ok()?
        .as_name()
        .ok()
}

/// Take every comment out (highlights, notes, drawings…). Links and form fields stay.
pub fn remove_comments(pdf: &[u8]) -> Result<Vec<u8>> {
    let mut doc = load(pdf)?;
    let mut removed = 0;
    for page in page_ids(&doc) {
        let entries = annots_entries(&doc, page)?;
        let kept: Vec<Object> = entries
            .iter()
            .filter(|annot| matches!(subtype(&doc, annot), Some(b"Link" | b"Widget")))
            .cloned()
            .collect();
        if kept.len() != entries.len() {
            removed += entries.len() - kept.len();
            set_annots(&mut doc, page, kept)?;
        }
    }
    if removed == 0 {
        return Err(Error::InvalidArgument(
            "the document has no comments".into(),
        ));
    }
    save(&mut doc)
}

/// Draw the comments and filled-in form fields into the pages, where they can no longer be
/// moved or changed. Links stay links.
pub fn flatten_annotations(pdf: &[u8]) -> Result<Vec<u8>> {
    let mut doc = load(pdf)?;
    let mut flattened = 0;
    for page in page_ids(&doc) {
        let entries = annots_entries(&doc, page)?;
        let mut kept = Vec::new();
        let mut ops = String::new();
        let mut xobjects = Vec::new();
        for annot in &entries {
            if matches!(subtype(&doc, annot), Some(b"Link") | None) {
                kept.push(annot.clone());
                continue;
            }
            flattened += 1;
            let Some((form, cm)) = appearance(&doc, annot) else {
                continue;
            };
            let name = format!("GodPdfFlat{}", xobjects.len());
            ops += &format!("q {cm} cm /{name} Do Q\n");
            xobjects.push((name, form));
        }
        if kept.len() == entries.len() {
            continue;
        }
        set_annots(&mut doc, page, kept)?;
        let resources: Vec<(&str, &str, ObjectId)> = xobjects
            .iter()
            .map(|(name, form)| ("XObject", name.as_str(), *form))
            .collect();
        wrap(&mut doc, page, "", &ops, &resources)?;
    }
    if flattened == 0 {
        return Err(Error::InvalidArgument(
            "the document has no comments or form fields".into(),
        ));
    }
    // The fields are drawn now: the form would only list widgets that are gone.
    doc.catalog_mut()?.remove(b"AcroForm");
    save(&mut doc)
}

/// The normal appearance of a visible annotation and the matrix that puts it on its
/// rectangle (PDF 32000 12.5.5).
fn appearance(doc: &Document, annot: &Object) -> Option<(ObjectId, String)> {
    let dict = deref(doc, annot)?.as_dict().ok()?;
    let flags = dict.get(b"F").and_then(Object::as_i64).unwrap_or(0);
    // Hidden (2) or not shown on screen (32).
    if flags & (2 | 32) != 0 {
        return None;
    }
    let normal = dict_entry(doc, dict, b"AP")?.get(b"N").ok()?;
    let form = match normal {
        Object::Reference(id) if doc.get_object(*id).is_ok_and(|o| o.as_stream().is_ok()) => *id,
        // One appearance per state (check boxes): the one named by /AS.
        other => {
            let states = deref(doc, other)?.as_dict().ok()?;
            let state = dict.get(b"AS").and_then(Object::as_name).ok()?;
            states.get(state).and_then(Object::as_reference).ok()?
        }
    };
    let stream = doc.get_object(form).ok()?.as_stream().ok()?;
    let numbers = |object: &Object| -> Option<Vec<f32>> {
        deref(doc, object)?
            .as_array()
            .ok()?
            .iter()
            .map(|n| deref(doc, n)?.as_float().ok())
            .collect()
    };
    let rect: [f32; 4] = numbers(dict.get(b"Rect").ok()?)?.try_into().ok()?;
    let bbox: [f32; 4] = numbers(stream.dict.get(b"BBox").ok()?)?.try_into().ok()?;
    let matrix: [f32; 6] = stream
        .dict
        .get(b"Matrix")
        .ok()
        .and_then(numbers)
        .and_then(|m| m.try_into().ok())
        .unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    let [a, b, c, d, e, f] = matrix;
    let [b0, b1, b2, b3] = bbox;
    let corners = [(b0, b1), (b2, b1), (b0, b3), (b2, b3)]
        .map(|(x, y)| (a * x + c * y + e, b * x + d * y + f));
    let min_x = corners.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
    let max_x = corners
        .iter()
        .map(|p| p.0)
        .fold(f32::NEG_INFINITY, f32::max);
    let min_y = corners.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
    let max_y = corners
        .iter()
        .map(|p| p.1)
        .fold(f32::NEG_INFINITY, f32::max);
    if max_x - min_x < 1e-3 || max_y - min_y < 1e-3 {
        return None;
    }
    let [r0, r1, r2, r3] = rect;
    let sx = (r0 - r2).abs() / (max_x - min_x);
    let sy = (r1 - r3).abs() / (max_y - min_y);
    let cm = format!(
        "{} 0 0 {} {} {}",
        num(sx),
        num(sy),
        num(r0.min(r2) - min_x * sx),
        num(r1.min(r3) - min_y * sy)
    );
    Some((form, cm))
}

/// Take out what a document carries besides its pages: title, author and other
/// properties, scripts, actions run on opening, attached files.
pub fn remove_hidden_data(pdf: &[u8]) -> Result<Vec<u8>> {
    let mut doc = load(pdf)?;
    doc.trailer.remove(b"Info");
    let catalog = doc.catalog_mut()?;
    for key in [b"Metadata".as_slice(), b"OpenAction", b"AA", b"PieceInfo"] {
        catalog.remove(key);
    }
    let names = match catalog.get_mut(b"Names") {
        Ok(Object::Reference(id)) => Some(*id),
        Ok(Object::Dictionary(names)) => {
            names.remove(b"JavaScript");
            names.remove(b"EmbeddedFiles");
            None
        }
        _ => None,
    };
    if let Some(id) = names
        && let Ok(names) = doc.get_dictionary_mut(id)
    {
        names.remove(b"JavaScript");
        names.remove(b"EmbeddedFiles");
    }
    for page in page_ids(&doc) {
        let dict = doc.get_dictionary_mut(page)?;
        for key in [b"AA".as_slice(), b"Metadata", b"PieceInfo"] {
            dict.remove(key);
        }
        // Links that run a script or a program go, and attached files; the rest stays.
        let entries = annots_entries(&doc, page)?;
        let kept: Vec<Object> = entries
            .iter()
            .filter(|annot| !runs_something(&doc, annot))
            .cloned()
            .collect();
        if kept.len() != entries.len() {
            set_annots(&mut doc, page, kept)?;
        }
    }
    save(&mut doc)
}

fn runs_something(doc: &Document, annot: &Object) -> bool {
    let Some(dict) = deref(doc, annot).and_then(|a| a.as_dict().ok()) else {
        return false;
    };
    if matches!(subtype(doc, annot), Some(b"FileAttachment")) || dict.has(b"AA") {
        return true;
    }
    let kind = dict_entry(doc, dict, b"A")
        .and_then(|action| action.get(b"S").ok())
        .and_then(|kind| kind.as_name().ok());
    matches!(kind, Some(b"JavaScript" | b"Launch" | b"ImportData"))
}

/// Make the file smaller without changing what it shows: unused and duplicated objects
/// go, every stream is compressed, small objects are packed together.
pub fn compress(pdf: &[u8]) -> Result<Vec<u8>> {
    let mut doc = load(pdf)?;
    deduplicate_streams(&mut doc);
    doc.prune_objects();
    doc.compress();
    let options = SaveOptions::builder()
        .use_object_streams(true)
        .use_xref_streams(true)
        .build();
    let mut out = Vec::new();
    doc.save_with_options(&mut out, options)
        .map_err(|err| Error::Pdf(err.to_string()))?;
    if out.len() >= pdf.len() {
        return Err(Error::InvalidArgument(
            "the document cannot be made smaller".into(),
        ));
    }
    Ok(out)
}

/// Point every reference to a stream at the first one with the same dictionary and bytes
/// (the same font or picture copied in by each merged file).
fn deduplicate_streams(doc: &mut Document) {
    let mut first: HashMap<(String, Vec<u8>), ObjectId> = HashMap::new();
    let mut same: HashMap<ObjectId, ObjectId> = HashMap::new();
    for (&id, object) in &doc.objects {
        if let Object::Stream(stream) = object {
            let key = (format!("{:?}", stream.dict), stream.content.clone());
            match first.get(&key) {
                Some(&kept) => {
                    same.insert(id, kept);
                }
                None => {
                    first.insert(key, id);
                }
            }
        }
    }
    if same.is_empty() {
        return;
    }
    for object in doc.objects.values_mut() {
        redirect(object, &same);
    }
}

fn redirect(object: &mut Object, same: &HashMap<ObjectId, ObjectId>) {
    match object {
        Object::Reference(id) => {
            if let Some(&kept) = same.get(id) {
                *id = kept;
            }
        }
        Object::Array(items) => items.iter_mut().for_each(|item| redirect(item, same)),
        Object::Dictionary(dict) => dict.iter_mut().for_each(|(_, v)| redirect(v, same)),
        Object::Stream(stream) => stream.dict.iter_mut().for_each(|(_, v)| redirect(v, same)),
        _ => {}
    }
}
