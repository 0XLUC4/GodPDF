//! Bookmarks, contents page, layouts, colours, cleanup and attachments.

mod common;

use common::{make_pdf, make_pdf_with_outline, page_texts};
use godpdf_core::{
    Annotation, Color, OutlineItem, Rect, Tint, add_annotation, add_attachment, annotations,
    attachments, booklet, compress, contents_page, crop_pages, flatten_annotations, halve_pages,
    interleave, join_pages, merge, outline, page_count, pages_per_sheet, read_metadata,
    remove_attachment, remove_comments, remove_hidden_data, resize_pages, tint_pages,
    write_outline,
};
use lopdf::{Document, Object};

fn item(title: &str, page: usize, children: Vec<OutlineItem>) -> OutlineItem {
    OutlineItem {
        title: title.to_owned(),
        page: Some(page),
        children,
    }
}

/// Four numbers of a box of the first page.
fn first_box(pdf: &[u8], key: &[u8]) -> Vec<f32> {
    let doc = Document::load_mem(pdf).unwrap();
    let page = doc.page_iter().next().unwrap();
    doc.get_dictionary(page)
        .unwrap()
        .get(key)
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_float().unwrap())
        .collect()
}

/// MediaBox width and height of each page.
fn sizes(pdf: &[u8]) -> Vec<(f32, f32)> {
    let doc = Document::load_mem(pdf).unwrap();
    doc.page_iter()
        .map(|id| {
            let media = doc.get_dictionary(id).unwrap().get(b"MediaBox").unwrap();
            let n: Vec<f32> = media
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_float().unwrap())
                .collect();
            (n[2] - n[0], n[3] - n[1])
        })
        .collect()
}

#[test]
fn bookmarks_are_written_and_read_back() {
    let pdf = make_pdf(4, None);
    let items = vec![
        item("Intro", 0, vec![]),
        item(
            "Part",
            1,
            vec![item("Été", 2, vec![]), item("End", 3, vec![])],
        ),
    ];
    let out = write_outline(&pdf, &items).unwrap();
    assert_eq!(outline(&out).unwrap(), items);
    let cleared = write_outline(&out, &[]).unwrap();
    assert!(outline(&cleared).unwrap().is_empty());
}

#[test]
fn bookmark_to_a_missing_page_is_refused() {
    let pdf = make_pdf(2, None);
    assert!(write_outline(&pdf, &[item("Far", 5, vec![])]).is_err());
}

#[test]
fn contents_page_lists_bookmarks_and_links_to_them() {
    let pdf = write_outline(
        &make_pdf(3, None),
        &[item("First chapter", 0, vec![]), item("Second", 2, vec![])],
    )
    .unwrap();
    let out = contents_page(&pdf, "Contents").unwrap();
    assert_eq!(page_count(&out).unwrap(), 4);
    let texts = page_texts(&out);
    assert!(texts[0].contains("First chapter"), "{}", texts[0]);
    assert_eq!(texts[1], "Page 1");
    let marks = outline(&out).unwrap();
    assert_eq!(marks[0].title, "Contents");
    assert_eq!(marks[0].page, Some(0));
    assert_eq!(marks[2].page, Some(3));
    let doc = Document::load_mem(&out).unwrap();
    let first = doc.page_iter().next().unwrap();
    assert_eq!(doc.get_page_annotations(first).unwrap().len(), 2);
}

#[test]
fn contents_page_needs_bookmarks() {
    assert!(contents_page(&make_pdf(2, None), "Contents").is_err());
}

#[test]
fn several_pages_per_sheet() {
    let pdf = make_pdf(5, None);
    let out = pages_per_sheet(&pdf, 4).unwrap();
    assert_eq!(page_count(&out).unwrap(), 2);
    let two = pages_per_sheet(&pdf, 2).unwrap();
    assert_eq!(page_count(&two).unwrap(), 3);
    // Two upright pages side by side: the sheet lies on its side.
    assert_eq!(sizes(&two)[0], (792.0, 612.0));
    assert!(pages_per_sheet(&pdf, 3).is_err());
}

#[test]
fn bookmarks_follow_redrawn_pages() {
    let pdf = make_pdf_with_outline(4, 3);
    let out = pages_per_sheet(&pdf, 2).unwrap();
    assert_eq!(outline(&out).unwrap()[0].page, Some(1));
}

#[test]
fn booklet_fills_to_a_multiple_of_four() {
    let out = booklet(&make_pdf(5, None)).unwrap();
    assert_eq!(page_count(&out).unwrap(), 4);
    assert_eq!(sizes(&out)[0], (1224.0, 792.0));
}

