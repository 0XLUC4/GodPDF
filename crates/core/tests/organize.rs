mod common;

use common::{
    annot_count, edit, make_nested_pdf, make_pdf, make_pdf_with_outline, page_dict, page_texts,
    texts,
};
use godpdf_core::{
    Error, OutlineItem, delete_pages, extract_pages, insert_blank_page, insert_pdf, merge, outline,
    page_count, reorder_pages, rotate_pages, split_at, split_every,
};
use lopdf::{Document, Object, ObjectId, dictionary};

fn rotation(pdf: &[u8], index: usize) -> i64 {
    page_dict(pdf, index)
        .get(b"Rotate")
        .and_then(Object::as_i64)
        .unwrap_or(0)
}

#[test]
fn page_count_matches_fixture() {
    assert_eq!(page_count(&make_pdf(4, None)).unwrap(), 4);
}

#[test]
fn garbage_is_parse_error() {
    assert!(matches!(page_count(b"not a pdf"), Err(Error::Parse(_))));
}

#[test]
fn merge_concatenates_in_order() {
    let a = make_pdf(2, None);
    let b = make_pdf(3, None);
    let merged = merge(&[&a, &b]).unwrap();
    assert_eq!(page_count(&merged).unwrap(), 5);
    assert_eq!(page_texts(&merged), texts(&[1, 2, 1, 2, 3]));
}

#[test]
fn merge_empty_input_is_error() {
    assert!(matches!(merge(&[]), Err(Error::InvalidArgument(_))));
}

#[test]
fn merge_builds_outline_per_input() {
    let a = make_pdf(2, Some("Alpha"));
    let b = common::make_pdf_with_outline(3, 2);
    let merged = merge(&[&a, &b]).unwrap();
    let items = outline(&merged).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].title, "Alpha");
    assert_eq!(items[0].page, Some(0));
    assert_eq!(items[1].title, "Document 2");
    assert_eq!(items[1].page, Some(2));
    assert_eq!(items[1].children.len(), 1);
    assert_eq!(items[1].children[0].title, "Chapter");
    assert_eq!(items[1].children[0].page, Some(4));
}

#[test]
fn extract_keeps_order_and_duplicates() {
    let pdf = make_pdf(4, None);
    let out = extract_pages(&pdf, &[3, 0, 0]).unwrap();
    assert_eq!(page_texts(&out), texts(&[4, 1, 1]));
}

#[test]
fn extract_rejects_empty_and_out_of_range() {
    let pdf = make_pdf(2, None);
    assert!(matches!(
        extract_pages(&pdf, &[]),
        Err(Error::InvalidArgument(_))
    ));
    assert!(matches!(
        extract_pages(&pdf, &[2]),
        Err(Error::PageOutOfRange { page: 2, count: 2 })
    ));
}

#[test]
fn delete_removes_pages() {
    let pdf = make_pdf(4, None);
    let out = delete_pages(&pdf, &[1, 3, 1]).unwrap();
    assert_eq!(page_texts(&out), texts(&[1, 3]));
}

#[test]
fn delete_all_is_error() {
    let pdf = make_pdf(2, None);
    assert!(matches!(
        delete_pages(&pdf, &[0, 1]),
        Err(Error::InvalidArgument(_))
    ));
    assert!(matches!(
        delete_pages(&pdf, &[5]),
        Err(Error::PageOutOfRange { .. })
    ));
}

#[test]
fn delete_drops_outline_to_removed_page() {
    let pdf = make_pdf_with_outline(3, 1);
    let out = delete_pages(&pdf, &[1]).unwrap();
    assert!(outline(&out).unwrap().is_empty());
}

#[test]
fn rotate_accumulates_and_normalizes() {
    let pdf = make_pdf(3, None);
    let once = rotate_pages(&pdf, &[0, 2], 90).unwrap();
    assert_eq!(
        [rotation(&once, 0), rotation(&once, 1), rotation(&once, 2)],
        [90, 0, 90]
    );
    let back = rotate_pages(&once, &[0], -180).unwrap();
    assert_eq!(rotation(&back, 0), 270);
    let full = rotate_pages(&pdf, &[1], 450).unwrap();
    assert_eq!(rotation(&full, 1), 90);
}

