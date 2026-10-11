//! Fixture PDFs built in-memory with lopdf.
#![allow(dead_code)]

use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, ObjectId, Stream, dictionary};

/// `pages`-page document; page i shows "Page i+1" in Helvetica. Optional Info /Title.
pub fn make_pdf(pages: usize, title: Option<&str>) -> Vec<u8> {
    let (mut doc, _) = build(pages, title, None);
    save(&mut doc)
}

/// Like [`make_pdf`] with a one-item outline "Chapter" pointing at `target` (0-based).
pub fn make_pdf_with_outline(pages: usize, target: usize) -> Vec<u8> {
    let (mut doc, _) = build(pages, None, Some(target));
    save(&mut doc)
}

/// Pages split across two intermediate /Pages nodes that carry MediaBox and Resources.
pub fn make_nested_pdf(pages: usize) -> Vec<u8> {
    let mut doc = Document::with_version("1.7");
    let root = doc.new_object_id();
    let font = doc.add_object(helvetica());
    let resources = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font } });
    let half = pages / 2;
    let mut branches = Vec::new();
    for range in [0..half, half..pages] {
        let branch = doc.new_object_id();
        let kids: Vec<Object> = range
            .clone()
            .map(|i| page(&mut doc, branch, i, false).into())
            .collect();
        let count = kids.len() as i64;
        doc.objects.insert(
            branch,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Parent" => root, "Kids" => kids, "Count" => count,
                "MediaBox" => vec![0.into(), 0.into(), 300.into(), 400.into()],
                "Resources" => resources,
                "Rotate" => 90,
            }),
        );
        branches.push(Object::Reference(branch));
    }
    doc.objects.insert(
        root,
        Object::Dictionary(
            dictionary! { "Type" => "Pages", "Kids" => branches, "Count" => pages as i64 },
        ),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => root });
    doc.trailer.set("Root", catalog);
    save(&mut doc)
}

fn build(
    pages: usize,
    title: Option<&str>,
    outline_target: Option<usize>,
) -> (Document, Vec<ObjectId>) {
    let mut doc = Document::with_version("1.7");
    let root = doc.new_object_id();
    let ids: Vec<ObjectId> = (0..pages).map(|i| page(&mut doc, root, i, true)).collect();
    let kids: Vec<Object> = ids.iter().map(|&id| id.into()).collect();
    doc.objects.insert(
        root,
        Object::Dictionary(
            dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => pages as i64 },
        ),
    );
    let mut catalog = dictionary! { "Type" => "Catalog", "Pages" => root };
    if let Some(target) = outline_target {
        let outlines = doc.new_object_id();
        let item = doc.add_object(dictionary! {
            "Title" => Object::string_literal("Chapter"),
            "Parent" => outlines,
            "Dest" => vec![ids[target].into(), "Fit".into()],
        });
        doc.objects.insert(
            outlines,
            Object::Dictionary(
                dictionary! { "Type" => "Outlines", "First" => item, "Last" => item, "Count" => 1 },
            ),
        );
        catalog.set("Outlines", outlines);
    }
    let catalog = doc.add_object(catalog);
    doc.trailer.set("Root", catalog);
    if let Some(title) = title {
        let info = doc.add_object(dictionary! { "Title" => Object::string_literal(title) });
        doc.trailer.set("Info", info);
    }
    (doc, ids)
}

fn page(doc: &mut Document, parent: ObjectId, index: usize, own_box: bool) -> ObjectId {
    let content = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 24.into()]),
            Operation::new("Td", vec![72.into(), 700.into()]),
            Operation::new(
                "Tj",
                vec![Object::string_literal(format!("Page {}", index + 1))],
            ),
            Operation::new("ET", vec![]),
        ],
    };
    let contents = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let mut dict = dictionary! { "Type" => "Page", "Parent" => parent, "Contents" => contents };
    if own_box {
        let font = doc.add_object(helvetica());
        dict.set("MediaBox", vec![0.into(), 0.into(), 612.into(), 792.into()]);
        dict.set(
            "Resources",
            dictionary! { "Font" => dictionary! { "F1" => font } },
        );
    }
    doc.add_object(dict)
}

fn helvetica() -> lopdf::Dictionary {
    dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding" }
}

fn save(doc: &mut Document) -> Vec<u8> {
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

/// Normalized text of every page, e.g. ["Page 1", "Page 2"].
pub fn page_texts(pdf: &[u8]) -> Vec<String> {
    let count = godpdf_core::page_count(pdf).unwrap();
    (0..count)
        .map(|i| {
            godpdf_core::extract_text(pdf, i)
                .unwrap()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

pub fn texts(labels: &[usize]) -> Vec<String> {
    labels.iter().map(|n| format!("Page {n}")).collect()
}

/// The page dictionary at `index` of a saved PDF.
pub fn page_dict(pdf: &[u8], index: usize) -> lopdf::Dictionary {
    let doc = Document::load_mem(pdf).unwrap();
    let id = doc.page_iter().nth(index).unwrap();
    doc.get_dictionary(id).unwrap().clone()
}

/// Number of annotations on page `index`.
pub fn annot_count(pdf: &[u8], index: usize) -> usize {
    let doc = Document::load_mem(pdf).unwrap();
    let id = doc.page_iter().nth(index).unwrap();
    doc.get_page_annotations(id).map(|a| a.len()).unwrap_or(0)
}

/// Load `pdf`, let `change` mutate it (gets the page ids in order), save.
pub fn edit(pdf: &[u8], change: impl FnOnce(&mut Document, &[ObjectId])) -> Vec<u8> {
    let mut doc = Document::load_mem(pdf).unwrap();
    let pages: Vec<ObjectId> = doc.page_iter().collect();
    change(&mut doc, &pages);
    save(&mut doc)
}
