//! Redaction: what a page shows under some areas is taken out of the file, then the areas are
//! painted black.

use std::collections::{HashMap, HashSet};

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

use crate::annotate::{Rect, annots_entries, normalized, read_rect, set_annots};
use crate::pdf::{check_page, deref, dict_entry, inherited, load, page_ids, save};
use crate::retype::{IDENTITY, Matrix, PageFont, moved, page_fonts, then};
use crate::{Error, Result};

/// How far a glyph reaches below and above its baseline, in font sizes. Generous on purpose:
/// a glyph that only nearly touches an area is better gone than left readable.
const DESCENT: f32 = 0.25;
const ASCENT: f32 = 0.9;

/// Redact `areas` of `page` (PDF points, unrotated page space): what is drawn there is taken
/// out of the page's content, the annotations there are removed, and each area is painted
/// opaque black.
///
/// What it does, and what it does not:
/// - Text: every glyph whose box touches an area is removed from the content; the glyphs
///   around it keep their places. Boxes come from the font's widths (/Widths, /W, or the
///   standard 14 metrics): a font without widths, or a Type 3 font, can get wrong boxes.
/// - Images, drawn (`Do`) or inline (`BI`), that touch an area are removed whole, not cut.
/// - Form XObjects that touch an area are removed whole without looking inside them, which
///   can take out more than the area.
/// - Vector paths and shadings are not removed: they are only painted over.
/// - Annotations whose /Rect touches an area are removed, with their popups. The values of
///   form fields stay in /AcroForm.
/// - Only this page is changed: the same text elsewhere in the file (other pages, metadata,
///   bookmarks, attachments, earlier revisions of an incrementally saved file) stays.
pub fn redact(pdf: &[u8], page: usize, areas: &[Rect]) -> Result<Vec<u8>> {
    if areas.is_empty() {
        return Err(Error::InvalidArgument("nothing to redact".into()));
    }
    let areas = areas
        .iter()
        .map(|area| normalized(*area))
        .collect::<Result<Vec<_>>>()?;
    let mut doc = load(pdf)?;
    let pages = page_ids(&doc);
    check_page(page, pages.len())?;
    let page = pages[page];
    let fonts = page_fonts(&doc, page);
    let xobjects = xobject_boxes(&doc, page);
    let operations = Content::decode(&doc.get_page_content(page))?.operations;
    let mut kept = vec![Operation::new("q", Vec::new())];
    kept.extend(strip(operations, &areas, &fonts, &xobjects));
    kept.push(Operation::new("Q", Vec::new()));
    kept.push(Operation::new("g", vec![Object::Integer(0)]));
    for area in &areas {
        let operands = [area.x0, area.y0, area.x1 - area.x0, area.y1 - area.y0];
        kept.push(Operation::new("re", operands.map(Object::Real).to_vec()));
    }
    kept.push(Operation::new("f", Vec::new()));
    // A stream of its own: the old one may be shared with other pages.
    let bytes = Content { operations: kept }.encode()?;
    let stream = doc.add_object(Stream::new(Dictionary::new(), bytes));
    doc.get_dictionary_mut(page)?
        .set("Contents", Object::Reference(stream));
    remove_annotations(&mut doc, page, &areas)?;
    save(&mut doc)
}

/// Whether two boxes overlap (touching edges do not count).
fn touches(one: Rect, other: Rect) -> bool {
    one.x0 < other.x1 && other.x0 < one.x1 && one.y0 < other.y1 && other.y0 < one.y1
}

