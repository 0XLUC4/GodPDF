use godpdf_core::{Error, Rect, extract_text, replace_text};
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};

fn op(operator: &str, operands: Vec<Object>) -> Operation {
    Operation::new(operator, operands)
}

fn shown(text: &str) -> Operation {
    op("Tj", vec![Object::string_literal(text)])
}

/// One page: two lines moved to with `Td` (the second in two pieces, in red), a word
/// drawn twice as big inside `q … Q`, and one turned a quarter.
fn fixture() -> Vec<u8> {
    let numbers = |values: &[f32]| values.iter().map(|value| Object::Real(*value)).collect();
    let font = |size: f32| op("Tf", vec![Object::Name("F1".into()), Object::Real(size)]);
    let operations = vec![
        op("BT", vec![]),
        font(12.0),
        op("Td", numbers(&[72.0, 700.0])),
        shown("First line"),
        op("rg", numbers(&[1.0, 0.0, 0.0])),
        op("Td", numbers(&[0.0, -20.0])),
        shown("Second"),
        shown(" part"),
        op("ET", vec![]),
        op("q", vec![]),
        op("cm", numbers(&[2.0, 0.0, 0.0, 2.0, 0.0, 0.0])),
        op("BT", vec![]),
        font(10.0),
        op("Tm", numbers(&[1.0, 0.0, 0.0, 1.0, 50.0, 100.0])),
        shown("Scaled"),
        op("ET", vec![]),
        op("Q", vec![]),
        op("BT", vec![]),
        font(12.0),
        op("Tm", numbers(&[0.0, 1.0, -1.0, 0.0, 400.0, 400.0])),
        shown("Turned"),
        op("ET", vec![]),
    ];
    let mut doc = Document::with_version("1.7");
    let pages = doc.new_object_id();
    let font = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let content = Content { operations }.encode().unwrap();
    let content = doc.add_object(Stream::new(dictionary! {}, content));
    let page = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
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

/// The operations of the first page, as (operator, operands as numbers or text).
fn drawn(pdf: &[u8]) -> Vec<(String, Vec<String>)> {
    let doc = Document::load_mem(pdf).unwrap();
    let page = doc.page_iter().next().unwrap();
    let content = Content::decode(&doc.get_page_content(page)).unwrap();
    let said = |operand: &Object| match operand {
        Object::String(bytes, _) => String::from_utf8_lossy(bytes).into_owned(),
        Object::Name(name) => String::from_utf8_lossy(name).into_owned(),
        other => other
            .as_float()
            .map_or_else(|_| "?".into(), |n| n.to_string()),
    };
    let operations = content.operations.iter();
    operations
        .map(|op| (op.operator.clone(), op.operands.iter().map(said).collect()))
        .collect()
}

/// The line that starts at (x, y): a box around its baseline.
fn line(x: f32, y: f32) -> Rect {
    Rect {
        x0: x,
        y0: y - 3.0,
        x1: x + 150.0,
        y1: y + 10.0,
    }
}

#[test]
fn a_line_is_replaced_where_it_was_in_its_size_and_colour() {
    let out = replace_text(&fixture(), 0, line(72.0, 680.0), "Changed").unwrap();
    let text = extract_text(&out, 0).unwrap();
    assert!(
        text.contains("First line") && text.contains("Scaled"),
        "{text}"
    );
    assert!(text.contains("Changed"), "{text}");
    // Both pieces of the line are gone, not only the first.
    assert!(!text.contains("Second") && !text.contains("part"), "{text}");

    let drawn = drawn(&out);
    let at = drawn.iter().position(|(_, said)| said == &["Changed"]);
    let at = at.expect("the new words");
    let numbers = |values: &[&str]| values.iter().map(|v| (*v).to_owned()).collect::<Vec<_>>();
    assert_eq!(
        drawn[at - 1],
        ("Tm".into(), numbers(&["1", "0", "0", "1", "72", "680"]))
    );
    assert_eq!(drawn[at - 2], ("Tf".into(), numbers(&["GodPDFHelv", "12"])));
    assert_eq!(drawn[at - 4], ("rg".into(), numbers(&["1", "0", "0"])));
    // What the page drew is closed before anything is added.
    assert_eq!((drawn[0].0.as_str(), drawn[at - 5].0.as_str()), ("q", "q"));
}

#[test]
fn text_made_bigger_by_the_page_keeps_its_place_and_size() {
    let out = replace_text(&fixture(), 0, line(100.0, 200.0), "Big").unwrap();
    let drawn = drawn(&out);
    let at = drawn.iter().position(|(_, said)| said == &["Big"]).unwrap();
    assert_eq!(drawn[at - 1].1[4..], ["100".to_owned(), "200".to_owned()]);
    assert_eq!(drawn[at - 2].1[1], "20");
    // The red set earlier on the page was still the colour in use.
    assert_eq!(drawn[at - 4].0, "rg");
    assert!(!extract_text(&out, 0).unwrap().contains("Scaled"));
    // Before any colour is set, text is black.
    let out = replace_text(&fixture(), 0, line(72.0, 700.0), "Top").unwrap();
    let drawn = self::drawn(&out);
    let at = drawn.iter().position(|(_, said)| said == &["Top"]).unwrap();
    assert_eq!(drawn[at - 4], ("g".into(), vec!["0".to_owned()]));
}

#[test]
fn an_empty_text_only_removes() {
    let out = replace_text(&fixture(), 0, line(72.0, 700.0), "  ").unwrap();
    let text = extract_text(&out, 0).unwrap();
    assert!(
        !text.contains("First line") && text.contains("Second"),
        "{text}"
    );
    let fonts = drawn(&out);
    let mut fonts = fonts.iter().filter_map(|(_, said)| said.first());
    assert!(fonts.all(|name| name != "GodPDFHelv"));
}

#[test]
fn what_cannot_be_edited_is_left_alone() {
    let pdf = fixture();
    // Nothing is written there.
    let nothing = replace_text(&pdf, 0, line(300.0, 300.0), "x");
    assert!(
        matches!(nothing, Err(Error::InvalidArgument(_))),
        "{nothing:?}"
    );
    // Written sideways.
    let turned = Rect {
        x0: 395.0,
        y0: 395.0,
        x1: 405.0,
        y1: 405.0,
    };
    let turned = replace_text(&pdf, 0, turned, "x");
    assert!(
        matches!(turned, Err(Error::InvalidArgument(_))),
        "{turned:?}"
    );
    let missing = replace_text(&pdf, 3, line(72.0, 700.0), "x");
    assert!(matches!(missing, Err(Error::PageOutOfRange { .. })));
}
