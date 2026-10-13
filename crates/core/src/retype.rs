//! Changing the text a page draws: the words of a line are taken out of the page's content
//! and new ones written where they started.
//!
//! The new words are set in a standard font: the font of the old ones usually holds only the
//! letters the document used, and cannot write others. The standard font is the one closest
//! to it, and each word stays as bold, as slanted, as underlined and as raised as it was, in
//! the colour it had.

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat, dictionary};

use std::collections::HashMap;

use crate::annotate::Rect;
use crate::fonts::{CharStyle, Font, Script, style_at};
use crate::helvetica::win_ansi;
use crate::pdf::{check_page, deref, dict_entry, inherited, load, page_ids, save};
use crate::{Error, Result};

/// Start of the name of the font the new words are written with, in the resources of the page.
const FONT: &str = "GodPDF";
/// How far outside `area` a word may start and still belong to it, in points.
const SLACK: f32 = 1.5;

/// A transformation, as PDF writes it: `[a b c d e f]`.
type Matrix = [f32; 6];
/// A standard font, bold or not, slanted or not.
type Face = (Font, bool, bool);
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
    /// How far above the baseline the text is written.
    rise: f32,
}

/// One piece of the text taken out of a page.
struct Piece {
    /// Where it started, in page space.
    x: f32,
    y: f32,
    /// Its height on the page, in points.
    size: f32,
    fill: Vec<Operation>,
    face: Face,
}

/// The text taken out of a page: a line.
struct Removed {
    /// Where the line started: its left end, on the baseline of its biggest letters.
    origin: (f32, f32),
    /// The height of its biggest letters, in points.
    size: f32,
    /// The colours of its pieces, each once, as the page set them. The first is the one of
    /// its biggest letters.
    inks: Vec<Vec<Operation>>,
    face: Face,
    /// Where each piece started, what it looked like, and its colour in `inks`.
    runs: Vec<(f32, CharStyle, usize)>,
    /// The lines drawn under its words: where each starts and ends.
    underlines: Vec<(f32, f32)>,
}

impl Removed {
    /// The line `pieces` made, or `None` when there are none.
    fn of(mut pieces: Vec<Piece>, underlines: Vec<(f32, f32)>) -> Option<Self> {
        pieces.sort_by(|a, b| a.x.total_cmp(&b.x));
        // The line is the one of its biggest letters: smaller ones are raised or lowered.
        let main = pieces
            .iter()
            .reduce(|main, piece| if piece.size > main.size { piece } else { main })?;
        let (baseline, size, face) = (main.y, main.size, main.face);
        let mut inks: Vec<Vec<Operation>> = vec![main.fill.clone()];
        let runs = pieces
            .iter()
            .map(|piece| {
                let script = if piece.size >= 0.85 * size {
                    Script::Normal
                } else if piece.y > baseline + 0.1 * size {
                    Script::Super
                } else if piece.y < baseline - 0.03 * size {
                    Script::Sub
                } else {
                    Script::Normal
                };
                let style = CharStyle {
                    bold: piece.face.1,
                    italic: piece.face.2,
                    underline: false,
                    script,
                };
                let known = inks.iter().position(|ink| same(ink, &piece.fill));
                let ink = known.unwrap_or_else(|| {
                    inks.push(piece.fill.clone());
                    inks.len() - 1
                });
                (piece.x, style, ink)
            })
            .collect();
        Some(Self {
            origin: (pieces[0].x, baseline),
            size,
            inks,
            face,
            runs,
            underlines,
        })
    }

    fn look(&self) -> LineLook {
        let inks = self.inks.iter();
        let inks: Vec<_> = inks.map(|ink| ink.last().and_then(color_of)).collect();
        LineLook {
            size: self.size,
            color: inks[0],
            font: self.face.0,
            inks,
            runs: self.runs.clone(),
            underlines: self.underlines.clone(),
        }
    }
}

/// Whether two colours are set the same way.
fn same(one: &[Operation], other: &[Operation]) -> bool {
    let said = |set: &Operation| (set.operator.clone(), set.operands.clone());
    one.iter().map(said).eq(other.iter().map(said))
}