/// The page-space box around `rect` once `matrix` has moved it.
fn placed(matrix: Matrix, rect: Rect) -> Rect {
    let corners = [
        (rect.x0, rect.y0),
        (rect.x1, rect.y0),
        (rect.x0, rect.y1),
        (rect.x1, rect.y1),
    ]
    .map(|corner| moved(matrix, corner));
    let xs = corners.map(|(x, _)| x);
    let ys = corners.map(|(_, y)| y);
    Rect {
        x0: xs.into_iter().fold(f32::INFINITY, f32::min),
        y0: ys.into_iter().fold(f32::INFINITY, f32::min),
        x1: xs.into_iter().fold(f32::NEG_INFINITY, f32::max),
        y1: ys.into_iter().fold(f32::NEG_INFINITY, f32::max),
    }
}

fn touches_any(areas: &[Rect], rect: Rect) -> bool {
    areas.iter().any(|area| touches(*area, rect))
}

const UNIT: Rect = Rect {
    x0: 0.0,
    y0: 0.0,
    x1: 1.0,
    y1: 1.0,
};

/// The box each XObject of the page covers in its own space: an image the unit square, a
/// form its /BBox moved by its /Matrix. Keyed by resource name.
fn xobject_boxes(doc: &Document, page: ObjectId) -> HashMap<Vec<u8>, (Rect, Matrix)> {
    let resources = inherited(doc, page, b"Resources");
    let resources = resources
        .as_ref()
        .and_then(|found| deref(doc, found))
        .and_then(|found| found.as_dict().ok());
    let Some(xobjects) = resources.and_then(|resources| dict_entry(doc, resources, b"XObject"))
    else {
        return HashMap::new();
    };
    let numbers = |dict: &Dictionary, key: &[u8]| -> Option<Vec<f32>> {
        deref(doc, dict.get(key).ok()?)?
            .as_array()
            .ok()?
            .iter()
            .map(|item| deref(doc, item)?.as_float().ok())
            .collect()
    };
    xobjects
        .iter()
        .filter_map(|(name, xobject)| {
            let dict = &deref(doc, xobject)?.as_stream().ok()?.dict;
            let subtype = dict.get(b"Subtype").ok()?.as_name().ok()?;
            let placement = match subtype {
                b"Image" => (UNIT, IDENTITY),
                b"Form" => {
                    let &[x0, y0, x1, y1] = numbers(dict, b"BBox")?.as_slice() else {
                        return None;
                    };
                    let matrix = numbers(dict, b"Matrix")
                        .and_then(|values| <Matrix>::try_from(values).ok())
                        .unwrap_or(IDENTITY);
                    (normalized(Rect { x0, y0, x1, y1 }).ok()?, matrix)
                }
                _ => return None,
            };
            Some((name.clone(), placement))
        })
        .collect()
}

/// What `q` saves and `Q` brings back, as far as placing text and images goes.
#[derive(Clone)]
struct State {
    ctm: Matrix,
    font: Vec<u8>,
    font_size: f32,
    leading: f32,
    char_spacing: f32,
    word_spacing: f32,
    scaling: f32,
    rise: f32,
}

