mod common;

use common::{edit, make_pdf};
use godpdf_core::{Annotation, Color, Error, Rect, add_annotation, annotations, redact};
use lopdf::content::Content;
use lopdf::{Document, Object, Stream, dictionary};

/// "Page 1" is drawn at (72, 700) in 24 pt Helvetica: "Page" spans about 72..125.
const WORD: Rect = Rect {
    x0: 60.0,
    y0: 690.0,
    x1: 120.0,
    y1: 730.0,
};

fn text(pdf: &[u8]) -> String {
    godpdf_core::extract_text(pdf, 0).unwrap()
}

/// Operators of the content of page 0.
fn operators(pdf: &[u8]) -> Vec<String> {
    let doc = Document::load_mem(pdf).unwrap();
    let page = doc.page_iter().next().unwrap();
    Content::decode(&doc.get_page_content(page))
        .unwrap()
        .operations
        .into_iter()
        .map(|operation| operation.operator)
        .collect()
}

#[test]
fn text_under_the_area_is_gone_and_the_rest_stays() {
    let pdf = redact(&make_pdf(1, None), 0, &[WORD]).unwrap();
    let left = text(&pdf);
    assert!(!left.contains("Page"), "{left:?}");
    assert!(left.contains('1'), "{left:?}");
    let ops = operators(&pdf);
    assert!(
        ops.ends_with(&["g".into(), "re".into(), "f".into()]),
        "{ops:?}"
    );
}

#[test]
fn whole_line_can_be_redacted() {
    let line = Rect { x1: 300.0, ..WORD };
    let pdf = redact(&make_pdf(1, None), 0, &[line]).unwrap();
    assert!(text(&pdf).trim().is_empty());
}

#[test]
fn text_elsewhere_is_untouched() {
    let elsewhere = WORD.translated(0.0, -300.0);
    let pdf = redact(&make_pdf(1, None), 0, &[elsewhere]).unwrap();
    assert!(text(&pdf).contains("Page 1"));
}

#[test]
fn images_touching_an_area_are_removed() {
    let pdf = edit(&make_pdf(1, None), |doc, pages| {
        let image = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => 1, "Height" => 1,
                "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
            },
            vec![0x80],
        ));
        let content = b"q 50 0 0 50 300 300 cm /Im1 Do Q q 50 0 0 50 400 400 cm /Im1 Do Q";
        let contents = doc.add_object(Stream::new(dictionary! {}, content.to_vec()));
        let page = doc.get_dictionary_mut(pages[0]).unwrap();
        page.set("Contents", contents);
        page.set(
            "Resources",
            dictionary! { "XObject" => dictionary! { "Im1" => image } },
        );
    });
    let hole = Rect {
        x0: 310.0,
        y0: 310.0,
        x1: 320.0,
        y1: 320.0,
    };
    let pdf = redact(&pdf, 0, &[hole]).unwrap();
    let draws = operators(&pdf).iter().filter(|op| *op == "Do").count();
    assert_eq!(draws, 1);
}

#[test]
fn annotations_in_the_area_are_removed() {
    let color = Color {
        r: 1.0,
        g: 1.0,
        b: 0.0,
    };
    let inside = Annotation::Highlight {
        rect: WORD,
        color,
        quads: Vec::new(),
    };
    let outside = Annotation::Highlight {
        rect: WORD.translated(0.0, -300.0),
        color,
        quads: Vec::new(),
    };
    let pdf = add_annotation(&make_pdf(1, None), 0, &inside).unwrap();
    let pdf = add_annotation(&pdf, 0, &outside).unwrap();
    let pdf = redact(&pdf, 0, &[WORD]).unwrap();
    let left = annotations(&pdf, 0).unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].annotation, Some(outside));
}

#[test]
fn shared_content_of_other_pages_is_kept() {
    let pdf = redact(&make_pdf(2, None), 0, &[WORD]).unwrap();
    assert!(
        godpdf_core::extract_text(&pdf, 1)
            .unwrap()
            .contains("Page 2")
    );
}

