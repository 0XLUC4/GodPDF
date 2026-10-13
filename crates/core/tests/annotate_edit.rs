mod common;

use common::{edit, make_pdf};
use godpdf_core::{
    Annotation, Color, Error, FREE_TEXT_LINE_HEIGHT, FREE_TEXT_PADDING, Rect, add_annotation,
    annotations, remove_annotation, replace_annotation, wrap_free_text,
};
use lopdf::{Object, dictionary};

const BLUE: Color = Color {
    r: 0.0,
    g: 0.2,
    b: 0.8,
};
const RECT: Rect = Rect {
    x0: 100.0,
    y0: 100.0,
    x1: 220.0,
    y1: 160.0,
};

fn free_text(text: &str, rect: Rect) -> Annotation {
    Annotation::FreeText {
        rect,
        text: text.into(),
        font_size: 13.5,
        color: BLUE,
        rotation: 0,
        font: godpdf_core::Font::Helvetica,
        styles: Vec::new(),
    }
}

fn assert_free_text(annotation: &Option<Annotation>, text: &str, rect: Rect) {
    let Some(Annotation::FreeText {
        rect: got_rect,
        text: got_text,
        font_size,
        color,
        ..
    }) = annotation
    else {
        panic!("not a FreeText: {annotation:?}");
    };
    assert_eq!(got_text, text);
    assert!((font_size - 13.5).abs() < 0.01);
    assert!((color.g - BLUE.g).abs() < 0.001 && (color.b - BLUE.b).abs() < 0.001);
    assert_eq!(*got_rect, text_rect(rect, text.lines().count()));
}

/// A free-text box keeps its top edge and width; its height follows its lines.
fn text_rect(rect: Rect, lines: usize) -> Rect {
    Rect {
        y0: rect.y1 - (lines as f32 * 13.5 * FREE_TEXT_LINE_HEIGHT + 2.0 * FREE_TEXT_PADDING),
        ..rect
    }
}

#[test]
fn free_text_wraps_inside_its_box_and_grows_downwards() {
    // 60 pt wide box, 12 pt Helvetica: "Hello" is 27.3 pt, so two words never share a line.
    let lines = wrap_free_text("Hello world again\nnew", 12.0, 60.0);
    assert_eq!(lines, ["Hello", "world", "again", "new"]);
    // A word wider than the box is split instead of sticking out.
    let split = wrap_free_text("Supercalifragilistic", 12.0, 40.0);
    assert!(split.len() > 1);
    assert_eq!(split.concat(), "Supercalifragilistic");
    assert_eq!(wrap_free_text("", 12.0, 60.0), [""]);

    let annotation = Annotation::FreeText {
        rect: Rect {
            x0: 100.0,
            y0: 690.0,
            x1: 160.0,
            y1: 700.0,
        },
        text: "Hello world again".into(),
        font_size: 12.0,
        color: BLUE,
        rotation: 0,
        font: godpdf_core::Font::Helvetica,
        styles: Vec::new(),
    };
    let pdf = add_annotation(&make_pdf(1, None), 0, &annotation).unwrap();
    let placed = &annotations(&pdf, 0).unwrap()[0];
    // Same top edge and width, tall enough for the three wrapped lines.
    assert_eq!(
        (placed.rect.x0, placed.rect.x1, placed.rect.y1),
        (100.0, 160.0, 700.0)
    );
    let height = 3.0 * 12.0 * FREE_TEXT_LINE_HEIGHT + 2.0 * FREE_TEXT_PADDING;
    assert!((placed.rect.y1 - placed.rect.y0 - height).abs() < 0.01);
}

#[test]
fn fitted_maps_rectangles_and_ink_onto_the_new_box() {
    let from = Rect {
        x0: 0.0,
        y0: 0.0,
        x1: 10.0,
        y1: 10.0,
    };
    let to = Rect {
        x0: 100.0,
        y0: 100.0,
        x1: 120.0,
        y1: 140.0,
    };
    let square = Annotation::Square {
        rect: from,
        color: BLUE,
        width: 1.0,
    };
    assert_eq!(
        square.fitted(from, to),
        Annotation::Square {
            rect: to,
            color: BLUE,
            width: 1.0
        }
    );
    let ink = Annotation::Ink {
        strokes: vec![vec![(0.0, 0.0), (5.0, 5.0), (10.0, 10.0)]],
        color: BLUE,
        width: 1.0,
    };
    let Annotation::Ink { strokes, .. } = ink.fitted(from, to) else {
        panic!("not ink");
    };
    assert_eq!(strokes[0], [(100.0, 100.0), (110.0, 120.0), (120.0, 140.0)]);
}

#[test]
fn free_text_round_trip_replace_remove() {
    let pdf = make_pdf(1, None);
    let pdf = add_annotation(
        &pdf,
        0,
        &Annotation::Highlight {
            rect: RECT,
            color: BLUE,
        },
    )
    .unwrap();
    let pdf = add_annotation(&pdf, 0, &free_text("Hello", RECT)).unwrap();
    let listed = annotations(&pdf, 0).unwrap();
    assert_eq!(listed.len(), 2);
    let id = listed[1].id.clone();
    assert!(id.starts_with("godpdf-"));
    assert_eq!(listed[1].rect, text_rect(RECT, 1));
    assert_free_text(&listed[1].annotation, "Hello", RECT);

    let moved = Rect {
        x0: 10.0,
        y0: 20.0,
        x1: 90.0,
        y1: 60.0,
    };
    let pdf = replace_annotation(&pdf, 0, &id, &free_text("Évangile\nline 2", moved)).unwrap();
    let listed = annotations(&pdf, 0).unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[1].id, id);
    assert_eq!(listed[1].rect, text_rect(moved, 2));
    assert_free_text(&listed[1].annotation, "Évangile\nline 2", moved);
    assert!(matches!(
        listed[0].annotation,
        Some(Annotation::Highlight { .. })
    ));

    let pdf = remove_annotation(&pdf, 0, &id).unwrap();
    let listed = annotations(&pdf, 0).unwrap();
    assert_eq!(listed.len(), 1);
    assert!(listed.iter().all(|placed| placed.id != id));
}

