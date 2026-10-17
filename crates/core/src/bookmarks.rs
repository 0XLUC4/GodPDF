//! Bookmarks written back to a document, and a contents page made from them.

use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary, text_string};

use crate::annotate::{num, win_ansi_hex};
use crate::helvetica::line_width;
use crate::info::{OutlineItem, outline};
use crate::pdf::{check_page, flatten, load, page_ids, save, set_pages};
use crate::stamp::{View, check_text, helvetica};
use crate::{Error, Result};

/// Space around the text of a contents page.
const MARGIN: f32 = 56.0;
const HEADING_SIZE: f32 = 20.0;
const ENTRY_SIZE: f32 = 11.0;
const LEADING: f32 = 20.0;
/// Shift to the right of each level of nesting.
const INDENT: f32 = 16.0;
const MAX_DEPTH: usize = 64;

/// Replace the bookmarks of the document with `items`. An empty list removes them all.
///
/// Items whose `page` is `None` are kept as headings that go nowhere.
pub fn write_outline(pdf: &[u8], items: &[OutlineItem]) -> Result<Vec<u8>> {
    let mut doc = load(pdf)?;
    let pages = page_ids(&doc);
    let outlines = if items.is_empty() {
        None
    } else {
        let outlines_id = doc.new_object_id();
        let (first, last) = write_level(&mut doc, items, outlines_id, &pages, 0)?;
        doc.objects.insert(
            outlines_id,
            Object::Dictionary(dictionary! {
                "Type" => "Outlines",
                "First" => first,
                "Last" => last,
                "Count" => items.len() as i64,
            }),
        );
        Some(outlines_id)
    };
    let catalog = doc.catalog_mut()?;
    match outlines {
        Some(id) => catalog.set("Outlines", id),
        None => drop(catalog.remove(b"Outlines")),
    }
    save(&mut doc)
}

/// Write one level of siblings under `parent`; returns its first and last ids.
fn write_level(
    doc: &mut Document,
    items: &[OutlineItem],
    parent: ObjectId,
    pages: &[ObjectId],
    depth: usize,
) -> Result<(ObjectId, ObjectId)> {
    if depth > MAX_DEPTH {
        return Err(Error::InvalidArgument(
            "bookmarks are nested too deep".into(),
        ));
    }
    let ids: Vec<ObjectId> = items.iter().map(|_| doc.new_object_id()).collect();
    for (index, item) in items.iter().enumerate() {
        let mut dict = dictionary! {
            "Title" => text_string(&item.title),
            "Parent" => parent,
        };
        if let Some(page) = item.page {
            check_page(page, pages.len())?;
            dict.set("Dest", vec![pages[page].into(), "Fit".into()]);
        }
        if index > 0 {
            dict.set("Prev", ids[index - 1]);
        }
        if let Some(&next) = ids.get(index + 1) {
            dict.set("Next", next);
        }
        if !item.children.is_empty() {
            let (first, last) = write_level(doc, &item.children, ids[index], pages, depth + 1)?;
            dict.set("First", first);
            dict.set("Last", last);
            // Negative: closed until the reader opens it.
            dict.set("Count", -(item.children.len() as i64));
        }
        doc.objects.insert(ids[index], Object::Dictionary(dict));
    }
    Ok((ids[0], ids[ids.len() - 1]))
}

/// A line of the contents page.
struct Entry {
    depth: usize,
    title: String,
    page: usize,
}

fn entries(items: &[OutlineItem], depth: usize, out: &mut Vec<Entry>) {
    for item in items {
        if let Some(page) = item.page {
            out.push(Entry {
                depth,
                title: item.title.trim().to_owned(),
                page,
            });
        }
        entries(&item.children, depth + 1, out);
    }
}