/// Take out of `operations` the line of text that starts in `area`, and the lines drawn under
/// its words. An error when the page draws no text there.
fn lift(
    operations: &mut Vec<Operation>,
    area: Rect,
    fonts: &HashMap<Vec<u8>, PageFont>,
) -> Result<Removed> {
    let pieces = take_out(operations, area, fonts)?;
    let Some(mut removed) = Removed::of(pieces, Vec::new()) else {
        let why = "this text is not written on the page itself: it cannot be edited";
        return Err(Error::InvalidArgument(why.into()));
    };
    removed.underlines = take_underlines(operations, area, removed.origin.1, removed.size);
    Ok(removed)
}

/// Where `(x, y)` is once `matrix` has moved it.
fn moved(matrix: Matrix, (x, y): (f32, f32)) -> (f32, f32) {
    let [a, b, c, d, e, f] = matrix;
    (a * x + c * y + e, b * x + d * y + f)
}

/// Take out of `operations` the thin lines drawn just under the baseline of a line of text
/// (`area`, its `baseline`, the `size` of its letters). Returns where each starts and ends.
fn take_underlines(
    operations: &mut Vec<Operation>,
    area: Rect,
    baseline: f32,
    size: f32,
) -> Vec<(f32, f32)> {
    let mut kept: Vec<Operation> = Vec::with_capacity(operations.len());
    let mut found = Vec::new();
    let (mut ctm, mut line_width) = (IDENTITY, 1.0f32);
    let mut saved: Vec<(Matrix, f32)> = Vec::new();
    // The path being drawn: where it starts in `kept`, and the box around it (x0, y0, x1, y1).
    let mut path: Option<(usize, [f32; 4])> = None;
    // Only straight lines and rectangles are looked at.
    let mut straight = true;
    for operation in operations.drain(..) {
        let number = |index: usize| {
            let operand = operation.operands.get(index);
            operand
                .and_then(|operand| operand.as_float().ok())
                .unwrap_or(0.0)
        };
        let mut corners: Vec<(f32, f32)> = Vec::new();
        match operation.operator.as_str() {
            "m" | "l" => corners.push((number(0), number(1))),
            "re" => {
                corners.push((number(0), number(1)));
                corners.push((number(0) + number(2), number(1) + number(3)));
            }
            "h" => {}
            "c" | "v" | "y" | "W" | "W*" => straight = false,
            "f" | "F" | "f*" | "S" | "s" => {
                let stroked = matches!(operation.operator.as_str(), "S" | "s");
                let scale = (ctm[0] * ctm[3] - ctm[1] * ctm[2]).abs().sqrt();
                let under = path.filter(|_| straight).filter(|(_, [x0, y0, x1, y1])| {
                    let (width, height) = (x1 - x0, y1 - y0);
                    let thickness = if stroked { line_width * scale } else { height };
                    let flat = !stroked || height <= thickness;
                    let middle = (y0 + y1) / 2.0;
                    flat && thickness > 0.0
                        && thickness <= 0.15 * size
                        && width >= 0.25 * size
                        && *x0 >= area.x0 - 0.5 * size
                        && *x1 <= area.x1 + 0.5 * size
                        && (baseline - 0.35 * size..=baseline - 0.02 * size).contains(&middle)
                });
                (path, straight) = (None, true);
                if let Some((start, [x0, _, x1, _])) = under {
                    kept.truncate(start);
                    found.push((x0, x1));
                    continue;
                }
            }
            other => {
                (path, straight) = (None, true);
                match other {
                    "q" => saved.push((ctm, line_width)),
                    "Q" => (ctm, line_width) = saved.pop().unwrap_or((ctm, line_width)),
                    "cm" => ctm = then([0, 1, 2, 3, 4, 5].map(number), ctm),
                    "w" => line_width = number(0),
                    _ => {}
                }
            }
        }
        if matches!(
            operation.operator.as_str(),
            "m" | "l" | "re" | "h" | "c" | "v" | "y"
        ) {
            let (_, bounds) = path.get_or_insert((
                kept.len(),
                [
                    f32::INFINITY,
                    f32::INFINITY,
                    f32::NEG_INFINITY,
                    f32::NEG_INFINITY,
                ],
            ));
            for corner in corners {
                let (x, y) = moved(ctm, corner);
                *bounds = [
                    bounds[0].min(x),
                    bounds[1].min(y),
                    bounds[2].max(x),
                    bounds[3].max(y),
                ];
            }
        }
        kept.push(operation);
    }
    *operations = kept;
    found
}

