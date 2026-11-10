//! Changing the words of one run of text (a cell of a table, a phrase) where the page draws
//! them. Only the characters that changed are written again: every other glyph keeps its
//! operator, code, font, colour and place. New characters are written in the font of the
//! glyphs around them when it has them, else in the standard font closest to it.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Encoding, Object, Stream, StringFormat};

use crate::annotate::Rect;
use crate::fonts::CharStyle;
use crate::helvetica::win_ansi;
use crate::pdf::{check_page, dict_entry, load, page_ids, save};
use crate::retype::{
    Face, IDENTITY, Matrix, PageFont, add_font, descendant, moved, page_fonts, then,
};
use crate::{Error, Result};

/// How far outside `area` a glyph may be and still belong to it, in points.
const SLACK: f32 = 1.5;
/// A gap wider than this, in ems, reads as a space the PDF does not write (as the viewer).
const IMPLIED_SPACE: f32 = 0.25;

/// What a font's codes are as Unicode, and back.
pub(crate) struct Unicode {
    /// The font's /ToUnicode map.
    map: Option<Encoding<'static>>,
    /// A simple font: what each byte is by its /Encoding.
    table: Vec<Option<char>>,
    /// Whether the font is in the file: then it may hold only the glyphs the document used.
    embedded: bool,
    /// How many bytes a code takes: 1, or 2 for a composite font.
    code_len: usize,
}

impl Unicode {
    pub(crate) fn read(doc: &Document, font: &Dictionary) -> Self {
        let composite = font
            .get(b"Subtype")
            .and_then(Object::as_name)
            .is_ok_and(|subtype| subtype == b"Type0");
        let mut typed = font.clone();
        typed.set("Type", "Font");
        // Without /Encoding, lopdf reads /ToUnicode.
        let mut unmapped = typed.clone();
        unmapped.remove(b"Encoding");
        let map = match unmapped.get_font_encoding(doc) {
            Ok(Encoding::UnicodeMapEncoding(map)) if unmapped.has(b"ToUnicode") => {
                Some(Encoding::UnicodeMapEncoding(map))
            }
            _ => None,
        };
        let table = match (composite, typed.get_font_encoding(doc)) {
            (false, Ok(encoding)) => (0..=255u8)
                .map(|code| {
                    let text = encoding.bytes_to_string(&[code]).ok()?;
                    let mut letters = text.chars();
                    letters.next().filter(|_| letters.next().is_none())
                })
                .collect(),
            _ => Vec::new(),
        };
        let descriptor = dict_entry(doc, font, b"FontDescriptor").or_else(|| {
            descendant(doc, font).and_then(|font| dict_entry(doc, font, b"FontDescriptor"))
        });
        let embedded = descriptor.is_some_and(|descriptor| {
            [b"FontFile".as_slice(), b"FontFile2", b"FontFile3"]
                .iter()
                .any(|key| descriptor.has(key))
        });
        Self {
            map,
            table,
            embedded,
            code_len: if composite { 2 } else { 1 },
        }
    }

    /// The text of one glyph code; U+FFFD when the font does not say.
    fn text(&self, code: &[u8]) -> String {
        let mapped = self.map.as_ref().and_then(|map| {
            let text = map.bytes_to_string(code).ok()?;
            (!text.is_empty() && !text.contains('\u{FFFD}')).then_some(text)
        });
        let listed = || {
            let &[byte] = code else { return None };
            self.table.get(usize::from(byte)).copied().flatten()
        };
        mapped
            .or_else(|| listed().map(String::from))
            .unwrap_or_else(|| "\u{FFFD}".into())
    }

    /// The code of `letter` in this font, when it surely has a glyph for it: listed in its
    /// /ToUnicode, or in its /Encoding when the font is not in the file or the page `used` it.
    fn code(&self, letter: char, used: &HashSet<Vec<u8>>) -> Option<Vec<u8>> {
        let text = letter.to_string();
        let mapped = self.map.as_ref().map(|map| map.string_to_bytes(&text));
        let mapped = mapped.filter(|code| code.len() == self.code_len && self.text(code) == text);
        mapped.or_else(|| {
            let byte = self.table.iter().position(|known| *known == Some(letter))?;
            let code = vec![u8::try_from(byte).ok()?];
            (!self.embedded || used.contains(&code)).then_some(code)
        })
    }
}

/// The codes each font of a page draws, by resource name.
type Used = HashMap<Vec<u8>, HashSet<Vec<u8>>>;

