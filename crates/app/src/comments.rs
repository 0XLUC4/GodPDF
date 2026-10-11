//! Comments panel: every annotation we understand, grouped by page.
//! Parsed in a background job once per document revision, never per frame.

use std::collections::HashMap;

use egui::{CornerRadius, RichText, ScrollArea, Sense, Ui, vec2};
use godpdf_core::Annotation;

use crate::theme::Palette;
use crate::widgets::icon;

#[derive(Clone, Copy)]
pub enum Kind {
    Highlight,
    Note,
    FreeText,
    Rectangle,
    Ink,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Self::Highlight => "Highlight",
            Self::Note => "Note",
            Self::FreeText => "Text box",
            Self::Rectangle => "Rectangle",
            Self::Ink => "Ink",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Highlight => icon::HIGHLIGHTER,
            Self::Note => icon::NOTE,
            Self::FreeText => icon::TEXT_T,
            Self::Rectangle => icon::RECTANGLE,
            Self::Ink => icon::PENCIL_SIMPLE,
        }
    }
}

pub struct Entry {
    pub page: usize,
    pub kind: Kind,
    /// Note / free-text content; empty for markup.
    pub text: String,
}

impl Entry {
    fn new(page: usize, annotation: Annotation) -> Self {
        let (kind, text) = match annotation {
            Annotation::Highlight { .. } => (Kind::Highlight, String::new()),
            Annotation::Note { text, .. } => (Kind::Note, text),
            Annotation::FreeText { text, .. } => (Kind::FreeText, text),
            Annotation::Square { .. } => (Kind::Rectangle, String::new()),
            Annotation::Ink { .. } => (Kind::Ink, String::new()),
        };
        Self { page, kind, text }
    }
}

/// Read the comments of every page. Slow on big files: run it in a job.
pub fn collect(pdf: &[u8], page_count: usize) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    for page in 0..page_count {
        let placed = crate::doc::guarded(|| godpdf_core::annotations(pdf, page))
            .map_err(|err| format!("Page {}: {err}", page + 1))?;
        entries.extend(
            placed
                .into_iter()
                .filter_map(|placed| placed.annotation)
                .map(|annotation| Entry::new(page, annotation)),
        );
    }
    Ok(entries)
}

type Comments = Result<Vec<Entry>, String>;

/// Comments per tab id, tagged with the document generation they were read from.
#[derive(Default)]
pub struct Cache {
    /// `None` while the job runs.
    slots: HashMap<u64, (u64, Option<Comments>)>,
}

impl Cache {
    /// True when `(tab, generation)` has no result nor job yet; marks it as loading.
    pub fn start(&mut self, tab: u64, generation: u64) -> bool {
        if self
            .slots
            .get(&tab)
            .is_some_and(|(cached, _)| *cached == generation)
        {
            return false;
        }
        self.slots.insert(tab, (generation, None));
        true
    }

    pub fn store(&mut self, tab: u64, generation: u64, comments: Comments) {
        if self
            .slots
            .get(&tab)
            .is_some_and(|(cached, _)| *cached == generation)
        {
            self.slots.insert(tab, (generation, Some(comments)));
        }
    }

    /// `None` while loading (or never requested).
    pub fn get(&self, tab: u64, generation: u64) -> Option<&Comments> {
        self.slots
            .get(&tab)
            .filter(|(cached, _)| *cached == generation)
            .and_then(|(_, comments)| comments.as_ref())
    }

    /// Forget loading slots (a job failed), so they are retried.
    pub fn drop_loading(&mut self) {
        self.slots.retain(|_, (_, comments)| comments.is_some());
    }

    pub fn retain_tabs(&mut self, alive: impl Fn(u64) -> bool) {
        self.slots.retain(|tab, _| alive(*tab));
    }
}

/// Number of comments, once loaded.
pub fn count(comments: Option<&Comments>) -> Option<usize> {
    comments
        .and_then(|result| result.as_ref().ok())
        .map(Vec::len)
}

/// The list; returns the page to jump to.
pub fn show(ui: &mut Ui, comments: Option<&Comments>, current_page: usize) -> Option<usize> {
    let palette = Palette::of(ui);
    let entries = match comments {
        None => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Reading comments…").color(palette.text_dim));
            });
            return None;
        }
        Some(Err(err)) => {
            ui.label(RichText::new(format!("Cannot read comments: {err}")).color(palette.error));
            return None;
        }
        Some(Ok(entries)) if entries.is_empty() => {
            ui.add_space(8.0);
            ui.label(RichText::new("No comments in this document.").color(palette.text_dim));
            ui.label(
                RichText::new("Pick a comment tool on the page to add one.")
                    .size(13.0)
                    .color(palette.text_dim),
            );
            return None;
        }
        Some(Ok(entries)) => entries,
    };
    let mut jump = None;
    ScrollArea::vertical()
        .id_salt("comments")
        .auto_shrink(false)
        .show(ui, |ui| {
            for group in entries.chunk_by(|a, b| a.page == b.page) {
                let page = group[0].page;
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("Page {}", page + 1)).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(group.len().to_string())
                                .size(13.0)
                                .color(palette.text_dim),
                        );
                    });
                });
                ui.add_space(2.0);
                for entry in group {
                    if row(ui, entry, entry.page == current_page, &palette) {
                        jump = Some(entry.page);
                    }
                }
            }
        });
    jump
}

fn row(ui: &mut Ui, entry: &Entry, current: bool, palette: &Palette) -> bool {
    // Background goes under the content, painted once the row size is known.
    let background = ui.painter().add(egui::Shape::Noop);
    let response = egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(8, 8))
        .corner_radius(CornerRadius::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                let (badge, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
                ui.painter()
                    .circle_filled(badge.center(), 14.0, palette.surface);
                ui.painter().text(
                    badge.center(),
                    egui::Align2::CENTER_CENTER,
                    entry.kind.icon(),
                    egui::FontId::proportional(15.0),
                    palette.text,
                );
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(entry.kind.label())
                            .size(13.0)
                            .color(palette.text_dim),
                    );
                    if !entry.text.is_empty() {
                        ui.add(egui::Label::new(RichText::new(&entry.text)).wrap());
                    }
                });
            });
        })
        .response
        .interact(Sense::click());
    if response.hovered() || current {
        let fill = if response.hovered() {
            palette.hover
        } else {
            palette.surface
        };
        ui.painter().set(
            background,
            egui::Shape::rect_filled(response.rect, CornerRadius::same(8), fill),
        );
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}