#[test]
fn nothing_to_redact_or_bad_area_is_refused() {
    let pdf = make_pdf(1, None);
    assert!(matches!(
        redact(&pdf, 0, &[]),
        Err(Error::InvalidArgument(_))
    ));
    let bad = Rect {
        x0: f32::NAN,
        ..WORD
    };
    assert!(redact(&pdf, 0, &[bad]).is_err());
    assert!(matches!(
        redact(&pdf, 3, &[WORD]),
        Err(Error::PageOutOfRange { .. })
    ));
}

#[test]
fn kerning_in_tj_keeps_the_kept_glyphs_in_place() {
    // "AB" with a gap, then "CD": redacting B must leave A, C and D readable.
    let pdf = edit(&make_pdf(1, None), |doc, pages| {
        let content = b"BT /F1 20 Tf 100 100 Td [(AB) -500 (CD)] TJ ET";
        let contents = doc.add_object(Stream::new(dictionary! {}, content.to_vec()));
        doc.get_dictionary_mut(pages[0])
            .unwrap()
            .set("Contents", Object::Reference(contents));
    });
    // A is 667/1000 * 20 = 13.3 wide: B spans 113.3..126.7.
    let b = Rect {
        x0: 115.0,
        y0: 95.0,
        x1: 125.0,
        y1: 115.0,
    };
    let pdf = redact(&pdf, 0, &[b]).unwrap();
    let left = text(&pdf);
    assert!(
        left.contains('A') && left.contains("CD") && !left.contains('B'),
        "{left:?}"
    );
}

/// Whether page 0 paints in grey `level` (0 black, 1 white).
fn paints_grey(pdf: &[u8], level: f32) -> bool {
    let doc = Document::load_mem(pdf).unwrap();
    let page = doc.page_iter().next().unwrap();
    Content::decode(&doc.get_page_content(page))
        .unwrap()
        .operations
        .iter()
        .any(|op| op.operator == "g" && op.operands[0].as_float().ok() == Some(level))
}

#[test]
fn white_cover_removes_the_text_and_paints_white() {
    let white = Color {
        r: 1.0,
        g: 1.0,
        b: 1.0,
    };
    let pdf = godpdf_core::redact_with(&make_pdf(1, None), 0, &[WORD], Some(white)).unwrap();
    assert!(!text(&pdf).contains("Page"));
    let ops = operators(&pdf);
    assert!(
        ops.ends_with(&["g".into(), "re".into(), "f".into()]),
        "{ops:?}"
    );
    assert!(paints_grey(&pdf, 1.0));
    // No cover: taken out, nothing painted.
    let bare = godpdf_core::redact_with(&make_pdf(1, None), 0, &[WORD], None).unwrap();
    assert!(!text(&bare).contains("Page"));
    assert!(!operators(&bare).iter().any(|op| op == "re"));
}

#[test]
fn erasing_page_content_keeps_images_and_comments_under_white() {
    let pdf = edit(&make_pdf(1, None), |doc, pages| {
        let image = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => 1, "Height" => 1,
                "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
            },
            vec![0x80],
        ));
        let content = b"BT /F1 24 Tf 72 700 Td (Page 1) Tj ET q 50 0 0 50 60 690 cm /Im1 Do Q";
        let contents = doc.add_object(Stream::new(dictionary! {}, content.to_vec()));
        let page = doc.get_dictionary_mut(pages[0]).unwrap();
        page.set("Contents", contents);
        let fonts = page
            .get(b"Resources")
            .and_then(Object::as_dict)
            .and_then(|resources| resources.get(b"Font"))
            .cloned()
            .unwrap_or(Object::Dictionary(dictionary! {}));
        page.set(
            "Resources",
            dictionary! { "Font" => fonts, "XObject" => dictionary! { "Im1" => image } },
        );
    });
    let color = Color {
        r: 1.0,
        g: 1.0,
        b: 0.0,
    };
    let pdf = add_annotation(
        &pdf,
        0,
        &Annotation::Highlight {
            rect: WORD,
            color,
            quads: Vec::new(),
        },
    )
    .unwrap();
    let erased = godpdf_core::erase_content(&pdf, 0, &[WORD]).unwrap();
    assert!(!text(&erased).contains("Page"), "{}", text(&erased));
    assert_eq!(
        operators(&erased).iter().filter(|op| *op == "Do").count(),
        1
    );
    assert_eq!(annotations(&erased, 0).unwrap().len(), 1);
    assert!(paints_grey(&erased, 1.0));
}
