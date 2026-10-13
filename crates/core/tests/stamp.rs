mod common;

use common::{annot_count, edit, make_nested_pdf, make_pdf, page_dict, page_texts};
use godpdf_core::{
    Error, PageNumbers, Place, Watermark, extract_text, number_pages, page_count, remove_links,
    rgb_document, rotate_pages, text_document, watermark,
};
use lopdf::{Document, Object, dictionary};

/// Everything the page at `index` draws, its streams one after the other.
fn content(pdf: &[u8], index: usize) -> String {
    let doc = Document::load_mem(pdf).unwrap();
    let id = doc.page_iter().nth(index).unwrap();
    String::from_utf8_lossy(&doc.get_page_content(id)).into_owned()
}

#[test]
fn every_page_gets_its_number() {
    let numbers = PageNumbers {
        template: "{n} of {total}".to_owned(),
        ..PageNumbers::default()
    };
    let out = number_pages(&make_pdf(3, None), &[0, 1, 2], &numbers).unwrap();
    assert_eq!(
        page_texts(&out),
        ["Page 1 1 of 3", "Page 2 2 of 3", "Page 3 3 of 3"]
    );
}

#[test]
fn numbering_starts_where_asked_on_the_pages_asked() {
    let numbers = PageNumbers {
        first: 5,
        place: Place::TopRight,
        ..PageNumbers::default()
    };
    let out = number_pages(&make_pdf(3, None), &[1, 2], &numbers).unwrap();
    assert_eq!(page_texts(&out), ["Page 1", "Page 2 5", "Page 3 6"]);
}

#[test]
fn a_number_is_upright_on_a_turned_page() {
    let turned = rotate_pages(&make_pdf(1, None), &[0], 90).unwrap();
    let out = number_pages(&turned, &[0], &PageNumbers::default()).unwrap();
    assert!(content(&out, 0).contains("0 1 -1 0 612.000 0.000 cm"));
}

#[test]
fn pages_that_share_their_resources_are_numbered() {
    let out = number_pages(&make_nested_pdf(4), &[0, 3], &PageNumbers::default()).unwrap();
    assert_eq!(page_count(&out).unwrap(), 4);
    assert!(extract_text(&out, 3).unwrap().contains('2'));
    assert_eq!(extract_text(&out, 1).unwrap().trim(), "Page 2");
}

#[test]
fn a_watermark_is_drawn_over_the_page_with_its_opacity() {
    let mark = Watermark {
        text: "DRAFT".to_owned(),
        offset: (10.0, -20.0),
        ..Watermark::default()
    };
    let out = watermark(&make_pdf(2, None), &[0], &mark).unwrap();
    assert_eq!(page_texts(&out), ["Page 1 DRAFT", "Page 2"]);
    let drawn = content(&out, 0);
    // What the page drew is closed before the mark is drawn.
    assert!(drawn.trim_start().starts_with('q'));
    assert!(drawn.contains("/GodPdfAlpha gs"));
    // Centre of a 612x792 page, moved by the offset.
    assert!(drawn.contains("316.000 376.000 cm"));
    let resources = page_dict(&out, 0);
    let resources = resources.get(b"Resources").unwrap().as_dict().unwrap();
    assert!(resources.get(b"ExtGState").is_ok());
    // The font the page had is still there.
    let fonts = resources.get(b"Font").unwrap().as_dict().unwrap();
    assert!(fonts.has(b"F1") && fonts.has(b"GodPdfHelv"));
}

#[test]
fn what_helvetica_cannot_write_is_refused() {
    let mark = Watermark {
        text: "機密".to_owned(),
        ..Watermark::default()
    };
    let pdf = make_pdf(1, None);
    assert!(matches!(
        watermark(&pdf, &[0], &mark),
        Err(Error::InvalidArgument(_))
    ));
    let empty = Watermark::default();
    assert!(watermark(&pdf, &[0], &empty).is_err());
    assert!(number_pages(&pdf, &[], &PageNumbers::default()).is_err());
    assert!(number_pages(&pdf, &[4], &PageNumbers::default()).is_err());
}

#[test]
fn links_are_removed_and_comments_stay() {
    let pdf = edit(&make_pdf(2, None), |doc, pages| {
        let annot = |subtype: &str| {
            dictionary! {
                "Type" => "Annot",
                "Subtype" => subtype.to_owned(),
                "Rect" => vec![0.into(), 0.into(), 10.into(), 10.into()],
            }
        };
        let link = doc.add_object(annot("Link"));
        let note = doc.add_object(annot("Text"));
        let other = doc.add_object(annot("Link"));
        doc.get_dictionary_mut(pages[0])
            .unwrap()
            .set("Annots", vec![Object::Reference(link), note.into()]);
        let array = doc.add_object(vec![Object::Reference(other)]);
        doc.get_dictionary_mut(pages[1])
            .unwrap()
            .set("Annots", array);
    });
    let out = remove_links(&pdf).unwrap();
    assert_eq!((annot_count(&out, 0), annot_count(&out, 1)), (1, 0));
    assert!(matches!(remove_links(&out), Err(Error::InvalidArgument(_))));
}

#[test]
fn text_becomes_as_many_pages_as_it_needs() {
    let text: String = (1..=120).map(|n| format!("line {n}\r\n")).collect();
    let out = text_document(&text).unwrap();
    assert_eq!(page_count(&out).unwrap(), 3);
    assert!(extract_text(&out, 0).unwrap().contains("line 1"));
    assert!(extract_text(&out, 2).unwrap().contains("line 120"));
    assert_eq!(page_count(&text_document("").unwrap()).unwrap(), 1);
    assert!(text_document("日本語").is_err());
}

#[test]
fn a_picture_becomes_a_page_of_its_size() {
    let out = rgb_document(200, 100, &[0x80; 200 * 100 * 3]).unwrap();
    let page = page_dict(&out, 0);
    let size: Vec<f32> = page
        .get(b"MediaBox")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_float().unwrap())
        .collect();
    assert_eq!(size, [0.0, 0.0, 150.0, 75.0]);
    assert!(rgb_document(200, 100, &[0; 7]).is_err());
    // Larger than A4: brought down to it, keeping its shape.
    let large = rgb_document(2000, 1000, &vec![0; 2000 * 1000 * 3]).unwrap();
    let page = page_dict(&large, 0);
    let width = page.get(b"MediaBox").unwrap().as_array().unwrap()[2]
        .as_float()
        .unwrap();
    assert!((width - 842.0).abs() < 0.01);
}
