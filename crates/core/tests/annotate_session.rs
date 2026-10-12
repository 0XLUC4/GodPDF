mod common;

use common::{annot_count, make_pdf, page_dict};
use godpdf_core::{
    Annotation, Color, Error, Rect, Session, add_annotation, page_count, rotate_pages,
};
use lopdf::{Document, Object};

const RED: Color = Color {
    r: 1.0,
    g: 0.0,
    b: 0.0,
};
const RECT: Rect = Rect {
    x0: 100.0,
    y0: 100.0,
    x1: 200.0,
    y1: 150.0,
};

fn all_kinds() -> Vec<Annotation> {
    vec![
        Annotation::Highlight {
            rect: RECT,
            color: Color {
                r: 1.0,
                g: 1.0,
                b: 0.0,
            },
        },
        Annotation::Note {
            rect: Rect {
                x0: 10.0,
                y0: 10.0,
                x1: 34.0,
                y1: 34.0,
            },
            text: "Note é".into(),
        },
        Annotation::FreeText {
            rect: RECT,
            text: "Line (1)\nLine \\2 — ü".into(),
            font_size: 12.0,
            color: RED,
            rotation: 0,
        },
        Annotation::Square {
            rect: RECT,
            color: RED,
            width: 2.0,
        },
        Annotation::Ink {
            strokes: vec![
                vec![(10.0, 10.0), (50.0, 60.0), (90.0, 20.0)],
                vec![(5.0, 5.0)],
            ],
            color: RED,
            width: 3.0,
        },
    ]
}

fn subtype(pdf: &[u8], page: usize, index: usize) -> Vec<u8> {
    let doc = Document::load_mem(pdf).unwrap();
    let id = doc.page_iter().nth(page).unwrap();
    let annots = doc.get_page_annotations(id).unwrap();
    let annot = annots[index];
    assert!(
        annot.get(b"AP").unwrap().as_dict().unwrap().has(b"N"),
        "missing appearance"
    );
    annot.get(b"Subtype").unwrap().as_name().unwrap().to_vec()
}

#[test]
fn every_kind_is_appended_with_appearance() {
    let mut pdf = make_pdf(2, None);
    let expected: [&[u8]; 5] = [b"Highlight", b"Text", b"FreeText", b"Square", b"Ink"];
    for (i, annotation) in all_kinds().iter().enumerate() {
        pdf = add_annotation(&pdf, 1, annotation).unwrap();
        assert_eq!(page_count(&pdf).unwrap(), 2);
        assert_eq!(annot_count(&pdf, 1), i + 1);
        assert_eq!(subtype(&pdf, 1, i), expected[i]);
    }
    assert_eq!(annot_count(&pdf, 0), 0);
}

#[test]
fn appends_to_indirect_annots_array() {
    let mut doc = Document::load_mem(&make_pdf(1, None)).unwrap();
    let page = doc.page_iter().next().unwrap();
    let array = doc.add_object(Object::Array(vec![]));
    doc.get_dictionary_mut(page).unwrap().set("Annots", array);
    let mut pdf = Vec::new();
    doc.save_to(&mut pdf).unwrap();
    let out = add_annotation(&pdf, 0, &all_kinds()[0]).unwrap();
    assert_eq!(annot_count(&out, 0), 1);
    assert!(
        page_dict(&out, 0)
            .get(b"Annots")
            .unwrap()
            .as_reference()
            .is_ok()
    );
}

#[test]
fn annotation_rejects_bad_input() {
    let pdf = make_pdf(1, None);
    assert!(matches!(
        add_annotation(&pdf, 1, &all_kinds()[0]),
        Err(Error::PageOutOfRange { .. })
    ));
    let empty_ink = Annotation::Ink {
        strokes: vec![],
        color: RED,
        width: 1.0,
    };
    assert!(matches!(
        add_annotation(&pdf, 0, &empty_ink),
        Err(Error::InvalidArgument(_))
    ));
    let bad_font = Annotation::FreeText {
        rect: RECT,
        text: "x".into(),
        font_size: 0.0,
        color: RED,
        rotation: 0,
    };
    assert!(matches!(
        add_annotation(&pdf, 0, &bad_font),
        Err(Error::InvalidArgument(_))
    ));
}

#[test]
fn session_apply_undo_redo() {
    let original = make_pdf(2, None);
    let mut session = Session::new("doc.pdf", original.clone());
    assert!(!session.is_dirty());
    assert_eq!(session.undo_label(), None);
    assert!(!session.undo());

    session
        .apply("Rotate", |pdf| rotate_pages(pdf, &[0], 90))
        .unwrap();
    let rotated = session.bytes().to_vec();
    assert!(session.is_dirty());
    assert_eq!(session.undo_label(), Some("Rotate"));
    assert_eq!(session.redo_label(), None);

    assert!(session.undo());
    assert_eq!(session.bytes(), original.as_slice());
    assert_eq!(session.redo_label(), Some("Rotate"));
    assert_eq!(session.undo_label(), None);

    assert!(session.redo());
    assert_eq!(session.bytes(), rotated.as_slice());
    assert!(!session.redo());

    session.mark_saved();
    assert!(!session.is_dirty());
    assert_eq!(session.undo_label(), Some("Rotate"));
}

#[test]
fn session_new_edit_clears_redo() {
    let mut session = Session::new("doc.pdf", make_pdf(2, None));
    session
        .apply("A", |pdf| rotate_pages(pdf, &[0], 90))
        .unwrap();
    session.undo();
    session
        .apply("B", |pdf| rotate_pages(pdf, &[1], 90))
        .unwrap();
    assert_eq!(session.redo_label(), None);
    assert_eq!(session.undo_label(), Some("B"));
}

#[test]
fn session_error_leaves_state_untouched() {
    let original = make_pdf(2, None);
    let mut session = Session::new("doc.pdf", original.clone());
    let result = session.apply("Bad", |pdf| rotate_pages(pdf, &[0], 45));
    assert!(matches!(result, Err(Error::InvalidArgument(_))));
    assert_eq!(session.bytes(), original.as_slice());
    assert!(!session.is_dirty());
    assert_eq!(session.undo_label(), None);
}

#[test]
fn session_history_is_capped() {
    let mut session = Session::new("doc.pdf", vec![0]);
    for i in 0..150u8 {
        session.apply("step", |_| Ok(vec![i])).unwrap();
    }
    let mut undos = 0;
    while session.undo() {
        undos += 1;
    }
    assert_eq!(undos, 100);
}

#[test]
fn session_amend_folds_into_last_step() {
    let mut session = godpdf_core::Session::new("doc", make_pdf(3, None));
    let original = session.bytes().to_vec();
    session
        .apply("Edit text", |pdf| godpdf_core::rotate_pages(pdf, &[0], 90))
        .unwrap();
    session
        .amend("Edit text", |pdf| godpdf_core::rotate_pages(pdf, &[1], 90))
        .unwrap();
    assert_eq!(session.undo_label(), Some("Edit text"));
    assert!(session.undo());
    assert_eq!(session.bytes(), original.as_slice());
    assert!(!session.undo());
}
