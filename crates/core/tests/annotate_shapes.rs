mod common;

use common::make_pdf;
use godpdf_core::{
    Annotation, Color, Rect, StampLabel, add_annotation, annotations, remove_annotation,
    replace_annotation,
};
use lopdf::{Document, Object};

const RED: Color = Color {
    r: 0.9,
    g: 0.1,
    b: 0.1,
};
const RECT: Rect = Rect {
    x0: 100.0,
    y0: 100.0,
    x1: 220.0,
    y1: 160.0,
};

/// Add `annotation` to a fresh page and read it back.
fn round_trip(annotation: &Annotation) -> (Vec<u8>, Rect, Annotation) {
    let pdf = add_annotation(&make_pdf(1, None), 0, annotation).unwrap();
    let placed = annotations(&pdf, 0).unwrap().remove(0);
    (pdf, placed.rect, placed.annotation.expect("parsed back"))
}

/// The only annotation dictionary of page 0, and its normal appearance content.
fn written(pdf: &[u8]) -> (lopdf::Dictionary, String) {
    let doc = Document::load_mem(pdf).unwrap();
    let page = doc.page_iter().next().unwrap();
    let annot = doc.get_page_annotations(page).unwrap()[0].clone();
    let ap = annot.get(b"AP").unwrap().as_dict().unwrap();
    let stream = doc
        .get_object(ap.get(b"N").unwrap().as_reference().unwrap())
        .unwrap()
        .as_stream()
        .unwrap();
    let content = stream
        .decompressed_content()
        .unwrap_or_else(|_| stream.content.clone());
    (annot, String::from_utf8_lossy(&content).into_owned())
}

fn subtype(dict: &lopdf::Dictionary) -> String {
    String::from_utf8_lossy(dict.get(b"Subtype").unwrap().as_name().unwrap()).into_owned()
}

#[test]
fn ellipse_round_trips_with_curves() {
    let circle = Annotation::Circle {
        rect: RECT,
        color: RED,
        width: 2.0,
        fill: None,
    };
    let (pdf, rect, back) = round_trip(&circle);
    assert_eq!(back, circle);
    assert_eq!(rect, RECT);
    let (dict, appearance) = written(&pdf);
    assert_eq!(subtype(&dict), "Circle");
    assert_eq!(appearance.matches(" c").count(), 4);
    assert_eq!(back.kind_name(), "Ellipse");
}

#[test]
fn arrow_round_trips_and_has_a_head() {
    let arrow = Annotation::Line {
        start: (100.0, 100.0),
        end: (200.0, 150.0),
        color: RED,
        width: 1.5,
        arrow: true,
    };
    let (pdf, rect, back) = round_trip(&arrow);
    assert_eq!(back, arrow);
    assert_eq!(back.kind_name(), "Arrow");
    // The box holds both ends and the head.
    assert!(rect.x0 < 100.0 && rect.y0 < 100.0 && rect.x1 > 200.0 && rect.y1 > 150.0);
    let (dict, appearance) = written(&pdf);
    assert_eq!(subtype(&dict), "Line");
    let ends = dict.get(b"LE").unwrap().as_array().unwrap();
    assert_eq!(ends[1].as_name().unwrap(), b"OpenArrow");
    // The line, then the two wings of the head.
    assert_eq!(appearance.matches(" m ").count(), 2);
}

#[test]
fn plain_line_has_no_head() {
    let line = Annotation::Line {
        start: (100.0, 100.0),
        end: (100.0, 100.0),
        color: RED,
        width: 1.0,
        arrow: false,
    };
    let (pdf, _, back) = round_trip(&line);
    assert_eq!(back, line);
    assert_eq!(back.kind_name(), "Line");
    assert_eq!(written(&pdf).1.matches(" m ").count(), 1);
}