#[test]
fn every_kind_parses_back() {
    let kinds = [
        Annotation::Note {
            rect: RECT,
            text: "Note é".into(),
        },
        Annotation::Square {
            rect: RECT,
            color: BLUE,
            width: 2.5,
        },
        Annotation::Ink {
            strokes: vec![vec![(10.0, 10.0), (50.0, 60.0)], vec![(5.0, 5.0)]],
            color: BLUE,
            width: 3.0,
        },
    ];
    let mut pdf = make_pdf(1, None);
    for kind in &kinds {
        pdf = add_annotation(&pdf, 0, kind).unwrap();
    }
    let parsed: Vec<Annotation> = annotations(&pdf, 0)
        .unwrap()
        .into_iter()
        .map(|placed| placed.annotation.unwrap())
        .collect();
    assert_eq!(parsed, kinds);
}

#[test]
fn foreign_link_gets_synthetic_id() {
    let pdf = edit(&make_pdf(1, None), |doc, pages| {
        let link = doc.add_object(dictionary! {
            "Type" => "Annot",
            "Subtype" => "Link",
            "Rect" => vec![50.into(), 60.into(), 10.into(), 20.into()],
        });
        let annots = doc.add_object(Object::Array(vec![Object::Null, link.into()]));
        doc.get_dictionary_mut(pages[0])
            .unwrap()
            .set("Annots", annots);
    });
    let listed = annotations(&pdf, 0).unwrap();
    assert_eq!(listed.len(), 1);
    // Index in /Annots, nulls included.
    assert_eq!(listed[0].id, "#1");
    assert_eq!(listed[0].annotation, None);
    assert_eq!(
        listed[0].rect,
        Rect {
            x0: 10.0,
            y0: 20.0,
            x1: 50.0,
            y1: 60.0
        }
    );

    let replaced = replace_annotation(&pdf, 0, "#1", &free_text("x", RECT)).unwrap();
    let listed = annotations(&replaced, 0).unwrap();
    assert_eq!(listed[0].id, "#1");
    assert_free_text(&listed[0].annotation, "x", RECT);

    let removed = remove_annotation(&pdf, 0, "#1").unwrap();
    assert!(annotations(&removed, 0).unwrap().is_empty());
}

#[test]
fn link_without_nm_is_hash_zero() {
    let pdf = edit(&make_pdf(1, None), |doc, pages| {
        let link = dictionary! {
            "Type" => "Annot",
            "Subtype" => "Link",
            "Rect" => vec![0.into(), 0.into(), 10.into(), 10.into()],
        };
        doc.get_dictionary_mut(pages[0])
            .unwrap()
            .set("Annots", vec![Object::Dictionary(link)]);
    });
    let listed = annotations(&pdf, 0).unwrap();
    assert_eq!(listed[0].id, "#0");
    assert_eq!(listed[0].annotation, None);
    let pdf = remove_annotation(&pdf, 0, "#0").unwrap();
    assert!(annotations(&pdf, 0).unwrap().is_empty());
}

#[test]
fn bad_ids_and_pages_are_rejected() {
    let pdf = add_annotation(&make_pdf(1, None), 0, &free_text("a", RECT)).unwrap();
    assert!(matches!(
        remove_annotation(&pdf, 0, "nope"),
        Err(Error::InvalidArgument(_))
    ));
    assert!(matches!(
        replace_annotation(&pdf, 0, "#5", &free_text("a", RECT)),
        Err(Error::InvalidArgument(_))
    ));
    assert!(matches!(
        annotations(&pdf, 3),
        Err(Error::PageOutOfRange { page: 3, count: 1 })
    ));
    assert!(annotations(&make_pdf(1, None), 0).unwrap().is_empty());
}

#[test]
fn free_text_rect_keeps_its_anchor_through_a_round_trip() {
    use godpdf_core::free_text_rect;
    let anchor = (100.0, 700.0);
    let rect = free_text_rect(anchor, "Hello\nworld", 12.0);
    assert_eq!((rect.x0, rect.y1), anchor);
    let height = 2.0 * 12.0 * FREE_TEXT_LINE_HEIGHT + 2.0 * FREE_TEXT_PADDING;
    assert!((rect.y1 - rect.y0 - height).abs() < 0.01);
    // Helvetica "Hello" at 12 pt is 27.34 pt wide.
    let width = free_text_rect(anchor, "Hello", 12.0).x1 - anchor.0 - 2.0 * FREE_TEXT_PADDING;
    assert!((width - 27.34).abs() < 0.2, "width {width}");

    let annotation = Annotation::FreeText {
        rect,
        text: "Hello\nworld".into(),
        font_size: 12.0,
        color: Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
        },
        rotation: 0,
        font: godpdf_core::Font::Helvetica,
        styles: Vec::new(),
    };
    let pdf = add_annotation(&make_pdf(1, None), 0, &annotation).unwrap();
    let placed = &annotations(&pdf, 0).unwrap()[0];
    assert!((placed.rect.x0 - anchor.0).abs() < 0.01);
    assert!((placed.rect.y1 - anchor.1).abs() < 0.01);
    assert_eq!(placed.annotation.as_ref(), Some(&annotation));
}
