//! `Document::text_chars` tests: fixtures are built in-memory with lopdf.

use std::sync::Arc;

use godpdf_render::{Document, Error, TextChar};
use lopdf::{Document as LoDoc, Object, Stream, dictionary};

const HELLO: &str = "BT /F1 12 Tf 20 50 Td (Hello) Tj ET";

/// One 200x100 page per `(content stream, /Rotate)` entry, with Helvetica as /F1.
fn open(pages: &[(&str, i64)]) -> Document {
    let mut doc = LoDoc::with_version("1.7");
    let pages_id = doc.new_object_id();
    let font = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let kids: Vec<Object> = pages
        .iter()
        .map(|(content, rotate)| {
            let contents = doc.add_object(Stream::new(dictionary! {}, content.as_bytes().to_vec()));
            doc.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 200.into(), 100.into()],
                "Rotate" => *rotate,
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
                "Contents" => contents,
            })
            .into()
        })
        .collect();
    let count = kids.len() as i64;
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).expect("save fixture");
    Document::open(Arc::new(bytes), None).expect("open fixture")
}

fn text(chars: &[TextChar]) -> String {
    chars.iter().map(|c| c.text.as_str()).collect()
}

#[test]
fn hello_geometry() {
    let chars = open(&[(HELLO, 0)]).text_chars(0).unwrap();
    assert_eq!(text(&chars), "Hello");
    assert_eq!(chars.len(), 5);
    assert!(chars.windows(2).all(|w| w[0].x0 < w[1].x0), "{chars:?}");
    // Cells tile the line: each one ends where the next begins.
    assert!(
        chars.windows(2).all(|w| (w[0].x1 - w[1].x0).abs() < 0.01),
        "{chars:?}"
    );
    assert!((chars[0].x0 - 20.0).abs() < 0.5, "{chars:?}");
    // Helvetica widths H=722 e=556 l=222 l=222 o=556 -> 2278/1000 * 12pt.
    let width = chars[4].x1 - chars[0].x0;
    assert!((width - 27.336).abs() < 1.0, "width {width}");
    for c in &chars {
        // Baseline is at displayed y = 100 - 50.
        assert!(c.y0 < 50.0 && 50.0 < c.y1, "{c:?}");
        assert!((c.y0 - (50.0 - 0.8 * 12.0)).abs() < 2.0, "{c:?}");
        assert!((c.y1 - (50.0 + 0.2 * 12.0)).abs() < 2.0, "{c:?}");
    }
}

#[test]
fn rotated_page_stays_in_bounds() {
    let doc = open(&[(HELLO, 90)]);
    let chars = doc.text_chars(0).unwrap();
    let size = doc.page_size(0).unwrap();
    assert_eq!((size.width, size.height), (100.0, 200.0));
    assert_eq!(text(&chars), "Hello");
    for c in &chars {
        assert!(c.x0 >= 0.0 && c.x1 <= size.width, "{c:?}");
        assert!(c.y0 >= 0.0 && c.y1 <= size.height, "{c:?}");
        assert!(c.x0 < c.x1 && c.y0 < c.y1, "{c:?}");
    }
    // Text now runs top to bottom.
    assert!(chars.windows(2).all(|w| w[0].y0 < w[1].y0), "{chars:?}");
}

#[test]
fn two_lines_give_two_bands() {
    let content = "BT /F1 12 Tf 20 70 Td (Ab) Tj 0 -30 Td (Cd) Tj ET";
    let chars = open(&[(content, 0)]).text_chars(0).unwrap();
    assert_eq!(text(&chars), "AbCd");
    assert_eq!(chars[0].y0, chars[1].y0);
    assert_eq!(chars[2].y0, chars[3].y0);
    // The second line starts below the bottom of the first.
    assert!(chars[2].y0 > chars[0].y1, "{chars:?}");
    assert!((chars[2].y0 - chars[0].y0 - 30.0).abs() < 0.01, "{chars:?}");
}

#[test]
fn spaces_kept_and_fill_stroke_not_duplicated() {
    // Render mode 2 = fill then stroke: hayro draws the run twice.
    let content = "BT /F1 12 Tf 2 Tr 20 50 Td (a b) Tj ET";
    let chars = open(&[(content, 0)]).text_chars(0).unwrap();
    assert_eq!(text(&chars), "a b");
}

#[test]
fn invisible_text_is_included() {
    let content = "BT /F1 12 Tf 3 Tr 20 50 Td (ocr) Tj ET";
    let chars = open(&[(content, 0)]).text_chars(0).unwrap();
    assert_eq!(text(&chars), "ocr");
}

#[test]
fn page_without_text_is_empty() {
    let doc = open(&[("0 0 0 rg 10 10 50 50 re f", 0), ("", 0)]);
    assert!(doc.text_chars(0).unwrap().is_empty());
    assert!(doc.text_chars(1).unwrap().is_empty());
}

#[test]
fn out_of_range_page() {
    assert!(matches!(
        open(&[(HELLO, 0)]).text_chars(1),
        Err(Error::PageOutOfRange { page: 1, count: 1 })
    ));
}

/// Boxes must line up with the rasterizer: all ink falls inside them.
#[test]
fn boxes_match_rendering() {
    const SCALE: f32 = 2.0;
    for rotate in [0, 90, 180, 270] {
        let doc = open(&[(HELLO, rotate)]);
        let chars = doc.text_chars(0).unwrap();
        let bmp = doc.render(0, SCALE).unwrap();
        let (mut inside, mut outside) = (0, 0);
        for y in 0..bmp.height {
            for x in 0..bmp.width {
                let i = ((y * bmp.width + x) * 4) as usize;
                if bmp.rgba[i] > 128 {
                    continue;
                }
                let (px, py) = ((x as f32 + 0.5) / SCALE, (y as f32 + 0.5) / SCALE);
                let hit = chars
                    .iter()
                    .any(|c| c.x0 <= px && px <= c.x1 && c.y0 <= py && py <= c.y1);
                if hit { inside += 1 } else { outside += 1 }
            }
        }
        assert!(
            inside > 50,
            "rotate {rotate}: only {inside} dark pixels in boxes"
        );
        assert_eq!(outside, 0, "rotate {rotate}: dark pixels outside boxes");
        // Every glyph cell contains ink of its own.
        for c in &chars {
            let dark = (0..bmp.height).any(|y| {
                (0..bmp.width).any(|x| {
                    let (px, py) = ((x as f32 + 0.5) / SCALE, (y as f32 + 0.5) / SCALE);
                    let i = ((y * bmp.width + x) * 4) as usize;
                    bmp.rgba[i] <= 128 && c.x0 <= px && px <= c.x1 && c.y0 <= py && py <= c.y1
                })
            });
            assert!(dark, "rotate {rotate}: no ink in {c:?}");
        }
    }
}
