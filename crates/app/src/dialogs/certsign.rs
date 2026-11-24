//! Certificate signatures: sign a copy with a .pfx/.p12 file, and list the signatures of a
//! document. Signing writes a new file, since the signature covers the saved bytes: any later
//! edit made here rewrites the file and invalidates it.

use std::path::PathBuf;

use godpdf_core::{self as core, SignatureInfo, SignatureStatus};
use gpui::prelude::*;
use gpui::{AnyElement, App, Entity, Global, Hsla, div, px};

use super::form::Form;
use super::kinds::{Kind, stem};
use super::{DialogCancel, Open, active_app, active_tab, card, close, has_password, plain, state};
use crate::doc::{DocTab, guarded};
use crate::theme::Palette;
use crate::widgets::{framed_button, with_icon};
use crate::{files, fonts, icons, toasts};

pub const CERTIFICATE: &str = "Certificate file";
pub const PASSWORD: &str = "Password";
const REASON: &str = "Reason";
const LOCATION: &str = "Location";
const NOT_TRUSTED: &str = "Certificates are not checked against a trusted authority: \
                           self-signed ones are accepted.";

/// The signatures of a document, for the Signatures dialog.
pub struct List {
    title: String,
    signatures: Vec<SignatureInfo>,
}

/// The certificate file picked for the Sign dialog being opened.
#[derive(Default)]
struct Picked(Option<PathBuf>);

impl Global for Picked {}

/// "Sign with certificate…": pick the .pfx/.p12 file, then ask its password.
pub fn start(cx: &mut App) {
    let Some(tab) = active_tab(cx) else {
        toasts::info(cx, "Open a PDF first");
        return;
    };
    if has_password(tab.entity_id(), cx) {
        toasts::error(
            cx,
            "Remove the password first: a signed copy cannot be encrypted",
        );
        return;
    }
    files::pick_file(
        cx,
        ("Certificate with its key (.pfx, .p12)", &["pfx", "p12"]),
        |path, cx| {
            cx.set_global(Picked(Some(path)));
            super::open_form(Kind::CertSign, cx);
        },
    );
}

/// Path of the certificate file picked (taken: once).
pub fn picked(cx: &mut App) -> String {
    cx.default_global::<Picked>()
        .0
        .take()
        .map(|path| path.display().to_string())
        .unwrap_or_default()
}

/// The fields of the Sign dialog.
pub fn fields(
    certificate: String,
    window: &mut gpui::Window,
    cx: &mut App,
) -> Vec<super::form::Field> {
    use super::form::{password, text};
    vec![
        text(CERTIFICATE, certificate, "", window, cx),
        password(PASSWORD, "", window, cx),
        text(REASON, "", "optional, e.g. Approved", window, cx),
        text(LOCATION, "", "optional, e.g. Paris", window, cx),
    ]
}

/// Read the key with the password (errors keep the dialog open), then ask where to save the
/// signed copy and write it in the background. The password is dropped here.
pub fn confirm(form: &Form, tab: &Entity<DocTab>, cx: &mut App) -> Result<(), String> {
    if has_password(tab.entity_id(), cx) {
        return Err("Remove the password first: a signed copy cannot be encrypted".into());
    }
    let path = PathBuf::from(form.text(CERTIFICATE, cx).trim());
    let p12 =
        std::fs::read(&path).map_err(|err| format!("Cannot read {}: {err}", path.display()))?;
    let signer = {
        let password = form.text(PASSWORD, cx);
        guarded(|| core::Signer::from_pkcs12(&p12, &password)).map_err(|err| match err {
            core::Error::BadPassword => "Wrong password for this certificate file".to_owned(),
            other => plain(other.to_string()),
        })?
    };
    let options = core::SignOptions {
        reason: form.text(REASON, cx),
        location: form.text(LOCATION, cx),
    };
    let doc = tab.read(cx);
    let (bytes, current) = (doc.session.bytes().to_vec(), doc.session.path.clone());
    let name = format!("{} (signed).pdf", stem(doc.title()));
    files::pick_save(cx, &name, true, move |target, cx| {
        if current.as_ref() == Some(&target) {
            toasts::error(
                cx,
                "Save the signed copy as a new file: the open one stays as it is",
            );
            return;
        }
        let written = crate::services::jobs::background(cx, move || {
            let signed = guarded(|| core::sign(&bytes, &signer, &options))
                .map_err(|err| plain(err.to_string()))?;
            files::write_atomic(&target, &signed)?;
            Ok(target)
        });
        cx.spawn(async move |cx| {
            let written = written.await;
            cx.update(|cx| match written {
                // Opened in a tab: its signature is checked and shown.
                Ok(path) => match active_app(cx) {
                    Some(app) => app.update(cx, |app, cx| app.open_path(&path, cx)),
                    None => toasts::note(cx, format!("Signed copy saved as {}", path.display())),
                },
                Err(err) => toasts::error(cx, err),
            });
        })
        .detach();
    });
    Ok(())
}

