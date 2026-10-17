use std::collections::{BTreeSet, HashSet};

use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary, text_string};

use crate::info::{info_dict, make_outline_dests_explicit, outline_first, text_entry};
use crate::pdf::{check_page, dict_entry, flatten, load, page_ids, rebuild, save, set_pages};
use crate::{Error, Result};

/// Concatenate documents in order. Each input's outline is kept, nested under a bookmark per input.
///
/// The per-input bookmark is named after the input's Title metadata, else "Document N".
pub fn merge(pdfs: &[&[u8]]) -> Result<Vec<u8>> {
    if pdfs.is_empty() {
        return Err(Error::InvalidArgument("nothing to merge".into()));
    }
    let mut out = Document::with_version("1.7");
    let root = out.new_object_id();
    let mut pages = Vec::new();
    let mut sections = Vec::new();
    for (index, bytes) in pdfs.iter().enumerate() {
        let input = absorb(&mut out, bytes, index)?;
        if let Some(&first_page) = input.pages.first() {
            sections.push(Section {
                title: input.title,
                first_page,
                children: input.outline,
            });
        }
        pages.extend(input.pages);
    }
    if pages.is_empty() {
        return Err(Error::InvalidArgument(
            "merged documents have no pages".into(),
        ));
    }
    out.objects
        .insert(root, Object::Dictionary(Dictionary::new()));
    set_pages(&mut out, root, &pages)?;
    let mut catalog = dictionary! { "Type" => "Catalog", "Pages" => root };
    if let Some(outlines) = build_merge_outline(&mut out, &sections) {
        catalog.set("Outlines", outlines);
    }
    let catalog_id = out.add_object(catalog);
    out.trailer.set("Root", catalog_id);
    save(&mut out)
}

/// Pages of `first` and `second` taken in turn, the leftovers of the longer one at the end.
/// With `reverse_second`, `second` is read from its last page: the backs of sheets scanned
/// as a pile come out in reverse.
pub fn interleave(first: &[u8], second: &[u8], reverse_second: bool) -> Result<Vec<u8>> {
    let a = crate::page_count(first)?;
    let b = crate::page_count(second)?;
    let merged = merge(&[first, second])?;
    let mut seconds: Vec<usize> = (a..a + b).collect();
    if reverse_second {
        seconds.reverse();
    }
    let mut order = Vec::with_capacity(a + b);
    for index in 0..a.max(b) {
        if index < a {
            order.push(index);
        }
        if let Some(&page) = seconds.get(index) {
            order.push(page);
        }
    }
    reorder_pages(&merged, &order)
}

/// New document with only `pages`, in the given order (duplicates allowed).
pub fn extract_pages(pdf: &[u8], pages: &[usize]) -> Result<Vec<u8>> {
    if pages.is_empty() {
        return Err(Error::InvalidArgument("no pages selected".into()));
    }
    let mut doc = load(pdf)?;
    rebuild(&mut doc, pages)?;
    save(&mut doc)
}

/// Remove `pages`. Error if that would remove every page.
pub fn delete_pages(pdf: &[u8], pages: &[usize]) -> Result<Vec<u8>> {
    if pages.is_empty() {
        return Err(Error::InvalidArgument("no pages selected".into()));
    }
    let mut doc = load(pdf)?;
    let count = doc.page_iter().count();
    let doomed = validated_set(pages, count)?;
    let keep: Vec<usize> = (0..count).filter(|i| !doomed.contains(i)).collect();
    if keep.is_empty() {
        return Err(Error::InvalidArgument("cannot delete every page".into()));
    }
    rebuild(&mut doc, &keep)?;
    save(&mut doc)
}

/// Add `degrees` (multiple of 90, may be negative) to each page's /Rotate.
pub fn rotate_pages(pdf: &[u8], pages: &[usize], degrees: i32) -> Result<Vec<u8>> {
    if degrees % 90 != 0 {
        return Err(Error::InvalidArgument(format!(
            "rotation {degrees} is not a multiple of 90"
        )));
    }
    if pages.is_empty() {
        return Err(Error::InvalidArgument("no pages selected".into()));
    }
    let mut doc = load(pdf)?;
    let (_, ids) = flatten(&mut doc)?;
    for index in validated_set(pages, ids.len())? {
        let page = doc.get_dictionary_mut(ids[index])?;
        let current = page.get(b"Rotate").and_then(Object::as_i64).unwrap_or(0);
        page.set("Rotate", (current + i64::from(degrees)).rem_euclid(360));
    }
    save(&mut doc)
}

