//! Comments panel: every annotation we understand,
//! grouped by page. Read in a background job once per document generation, never per frame.

use godpdf_core::Annotation;
use gpui::component::spinner::Spinner;
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, div, px};

use super::label;
use crate::doc::{DocTab, guarded};
use crate::icons;
use crate::theme::Palette;
use crate::widgets;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Highlight,
    Note,
    FreeText,
    Rectangle,
    Ink,
    Picture,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Self::Highlight => "Highlight",
            Self::Note => "Note",
            Self::FreeText => "Text box",
            Self::Rectangle => "Rectangle",
            Self::Ink => "Ink",
            Self::Picture => "Picture",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Highlight => icons::HIGHLIGHTER,
            Self::Note => icons::NOTE,
            Self::FreeText => icons::TEXT_T,
            Self::Rectangle => icons::RECTANGLE,
            Self::Ink => icons::PENCIL_SIMPLE,
            Self::Picture => icons::IMAGE,
        }
    }
}

pub struct Entry {
    page: usize,
    kind: Kind,
    /// Note / free-text content; empty for markup.
    text: String,
}

impl Entry {
    fn new(page: usize, annotation: Annotation) -> Self {
        let (kind, text) = match annotation {
            Annotation::Highlight { .. }
            | Annotation::Underline { .. }
            | Annotation::StrikeOut { .. } => (Kind::Highlight, String::new()),
            Annotation::Note { text, .. } => (Kind::Note, text),
            Annotation::FreeText { text, .. } => (Kind::FreeText, text),
            Annotation::Square { .. } | Annotation::Circle { .. } => {
                (Kind::Rectangle, String::new())
            }
            Annotation::Ink { .. } | Annotation::Line { .. } => (Kind::Ink, String::new()),
            Annotation::Image { .. } | Annotation::Stamp { .. } => (Kind::Picture, String::new()),
        };
        Self { page, kind, text }
    }
}

type Found = Result<Vec<Entry>, String>;

/// Comments of one document: the generation they were read from, `None` while reading.
#[derive(Default)]
pub struct Comments(Option<(u64, Option<Found>)>);

/// Read the comments of every page. Slow on big files: run it in the background.
fn collect(pdf: &[u8], page_count: usize) -> Found {
    let mut entries = Vec::new();
    for page in 0..page_count {
        let placed = guarded(|| godpdf_core::annotations(pdf, page))
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

/// Start reading the comments when they are missing or from older bytes.
fn request(tab: &mut DocTab, cx: &mut Context<DocTab>) {
    let generation = tab.generation();
    if tab
        .panels
        .comments
        .0
        .as_ref()
        .is_some_and(|(read, _)| *read == generation)
    {
        return;
    }
    tab.panels.comments.0 = Some((generation, None));
    let (bytes, pages) = (tab.session.bytes().to_vec(), tab.page_count());
    let found = crate::services::jobs::background(cx, async move { collect(&bytes, pages) });
    cx.spawn(async move |this, cx| {
        let result = found.await;
        let _ = this.update(cx, |tab, cx| {
            if let Some((read, slot)) = tab.panels.comments.0.as_mut()
                && *read == generation
            {
                *slot = Some(result);
                cx.notify();
            }
        });
    })
    .detach();
}

/// Number of comments once read, for the panel title.
pub fn count(tab: &Entity<DocTab>, cx: &App) -> Option<usize> {
    let tab = tab.read(cx);
    match &tab.panels.comments.0 {
        Some((read, Some(Ok(entries)))) if *read == tab.generation() => Some(entries.len()),
        _ => None,
    }
}

pub fn render(tab: &Entity<DocTab>, cx: &mut App) -> AnyElement {
    let palette = Palette::of(cx);
    tab.update(cx, request);
    let doc = tab.read(cx);
    let current = doc.view.current_page;
    let entries = match doc
        .panels
        .comments
        .0
        .as_ref()
        .and_then(|(_, found)| found.as_ref())
    {
        None => {
            return div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(Spinner::new().color(palette.text_dim))
                .child(label("Reading comments…", palette.text_dim))
                .into_any_element();
        }
        Some(Err(err)) => {
            return label(format!("Cannot read comments: {err}"), palette.error).into_any_element();
        }
        Some(Ok(entries)) if entries.is_empty() => {
            return div()
                .pt(px(8.))
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(label("No comments in this document.", palette.text_dim))
                .child(label(
                    "Pick a comment tool on the page to add one.",
                    palette.text_dim,
                ))
                .into_any_element();
        }
        Some(Ok(entries)) => entries,
    };
    let mut list = Vec::new();
    for (index, group) in entries.chunk_by(|a, b| a.page == b.page).enumerate() {
        let page = group[0].page;
        list.push(
            div()
                .mt(px(6.))
                .mb(px(2.))
                .flex()
                .items_center()
                .justify_between()
                .child(format!("Page {}", page + 1))
                .child(label(group.len().to_string(), palette.text_dim))
                .into_any_element(),
        );
        for (row_index, entry) in group.iter().enumerate() {
            list.push(row(
                tab,
                (index, row_index),
                entry,
                page == current,
                &palette,
            ));
        }
    }
    div()
        .id("comments")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(6.))
        .children(list)
        .into_any_element()
}

fn row(
    tab: &Entity<DocTab>,
    id: (usize, usize),
    entry: &Entry,
    current: bool,
    palette: &Palette,
) -> AnyElement {
    let (tab, page, hover) = (tab.downgrade(), entry.page, palette.hover);
    div()
        .id(gpui::ElementId::Name(
            format!("comment-{}-{}", id.0, id.1).into(),
        ))
        .flex_none()
        .p(px(8.))
        .rounded(px(8.))
        .flex()
        .items_start()
        .gap(px(8.))
        .cursor_pointer()
        .when(current, |row| row.bg(palette.surface))
        .hover(move |style| style.bg(hover))
        .child(
            div()
                .flex_none()
                .size(px(28.))
                .rounded(px(14.))
                .bg(palette.surface)
                .flex()
                .items_center()
                .justify_center()
                .child(widgets::icon(entry.kind.icon(), 16.0, palette.text)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(label(entry.kind.label(), palette.text_dim))
                .when(!entry.text.is_empty(), |column| {
                    column.child(entry.text.clone())
                }),
        )
        .on_click(move |_, _, cx| {
            let _ = tab.update(cx, |tab, cx| {
                tab.scroll_to_page(page);
                cx.notify();
            });
        })
        .into_any_element()
}
