mod common;

use common::{edit, make_pdf, make_pdf_with_outline};
use godpdf_core::{
    Error, Metadata, PageGeometry, SearchHit, extract_all_text, extract_text, outline,
    page_geometry, read_metadata, rotate_pages, search, write_metadata,
};

#[test]
fn metadata_reads_fixture_title() {
    let meta = read_metadata(&make_pdf(1, Some("Hello"))).unwrap();
    assert_eq!(meta.title.as_deref(), Some("Hello"));
    assert_eq!(meta.author, None);
    assert_eq!(
        read_metadata(&make_pdf(1, None)).unwrap(),
        Metadata::default()
    );
}

#[test]
fn metadata_round_trip_with_unicode() {
    let meta = Metadata {
        title: Some("Évangile 日本".into()),
        author: Some("Luca (test) \\ x".into()),
        subject: None,
        keywords: Some("pdf, rust".into()),
        creator: Some("Café".into()),
        producer: Some("GodPDF".into()),
    };
    let out = write_metadata(&make_pdf(2, Some("Old")), &meta).unwrap();
    assert_eq!(read_metadata(&out).unwrap(), meta);
}

#[test]
fn metadata_none_removes_field() {
    let out = write_metadata(&make_pdf(1, Some("Old")), &Metadata::default()).unwrap();
    assert_eq!(read_metadata(&out).unwrap().title, None);
}

#[test]
fn outline_empty_without_bookmarks() {
    assert!(outline(&make_pdf(2, None)).unwrap().is_empty());
}

#[test]
fn outline_resolves_page() {
    let items = outline(&make_pdf_with_outline(3, 2)).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title, "Chapter");
    assert_eq!(items[0].page, Some(2));
}

#[test]
fn extract_text_per_page() {
    let pdf = make_pdf(3, None);
    assert!(extract_text(&pdf, 1).unwrap().contains("Page 2"));
    assert!(matches!(
        extract_text(&pdf, 3),
        Err(Error::PageOutOfRange { page: 3, count: 3 })
    ));
}

#[test]
fn search_is_case_insensitive_and_counts() {
    let pdf = make_pdf(12, None);
    assert_eq!(
        search(&pdf, "PAGE 1").unwrap(),
        vec![
            SearchHit { page: 0, count: 1 },
            SearchHit { page: 9, count: 1 },
            SearchHit { page: 10, count: 1 },
            SearchHit { page: 11, count: 1 },
        ]
    );
    assert_eq!(
        search(&pdf, "page   3").unwrap(),
        vec![SearchHit { page: 2, count: 1 }]
    );
    assert_eq!(search(&pdf, "a").unwrap().len(), 12);
    assert!(search(&pdf, "nothing here").unwrap().is_empty());
    assert!(search(&pdf, "  ").unwrap().is_empty());
}

#[test]
fn page_geometry_reports_rotation() {
    let pdf = make_pdf(2, None);
    let rotated = rotate_pages(&pdf, &[1], -90).unwrap();
    assert_eq!(page_geometry(&rotated, 0).unwrap().rotation, 0);
    assert_eq!(
        page_geometry(&rotated, 1).unwrap(),
        PageGeometry {
            rotation: 270,
            origin: (0.0, 0.0)
        }
    );
    assert!(matches!(
        page_geometry(&pdf, 2),
        Err(Error::PageOutOfRange { page: 2, count: 2 })
    ));
}

#[test]
fn page_geometry_origin_is_crop_intersected_with_media() {
    let pdf = edit(&make_pdf(2, None), |doc, pages| {
        // Unnormalized CropBox reaching outside the 612x792 MediaBox on the left and bottom.
        let crop = vec![700.into(), 900.into(), (-50).into(), 100.into()];
        doc.get_dictionary_mut(pages[0])
            .unwrap()
            .set("CropBox", crop);
        let inside = vec![10.into(), 20.into(), 300.into(), 400.into()];
        doc.get_dictionary_mut(pages[1])
            .unwrap()
            .set("CropBox", inside);
    });
    assert_eq!(page_geometry(&pdf, 0).unwrap().origin, (0.0, 100.0));
    assert_eq!(page_geometry(&pdf, 1).unwrap().origin, (10.0, 20.0));
}

#[test]
fn extract_all_text_returns_every_page() {
    let all: Vec<String> = extract_all_text(&make_pdf(3, None))
        .unwrap()
        .iter()
        .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect();
    assert_eq!(all, ["Page 1", "Page 2", "Page 3"]);
    assert!(matches!(extract_all_text(b"nope"), Err(Error::Parse(_))));
}
