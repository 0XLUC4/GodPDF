//! Text that cannot be seen, laid over the words of a scanned page: the page can then be
//! searched and its words selected and copied.

use lopdf::{Document, ObjectId};

use crate::annotate::{num, win_ansi_hex};
use crate::helvetica::{line_width, win_ansi};
use crate::pdf::{load, save};
use crate::stamp::{View, helvetica, targets, wrap};
use crate::{Error, Result};

/// A word found on a page: its text and its box (left, top, right, bottom) in points of the
/// page as displayed, origin top left.
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub text: String,
    pub rect: [f32; 4],
}

/// Lay invisible `words` over each `(page, words)`. Characters Helvetica cannot write are
/// left out.
pub fn add_text_layer(pdf: &[u8], pages: &[(usize, Vec<Word>)]) -> Result<Vec<u8>> {
    if pages.iter().all(|(_, words)| words.is_empty()) {
        return Err(Error::InvalidArgument("no text was found".into()));
    }
    let mut doc = load(pdf)?;
    let indices: Vec<usize> = pages.iter().map(|(page, _)| *page).collect();
    let ids = targets(&mut doc, &indices)?;
    let font = doc.add_object(helvetica());
    for (id, (_, words)) in ids.into_iter().zip(pages) {
        let ops = layer_ops(&doc, id, words);
        if !ops.is_empty() {
            wrap(&mut doc, id, "", &ops, &[("Font", "GodPdfOcr", font)])?;
        }
    }
    save(&mut doc)
}

/// Text in render mode 3 (neither filled nor stroked), each word stretched over its box.
fn layer_ops(doc: &Document, page: ObjectId, words: &[Word]) -> String {
    let view = View::of(doc, page);
    let mut ops = format!("q {} cm BT 3 Tr\n", view.matrix());
    let mut any = false;
    for word in words {
        let text: String = word
            .text
            .chars()
            .filter(|&c| c == '?' || win_ansi(c) != b'?')
            .collect();
        let [left, top, right, bottom] = word.rect;
        let (width, height) = (right - left, bottom - top);
        if text.trim().is_empty() || width <= 0.5 || height <= 0.5 {
            continue;
        }
        let size = height * 0.9;
        let natural = line_width(&text, size).max(0.1);
        let stretch = (width / natural * 100.0).clamp(10.0, 1000.0);
        // Baseline a fifth of the box above its bottom edge, y measured upwards.
        let baseline = view.height - bottom + height * 0.2;
        ops += &format!(
            "/GodPdfOcr {} Tf {} Tz 1 0 0 1 {} {} Tm <{}> Tj\n",
            num(size),
            num(stretch),
            num(left),
            num(baseline),
            // The space keeps the words apart when the text is copied.
            win_ansi_hex(&format!("{text} "))
        );
        any = true;
    }
    if !any {
        return String::new();
    }
    ops + "ET Q\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_layer_is_refused() {
        let pdf = crate::blank_document(200.0, 200.0).unwrap();
        assert!(add_text_layer(&pdf, &[(0, Vec::new())]).is_err());
    }

    #[test]
    fn words_become_searchable_text() {
        let pdf = crate::blank_document(400.0, 300.0).unwrap();
        let words = vec![
            Word {
                text: "Hello".into(),
                rect: [20.0, 20.0, 80.0, 40.0],
            },
            Word {
                text: "scan".into(),
                rect: [90.0, 20.0, 140.0, 40.0],
            },
        ];
        let out = add_text_layer(&pdf, &[(0, words)]).unwrap();
        let text = crate::extract_text(&out, 0).unwrap();
        assert!(text.contains("Hello") && text.contains("scan"), "{text}");
        assert_eq!(crate::search(&out, "scan").unwrap().len(), 1);
    }
}
