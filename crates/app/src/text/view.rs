//! The windows over the document: the extracted text and the
//! comparison of two documents. Not modal: the document stays usable.

use gpui::component::input::{Textarea, TextareaState};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClipboardItem, Div, Entity, MouseButton, SharedString, Window, div, px,
    relative, rgb,
};

use super::compare::CompareView;
use crate::theme::{self, Palette};
use crate::{fonts, icons, widgets};

/// Lines a page lost / gained (rgb D6 3B 3B / 2F 9E 44).
const REMOVED: u32 = 0xD63B3B;
const ADDED: u32 = 0x2F9E44;

pub struct TextView {
    pub title: String,
    pub text: String,
    /// Characters of the document's text (page headers left out); 0 for scanned pages.
    pub chars: usize,
    /// The read-only field, made on the first frame (it needs the window).
    pub area: Option<Entity<TextareaState>>,
}

#[derive(Default)]
pub struct Windows {
    pub text: Option<TextView>,
    pub compare: Option<CompareView>,
}

/// A floating window: title bar with a close cross, panel fill, window radius and shadow.
/// `offset` moves it from the middle of the app window, so two do not hide each other.
fn floating(
    id: &'static str,
    title: String,
    size: (f32, f32),
    offset: f32,
    close: impl Fn(&mut App) + 'static,
    palette: &Palette,
) -> Div {
    let card = div()
        .id(id)
        .w(px(size.0))
        .h(px(size.1))
        .max_w_full()
        .max_h_full()
        .ml(px(offset))
        .mt(px(offset))
        .flex()
        .flex_col()
        .gap(px(theme::ITEM_SPACING.1))
        .p(px(theme::WINDOW_MARGIN))
        .rounded(px(theme::WINDOW_RADIUS))
        .shadow(widgets::shadow(10.0, 28.0, palette.shadow_alpha))
        .bg(palette.panel)
        .border_1()
        .border_color(palette.border)
        .text_color(palette.text)
        .text_size(px(fonts::BODY))
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .flex()
                .items_center()
                .child(div().flex_1().truncate().text_size(px(18.)).child(title))
                .child(
                    widgets::small_icon_button((id, 0usize), icons::X, "Close", palette)
                        .on_click(move |_, _, cx| close(cx)),
                ),
        )
        .child(separator(palette));
    // Centered over the window without catching clicks around the card.
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .child(card)
}

fn separator(palette: &Palette) -> Div {
    div().flex_none().h(px(1.)).w_full().bg(palette.border)
}

fn windows(cx: &mut App) -> &mut Windows {
    &mut super::state(cx).windows
}

/// The open windows, drawn over the app.
pub fn render(window: &mut Window, cx: &mut App) -> Vec<AnyElement> {
    let palette = Palette::of(cx);
    let mut shown = Vec::new();
    if let Some(view) = windows(cx).text.as_mut() {
        let area = match &view.area {
            Some(area) => area.clone(),
            None => {
                let text = view.text.clone();
                let area = cx.new(|cx| TextareaState::new(window, cx).default_value(text));
                // `view` borrows the global: look it up again.
                if let Some(view) = windows(cx).text.as_mut() {
                    view.area = Some(area.clone());
                }
                area
            }
        };
        let Some(view) = windows(cx).text.as_ref() else {
            return shown;
        };
        let (text, chars) = (view.text.clone(), view.chars);
        let count = match chars {
            1 => "1 character".to_owned(),
            chars => format!("{chars} characters"),
        };
        let close = |cx: &mut App| {
            windows(cx).text = None;
            cx.refresh_windows();
        };
        let copy = widgets::framed_button(
            "copy-all-text",
            widgets::with_icon(icons::COPY, "Copy all", fonts::BODY, palette.text),
            true,
            &palette,
        )
        .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.clone())));
        let frame = floating(
            "text-view",
            view.title.clone(),
            (560., 520.),
            0.,
            close,
            &palette,
        );
        if chars == 0 {
            shown.push(
                frame
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .gap(px(theme::ITEM_SPACING.0))
                            .text_color(palette.text_dim)
                            .child(widgets::icon(icons::SCAN, 32.0, palette.text_dim))
                            .child("No text found")
                            .child("Scanned pages need Recognize text (OCR) first."),
                    )
                    .into_any_element(),
            );
        } else {
            shown.push(
                frame
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(theme::ITEM_SPACING.0))
                            .child(copy)
                            .child(div().text_color(palette.text_dim).child(count)),
                    )
                    .child(separator(&palette))
                    .child(
                        // A flex row would size the text area to its content, a sliver on the left.
                        div().flex_1().min_h_0().w_full().flex().flex_col().child(
                            Textarea::new(&area)
                                .readonly(true)
                                .w_full()
                                .h(relative(1.))
                                .font_family(fonts::MONO),
                        ),
                    )
                    .into_any_element(),
            );
        }
    }
    if let Some(view) = windows(cx).compare.as_ref() {
        let close = |cx: &mut App| {
            windows(cx).compare = None;
            cx.refresh_windows();
        };
        let body: AnyElement = if view.pages.is_empty() {
            widgets::with_icon(
                icons::CHECK_CIRCLE,
                "The two documents are the same",
                fonts::BODY,
                palette.success,
            )
            .into_any_element()
        } else {
            let reports = view.pages.iter().map(|report| {
                let page = report.page;
                let link = div()
                    .id(("compare-page", page))
                    .text_color(palette.accent_text)
                    .cursor_pointer()
                    .hover(|style| style.underline())
                    .child(format!("Page {}", page + 1))
                    .on_click(move |_, _, cx| super::show_page(page, cx));
                div()
                    .pt(px(6.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(widgets::tooltip(link, "Show this page"))
                    .children(
                        report
                            .note()
                            .map(|note| div().text_color(palette.text_dim).child(note)),
                    )
                    .children(report.removed.iter().map(|line| {
                        div()
                            .text_color(rgb(REMOVED))
                            .child(SharedString::from(format!("- {line}")))
                    }))
                    .children(report.added.iter().map(|line| {
                        div()
                            .text_color(rgb(ADDED))
                            .child(SharedString::from(format!("+ {line}")))
                    }))
            });
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .gap(px(theme::ITEM_SPACING.1))
                .child(
                    div()
                        .text_color(palette.text_dim)
                        .child(match view.pages.len() {
                            1 => "1 page differs".to_owned(),
                            count => format!("{count} pages differ"),
                        }),
                )
                .child(separator(&palette))
                .child(
                    div()
                        .id("compare-list")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .children(reports),
                )
                .into_any_element()
        };
        shown.push(
            floating(
                "compare-view",
                view.title.clone(),
                (620., 540.),
                40.,
                close,
                &palette,
            )
            .child(body)
            .into_any_element(),
        );
    }
    shown
}
