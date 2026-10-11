//! Changing the text a page draws: the words of a line are taken out of the page's content
//! and new ones written where they started.
//!
//! The new words are set in Helvetica: the font of the old ones usually holds only the
//! letters the document used, and cannot write others.

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat, dictionary};

use crate::annotate::Rect;
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
    /// The operation that set the colour text is filled with, when it is a plain one.
    fill: Option<Operation>,
    font_size: f32,
    leading: f32,
}

/// The text taken out of a page.
struct Removed {
    /// Where its leftmost word started, in page space.
    origin: (f32, f32),
    /// Its height on the page, in points.
    size: f32,
    fill: Option<Operation>,
}

/// Take out of `operations` every piece of text that starts in `area`. Returns what it was
/// like, or `None` when the page draws no text there.
fn take_out(operations: &mut Vec<Operation>, area: Rect) -> Result<Option<Removed>> {
    let mut kept = Vec::with_capacity(operations.len());
    let mut saved: Vec<State> = Vec::new();
    let mut state = State {
        ctm: IDENTITY,
        fill: None,
        font_size: 0.0,
        leading: 0.0,
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
        match operation.operator.as_str() {
            "q" => saved.push(state.clone()),
            "Q" => state = saved.pop().unwrap_or(state),
            "cm" => {
                let matrix = [0, 1, 2, 3, 4, 5].map(number);
                state.ctm = then(matrix, state.ctm);
            }
            "BT" => (text, line) = (IDENTITY, IDENTITY),
            "Tf" => state.font_size = number(1),
            "TL" => state.leading = number(0),
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
                line = next_line(line, state.leading);
                text = line;
                shows = true;
            }
            "Tj" | "TJ" => shows = true,
            "g" | "rg" | "k" => state.fill = Some(operation.clone()),
            // A colour of another kind (a pattern, a named space): not one to write with.
            "cs" | "sc" | "scn" => state.fill = None,
            _ => {}
        }
        let drawn = then(text, state.ctm);
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
        if operation.operator != "Tj" && operation.operator != "TJ" {
            kept.push(Operation::new("T*", Vec::new()));
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
    let Some(removed) = take_out(&mut content.operations, area)? else {
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
        let black = Operation::new("g", vec![Object::Integer(0)]);
        let font = vec![Object::Name(FONT.into()), Object::Real(removed.size)];
        let (x, y) = removed.origin;
        let place = [1.0, 0.0, 0.0, 1.0, x, y].map(Object::Real).to_vec();
        let written = text.chars().map(win_ansi).collect();
        operations.extend([
            plain("q"),
            removed.fill.unwrap_or(black),
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
