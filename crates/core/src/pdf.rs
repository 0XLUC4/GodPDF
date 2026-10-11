//! Shared lopdf plumbing: load/save, error mapping and a flat page tree.

use std::collections::HashSet;

use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::info::{has_dangling_dest, is_dangling_dest, prune_outline};
use crate::{Error, Result};

/// Attributes a page may inherit from its ancestors in the page tree.
const INHERITABLE: [&[u8]; 4] = [b"MediaBox", b"CropBox", b"Resources", b"Rotate"];
/// Guard against malicious /Parent cycles.
const MAX_TREE_DEPTH: usize = 64;
/// US Letter, used when a page has no MediaBox at all.
const DEFAULT_MEDIA_BOX: [i64; 4] = [0, 0, 612, 792];

impl From<lopdf::Error> for Error {
    fn from(err: lopdf::Error) -> Self {
        Error::Pdf(err.to_string())
    }
}

/// Parse `pdf`. Fails with [`Error::Encrypted`] when a password is needed.
pub(crate) fn load(pdf: &[u8]) -> Result<Document> {
    let doc = Document::load_mem(pdf).map_err(|err| Error::Parse(err.to_string()))?;
    if doc.is_encrypted() {
        return Err(Error::Encrypted);
    }
    Ok(doc)
}

/// Drop unreachable objects, compress streams and serialize.
pub(crate) fn save(doc: &mut Document) -> Result<Vec<u8>> {
    doc.prune_objects();
    doc.compress();
    save_raw(doc)
}

/// Serialize as-is.
pub(crate) fn save_raw(doc: &mut Document) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|err| Error::Pdf(err.to_string()))?;
    Ok(out)
}

pub(crate) fn check_page(page: usize, count: usize) -> Result<()> {
    if page < count {
        Ok(())
    } else {
        Err(Error::PageOutOfRange { page, count })
    }
}

pub(crate) fn page_ids(doc: &Document) -> Vec<ObjectId> {
    doc.page_iter().collect()
}

/// Resolve `object` through references (no-op for direct objects).
pub(crate) fn deref<'a>(doc: &'a Document, object: &'a Object) -> Option<&'a Object> {
    doc.dereference(object).ok().map(|(_, resolved)| resolved)
}

/// Look up `key` in `dict`, following a reference, and return it as a dictionary.
pub(crate) fn dict_entry<'a>(
    doc: &'a Document,
    dict: &'a Dictionary,
    key: &[u8],
) -> Option<&'a Dictionary> {
    let value = dict.get(key).ok()?;
    deref(doc, value)?.as_dict().ok()
}

fn pages_root(doc: &Document) -> Result<ObjectId> {
    doc.catalog()?
        .get(b"Pages")?
        .as_reference()
        .map_err(|_| Error::Parse("page tree root is not an indirect object".into()))
}

/// Flatten the page tree: every page becomes a direct kid of the root /Pages node and
/// gets its inherited attributes copied onto itself. Returns the root id and pages in order.
pub(crate) fn flatten(doc: &mut Document) -> Result<(ObjectId, Vec<ObjectId>)> {
    let root = pages_root(doc)?;
    let pages = page_ids(doc);
    for &page in &pages {
        push_down_inherited(doc, page)?;
    }
    set_pages(doc, root, &pages)?;
    Ok((root, pages))
}

fn push_down_inherited(doc: &mut Document, page: ObjectId) -> Result<()> {
    let found: Vec<(&[u8], Option<Object>)> = INHERITABLE
        .iter()
        .map(|&key| (key, inherited(doc, page, key)))
        .collect();
    let dict = doc.get_dictionary_mut(page)?;
    for (key, value) in found {
        if let Some(value) = value {
            dict.set(key, value);
        }
    }
    if !dict.has(b"MediaBox") {
        dict.set(
            "MediaBox",
            DEFAULT_MEDIA_BOX
                .iter()
                .map(|&v| Object::Integer(v))
                .collect::<Vec<_>>(),
        );
    }
    if !dict.has(b"Resources") {
        dict.set("Resources", Dictionary::new());
    }
    Ok(())
}

