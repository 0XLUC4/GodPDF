//! Other formats: an archival PDF/A copy and a Word document, made in the background.

use godpdf_core as core;
use gpui::{App, Global};

use super::kinds::{Kind, stem};
use super::{active_tab, open_form, plain};
use crate::doc::guarded;
use crate::toasts;

/// A PDF/A copy made, waiting in its dialog (which shows its warnings) to be saved.
#[derive(Default)]
struct Converted(Option<(String, core::PdfA)>);

impl Global for Converted {}

/// Convert the document in front to PDF/A in the background, then show what could not be
/// guaranteed before asking where to save it.
pub fn pdfa(cx: &mut App) {
    let Some(tab) = active_tab(cx) else {
        toasts::info(cx, "Open a PDF first");
        return;
    };
    let doc = tab.read(cx);
    let name = format!("{} (PDF-A).pdf", stem(doc.title()));
    let bytes = doc.session.bytes().to_vec();
    let converted = crate::services::jobs::background(cx, move || {
        guarded(|| core::convert_to_pdfa(&bytes)).map_err(|err| plain(err.to_string()))
    });
    cx.spawn(async move |cx| {
        let converted = converted.await;
        cx.update(|cx| match converted {
            Ok(pdfa) => {
                cx.set_global(Converted(Some((name, pdfa))));
                open_form(Kind::PdfA, cx);
            }
            Err(err) => toasts::error(cx, format!("Cannot make a PDF/A copy: {err}")),
        });
    })
    .detach();
}

/// What the PDF/A copy waiting to be saved does not guarantee, as one line.
pub fn pdfa_warnings(cx: &App) -> Option<String> {
    let (_, pdfa) = cx.try_global::<Converted>()?.0.as_ref()?;
    (!pdfa.warnings.is_empty()).then(|| format!("Not guaranteed. {}", pdfa.warnings.join(". ")))
}

/// Ask where to save the PDF/A copy and write it.
pub fn save_pdfa(cx: &mut App) -> Result<(), String> {
    let (name, pdfa) = cx
        .try_global::<Converted>()
        .and_then(|converted| converted.0.clone())
        .ok_or("The PDF/A copy is gone: make it again")?;
    crate::files::save_new_pdf(cx, &name, pdfa.bytes, |_, cx| {
        cx.set_global(Converted::default());
        toasts::note(cx, "Saved as PDF/A-2b (best effort)");
    });
    Ok(())
}

/// Make the Word document in the background, then ask where to save it.
pub fn export_word(pdf: Vec<u8>, pictures: bool, title: &str, cx: &mut App) {
    let name = format!("{}.docx", stem(title));
    let exported = crate::services::jobs::background(cx, move || {
        guarded(|| core::export_docx(&pdf, pictures)).map_err(|err| plain(err.to_string()))
    });
    cx.spawn(async move |cx| {
        let exported = exported.await;
        cx.update(|cx| match exported {
            Ok(docx) => crate::files::pick_save(cx, &name, false, move |target, cx| {
                match crate::files::write_atomic(&target, &docx) {
                    Ok(()) => toasts::note(cx, "Exported to Word"),
                    Err(err) => toasts::error(cx, err),
                }
            }),
            Err(err) => toasts::error(cx, format!("Cannot export to Word: {err}")),
        });
    })
    .detach();
}
