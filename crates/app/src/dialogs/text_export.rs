//! Export as text: the text of every page of the document in front, in a .txt file.

use godpdf_core as core;
use gpui::App;

use super::{active_tab, plain};
use crate::doc::guarded;
use crate::toasts;

/// Ask where to save the text of the document in front, then extract and write it in the
/// background.
pub(super) fn export_text(cx: &mut App) {
    let Some(tab) = active_tab(cx) else {
        toasts::info(cx, "Open a PDF first");
        return;
    };
    let doc = tab.read(cx);
    let name = txt_name(doc.title());
    let bytes = doc.session.bytes().to_vec();
    crate::files::pick_save(cx, &name, false, move |target, cx| {
        let written = crate::services::jobs::background(cx, move || {
            let pages =
                guarded(|| core::extract_all_text(&bytes)).map_err(|err| plain(err.to_string()))?;
            crate::files::write_atomic(&target, joined(&pages).as_bytes())?;
            Ok(target)
        });
        cx.spawn(async move |cx| {
            let result = written.await;
            cx.update(|cx| match result {
                Ok(target) => toasts::note(cx, format!("Text saved to {}", target.display())),
                Err(err) => toasts::error(cx, format!("Cannot export the text: {err}")),
            });
        })
        .detach();
    });
}

/// `report.pdf` -> `report.txt`.
fn txt_name(title: &str) -> String {
    let stem = std::path::Path::new(title)
        .file_stem()
        .map_or_else(|| "document".into(), |stem| stem.to_string_lossy());
    format!("{stem}.txt")
}

/// The pages' text one after the other, each under a `--- Page N ---` line.
fn joined(pages: &[String]) -> String {
    pages
        .iter()
        .enumerate()
        .map(|(index, text)| format!("--- Page {} ---\n{}\n", index + 1, text.trim_end()))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_are_separated_and_numbered() {
        let pages = ["Hello\n".to_owned(), String::new(), "End".to_owned()];
        assert_eq!(
            joined(&pages),
            "--- Page 1 ---\nHello\n\n--- Page 2 ---\n\n\n--- Page 3 ---\nEnd\n"
        );
        assert_eq!(joined(&[]), "");
    }

    #[test]
    fn text_file_named_after_the_document() {
        assert_eq!(txt_name("report.pdf"), "report.txt");
        assert_eq!(txt_name("notes"), "notes.txt");
        assert_eq!(txt_name(""), "document.txt");
    }
}