/// Value of `key` on `page` or its nearest ancestor.
pub(crate) fn inherited(doc: &Document, page: ObjectId, key: &[u8]) -> Option<Object> {
    let mut node = doc.get_dictionary(page).ok()?;
    for _ in 0..MAX_TREE_DEPTH {
        if let Ok(value) = node.get(key) {
            return Some(value.clone());
        }
        let parent = node.get(b"Parent").and_then(Object::as_reference).ok()?;
        node = doc.get_dictionary(parent).ok()?;
    }
    None
}

/// Make `pages` the (flat) kids of `root`, in order.
pub(crate) fn set_pages(doc: &mut Document, root: ObjectId, pages: &[ObjectId]) -> Result<()> {
    for &page in pages {
        doc.get_dictionary_mut(page)?.set("Parent", root);
    }
    let dict = doc.get_dictionary_mut(root)?;
    dict.set("Type", "Pages");
    dict.set(
        "Kids",
        pages
            .iter()
            .map(|&id| Object::Reference(id))
            .collect::<Vec<_>>(),
    );
    dict.set("Count", pages.len() as i64);
    dict.remove(b"Parent");
    Ok(())
}

/// Rebuild the document so it holds exactly the pages at `order` (0-based, duplicates allowed).
/// Pages left out are removed and references to them are nulled.
pub(crate) fn rebuild(doc: &mut Document, order: &[usize]) -> Result<()> {
    let (root, pages) = flatten(doc)?;
    let mut used = HashSet::new();
    let mut new_pages = Vec::with_capacity(order.len());
    for &index in order {
        check_page(index, pages.len())?;
        let id = pages[index];
        let id = if used.insert(id) {
            id
        } else {
            duplicate_page(doc, id)?
        };
        new_pages.push(id);
    }
    set_pages(doc, root, &new_pages)?;
    let removed: HashSet<ObjectId> = pages.into_iter().filter(|id| !used.contains(id)).collect();
    detach(doc, &removed);
    Ok(())
}

/// Copy a page object (contents/resources shared) with its own copies of its annotations.
///
/// Form widgets are not copied: a field's widgets must be listed in its /Kids, so form
/// fields stay on the first instance of the page only.
fn duplicate_page(doc: &mut Document, page: ObjectId) -> Result<ObjectId> {
    let mut dict = doc.get_dictionary(page)?.clone();
    let new_id = doc.new_object_id();
    if let Some(annots) = dict
        .get(b"Annots")
        .ok()
        .and_then(|a| deref(doc, a))
        .and_then(|a| a.as_array().ok())
    {
        let annots: Vec<Object> = annots
            .iter()
            .filter(|annot| !is_widget(doc, annot))
            .cloned()
            .collect();
        let copies = annots
            .iter()
            .map(|annot| copy_annotation(doc, annot, new_id))
            .collect::<Vec<_>>();
        dict.set("Annots", copies);
    }
    doc.objects.insert(new_id, Object::Dictionary(dict));
    Ok(new_id)
}

fn is_widget(doc: &Document, annot: &Object) -> bool {
    deref(doc, annot)
        .and_then(|a| a.as_dict().ok())
        .and_then(|a| a.get(b"Subtype").ok())
        .and_then(|s| s.as_name().ok())
        == Some(b"Widget".as_slice())
}

fn copy_annotation(doc: &mut Document, annot: &Object, page: ObjectId) -> Object {
    let Some(mut dict) = deref(doc, annot).and_then(|a| a.as_dict().ok()).cloned() else {
        return annot.clone();
    };
    dict.set("P", page);
    dict.remove(b"Popup");
    Object::Reference(doc.add_object(dict))
}

