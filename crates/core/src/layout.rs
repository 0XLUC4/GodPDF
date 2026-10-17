//! New page layouts made from the pages of a document: several pages per sheet, booklets,
//! another paper size, pages cut in two or joined into one, margins cropped.
//!
//! Each page is drawn as a form (its content and resources, as displayed) on new pages.
//! Bookmarks and links that led to a page lead to the first new page that shows it.
//! The comments of a redrawn page are not carried over.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};

use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};

use crate::annotate::num;
use crate::pdf::{check_page, flatten, load, save, set_pages};
use crate::stamp::View;
use crate::{Error, Result};

/// One page drawn on a new page: which, where (lower-left corner) and how big.
struct Placed {
    page: usize,
    x: f32,
    y: f32,
    scale: f32,
}

enum Out {
    /// The page, as it was.
    Keep(usize),
    Sheet {
        width: f32,
        height: f32,
        placed: Vec<Placed>,
    },
}

/// A page made into a form, and its size as displayed.
struct Form {
    id: ObjectId,
    width: f32,
    height: f32,
}

/// Put `per_sheet` pages (2, 4, 6, 8, 9 or 16) on each sheet, in reading order. A sheet is
/// as big as the first page; for 2, 6 and 8 it lies on its side.
pub fn pages_per_sheet(pdf: &[u8], per_sheet: usize) -> Result<Vec<u8>> {
    let (cols, rows) = match per_sheet {
        2 => (2, 1),
        4 => (2, 2),
        6 => (3, 2),
        8 => (4, 2),
        9 => (3, 3),
        16 => (4, 4),
        _ => {
            return Err(Error::InvalidArgument(format!(
                "{per_sheet} pages per sheet is not offered"
            )));
        }
    };
    compose(pdf, |sizes| {
        let (w, h) = sizes[0];
        let (width, height) = if cols > rows {
            (w.max(h), w.min(h))
        } else {
            (w, h)
        };
        let order: Vec<Option<usize>> = (0..sizes.len()).map(Some).collect();
        grid(sizes, &order, cols, rows, width, height)
    })
}

/// Arrange the pages to be printed on both sides, folded and stapled in the middle: two
/// pages per sheet side, in booklet order. Blank pages fill up to a multiple of four.
pub fn booklet(pdf: &[u8]) -> Result<Vec<u8>> {
    compose(pdf, |sizes| {
        let count = sizes.len().div_ceil(4) * 4;
        let page = |index: usize| (index < sizes.len()).then_some(index);
        let mut order = Vec::with_capacity(count);
        for side in 0..count / 2 {
            let (left, right) = if side % 2 == 0 {
                (count - 1 - side, side)
            } else {
                (side, count - 1 - side)
            };
            order.push(page(left));
            order.push(page(right));
        }
        let (w, h) = sizes[0];
        grid(sizes, &order, 2, 1, w * 2.0, h)
    })
}

/// Lay `order` (None = an empty cell) out `cols` × `rows` per sheet of `width` × `height`.
fn grid(
    sizes: &[(f32, f32)],
    order: &[Option<usize>],
    cols: usize,
    rows: usize,
    width: f32,
    height: f32,
) -> Vec<Out> {
    let (cell_w, cell_h) = (width / cols as f32, height / rows as f32);
    order
        .chunks(cols * rows)
        .map(|chunk| {
            let placed = chunk
                .iter()
                .enumerate()
                .filter_map(|(slot, page)| {
                    let page = (*page)?;
                    let (w, h) = sizes[page];
                    let scale = (cell_w / w).min(cell_h / h);
                    let (col, row) = (slot % cols, slot / cols);
                    Some(Placed {
                        page,
                        x: col as f32 * cell_w + (cell_w - w * scale) / 2.0,
                        y: height - (row + 1) as f32 * cell_h + (cell_h - h * scale) / 2.0,
                        scale,
                    })
                })
                .collect();
            Out::Sheet {
                width,
                height,
                placed,
            }
        })
        .collect()
}