#[test]
fn pages_get_a_new_size_halves_or_one_long_page() {
    let pdf = make_pdf(3, None);
    let a4 = resize_pages(&pdf, &[1], 595.0, 842.0).unwrap();
    assert_eq!(
        sizes(&a4),
        vec![(612.0, 792.0), (595.0, 842.0), (612.0, 792.0)]
    );
    let halves = halve_pages(&pdf, &[0], true).unwrap();
    assert_eq!(page_count(&halves).unwrap(), 4);
    assert_eq!(sizes(&halves)[0], (306.0, 792.0));
    let joined = join_pages(&pdf).unwrap();
    assert_eq!(sizes(&joined), vec![(612.0, 2376.0)]);
}

#[test]
fn crop_shrinks_the_crop_box() {
    let out = crop_pages(&make_pdf(1, None), &[0], [10.0, 20.0, 30.0, 40.0]).unwrap();
    assert_eq!(first_box(&out, b"CropBox"), vec![10.0, 40.0, 582.0, 772.0]);
    assert!(crop_pages(&out, &[0], [400.0, 0.0, 400.0, 0.0]).is_err());
}

#[test]
fn interleave_takes_pages_in_turn() {
    let a = make_pdf(2, None);
    let b = make_pdf(3, None);
    let out = interleave(&a, &b, true).unwrap();
    assert_eq!(
        page_texts(&out),
        vec!["Page 1", "Page 3", "Page 2", "Page 2", "Page 1"]
    );
}

#[test]
fn tints_keep_the_pages() {
    let pdf = make_pdf(2, None);
    let cream = Color {
        r: 1.0,
        g: 0.95,
        b: 0.8,
    };
    for tint in [Tint::Invert, Tint::Greyscale, Tint::Paper(cream)] {
        let out = tint_pages(&pdf, &[0, 1], tint).unwrap();
        assert_eq!(page_texts(&out), vec!["Page 1", "Page 2"]);
    }
}

fn highlighted() -> Vec<u8> {
    let highlight = Annotation::Highlight {
        rect: Rect {
            x0: 70.0,
            y0: 690.0,
            x1: 200.0,
            y1: 720.0,
        },
        color: Color {
            r: 1.0,
            g: 1.0,
            b: 0.0,
        },
    };
    add_annotation(&make_pdf(1, None), 0, &highlight).unwrap()
}

#[test]
fn comments_are_removed_or_flattened() {
    let pdf = highlighted();
    let removed = remove_comments(&pdf).unwrap();
    assert!(annotations(&removed, 0).unwrap().is_empty());
    assert!(remove_comments(&removed).is_err());

    let flat = flatten_annotations(&pdf).unwrap();
    assert!(annotations(&flat, 0).unwrap().is_empty());
    let doc = Document::load_mem(&flat).unwrap();
    let page = doc.page_iter().next().unwrap();
    let content = String::from_utf8_lossy(&doc.get_page_content(page)).into_owned();
    assert!(content.contains("GodPdfFlat0 Do"), "{content}");
}

#[test]
fn hidden_data_goes() {
    let pdf = add_attachment(&make_pdf(1, Some("Secret")), "a.txt", b"hi").unwrap();
    let out = remove_hidden_data(&pdf).unwrap();
    assert_eq!(read_metadata(&out).unwrap().title, None);
    assert!(attachments(&out).unwrap().is_empty());
}

#[test]
fn compress_shares_what_is_copied_twice() {
    let pdf = make_pdf(3, None);
    let merged = merge(&[&pdf, &pdf, &pdf]).unwrap();
    let small = compress(&merged).unwrap();
    assert!(small.len() < merged.len());
    assert_eq!(page_texts(&small), page_texts(&merged));
}

#[test]
fn attachments_are_added_replaced_and_removed() {
    let pdf = make_pdf(1, None);
    let one = add_attachment(&pdf, "notes.txt", b"first").unwrap();
    let two = add_attachment(&one, "data.csv", b"a,b").unwrap();
    let replaced = add_attachment(&two, "notes.txt", b"second").unwrap();
    let files = attachments(&replaced).unwrap();
    let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec!["data.csv", "notes.txt"]);
    assert_eq!(files[1].bytes, b"second");
    let left = remove_attachment(&replaced, "data.csv").unwrap();
    assert_eq!(attachments(&left).unwrap().len(), 1);
    assert!(remove_attachment(&left, "data.csv").is_err());
    let none = remove_attachment(&left, "notes.txt").unwrap();
    let doc = Document::load_mem(&none).unwrap();
    assert!(!matches!(
        doc.catalog().unwrap().get(b"Names"),
        Ok(Object::Dictionary(_))
    ));
}
