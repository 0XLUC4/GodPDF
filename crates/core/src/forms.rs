//! Form fields: read, filled in, and new ones added.
//!
//! Only the fields that can be typed or clicked are handled: text, check boxes, radio
//! buttons and lists. A filled field gets a new appearance, so every viewer shows the value.

use std::collections::{HashMap, HashSet};

use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary, text_string};

use crate::annotate::{annots_entries, num, set_annots, win_ansi_hex};
use crate::helvetica::line_width;
use crate::info::text_entry;
use crate::pdf::{check_page, deref, dict_entry, load, page_ids, save};
use crate::stamp::{check_text, helvetica};
use crate::{Error, Rect, Result};

const MAX_DEPTH: usize = 32;
/// Field flags (PDF 32000 12.7.4).
const READ_ONLY: i64 = 1;
const MULTILINE: i64 = 1 << 12;
const RADIO: i64 = 1 << 15;
const PUSH_BUTTON: i64 = 1 << 16;
/// Space between the edge of a text field and its text.
const PADDING: f32 = 2.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldKind {
    Text {
        multiline: bool,
    },
    CheckBox,
    /// One of `options` (the names of its buttons).
    Radio {
        options: Vec<String>,
    },
    /// One of `options`, from a list.
    Choice {
        options: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldValue {
    Text(String),
    Checked(bool),
    /// The option chosen in a radio group or a list; empty for none.
    Choice(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormField {
    /// Full name, parts joined by dots: what [`fill_form`] takes.
    pub name: String,
    pub kind: FieldKind,
    pub value: FieldValue,
    /// Page of its first widget.
    pub page: Option<usize>,
    pub read_only: bool,
}

/// A field that holds a value, with what it inherits from its parents.
struct Terminal {
    id: ObjectId,
    name: String,
    kind: Vec<u8>,
    flags: i64,
    widgets: Vec<ObjectId>,
}

/// Every field of the document, in the order of the form.
pub fn form_fields(pdf: &[u8]) -> Result<Vec<FormField>> {
    let doc = load(pdf)?;
    let pages = widget_pages(&doc);
    Ok(terminals(&doc)
        .into_iter()
        .filter_map(|field| read_field(&doc, field, &pages))
        .collect())
}

fn read_field(
    doc: &Document,
    field: Terminal,
    pages: &HashMap<ObjectId, usize>,
) -> Option<FormField> {
    let dict = doc.get_dictionary(field.id).ok()?;
    let name_value = dict
        .get(b"V")
        .ok()
        .and_then(|v| deref(doc, v))
        .and_then(|v| v.as_name().ok())
        .map(|n| String::from_utf8_lossy(n).into_owned())
        .filter(|n| n != "Off");
    let (kind, value) = match field.kind.as_slice() {
        b"Tx" => (
            FieldKind::Text {
                multiline: field.flags & MULTILINE != 0,
            },
            FieldValue::Text(text_entry(doc, dict, b"V").unwrap_or_default()),
        ),
        b"Btn" if field.flags & PUSH_BUTTON != 0 => return None,
        b"Btn" if field.flags & RADIO != 0 => {
            let mut options = Vec::new();
            for &widget in &field.widgets {
                for name in on_states(doc, widget) {
                    if !options.contains(&name) {
                        options.push(name);
                    }
                }
            }
            (
                FieldKind::Radio { options },
                FieldValue::Choice(name_value.unwrap_or_default()),
            )
        }
        b"Btn" => (
            FieldKind::CheckBox,
            FieldValue::Checked(name_value.is_some()),
        ),
        b"Ch" => (
            FieldKind::Choice {
                options: choice_options(doc, dict),
            },
            FieldValue::Choice(text_entry(doc, dict, b"V").unwrap_or_default()),
        ),
        _ => return None,
    };
    Some(FormField {
        page: field
            .widgets
            .iter()
            .find_map(|widget| pages.get(widget).copied()),
        name: field.name,
        kind,
        value,
        read_only: field.flags & READ_ONLY != 0,
    })
}

/// Give each named field its value and a new appearance.
pub fn fill_form(pdf: &[u8], values: &[(String, FieldValue)]) -> Result<Vec<u8>> {
    let mut doc = load(pdf)?;
    let fields: HashMap<String, Terminal> = terminals(&doc)
        .into_iter()
        .map(|field| (field.name.clone(), field))
        .collect();
    let font = form_font(&mut doc)?;
    for (field_name, value) in values {
        let field = fields
            .get(field_name)
            .ok_or_else(|| Error::InvalidArgument(format!("no field “{field_name}”")))?;
        match value {
            FieldValue::Text(text) => {
                check_text(text)?;
                set_entry(&mut doc, field.id, "V", text_string(text))?;
                let multiline = field.flags & MULTILINE != 0;
                for &widget in &field.widgets {
                    text_appearance(&mut doc, widget, text, multiline, font)?;
                }
            }
            FieldValue::Choice(option) if field.kind == b"Ch" => {
                check_text(option)?;
                set_entry(&mut doc, field.id, "V", text_string(option))?;
                for &widget in &field.widgets {
                    text_appearance(&mut doc, widget, option, false, font)?;
                }
            }
            FieldValue::Choice(option) => {
                let state = if option.is_empty() { "Off" } else { option };
                set_entry(&mut doc, field.id, "V", name(state))?;
                for &widget in &field.widgets {
                    let shown = if on_states(&doc, widget).contains(option) {
                        state
                    } else {
                        "Off"
                    };
                    set_entry(&mut doc, widget, "AS", name(shown))?;
                }
            }
            FieldValue::Checked(checked) => {
                let on = field
                    .widgets
                    .iter()
                    .flat_map(|&widget| on_states(&doc, widget))
                    .next()
                    .unwrap_or_else(|| "Yes".to_owned());
                let state = name(if *checked { &on } else { "Off" });
                set_entry(&mut doc, field.id, "V", state.clone())?;
                for &widget in &field.widgets {
                    set_entry(&mut doc, widget, "AS", state.clone())?;
                }
            }
        }
    }
    save(&mut doc)
}

fn name(text: &str) -> Object {
    Object::Name(text.as_bytes().to_vec())
}

fn set_entry(doc: &mut Document, id: ObjectId, key: &str, value: Object) -> Result<()> {
    doc.get_dictionary_mut(id)?.set(key, value);
    Ok(())
}

/// A field to add.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewField {
    Text,
    CheckBox,
}

/// Add an empty field over `rect` of `page`, named `name` (or `name 2`, `name 3`… when the
/// name is taken).
pub fn add_form_field(
    pdf: &[u8],
    page: usize,
    rect: Rect,
    kind: NewField,
    name: &str,
) -> Result<Vec<u8>> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::InvalidArgument("the field has no name".into()));
    }
    let (x0, x1) = (rect.x0.min(rect.x1), rect.x0.max(rect.x1));
    let (y0, y1) = (rect.y0.min(rect.y1), rect.y0.max(rect.y1));
    if x1 - x0 < 4.0 || y1 - y0 < 4.0 {
        return Err(Error::InvalidArgument("the field is too small".into()));
    }
    let mut doc = load(pdf)?;
    let ids = page_ids(&doc);
    check_page(page, ids.len())?;
    let page_id = ids[page];
    let taken: HashSet<String> = terminals(&doc).into_iter().map(|f| f.name).collect();
    let name = (1..)
        .map(|n| match n {
            1 => name.to_owned(),
            n => format!("{name} {n}"),
        })
        .find(|candidate| !taken.contains(candidate))
        .unwrap_or_else(|| name.to_owned());
    form_font(&mut doc)?;
    let (w, h) = (x1 - x0, y1 - y0);
    let border = border_ops(w, h);
    let mut widget = dictionary! {
        "Type" => "Annot",
        "Subtype" => "Widget",
        "T" => text_string(&name),
        "Rect" => vec![x0.into(), y0.into(), x1.into(), y1.into()],
        "P" => page_id,
        "F" => 4,
        "MK" => dictionary! { "BC" => vec![0.55.into(), 0.55.into(), 0.55.into()] },
    };
    match kind {
        NewField::Text => {
            widget.set("FT", "Tx");
            widget.set("DA", text_string("/Helv 0 Tf 0 g"));
            let look = doc.add_object(form_stream(w, h, &border, Dictionary::new()));
            widget.set("AP", dictionary! { "N" => look });
        }
        NewField::CheckBox => {
            widget.set("FT", "Btn");
            widget.set("V", "Off");
            widget.set("AS", "Off");
            let tick = format!(
                "{border}q 0 g 1.5 w 1 J 1 j {} {} m {} {} l {} {} l S Q\n",
                num(w * 0.22),
                num(h * 0.52),
                num(w * 0.42),
                num(h * 0.26),
                num(w * 0.8),
                num(h * 0.78)
            );
            let on = doc.add_object(form_stream(w, h, &tick, Dictionary::new()));
            let off = doc.add_object(form_stream(w, h, &border, Dictionary::new()));
            widget.set(
                "AP",
                dictionary! { "N" => dictionary! { "Yes" => on, "Off" => off } },
            );
        }
    }
    let widget = doc.add_object(widget);
    let mut annots = annots_entries(&doc, page_id)?;
    annots.push(Object::Reference(widget));
    set_annots(&mut doc, page_id, annots)?;
    let form_id = acro_form(&mut doc)?;
    let form = doc.get_dictionary_mut(form_id)?;
    let mut fields = form
        .get(b"Fields")
        .and_then(Object::as_array)
        .cloned()
        .unwrap_or_default();
    fields.push(Object::Reference(widget));
    form.set("Fields", fields);
    save(&mut doc)
}

