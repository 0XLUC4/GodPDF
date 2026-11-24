mod common;

use common::make_pdf;
use godpdf_core::{Annotation, Color, MeasureKind, Rect, Scale, add_annotation, annotations};
use lopdf::{Document, Object};

const BLUE: Color = Color {
    r: 0.1,
    g: 0.2,
    b: 0.9,
};
const TRIANGLE: [(f32, f32); 3] = [(100.0, 100.0), (300.0, 100.0), (200.0, 250.0)];

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

fn name<'a>(dict: &'a lopdf::Dictionary, key: &[u8]) -> &'a [u8] {
    dict.get(key).unwrap().as_name().unwrap()
}

#[test]
fn polygon_and_polyline_round_trip() {
    let polygon = Annotation::Polygon {
        vertices: TRIANGLE.to_vec(),
        color: BLUE,
        width: 2.0,
        fill: Some(BLUE),
        cloud: false,
    };
    let (pdf, rect, back) = round_trip(&polygon);
    assert_eq!(back, polygon);
    assert_eq!(back.kind_name(), "Polygon");
    assert!(rect.x0 < 100.0 && rect.y0 < 100.0 && rect.x1 > 300.0 && rect.y1 > 250.0);
    let (dict, appearance) = written(&pdf);
    assert_eq!(name(&dict, b"Subtype"), b"Polygon");
    assert_eq!(dict.get(b"Vertices").unwrap().as_array().unwrap().len(), 6);
    assert!(appearance.contains(" h B"));

    let polyline = Annotation::PolyLine {
        vertices: TRIANGLE.to_vec(),
        color: BLUE,
        width: 1.0,
    };
    let (pdf, _, back) = round_trip(&polyline);
    assert_eq!(back, polyline);
    let (dict, appearance) = written(&pdf);
    assert_eq!(name(&dict, b"Subtype"), b"PolyLine");
    assert!(appearance.ends_with(" S") && !appearance.contains(" h"));
}

#[test]
fn cloud_is_a_polygon_with_a_cloudy_border() {
    let cloud = Annotation::Polygon {
        vertices: TRIANGLE.to_vec(),
        color: BLUE,
        width: 1.0,
        fill: None,
        cloud: true,
    };
    let (pdf, rect, back) = round_trip(&cloud);
    assert_eq!(back, cloud);
    assert_eq!(back.kind_name(), "Cloud");
    // The scallops reach out of the triangle.
    assert!(rect.y0 < 95.0);
    let (dict, appearance) = written(&pdf);
    let effect = dict.get(b"BE").unwrap().as_dict().unwrap();
    assert_eq!(name(effect, b"S"), b"C");
    assert!(appearance.matches(" c").count() > 10);
}

#[test]
fn measurements_round_trip_with_their_scale_and_value() {
    let scale = Scale::parse("1 cm = 2 m").unwrap();
    let ten_cm = 720.0 / 2.54;
    let cases = [
        (
            MeasureKind::Distance,
            vec![(100.0, 100.0), (100.0 + ten_cm, 100.0)],
            "Line",
            "LineDimension",
            "20.00 m",
        ),
        (
            MeasureKind::Perimeter,
            vec![
                (100.0, 100.0),
                (100.0 + ten_cm, 100.0),
                (100.0 + ten_cm, 400.0),
            ],
            "PolyLine",
            "PolyLineDimension",
            "41.17 m",
        ),
        (
            MeasureKind::Area,
            vec![
                (100.0, 100.0),
                (100.0 + ten_cm, 100.0),
                (100.0 + ten_cm, 100.0 + ten_cm),
                (100.0, 100.0 + ten_cm),
            ],
            "Polygon",
            "PolygonDimension",
            "400.00 m²",
        ),
    ];
    for (kind, vertices, subtype, intent, label) in cases {
        let measure = Annotation::Measure {
            kind,
            vertices,
            color: BLUE,
            width: 1.0,
            scale,
        };
        let (pdf, _, back) = round_trip(&measure);
        assert_eq!(back, measure, "{intent}");
        let (dict, appearance) = written(&pdf);
        assert_eq!(name(&dict, b"Subtype"), subtype.as_bytes());
        assert_eq!(name(&dict, b"IT"), intent.as_bytes());
        let contents = lopdf::decode_text_string(dict.get(b"Contents").unwrap()).unwrap();
        assert_eq!(contents, label);
        let ratio = dict.get(b"Measure").unwrap().as_dict().unwrap().get(b"R");
        assert!(matches!(ratio, Ok(Object::String(..))));
        // The label is drawn, in WinAnsi hex.
        assert!(appearance.contains(" Tj ET"));
    }
}

#[test]
fn moved_measurement_keeps_its_value() {
    let measure = Annotation::Measure {
        kind: MeasureKind::Area,
        vertices: TRIANGLE.to_vec(),
        color: BLUE,
        width: 1.0,
        scale: Scale::default(),
    };
    let moved = measure.translated(40.0, -30.0);
    let (pdf, _, back) = round_trip(&moved);
    assert_eq!(back, moved);
    let (dict, _) = written(&pdf);
    let contents = lopdf::decode_text_string(dict.get(b"Contents").unwrap()).unwrap();
    // 15 000 square points, in square inches.
    assert_eq!(contents, format!("{:.2} in²", 15_000.0 / 72.0 / 72.0));
}

#[test]
fn bad_paths_are_refused() {
    let polygon = Annotation::Polygon {
        vertices: TRIANGLE[..2].to_vec(),
        color: BLUE,
        width: 1.0,
        fill: None,
        cloud: false,
    };
    assert!(add_annotation(&make_pdf(1, None), 0, &polygon).is_err());
    let distance = Annotation::Measure {
        kind: MeasureKind::Distance,
        vertices: TRIANGLE.to_vec(),
        color: BLUE,
        width: 1.0,
        scale: Scale::default(),
    };
    assert!(add_annotation(&make_pdf(1, None), 0, &distance).is_err());
}
