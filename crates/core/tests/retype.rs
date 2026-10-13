use godpdf_core::{
    CharStyle, Error, Rect, Script, extract_text, line_look, replace_styled, replace_text,
};
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
    page_with(operations)
}

fn page_with(operations: Vec<Operation>) -> Vec<u8> {
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

#[test]
fn what_follows_on_the_line_stays_in_place() {
    let numbers = |values: &[f32]| values.iter().map(|value| Object::Real(*value)).collect();
    let gap = Object::Array(vec![Object::Real(-20000.0)]);
    let pdf = page_with(vec![
        op("BT", vec![]),
        op("Tf", vec![Object::Name("F1".into()), Object::Real(12.0)]),
        op("Td", numbers(&[72.0, 400.0])),
        shown("Name"),
        op("TJ", vec![gap]),
        shown("Value"),
        op("ET", vec![]),
    ]);
    let out = replace_text(&pdf, 0, line(72.0, 400.0), "New").unwrap();
    let text = extract_text(&out, 0).unwrap();
    assert!(text.contains("Value") && !text.contains("Name"), "{text}");
    let drawn = drawn(&out);
    let at = drawn
        .iter()
        .position(|(_, said)| said == &["Value"])
        .unwrap();
    let name_width = (722.0 + 556.0 + 833.0 + 556.0) / 1000.0 * 12.0;
    assert_eq!(drawn[at - 2].0, "TJ");
    let doc = Document::load_mem(&out).unwrap();
    let page = doc.page_iter().next().unwrap();
    let content = Content::decode(&doc.get_page_content(page)).unwrap();
    let ops: Vec<&Operation> = content.operations.iter().collect();
    let value = ops
        .iter()
        .position(|op| op.operands.first().and_then(|o| o.as_str().ok()) == Some(b"Value"))
        .unwrap();
    let replacement = ops[value - 2].operands[0].as_array().unwrap()[0]
        .as_float()
        .unwrap();
    assert!(
        (replacement + name_width / 12.0 * 1000.0).abs() < 0.5,
        "{replacement}"
    );
}

#[test]
fn a_colour_from_a_named_space_is_kept() {
    let numbers = |values: &[f32]| values.iter().map(|value| Object::Real(*value)).collect();
    let pdf = page_with(vec![
        op("BT", vec![]),
        op("cs", vec![Object::Name("CS0".into())]),
        op("scn", numbers(&[0.1, 0.4, 0.8])),
        op("Tf", vec![Object::Name("F1".into()), Object::Real(12.0)]),
        op("Td", numbers(&[72.0, 400.0])),
        shown("Blue"),
        op("ET", vec![]),
    ]);
    let out = replace_text(&pdf, 0, line(72.0, 400.0), "Still blue").unwrap();
    let drawn = drawn(&out);
    let at = drawn
        .iter()
        .position(|(_, said)| said == &["Still blue"])
        .unwrap();
    assert_eq!(drawn[at - 5], ("cs".into(), vec!["CS0".to_owned()]));
    assert_eq!(drawn[at - 4].0, "scn");
}

#[test]
fn the_look_of_a_line_is_its_size_and_colour() {
    let look = line_look(&fixture(), 0, line(72.0, 680.0)).unwrap();
    assert!((look.size - 12.0).abs() < 0.01);
    let red = look.color.unwrap();
    assert_eq!((red.r, red.g, red.b), (1.0, 0.0, 0.0));
    let look = line_look(&fixture(), 0, line(72.0, 700.0)).unwrap();
    assert_eq!(look.color, None);
    assert!(line_look(&fixture(), 0, line(300.0, 300.0)).is_err());
}

#[test]
fn a_bold_slanted_line_stays_bold_and_slanted() {
    let numbers = |values: &[f32]| values.iter().map(|value| Object::Real(*value)).collect();
    let pdf = page_with(vec![
        op("BT", vec![]),
        op("Tf", vec![Object::Name("F1".into()), Object::Real(12.0)]),
        op("Td", numbers(&[72.0, 400.0])),
        shown("Title"),
        op("ET", vec![]),
    ]);
    // The font of the page is a subset of one that is not standard.
    let mut doc = Document::load_mem(&pdf).unwrap();
    for object in doc.objects.values_mut() {
        if let Ok(font) = object.as_dict_mut()
            && font.has(b"BaseFont")
        {
            font.set("BaseFont", Object::Name("ABCDEF+Calibri-BoldItalic".into()));
        }
    }
    let mut pdf = Vec::new();
    doc.save_to(&mut pdf).unwrap();

    let look = line_look(&pdf, 0, line(72.0, 400.0)).unwrap();
    let style = look.style_at(75.0);
    assert!(style.bold && style.italic && !style.underline);
    let out = replace_text(&pdf, 0, line(72.0, 400.0), "Heading").unwrap();
    let drawn = drawn(&out);
    let at = drawn
        .iter()
        .position(|(_, said)| said == &["Heading"])
        .unwrap();
    assert_eq!(drawn[at - 2].1[0], "GodPDFHeBO");
    let doc = Document::load_mem(&out).unwrap();
    let bases = doc.objects.values().filter_map(|object| {
        let name = object
            .as_dict()
            .ok()?
            .get(b"BaseFont")
            .ok()?
            .as_name()
            .ok()?;
        Some(String::from_utf8_lossy(name).into_owned())
    });
    assert!(
        bases
            .into_iter()
            .any(|name| name == "Helvetica-BoldOblique")
    );
}

/// One line: "Plain " in F1, "bold" in F2 (bold) with a line drawn under it, then a small
/// raised "2". Above it a rule as wide as the page, which is not an underline.
fn mixed() -> Vec<u8> {
    let numbers = |values: &[f32]| values.iter().map(|value| Object::Real(*value)).collect();
    let font =
        |name: &str, size: f32| op("Tf", vec![Object::Name(name.into()), Object::Real(size)]);
    let pdf = page_with(vec![
        op("BT", vec![]),
        font("F1", 12.0),
        op("Td", numbers(&[72.0, 400.0])),
        shown("Plain "),
        font("F2", 12.0),
        op("Tm", numbers(&[1.0, 0.0, 0.0, 1.0, 110.0, 400.0])),
        shown("bold"),
        font("F1", 7.0),
        op("Tm", numbers(&[1.0, 0.0, 0.0, 1.0, 135.0, 405.0])),
        shown("2"),
        op("ET", vec![]),
        op("re", numbers(&[110.0, 398.2, 24.0, 0.7])),
        op("f", vec![]),
        op("w", numbers(&[0.5])),
        op("m", numbers(&[0.0, 396.0])),
        op("l", numbers(&[612.0, 396.0])),
        op("S", vec![]),
    ]);
    let mut doc = Document::load_mem(&pdf).unwrap();
    let bold = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica-Bold",
        "Encoding" => "WinAnsiEncoding",
    });
    for object in doc.objects.values_mut() {
        if let Ok(resources) = object
            .as_dict_mut()
            .and_then(|page| page.get_mut(b"Resources"))
            && let Ok(fonts) = resources.as_dict_mut().and_then(|all| all.get_mut(b"Font"))
        {
            fonts.as_dict_mut().unwrap().set("F2", bold);
        }
    }
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