/// `new_order` must be a permutation of `0..page_count`.
pub fn reorder_pages(pdf: &[u8], new_order: &[usize]) -> Result<Vec<u8>> {
    let mut doc = load(pdf)?;
    let count = doc.page_iter().count();
    let unique: HashSet<usize> = new_order.iter().copied().collect();
    let is_permutation =
        new_order.len() == count && unique.len() == count && unique.iter().all(|&i| i < count);
    if !is_permutation {
        return Err(Error::InvalidArgument(format!(
            "order is not a permutation of 0..{count}"
        )));
    }
    rebuild(&mut doc, new_order)?;
    save(&mut doc)
}

/// A new document made of one empty page.
pub fn blank_document(width_pt: f32, height_pt: f32) -> Result<Vec<u8>> {
    let valid_size = |v: f32| v.is_finite() && v > 0.0;
    if !valid_size(width_pt) || !valid_size(height_pt) {
        return Err(Error::InvalidArgument(format!(
            "invalid page size {width_pt}x{height_pt}"
        )));
    }
    let mut doc = Document::with_version("1.7");
    let root = doc.new_object_id();
    let contents = doc.add_object(Stream::new(Dictionary::new(), Vec::new()));
    let page = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => root,
        "MediaBox" => vec![0.into(), 0.into(), width_pt.into(), height_pt.into()],
        "Contents" => contents,
        "Resources" => Dictionary::new(),
    });
    let pages = dictionary! {
        "Type" => "Pages",
        "Kids" => vec![Object::Reference(page)],
        "Count" => 1,
    };
    doc.objects.insert(root, Object::Dictionary(pages));
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => root });
    doc.trailer.set("Root", catalog);
    save(&mut doc)
}

/// Insert an empty page so it lands at index `at` (`at == page_count` appends).
pub fn insert_blank_page(pdf: &[u8], at: usize, width_pt: f32, height_pt: f32) -> Result<Vec<u8>> {
    let valid_size = |v: f32| v.is_finite() && v > 0.0;
    if !valid_size(width_pt) || !valid_size(height_pt) {
        return Err(Error::InvalidArgument(format!(
            "invalid page size {width_pt}x{height_pt}"
        )));
    }
    let mut doc = load(pdf)?;
    let (root, mut pages) = flatten(&mut doc)?;
    check_page(at, pages.len() + 1)?;
    let contents = doc.add_object(Stream::new(Dictionary::new(), Vec::new()));
    let page = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => root,
        "MediaBox" => vec![0.into(), 0.into(), width_pt.into(), height_pt.into()],
        "Contents" => contents,
        "Resources" => Dictionary::new(),
    });
    pages.insert(at, page);
    set_pages(&mut doc, root, &pages)?;
    save(&mut doc)
}

/// Insert all pages of `other` starting at index `at`.
pub fn insert_pdf(pdf: &[u8], at: usize, other: &[u8]) -> Result<Vec<u8>> {
    let mut doc = load(pdf)?;
    let (root, mut pages) = flatten(&mut doc)?;
    check_page(at, pages.len() + 1)?;
    let inserted = absorb(&mut doc, other, 0)?.pages;
    pages.splice(at..at, inserted);
    set_pages(&mut doc, root, &pages)?;
    save(&mut doc)
}

/// Split into chunks of `n` pages (last chunk may be shorter).
pub fn split_every(pdf: &[u8], n: usize) -> Result<Vec<Vec<u8>>> {
    if n == 0 {
        return Err(Error::InvalidArgument(
            "chunk size must be at least 1".into(),
        ));
    }
    let doc = load(pdf)?;
    let indices: Vec<usize> = (0..doc.page_iter().count()).collect();
    indices
        .chunks(n)
        .map(|chunk| extract_from(&doc, chunk))
        .collect()
}

/// Split so a new part starts at each index in `starts` (index 0 implied, duplicates ignored).
pub fn split_at(pdf: &[u8], starts: &[usize]) -> Result<Vec<Vec<u8>>> {
    let doc = load(pdf)?;
    let count = doc.page_iter().count();
    let mut bounds = validated_set(starts, count)?;
    bounds.insert(0);
    let bounds: Vec<usize> = bounds.into_iter().chain([count]).collect();
    bounds
        .windows(2)
        .map(|w| extract_from(&doc, &(w[0]..w[1]).collect::<Vec<_>>()))
        .collect()
}