/// Give `pages` a new size in points, upright: what they show is scaled to fit and centred.
pub fn resize_pages(pdf: &[u8], pages: &[usize], width: f32, height: f32) -> Result<Vec<u8>> {
    let valid = |v: f32| v.is_finite() && (36.0..=14_400.0).contains(&v);
    if !valid(width) || !valid(height) {
        return Err(Error::InvalidArgument(format!(
            "invalid page size {width} × {height}"
        )));
    }
    let chosen = chosen(pdf, pages)?;
    compose(pdf, |sizes| {
        (0..sizes.len())
            .map(|page| {
                if !chosen.contains(&page) {
                    return Out::Keep(page);
                }
                let (w, h) = sizes[page];
                let scale = (width / w).min(height / h);
                Out::Sheet {
                    width,
                    height,
                    placed: vec![Placed {
                        page,
                        x: (width - w * scale) / 2.0,
                        y: (height - h * scale) / 2.0,
                        scale,
                    }],
                }
            })
            .collect()
    })
}

/// Cut each of `pages` in two: left and right halves (`across`), or top and bottom.
pub fn halve_pages(pdf: &[u8], pages: &[usize], across: bool) -> Result<Vec<u8>> {
    let chosen = chosen(pdf, pages)?;
    compose(pdf, |sizes| {
        let mut out = Vec::new();
        for (page, &(w, h)) in sizes.iter().enumerate() {
            if !chosen.contains(&page) {
                out.push(Out::Keep(page));
                continue;
            }
            // The page drawn twice, shifted so each half fills its own sheet.
            let halves = if across {
                [(w / 2.0, h, 0.0, 0.0), (w / 2.0, h, -w / 2.0, 0.0)]
            } else {
                [(w, h / 2.0, 0.0, -h / 2.0), (w, h / 2.0, 0.0, 0.0)]
            };
            for (width, height, x, y) in halves {
                out.push(Out::Sheet {
                    width,
                    height,
                    placed: vec![Placed {
                        page,
                        x,
                        y,
                        scale: 1.0,
                    }],
                });
            }
        }
        out
    })
}

/// Every page one under the other, on one long page.
pub fn join_pages(pdf: &[u8]) -> Result<Vec<u8>> {
    compose(pdf, |sizes| {
        let width = sizes.iter().map(|size| size.0).fold(0.0, f32::max);
        let height: f32 = sizes.iter().map(|size| size.1).sum();
        let mut top = height;
        let placed = sizes
            .iter()
            .enumerate()
            .map(|(page, &(w, h))| {
                top -= h;
                Placed {
                    page,
                    x: (width - w) / 2.0,
                    y: top,
                    scale: 1.0,
                }
            })
            .collect();
        vec![Out::Sheet {
            width,
            height,
            placed,
        }]
    })
}

/// Hide `margins` (left, top, right, bottom, in points of the page as displayed) of `pages`.
/// Nothing is deleted: the crop box shrinks.
pub fn crop_pages(pdf: &[u8], pages: &[usize], margins: [f32; 4]) -> Result<Vec<u8>> {
    if margins.iter().any(|m| !m.is_finite() || *m < 0.0) {
        return Err(Error::InvalidArgument("margins cannot be negative".into()));
    }
    if pages.is_empty() {
        return Err(Error::InvalidArgument("no pages selected".into()));
    }
    let mut doc = load(pdf)?;
    let (_, ids) = flatten(&mut doc)?;
    let [left, top, right, bottom] = margins;
    for &page in pages {
        check_page(page, ids.len())?;
        let view = View::of(&doc, ids[page]);
        if left + right >= view.width - 1.0 || top + bottom >= view.height - 1.0 {
            return Err(Error::InvalidArgument(format!(
                "the margins are wider than page {}",
                page + 1
            )));
        }
        let m = view.to_page();
        let [(ax, ay), (bx, by)] = [(left, bottom), (view.width - right, view.height - top)]
            .map(|(x, y)| (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]));
        let crop = [ax.min(bx), ay.min(by), ax.max(bx), ay.max(by)];
        doc.get_dictionary_mut(ids[page])?
            .set("CropBox", crop.map(Object::Real).to_vec());
    }
    save(&mut doc)
}

/// `pages` checked against the document.
fn chosen(pdf: &[u8], pages: &[usize]) -> Result<HashSet<usize>> {
    if pages.is_empty() {
        return Err(Error::InvalidArgument("no pages selected".into()));
    }
    let count = crate::page_count(pdf)?;
    pages
        .iter()
        .map(|&page| check_page(page, count).map(|()| page))
        .collect()
}