#[test]
fn rotate_rejects_bad_angles_and_selection() {
    let pdf = make_pdf(2, None);
    assert!(matches!(
        rotate_pages(&pdf, &[0], 45),
        Err(Error::InvalidArgument(_))
    ));
    assert!(matches!(
        rotate_pages(&pdf, &[], 90),
        Err(Error::InvalidArgument(_))
    ));
    assert!(matches!(
        rotate_pages(&pdf, &[9], 90),
        Err(Error::PageOutOfRange { .. })
    ));
}

#[test]
fn rotate_respects_inherited_rotation() {
    let pdf = make_nested_pdf(4);
    let out = rotate_pages(&pdf, &[0], 90).unwrap();
    assert_eq!(rotation(&out, 0), 180);
    assert_eq!(rotation(&out, 1), 90);
}

#[test]
fn reorder_applies_permutation() {
    let pdf = make_pdf(3, None);
    let out = reorder_pages(&pdf, &[2, 0, 1]).unwrap();
    assert_eq!(page_texts(&out), texts(&[3, 1, 2]));
}

#[test]
fn reorder_rejects_non_permutation() {
    let pdf = make_pdf(3, None);
    for bad in [&[0, 1][..], &[0, 0, 1], &[0, 1, 3], &[0, 1, 2, 3]] {
        assert!(
            matches!(reorder_pages(&pdf, bad), Err(Error::InvalidArgument(_))),
            "{bad:?}"
        );
    }
}

#[test]
fn reorder_flattens_nested_tree_and_keeps_inherited_attributes() {
    let pdf = make_nested_pdf(4);
    let out = reorder_pages(&pdf, &[3, 2, 1, 0]).unwrap();
    assert_eq!(page_texts(&out), texts(&[4, 3, 2, 1]));
    let page = page_dict(&out, 0);
    let media_box = page.get(b"MediaBox").unwrap().as_array().unwrap();
    assert_eq!(media_box[2].as_i64().unwrap(), 300);
    assert!(page.has(b"Resources"));
}

#[test]
fn insert_blank_at_start_and_end() {
    let pdf = make_pdf(2, None);
    let start = insert_blank_page(&pdf, 0, 200.0, 100.0).unwrap();
    assert_eq!(
        page_texts(&start),
        vec![String::new(), "Page 1".into(), "Page 2".into()]
    );
    let media_box = page_dict(&start, 0)
        .get(b"MediaBox")
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(media_box[2].as_float().unwrap(), 200.0);
    assert_eq!(media_box[3].as_float().unwrap(), 100.0);
    let end = insert_blank_page(&pdf, 2, 612.0, 792.0).unwrap();
    assert_eq!(
        page_texts(&end),
        vec!["Page 1".into(), "Page 2".into(), String::new()]
    );
}

#[test]
fn insert_blank_rejects_bad_args() {
    let pdf = make_pdf(2, None);
    assert!(matches!(
        insert_blank_page(&pdf, 3, 100.0, 100.0),
        Err(Error::PageOutOfRange { page: 3, count: 3 })
    ));
    assert!(matches!(
        insert_blank_page(&pdf, 0, 0.0, 100.0),
        Err(Error::InvalidArgument(_))
    ));
    assert!(matches!(
        insert_blank_page(&pdf, 0, f32::NAN, 100.0),
        Err(Error::InvalidArgument(_))
    ));
}

#[test]
fn insert_pdf_in_the_middle() {
    let pdf = make_pdf(2, None);
    let other = make_pdf(3, None);
    let out = insert_pdf(&pdf, 1, &other).unwrap();
    assert_eq!(page_texts(&out), texts(&[1, 1, 2, 3, 2]));
    assert!(matches!(
        insert_pdf(&pdf, 3, &other),
        Err(Error::PageOutOfRange { .. })
    ));
}