#[test]
fn line_moves_and_stretches_with_its_box() {
    let line = Annotation::Line {
        start: (100.0, 100.0),
        end: (200.0, 200.0),
        color: RED,
        width: 1.0,
        arrow: false,
    };
    let Annotation::Line { start, end, .. } = line.translated(10.0, -5.0) else {
        panic!("not a line");
    };
    assert_eq!((start, end), ((110.0, 95.0), (210.0, 195.0)));
    let from = Rect {
        x0: 100.0,
        y0: 100.0,
        x1: 200.0,
        y1: 200.0,
    };
    let to = Rect { x1: 300.0, ..from };
    let Annotation::Line { start, end, .. } = line.fitted(from, to) else {
        panic!("not a line");
    };
    assert_eq!((start, end), ((100.0, 100.0), (300.0, 200.0)));
}

#[test]
fn underline_and_strike_out_carry_quad_points() {
    for (annotation, name, kind) in [
        (
            Annotation::Underline {
                rect: RECT,
                color: RED,
                quads: Vec::new(),
            },
            "Underline",
            "Underline",
        ),
        (
            Annotation::StrikeOut {
                rect: RECT,
                color: RED,
                quads: Vec::new(),
            },
            "StrikeOut",
            "Strikethrough",
        ),
    ] {
        let (pdf, rect, back) = round_trip(&annotation);
        assert_eq!(back, annotation);
        assert_eq!(rect, RECT);
        assert_eq!(back.kind_name(), kind);
        let (dict, _) = written(&pdf);
        assert_eq!(subtype(&dict), name);
        assert_eq!(
            dict.get(b"QuadPoints").unwrap().as_array().unwrap().len(),
            8
        );
    }
}

fn quads_of(annotation: &Annotation) -> Vec<Rect> {
    match annotation {
        Annotation::Highlight { quads, .. }
        | Annotation::Underline { quads, .. }
        | Annotation::StrikeOut { quads, .. } => quads.clone(),
        _ => unreachable!(),
    }
}

#[test]
fn text_markup_keeps_one_quad_per_line() {
    let lines = vec![
        Rect {
            x0: 150.0,
            y0: 140.0,
            x1: 220.0,
            y1: 160.0,
        },
        Rect {
            x0: 100.0,
            y0: 100.0,
            x1: 180.0,
            y1: 120.0,
        },
    ];
    let kinds = [
        Annotation::Highlight {
            rect: RECT,
            color: RED,
            quads: lines.clone(),
        },
        Annotation::Underline {
            rect: RECT,
            color: RED,
            quads: lines.clone(),
        },
        Annotation::StrikeOut {
            rect: RECT,
            color: RED,
            quads: lines.clone(),
        },
    ];
    for annotation in kinds {
        let (pdf, rect, back) = round_trip(&annotation);
        assert_eq!(back, annotation);
        assert_eq!(rect, RECT, "/Rect is the box around the lines");
        let (dict, appearance) = written(&pdf);
        let quads = dict.get(b"QuadPoints").unwrap().as_array().unwrap();
        assert_eq!(quads.len(), 16);
        assert_eq!(appearance.matches(" re").count(), 2, "{appearance}");
        // Moving moves every line; resizing scales them with the box.
        let quads = quads_of(&back.translated(10.0, -5.0));
        assert_eq!(quads[1], lines[1].translated(10.0, -5.0));
        let to = Rect {
            x0: 100.0,
            y0: 100.0,
            x1: 340.0,
            y1: 160.0,
        };
        let quads = quads_of(&back.fitted(RECT, to));
        assert_eq!((quads[0].x0, quads[0].x1), (200.0, 340.0));
        assert_eq!((quads[1].x0, quads[1].x1), (100.0, 260.0));
    }
}

#[test]
fn stamps_round_trip_with_and_without_date() {
    for label in StampLabel::ALL {
        for date in [None, Some("2026-11-10".to_string())] {
            let stamp = Annotation::Stamp {
                rect: RECT,
                label: label.clone(),
                date: date.clone(),
            };
            let (pdf, _, back) = round_trip(&stamp);
            assert_eq!(back, stamp);
            let (dict, appearance) = written(&pdf);
            assert_eq!(subtype(&dict), "Stamp");
            assert_eq!(
                appearance.matches(" Tj").count(),
                1 + usize::from(date.is_some())
            );
        }
    }
}