/// Rebuild the document from the pages `layout` makes out of the displayed page sizes.
fn compose(pdf: &[u8], layout: impl FnOnce(&[(f32, f32)]) -> Vec<Out>) -> Result<Vec<u8>> {
    let mut doc = load(pdf)?;
    let (root, pages) = flatten(&mut doc)?;
    let views: Vec<View> = pages.iter().map(|&id| View::of(&doc, id)).collect();
    let sizes: Vec<(f32, f32)> = views.iter().map(|v| (v.width, v.height)).collect();
    if sizes.iter().any(|&(w, h)| w < 1.0 || h < 1.0) {
        return Err(Error::InvalidArgument("a page has no size".into()));
    }
    let outs = layout(&sizes);
    let mut forms: HashMap<usize, Form> = HashMap::new();
    let mut new_pages = Vec::with_capacity(outs.len());
    // Where each old page went first: links and bookmarks follow it there.
    let mut moved: HashMap<ObjectId, ObjectId> = HashMap::new();
    let mut kept = HashSet::new();
    for out in outs {
        match out {
            Out::Keep(page) => {
                kept.insert(pages[page]);
                new_pages.push(pages[page]);
            }
            Out::Sheet {
                width,
                height,
                placed,
            } => {
                let sheet = doc.new_object_id();
                let mut ops = String::new();
                let mut xobjects = Dictionary::new();
                for place in &placed {
                    let form = match forms.entry(place.page) {
                        Entry::Occupied(entry) => entry.into_mut(),
                        Entry::Vacant(entry) => entry.insert(page_form(
                            &mut doc,
                            pages[place.page],
                            &views[place.page],
                        )?),
                    };
                    let name = format!("GodPdfPage{}", place.page);
                    xobjects.set(name.as_bytes(), form.id);
                    ops += &format!(
                        "q {s} 0 0 {s} {} {} cm 0 0 {} {} re W n /{name} Do Q\n",
                        num(place.x),
                        num(place.y),
                        num(form.width),
                        num(form.height),
                        s = num(place.scale),
                    );
                    moved.entry(pages[place.page]).or_insert(sheet);
                }
                let contents = doc.add_object(Stream::new(Dictionary::new(), ops.into_bytes()));
                doc.objects.insert(
                    sheet,
                    Object::Dictionary(dictionary! {
                        "Type" => "Page",
                        "Parent" => root,
                        "MediaBox" => vec![0.into(), 0.into(), width.into(), height.into()],
                        "Contents" => contents,
                        "Resources" => dictionary! { "XObject" => xobjects },
                    }),
                );
                new_pages.push(sheet);
            }
        }
    }
    let redrawn: HashSet<ObjectId> = pages.into_iter().filter(|id| !kept.contains(id)).collect();
    // The redrawn pages go; what pointed at them points at their new place.
    for id in &redrawn {
        doc.objects.remove(id);
    }
    for object in doc.objects.values_mut() {
        retarget(object, &moved, &redrawn);
    }
    set_pages(&mut doc, root, &new_pages)?;
    save(&mut doc)
}

/// The page as a form: its content in displayed space, `0 0 width height`.
fn page_form(doc: &mut Document, page: ObjectId, view: &View) -> Result<Form> {
    let content = doc.get_page_content(page);
    let resources = doc
        .get_dictionary(page)?
        .get(b"Resources")
        .cloned()
        .unwrap_or(Object::Dictionary(Dictionary::new()));
    let [a, b, c, d, e, f] = view.to_page();
    let det = a * d - b * c;
    let inverse = [
        d / det,
        -b / det,
        -c / det,
        a / det,
        (c * f - d * e) / det,
        (b * e - a * f) / det,
    ];
    let form = Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => view.corners.map(Object::Real).to_vec(),
            "Matrix" => inverse.map(Object::Real).to_vec(),
            "Resources" => resources,
        },
        content,
    );
    Ok(Form {
        id: doc.add_object(form),
        width: view.width,
        height: view.height,
    })
}

/// Point references to a redrawn page at its new page, or at nothing when it has none.
fn retarget(object: &mut Object, moved: &HashMap<ObjectId, ObjectId>, gone: &HashSet<ObjectId>) {
    match object {
        Object::Reference(id) if gone.contains(id) => {
            *object = moved
                .get(id)
                .map_or(Object::Null, |&to| Object::Reference(to));
        }
        Object::Array(items) => items
            .iter_mut()
            .for_each(|item| retarget(item, moved, gone)),
        Object::Dictionary(dict) => dict
            .iter_mut()
            .for_each(|(_, value)| retarget(value, moved, gone)),
        Object::Stream(stream) => stream
            .dict
            .iter_mut()
            .for_each(|(_, value)| retarget(value, moved, gone)),
        _ => {}
    }
}
