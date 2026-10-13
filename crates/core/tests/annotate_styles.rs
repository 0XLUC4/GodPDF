mod common;

use common::make_pdf;
use godpdf_core::{
    Annotation, CharStyle, Color, Font, Rect, Script, add_annotation, annotations, wrap_styled,
};

fn styled(text: &str, styles: Vec<CharStyle>, font: Font) -> Annotation {
    Annotation::FreeText {
        rect: Rect {
            x0: 100.0,
            y0: 600.0,
            x1: 300.0,
            y1: 700.0,
        },
        text: text.into(),
        font_size: 12.0,
        color: Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
        },
        rotation: 0,
        font,
        styles,
    }
}

#[test]
fn styles_and_font_survive_a_round_trip() {
    let bold = CharStyle {
        bold: true,
        ..CharStyle::default()
    };
    let sup = CharStyle {
        script: Script::Super,
        italic: true,
        underline: true,
        ..CharStyle::default()
    };
    let text = "H2O <&>\nx2";
    let mut styles = vec![CharStyle::default(); text.chars().count()];
    styles[0] = bold;
    styles[1] = CharStyle {
        script: Script::Sub,
        ..CharStyle::default()
    };
    styles[9] = sup;
    let annotation = styled(text, styles.clone(), Font::Times);
    let pdf = add_annotation(&make_pdf(1, None), 0, &annotation).unwrap();
    let placed = &annotations(&pdf, 0).unwrap()[0];
    let Some(Annotation::FreeText {
        text: got_text,
        font,
        styles: got_styles,
        ..
    }) = &placed.annotation
    else {
        panic!("not a FreeText");
    };
    assert_eq!(got_text, text);
    assert_eq!(*font, Font::Times);
    assert_eq!(got_styles, &styles);
}

#[test]
fn plain_text_keeps_no_styles() {
    let pdf = add_annotation(
        &make_pdf(1, None),
        0,
        &styled("plain", Vec::new(), Font::Courier),
    )
    .unwrap();
    let Some(Annotation::FreeText { styles, font, .. }) =
        &annotations(&pdf, 0).unwrap()[0].annotation
    else {
        panic!("not a FreeText");
    };
    assert!(styles.is_empty());
    assert_eq!(*font, Font::Courier);
}

#[test]
fn wider_fonts_wrap_sooner() {
    let text = "aaaa aaaa";
    let bold = vec![
        CharStyle {
            bold: true,
            ..CharStyle::default()
        };
        9
    ];
    let width = 2.0 * 2.0 + godpdf_core::helvetica_line_width(text, 12.0) + 0.5;
    assert_eq!(
        wrap_styled(text, &[], Font::Helvetica, 12.0, width).len(),
        1
    );
    assert_eq!(
        wrap_styled(text, &bold, Font::Courier, 12.0, width).len(),
        2
    );
}
