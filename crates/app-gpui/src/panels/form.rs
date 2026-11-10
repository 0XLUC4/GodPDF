//! The Form panel (ported from `crates/app/src/formfill.rs`): buttons to add fields, then
//! every field of the form with its answer, written in one undo step by "Fill in".

use godpdf_core::{FieldKind, FieldValue, FormField, NewField};
use gpui::component::Disableable;
use gpui::component::button::Button;
use gpui::component::checkbox::Checkbox;
use gpui::component::input::{Input, InputState, Textarea, TextareaState};
use gpui::component::menu::{DropdownMenu, PopupMenuItem};
use gpui::prelude::*;
use gpui::{AnyElement, App, Entity, SharedString, Window, div, px};

use crate::doc::{DocTab, guarded};
use crate::theme::Palette;
use crate::widgets::{accent_button, framed_button, with_icon};
use crate::{fonts, icons, toasts, tools};

const INPUT_WIDTH: f32 = 260.0;

/// The form of a document revision and the answers being typed in the panel.
pub struct FormState {
    generation: Option<u64>,
    fields: Result<Vec<FormField>, String>,
    /// Answers of the check boxes, radio groups and lists (text answers are in `inputs`).
    values: Vec<FieldValue>,
    inputs: Vec<Option<Typed>>,
}

impl Default for FormState {
    fn default() -> Self {
        Self {
            generation: None,
            fields: Ok(Vec::new()),
            values: Vec::new(),
            inputs: Vec::new(),
        }
    }
}

#[derive(Clone)]
enum Typed {
    Line(Entity<InputState>),
    Area(Entity<TextareaState>),
}

impl Typed {
    fn value(&self, cx: &App) -> String {
        match self {
            Self::Line(state) => state.read(cx).value().to_string(),
            Self::Area(state) => state.read(cx).value().to_string(),
        }
    }

    fn set(&self, text: &str, window: &mut Window, cx: &mut App) {
        let text = text.to_owned();
        match self {
            Self::Line(state) => state.update(cx, |state, cx| state.set_value(text, window, cx)),
            Self::Area(state) => state.update(cx, |state, cx| state.set_value(text, window, cx)),
        }
    }
}

/// The fields whose value was changed, with their new value.
pub fn changes(fields: &[FormField], values: &[FieldValue]) -> Vec<(String, FieldValue)> {
    fields
        .iter()
        .zip(values)
        .filter(|(field, value)| field.value != **value)
        .map(|(field, value)| (field.name.clone(), value.clone()))
        .collect()
}

/// The answers as they are now: typed text read from the inputs.
fn current(form: &FormState, cx: &App) -> Vec<FieldValue> {
    form.values
        .iter()
        .zip(&form.inputs)
        .map(|(value, input)| match input {
            Some(input) => FieldValue::Text(input.value(cx)),
            None => value.clone(),
        })
        .collect()
}

/// Read the form again when the document changed.
fn refresh(tab: &Entity<DocTab>, window: &mut Window, cx: &mut App) {
    let doc = tab.read(cx);
    let generation = doc.generation();
    if doc.panels.form.generation == Some(generation) {
        return;
    }
    let fields =
        guarded(|| godpdf_core::form_fields(doc.session.bytes())).map_err(|e| e.to_string());
    let list = fields.as_deref().unwrap_or_default();
    let values: Vec<FieldValue> = list.iter().map(|field| field.value.clone()).collect();
    let inputs = list
        .iter()
        .map(|field| match (&field.kind, &field.value) {
            (FieldKind::Text { multiline: true }, FieldValue::Text(text)) => {
                Some(Typed::Area(cx.new(|cx| {
                    TextareaState::new(window, cx).default_value(text.clone())
                })))
            }
            (FieldKind::Text { .. }, FieldValue::Text(text)) => {
                Some(Typed::Line(cx.new(|cx| {
                    InputState::new(window, cx).default_value(text.clone())
                })))
            }
            _ => None,
        })
        .collect();
    tab.update(cx, |tab, _| {
        tab.panels.form = FormState {
            generation: Some(generation),
            fields,
            values,
            inputs,
        };
    });
}

