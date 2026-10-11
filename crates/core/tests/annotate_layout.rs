mod common;

use common::make_pdf;
use godpdf_core::{
    Annotation, Color, FREE_TEXT_LINE_HEIGHT, FREE_TEXT_PADDING, Rect, add_annotation, annotations,
    free_text_rect, replace_annotation,
};

const BLACK: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
};

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn hello_rect_has_helvetica_width() {
    // H 722 + e 556 + l 222 + l 222 + o 556 = 2278 units = 2.278 em.
    let rect = free_text_rect((100.0, 700.0), "Hello", 12.0);
    assert!(close(rect.x0, 100.0) && close(rect.y1, 700.0));
    assert!(close(
        rect.x1 - rect.x0,
        2.278 * 12.0 + 2.0 * FREE_TEXT_PADDING
    ));
    assert!(close(
        rect.y1 - rect.y0,
        12.0 * FREE_TEXT_LINE_HEIGHT + 2.0 * FREE_TEXT_PADDING
    ));
}

#[test]
fn multi_line_rect_uses_longest_line_and_line_height() {
    let rect = free_text_rect((0.0, 500.0), "Hi\nHello\n", 10.0);
    assert!(close(rect.x1, 22.78 + 2.0 * FREE_TEXT_PADDING));
    // Three lines: the trailing newline starts an (empty) line being typed.
    assert!(close(
        rect.y1 - rect.y0,
        3.0 * 10.0 * FREE_TEXT_LINE_HEIGHT + 2.0 * FREE_TEXT_PADDING
    ));
}

#[test]
fn free_text_rect_round_trips_through_annotations() {
    let rect = free_text_rect((72.5, 640.25), "Bonjour\nà tous", 14.0);
    let annotation = Annotation::FreeText {
        rect,
        text: "Bonjour\nà tous".into(),
        font_size: 14.0,
        color: BLACK,
    };
    let pdf = add_annotation(&make_pdf(1, None), 0, &annotation).unwrap();
    let listed = annotations(&pdf, 0).unwrap();
    assert_eq!(listed[0].annotation.as_ref(), Some(&annotation));
    assert_eq!(listed[0].rect, rect);
}

#[test]
fn translated_moves_rect_and_ink_points() {
    let rect = Rect {
        x0: 10.0,
        y0: 20.0,
        x1: 30.0,
        y1: 40.0,
    };
    let square = Annotation::Square {
        rect,
        color: BLACK,
        width: 1.0,
    };
    assert_eq!(
        square.translated(5.0, -10.0),
        Annotation::Square {
            rect: Rect {
                x0: 15.0,
                y0: 10.0,
                x1: 35.0,
                y1: 30.0,
            },
            color: BLACK,
            width: 1.0,
        }
    );
    let ink = Annotation::Ink {
        strokes: vec![vec![(1.0, 2.0), (3.0, 4.0)], vec![(5.0, 6.0)]],
        color: BLACK,
        width: 2.0,
    };
    let Annotation::Ink { strokes, .. } = ink.translated(1.0, 1.0) else {
        panic!("kind changed");
    };
    assert_eq!(
        strokes,
        vec![vec![(2.0, 3.0), (4.0, 5.0)], vec![(6.0, 7.0)]]
    );
}

#[test]
fn moved_annotation_keeps_its_id() {
    let rect = free_text_rect((50.0, 300.0), "Move me", 12.0);
    let text = Annotation::FreeText {
        rect,
        text: "Move me".into(),
        font_size: 12.0,
        color: BLACK,
    };
    let pdf = add_annotation(&make_pdf(1, None), 0, &text).unwrap();
    let id = annotations(&pdf, 0).unwrap()[0].id.clone();
    let moved = text.translated(20.0, -30.0);
    let pdf = replace_annotation(&pdf, 0, &id, &moved).unwrap();
    let listed = annotations(&pdf, 0).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, id);
    assert_eq!(listed[0].annotation.as_ref(), Some(&moved));
}