/// Delete `removed` pages, replace every reference to them with null, then drop what now
/// points nowhere: outline items, link annotations, /OpenAction and null array entries.
pub(crate) fn detach(doc: &mut Document, removed: &HashSet<ObjectId>) {
    if removed.is_empty() {
        return;
    }
    for id in removed {
        doc.objects.remove(id);
    }
    for object in doc.objects.values_mut() {
        null_refs(object, removed);
    }
    prune_outline(doc);
    drop_dangling_links(doc);
}

fn drop_dangling_links(doc: &mut Document) {
    for page in page_ids(doc) {
        retain_array(doc, page, &[b"Annots"], |doc, annot| {
            let dangling = deref(doc, annot)
                .and_then(|a| a.as_dict().ok())
                .is_some_and(|a| has_dangling_dest(doc, a));
            !matches!(annot, Object::Null) && !dangling
        });
    }
    let Ok(catalog_id) = doc.trailer.get(b"Root").and_then(Object::as_reference) else {
        return;
    };
    retain_array(doc, catalog_id, &[b"AcroForm", b"Fields"], |_, field| {
        !matches!(field, Object::Null)
    });
    let open_dangling = doc
        .get_dictionary(catalog_id)
        .ok()
        .and_then(|c| c.get(b"OpenAction").ok())
        .is_some_and(|action| is_dangling_dest(doc, action));
    if open_dangling && let Ok(catalog) = doc.get_dictionary_mut(catalog_id) {
        catalog.remove(b"OpenAction");
    }
}

/// Keep only the items of the array at `path` (dictionary keys from `holder`, following
/// references) for which `keep` is true. Missing array = no-op.
fn retain_array(
    doc: &mut Document,
    holder: ObjectId,
    path: &[&[u8]],
    keep: impl Fn(&Document, &Object) -> bool,
) {
    let Some((id, rest)) = locate(doc, holder, path) else {
        return;
    };
    let Some(items) = object_at(doc, id, rest).and_then(|a| a.as_array().ok()) else {
        return;
    };
    let kept: Vec<Object> = items.iter().filter(|o| keep(doc, o)).cloned().collect();
    if kept.len() == items.len() {
        return;
    }
    let mut target = doc.get_object_mut(id).ok();
    for key in rest {
        target = target
            .and_then(|o| o.as_dict_mut().ok())
            .and_then(|d| d.get_mut(key).ok());
    }
    if let Some(target) = target {
        *target = Object::Array(kept);
    }
}

/// Follow `path` from `holder`, jumping through references. Returns the last indirect
/// object reached and the keys still to walk inside it (direct objects only).
fn locate<'p>(
    doc: &Document,
    holder: ObjectId,
    path: &'p [&'p [u8]],
) -> Option<(ObjectId, &'p [&'p [u8]])> {
    let mut id = holder;
    let mut start = 0;
    'outer: loop {
        let mut object = doc.get_object(id).ok()?;
        for (i, key) in path[start..].iter().enumerate() {
            let value = object.as_dict().ok()?.get(key).ok()?;
            if let Object::Reference(next) = value {
                id = *next;
                start += i + 1;
                continue 'outer;
            }
            object = value;
        }
        return Some((id, &path[start..]));
    }
}

/// Object reached from indirect object `id` through direct dictionary `keys`.
fn object_at<'a>(doc: &'a Document, id: ObjectId, keys: &[&[u8]]) -> Option<&'a Object> {
    let mut object = doc.get_object(id).ok()?;
    for key in keys {
        object = object.as_dict().ok()?.get(key).ok()?;
    }
    Some(object)
}

fn null_refs(object: &mut Object, removed: &HashSet<ObjectId>) {
    match object {
        Object::Reference(id) if removed.contains(id) => *object = Object::Null,
        Object::Array(items) => items.iter_mut().for_each(|item| null_refs(item, removed)),
        Object::Dictionary(dict) => dict.iter_mut().for_each(|(_, v)| null_refs(v, removed)),
        Object::Stream(stream) => stream
            .dict
            .iter_mut()
            .for_each(|(_, v)| null_refs(v, removed)),
        _ => {}
    }
}
