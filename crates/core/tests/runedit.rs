use godpdf_core::{Rect, replace_run, run_text};
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};

fn op(operator: &str, operands: Vec<Object>) -> Operation {
    Operation::new(operator, operands)
}

fn numbers(values: &[f32]) -> Vec<Object> {
    values.iter().map(|value| Object::Real(*value)).collect()
}

fn font(name: &str) -> Operation {
    op("Tf", vec![Object::Name(name.into()), Object::Real(11.0)])
}

fn bytes(code: &[u8]) -> Object {
    Object::String(code.to_vec(), lopdf::StringFormat::Literal)
}

/// Two-byte codes of a composite font for `text` (identity for ASCII).
fn wide(text: &str) -> Object {
    let codes = text.bytes().flat_map(|byte| [0, byte]).collect();
    Object::String(codes, lopdf::StringFormat::Hexadecimal)
}

const CMAP: &str = "/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
/CMapName /Test def
1 begincodespacerange
<00> <FF>
endcodespacerange
5 beginbfchar
<5B> <005B>
<5D> <005D>
<62> <0062>
<01> <026A>
<64> <0064>
endbfchar
endcmap
CMapName currentdict /CMap defineresource pop
end
end";

const WIDE_CMAP: &str = "/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
/CMapName /Wide def
1 begincodespacerange
<0000> <FFFF>
endcodespacerange
1 beginbfrange
<0020> <007E> <0020>
endbfrange
endcmap
CMapName currentdict /CMap defineresource pop
end
end";

/// A row of a vocabulary table, four cells each placed with `Td`:
/// `to bid [bɪd]` | `bid [bɪd]` | `bid [bɪd]` | `faire une offre`.
/// Words in black Times (F1), the sounds in red in an embedded font with a /ToUnicode (F2,
/// `ɪ` is its code 1), the last cell in a composite font (F3).
fn row() -> Vec<u8> {
    let black = || op("g", numbers(&[0.0]));
    let red = || op("rg", numbers(&[1.0, 0.0, 0.0]));
    let sound = || op("Tj", vec![bytes(b"[b\x01d]")]);
    let operations = vec![
        op("BT", vec![]),
        black(),
        font("F1"),
        op("Td", numbers(&[72.0, 700.0])),
        op("Tj", vec![bytes(b"to bid ")]),
        red(),
        font("F2"),
        sound(),
        black(),
        font("F1"),
        op("Td", numbers(&[128.0, 0.0])),
        op("Tj", vec![bytes(b"bid ")]),
        red(),
        font("F2"),
        sound(),
        black(),
        font("F1"),
        op("Td", numbers(&[100.0, 0.0])),
        op(
            "TJ",
            vec![Object::Array(vec![
                bytes(b"b"),
                Object::Real(-20.0),
                bytes(b"id "),
            ])],
        ),
        red(),
        font("F2"),
        sound(),
        black(),
        font("F3"),
        op("Td", numbers(&[100.0, 0.0])),
        op("Tj", vec![wide("faire une offre")]),
        op("ET", vec![]),
    ];
    let mut doc = Document::with_version("1.7");
    let pages = doc.new_object_id();
    let times = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Times-Roman",
        "Encoding" => "WinAnsiEncoding",
    });
    let file = doc.add_object(Stream::new(dictionary! {}, b"not a real font".to_vec()));
    let descriptor = doc.add_object(dictionary! {
        "Type" => "FontDescriptor", "FontName" => "ABCDEF+IPA", "Flags" => 2,
        "FontFile2" => file,
    });
    let cmap = doc.add_object(Stream::new(dictionary! {}, CMAP.as_bytes().to_vec()));
    let ipa = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "TrueType", "BaseFont" => "ABCDEF+IPA",
        "Encoding" => "WinAnsiEncoding", "FirstChar" => 0, "LastChar" => 255,
        "Widths" => vec![Object::Integer(500); 256],
        "FontDescriptor" => descriptor, "ToUnicode" => cmap,
    });
    let wide_cmap = doc.add_object(Stream::new(dictionary! {}, WIDE_CMAP.as_bytes().to_vec()));
    let cid = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "CIDFontType2", "BaseFont" => "Sans", "DW" => 500,
    });
    let composite = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "Sans",
        "Encoding" => "Identity-H", "DescendantFonts" => vec![cid.into()],
        "ToUnicode" => wide_cmap,
    });
    let content = Content { operations }.encode().unwrap();
    let content = doc.add_object(Stream::new(dictionary! {}, content));
    let page = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Resources" => dictionary! {
            "Font" => dictionary! { "F1" => times, "F2" => ipa, "F3" => composite },
        },
        "Contents" => content,
    });
    doc.objects.insert(
        pages,
        dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 }.into(),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