fn border_ops(w: f32, h: f32) -> String {
    format!(
        "q 0.55 0.55 0.55 RG 1 w 0.5 0.5 {} {} re S Q\n",
        num(w - 1.0),
        num(h - 1.0)
    )
}

fn form_stream(w: f32, h: f32, ops: &str, resources: Dictionary) -> Stream {
    Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), w.into(), h.into()],
            "Resources" => resources,
        },
        ops.as_bytes().to_vec(),
    )
}

/// The AcroForm dictionary as an object of its own, made when missing.
fn acro_form(doc: &mut Document) -> Result<ObjectId> {
    match doc.catalog()?.get(b"AcroForm").ok().cloned() {
        Some(Object::Reference(id)) if doc.get_dictionary(id).is_ok() => Ok(id),
        Some(Object::Dictionary(dict)) => {
            let id = doc.add_object(dict);
            doc.catalog_mut()?.set("AcroForm", id);
            Ok(id)
        }
        _ => {
            let id = doc.add_object(dictionary! { "Fields" => Vec::<Object>::new() });
            doc.catalog_mut()?.set("AcroForm", id);
            Ok(id)
        }
    }
}

/// The font fields are written with: /Helv of the form resources, added when missing.
fn form_font(doc: &mut Document) -> Result<ObjectId> {
    let form_id = acro_form(doc)?;
    let form = doc.get_dictionary(form_id)?;
    let existing = dict_entry(doc, form, b"DR")
        .and_then(|resources| dict_entry(doc, resources, b"Font"))
        .and_then(|fonts| fonts.get(b"Helv").ok())
        .and_then(|font| font.as_reference().ok());
    if let Some(font) = existing {
        return Ok(font);
    }
    let font = doc.add_object(helvetica());
    let form = doc.get_dictionary(form_id)?;
    let mut resources = dict_entry(doc, form, b"DR").cloned().unwrap_or_default();
    let mut fonts = dict_entry(doc, &resources, b"Font")
        .cloned()
        .unwrap_or_default();
    fonts.set("Helv", font);
    resources.set("Font", fonts);
    let form = doc.get_dictionary_mut(form_id)?;
    form.set("DR", resources);
    if !form.has(b"DA") {
        form.set("DA", text_string("/Helv 0 Tf 0 g"));
    }
    Ok(font)
}

