//! Changing the text a page draws: the words of a line are taken out of the page's content
//! and new ones written where they started.
//!
//! The new words are set in Helvetica: the font of the old ones usually holds only the
//! letters the document used, and cannot write others.

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat, dictionary};

use std::collections::HashMap;

use crate::annotate::Rect;
use crate::fonts::Font;
use crate::helvetica::win_ansi;
use crate::pdf::{check_page, deref, dict_entry, inherited, load, page_ids, save};
use crate::{Error, Result};

/// Name of the font the new words are written with, in the resources of the page.
const FONT: &str = "GodPDFHelv";
/// How far outside `area` a word may start and still belong to it, in points.
const SLACK: f32 = 1.5;

/// A transformation, as PDF writes it: `[a b c d e f]`.
type Matrix = [f32; 6];
const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `first`, then `second`.
fn then(first: Matrix, second: Matrix) -> Matrix {
    let ([a, b, c, d, e, f], n) = (first, second);
    [
        a * n[0] + b * n[2],
        a * n[1] + b * n[3],
        c * n[0] + d * n[2],
        c * n[1] + d * n[3],
        e * n[0] + f * n[2] + n[4],
        e * n[1] + f * n[3] + n[5],
    ]
}

/// What `q` saves and `Q` brings back, as far as text goes.
#[derive(Clone)]
struct State {
    ctm: Matrix,
    fill: Vec<Operation>,
    font: Vec<u8>,
    font_size: f32,
    leading: f32,
    char_spacing: f32,
    word_spacing: f32,
    scaling: f32,
}

/// The text taken out of a page.
struct Removed {
    /// Where its leftmost word started, in page space.
    origin: (f32, f32),
    /// Its height on the page, in points.
    size: f32,
    fill: Vec<Operation>,
}

/// Take out of `operations` every piece of text that starts in `area`. Returns what it was
/// like, or `None` when the page draws no text there.
fn take_out(
    operations: &mut Vec<Operation>,
    area: Rect,
    fonts: &HashMap<Vec<u8>, Widths>,
) -> Result<Option<Removed>> {
    let mut kept = Vec::with_capacity(operations.len());
    let mut saved: Vec<State> = Vec::new();
    let mut state = State {
        ctm: IDENTITY,
        fill: Vec::new(),
        font: Vec::new(),
        font_size: 0.0,
        leading: 0.0,
        char_spacing: 0.0,
        word_spacing: 0.0,
        scaling: 1.0,
    };
    // The text matrix, and the one of the start of the line.
    let (mut text, mut line) = (IDENTITY, IDENTITY);
    let mut removed: Option<Removed> = None;
    for operation in operations.drain(..) {
        let number = |index: usize| {
            let operand = operation.operands.get(index);
            operand
                .and_then(|operand| operand.as_float().ok())
                .unwrap_or(0.0)
        };
        let next_line =
            |line: Matrix, leading: f32| then([1.0, 0.0, 0.0, 1.0, 0.0, -leading], line);
        let mut shows = false;
        let mut shown: Vec<&Object> = Vec::new();
        match operation.operator.as_str() {
            "q" => saved.push(state.clone()),
            "Q" => state = saved.pop().unwrap_or(state),
            "cm" => {
                let matrix = [0, 1, 2, 3, 4, 5].map(number);
                state.ctm = then(matrix, state.ctm);
            }
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
            "'" | "\"" => {
                if operation.operator == "\"" {
                    state.word_spacing = number(0);
                    state.char_spacing = number(1);
                }
                line = next_line(line, state.leading);
                text = line;
                shows = true;
                shown.extend(operation.operands.last());
            }
            "Tj" => {
                shows = true;
                shown.extend(operation.operands.first());
            }
            "TJ" => {
                shows = true;
                if let Some(Ok(items)) = operation.operands.first().map(Object::as_array) {
                    shown.extend(items);
                }
            }
            "g" | "rg" | "k" | "cs" => state.fill = vec![operation.clone()],
            "sc" | "scn" => {
                state.fill.retain(|set| set.operator == "cs");
                state.fill.push(operation.clone());
            }
            _ => {}
        }
        let advance = shows.then(|| {
            let widths = fonts.get(&state.font);
            shown
                .iter()
                .map(|item| match item {
                    Object::String(bytes, _) => widths.map_or(0.0, |widths| {
                        widths.displacement(
                            bytes,
                            state.font_size,
                            state.char_spacing,
                            state.word_spacing,
                        )
                    }),
                    other => -other.as_float().unwrap_or(0.0) / 1000.0 * state.font_size,
                })
                .sum::<f32>()
        });
        let drawn = then(text, state.ctm);
        if let Some(advance) = advance {
            text = then([1.0, 0.0, 0.0, 1.0, advance * state.scaling, 0.0], text);
        }
        let (x, y) = (drawn[4], drawn[5]);
        let inside = (area.x0 - SLACK..=area.x1 + SLACK).contains(&x)
            && (area.y0 - SLACK..=area.y1 + SLACK).contains(&y);
        if !(shows && inside) {
            kept.push(operation);
            continue;
        }
        // Text that is turned or slanted is not something a straight line replaces.
        let scale = drawn[0].abs().max(drawn[3].abs());
        let straight = drawn[0] > 0.0 && drawn[3] > 0.0;
        if !straight || drawn[1].abs().max(drawn[2].abs()) > 0.02 * scale {
            let why = "text that is turned or slanted cannot be edited yet";
            return Err(Error::InvalidArgument(why.into()));
        }
        // These two also start a line: the line is still started.
        if operation.operator == "\"" {
            kept.push(Operation::new("Tw", vec![Object::Real(number(0))]));
            kept.push(Operation::new("Tc", vec![Object::Real(number(1))]));
        }
        if operation.operator != "Tj" && operation.operator != "TJ" {
            kept.push(Operation::new("T*", Vec::new()));
        }
        if let Some(advance) = advance.filter(|advance| *advance != 0.0 && state.font_size != 0.0) {
            let gap = Object::Real(-advance / state.font_size * 1000.0);
            kept.push(Operation::new("TJ", vec![Object::Array(vec![gap])]));
        }
        if removed.as_ref().is_none_or(|first| x < first.origin.0) {
            removed = Some(Removed {
                origin: (x, y),
                size: state.font_size * drawn[3],
                fill: state.fill.clone(),
            });
        }
    }
    *operations = kept;
    Ok(removed)
}