#[test]
fn split_every_chunks() {
    let pdf = make_pdf(5, None);
    let parts = split_every(&pdf, 2).unwrap();
    let sizes: Vec<usize> = parts.iter().map(|p| page_count(p).unwrap()).collect();
    assert_eq!(sizes, [2, 2, 1]);
    assert_eq!(page_texts(&parts[2]), texts(&[5]));
    assert!(matches!(
        split_every(&pdf, 0),
        Err(Error::InvalidArgument(_))
    ));
}

#[test]
fn split_at_starts() {
    let pdf = make_pdf(5, None);
    let parts = split_at(&pdf, &[3, 1, 3, 0]).unwrap();
    let sizes: Vec<usize> = parts.iter().map(|p| page_count(p).unwrap()).collect();
    assert_eq!(sizes, [1, 2, 2]);
    assert_eq!(page_texts(&parts[1]), texts(&[2, 3]));
    assert_eq!(split_at(&pdf, &[]).unwrap().len(), 1);
    assert!(matches!(
        split_at(&pdf, &[5]),
        Err(Error::PageOutOfRange { .. })
    ));
}

fn item(title: &str, page: Option<usize>, children: Vec<OutlineItem>) -> OutlineItem {
    OutlineItem {
        title: title.into(),
        page,
        children,
    }
}

fn link(doc: &mut Document, page: ObjectId, dest: Object, via_action: bool) -> Object {
    let mut annot = dictionary! { "Type" => "Annot", "Subtype" => "Link",
    "Rect" => vec![0.into(), 0.into(), 10.into(), 10.into()] };
    if via_action {
        annot.set("A", dictionary! { "S" => "GoTo", "D" => dest });
    } else {
        annot.set("Dest", dest);
    }
    annot.set("P", page);
    doc.add_object(annot).into()
}

/// 4 pages. Outline: A->p1, B->p3, C->p3 { D->p2 }. Page 1 links to p3 (Dest) and p2 (action).
/// OpenAction -> p3. AcroForm /Fields holds a null entry.
fn linked_pdf() -> Vec<u8> {
    edit(&make_pdf(4, None), |doc, pages| {
        let fit = |page: ObjectId| Object::Array(vec![page.into(), "Fit".into()]);
        let outlines = doc.new_object_id();
        let [a, b, c, d] = [0; 4].map(|_| doc.new_object_id());
        let entries = [
            (a, "A", pages[0], outlines, None, Some(b)),
            (b, "B", pages[2], outlines, Some(a), Some(c)),
            (c, "C", pages[2], outlines, Some(b), None),
            (d, "D", pages[1], c, None, None),
        ];
        for (id, title, target, parent, prev, next) in entries {
            let mut dict = dictionary! { "Title" => Object::string_literal(title),
            "Parent" => parent, "Dest" => fit(target) };
            if let Some(prev) = prev {
                dict.set("Prev", prev);
            }
            if let Some(next) = next {
                dict.set("Next", next);
            }
            doc.objects.insert(id, Object::Dictionary(dict));
        }
        let c_dict = doc.get_dictionary_mut(c).unwrap();
        c_dict.set("First", d);
        c_dict.set("Last", d);
        c_dict.set("Count", 1);
        doc.objects.insert(
            outlines,
            Object::Dictionary(
                dictionary! { "Type" => "Outlines", "First" => a, "Last" => c, "Count" => 4 },
            ),
        );
        let gone = link(doc, pages[0], fit(pages[2]), false);
        let kept = link(doc, pages[0], fit(pages[1]), true);
        doc.get_dictionary_mut(pages[0])
            .unwrap()
            .set("Annots", vec![gone, kept]);
        let field =
            doc.add_object(dictionary! { "FT" => "Tx", "T" => Object::string_literal("f") });
        let catalog = doc.catalog_mut().unwrap();
        catalog.set("Outlines", outlines);
        catalog.set("OpenAction", fit(pages[2]));
        catalog.set(
            "AcroForm",
            dictionary! { "Fields" => vec![Object::Null, field.into()] },
        );
    })
}