/// The cell that starts at `x` on the row.
fn cell(x: f32) -> Rect {
    Rect {
        x0: x,
        y0: 697.0,
        x1: x + 95.0,
        y1: 709.0,
    }
}

/// The operations of the first page, each as its operator and operands.
fn drawn(pdf: &[u8]) -> Vec<(String, Vec<Object>)> {
    let doc = Document::load_mem(pdf).unwrap();
    let page = doc.page_iter().next().unwrap();
    let content = Content::decode(&doc.get_page_content(page)).unwrap();
    let operations = content.operations.into_iter();
    operations.map(|op| (op.operator, op.operands)).collect()
}

fn text(pdf: &[u8], x: f32) -> String {
    run_text(pdf, 0, cell(x))
        .unwrap()
        .iter()
        .map(|(letter, _)| letter)
        .collect()
}

fn close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 0.01,
        "{actual} is not {expected}"
    );
}

/// The same letters at the same places (to a thousandth of a point).
fn same_places(actual: &[(char, f32)], expected: &[(char, f32)]) {
    assert_eq!(actual.len(), expected.len(), "{actual:?} / {expected:?}");
    for (got, want) in actual.iter().zip(expected) {
        assert_eq!(got.0, want.0);
        assert!((got.1 - want.1).abs() < 0.001, "{actual:?} / {expected:?}");
    }
}

/// A one-page PDF drawing `operations` with Helvetica as F1.
fn page_of(operations: Vec<Operation>) -> Vec<u8> {
    let mut doc = Document::with_version("1.7");
    let pages = doc.new_object_id();
    let helvetica = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let content = Content { operations }.encode().unwrap();
    let content = doc.add_object(Stream::new(dictionary! {}, content));
    let page = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Resources" => dictionary! { "Font" => dictionary! { "F1" => helvetica } },
        "Contents" => content,
    });
    doc.objects.insert(
        pages,
        dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 }.into(),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

/// Each letter of `pdf` in `area` and the middle x of its glyph.
fn places(pdf: &[u8], area: Rect) -> Vec<(char, f32)> {
    run_text(pdf, 0, area).unwrap()
}

fn text_in(pdf: &[u8], area: Rect) -> String {
    places(pdf, area).iter().map(|place| place.0).collect()
}

/// From `x0` to `x1`, around a baseline at `y`.
fn area(x0: f32, x1: f32, y: f32) -> Rect {
    Rect {
        x0,
        y0: y - 3.0,
        x1,
        y1: y + 9.0,
    }
}

/// Two cells of a table row in one `TJ`, the second pushed right by a wide gap, as browsers
/// write them: "49 500" at 72 and "TVA" about 100 points further.
fn one_show_row() -> Vec<u8> {
    page_of(vec![
        op("BT", vec![]),
        font("F1"),
        op("Td", numbers(&[72.0, 700.0])),
        op(
            "TJ",
            vec![Object::Array(vec![
                bytes(b"49 500"),
                Object::Real(-6000.0),
                bytes(b"TVA"),
            ])],
        ),
        op("ET", vec![]),
    ])
}

const FIRST: Rect = Rect {
    x0: 70.0,
    y0: 697.0,
    x1: 110.0,
    y1: 709.0,
};
const SECOND: Rect = Rect {
    x0: 160.0,
    y0: 697.0,
    x1: 240.0,
    y1: 709.0,
};

/// The page `after` is `before` with operation `at` replaced: all others are the same.
fn only_changed(before: &[(String, Vec<Object>)], after: &[(String, Vec<Object>)], at: usize) {
    let tail = before.len() - at - 1;
    assert_eq!(before[..at], after[..at]);
    assert_eq!(before[at + 1..], after[after.len() - tail..]);
}

#[test]
fn a_cell_is_read_alone_with_its_true_letters() {
    let pdf = row();
    assert_eq!(text(&pdf, 72.0), "to bid [bɪd]");
    assert_eq!(text(&pdf, 200.0), "bid [bɪd]");
    assert_eq!(text(&pdf, 300.0), "bid [bɪd]");
    assert_eq!(text(&pdf, 400.0), "faire une offre");
}