/// Take out of `operations` every piece of text that starts in `area`. Returns what each was
/// like: nothing when the page draws no text there.
fn take_out(
    operations: &mut Vec<Operation>,
    area: Rect,
    fonts: &HashMap<Vec<u8>, PageFont>,
) -> Result<Vec<Piece>> {
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
        rise: 0.0,
    };
    // The text matrix, and the one of the start of the line.
    let (mut text, mut line) = (IDENTITY, IDENTITY);
    let mut pieces = Vec::new();
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
            let widths = fonts.get(&state.font).map(|font| &font.widths);
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
        // What only moves on draws nothing: such is what an earlier change left of the text
        // it took out.
        let draws = shown
            .iter()
            .any(|item| matches!(item, Object::String(bytes, _) if !bytes.is_empty()));
        if !(shows && inside && draws) {
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
        pieces.push(Piece {
            x,
            y: y + state.rise * drawn[3],
            size: state.font_size * drawn[3],
            fill: state.fill.clone(),
            face: fonts
                .get(&state.font)
                .map(|font| font.face)
                .unwrap_or_default(),
        });
    }
    *operations = kept;
    Ok(pieces)
}

/// Have `page` know the font the new words are written with. Returns its name there.
fn add_font(doc: &mut Document, page: ObjectId, (font, bold, italic): Face) -> Result<String> {
    let name = format!("{FONT}{}", font.resource(bold, italic));
    let font = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => font.base_font(bold, italic),
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
    fonts.set(name.as_str(), Object::Reference(font));
    resources.set("Font", fonts);
    doc.get_dictionary_mut(page)?.set("Resources", resources);
    Ok(name)
}

/// Replace the text `page` draws in `area` (a line, in page space) by `text`, written from
/// where the old one started, at its size, in its colour, and all of it as bold, as slanted
/// and as underlined as its first word was. An empty `text` only removes.
///
/// The text has to be written on the page itself, upright: otherwise nothing is changed and
/// the error says why.
pub fn replace_text(pdf: &[u8], page: usize, area: Rect, text: &str) -> Result<Vec<u8>> {
    rewrite(pdf, page, area, text, None)
}

/// Like [`replace_text`], each character of `text` with its own style and its own colour:
/// `styles` and `inks` have one per character. An ink is one of [`LineLook::inks`]. The
/// characters `styles` does not reach are plain, those `inks` does not reach are in the first.
pub fn replace_styled(
    pdf: &[u8],
    page: usize,
    area: Rect,
    text: &str,
    styles: &[CharStyle],
    inks: &[usize],
) -> Result<Vec<u8>> {
    rewrite(pdf, page, area, text, Some((styles, inks)))
}