fn set_value(tab: &Entity<DocTab>, index: usize, value: FieldValue, cx: &mut App) {
    tab.update(cx, |tab, cx| {
        if let Some(slot) = tab.panels.form.values.get_mut(index) {
            *slot = value;
        }
        cx.notify();
    });
}

fn add_field(kind: NewField, window: &mut Window, cx: &mut App) {
    cx.default_global::<tools::ToolState>().field_kind = kind;
    window.dispatch_action(Box::new(tools::FieldTool), cx);
    toasts::info(cx, "Drag the box of the field on the page");
}

/// The answer of field `index`: a text box, a check box or a list of options.
fn input(
    tab: &Entity<DocTab>,
    index: usize,
    field: &FormField,
    value: &FieldValue,
    typed: Option<&Typed>,
    palette: &Palette,
) -> AnyElement {
    let enabled = !field.read_only;
    let options = match &field.kind {
        FieldKind::Radio { options } | FieldKind::Choice { options } => Some(options.clone()),
        _ => None,
    };
    match (typed, value, options) {
        (Some(Typed::Line(state)), ..) => div()
            .w(px(INPUT_WIDTH))
            .child(Input::new(state).disabled(!enabled))
            .into_any_element(),
        (Some(Typed::Area(state)), ..) => div()
            .w(px(INPUT_WIDTH))
            .child(Textarea::new(state).h(px(66.)).disabled(!enabled))
            .into_any_element(),
        (None, FieldValue::Checked(checked), _) => {
            let tab = tab.clone();
            Checkbox::new(("form-check", index))
                .checked(*checked)
                .disabled(!enabled)
                .on_click(move |checked: &bool, _, cx| {
                    set_value(&tab, index, FieldValue::Checked(*checked), cx)
                })
                .into_any_element()
        }
        (None, FieldValue::Choice(chosen), Some(options)) => {
            let shown = if chosen.is_empty() {
                "None"
            } else {
                chosen.as_str()
            };
            let (tab, chosen) = (tab.clone(), chosen.clone());
            Button::new(("form-choice", index))
                .w(px(INPUT_WIDTH))
                .h(px(crate::theme::INTERACT_HEIGHT))
                .text_color(palette.text)
                .disabled(!enabled)
                .child(div().flex_1().text_left().child(shown.to_owned()))
                .child(crate::widgets::icon(
                    icons::CARET_DOWN,
                    13.0,
                    palette.text_dim,
                ))
                .dropdown_menu(move |mut menu, _, _| {
                    let none = std::iter::once((String::new(), "None".to_owned()));
                    let all = none.chain(
                        options
                            .iter()
                            .map(|option| (option.clone(), option.clone())),
                    );
                    for (option, name) in all {
                        let tab = tab.clone();
                        menu =
                            menu.item(PopupMenuItem::new(name).checked(option == chosen).on_click(
                                move |_, _, cx| {
                                    set_value(&tab, index, FieldValue::Choice(option.clone()), cx)
                                },
                            ));
                    }
                    menu
                })
                .into_any_element()
        }
        _ => super::label("Cannot be filled here", palette.text_dim).into_any_element(),
    }
}

