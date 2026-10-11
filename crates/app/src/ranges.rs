//! Human page ranges ("1-3, 5, 8-") <-> 0-based page indices.

/// Parse 1-based ranges into 0-based indices, in the order written. `8-` means 8..=last.
pub fn parse(text: &str, page_count: usize) -> Result<Vec<usize>, String> {
    let mut pages = Vec::new();
    for part in text
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let (start, end) = match part.split_once('-') {
            Some((start, end)) => {
                let start = if start.trim().is_empty() {
                    1
                } else {
                    number(start)?
                };
                let end = if end.trim().is_empty() {
                    page_count
                } else {
                    number(end)?
                };
                (start, end)
            }
            None => {
                let page = number(part)?;
                (page, page)
            }
        };
        if start == 0 || end > page_count || start > end {
            return Err(format!("\"{part}\" is outside 1-{page_count}"));
        }
        pages.extend(start - 1..end);
    }
    if pages.is_empty() {
        return Err("No pages given".to_owned());
    }
    Ok(pages)
}

fn number(text: &str) -> Result<usize, String> {
    text.trim()
        .parse()
        .map_err(|_| format!("\"{}\" is not a page number", text.trim()))
}

/// Format sorted 0-based indices as compact 1-based ranges.
pub fn format(pages: &[usize]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut iter = pages.iter().copied().peekable();
    while let Some(start) = iter.next() {
        let mut end = start;
        while iter.peek() == Some(&(end + 1)) {
            end += 1;
            iter.next();
        }
        out.push(if start == end {
            format!("{}", start + 1)
        } else {
            format!("{}-{}", start + 1, end + 1)
        });
    }
    out.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mixed_ranges() {
        assert_eq!(parse("1-3, 5, 8-", 9), Ok(vec![0, 1, 2, 4, 7, 8]));
        assert_eq!(parse("-2", 5), Ok(vec![0, 1]));
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse("0", 3).is_err());
        assert!(parse("4", 3).is_err());
        assert!(parse("3-1", 3).is_err());
        assert!(parse("abc", 3).is_err());
        assert!(parse(" , ", 3).is_err());
    }

    #[test]
    fn formats_compactly() {
        assert_eq!(format(&[0, 1, 2, 4, 7, 8]), "1-3, 5, 8-9");
        assert_eq!(format(&[]), "");
    }
}
