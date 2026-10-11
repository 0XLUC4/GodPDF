use lopdf::Document;

use crate::Result;
use crate::pdf::{check_page, load};

/// A page containing the query and how many times.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub page: usize,
    pub count: usize,
}

/// Plain text of one page (best effort, may be empty for scanned pages).
pub fn extract_text(pdf: &[u8], page: usize) -> Result<String> {
    let doc = load(pdf)?;
    check_page(page, doc.page_iter().count())?;
    Ok(page_text(&doc, page))
}

/// Plain text of every page, parsing the document once.
pub fn extract_all_text(pdf: &[u8]) -> Result<Vec<String>> {
    let doc = load(pdf)?;
    let count = doc.page_iter().count();
    Ok((0..count).map(|page| page_text(&doc, page)).collect())
}

/// Case-insensitive search across all pages; only pages with hits are returned, ascending.
///
/// Whitespace runs count as one space on both sides; occurrences are non-overlapping.
/// An empty (or blank) query yields no hits.
pub fn search(pdf: &[u8], query: &str) -> Result<Vec<SearchHit>> {
    let needle = normalize(query);
    if needle.is_empty() {
        return Ok(Vec::new());
    }
    let doc = load(pdf)?;
    let count = doc.page_iter().count();
    let hits = (0..count)
        .map(|page| SearchHit {
            page,
            count: normalize(&page_text(&doc, page)).matches(&needle).count(),
        })
        .filter(|hit| hit.count > 0)
        .collect();
    Ok(hits)
}

/// Text chunks lopdf could decode; undecodable parts are skipped.
fn page_text(doc: &Document, page: usize) -> String {
    let Ok(number) = u32::try_from(page + 1) else {
        return String::new();
    };
    doc.extract_text_chunks(&[number])
        .into_iter()
        .filter_map(|chunk| chunk.ok())
        .collect()
}

fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