#[test]
fn a_word_changed_leaves_everything_else_as_it_was() {
    let pdf = row();
    let before = drawn(&pdf);
    let places = run_text(&pdf, 0, cell(200.0)).unwrap();
    let out = replace_run(&pdf, 0, cell(200.0), "bids [bɪd]").unwrap();
    let after = drawn(&out);
    // Only the black "bid " of the second cell is written again, in its own font.
    let at = before
        .iter()
        .position(|(_, said)| said == &[bytes(b"bid ")])
        .unwrap();
    only_changed(&before, &after, at);
    let new = &after[at..after.len() - (before.len() - at - 1)];
    assert_eq!(new.len(), 1, "{new:?}");
    let Object::Array(items) = &new[0].1[0] else {
        panic!("{new:?}")
    };
    assert_eq!(items, &[bytes(b"bids ")]);
    assert_eq!(text(&out, 200.0), "bids [bɪd]");
    // The sound after it moves right by the "s" (Times 389 / 1000 em at 11), its operators
    // (red, its font) untouched.
    let moved = run_text(&out, 0, cell(200.0)).unwrap();
    let bracket = |letters: &[(char, f32)]| letters.iter().find(|(l, _)| *l == '[').unwrap().1;
    close(bracket(&moved) - bracket(&places), 0.389 * 11.0);
    for x in [72.0, 300.0, 400.0] {
        assert_eq!(text(&out, x), text(&pdf, x));
    }
}

#[test]
fn a_letter_the_font_has_not_is_written_in_the_closest_standard_font() {
    let pdf = row();
    let before = drawn(&pdf);
    let out = replace_run(&pdf, 0, cell(300.0), "bid [bɪt]").unwrap();
    let after = drawn(&out);
    let sounds: Vec<usize> = before
        .iter()
        .enumerate()
        .filter(|(_, (_, said))| said == &[bytes(b"[b\x01d]")])
        .map(|(at, _)| at)
        .collect();
    let at = sounds[2];
    only_changed(&before, &after, at);
    let new: Vec<&str> = after[at..after.len() - (before.len() - at - 1)]
        .iter()
        .map(|(operator, _)| operator.as_str())
        .collect();
    // "[bɪ" in its font, "t" in Times (the font is serif), back to its font for "]".
    assert_eq!(new, ["TJ", "Tf", "Tj", "Tf", "TJ"]);
    let new = &after[at..at + 5];
    assert_eq!(new[1].1[0], Object::Name(b"GodPDFTiRo".to_vec()));
    assert_eq!(new[2].1[0], bytes(b"t"));
    assert_eq!(new[3].1[0], Object::Name(b"F2".to_vec()));
    // Still in red: the colour set before it is untouched.
    assert_eq!(after[at - 2].0, "rg");
    assert_eq!(text(&out, 300.0), "bid [bɪt]");
}

#[test]
fn a_composite_font_writes_the_letters_it_has() {
    let pdf = row();
    let out = replace_run(&pdf, 0, cell(400.0), "faire des offres").unwrap();
    let after = drawn(&out);
    assert!(after.iter().all(|(operator, said)| operator != "Tf"
        || !matches!(&said[0], Object::Name(name) if name.starts_with(b"GodPDF"))));
    assert_eq!(text(&out, 400.0), "faire des offres");
    assert_eq!(text(&out, 72.0), "to bid [bɪd]");
}

#[test]
fn letters_taken_out_leave_the_rest_in_place() {
    let pdf = row();
    let before = run_text(&pdf, 0, cell(300.0)).unwrap();
    let out = replace_run(&pdf, 0, cell(300.0), "b [bɪd]").unwrap();
    let after = run_text(&out, 0, cell(300.0)).unwrap();
    let spelled: String = after.iter().map(|(letter, _)| letter).collect();
    // The space and the sound close up over "id" (Times i 278 + d 500, at 11).
    assert_eq!(spelled, "b [bɪd]");
    let last = |letters: &[(char, f32)]| letters.last().unwrap().1;
    close(last(&before) - last(&after), 0.778 * 11.0);
    // The next cell has its own place.
    let next = run_text(&out, 0, cell(400.0)).unwrap();
    same_places(&next, &run_text(&pdf, 0, cell(400.0)).unwrap());
}

#[test]
fn nothing_changed_writes_nothing() {
    let pdf = row();
    let out = replace_run(&pdf, 0, cell(200.0), "bid [bɪd]").unwrap();
    assert_eq!(drawn(&out), drawn(&pdf));
}