/// What the text state is at a show operator.
struct Shown {
    font: Vec<u8>,
    size: f32,
    char_spacing: f32,
    word_spacing: f32,
}

/// One glyph a page draws.
struct Glyph {
    /// Its show operator, the string in it and the code in that string.
    op: usize,
    item: usize,
    code: Range<usize>,
    /// Where it is in page space: left and right edge, baseline, and its size.
    x0: f32,
    x1: f32,
    y: f32,
    em: f32,
    /// How far it moves the pen, in text space before horizontal scaling.
    advance: f32,
    text: String,
}

/// The strings and gaps a show operator draws.
fn items(operation: &Operation) -> &[Object] {
    match (operation.operator.as_str(), operation.operands.last()) {
        ("TJ", Some(Object::Array(items))) => items,
        (_, Some(last)) => std::slice::from_ref(last),
        (_, None) => &[],
    }
}

/// Every upright glyph of `operations`, the text state of each show operator by its index,
/// and the codes each font drew.
fn scan(
    operations: &[Operation],
    fonts: &HashMap<Vec<u8>, PageFont>,
) -> (Vec<Glyph>, HashMap<usize, Shown>, Used) {
    #[derive(Clone)]
    struct State {
        ctm: Matrix,
        font: Vec<u8>,
        size: f32,
        leading: f32,
        char_spacing: f32,
        word_spacing: f32,
        scaling: f32,
    }
    let mut state = State {
        ctm: IDENTITY,
        font: Vec::new(),
        size: 0.0,
        leading: 0.0,
        char_spacing: 0.0,
        word_spacing: 0.0,
        scaling: 1.0,
    };
    let mut saved: Vec<State> = Vec::new();
    let (mut text, mut line) = (IDENTITY, IDENTITY);
    let (mut glyphs, mut shows, mut used) = (Vec::new(), HashMap::new(), HashMap::new());
    for (index, operation) in operations.iter().enumerate() {
        let number = |at: usize| {
            let operand = operation.operands.get(at);
            operand
                .and_then(|operand| operand.as_float().ok())
                .unwrap_or(0.0)
        };
        let next_line =
            |line: Matrix, leading: f32| then([1.0, 0.0, 0.0, 1.0, 0.0, -leading], line);
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
                state.size = number(1);
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
            "Tj" | "TJ" | "'" | "\"" => {
                if operation.operator == "\"" {
                    state.word_spacing = number(0);
                    state.char_spacing = number(1);
                }
                if operation.operator != "Tj" && operation.operator != "TJ" {
                    line = next_line(line, state.leading);
                    text = line;
                }
                shows.insert(
                    index,
                    Shown {
                        font: state.font.clone(),
                        size: state.size,
                        char_spacing: state.char_spacing,
                        word_spacing: state.word_spacing,
                    },
                );
                let drawn = then(text, state.ctm);
                let skew = drawn[1].abs().max(drawn[2].abs());
                let upright = drawn[0] > 0.0 && drawn[3] > 0.0 && skew <= 0.02 * drawn[0];
                let font = fonts.get(&state.font);
                let mut pen = 0.0;
                for (item, shown) in items(operation).iter().enumerate() {
                    let Object::String(bytes, _) = shown else {
                        pen -= shown.as_float().unwrap_or(0.0) / 1000.0 * state.size;
                        continue;
                    };
                    let Some(font) = font else { continue };
                    let mut at = 0;
                    for (length, units, space) in font.widths.glyphs(bytes) {
                        let code = at..(at + length).min(bytes.len());
                        let width = units / 1000.0 * state.size;
                        let spacing = if space { state.word_spacing } else { 0.0 };
                        let advance = width + state.char_spacing + spacing;
                        used.entry(state.font.clone())
                            .or_insert_with(HashSet::new)
                            .insert(bytes[code.clone()].to_vec());
                        if upright {
                            let (x0, y) = moved(drawn, (pen * state.scaling, 0.0));
                            let (x1, _) = moved(drawn, ((pen + width) * state.scaling, 0.0));
                            glyphs.push(Glyph {
                                op: index,
                                item,
                                text: font.unicode.text(&bytes[code.clone()]),
                                code,
                                x0,
                                x1,
                                y,
                                em: state.size * drawn[3],
                                advance,
                            });
                        }
                        pen += advance;
                        at += length;
                    }
                }
                text = then([1.0, 0.0, 0.0, 1.0, pen * state.scaling, 0.0], text);
            }
            _ => {}
        }
    }
    (glyphs, shows, used)
}

/// One character of a run: which glyph it comes from (`None`: a space only implied by a gap).
struct Letter {
    letter: char,
    glyph: Option<usize>,
    x: f32,
}