#[test]
fn each_word_of_a_line_has_its_own_look() {
    let look = line_look(&mixed(), 0, line(72.0, 400.0)).unwrap();
    assert!((look.size - 12.0).abs() < 0.01);
    assert_eq!(look.style_at(80.0), CharStyle::default());
    let bold = look.style_at(120.0);
    assert!(bold.bold && bold.underline && bold.script == Script::Normal);
    let raised = look.style_at(137.0);
    assert!(!raised.bold && !raised.underline && raised.script == Script::Super);
}

#[test]
fn a_line_is_written_back_word_by_word() {
    let plain = CharStyle::default();
    let bold = CharStyle {
        bold: true,
        underline: true,
        ..plain
    };
    let styles = [plain, plain, bold, bold];
    let out = replace_styled(&mixed(), 0, line(72.0, 400.0), "A BC", &styles, &[]).unwrap();
    let drawn = drawn(&out);
    let says = |words: &str| drawn.iter().position(|(_, said)| said == &[words]).unwrap();
    let (first, second) = (says("A "), says("BC"));
    assert_eq!(drawn[first - 2].1, ["GodPDFHelv", "12"]);
    assert_eq!(drawn[second - 1].1, ["GodPDFHeBo", "12"]);
    // The old underline is gone, the rule of the page is not, and "BC" has a new one.
    let rects: Vec<&Vec<String>> = drawn
        .iter()
        .filter(|(operator, _)| operator == "re")
        .map(|(_, numbers)| numbers)
        .collect();
    assert_eq!(rects.len(), 1, "{rects:?}");
    let start: f32 = rects[0][0].parse().unwrap();
    let a_and_space = (667.0 + 278.0) / 1000.0 * 12.0;
    assert!((start - (72.0 + a_and_space)).abs() < 0.01, "{start}");
    assert!(
        drawn
            .iter()
            .any(|(operator, said)| operator == "l" && said[0] == "612")
    );
    let text = extract_text(&out, 0).unwrap();
    assert!(!text.contains("Plain") && !text.contains("bold"), "{text}");
}

