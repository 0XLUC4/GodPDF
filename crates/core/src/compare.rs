//! The text of two documents compared page by page, line by line.

use crate::Result;
use crate::text::extract_all_text;

/// Lines past this count on a page are compared as one block, to keep the work small.
const MAX_LINES: usize = 3000;

/// What changed on one page, from the first document to the second.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageDiff {
    /// 0-based page index.
    pub page: usize,
    /// Lines of the first document that are not in the second.
    pub removed: Vec<String>,
    /// Lines of the second document that are not in the first.
    pub added: Vec<String>,
}

/// Pages whose text differs between `first` and `second`. A page only one of them has
/// shows all its lines as removed or added.
pub fn compare_text(first: &[u8], second: &[u8]) -> Result<Vec<PageDiff>> {
    let a = extract_all_text(first)?;
    let b = extract_all_text(second)?;
    let empty = String::new();
    Ok((0..a.len().max(b.len()))
        .filter_map(|page| {
            let old = lines(a.get(page).unwrap_or(&empty));
            let new = lines(b.get(page).unwrap_or(&empty));
            let (removed, added) = diff(&old, &new);
            (!removed.is_empty() || !added.is_empty()).then_some(PageDiff {
                page,
                removed,
                added,
            })
        })
        .collect())
}

/// Non-empty lines, spaces made single.
fn lines(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = text
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect();
    if lines.len() > MAX_LINES {
        let rest = lines.split_off(MAX_LINES - 1).join(" ");
        lines.push(rest);
    }
    lines
}

/// Lines only in `old` and lines only in `new`, after their longest common sequence.
fn diff(old: &[String], new: &[String]) -> (Vec<String>, Vec<String>) {
    let (n, m) = (old.len(), new.len());
    // common[i][j]: length of the longest common sequence of old[i..] and new[j..].
    let mut common = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            common[i][j] = if old[i] == new[j] {
                common[i + 1][j + 1] + 1
            } else {
                common[i + 1][j].max(common[i][j + 1])
            };
        }
    }
    let (mut removed, mut added) = (Vec::new(), Vec::new());
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if old[i] == new[j] {
            i += 1;
            j += 1;
        } else if common[i + 1][j] >= common[i][j + 1] {
            removed.push(old[i].clone());
            i += 1;
        } else {
            added.push(new[j].clone());
            j += 1;
        }
    }
    removed.extend(old[i..].iter().cloned());
    added.extend(new[j..].iter().cloned());
    (removed, added)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|l| (*l).to_owned()).collect()
    }

    #[test]
    fn changed_lines_are_found() {
        let old = owned(&["a", "b", "c", "d"]);
        let new = owned(&["a", "x", "c", "d", "e"]);
        assert_eq!(diff(&old, &new), (owned(&["b"]), owned(&["x", "e"])));
        assert_eq!(diff(&old, &old), (Vec::new(), Vec::new()));
    }

    #[test]
    fn spaces_do_not_count() {
        assert_eq!(lines("  a   b \n\n c"), owned(&["a b", "c"]));
    }

    #[test]
    fn same_document_has_no_difference() {
        let pdf = crate::text_document("one\ntwo").unwrap();
        assert!(compare_text(&pdf, &pdf).unwrap().is_empty());
        let other = crate::text_document("one\nthree").unwrap();
        let diffs = compare_text(&pdf, &other).unwrap();
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].removed, ["two"]);
        assert_eq!(diffs[0].added, ["three"]);
    }
}
