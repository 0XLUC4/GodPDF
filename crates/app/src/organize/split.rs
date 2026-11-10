//! "Side by side": a second viewer on the right
//! half, showing another open document or another place of the one in front. It is for
//! reading: the tools act on the document in front.
//!
//! The second viewer is its own `DocTab` (flagged `companion`, so no tools are drawn on it)
//! holding a copy of the shown document's bytes, copied again whenever that one changes.

use gpui::component::button::{Button, ButtonVariants as _};
use gpui::component::input::{Input, InputEvent, InputState};
use gpui::component::menu::{DropdownMenu, PopupMenuItem};
use gpui::prelude::*;
use gpui::{AnyElement, App, Entity, Subscription, WeakEntity, Window, div, px};

use super::state;
use crate::app::GodPdfApp;
use crate::doc::DocTab;
use crate::theme::{self, Palette};
use crate::{fonts, icons, widgets};

pub struct Split {
    /// The document shown on the right.
    source: WeakEntity<DocTab>,
    /// Its copy the second viewer draws.
    mirror: Entity<DocTab>,
    /// `source`'s generation the copy was made from.
    generation: u64,
    /// Page to show once the second viewer knows its size.
    jump: Option<usize>,
    /// The page box while it is typed in.
    field: Option<(Entity<InputState>, Subscription)>,
}

/// Open the second viewer on the document in front, at its page; or close it.
pub fn toggle(cx: &mut App) {
    if state(cx).split.take().is_some() {
        cx.refresh_windows();
        return;
    }
    let Some(tab) = super::front(cx) else { return };
    tab.update(cx, |tab, cx| {
        tab.grid.on = false;
        cx.notify();
    });
    show(&tab.downgrade(), cx);
}

/// Show `source` in the second viewer, at its current page.
fn show(source: &WeakEntity<DocTab>, cx: &mut App) {
    let Some(tab) = source.upgrade() else { return };
    let doc = tab.read(cx);
    let (name, bytes, page) = (
        doc.title().to_owned(),
        doc.session.bytes().to_vec(),
        doc.view.current_page,
    );
    let generation = doc.generation();
    let mirror = cx.new(|_| {
        let mut mirror = DocTab::new(godpdf_core::Session::new(name, bytes));
        mirror.companion = true;
        mirror
    });
    state(cx).split = Some(Split {
        source: source.clone(),
        mirror,
        generation,
        jump: Some(page),
        field: None,
    });
    cx.refresh_windows();
}

/// The second viewer with its row (document, page, close), `None` when closed.
pub fn render(
    app: &GodPdfApp,
    _front: &Entity<DocTab>,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    let split = state(cx).split.as_ref()?;
    let (source, mirror) = (split.source.upgrade(), split.mirror.clone());
    let Some(source) = source else {
        // Its document was closed: back to one viewer.
        state(cx).split = None;
        return None;
    };
    refresh(&source, &mirror, window, cx);
    let palette = Palette::of(cx);
    let doc = mirror.read(cx);
    let (title, count, current) = (
        doc.title().to_owned(),
        doc.page_count(),
        doc.view.current_page,
    );
    let names: Vec<(WeakEntity<DocTab>, String, bool)> = app
        .tabs
        .iter()
        .map(|tab| {
            let doc = tab.read(cx);
            (tab.downgrade(), doc.title().to_owned(), *tab == source)
        })
        .collect();
    let picker = Button::new("companion-document")
        .secondary()
        .h(px(theme::INTERACT_HEIGHT))
        .min_w(px(80.))
        .max_w(px(240.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(fonts::BODY))
                .child(
                    div()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(title),
                )
                .child(widgets::icon(icons::CARET_DOWN, 12., palette.text_dim)),
        )
        .dropdown_menu(move |mut menu, _, _| {
            for (tab, name, shown) in &names {
                let tab = tab.clone();
                menu = menu.item(
                    PopupMenuItem::new(name.clone())
                        .checked(*shown)
                        .on_click(move |_, _, cx| show(&tab, cx)),
                );
            }
            menu
        });
    let row = div()
        .h(px(34.))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(theme::ITEM_SPACING.0))
        .pl(px(10.))
        .pr(px(6.))
        .child(picker)
        .child(page_box(&mirror, current, count, &palette, cx))
        .child(
            div()
                .text_color(palette.text_dim)
                .child(format!("/ {count}")),
        )
        .child(div().flex_1())
        .child(
            widgets::small_icon_button(
                "close-second-view",
                icons::X,
                "Close the second view",
                &palette,
            )
            .on_click(|_, _, cx| {
                state(cx).split = None;
                cx.refresh_windows();
            }),
        );
    Some(
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(palette.border)
            .child(row)
            .child(div().flex_1().min_h_0().child(mirror))
            .into_any_element(),
    )
}