/// Draw `text` as the look of `widget`: one line, or wrapped lines from the top.
fn text_appearance(
    doc: &mut Document,
    widget: ObjectId,
    text: &str,
    multiline: bool,
    font: ObjectId,
) -> Result<()> {
    let dict = doc.get_dictionary(widget)?;
    let rect: Vec<f32> = dict
        .get(b"Rect")
        .ok()
        .and_then(|r| deref(doc, r))
        .and_then(|r| r.as_array().ok())
        .map(|r| {
            r.iter()
                .filter_map(|n| deref(doc, n)?.as_float().ok())
                .collect()
        })
        .unwrap_or_default();
    let [a, b, c, d] = rect[..] else {
        return Ok(());
    };
    let (w, h) = ((c - a).abs(), (d - b).abs());
    let (size, color) = da_look(doc, dict);
    let size = match size {
        size if size > 0.0 => size,
        _ if multiline => 10.0,
        _ => (h * 0.65).clamp(4.0, 12.0),
    };
    let align = dict.get(b"Q").and_then(Object::as_i64).unwrap_or(0);
    let bordered = dict_entry(doc, dict, b"MK").is_some_and(|mk| mk.has(b"BC"));
    let lines = if multiline {
        wrap(text, size, w - 2.0 * PADDING)
    } else {
        vec![text.to_owned()]
    };
    let mut ops = if bordered {
        border_ops(w, h)
    } else {
        String::new()
    };
    ops += &format!(
        "/Tx BMC q 1 1 {} {} re W n BT /Helv {} Tf {color}\n",
        num(w - 2.0),
        num(h - 2.0),
        num(size)
    );
    let leading = size * 1.15;
    for (index, line) in lines.iter().enumerate() {
        let width = line_width(line, size);
        let x = match align {
            1 => (w - width) / 2.0,
            2 => w - PADDING - width,
            _ => PADDING,
        };
        let y = if multiline {
            h - PADDING - size * 0.85 - index as f32 * leading
        } else {
            (h - size * 0.7) / 2.0
        };
        ops += &format!(
            "1 0 0 1 {} {} Tm <{}> Tj\n",
            num(x),
            num(y),
            win_ansi_hex(line)
        );
    }
    ops += "ET Q EMC\n";
    let resources = dictionary! { "Font" => dictionary! { "Helv" => font } };
    let look = doc.add_object(form_stream(w, h, &ops, resources));
    set_entry(
        doc,
        widget,
        "AP",
        Object::Dictionary(dictionary! { "N" => look }),
    )
}

