use std::collections::{HashMap, HashSet};

use lopdf::{Dictionary, Document, Object, ObjectId, decode_text_string, text_string};

use crate::Result;
use crate::pdf::{check_page, deref, dict_entry, inherited, load, page_ids, save};

/// Document information dictionary fields. `None` = absent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Metadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    pub creator: Option<String>,
    pub producer: Option<String>,
}

/// One bookmark. `page` is `None` when the destination cannot be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineItem {
    pub title: String,
    pub page: Option<usize>,
    pub children: Vec<OutlineItem>,
}

/// Guards against deep / cyclic outline and name trees.
const MAX_DEPTH: usize = 64;

/// How a page is displayed: needed to map screen points back to unrotated page space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageGeometry {
    /// Clockwise /Rotate normalized to 0, 90, 180 or 270.
    pub rotation: i32,
    /// Lower-left corner of the visible box (CropBox intersected with MediaBox).
    pub origin: (f32, f32),
}

pub fn page_count(pdf: &[u8]) -> Result<usize> {
    Ok(load(pdf)?.page_iter().count())
}

pub fn page_geometry(pdf: &[u8], page: usize) -> Result<PageGeometry> {
    let doc = load(pdf)?;
    let ids = page_ids(&doc);
    check_page(page, ids.len())?;
    let id = ids[page];
    let rotation = inherited(&doc, id, b"Rotate")
        .and_then(|rotate| deref(&doc, &rotate)?.as_i64().ok())
        .map_or(0, |deg| (deg.rem_euclid(360) / 90 * 90) as i32);
    let page_box = |key: &[u8]| inherited(&doc, id, key).and_then(|b| box_origin(&doc, &b));
    // Viewers display CropBox ∩ MediaBox, so the visible origin is the max of both corners.
    let origin = match (page_box(b"MediaBox"), page_box(b"CropBox")) {
        (Some(media), Some(crop)) => (media.0.max(crop.0), media.1.max(crop.1)),
        (media, crop) => crop.or(media).unwrap_or((0.0, 0.0)),
    };
    Ok(PageGeometry { rotation, origin })
}

/// Lower-left corner of a (possibly unnormalized) page box.
fn box_origin(doc: &Document, page_box: &Object) -> Option<(f32, f32)> {
    let numbers: Vec<f32> = deref(doc, page_box)?
        .as_array()
        .ok()?
        .iter()
        .filter_map(|n| deref(doc, n)?.as_float().ok())
        .collect();
    match numbers[..] {
        [x0, y0, x1, y1] => Some((x0.min(x1), y0.min(y1))),
        _ => None,
    }
}

pub fn read_metadata(pdf: &[u8]) -> Result<Metadata> {
    let doc = load(pdf)?;
    let Some(info) = info_dict(&doc) else {
        return Ok(Metadata::default());
    };
    let field = |key: &[u8]| text_entry(&doc, info, key);
    Ok(Metadata {
        title: field(b"Title"),
        author: field(b"Author"),
        subject: field(b"Subject"),
        keywords: field(b"Keywords"),
        creator: field(b"Creator"),
        producer: field(b"Producer"),
    })
}

/// Replace the Info dictionary with `meta` (`None` fields are removed). Non-ASCII stored as UTF-16BE.
///
/// Entries outside these six fields (dates, custom keys) are kept.
pub fn write_metadata(pdf: &[u8], meta: &Metadata) -> Result<Vec<u8>> {
    let mut doc = load(pdf)?;
    let mut info = info_dict(&doc).cloned().unwrap_or_default();
    let fields = [
        ("Title", &meta.title),
        ("Author", &meta.author),
        ("Subject", &meta.subject),
        ("Keywords", &meta.keywords),
        ("Creator", &meta.creator),
        ("Producer", &meta.producer),
    ];
    for (key, value) in fields {
        match value {
            Some(text) => info.set(key, text_string(text)),
            None => {
                info.remove(key.as_bytes());
            }
        }
    }
    let info_id = doc.add_object(info);
    doc.trailer.set("Info", info_id);
    save(&mut doc)
}

/// Bookmark tree; empty when the document has no outline.
pub fn outline(pdf: &[u8]) -> Result<Vec<OutlineItem>> {
    let doc = load(pdf)?;
    let Some(first) = outline_first(&doc) else {
        return Ok(Vec::new());
    };
    let pages: HashMap<ObjectId, usize> = page_ids(&doc)
        .into_iter()
        .enumerate()
        .map(|(i, id)| (id, i))
        .collect();
    let mut visited = HashSet::new();
    Ok(read_items(&doc, first, &pages, &mut visited, 0))
}

pub(crate) fn info_dict(doc: &Document) -> Option<&Dictionary> {
    dict_entry(doc, &doc.trailer, b"Info")
}