fn rewrite(
    pdf: &[u8],
    page: usize,
    area: Rect,
    text: &str,
    marks: Option<(&[CharStyle], &[usize])>,
) -> Result<Vec<u8>> {
    let mut doc = load(pdf)?;
    let pages = page_ids(&doc);
    check_page(page, pages.len())?;
    let page = pages[page];
    let mut content = Content::decode(&doc.get_page_content(page))?;
    let fonts = page_fonts(&doc, page);
    let removed = lift(&mut content.operations, area, &fonts)?;

    let plain = |operator: &str| Operation::new(operator, Vec::new());
    // What the page drew is closed on itself: what is added starts from a clean state.
    let mut operations = vec![plain("q")];
    operations.append(&mut content.operations);
    operations.push(plain("Q"));
    let chars: Vec<char> = text.replace(['\r', '\n'], " ").chars().collect();
    if !text.trim().is_empty() && removed.size > 0.0 {
        let (family, size) = (removed.face.0, removed.size);
        let (x, y) = removed.origin;
        let first = CharStyle {
            script: Script::Normal,
            ..removed.look().style_at(x + 0.25 * size)
        };
        let style_of = |index: usize| marks.map_or(first, |(styles, _)| style_at(styles, index));
        let ink_of = |index: usize| {
            let ink = marks.and_then(|(_, inks)| inks.get(index).copied());
            ink.filter(|ink| *ink < removed.inks.len()).unwrap_or(0)
        };
        // A page that never set a colour writes in black.
        let fill_of = |ink: usize| match removed.inks[ink].as_slice() {
            [] => vec![Operation::new("g", vec![Object::Integer(0)])],
            fill => fill.to_vec(),
        };
        let mut ink = ink_of(0);
        operations.push(plain("q"));
        operations.extend(fill_of(ink));
        operations.push(plain("BT"));
        // The fonts already given to the page, by how bold and slanted they are.
        let mut names: HashMap<(bool, bool), String> = HashMap::new();
        let (mut pen, mut rise) = (x, 0.0);
        let mut underlines = Vec::new();
        let mut at = 0;
        while at < chars.len() {
            let mark = (style_of(at), ink_of(at));
            let end = (at..chars.len())
                .find(|&index| (style_of(index), ink_of(index)) != mark)
                .unwrap_or(chars.len());
            let (style, run_ink) = mark;
            if run_ink != ink {
                ink = run_ink;
                operations.extend(fill_of(ink));
            }
            let name = match names.get(&(style.bold, style.italic)) {
                Some(name) => name.clone(),
                None => {
                    let name = add_font(&mut doc, page, (family, style.bold, style.italic))?;
                    names.insert((style.bold, style.italic), name.clone());
                    name
                }
            };
            let font = vec![
                Object::Name(name.into_bytes()),
                Object::Real(style.size(size)),
            ];
            operations.push(Operation::new("Tf", font));
            if at == 0 {
                let place = [1.0, 0.0, 0.0, 1.0, x, y].map(Object::Real).to_vec();
                operations.push(Operation::new("Tm", place));
            }
            if style.rise(size) != rise {
                rise = style.rise(size);
                operations.push(Operation::new("Ts", vec![Object::Real(rise)]));
            }
            let run = &chars[at..end];
            let written = run.iter().copied().map(win_ansi).collect();
            let shown = vec![Object::String(written, StringFormat::Literal)];
            operations.push(Operation::new("Tj", shown));
            let advance: f32 = run.iter().map(|&c| family.char_width(c, style, size)).sum();
            if style.underline {
                underlines.push((pen, pen + advance, ink));
            }
            pen += advance;
            at = end;
        }
        operations.push(plain("ET"));
        for (x0, x1, under_ink) in underlines {
            if under_ink != ink {
                ink = under_ink;
                operations.extend(fill_of(ink));
            }
            let line = [x0, y - 0.13 * size, x1 - x0, 0.06 * size];
            operations.push(Operation::new("re", line.map(Object::Real).to_vec()));
            operations.push(plain("f"));
        }
        operations.push(plain("Q"));
    }
    // A stream of its own: the old one may be shared with other pages.
    let bytes = Content { operations }.encode()?;
    let stream = doc.add_object(Stream::new(Dictionary::new(), bytes));
    doc.get_dictionary_mut(page)?
        .set("Contents", Object::Reference(stream));
    save(&mut doc)
}

/// A font of a page: how wide its letters are, and the standard font closest to it.
struct PageFont {
    widths: Widths,
    face: Face,
}