pub fn render(tab: &Entity<DocTab>, window: &mut Window, cx: &mut App) -> AnyElement {
    let palette = Palette::of(cx);
    let add = div()
        .flex()
        .gap(px(8.))
        .child(
            framed_button(
                "form-add-text",
                with_icon(icons::TEXTBOX, "Text field", fonts::BODY, palette.text),
                true,
                &palette,
            )
            .on_click(|_, window, cx| add_field(NewField::Text, window, cx)),
        )
        .child(
            framed_button(
                "form-add-check",
                with_icon(icons::CHECK_SQUARE, "Check box", fonts::BODY, palette.text),
                true,
                &palette,
            )
            .on_click(|_, window, cx| add_field(NewField::CheckBox, window, cx)),
        );
    let head = div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(super::label(
            "Add a field: drag its box on the page.",
            palette.text_dim,
        ))
        .child(add)
        .child(div().my(px(2.)).h(px(1.)).bg(palette.border));
    refresh(tab, window, cx);
    let form = &tab.read(cx).panels.form;
    let fields = match &form.fields {
        Ok(fields) if fields.is_empty() => {
            return head
                .child(super::label(
                    "This document has no form fields.",
                    palette.text_dim,
                ))
                .into_any_element();
        }
        Ok(fields) => fields.clone(),
        Err(err) => {
            return head
                .child(super::label(
                    format!("Cannot read the form: {err}"),
                    palette.error,
                ))
                .into_any_element();
        }
    };
    let values = current(form, cx);
    let inputs = form.inputs.clone();
    let pending = changes(&fields, &values);
    let count = pending.len();
    let fill_tab = tab.clone();
    let undo_tab = tab.clone();
    let buttons = div()
        .flex()
        .gap(px(8.))
        .child(
            accent_button(
                "form-fill",
                format!("Fill in ({count})"),
                count > 0,
                &palette,
            )
            .when(count > 0, |button| {
                button.on_click(move |_, _, cx| {
                    let result = fill_tab.update(cx, |tab, cx| {
                        let result =
                            tab.apply("Fill in form", |pdf| godpdf_core::fill_form(pdf, &pending));
                        cx.notify();
                        result
                    });
                    match result {
                        Ok(()) => toasts::success(cx, "Form filled in"),
                        Err(err) => toasts::error(cx, err),
                    }
                })
            }),
        )
        .child(
            framed_button("form-undo-typing", "Undo typing", count > 0, &palette).when(
                count > 0,
                |button| {
                    button.on_click(move |_, window, cx| {
                        let form = &undo_tab.read(cx).panels.form;
                        let original: Vec<FormField> =
                            form.fields.as_deref().unwrap_or_default().to_vec();
                        let inputs = form.inputs.clone();
                        for (field, input) in original.iter().zip(&inputs) {
                            if let (Some(input), FieldValue::Text(text)) = (input, &field.value) {
                                input.set(text, window, cx);
                            }
                        }
                        undo_tab.update(cx, |tab, cx| {
                            tab.panels.form.values =
                                original.iter().map(|field| field.value.clone()).collect();
                            cx.notify();
                        });
                    })
                },
            ),
        );
    let rows = fields.iter().enumerate().map(|(index, field)| {
        let label: SharedString = match field.page {
            Some(page) => format!("{}  ·  page {}", field.label, page + 1),
            None => field.label.clone(),
        }
        .into();
        let page = field.page;
        let jump_tab = tab.clone();
        let title = div()
            .id(("form-label", index))
            .text_color(palette.text_dim)
            .cursor_pointer()
            .child(label)
            .on_click(move |_, _, cx| {
                if let Some(page) = page {
                    jump_tab.update(cx, |tab, cx| {
                        tab.scroll_to_page(page);
                        cx.notify();
                    });
                }
            });
        div()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(4.))
            .pb(px(6.))
            .child(crate::widgets::tooltip(title, "Show its page"))
            .child(input(
                tab,
                index,
                field,
                &values[index],
                inputs[index].as_ref(),
                &palette,
            ))
    });
    head.flex_1()
        .min_h_0()
        .child(buttons)
        .child(
            div()
                .id("form-fields")
                .mt(px(6.))
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .children(rows),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_changed_answers_are_written() {
        let field = |name: &str, value| FormField {
            name: name.to_owned(),
            label: name.to_owned(),
            kind: FieldKind::CheckBox,
            value,
            page: Some(0),
            read_only: false,
        };
        let fields = [
            field("a", FieldValue::Checked(false)),
            field("b", FieldValue::Checked(true)),
        ];
        let values = [FieldValue::Checked(true), FieldValue::Checked(true)];
        assert_eq!(
            changes(&fields, &values),
            [("a".to_owned(), FieldValue::Checked(true))]
        );
    }
}