/// Decoded text string at `key` (PDFDocEncoding, UTF-16BE or UTF-8).
pub(crate) fn text_entry(doc: &Document, dict: &Dictionary, key: &[u8]) -> Option<String> {
    let value = deref(doc, dict.get(key).ok()?)?;
    decode_text_string(value).ok()
}

/// First top-level outline item, if any.
pub(crate) fn outline_first(doc: &Document) -> Option<ObjectId> {
    let outlines = dict_entry(doc, doc.catalog().ok()?, b"Outlines")?;
    outlines.get(b"First").and_then(Object::as_reference).ok()
}

fn read_items(
    doc: &Document,
    first: ObjectId,
    pages: &HashMap<ObjectId, usize>,
    visited: &mut HashSet<ObjectId>,
    depth: usize,
) -> Vec<OutlineItem> {
    let mut items = Vec::new();
    let mut next = Some(first);
    while let Some(id) = next {
        if depth > MAX_DEPTH || !visited.insert(id) {
            break;
        }
        let Ok(dict) = doc.get_dictionary(id) else {
            break;
        };
        let children = match dict.get(b"First").and_then(Object::as_reference) {
            Ok(child) => read_items(doc, child, pages, visited, depth + 1),
            Err(_) => Vec::new(),
        };
        items.push(OutlineItem {
            title: text_entry(doc, dict, b"Title").unwrap_or_default(),
            page: item_dest(doc, dict).and_then(|dest| dest_page(&dest, pages)),
            children,
        });
        next = dict.get(b"Next").and_then(Object::as_reference).ok();
    }
    items
}

/// Every outline item id (any depth), cycle-safe.
pub(crate) fn outline_item_ids(doc: &Document) -> Vec<ObjectId> {
    let mut ids = Vec::new();
    let mut visited = HashSet::new();
    let mut stack: Vec<ObjectId> = outline_first(doc).into_iter().collect();
    while let Some(id) = stack.pop() {
        if !visited.insert(id) {
            continue;
        }
        let Ok(dict) = doc.get_dictionary(id) else {
            continue;
        };
        ids.push(id);
        for key in [b"First".as_slice(), b"Next"] {
            if let Ok(child) = dict.get(key).and_then(Object::as_reference) {
                stack.push(child);
            }
        }
    }
    ids
}

/// Rewrite every resolvable outline destination as an explicit `/Dest [page ...]` array,
/// so outlines survive when name trees are not carried over (merge).
pub(crate) fn make_outline_dests_explicit(doc: &mut Document) {
    let resolved: Vec<(ObjectId, Vec<Object>)> = outline_item_ids(doc)
        .into_iter()
        .filter_map(|id| {
            let dict = doc.get_dictionary(id).ok()?;
            Some((id, item_dest(doc, dict)?))
        })
        .collect();
    for (id, dest) in resolved {
        if let Ok(dict) = doc.get_dictionary_mut(id) {
            dict.set("Dest", dest);
            dict.remove(b"A");
        }
    }
}

/// True when `dest` (array, named dest or action) resolves to a destination whose page
/// reference was nulled because the page was removed.
pub(crate) fn is_dangling_dest(doc: &Document, dest: &Object) -> bool {
    resolve_dest(doc, dest, 0).is_some_and(|d| matches!(d.first(), Some(Object::Null)))
}

/// True when an outline item or link annotation points at a removed page.
pub(crate) fn has_dangling_dest(doc: &Document, item: &Dictionary) -> bool {
    item_dest(doc, item).is_some_and(|d| matches!(d.first(), Some(Object::Null)))
}

/// Drop outline items that point at removed pages (unless they still have children),
/// relinking siblings and fixing /First /Last /Count on every level.
pub(crate) fn prune_outline(doc: &mut Document) {
    let Some(outlines_id) = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"Outlines").ok())
        .and_then(|o| o.as_reference().ok())
    else {
        return;
    };
    let first = outline_first(doc);
    let mut visited = HashSet::new();
    let (kept, visible) = match first {
        Some(first) => prune_level(doc, first, outlines_id, &mut visited, 0),
        None => (Vec::new(), 0),
    };
    if let Ok(outlines) = doc.get_dictionary_mut(outlines_id) {
        set_children(outlines, &kept, visible as i64);
    }
}