/// Have `page` know the font the new words are written with.
fn add_font(doc: &mut Document, page: ObjectId) -> Result<()> {
    let font = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    // The resources may be shared with other pages: this page gets its own copy.
    let found = inherited(doc, page, b"Resources");
    let found = found.as_ref().and_then(|found| deref(doc, found));
    let mut resources = found
        .and_then(|found| found.as_dict().ok())
        .cloned()
        .unwrap_or_default();
    let mut fonts = dict_entry(doc, &resources, b"Font")
        .cloned()
        .unwrap_or_default();
    fonts.set(FONT, Object::Reference(font));
    resources.set("Font", fonts);
    doc.get_dictionary_mut(page)?.set("Resources", resources);
    Ok(())
}

/// Replace the text `page` draws in `area` (a line, in page space) by `text`, written from
/// where the old one started, at its size and in its colour. An empty `text` only removes.
///
/// The text has to be written on the page itself, upright: otherwise nothing is changed and
/// the error says why.
pub fn replace_text(pdf: &[u8], page: usize, area: Rect, text: &str) -> Result<Vec<u8>> {
    let mut doc = load(pdf)?;
    let pages = page_ids(&doc);
    check_page(page, pages.len())?;
    let page = pages[page];
    let mut content = Content::decode(&doc.get_page_content(page))?;
    let fonts = page_fonts(&doc, page);
    let Some(removed) = take_out(&mut content.operations, area, &fonts)? else {
        let why = "this text is not written on the page itself: it cannot be edited";
        return Err(Error::InvalidArgument(why.into()));
    };

    let plain = |operator: &str| Operation::new(operator, Vec::new());
    // What the page drew is closed on itself: what is added starts from a clean state.
    let mut operations = vec![plain("q")];
    operations.append(&mut content.operations);
    operations.push(plain("Q"));
    let text = text.replace(['\r', '\n'], " ");
    if !text.trim().is_empty() && removed.size > 0.0 {
        add_font(&mut doc, page)?;
        let mut fill = removed.fill;
        if fill.is_empty() {
            fill.push(Operation::new("g", vec![Object::Integer(0)]));
        }
        let font = vec![Object::Name(FONT.into()), Object::Real(removed.size)];
        let (x, y) = removed.origin;
        let place = [1.0, 0.0, 0.0, 1.0, x, y].map(Object::Real).to_vec();
        let written = text.chars().map(win_ansi).collect();
        operations.push(plain("q"));
        operations.extend(fill);
        operations.extend([
            plain("BT"),
            Operation::new("Tf", font),
            Operation::new("Tm", place),
            Operation::new("Tj", vec![Object::String(written, StringFormat::Literal)]),
            plain("ET"),
            plain("Q"),
        ]);
    }
    // A stream of its own: the old one may be shared with other pages.
    let bytes = Content { operations }.encode()?;
    let stream = doc.add_object(Stream::new(Dictionary::new(), bytes));
    doc.get_dictionary_mut(page)?
        .set("Contents", Object::Reference(stream));
    save(&mut doc)
}

