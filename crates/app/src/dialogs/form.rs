//! A dialog made of labelled rows: text fields and choices (segmented buttons), an error
//! line and its buttons. What each one does is in `kinds.rs`.

use gpui::component::input::{Input, InputState};
use gpui::prelude::*;
use gpui::{AnyElement, App, Entity, SharedString, WeakEntity, Window, div, px};

use super::kinds::Kind;
use super::{DialogCancel, DialogConfirm, Open, card, confirm, state};
use crate::doc::DocTab;
use crate::theme::Palette;
use crate::widgets::{accent_button, framed_button};

const LABEL_WIDTH: f32 = 150.0;

pub enum Field {
    Text {
        label: &'static str,
        input: Entity<InputState>,
    },
    Choice {
        label: &'static str,
        options: Vec<SharedString>,
        selected: usize,
    },
}

impl Field {
    fn label(&self) -> &'static str {
        match self {
            Self::Text { label, .. } | Self::Choice { label, .. } => label,
        }
    }
}

pub fn text(
    label: &'static str,
    value: impl Into<SharedString>,
    placeholder: &'static str,
    window: &mut Window,
    cx: &mut App,
) -> Field {
    let value = value.into();
    let input = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(placeholder)
            .default_value(value)
    });
    Field::Text { label, input }
}

pub fn password(
    label: &'static str,
    placeholder: &'static str,
    window: &mut Window,
    cx: &mut App,
) -> Field {
    let input = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(placeholder)
            .masked(true)
    });
    Field::Text { label, input }
}

pub fn choice(
    label: &'static str,
    options: impl IntoIterator<Item = impl Into<SharedString>>,
    selected: usize,
) -> Field {
    Field::Choice {
        label,
        options: options.into_iter().map(Into::into).collect(),
        selected,
    }
}

pub struct Form {
    pub kind: Kind,
    pub tab: WeakEntity<DocTab>,
    pub fields: Vec<Field>,
    pub error: Option<String>,
}

impl Form {
    /// Text of the field `label` (empty when there is none).
    pub fn text(&self, label: &str, cx: &App) -> String {
        self.fields
            .iter()
            .find_map(|field| match field {
                Field::Text { label: name, input } if *name == label => {
                    Some(input.read(cx).value().to_string())
                }
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Chosen option of the choice `label` (0 when there is none).
    pub fn choice(&self, label: &str) -> usize {
        self.fields
            .iter()
            .find_map(|field| match field {
                Field::Choice {
                    label: name,
                    selected,
                    ..
                } if *name == label => Some(*selected),
                _ => None,
            })
            .unwrap_or(0)
    }

    pub fn first_input(&self) -> Option<Entity<InputState>> {
        self.fields.iter().find_map(|field| match field {
            Field::Text { input, .. } => Some(input.clone()),
            Field::Choice { .. } => None,
        })
    }
}

fn select(field: usize, option: usize, cx: &mut App) {
    if let Some(Open::Form(form)) = &mut state(cx).open
        && let Some(Field::Choice { selected, .. }) = form.fields.get_mut(field)
    {
        *selected = option;
    }
}

pub fn render(form: &Form, palette: &Palette) -> AnyElement {
    let palette = *palette;
    let (title, description) = form.kind.title();
    let (primary, secondary) = form.kind.buttons();
    let rows = form.fields.iter().enumerate().map(|(index, field)| {
        let value = match field {
            Field::Text { input, .. } => div().flex_1().child(Input::new(input)),
            Field::Choice {
                options, selected, ..
            } => div().flex_1().flex().flex_wrap().gap(px(4.)).children(
                options.iter().enumerate().map(|(option, name)| {
                    framed_button(
                        ("choice", index * 100 + option),
                        name.clone(),
                        true,
                        &palette,
                    )
                    .when(option == *selected, |chip| chip.bg(palette.selected))
                    .on_click(move |_, window, cx| {
                        select(index, option, cx);
                        window.refresh();
                    })
                }),
            ),
        };
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().w(px(LABEL_WIDTH)).flex_none().child(field.label()))
            .child(value)
    });
    card(&palette, 420.)
        .key_context("GodPdfDialog")
        .on_action(|_: &DialogConfirm, window, cx| confirm(false, window, cx))
        .on_action(|_: &DialogCancel, window, cx| super::close(window, cx))
        .child(super::title(title))
        .when(!description.is_empty(), |card| {
            card.child(div().text_color(palette.text_dim).child(description))
        })
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .pt(px(6.))
                .children(rows),
        )
        .children(
            form.error
                .clone()
                .map(|error| super::error_line(error, &palette)),
        )
        .child(
            super::button_row()
                .child(
                    accent_button("dialog-primary", primary, true, &palette)
                        .on_click(|_, window, cx| confirm(false, window, cx)),
                )
                .children(secondary.map(|label| {
                    framed_button("dialog-secondary", label, true, &palette)
                        .on_click(|_, window, cx| confirm(true, window, cx))
                }))
                .child(
                    framed_button("dialog-cancel", "Cancel", true, &palette)
                        .on_click(|_, window, cx| super::close(window, cx)),
                ),
        )
        .into_any_element()
}