/// Put a contents page (or as many as needed) in front of the document: the title of each
/// bookmark, dots, and its page number. A click on a line goes to its page. A bookmark
/// named `heading` leads back to it.
pub fn contents_page(pdf: &[u8], heading: &str) -> Result<Vec<u8>> {
    let mut lines = Vec::new();
    entries(&outline(pdf)?, 0, &mut lines);
    if lines.is_empty() {
        return Err(Error::InvalidArgument(
            "the document has no bookmarks to list".into(),
        ));
    }
    check_text(heading)?;
    for line in &lines {
        check_text(&line.title)?;
    }
    let mut doc = load(pdf)?;
    let (root, pages) = flatten(&mut doc)?;
    let view = View::of(&doc, pages[0]);
    let (width, height) = if view.width < 200.0 || view.height < 200.0 {
        (595.0, 842.0)
    } else {
        (view.width, view.height)
    };
    let first_top = height - MARGIN - HEADING_SIZE - LEADING;
    let per_page = (((first_top - MARGIN) / LEADING).floor() as usize).max(1);
    let sheets = lines.len().div_ceil(per_page);

    let regular = doc.add_object(helvetica());
    let mut bold = helvetica();
    bold.set("BaseFont", "Helvetica-Bold");
    let bold = doc.add_object(bold);
    let resources = dictionary! {
        "Font" => dictionary! { "GodPdfHelv" => regular, "GodPdfHelvB" => bold },
    };
    let dot = line_width(".", ENTRY_SIZE);
    let mut new_pages = Vec::with_capacity(sheets);
    for (sheet, chunk) in lines.chunks(per_page).enumerate() {
        let page_id = doc.new_object_id();
        let mut ops = String::new();
        if sheet == 0 {
            ops += &format!(
                "BT /GodPdfHelvB {} Tf 0 g {} {} Td <{}> Tj ET\n",
                num(HEADING_SIZE),
                num(MARGIN),
                num(height - MARGIN - HEADING_SIZE),
                win_ansi_hex(heading)
            );
        }
        let mut links = Vec::new();
        for (row, line) in chunk.iter().enumerate() {
            let y = first_top - row as f32 * LEADING;
            let x = MARGIN + INDENT * line.depth.min(8) as f32;
            let number = (line.page + sheets + 1).to_string();
            let number_x = width - MARGIN - line_width(&number, ENTRY_SIZE);
            let title = fit(&line.title, number_x - x - 24.0);
            let title_end = x + line_width(&title, ENTRY_SIZE);
            let dots = ((number_x - title_end - 12.0) / dot).floor().max(0.0) as usize;
            // Regular weight: the widths measured above are those of Helvetica.
            let size = num(ENTRY_SIZE);
            ops += &format!(
                "BT /GodPdfHelv {size} Tf 0 g {} {} Td <{}> Tj ET\n\
                 BT /GodPdfHelv {size} Tf 0.55 g {} {} Td <{}> Tj ET\n\
                 BT /GodPdfHelv {size} Tf 0 g {} {} Td <{}> Tj ET\n",
                num(x),
                num(y),
                win_ansi_hex(&title),
                num(number_x - 6.0 - dots as f32 * dot),
                num(y),
                win_ansi_hex(&".".repeat(dots)),
                num(number_x),
                num(y),
                win_ansi_hex(&number),
            );
            let rect = [x, y - 5.0, width - MARGIN, y + ENTRY_SIZE + 3.0];
            links.push(Object::Reference(doc.add_object(dictionary! {
                "Type" => "Annot",
                "Subtype" => "Link",
                "Rect" => rect.map(Object::Real).to_vec(),
                "Border" => vec![0.into(), 0.into(), 0.into()],
                "Dest" => vec![pages[line.page].into(), "Fit".into()],
                "P" => page_id,
            })));
        }
        let contents = doc.add_object(Stream::new(Dictionary::new(), ops.into_bytes()));
        doc.objects.insert(
            page_id,
            Object::Dictionary(dictionary! {
                "Type" => "Page",
                "Parent" => root,
                "MediaBox" => vec![0.into(), 0.into(), width.into(), height.into()],
                "Contents" => contents,
                "Resources" => resources.clone(),
                "Annots" => links,
            }),
        );
        new_pages.push(page_id);
    }
    let first_sheet = new_pages[0];
    new_pages.extend(pages);
    set_pages(&mut doc, root, &new_pages)?;
    prepend_bookmark(&mut doc, heading, first_sheet)?;
    save(&mut doc)
}

/// `text`, cut with an ellipsis so it is at most `width` points wide.
fn fit(text: &str, width: f32) -> String {
    if line_width(text, ENTRY_SIZE) <= width {
        return text.to_owned();
    }
    let mut cut = text.to_owned();
    while !cut.is_empty() && line_width(&format!("{cut}…"), ENTRY_SIZE) > width {
        cut.pop();
    }
    format!("{}…", cut.trim_end())
}

/// A first top-level bookmark named `title` that leads to `page`.
fn prepend_bookmark(doc: &mut Document, title: &str, page: ObjectId) -> Result<()> {
    let outlines_id = match doc.catalog()?.get(b"Outlines")?.clone() {
        Object::Reference(id) => id,
        // Held in the catalog itself: give it an id of its own.
        direct => {
            let id = doc.add_object(direct);
            doc.catalog_mut()?.set("Outlines", id);
            id
        }
    };
    let outlines = doc.get_dictionary(outlines_id)?;
    let old_first = outlines.get(b"First").and_then(Object::as_reference).ok();
    let count = outlines.get(b"Count").and_then(Object::as_i64).unwrap_or(0);
    let mut item = dictionary! {
        "Title" => text_string(title),
        "Parent" => outlines_id,
        "Dest" => vec![page.into(), "Fit".into()],
    };
    if let Some(next) = old_first {
        item.set("Next", next);
    }
    let item_id = doc.add_object(item);
    if let Some(next) = old_first {
        doc.get_dictionary_mut(next)?.set("Prev", item_id);
    }
    let outlines = doc.get_dictionary_mut(outlines_id)?;
    outlines.set("First", item_id);
    if old_first.is_none() {
        outlines.set("Last", item_id);
    }
    outlines.set("Count", count.max(0) + 1);
    Ok(())
}