/// The glyphs of `glyphs` in `area`, left to right, spelled out.
fn spell(glyphs: &[Glyph], area: Rect) -> Vec<Letter> {
    let mut inside: Vec<usize> = (0..glyphs.len())
        .filter(|&at| {
            let glyph = &glyphs[at];
            let middle = (glyph.x0 + glyph.x1) / 2.0;
            (area.x0 - SLACK..=area.x1 + SLACK).contains(&middle)
                && (area.y0 - SLACK..=area.y1 + SLACK).contains(&glyph.y)
        })
        .collect();
    inside.sort_by(|a, b| glyphs[*a].x0.total_cmp(&glyphs[*b].x0));
    let mut letters: Vec<Letter> = Vec::new();
    let mut prev: Option<&Glyph> = None;
    for at in inside {
        let glyph = &glyphs[at];
        let middle = (glyph.x0 + glyph.x1) / 2.0;
        if let Some(prev) = prev
            && glyph.x0 - prev.x1 > IMPLIED_SPACE * glyph.em
            && !prev.text.ends_with(char::is_whitespace)
            && !glyph.text.starts_with(char::is_whitespace)
        {
            letters.push(Letter {
                letter: ' ',
                glyph: None,
                x: prev.x1,
            });
        }
        letters.extend(glyph.text.chars().map(|letter| Letter {
            letter,
            glyph: Some(at),
            x: middle,
        }));
        prev = Some(glyph);
    }
    letters
}

/// The page's content and fonts, and the run of `page` in `area`.
struct Run {
    doc: Document,
    page: lopdf::ObjectId,
    operations: Vec<Operation>,
    fonts: HashMap<Vec<u8>, PageFont>,
    glyphs: Vec<Glyph>,
    shows: HashMap<usize, Shown>,
    used: Used,
    letters: Vec<Letter>,
}

impl Run {
    fn read(pdf: &[u8], page: usize, area: Rect) -> Result<Self> {
        let doc = load(pdf)?;
        let pages = page_ids(&doc);
        check_page(page, pages.len())?;
        let page = pages[page];
        let operations = Content::decode(&doc.get_page_content(page))?.operations;
        let fonts = page_fonts(&doc, page);
        let (glyphs, shows, used) = scan(&operations, &fonts);
        let letters = spell(&glyphs, area);
        if letters.is_empty() {
            let why = "this text is not written on the page itself: it cannot be edited";
            return Err(Error::InvalidArgument(why.into()));
        }
        Ok(Self {
            doc,
            page,
            operations,
            fonts,
            glyphs,
            shows,
            used,
            letters,
        })
    }
}

/// The text `page` draws in `area` (a run of one line, in page space) as its fonts say it,
/// each character with the middle of its glyph (x, page space). Spaces the page only implies
/// by a gap are in it.
pub fn run_text(pdf: &[u8], page: usize, area: Rect) -> Result<Vec<(char, f32)>> {
    let run = Run::read(pdf, page, area)?;
    Ok(run
        .letters
        .iter()
        .map(|letter| (letter.letter, letter.x))
        .collect())
}