/// "Signatures…": check the signatures of the document in front, then list them.
pub fn show(cx: &mut App) {
    let Some(tab) = active_tab(cx) else {
        toasts::info(cx, "Open a PDF first");
        return;
    };
    let doc = tab.read(cx);
    let (title, bytes) = (doc.title().to_owned(), doc.session.bytes().to_vec());
    let checked = crate::services::jobs::background(cx, move || {
        guarded(|| core::signatures(&bytes)).map_err(|err| plain(err.to_string()))
    });
    cx.spawn(async move |cx| {
        let checked = checked.await;
        cx.update(|cx| match checked {
            Ok(signatures) => {
                state(cx).open = Some(Open::Signatures(List { title, signatures }));
                cx.refresh_windows();
            }
            Err(err) => toasts::error(cx, err),
        });
    })
    .detach();
}

/// A document just opened: when it is signed, say whether its signatures hold.
pub fn check_on_open(tab: &Entity<DocTab>, cx: &mut App) {
    let bytes = tab.read(cx).session.bytes().to_vec();
    let checked = crate::services::jobs::background(cx, move || {
        if !bytes.windows(10).any(|window| window == b"/ByteRange") {
            return Ok(Vec::new());
        }
        guarded(|| core::signatures(&bytes)).map_err(|err| plain(err.to_string()))
    });
    cx.spawn(async move |cx| {
        let Ok(signatures) = checked.await else {
            return;
        };
        cx.update(|cx| match &signatures[..] {
            [] => {}
            [one] if matches!(one.status, SignatureStatus::Invalid(_)) => toasts::error(
                cx,
                format!("Signed by {}: {}", one.signer, status_text(&one.status)),
            ),
            [one] => toasts::note(
                cx,
                format!(
                    "Signed by {}: {}. Certificate not checked against a trusted authority.",
                    one.signer,
                    status_text(&one.status)
                ),
            ),
            many => toasts::note(
                cx,
                format!("{} signatures: “Signatures…” checks each one", many.len()),
            ),
        });
    })
    .detach();
}

fn status_text(status: &SignatureStatus) -> String {
    match status {
        SignatureStatus::Valid => "Valid — document unchanged since signing".into(),
        SignatureStatus::ModifiedAfter => {
            "Modified after signing — the signed version is intact".into()
        }
        SignatureStatus::Invalid(why) => format!("Invalid — {why}"),
        SignatureStatus::Unchecked(why) => format!("Not checked — {why}"),
    }
}

fn status_look(status: &SignatureStatus, palette: &Palette) -> (&'static str, Hsla) {
    match status {
        SignatureStatus::Valid => (icons::CHECK_CIRCLE, palette.success),
        SignatureStatus::ModifiedAfter | SignatureStatus::Unchecked(_) => {
            (icons::INFO, palette.text_dim)
        }
        SignatureStatus::Invalid(_) => (icons::WARNING_CIRCLE, palette.error),
    }
}

pub fn render(list: &List, palette: &Palette) -> AnyElement {
    let palette = *palette;
    let rows = list.signatures.iter().map(|info| {
        let (glyph, colour) = status_look(&info.status, &palette);
        let details = [
            info.signed_at.as_ref().map(|at| format!("Signed {at}")),
            info.reason
                .as_ref()
                .map(|reason| format!("Reason: {reason}")),
            info.location
                .as_ref()
                .map(|place| format!("Location: {place}")),
            Some(format!("Field: {}", info.field)),
        ];
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .p(px(8.))
            .rounded(px(6.))
            .bg(palette.surface)
            .child(div().child(info.signer.clone()))
            .child(with_icon(
                glyph,
                status_text(&info.status),
                fonts::BODY,
                colour,
            ))
            .children(
                details
                    .into_iter()
                    .flatten()
                    .map(|line| div().text_color(palette.text_dim).child(line)),
            )
    });
    card(&palette, 460.)
        .key_context("GodPdfDialog")
        .on_action(|_: &DialogCancel, window, cx| close(window, cx))
        .child(super::title(with_icon(
            icons::SIGNATURE,
            format!("Signatures of {}", list.title),
            19.0,
            palette.text,
        )))
        .when(list.signatures.is_empty(), |card| {
            card.child(
                div()
                    .text_color(palette.text_dim)
                    .child("This document has no certificate signature."),
            )
        })
        .child(
            div()
                .id("signature-list")
                .flex()
                .flex_col()
                .gap(px(6.))
                .max_h(px(320.))
                .overflow_y_scroll()
                .children(rows),
        )
        .child(div().text_color(palette.text_dim).child(NOT_TRUSTED))
        .child(
            super::button_row().child(
                framed_button("signatures-close", "Close", true, &palette)
                    .on_click(|_, window, cx| close(window, cx)),
            ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_read_well() {
        assert_eq!(
            status_text(&SignatureStatus::Valid),
            "Valid — document unchanged since signing"
        );
        assert_eq!(
            status_text(&SignatureStatus::Invalid("bad".into())),
            "Invalid — bad"
        );
    }
}