/// One split part: a copy of the already-parsed `doc` reduced to `pages` (no re-parse).
fn extract_from(doc: &Document, pages: &[usize]) -> Result<Vec<u8>> {
    let mut part = doc.clone();
    rebuild(&mut part, pages)?;
    save(&mut part)
}

/// Deduplicated, range-checked page indices.
fn validated_set(pages: &[usize], count: usize) -> Result<BTreeSet<usize>> {
    pages
        .iter()
        .map(|&page| check_page(page, count).map(|()| page))
        .collect()
}

/// One input copied into a target document.
struct Absorbed {
    title: String,
    pages: Vec<ObjectId>,
    /// (First, Last) of the input's top-level outline items.
    outline: Option<(ObjectId, ObjectId)>,
}

/// Copy every object of `bytes` into `target` under fresh ids. The caller links the pages.
fn absorb(target: &mut Document, bytes: &[u8], index: usize) -> Result<Absorbed> {
    let mut doc = load(bytes)?;
    flatten(&mut doc)?;
    make_outline_dests_explicit(&mut doc);
    doc.prune_objects();
    doc.renumber_objects_with(target.max_id + 1);
    let title = info_dict(&doc)
        .and_then(|info| text_entry(&doc, info, b"Title"))
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| format!("Document {}", index + 1));
    let outline = outline_bounds(&doc);
    let pages = page_ids(&doc);
    let max_id = doc.objects.keys().map(|id| id.0).max().unwrap_or(0);
    target.objects.extend(doc.objects);
    target.max_id = target.max_id.max(max_id);
    Ok(Absorbed {
        title,
        pages,
        outline,
    })
}

fn outline_bounds(doc: &Document) -> Option<(ObjectId, ObjectId)> {
    let first = outline_first(doc)?;
    let outlines = dict_entry(doc, doc.catalog().ok()?, b"Outlines")?;
    let last = outlines
        .get(b"Last")
        .and_then(Object::as_reference)
        .unwrap_or(first);
    Some((first, last))
}

struct Section {
    title: String,
    first_page: ObjectId,
    children: Option<(ObjectId, ObjectId)>,
}

/// One top-level bookmark per input; each input's own outline nested beneath it.
fn build_merge_outline(doc: &mut Document, sections: &[Section]) -> Option<ObjectId> {
    if sections.is_empty() {
        return None;
    }
    let outlines_id = doc.new_object_id();
    let ids: Vec<ObjectId> = sections.iter().map(|_| doc.new_object_id()).collect();
    for (i, section) in sections.iter().enumerate() {
        let mut item = dictionary! {
            "Title" => text_string(&section.title),
            "Parent" => outlines_id,
            "Dest" => vec![section.first_page.into(), "Fit".into()],
        };
        if i > 0 {
            item.set("Prev", ids[i - 1]);
        }
        if let Some(&next) = ids.get(i + 1) {
            item.set("Next", next);
        }
        if let Some((child_first, child_last)) = section.children {
            let count = reparent_siblings(doc, child_first, ids[i]);
            item.set("First", child_first);
            item.set("Last", child_last);
            // Negative count = collapsed by default.
            item.set("Count", -(count as i64));
        }
        doc.objects.insert(ids[i], Object::Dictionary(item));
    }
    doc.objects.insert(
        outlines_id,
        Object::Dictionary(dictionary! {
            "Type" => "Outlines",
            "First" => ids[0],
            "Last" => ids[ids.len() - 1],
            "Count" => ids.len() as i64,
        }),
    );
    Some(outlines_id)
}

/// Point /Parent of `first` and its /Next siblings at `parent`; returns how many there are.
fn reparent_siblings(doc: &mut Document, first: ObjectId, parent: ObjectId) -> usize {
    let mut visited = HashSet::new();
    let mut next = Some(first);
    while let Some(id) = next {
        if !visited.insert(id) {
            break;
        }
        let Ok(dict) = doc.get_dictionary_mut(id) else {
            break;
        };
        dict.set("Parent", parent);
        next = dict.get(b"Next").and_then(Object::as_reference).ok();
    }
    visited.len()
}