/// Prune the sibling chain starting at `first`; returns kept ids and their visible-item count.
fn prune_level(
    doc: &mut Document,
    first: ObjectId,
    parent: ObjectId,
    visited: &mut HashSet<ObjectId>,
    depth: usize,
) -> (Vec<ObjectId>, usize) {
    let mut kept = Vec::new();
    let mut visible = 0;
    let mut next = Some(first);
    while let Some(id) = next {
        if depth > MAX_DEPTH || !visited.insert(id) {
            break;
        }
        let Ok(dict) = doc.get_dictionary(id) else {
            break;
        };
        next = dict.get(b"Next").and_then(Object::as_reference).ok();
        let child = dict.get(b"First").and_then(Object::as_reference).ok();
        let open = dict.get(b"Count").and_then(Object::as_i64).unwrap_or(0) > 0;
        let dangling = has_dangling_dest(doc, dict);
        let (kids, kids_visible) = match child {
            Some(child) => prune_level(doc, child, id, visited, depth + 1),
            None => (Vec::new(), 0),
        };
        if dangling && kids.is_empty() {
            continue;
        }
        let Ok(dict) = doc.get_dictionary_mut(id) else {
            break;
        };
        if dangling {
            dict.remove(b"Dest");
            dict.remove(b"A");
        }
        let sign = if open { 1 } else { -1 };
        set_children(dict, &kids, sign * kids_visible as i64);
        dict.set("Parent", parent);
        kept.push(id);
        visible += 1 + if open { kids_visible } else { 0 };
    }
    for (i, &id) in kept.iter().enumerate() {
        if let Ok(dict) = doc.get_dictionary_mut(id) {
            match i.checked_sub(1).map(|p| kept[p]) {
                Some(prev) => dict.set("Prev", prev),
                None => drop(dict.remove(b"Prev")),
            }
            match kept.get(i + 1) {
                Some(&next) => dict.set("Next", next),
                None => drop(dict.remove(b"Next")),
            }
        }
    }
    (kept, visible)
}

/// Point /First /Last at `kids` and set /Count, or clear them when there are no kids.
fn set_children(dict: &mut Dictionary, kids: &[ObjectId], count: i64) {
    match (kids.first(), kids.last()) {
        (Some(&first), Some(&last)) => {
            dict.set("First", first);
            dict.set("Last", last);
            dict.set("Count", count);
        }
        _ => {
            dict.remove(b"First");
            dict.remove(b"Last");
            dict.remove(b"Count");
        }
    }
}

/// Explicit destination array of an outline item (/Dest or /A GoTo /D).
fn item_dest(doc: &Document, item: &Dictionary) -> Option<Vec<Object>> {
    if let Ok(dest) = item.get(b"Dest") {
        return resolve_dest(doc, dest, 0);
    }
    let action = dict_entry(doc, item, b"A")?;
    if action.get(b"S").and_then(Object::as_name).ok()? != b"GoTo" {
        return None;
    }
    resolve_dest(doc, action.get(b"D").ok()?, 0)
}

fn resolve_dest(doc: &Document, dest: &Object, depth: usize) -> Option<Vec<Object>> {
    if depth > 2 {
        return None;
    }
    match deref(doc, dest)? {
        Object::Array(items) => Some(items.clone()),
        Object::Dictionary(dict) => resolve_dest(doc, dict.get(b"D").ok()?, depth + 1),
        Object::Name(name) => resolve_dest(doc, named_dest(doc, name)?, depth + 1),
        Object::String(name, _) => resolve_dest(doc, named_dest(doc, name)?, depth + 1),
        _ => None,
    }
}

/// Look `name` up in the catalog /Dests dictionary, then the /Names /Dests name tree.
fn named_dest<'a>(doc: &'a Document, name: &[u8]) -> Option<&'a Object> {
    let catalog = doc.catalog().ok()?;
    if let Some(found) = dict_entry(doc, catalog, b"Dests").and_then(|dests| dests.get(name).ok()) {
        return Some(found);
    }
    let names = dict_entry(doc, catalog, b"Names")?;
    let tree = dict_entry(doc, names, b"Dests")?;
    search_name_tree(doc, tree, name, 0)
}

fn search_name_tree<'a>(
    doc: &'a Document,
    node: &'a Dictionary,
    name: &[u8],
    depth: usize,
) -> Option<&'a Object> {
    if depth > MAX_DEPTH {
        return None;
    }
    if let Some(pairs) = node
        .get(b"Names")
        .ok()
        .and_then(|n| deref(doc, n))
        .and_then(|n| n.as_array().ok())
    {
        for pair in pairs.chunks(2) {
            if let [key, value] = pair
                && deref(doc, key).and_then(|k| k.as_str().ok()) == Some(name)
            {
                return Some(value);
            }
        }
    }
    let kids = node
        .get(b"Kids")
        .ok()
        .and_then(|k| deref(doc, k))?
        .as_array()
        .ok()?;
    kids.iter()
        .filter_map(|kid| deref(doc, kid)?.as_dict().ok())
        .find_map(|kid| search_name_tree(doc, kid, name, depth + 1))
}

/// Page index of an explicit destination: a page reference, or an integer page number.
fn dest_page(dest: &[Object], pages: &HashMap<ObjectId, usize>) -> Option<usize> {
    match dest.first()? {
        Object::Reference(id) => pages.get(id).copied(),
        Object::Integer(n) => usize::try_from(*n).ok().filter(|&n| n < pages.len()),
        _ => None,
    }
}