#[test]
fn rejected_stamp_uses_the_standard_name() {
    let stamp = Annotation::Stamp {
        rect: RECT,
        label: StampLabel::Rejected,
        date: None,
    };
    let (pdf, _, _) = round_trip(&stamp);
    let (dict, _) = written(&pdf);
    assert_eq!(
        dict.get(b"Name").unwrap(),
        &Object::Name(b"NotApproved".to_vec())
    );
}

#[test]
fn new_kinds_can_be_moved_and_removed() {
    let pdf = add_annotation(
        &make_pdf(1, None),
        0,
        &Annotation::Stamp {
            rect: RECT,
            label: StampLabel::Draft,
            date: None,
        },
    )
    .unwrap();
    let placed = annotations(&pdf, 0).unwrap().remove(0);
    let moved = placed.annotation.unwrap().translated(50.0, 0.0);
    let pdf = replace_annotation(&pdf, 0, &placed.id, &moved).unwrap();
    let again = annotations(&pdf, 0).unwrap().remove(0);
    assert_eq!(again.id, placed.id);
    assert_eq!(again.rect, RECT.translated(50.0, 0.0));
    let pdf = remove_annotation(&pdf, 0, &again.id).unwrap();
    assert!(annotations(&pdf, 0).unwrap().is_empty());
}

#[test]
fn bad_line_is_refused() {
    let line = Annotation::Line {
        start: (f32::NAN, 0.0),
        end: (1.0, 1.0),
        color: RED,
        width: 1.0,
        arrow: true,
    };
    assert!(add_annotation(&make_pdf(1, None), 0, &line).is_err());
}

const BLUE: Color = Color {
    r: 0.1,
    g: 0.2,
    b: 0.9,
};

#[test]
fn filled_rectangle_round_trips_its_interior_colour() {
    let square = Annotation::Square {
        rect: RECT,
        color: RED,
        width: 2.0,
        fill: Some(BLUE),
    };
    let (pdf, _, back) = round_trip(&square);
    assert_eq!(back, square);
    let (dict, appearance) = written(&pdf);
    let ic: Vec<f32> = dict
        .get(b"IC")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_float().unwrap())
        .collect();
    assert_eq!(ic, [0.1, 0.2, 0.9]);
    // Interior colour set, then the path filled and stroked.
    assert!(appearance.contains("0.100 0.200 0.900 rg"));
    assert!(appearance.ends_with(" re B"));
}

#[test]
fn filled_ellipse_without_border_only_fills() {
    let circle = Annotation::Circle {
        rect: RECT,
        color: RED,
        width: 0.0,
        fill: Some(BLUE),
    };
    let (pdf, _, back) = round_trip(&circle);
    assert_eq!(back, circle);
    assert!(written(&pdf).1.ends_with(" h f"));
}

#[test]
fn hollow_shape_writes_no_interior_colour() {
    let square = Annotation::Square {
        rect: RECT,
        color: RED,
        width: 1.0,
        fill: None,
    };
    let (pdf, _, back) = round_trip(&square);
    assert_eq!(back, square);
    let (dict, appearance) = written(&pdf);
    assert!(dict.get(b"IC").is_err());
    assert!(appearance.ends_with(" re S"));
}

#[test]
fn restyling_keeps_the_fill() {
    let pdf = add_annotation(
        &make_pdf(1, None),
        0,
        &Annotation::Circle {
            rect: RECT,
            color: RED,
            width: 1.0,
            fill: None,
        },
    )
    .unwrap();
    let id = annotations(&pdf, 0).unwrap().remove(0).id;
    let filled = Annotation::Circle {
        rect: RECT,
        color: RED,
        width: 1.0,
        fill: Some(BLUE),
    };
    let pdf = replace_annotation(&pdf, 0, &id, &filled).unwrap();
    let back = annotations(&pdf, 0).unwrap().remove(0).annotation.unwrap();
    assert_eq!(back, filled);
    assert_eq!(back.translated(5.0, 5.0).fitted(RECT, RECT), filled);
}
