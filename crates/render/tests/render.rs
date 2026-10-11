//! Integration tests: fixtures are built in-memory with lopdf.

use std::sync::Arc;

use godpdf_render::{Bitmap, Document, Error, PageSize};
use lopdf::{
    Document as LoDoc, EncryptionState, EncryptionVersion, Object, Permissions, Stream, dictionary,
};

/// Page 0: 100x100 with a black rect at (10,10)-(60,60). Page 1: 200x100, /Rotate 90.
fn build_doc() -> LoDoc {
    let mut doc = LoDoc::with_version("1.7");
    let pages_id = doc.new_object_id();
    let rect = doc.add_object(Stream::new(
        dictionary! {},
        b"0 0 0 rg 10 10 50 50 re f".to_vec(),
    ));
    let empty = doc.add_object(Stream::new(dictionary! {}, Vec::new()));
    let page1 = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
        "Contents" => rect,
    });
    let page2 = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 200.into(), 100.into()],
        "Rotate" => 90,
        "Contents" => empty,
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page1.into(), page2.into()],
            "Count" => 2,
        }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    doc
}

fn save(mut doc: LoDoc) -> Arc<Vec<u8>> {
    let mut out = Vec::new();
    doc.save_to(&mut out).expect("save fixture");
    Arc::new(out)
}

fn open() -> Document {
    Document::open(save(build_doc()), None).expect("open fixture")
}

fn pixel(bmp: &Bitmap, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * bmp.width + x) * 4) as usize;
    [
        bmp.rgba[i],
        bmp.rgba[i + 1],
        bmp.rgba[i + 2],
        bmp.rgba[i + 3],
    ]
}

#[test]
fn document_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Document>();
}

#[test]
fn page_count_and_sizes() {
    let doc = open();
    assert_eq!(doc.page_count(), 2);
    assert_eq!(
        doc.page_size(0).unwrap(),
        PageSize {
            width: 100.0,
            height: 100.0
        }
    );
    // /Rotate 90 swaps width and height.
    assert_eq!(
        doc.page_size(1).unwrap(),
        PageSize {
            width: 100.0,
            height: 200.0
        }
    );
}

#[test]
fn render_dims_and_pixels() {
    let doc = open();
    let scale = 1.5;
    let bmp = doc.render(0, scale).unwrap();
    assert_eq!((bmp.width, bmp.height), (150, 150));
    assert_eq!(bmp.rgba.len(), (bmp.width * bmp.height * 4) as usize);

    // Rect spans PDF y 10..60 -> image y (100-60)..(100-10) = 40..90, times 1.5.
    let inside = pixel(&bmp, (35.0 * scale) as u32, (65.0 * scale) as u32);
    assert!(
        inside[0] < 30 && inside[3] == 255,
        "inside rect not dark: {inside:?}"
    );
    let outside = pixel(&bmp, (80.0 * scale) as u32, (20.0 * scale) as u32);
    assert_eq!(outside, [255, 255, 255, 255]);

    let rotated = doc.render(1, 0.5).unwrap();
    assert_eq!((rotated.width, rotated.height), (50, 100));
    assert_eq!(rotated.rgba.len(), 50 * 100 * 4);
}

#[test]
fn scale_is_clamped() {
    let doc = open();
    let tiny = doc.render(0, 0.0).unwrap();
    assert_eq!((tiny.width, tiny.height), (5, 5));
    let huge = doc.render(0, 1000.0).unwrap();
    assert_eq!((huge.width, huge.height), (800, 800));
    let nan = doc.render(0, f32::NAN).unwrap();
    assert_eq!((nan.width, nan.height), (100, 100));
}

#[test]
fn thumbnail_longest_side() {
    let doc = open();
    for page in 0..2 {
        let bmp = doc.thumbnail(page, 64).unwrap();
        assert!(
            bmp.width.max(bmp.height).abs_diff(64) <= 1,
            "page {page}: {}x{}",
            bmp.width,
            bmp.height
        );
    }
    let rotated = doc.thumbnail(1, 64).unwrap();
    assert!(rotated.height > rotated.width);
}

#[test]
fn out_of_range_page() {
    let doc = open();
    assert!(matches!(
        doc.page_size(2),
        Err(Error::PageOutOfRange { page: 2, count: 2 })
    ));
    assert!(matches!(
        doc.render(5, 1.0),
        Err(Error::PageOutOfRange { page: 5, count: 2 })
    ));
    assert!(matches!(
        doc.thumbnail(9, 64),
        Err(Error::PageOutOfRange { .. })
    ));
}

#[test]
fn garbage_is_parse_error() {
    let res = Document::open(Arc::new(b"definitely not a pdf".to_vec()), None);
    assert!(matches!(res, Err(Error::Parse(_))));
}

#[test]
fn encrypted_requires_password() {
    let mut doc = build_doc();
    let id = Object::String(
        b"0123456789abcdef".to_vec(),
        lopdf::StringFormat::Hexadecimal,
    );
    doc.trailer.set("ID", vec![id.clone(), id]);
    let state = EncryptionState::try_from(EncryptionVersion::V2 {
        document: &doc,
        owner_password: "owner",
        user_password: "secret",
        key_length: 128,
        permissions: Permissions::all(),
    })
    .expect("encryption state");
    doc.encrypt(&state).expect("encrypt");
    let bytes = save(doc);

    assert!(matches!(
        Document::open(bytes.clone(), None),
        Err(Error::PasswordRequired)
    ));
    assert!(matches!(
        Document::open(bytes.clone(), Some("wrong")),
        Err(Error::PasswordRequired)
    ));
    let opened = Document::open(bytes, Some("secret")).expect("correct password");
    assert_eq!(opened.page_count(), 2);
    let bmp = opened.render(0, 1.0).unwrap();
    assert!(pixel(&bmp, 35, 65)[0] < 30);
}