/// `operations` without what they draw in `areas`.
fn strip(
    operations: Vec<Operation>,
    areas: &[Rect],
    fonts: &HashMap<Vec<u8>, PageFont>,
    xobjects: &HashMap<Vec<u8>, (Rect, Matrix)>,
) -> Vec<Operation> {
    let mut kept = Vec::with_capacity(operations.len());
    let mut saved: Vec<State> = Vec::new();
    let mut state = State {
        ctm: IDENTITY,
        font: Vec::new(),
        font_size: 0.0,
        leading: 0.0,
        char_spacing: 0.0,
        word_spacing: 0.0,
        scaling: 1.0,
        rise: 0.0,
    };
    // The text matrix, and the one of the start of the line.
    let (mut text, mut line) = (IDENTITY, IDENTITY);
    for operation in operations {
        let number = |index: usize| {
            let operand = operation.operands.get(index);
            operand
                .and_then(|operand| operand.as_float().ok())
                .unwrap_or(0.0)
        };
        let next_line =
            |line: Matrix, leading: f32| then([1.0, 0.0, 0.0, 1.0, 0.0, -leading], line);
        // What replaces the operation; `None` keeps it as it is.
        let mut replaced: Option<Vec<Operation>> = None;
        match operation.operator.as_str() {
            "q" => saved.push(state.clone()),
            "Q" => state = saved.pop().unwrap_or(state.clone()),
            "cm" => state.ctm = then([0, 1, 2, 3, 4, 5].map(number), state.ctm),
            "BT" => (text, line) = (IDENTITY, IDENTITY),
            "Tf" => {
                state.font = operation
                    .operands
                    .first()
                    .and_then(|name| name.as_name().ok())
                    .map(<[u8]>::to_vec)
                    .unwrap_or_default();
                state.font_size = number(1);
            }
            "TL" => state.leading = number(0),
            "Tc" => state.char_spacing = number(0),
            "Tw" => state.word_spacing = number(0),
            "Tz" => state.scaling = number(0) / 100.0,
            "Ts" => state.rise = number(0),
            "Td" | "TD" => {
                if operation.operator == "TD" {
                    state.leading = -number(1);
                }
                line = then([1.0, 0.0, 0.0, 1.0, number(0), number(1)], line);
                text = line;
            }
            "Tm" => {
                line = [0, 1, 2, 3, 4, 5].map(number);
                text = line;
            }
            "T*" => {
                line = next_line(line, state.leading);
                text = line;
            }
            "Tj" | "TJ" | "'" | "\"" => {
                let mut prefix = Vec::new();
                if operation.operator == "\"" {
                    state.word_spacing = number(0);
                    state.char_spacing = number(1);
                    prefix.push(Operation::new("Tw", vec![Object::Real(number(0))]));
                    prefix.push(Operation::new("Tc", vec![Object::Real(number(1))]));
                }
                if matches!(operation.operator.as_str(), "'" | "\"") {
                    line = next_line(line, state.leading);
                    text = line;
                    prefix.push(Operation::new("T*", Vec::new()));
                }
                let items: &[Object] =
                    match (operation.operator.as_str(), operation.operands.last()) {
                        ("TJ", Some(Object::Array(items))) => items,
                        (_, Some(last)) => std::slice::from_ref(last),
                        (_, None) => &[],
                    };
                let font = fonts.get(&state.font);
                if let Some(shown) = show(items, &state, &mut text, areas, font) {
                    prefix.push(Operation::new("TJ", vec![Object::Array(shown)]));
                    replaced = Some(prefix);
                }
            }
            "Do" => {
                let name = operation
                    .operands
                    .first()
                    .and_then(|name| name.as_name().ok());
                if let Some((rect, matrix)) = name.and_then(|name| xobjects.get(name))
                    && touches_any(areas, placed(then(*matrix, state.ctm), *rect))
                {
                    replaced = Some(Vec::new());
                }
            }
            "BI" if touches_any(areas, placed(state.ctm, UNIT)) => replaced = Some(Vec::new()),
            _ => {}
        }
        match replaced {
            Some(operations) => kept.extend(operations),
            None => kept.push(operation),
        }
    }
    kept
}