enum Widths {
    Simple {
        first: i64,
        widths: Vec<f32>,
        missing: f32,
        standard: Option<(Font, bool, bool)>,
    },
    Composite {
        ranges: Vec<(u32, u32, f32)>,
        default: f32,
    },
}

impl Widths {
    fn read(doc: &Document, font: &Dictionary) -> Self {
        let numbers = |object: Option<&Object>| -> Vec<f32> {
            object
                .and_then(|object| deref(doc, object))
                .and_then(|object| object.as_array().ok())
                .map(|items| {
                    items
                        .iter()
                        .map(|item| {
                            deref(doc, item)
                                .and_then(|item| item.as_float().ok())
                                .unwrap_or(0.0)
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        let number = |dict: &Dictionary, key: &[u8]| {
            dict.get(key)
                .ok()
                .and_then(|value| deref(doc, value))
                .and_then(|value| value.as_float().ok())
        };
        let subtype = font
            .get(b"Subtype")
            .ok()
            .and_then(|name| name.as_name().ok());
        if subtype == Some(b"Type0".as_slice()) {
            let descendant = font
                .get(b"DescendantFonts")
                .ok()
                .and_then(|list| deref(doc, list))
                .and_then(|list| list.as_array().ok())
                .and_then(|list| list.first())
                .and_then(|first| deref(doc, first))
                .and_then(|first| first.as_dict().ok());
            let Some(descendant) = descendant else {
                return Self::Composite {
                    ranges: Vec::new(),
                    default: 1000.0,
                };
            };
            let list = descendant
                .get(b"W")
                .ok()
                .and_then(|list| deref(doc, list))
                .and_then(|list| list.as_array().ok())
                .cloned()
                .unwrap_or_default();
            let mut ranges = Vec::new();
            let mut at = 0;
            while at + 1 < list.len() {
                let start = deref(doc, &list[at])
                    .and_then(|value| value.as_float().ok())
                    .unwrap_or(0.0) as u32;
                match deref(doc, &list[at + 1]) {
                    Some(Object::Array(_)) => {
                        let widths = numbers(Some(&list[at + 1]));
                        for (offset, width) in widths.into_iter().enumerate() {
                            let code = start + offset as u32;
                            ranges.push((code, code, width));
                        }
                        at += 2;
                    }
                    Some(end) => {
                        let end = end.as_float().unwrap_or(0.0) as u32;
                        let width = list
                            .get(at + 2)
                            .and_then(|value| deref(doc, value))
                            .and_then(|value| value.as_float().ok())
                            .unwrap_or(0.0);
                        ranges.push((start, end, width));
                        at += 3;
                    }
                    None => break,
                }
            }
            return Self::Composite {
                ranges,
                default: number(descendant, b"DW").unwrap_or(1000.0),
            };
        }
        let missing = dict_entry(doc, font, b"FontDescriptor")
            .and_then(|descriptor| number(descriptor, b"MissingWidth"))
            .unwrap_or(0.0);
        let standard = font
            .get(b"BaseFont")
            .ok()
            .and_then(|name| name.as_name().ok())
            .and_then(|name| Font::standard(&String::from_utf8_lossy(name)));
        Self::Simple {
            first: number(font, b"FirstChar").map_or(0, |first| first as i64),
            widths: numbers(font.get(b"Widths").ok()),
            missing,
            standard,
        }
    }

    fn displacement(&self, bytes: &[u8], size: f32, char_spacing: f32, word_spacing: f32) -> f32 {
        let glyph = |units: f32, space: bool| {
            units / 1000.0 * size + char_spacing + if space { word_spacing } else { 0.0 }
        };
        match self {
            Self::Simple {
                first,
                widths,
                missing,
                standard,
            } => bytes
                .iter()
                .map(|&code| {
                    let listed = usize::try_from(i64::from(code) - first)
                        .ok()
                        .and_then(|index| widths.get(index))
                        .copied();
                    let units = match (listed, standard) {
                        (Some(width), _) => width,
                        (None, Some((font, bold, italic))) => {
                            f32::from(font.code_units(code, *bold, *italic))
                        }
                        (None, None) => *missing,
                    };
                    glyph(units, code == b' ')
                })
                .sum(),
            Self::Composite { ranges, default } => bytes
                .chunks(2)
                .map(|pair| {
                    let code = pair
                        .iter()
                        .fold(0u32, |code, &byte| code << 8 | u32::from(byte));
                    let units = ranges
                        .iter()
                        .find(|(start, end, _)| (*start..=*end).contains(&code))
                        .map_or(*default, |(_, _, width)| *width);
                    glyph(units, pair == [b' '])
                })
                .sum(),
        }
    }
}

fn page_fonts(doc: &Document, page: ObjectId) -> HashMap<Vec<u8>, Widths> {
    let resources = inherited(doc, page, b"Resources");
    let resources = resources
        .as_ref()
        .and_then(|found| deref(doc, found))
        .and_then(|found| found.as_dict().ok());
    let Some(fonts) = resources.and_then(|resources| dict_entry(doc, resources, b"Font")) else {
        return HashMap::new();
    };
    fonts
        .iter()
        .filter_map(|(name, font)| {
            let font = deref(doc, font)?.as_dict().ok()?;
            Some((name.clone(), Widths::read(doc, font)))
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineLook {
    pub size: f32,
    pub color: Option<crate::Color>,
}

pub fn line_look(pdf: &[u8], page: usize, area: Rect) -> Result<LineLook> {
    let doc = load(pdf)?;
    let pages = page_ids(&doc);
    check_page(page, pages.len())?;
    let page = pages[page];
    let mut operations = Content::decode(&doc.get_page_content(page))?.operations;
    let fonts = page_fonts(&doc, page);
    let Some(removed) = take_out(&mut operations, area, &fonts)? else {
        let why = "this text is not written on the page itself: it cannot be edited";
        return Err(Error::InvalidArgument(why.into()));
    };
    Ok(LineLook {
        size: removed.size,
        color: removed.fill.last().and_then(color_of),
    })
}

fn color_of(fill: &Operation) -> Option<crate::Color> {
    let values: Vec<f32> = fill
        .operands
        .iter()
        .map_while(|operand| operand.as_float().ok())
        .collect();
    let clamp = |value: f32| value.clamp(0.0, 1.0);
    match (fill.operator.as_str(), values.as_slice()) {
        ("g" | "sc" | "scn", &[gray]) => Some(crate::Color {
            r: clamp(gray),
            g: clamp(gray),
            b: clamp(gray),
        }),
        ("rg" | "sc" | "scn", &[r, g, b]) => Some(crate::Color {
            r: clamp(r),
            g: clamp(g),
            b: clamp(b),
        }),
        ("k" | "sc" | "scn", &[c, m, y, k]) => Some(crate::Color {
            r: clamp((1.0 - c) * (1.0 - k)),
            g: clamp((1.0 - m) * (1.0 - k)),
            b: clamp((1.0 - y) * (1.0 - k)),
        }),
        _ => None,
    }
}