/// Font size and colour operator of a field's /DA ("/Helv 10 Tf 0 g"); 0 = automatic.
fn da_look(doc: &Document, dict: &Dictionary) -> (f32, String) {
    let da = text_entry(doc, dict, b"DA").unwrap_or_default();
    let parts: Vec<&str> = da.split_whitespace().collect();
    let size = parts
        .iter()
        .position(|&p| p == "Tf")
        .and_then(|at| parts.get(at.checked_sub(1)?)?.parse::<f32>().ok())
        .filter(|size| size.is_finite() && *size >= 0.0)
        .unwrap_or(0.0);
    let color = [("g", 1), ("rg", 3), ("k", 4)]
        .iter()
        .find_map(|&(op, count)| {
            let at = parts.iter().position(|&p| p == op)?;
            let operands = parts.get(at.checked_sub(count)?..at)?;
            operands
                .iter()
                .all(|v| v.parse::<f32>().is_ok())
                .then(|| format!("{} {op}", operands.join(" ")))
        })
        .unwrap_or_else(|| "0 g".to_owned());
    (size, color)
}

/// Words of `text` in lines no wider than `width`; line breaks of the text are kept.
fn wrap(text: &str, size: f32, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split(' ') {
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if !line.is_empty() && line_width(&candidate, size) > width {
                lines.push(std::mem::replace(&mut line, word.to_owned()));
            } else {
                line = candidate;
            }
        }
        lines.push(line);
    }
    lines
}

/// The names of the "on" appearances of a check box or radio button.
fn on_states(doc: &Document, widget: ObjectId) -> Vec<String> {
    let Ok(dict) = doc.get_dictionary(widget) else {
        return Vec::new();
    };
    dict_entry(doc, dict, b"AP")
        .and_then(|ap| dict_entry(doc, ap, b"N"))
        .map(|states| {
            states
                .iter()
                .map(|(name, _)| String::from_utf8_lossy(name).into_owned())
                .filter(|name| name != "Off")
                .collect()
        })
        .unwrap_or_default()
}

fn choice_options(doc: &Document, dict: &Dictionary) -> Vec<String> {
    let Some(options) = dict
        .get(b"Opt")
        .ok()
        .and_then(|o| deref(doc, o))
        .and_then(|o| o.as_array().ok())
    else {
        return Vec::new();
    };
    options
        .iter()
        .filter_map(|option| match deref(doc, option)? {
            // [export value, shown text]: the value is what the field holds.
            Object::Array(pair) => lopdf::decode_text_string(deref(doc, pair.first()?)?).ok(),
            other => lopdf::decode_text_string(other).ok(),
        })
        .collect()
}

/// Page index of each widget listed in the /Annots of a page.
fn widget_pages(doc: &Document) -> HashMap<ObjectId, usize> {
    let mut pages = HashMap::new();
    for (index, page) in page_ids(doc).into_iter().enumerate() {
        for entry in annots_entries(doc, page).unwrap_or_default() {
            if let Object::Reference(id) = entry {
                pages.entry(id).or_insert(index);
            }
        }
    }
    pages
}

/// What a field passes on to the fields under it.
#[derive(Clone, Default)]
struct Inherited {
    name: String,
    kind: Vec<u8>,
    flags: i64,
}

