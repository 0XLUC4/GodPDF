//! Comments panel: every annotation we understand,
//! grouped by page. Read in a background job once per document generation, never per frame.

use godpdf_core::{Annotation, MeasureKind};
use gpui::component::spinner::Spinner;
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, div, px};

use super::label;
use crate::doc::{DocTab, guarded};
use crate::theme::{self, Palette};
use crate::{fonts, icons, widgets};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Highlight,
    Underline,
    StrikeOut,
    Note,
    FreeText,
    Rectangle,
    Ellipse,
    Line,
    Ink,
    Picture,
    Stamp,
    Polygon,
    Cloud,
    PolyLine,
    Distance,
    Perimeter,
    Area,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Self::Highlight => "Highlight",
            Self::Underline => "Underline",
            Self::StrikeOut => "Strikethrough",
            Self::Note => "Note",
            Self::FreeText => "Free text",
            Self::Rectangle => "Rectangle",
            Self::Ellipse => "Ellipse",
            Self::Line => "Line",
            Self::Ink => "Ink",
            Self::Picture => "Picture",
            Self::Stamp => "Stamp",
            Self::Polygon => "Polygon",
            Self::Cloud => "Cloud",
            Self::PolyLine => "Polyline",
            Self::Distance => "Distance",
            Self::Perimeter => "Perimeter",
            Self::Area => "Area",
        }
    }

    /// The colour of its round badge: one per kind of action.
    fn tint(self) -> u32 {
        match self {
            Self::Highlight => 0xCA8A04,
            Self::Note => theme::AMBER,
            Self::Underline => theme::BLUE,
            Self::StrikeOut => theme::RED,
            Self::FreeText => 0x71717A,
            Self::Rectangle
            | Self::Ellipse
            | Self::Line
            | Self::Polygon
            | Self::Cloud
            | Self::PolyLine => theme::PURPLE,
            // Measurements: an orange of their own, apart from the comment colours.
            Self::Distance | Self::Perimeter | Self::Area => 0xEA580C,
            Self::Ink => theme::PINK,
            Self::Picture => theme::TEAL,
            Self::Stamp => theme::GREEN,
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Highlight => icons::HIGHLIGHTER,
            Self::Underline => icons::TEXT_UNDERLINE,
            Self::StrikeOut => icons::TEXT_STRIKETHROUGH,
            Self::Note => icons::NOTE,
            Self::FreeText => icons::TEXT_T,
            Self::Rectangle => icons::RECTANGLE,
            Self::Ellipse => icons::CIRCLE,
            Self::Line => icons::LINE_SEGMENT,
            Self::Ink => icons::PENCIL_SIMPLE,
            Self::Picture => icons::IMAGE,
            Self::Stamp => icons::STAMP,
            Self::Polygon => icons::POLYGON,
            Self::Cloud => icons::CLOUD,
            Self::PolyLine => icons::LINE_SEGMENTS,
            Self::Distance => icons::RULER,
            Self::Perimeter => icons::PATH,
            Self::Area => icons::BOUNDING_BOX,
        }
    }
}

pub struct Entry {
    page: usize,
    kind: Kind,
    /// Note / free-text content; empty for markup.
    text: String,
    /// Author and last change, empty when the file has none.
    author: String,
    modified: String,
}

impl Entry {
    fn new(page: usize, annotation: Annotation, author: String, modified: String) -> Self {
        let (kind, text) = match annotation {
            Annotation::Highlight { .. } => (Kind::Highlight, String::new()),
            Annotation::Underline { .. } => (Kind::Underline, String::new()),
            Annotation::StrikeOut { .. } => (Kind::StrikeOut, String::new()),
            Annotation::Note { text, .. } => (Kind::Note, text),
            Annotation::FreeText { text, .. } => (Kind::FreeText, text),
            Annotation::Square { .. } => (Kind::Rectangle, String::new()),
            Annotation::Circle { .. } => (Kind::Ellipse, String::new()),
            Annotation::Line { .. } => (Kind::Line, String::new()),
            Annotation::Ink { .. } => (Kind::Ink, String::new()),
            Annotation::Image { .. } => (Kind::Picture, String::new()),
            Annotation::Stamp { .. } => (Kind::Stamp, String::new()),
            Annotation::Polygon { cloud: true, .. } => (Kind::Cloud, String::new()),
            Annotation::Polygon { .. } => (Kind::Polygon, String::new()),
            Annotation::PolyLine { .. } => (Kind::PolyLine, String::new()),
            // The measured value, as written in /Contents.
            Annotation::Measure {
                kind,
                vertices,
                scale,
                ..
            } => {
                let shown = match kind {
                    MeasureKind::Distance => Kind::Distance,
                    MeasureKind::Perimeter => Kind::Perimeter,
                    MeasureKind::Area => Kind::Area,
                };
                (shown, scale.label(kind, &vertices))
            }
        };
        Self {
            page,
            kind,
            text,
            author,
            modified,
        }
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
        entries.extend(placed.into_iter().filter_map(|placed| {
            let annotation = placed.annotation?;
            Some(Entry::new(page, annotation, placed.author, placed.modified))
        }));
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
    let found = crate::services::jobs::background(cx, move || collect(&bytes, pages));
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
                .mt(px(10.))
                .mb(px(2.))
                .flex()
                .items_center()
                .justify_between()
                .font_weight(fonts::STRONG)
                .child(format!("Page {}", page + 1))
                .child(label(group.len().to_string(), palette.text_dim))
                .into_any_element(),
        );
        for (row_index, entry) in group.iter().enumerate() {
            list.push(row(tab, (index, row_index), entry, &palette));
        }
    }
    div()
        .id("comments")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(2.))
        .children(list)
        .into_any_element()
}

fn row(tab: &Entity<DocTab>, id: (usize, usize), entry: &Entry, palette: &Palette) -> AnyElement {
    let (tab, page, hover) = (tab.downgrade(), entry.page, palette.hover);
    let tint = theme::hex(entry.kind.tint());
    // "Author" in bold, then "Kind · date" dimmed; without an author the kind is the title.
    let (title, kind) = if entry.author.is_empty() {
        (entry.kind.label().to_owned(), None)
    } else {
        (entry.author.clone(), Some(entry.kind.label()))
    };
    let details: Vec<&str> = kind
        .into_iter()
        .chain((!entry.modified.is_empty()).then_some(entry.modified.as_str()))
        .collect();
    div()
        .id(gpui::ElementId::Name(
            format!("comment-{}-{}", id.0, id.1).into(),
        ))
        .flex_none()
        .px(px(8.))
        .py(px(10.))
        .rounded(px(8.))
        .flex()
        .items_start()
        .gap(px(10.))
        .cursor_pointer()
        .border_b_1()
        .border_color(palette.border)
        .hover(move |style| style.bg(hover))
        .child(
            div()
                .flex_none()
                .size(px(30.))
                .rounded_full()
                .bg(tint.opacity(0.16))
                .flex()
                .items_center()
                .justify_center()
                .child(widgets::icon(entry.kind.icon(), 16.0, tint)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().font_weight(fonts::STRONG).child(title))
                .when(!details.is_empty(), |column| {
                    column.child(label(details.join("  ·  "), palette.text_dim))
                })
                .when(!entry.text.is_empty(), |column| {
                    column.child(div().pt(px(4.)).child(entry.text.clone()))
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