/// Walk the strings and gaps of a text show from `text` on, moving `text` past them. Returns
/// them as a `TJ` array with the glyphs that touch `areas` turned into gaps of their width,
/// or `None` when none does.
fn show(
    items: &[Object],
    state: &State,
    text: &mut Matrix,
    areas: &[Rect],
    font: Option<&PageFont>,
) -> Option<Vec<Object>> {
    let (size, scaling) = (state.font_size, state.scaling);
    // Thousandths of an em that move the text by `advance`, as a TJ number says them.
    let gap = |advance: f32| {
        let scale = size * scaling;
        (scale != 0.0).then(|| -advance * 1000.0 / scale)
    };
    let mut shown: Vec<Object> = Vec::with_capacity(items.len());
    let mut removed = false;
    let push_gap = |shown: &mut Vec<Object>, amount: f32| match shown.last_mut() {
        Some(Object::Real(previous)) => *previous += amount,
        _ => shown.push(Object::Real(amount)),
    };
    for item in items {
        let Object::String(bytes, format) = item else {
            let amount = item.as_float().unwrap_or(0.0);
            *text = then(
                [1.0, 0.0, 0.0, 1.0, -amount / 1000.0 * size * scaling, 0.0],
                *text,
            );
            shown.push(item.clone());
            continue;
        };
        let glyphs = font
            .map(|font| font.widths.glyphs(bytes))
            .unwrap_or_default();
        let mut at = 0;
        for (length, units, space) in glyphs {
            let width = units / 1000.0 * size;
            let spacing = state.char_spacing + if space { state.word_spacing } else { 0.0 };
            let advance = (width + spacing) * scaling;
            let glyph = Rect {
                x0: 0.0,
                y0: state.rise - DESCENT * size,
                x1: width * scaling,
                y1: state.rise + ASCENT * size,
            };
            let code = &bytes[at..(at + length).min(bytes.len())];
            if touches_any(areas, placed(then(*text, state.ctm), glyph)) {
                removed = true;
                if let Some(amount) = gap(advance) {
                    push_gap(&mut shown, amount);
                }
            } else {
                match shown.last_mut() {
                    Some(Object::String(run, _)) => run.extend_from_slice(code),
                    _ => shown.push(Object::String(code.to_vec(), *format)),
                }
            }
            *text = then([1.0, 0.0, 0.0, 1.0, advance, 0.0], *text);
            at += length;
        }
    }
    removed.then_some(shown)
}

/// Remove from /Annots of `page` every annotation that touches `areas`, and their popups.
fn remove_annotations(doc: &mut Document, page: ObjectId, areas: &[Rect]) -> Result<()> {
    let entries = annots_entries(doc, page)?;
    fn dict_of<'a>(doc: &'a Document, entry: &'a Object) -> Option<&'a Dictionary> {
        deref(doc, entry)?.as_dict().ok()
    }
    let hit = |entry: &Object| {
        dict_of(doc, entry)
            .and_then(|dict| read_rect(doc, dict))
            .is_some_and(|rect| touches_any(areas, rect))
    };
    let gone: HashSet<ObjectId> = entries
        .iter()
        .filter(|entry| hit(entry))
        .filter_map(|entry| entry.as_reference().ok())
        .collect();
    let parent_gone = |entry: &Object| {
        dict_of(doc, entry)
            .and_then(|dict| dict.get(b"Parent").ok())
            .and_then(|parent| parent.as_reference().ok())
            .is_some_and(|parent| gone.contains(&parent))
    };
    let kept: Vec<Object> = entries
        .iter()
        .filter(|entry| !hit(entry) && !parent_gone(entry))
        .cloned()
        .collect();
    if kept.len() != entries.len() {
        set_annots(doc, page, kept)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect {
        x0: 10.0,
        y0: 10.0,
        x1: 20.0,
        y1: 20.0,
    };

    #[test]
    fn boxes_touch_only_when_they_overlap() {
        assert!(touches(AREA, AREA.translated(5.0, 5.0)));
        assert!(!touches(AREA, AREA.translated(10.0, 0.0)));
        assert!(!touches(AREA, AREA.translated(0.0, -11.0)));
    }

    #[test]
    fn placed_box_follows_the_matrix() {
        // 100 x 50 image drawn at (30, 40), then turned a quarter.
        let image = then(
            [100.0, 0.0, 0.0, 50.0, 30.0, 40.0],
            [0.0, 1.0, -1.0, 0.0, 0.0, 0.0],
        );
        let rect = placed(image, UNIT);
        assert_eq!(
            rect,
            Rect {
                x0: -90.0,
                y0: 30.0,
                x1: -40.0,
                y1: 130.0
            }
        );
    }
}
