//! The Sign tool (egui `GodApp::place_signature`): a click puts the saved signature there,
//! or asks for one first on the pad.

use godpdf_core::Annotation;
use gpui::{App, Context, Entity};

use super::canvas::{Selected, annotations, page_frame};
use super::{Tool, tool_state};
use crate::doc::DocTab;
use crate::signature::{self, Signature};
use crate::toasts;

/// A click of the Sign tool at `at` (displayed points) of `page`.
pub fn click(tab: &mut DocTab, page: usize, at: (f32, f32), cx: &mut Context<DocTab>) {
    match signature::saved(cx) {
        Some(signature) => place(tab, page, at, &signature, cx),
        None => crate::dialogs::draw_signature(Some((cx.entity().downgrade(), page, at)), cx),
    }
}

/// Put `signature` on `page`, top left corner at `at` (displayed points). It is then
/// selected, to be moved or resized.
pub fn place(
    tab: &mut DocTab,
    page: usize,
    at: (f32, f32),
    signature: &Signature,
    cx: &mut Context<DocTab>,
) {
    let Some((geom, displayed)) = page_frame(tab, page) else {
        return;
    };
    let strokes = signature
        .placed(at)
        .into_iter()
        .map(|stroke| {
            stroke
                .into_iter()
                .map(|point| crate::geometry::to_pdf(&geom, displayed, point))
                .collect()
        })
        .collect();
    let annotation = Annotation::Ink {
        strokes,
        color: signature::INK,
        width: signature::PEN,
    };
    let result = tab.apply("Sign", |pdf| {
        godpdf_core::add_annotation(pdf, page, &annotation)
    });
    if let Err(err) = result {
        toasts::error(cx, err);
        return;
    }
    if let Some(placed) = annotations(tab, page, cx).last() {
        tab.canvas.selected = Some(Selected {
            page,
            id: placed.id.clone(),
        });
    }
    tool_state(cx).tool = Tool::Select;
    cx.refresh_windows();
    toasts::success(cx, "Signed: drag the signature to move it");
}

/// The pad was confirmed for a click on `tab`: sign there now.
pub fn place_in(tab: &Entity<DocTab>, page: usize, at: (f32, f32), cx: &mut App) {
    let Some(signature) = signature::saved(cx) else {
        return;
    };
    tab.update(cx, |tab, cx| {
        place(tab, page, at, &signature, cx);
        cx.notify();
    });
}