/// Fields that hold a value, found from /AcroForm /Fields.
fn terminals(doc: &Document) -> Vec<Terminal> {
    let Some(fields) = doc
        .catalog()
        .ok()
        .and_then(|catalog| dict_entry(doc, catalog, b"AcroForm"))
        .and_then(|form| form.get(b"Fields").ok())
        .and_then(|fields| deref(doc, fields))
        .and_then(|fields| fields.as_array().ok())
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for field in fields {
        if let Object::Reference(id) = field {
            walk(doc, *id, &Inherited::default(), 0, &mut out, &mut seen);
        }
    }
    out
}

fn walk(
    doc: &Document,
    id: ObjectId,
    parent: &Inherited,
    depth: usize,
    out: &mut Vec<Terminal>,
    seen: &mut HashSet<ObjectId>,
) {
    if depth > MAX_DEPTH || !seen.insert(id) {
        return;
    }
    let Ok(dict) = doc.get_dictionary(id) else {
        return;
    };
    let name = match text_entry(doc, dict, b"T") {
        Some(part) if parent.name.is_empty() => part,
        Some(part) => format!("{}.{part}", parent.name),
        None => parent.name.clone(),
    };
    let here = Inherited {
        name,
        kind: dict
            .get(b"FT")
            .and_then(Object::as_name)
            .map_or_else(|_| parent.kind.clone(), <[u8]>::to_vec),
        flags: dict
            .get(b"Ff")
            .ok()
            .and_then(|f| deref(doc, f)?.as_i64().ok())
            .unwrap_or(parent.flags),
    };
    let kids: Vec<ObjectId> = dict
        .get(b"Kids")
        .ok()
        .and_then(|k| deref(doc, k))
        .and_then(|k| k.as_array().ok())
        .map(|kids| kids.iter().filter_map(|k| k.as_reference().ok()).collect())
        .unwrap_or_default();
    // Kids with their own names are fields; nameless kids are the widgets of this one.
    let named_kids = kids
        .iter()
        .any(|&kid| doc.get_dictionary(kid).is_ok_and(|kid| kid.has(b"T")));
    if named_kids {
        for kid in kids {
            walk(doc, kid, &here, depth + 1, out, seen);
        }
        return;
    }
    out.push(Terminal {
        id,
        name: here.name,
        kind: here.kind,
        flags: here.flags,
        widgets: if kids.is_empty() { vec![id] } else { kids },
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect { x0, y0, x1, y1 }
    }

    fn form() -> Vec<u8> {
        let pdf = crate::blank_document(400.0, 400.0).unwrap();
        let text = NewField::Text;
        let pdf = add_form_field(&pdf, 0, rect(20.0, 300.0, 220.0, 320.0), text, "Name").unwrap();
        let pdf = add_form_field(&pdf, 0, rect(20.0, 260.0, 220.0, 280.0), text, "Name").unwrap();
        let check = NewField::CheckBox;
        add_form_field(&pdf, 0, rect(20.0, 220.0, 34.0, 234.0), check, "Agree").unwrap()
    }

    #[test]
    fn new_fields_are_listed_with_free_names() {
        let fields = form_fields(&form()).unwrap();
        let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["Name", "Name 2", "Agree"]);
        assert_eq!(fields[2].kind, FieldKind::CheckBox);
        assert_eq!(fields[2].value, FieldValue::Checked(false));
        assert_eq!(fields[0].page, Some(0));
    }

    #[test]
    fn fields_are_filled_and_read_back() {
        let values = [
            ("Name".to_owned(), FieldValue::Text("Ada Lovelace".into())),
            ("Agree".to_owned(), FieldValue::Checked(true)),
        ];
        let filled = fill_form(&form(), &values).unwrap();
        let fields = form_fields(&filled).unwrap();
        assert_eq!(fields[0].value, FieldValue::Text("Ada Lovelace".into()));
        assert_eq!(fields[1].value, FieldValue::Text(String::new()));
        assert_eq!(fields[2].value, FieldValue::Checked(true));
        let unknown = [("Nope".to_owned(), FieldValue::Checked(true))];
        assert!(fill_form(&filled, &unknown).is_err());
    }

    #[test]
    fn long_text_wraps() {
        assert_eq!(wrap("one two three", 10.0, 40.0), ["one two", "three"]);
        assert_eq!(wrap("a\nb", 10.0, 100.0), ["a", "b"]);
    }

    #[test]
    fn da_gives_size_and_colour() {
        let dict = dictionary! { "DA" => text_string("/Helv 9 Tf 1 0 0 rg") };
        let doc = Document::new();
        assert_eq!(da_look(&doc, &dict), (9.0, "1 0 0 rg".to_owned()));
    }
}