#[test]
fn plain_new_words_are_like_the_first_of_the_old() {
    let out = replace_text(&mixed(), 0, line(72.0, 400.0), "New").unwrap();
    let drawn = drawn(&out);
    let at = drawn.iter().position(|(_, said)| said == &["New"]).unwrap();
    assert_eq!(drawn[at - 2].1, ["GodPDFHelv", "12"]);
    assert!(drawn.iter().all(|(operator, _)| operator != "re"));
}

#[test]
fn each_word_keeps_its_colour() {
    let numbers = |values: &[f32]| values.iter().map(|value| Object::Real(*value)).collect();
    let pdf = page_with(vec![
        op("BT", vec![]),
        op("Tf", vec![Object::Name("F1".into()), Object::Real(12.0)]),
        op("Td", numbers(&[72.0, 400.0])),
        shown("Black "),
        op("rg", numbers(&[1.0, 0.0, 0.0])),
        op("Tm", numbers(&[1.0, 0.0, 0.0, 1.0, 110.0, 400.0])),
        shown("red"),
        op("ET", vec![]),
    ]);
    let look = line_look(&pdf, 0, line(72.0, 400.0)).unwrap();
    let red = look.inks[1].unwrap();
    assert_eq!((look.inks.len(), look.inks[0]), (2, None));
    assert_eq!((red.r, red.g, red.b), (1.0, 0.0, 0.0));
    assert_eq!((look.ink_at(80.0), look.ink_at(115.0)), (0, 1));

    let underlined = CharStyle {
        underline: true,
        ..CharStyle::default()
    };
    let (styles, inks) = ([underlined; 4], [1, 1, 0, 0]);
    let out = replace_styled(&pdf, 0, line(72.0, 400.0), "ReBl", &styles, &inks).unwrap();
    let drawn = drawn(&out);
    let operators: Vec<&str> = drawn
        .iter()
        .map(|(operator, _)| operator.as_str())
        .collect();
    let from = operators
        .iter()
        .rposition(|operator| *operator == "q")
        .unwrap();
    // Red words, black words, then a line under each in its colour.
    let wanted = [
        "q", "rg", "BT", "Tf", "Tm", "Tj", "g", "Tf", "Tj", "ET", "rg", "re", "f", "g", "re", "f",
        "Q",
    ];
    assert_eq!(operators[from..], wanted);
}

#[test]
fn letters_raised_by_the_page_are_raised() {
    let numbers = |values: &[f32]| values.iter().map(|value| Object::Real(*value)).collect();
    let font = |size: f32| op("Tf", vec![Object::Name("F1".into()), Object::Real(size)]);
    let pdf = page_with(vec![
        op("BT", vec![]),
        font(12.0),
        op("Td", numbers(&[72.0, 400.0])),
        shown("mc"),
        font(7.0),
        op("Ts", numbers(&[5.0])),
        shown("2"),
        op("ET", vec![]),
    ]);
    let look = line_look(&pdf, 0, line(72.0, 400.0)).unwrap();
    assert_eq!(look.style_at(75.0).script, Script::Normal);
    assert_eq!(look.style_at(90.0).script, Script::Super);
}

#[test]
fn a_line_changed_twice_has_the_look_of_the_last_change() {
    let numbers = |values: &[f32]| values.iter().map(|value| Object::Real(*value)).collect();
    let pdf = page_with(vec![
        op("BT", vec![]),
        op("Tf", vec![Object::Name("F1".into()), Object::Real(12.0)]),
        op("Td", numbers(&[72.0, 400.0])),
        shown("Black "),
        op("rg", numbers(&[1.0, 0.0, 0.0])),
        op("Tm", numbers(&[1.0, 0.0, 0.0, 1.0, 110.0, 400.0])),
        shown("red"),
        op("ET", vec![]),
    ]);
    // Red first this time, then black: "Re" ends at 87.3.
    let inks = [1, 1, 0, 0, 0, 0, 0, 0, 0, 0];
    let out = replace_styled(&pdf, 0, line(72.0, 400.0), "Re black.", &[], &inks).unwrap();
    let look = line_look(&out, 0, line(72.0, 400.0)).unwrap();
    // Nothing is left of where the old words were, nor of their colours.
    assert_eq!(look.inks.len(), 2, "{look:?}");
    let color = |x: f32| look.inks[look.ink_at(x)].map(|color| (color.r, color.g, color.b));
    assert_eq!(color(80.0), Some((1.0, 0.0, 0.0)));
    assert_eq!(color(100.0), Some((0.0, 0.0, 0.0)));
    assert_eq!(color(115.0), Some((0.0, 0.0, 0.0)));
    // And a third change still finds one line there.
    let again = replace_styled(&out, 0, line(72.0, 400.0), "Once more", &[], &[]).unwrap();
    let text = extract_text(&again, 0).unwrap();
    assert!(
        text.contains("Once more") && !text.contains("black"),
        "{text}"
    );
}
