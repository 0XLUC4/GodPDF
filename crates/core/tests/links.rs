//! Links: read where they lead, add new ones.

mod common;

use common::make_pdf;
use godpdf_core::{
    Link, LinkTarget, OutlineItem, Rect, add_link, contents_page, is_web_uri, links, write_outline,
};
use lopdf::{Document, Object, dictionary};

const AREA: Rect = Rect {
    x0: 72.0,
    y0: 700.0,
    x1: 200.0,
    y1: 720.0,
};

/// `pdf` with `link` (a /Link dictionary) on its first page, and a named destination
/// "chapter" leading to page `named`.
fn with_raw_link(pdf: &[u8], link: lopdf::Dictionary, named: usize) -> Vec<u8> {
    let mut doc = Document::load_mem(pdf).unwrap();
    let pages: Vec<_> = doc.page_iter().collect();
    let dests = doc.add_object(dictionary! {
        "chapter" => vec![pages[named].into(), "Fit".into()],
    });
    let catalog = doc.catalog_mut().unwrap();
    catalog.set("Dests", dests);
    let mut link = link;
    link.set("Rect", vec![10.into(), 10.into(), 50.into(), 30.into()]);
    let id = doc.add_object(link);
    doc.get_dictionary_mut(pages[0])
        .unwrap()
        .set("Annots", vec![Object::from(id)]);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

#[test]
fn added_links_are_read_back() {
    let pdf = make_pdf(3, None);
    let to_page = add_link(&pdf, 0, AREA, &LinkTarget::Page(2)).unwrap();
    let uri = LinkTarget::Uri("https://example.com/a".into());
    let both = add_link(&to_page, 0, AREA, &uri).unwrap();
    let found = links(&both, 0).unwrap();
    assert_eq!(
        found,
        vec![
            Link {
                rect: AREA,
                target: LinkTarget::Page(2)
            },
            Link {
                rect: AREA,
                target: uri
            },
        ]
    );
    assert!(links(&both, 1).unwrap().is_empty());
    assert!(links(&both, 3).is_err());
}

#[test]
fn bad_links_are_refused() {
    let pdf = make_pdf(2, None);
    assert!(add_link(&pdf, 0, AREA, &LinkTarget::Page(2)).is_err());
    assert!(add_link(&pdf, 2, AREA, &LinkTarget::Page(0)).is_err());
    let script = LinkTarget::Uri("javascript:alert(1)".into());
    assert!(add_link(&pdf, 0, AREA, &script).is_err());
    let tiny = Rect {
        x1: AREA.x0 + 0.5,
        ..AREA
    };
    assert!(add_link(&pdf, 0, tiny, &LinkTarget::Page(1)).is_err());
}

#[test]
fn goto_actions_and_named_destinations_lead_to_their_page() {
    let pdf = make_pdf(3, None);
    let named = with_raw_link(
        &pdf,
        dictionary! { "Type" => "Annot", "Subtype" => "Link", "Dest" => "chapter" },
        2,
    );
    assert_eq!(links(&named, 0).unwrap()[0].target, LinkTarget::Page(2));
    let action = with_raw_link(
        &pdf,
        dictionary! {
            "Subtype" => "Link",
            "A" => dictionary! { "S" => "GoTo", "D" => Object::string_literal("chapter") },
        },
        1,
    );
    assert_eq!(links(&action, 0).unwrap()[0].target, LinkTarget::Page(1));
}

#[test]
fn only_web_and_mail_addresses_are_kept() {
    let pdf = make_pdf(1, None);
    let uri = |uri: &str| {
        let link = dictionary! {
            "Subtype" => "Link",
            "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal(uri) },
        };
        links(&with_raw_link(&pdf, link, 0), 0).unwrap()
    };
    assert_eq!(
        uri("mailto:someone@example.com")[0].target,
        LinkTarget::Uri("mailto:someone@example.com".into())
    );
    assert!(uri("file:///etc/passwd").is_empty());
    assert!(uri("javascript:alert(1)").is_empty());
    assert!(is_web_uri("HTTP://example.com") && !is_web_uri("https://"));
}

#[test]
fn contents_page_lines_are_links_to_their_pages() {
    let marks = [("One", 0), ("Two", 2)].map(|(title, page)| OutlineItem {
        title: title.to_owned(),
        page: Some(page),
        children: Vec::new(),
    });
    let pdf = write_outline(&make_pdf(3, None), &marks).unwrap();
    let out = contents_page(&pdf, "Contents").unwrap();
    let targets: Vec<LinkTarget> = links(&out, 0)
        .unwrap()
        .into_iter()
        .map(|link| link.target)
        .collect();
    // The contents page comes first: the pages moved one later.
    assert_eq!(targets, [LinkTarget::Page(1), LinkTarget::Page(3)]);
}