/// The font a composite font takes its letters from.
fn descendant<'a>(doc: &'a Document, font: &'a Dictionary) -> Option<&'a Dictionary> {
    font.get(b"DescendantFonts")
        .ok()
        .and_then(|list| deref(doc, list))
        .and_then(|list| list.as_array().ok())
        .and_then(|list| list.first())
        .and_then(|first| deref(doc, first))
        .and_then(|first| first.as_dict().ok())
}

/// The standard font closest to `font`: its name says most, its descriptor the rest.
fn face(doc: &Document, font: &Dictionary) -> Face {
    let name = font
        .get(b"BaseFont")
        .ok()
        .and_then(|name| name.as_name().ok())
        .map(|name| String::from_utf8_lossy(name).into_owned())
        .unwrap_or_default();
    let descriptor = dict_entry(doc, font, b"FontDescriptor").or_else(|| {
        descendant(doc, font).and_then(|font| dict_entry(doc, font, b"FontDescriptor"))
    });
    let number = |key: &[u8]| {
        descriptor
            .and_then(|descriptor| descriptor.get(key).ok())
            .and_then(|value| deref(doc, value))
            .and_then(|value| value.as_float().ok())
            .unwrap_or(0.0)
    };
    // Bits of `Flags`: 1 is fixed pitch, 2 is serif, 64 is italic.
    let flags = number(b"Flags") as u32;
    let family = match Font::standard(&name) {
        Some((family, ..)) => family,
        None if flags & 1 != 0 => Font::Courier,
        None if flags & 2 != 0 => Font::Times,
        None => Font::Helvetica,
    };
    let name = name.to_lowercase();
    let named = |words: &[&str]| words.iter().any(|word| name.contains(word));
    let bold = named(&["bold", "black", "heavy"]) || number(b"FontWeight") >= 600.0;
    let italic = named(&["italic", "oblique"]) || flags & 64 != 0;
    (family, bold, italic)
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
            let Some(descendant) = descendant(doc, font) else {
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

fn page_fonts(doc: &Document, page: ObjectId) -> HashMap<Vec<u8>, PageFont> {
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
            let widths = Widths::read(doc, font);
            let face = face(doc, font);
            Some((name.clone(), PageFont { widths, face }))
        })
        .collect()
}

/// What a line of a page looks like.
#[derive(Debug, Clone, PartialEq)]
pub struct LineLook {
    /// The height of its biggest letters, in points.
    pub size: f32,
    /// The colour of its biggest letters.
    pub color: Option<crate::Color>,
    /// The standard font closest to the one of the line.
    pub font: Font,
    /// The colours of the line, each once: the first is `color`. `None` is one that cannot
    /// be shown as red, green and blue; it is still written back as it was.
    pub inks: Vec<Option<crate::Color>>,
    runs: Vec<(f32, CharStyle, usize)>,
    underlines: Vec<(f32, f32)>,
}

impl LineLook {
    /// What the line draws at `x`: its style, and its colour in `inks`.
    fn run_at(&self, x: f32) -> (CharStyle, usize) {
        let started = self.runs.iter().rev().find(|(start, ..)| *start <= x);
        let run = started.or(self.runs.first()).copied();
        run.map_or_else(Default::default, |(_, style, ink)| (style, ink))
    }

    /// The style of what the line draws at `x`, in page space.
    pub fn style_at(&self, x: f32) -> CharStyle {
        let underline = self
            .underlines
            .iter()
            .any(|(x0, x1)| (*x0..=*x1).contains(&x));
        CharStyle {
            underline,
            ..self.run_at(x).0
        }
    }

    /// The colour of what the line draws at `x`, in page space: which of `inks`.
    pub fn ink_at(&self, x: f32) -> usize {
        self.run_at(x).1
    }
}

pub fn line_look(pdf: &[u8], page: usize, area: Rect) -> Result<LineLook> {
    let doc = load(pdf)?;
    let pages = page_ids(&doc);
    check_page(page, pages.len())?;
    let page = pages[page];
    let mut operations = Content::decode(&doc.get_page_content(page))?.operations;
    let fonts = page_fonts(&doc, page);
    Ok(lift(&mut operations, area, &fonts)?.look())
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