#[test]
fn typing_in_a_cell_moves_its_letters_but_not_the_next_cell() {
    let pdf = one_show_row();
    let tva = places(&pdf, SECOND);
    let before = places(&pdf, FIRST);
    // A digit typed in the middle: "49 1500".
    let out = replace_run(&pdf, 0, FIRST, "49 1500").unwrap();
    let after = places(&out, area(70.0, 120.0, 700.0));
    assert_eq!(text_in(&out, area(70.0, 120.0, 700.0)), "49 1500");
    // The letters before it stay, those after move by one digit (Helvetica 556 at 11).
    close(after[2].1, before[2].1);
    close(after[6].1 - before[5].1, 0.556 * 11.0);
    same_places(&places(&out, SECOND), &tva);
}

#[test]
fn shortening_a_cell_closes_it_up_but_leaves_the_next_cell() {
    let pdf = one_show_row();
    let tva = places(&pdf, SECOND);
    let before = places(&pdf, FIRST);
    let out = replace_run(&pdf, 0, FIRST, "9 500").unwrap();
    let after = places(&out, FIRST);
    assert_eq!(text_in(&out, FIRST), "9 500");
    // "4" taken out at the start: everything after it moves left by one digit.
    close(before[1].1 - after[0].1, 0.556 * 11.0);
    close(before[5].1 - after[4].1, 0.556 * 11.0);
    same_places(&places(&out, SECOND), &tva);
}

#[test]
fn a_whole_cell_rewritten_or_emptied_keeps_the_next_cell() {
    let pdf = one_show_row();
    let tva = places(&pdf, SECOND);
    let out = replace_run(&pdf, 0, FIRST, "1 234 567").unwrap();
    assert_eq!(text_in(&out, area(70.0, 130.0, 700.0)), "1 234 567");
    same_places(&places(&out, SECOND), &tva);
    let out = replace_run(&pdf, 0, FIRST, "").unwrap();
    assert!(run_text(&out, 0, FIRST).is_err());
    same_places(&places(&out, SECOND), &tva);
}

#[test]
fn a_scaled_line_moves_by_the_scaled_width() {
    // Text drawn twice its size by the matrix, in two shows: "ab" then "cd".
    let pdf = page_of(vec![
        op("BT", vec![]),
        font("F1"),
        op("Tm", numbers(&[2.0, 0.0, 0.0, 2.0, 72.0, 600.0])),
        op("Tj", vec![bytes(b"ab")]),
        op("Tj", vec![bytes(b"cd")]),
        op("ET", vec![]),
    ]);
    let line = Rect {
        x0: 70.0,
        y0: 595.0,
        x1: 200.0,
        y1: 625.0,
    };
    let before = places(&pdf, line);
    let out = replace_run(&pdf, 0, line, "aabcd").unwrap();
    let after = places(&out, line);
    assert_eq!(text_in(&out, line), "aabcd");
    // Helvetica "a" is 556 / 1000 em, at 11, twice: every letter after it moves as much.
    for at in 1..4 {
        close(after[at + 1].1 - before[at].1, 2.0 * 0.556 * 11.0);
    }
}

#[test]
fn the_next_line_does_not_move() {
    // The second line starts with T*: what is typed on the first one leaves it.
    let pdf = page_of(vec![
        op("BT", vec![]),
        font("F1"),
        op("TL", numbers(&[14.0])),
        op("Td", numbers(&[72.0, 700.0])),
        op("Tj", vec![bytes(b"first")]),
        op("T*", vec![]),
        op("Tj", vec![bytes(b"second")]),
        op("ET", vec![]),
    ]);
    let first = area(70.0, 200.0, 700.0);
    let next = area(70.0, 200.0, 686.0);
    let below = places(&pdf, next);
    let out = replace_run(&pdf, 0, first, "the first").unwrap();
    assert_eq!(text_in(&out, first), "the first");
    same_places(&places(&out, next), &below);
}

#[test]
fn text_after_the_run_in_the_same_show_keeps_its_place_when_typing_at_the_end() {
    let pdf = one_show_row();
    let tva = places(&pdf, SECOND);
    let out = replace_run(&pdf, 0, FIRST, "49 500 €").unwrap();
    assert_eq!(text_in(&out, area(70.0, 125.0, 700.0)), "49 500 €");
    same_places(&places(&out, SECOND), &tva);
}