#[test]
fn delete_drops_outline_links_and_open_action_to_removed_page() {
    let out = delete_pages(&linked_pdf(), &[2]).unwrap();
    assert_eq!(
        outline(&out).unwrap(),
        [
            item("A", Some(0), vec![]),
            item("C", None, vec![item("D", Some(1), vec![])]),
        ]
    );
    let doc = Document::load_mem(&out).unwrap();
    for object in doc.objects.values() {
        if let Ok(dict) = object.as_dict()
            && let Ok(dest) = dict.get(b"Dest").and_then(Object::as_array)
        {
            assert!(
                !matches!(dest.first(), Some(Object::Null)),
                "null dest left"
            );
        }
    }
    assert_eq!(annot_count(&out, 0), 1);
    let catalog = doc.catalog().unwrap();
    assert!(!catalog.has(b"OpenAction"));
    let acroform = catalog.get(b"AcroForm").and_then(Object::as_dict).unwrap();
    let fields = acroform.get(b"Fields").and_then(Object::as_array).unwrap();
    assert_eq!(fields.len(), 1);
    assert!(fields[0].as_reference().is_ok());
    // Outline root counts visible items: A, C and D (C is open).
    let outlines = doc
        .get_dictionary(
            catalog
                .get(b"Outlines")
                .and_then(Object::as_reference)
                .unwrap(),
        )
        .unwrap();
    assert_eq!(outlines.get(b"Count").and_then(Object::as_i64).unwrap(), 3);
}

#[test]
fn delete_keeps_outline_when_target_survives() {
    let out = delete_pages(&make_pdf_with_outline(4, 3), &[0]).unwrap();
    assert_eq!(outline(&out).unwrap(), [item("Chapter", Some(2), vec![])]);
    let out = delete_pages(&make_pdf_with_outline(4, 3), &[3]).unwrap();
    assert!(outline(&out).unwrap().is_empty());
}

#[test]
fn duplicated_page_does_not_copy_form_widgets() {
    let pdf = edit(&make_pdf(1, None), |doc, pages| {
        let rect = || vec![0.into(), 0.into(), 10.into(), 10.into()];
        let widget = doc.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Widget",
        "FT" => "Tx", "T" => Object::string_literal("name"), "Rect" => rect() });
        let square = doc
            .add_object(dictionary! { "Type" => "Annot", "Subtype" => "Square", "Rect" => rect() });
        doc.get_dictionary_mut(pages[0])
            .unwrap()
            .set("Annots", vec![widget.into(), square.into()]);
        let catalog = doc.catalog_mut().unwrap();
        catalog.set("AcroForm", dictionary! { "Fields" => vec![widget.into()] });
    });
    let out = extract_pages(&pdf, &[0, 0]).unwrap();
    assert_eq!(annot_count(&out, 0), 2);
    assert_eq!(annot_count(&out, 1), 1);
}

#[test]
fn split_parts_are_independent() {
    let pdf = make_pdf_with_outline(5, 2);
    let parts = split_every(&pdf, 2).unwrap();
    assert_eq!(parts.len(), 3);
    assert!(outline(&parts[0]).unwrap().is_empty());
    assert_eq!(
        outline(&parts[1]).unwrap(),
        [item("Chapter", Some(0), vec![])]
    );
    assert_eq!(page_texts(&parts[2]), texts(&[5]));
    let parts = split_at(&pdf, &[3]).unwrap();
    assert_eq!(
        outline(&parts[0]).unwrap(),
        [item("Chapter", Some(2), vec![])]
    );
    assert_eq!(page_texts(&parts[1]), texts(&[4, 5]));
}

#[test]
fn blank_document_has_one_page_of_the_given_size() {
    let pdf = godpdf_core::blank_document(595.0, 842.0).unwrap();
    assert_eq!(godpdf_core::page_count(&pdf).unwrap(), 1);
    let grown = godpdf_core::insert_blank_page(&pdf, 1, 595.0, 842.0).unwrap();
    assert_eq!(godpdf_core::page_count(&grown).unwrap(), 2);
    assert!(godpdf_core::blank_document(0.0, 842.0).is_err());
}
