//! Two documents compared: the lines of text each page lost or gained, and the pages that
//! look different though their text is the same.

use std::sync::Arc;

use egui::{Color32, Context, Id, RichText, ScrollArea};

use crate::theme::Palette;
use crate::widgets::{icon, with_icon};

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

pub struct CompareView {
    pub title: String,
    pub pages: Vec<PageReport>,
}

/// Compare `first` with `second`. Slow on big files: run it in a job.
pub fn run(first: Vec<u8>, second: Vec<u8>, title: String) -> Result<CompareView, String> {
    let diffs = godpdf_core::compare_text(&first, &second).map_err(|err| err.to_string())?;
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
    let total: u64 = left
        .rgba
        .iter()
        .zip(&right.rgba)
        .map(|(x, y)| u64::from(x.abs_diff(*y)))
        .sum();
    total as f64 / left.rgba.len().max(1) as f64 > VISIBLE
}

/// The report in a window. Returns false when closed, and the page to show.
pub fn window(ctx: &Context, view: &CompareView) -> (bool, Option<usize>) {
    let mut open = true;
    let mut jump = None;
    egui::Window::new(&view.title)
        .id(Id::new("compare-view"))
        .open(&mut open)
        .default_size([620.0, 540.0])
        .show(ctx, |ui| {
            let palette = Palette::of(ui);
            if view.pages.is_empty() {
                let same = with_icon(icon::CHECK_CIRCLE, "The two documents are the same");
                ui.label(RichText::new(same).color(palette.success));
                return;
            }
            let count = format!("{} pages differ", view.pages.len());
            ui.label(RichText::new(count).color(palette.text_dim));
            ui.separator();
            let removed = Color32::from_rgb(0xD6, 0x3B, 0x3B);
            let added = Color32::from_rgb(0x2F, 0x9E, 0x44);
            ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                for report in &view.pages {
                    ui.add_space(6.0);
                    let title = RichText::new(format!("Page {}", report.page + 1)).strong();
                    if ui.link(title).on_hover_text("Show this page").clicked() {
                        jump = Some(report.page);
                    }
                    let note = if report.missing {
                        Some("Only one of the documents has this page")
                    } else if report.looks_different
                        && report.removed.is_empty()
                        && report.added.is_empty()
                    {
                        Some("Same text, drawn differently")
                    } else {
                        None
                    };
                    if let Some(note) = note {
                        ui.label(RichText::new(note).color(palette.text_dim));
                    }
                    for line in &report.removed {
                        ui.label(RichText::new(format!("- {line}")).color(removed));
                    }
                    for line in &report.added {
                        ui.label(RichText::new(format!("+ {line}")).color(added));
                    }
                }
            });
        });
    (open, jump)
}
