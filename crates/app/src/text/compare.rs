//! Two documents compared (ported from the egui `src/compare.rs`): the lines of text each
//! page lost or gained, and the pages that look different though their text is the same.

use std::sync::Arc;

use crate::doc::guarded;

/// Side of the small renders compared pixel by pixel.
const THUMB: u32 = 160;
/// Mean difference of a channel (0 to 255) above which two renders differ.
const VISIBLE: f64 = 1.5;

pub struct PageReport {
    pub page: usize,
    pub removed: Vec<String>,
    pub added: Vec<String>,
    /// The pages are drawn differently (pictures, layout, colours).
    pub looks_different: bool,
    /// Only one of the documents has this page.
    pub missing: bool,
}

impl PageReport {
    /// The line under the page title, if any.
    pub fn note(&self) -> Option<&'static str> {
        if self.missing {
            Some("Only one of the documents has this page")
        } else if self.looks_different && self.removed.is_empty() && self.added.is_empty() {
            Some("Same text, drawn differently")
        } else {
            None
        }
    }
}

pub struct CompareView {
    pub title: String,
    pub pages: Vec<PageReport>,
}

/// Compare `first` with `second`. Slow on big files: run it in the background.
pub fn run(first: Vec<u8>, second: Vec<u8>, title: String) -> Result<CompareView, String> {
    let diffs =
        guarded(|| godpdf_core::compare_text(&first, &second)).map_err(|err| err.to_string())?;
    let open = |bytes: Vec<u8>| {
        godpdf_render::Document::open(Arc::new(bytes), None).map_err(|err| err.to_string())
    };
    let (a, b) = (open(first)?, open(second)?);
    let (count_a, count_b) = (a.page_count(), b.page_count());
    let mut pages = Vec::new();
    for page in 0..count_a.max(count_b) {
        let text = diffs.iter().find(|diff| diff.page == page);
        let missing = page >= count_a || page >= count_b;
        let looks_different = !missing && drawn_differently(&a, &b, page);
        if text.is_none() && !looks_different && !missing {
            continue;
        }
        pages.push(PageReport {
            page,
            removed: text.map(|diff| diff.removed.clone()).unwrap_or_default(),
            added: text.map(|diff| diff.added.clone()).unwrap_or_default(),
            looks_different,
            missing,
        });
    }
    Ok(CompareView { title, pages })
}

fn drawn_differently(
    a: &godpdf_render::Document,
    b: &godpdf_render::Document,
    page: usize,
) -> bool {
    let (Ok(left), Ok(right)) = (a.thumbnail(page, THUMB), b.thumbnail(page, THUMB)) else {
        return true;
    };
    if (left.width, left.height) != (right.width, right.height) {
        return true;
    }
    differs(&left.rgba, &right.rgba)
}

/// The mean channel difference of two pictures of the same size is visible.
fn differs(left: &[u8], right: &[u8]) -> bool {
    let total: u64 = left
        .iter()
        .zip(right)
        .map(|(x, y)| u64::from(x.abs_diff(*y)))
        .sum();
    total as f64 / left.len().max(1) as f64 > VISIBLE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_visible_change_counts() {
        let white = vec![255u8; 400];
        let mut speck = white.clone();
        speck[0] = 0;
        assert!(!differs(&white, &white));
        assert!(!differs(&white, &speck));
        assert!(differs(&white, &vec![0u8; 400]));
    }

    #[test]
    fn notes_say_why_a_page_is_listed() {
        let report = |missing, looks_different, added: &[&str]| PageReport {
            page: 0,
            removed: Vec::new(),
            added: added.iter().map(|line| line.to_string()).collect(),
            looks_different,
            missing,
        };
        assert_eq!(
            report(true, false, &[]).note(),
            Some("Only one of the documents has this page")
        );
        assert_eq!(
            report(false, true, &[]).note(),
            Some("Same text, drawn differently")
        );
        assert_eq!(report(false, true, &["x"]).note(), None);
    }
}