/// Have the run `page` draws in `area` say `text` (as [`run_text`] spells it, changed).
/// Only what changed is written again, where it was, in the colour and font of the glyphs
/// around it (a standard font for letters that font has not); every other glyph keeps its
/// place, even when the new words are longer or shorter.
pub fn replace_run(pdf: &[u8], page: usize, area: Rect, text: &str) -> Result<Vec<u8>> {
    let mut run = Run::read(pdf, page, area)?;
    let old: Vec<char> = run.letters.iter().map(|letter| letter.letter).collect();
    let new: Vec<char> = text.replace(['\r', '\n'], " ").chars().collect();
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let (mut lo, mut hi) = (prefix, old.len() - suffix);
    // A glyph of several letters (a ligature) is changed whole.
    let glyph_of = |at: usize| run.letters[at].glyph;
    let split = |at: usize| {
        at > 0 && at < old.len() && glyph_of(at).is_some() && glyph_of(at - 1) == glyph_of(at)
    };
    while split(lo) {
        lo -= 1;
    }
    while split(hi) {
        hi += 1;
    }
    let inserted: String = old[lo..prefix]
        .iter()
        .chain(&new[prefix..new.len() - suffix])
        .chain(&old[old.len() - suffix..hi])
        .collect();
    let removed: HashSet<usize> = (lo..hi).filter_map(glyph_of).collect();
    if removed.is_empty() && inserted.is_empty() {
        return save(&mut run.doc);
    }
    // Where the new letters go: in place of the first glyph taken out, else after the glyph
    // before them, else before the one after them.
    let first = (lo..hi).find_map(glyph_of);
    let before = (0..lo).rev().find_map(glyph_of);
    let after = (hi..old.len()).find_map(glyph_of);
    let Some(anchor) = first
        .map(|glyph| (glyph, true))
        .or(before.map(|glyph| (glyph, false)))
        .or(after.map(|glyph| (glyph, true)))
    else {
        return Err(Error::InvalidArgument("nothing to edit here".into()));
    };

    let mut changed: Vec<usize> = removed.iter().map(|&glyph| run.glyphs[glyph].op).collect();
    changed.push(run.glyphs[anchor.0].op);
    changed.sort_unstable();
    changed.dedup();
    let edit = Edit {
        removed: &removed,
        anchor,
        inserted: &inserted,
    };
    let mut faces: HashMap<Face, String> = HashMap::new();
    let mut rebuilt: HashMap<usize, Vec<Operation>> = HashMap::new();
    for op in changed {
        let (mut operations, fallbacks) = rebuild(&run, op, &edit);
        let shown = &run.shows[&op];
        let face = run
            .fonts
            .get(&shown.font)
            .map(|font| font.face)
            .unwrap_or_default();
        let look = (face, shown.font.clone(), shown.size);
        fall_back(&mut run, &mut faces, &mut operations, fallbacks, look)?;
        rebuilt.insert(op, operations);
    }
    let mut operations = Vec::with_capacity(run.operations.len() + 8);
    for (index, operation) in std::mem::take(&mut run.operations).into_iter().enumerate() {
        match rebuilt.remove(&index) {
            Some(new) => operations.extend(new),
            None => operations.push(operation),
        }
    }
    // A stream of its own: the old one may be shared with other pages.
    let bytes = Content { operations }.encode()?;
    let stream = run.doc.add_object(Stream::new(Dictionary::new(), bytes));
    run.doc
        .get_dictionary_mut(run.page)?
        .set("Contents", Object::Reference(stream));
    save(&mut run.doc)
}

/// What changes in a run: the glyphs taken out, and the letters written at `anchor` (a glyph,
/// and whether they go in its place / before it rather than after it).
struct Edit<'a> {
    removed: &'a HashSet<usize>,
    anchor: (usize, bool),
    inserted: &'a str,
}

/// A `TJ` being written: its strings and gaps, and the operators before it.
struct Writer {
    operations: Vec<Operation>,
    items: Vec<Object>,
}

impl Writer {
    fn code(&mut self, code: &[u8], format: StringFormat) {
        match self.items.last_mut() {
            Some(Object::String(bytes, last)) if *last == format => bytes.extend_from_slice(code),
            _ => self.items.push(Object::String(code.to_vec(), format)),
        }
    }

    /// Move the pen by `advance` (text space, before scaling) at font size `size`.
    fn gap(&mut self, advance: f32, size: f32) {
        if advance != 0.0 && size != 0.0 {
            self.items.push(Object::Real(-advance * 1000.0 / size));
        }
    }

    fn flush(&mut self) {
        if !self.items.is_empty() {
            let items = std::mem::take(&mut self.items);
            self.operations
                .push(Operation::new("TJ", vec![Object::Array(items)]));
        }
    }
}