/// Copy the shown document again when it changed, and show the pending page once the
/// second viewer has its size.
fn refresh(source: &Entity<DocTab>, mirror: &Entity<DocTab>, window: &mut Window, cx: &mut App) {
    let generation = source.read(cx).generation();
    let Some(split) = state(cx).split.as_mut() else {
        return;
    };
    let stale = std::mem::replace(&mut split.generation, generation) != generation;
    let jump = split.jump;
    if stale {
        let doc = source.read(cx);
        let session =
            godpdf_core::Session::new(doc.title().to_owned(), doc.session.bytes().to_vec());
        mirror.update(cx, |mirror, cx| {
            mirror.session = session;
            mirror.reload();
            cx.notify();
        });
    }
    if let Some(page) = jump {
        let ready = mirror.update(cx, |mirror, cx| {
            let ready = mirror.view.bounds.size.width > px(0.) && !mirror.view.fit_pending;
            if ready {
                mirror.scroll_to_page(page);
                cx.notify();
            }
            ready
        });
        if ready {
            if let Some(split) = state(cx).split.as_mut() {
                split.jump = None;
            }
        } else {
            window.request_animation_frame();
        }
    }
}

/// The page shown on the right, 38 × 26: click to type a number,
/// Enter or leaving the field goes there.
fn page_box(
    mirror: &Entity<DocTab>,
    current: usize,
    count: usize,
    palette: &Palette,
    cx: &mut App,
) -> AnyElement {
    let size = |element: gpui::Div| element.w(px(38.)).h(px(theme::INTERACT_HEIGHT));
    if let Some((field, _)) = state(cx)
        .split
        .as_ref()
        .and_then(|split| split.field.as_ref())
    {
        return size(div())
            .child(Input::new(field).size_full())
            .into_any_element();
    }
    let shown = (current + 1).min(count.max(1));
    let hover = palette.hover;
    let mirror = mirror.downgrade();
    let element = size(div())
        .id("companion-page")
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme::RADIUS))
        .bg(palette.surface)
        .border_1()
        .border_color(palette.border)
        .hover(move |style| style.bg(hover))
        .cursor_text()
        .text_size(px(fonts::BODY))
        .child(shown.to_string())
        .on_click(move |_, window, cx| edit(&mirror, shown, window, cx));
    widgets::tooltip(element, "Page shown here").into_any_element()
}

fn edit(mirror: &WeakEntity<DocTab>, shown: usize, window: &mut Window, cx: &mut App) {
    let field = cx.new(|cx| InputState::new(window, cx).default_value(shown.to_string()));
    field.update(cx, |field, cx| {
        field.focus(window, cx);
        field.select_all(window, cx);
    });
    let mirror = mirror.clone();
    let subscription = cx.subscribe(&field, move |field, event: &InputEvent, cx| {
        if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
            let typed = field.read(cx).value().trim().parse::<usize>().ok();
            if let Some(split) = state(cx).split.as_mut() {
                split.field = None;
            }
            if let Some(page) = typed.filter(|&page| page >= 1) {
                let _ = mirror.update(cx, |mirror, cx| {
                    let last = mirror.page_count().saturating_sub(1);
                    mirror.scroll_to_page((page - 1).min(last));
                    cx.notify();
                });
            }
            cx.refresh_windows();
        }
    });
    if let Some(split) = state(cx).split.as_mut() {
        split.field = Some((field, subscription));
    }
    cx.refresh_windows();
}
