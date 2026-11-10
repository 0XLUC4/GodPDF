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
    assert_eq!(items[0], bytes(b"bids"));
    assert_eq!(text(&out, 200.0), "bids [bɪd]");
    // The sound after it keeps its place (and its red, its font: the same operators).
    let moved = run_text(&out, 0, cell(200.0)).unwrap();
    let bracket = |letters: &[(char, f32)]| letters.iter().find(|(l, _)| *l == '[').unwrap().1;
    assert!((bracket(&places) - bracket(&moved)).abs() < 0.001);
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
    // A gap is left where "id" was: the space and the sound stay where they were.
    assert_eq!(spelled, "b [bɪd]");
    let last = |letters: &[(char, f32)]| letters.last().unwrap().1;
    assert!((last(&before) - last(&after)).abs() < 0.001);
}

#[test]
fn nothing_changed_writes_nothing() {
    let pdf = row();
    let out = replace_run(&pdf, 0, cell(200.0), "bid [bɪd]").unwrap();
    assert_eq!(drawn(&out), drawn(&pdf));
}