/// The show operator `op` of `run` with `edit` made: its other glyphs and gaps as they were,
/// each glyph taken out a gap as wide, the new letters drawn over their width then the pen
/// moved back. Also the letters its font has not, each with the operator they go before.
fn rebuild(run: &Run, op: usize, edit: &Edit) -> (Vec<Operation>, Vec<(String, usize)>) {
    let operation = &run.operations[op];
    let shown = &run.shows[&op];
    let font = run.fonts.get(&shown.font);
    let empty = HashSet::new();
    let used = run.used.get(&shown.font).unwrap_or(&empty);
    let mut writer = Writer {
        operations: Vec::new(),
        items: Vec::new(),
    };
    // These two also start a line: the line is still started.
    if operation.operator == "\"" {
        let (word, char) = (shown.word_spacing, shown.char_spacing);
        writer
            .operations
            .push(Operation::new("Tw", vec![Object::Real(word)]));
        writer
            .operations
            .push(Operation::new("Tc", vec![Object::Real(char)]));
    }
    if operation.operator == "'" || operation.operator == "\"" {
        writer.operations.push(Operation::new("T*", Vec::new()));
    }
    let glyphs: HashMap<(usize, usize), usize> = run
        .glyphs
        .iter()
        .enumerate()
        .filter(|(_, glyph)| glyph.op == op)
        .map(|(at, glyph)| ((glyph.item, glyph.code.start), at))
        .collect();
    let mut fallbacks = Vec::new();
    let mut write_new = |writer: &mut Writer, format: StringFormat| {
        insert(
            shown,
            font,
            used,
            edit.inserted,
            format,
            writer,
            &mut fallbacks,
        );
    };
    for (item, shown_item) in items(operation).iter().enumerate() {
        let Object::String(bytes, format) = shown_item else {
            writer.items.push(shown_item.clone());
            continue;
        };
        let lengths = font
            .map(|font| font.widths.glyphs(bytes))
            .unwrap_or_default();
        let mut at = 0;
        for (length, ..) in lengths {
            let code = &bytes[at..(at + length).min(bytes.len())];
            let glyph = glyphs.get(&(item, at)).copied();
            let anchored = glyph == Some(edit.anchor.0);
            if anchored && edit.anchor.1 {
                write_new(&mut writer, *format);
            }
            match glyph.filter(|glyph| edit.removed.contains(glyph)) {
                Some(glyph) => writer.gap(run.glyphs[glyph].advance, shown.size),
                None => writer.code(code, *format),
            }
            if anchored && !edit.anchor.1 {
                write_new(&mut writer, *format);
            }
            at += length;
        }
    }
    writer.flush();
    (writer.operations, fallbacks)
}

/// Put in `operations` (of a show in font `font` at `size`) the letters that font has not,
/// in the standard font `face`.
fn fall_back(
    run: &mut Run,
    faces: &mut HashMap<Face, String>,
    operations: &mut Vec<Operation>,
    fallbacks: Vec<(String, usize)>,
    (face, font, size): (Face, Vec<u8>, f32),
) -> Result<()> {
    if fallbacks.is_empty() {
        return Ok(());
    }
    let name = match faces.get(&face) {
        Some(name) => name.clone(),
        None => {
            let name = add_font(&mut run.doc, run.page, face)?;
            faces.insert(face, name.clone());
            name
        }
    };
    // From the last: each one moves those after it.
    for (letters, at) in fallbacks.into_iter().rev() {
        let written = letters.chars().map(win_ansi).collect();
        operations.splice(
            at..at,
            [
                Operation::new(
                    "Tf",
                    vec![Object::Name(name.clone().into_bytes()), Object::Real(size)],
                ),
                Operation::new("Tj", vec![Object::String(written, StringFormat::Literal)]),
                Operation::new("Tf", vec![Object::Name(font.clone()), Object::Real(size)]),
            ],
        );
    }
    Ok(())
}

/// Write `letters` at the pen of `writer` in the font of `shown`, then move the pen back
/// over them. Those the font has not are left to `fallbacks`: the letters, and the operator
/// they go before.
fn insert(
    shown: &Shown,
    font: Option<&PageFont>,
    used: &HashSet<Vec<u8>>,
    letters: &str,
    format: StringFormat,
    writer: &mut Writer,
    fallbacks: &mut Vec<(String, usize)>,
) {
    let spacing = |letter: char| {
        shown.char_spacing
            + if letter == ' ' {
                shown.word_spacing
            } else {
                0.0
            }
    };
    let mut advance = 0.0;
    let mut missing = String::new();
    for letter in letters.chars() {
        let code = font.and_then(|font| font.unicode.code(letter, used).map(|code| (font, code)));
        if let Some((font, code)) = code {
            if !missing.is_empty() {
                writer.flush();
                fallbacks.push((std::mem::take(&mut missing), writer.operations.len()));
            }
            let units: f32 = font.widths.glyphs(&code).iter().map(|glyph| glyph.1).sum();
            let space = font
                .widths
                .glyphs(&code)
                .first()
                .is_some_and(|glyph| glyph.2);
            let word = if space { shown.word_spacing } else { 0.0 };
            advance += units / 1000.0 * shown.size + shown.char_spacing + word;
            writer.code(&code, format);
        } else {
            let (family, bold, italic) = font.map(|font| font.face).unwrap_or_default();
            let style = CharStyle {
                bold,
                italic,
                ..CharStyle::default()
            };
            advance += family.char_width(letter, style, shown.size) + spacing(letter);
            missing.push(letter);
        }
    }
    if !missing.is_empty() {
        writer.flush();
        fallbacks.push((missing, writer.operations.len()));
    }
    writer.gap(-advance, shown.size);
}
