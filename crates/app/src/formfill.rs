//! The fields of a form in a list, to be filled in.

use egui::{RichText, ScrollArea, TextEdit, Ui};
use godpdf_core::{FieldKind, FieldValue, FormField};

use crate::theme::Palette;

fn input(ui: &mut Ui, index: usize, field: &FormField, value: &mut FieldValue) {
    match (&field.kind, value) {
        (FieldKind::Text { multiline }, FieldValue::Text(text)) => {
            let edit = if *multiline {
                TextEdit::multiline(text).desired_rows(3)
            } else {
                TextEdit::singleline(text)
            };
            ui.add(
                edit.margin(crate::widgets::FIELD_MARGIN)
                    .desired_width(260.0),
            );
        }
        (FieldKind::CheckBox, FieldValue::Checked(checked)) => {
            ui.checkbox(checked, "");
        }
        (
            FieldKind::Radio { options } | FieldKind::Choice { options },
            FieldValue::Choice(chosen),
        ) => {
            let shown = if chosen.is_empty() {
                "None".to_owned()
            } else {
                chosen.clone()
            };
            egui::ComboBox::from_id_salt(("field", index))
                .selected_text(shown)
                .width(260.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(chosen, String::new(), "None");
                    for option in options {
                        ui.selectable_value(chosen, option.clone(), option);
                    }
                });
        }
        _ => {
            ui.label("Cannot be filled here");
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

/// The form of a document revision and the answers being typed in the panel.
pub struct FormState {
    generation: u64,
    fields: Result<Vec<FormField>, String>,
    values: Vec<FieldValue>,
}

/// What the form panel asks the app to do.
#[derive(Default)]
pub struct PanelOut {
    /// Answers to write into the document.
    pub fill: Option<Vec<(String, FieldValue)>>,
    /// Draw a new field of this kind on the page.
    pub add: Option<godpdf_core::NewField>,
    pub jump: Option<usize>,
}

/// The form panel: buttons to add fields, then every field with its answer.
pub fn panel(ui: &mut Ui, tab: &mut crate::doc::DocTab) -> PanelOut {
    let palette = Palette::of(ui);
    let mut out = PanelOut::default();
    ui.label(RichText::new("Add a field: drag its box on the page.").color(palette.text_dim));
    ui.horizontal(|ui| {
        let text = crate::widgets::with_icon(crate::widgets::icon::TEXTBOX, "Text field");
        if ui.button(text).clicked() {
            out.add = Some(godpdf_core::NewField::Text);
        }
        let check = crate::widgets::with_icon(crate::widgets::icon::CHECK_SQUARE, "Check box");
        if ui.button(check).clicked() {
            out.add = Some(godpdf_core::NewField::CheckBox);
        }
    });
    ui.separator();

    let generation = tab.generation();
    if tab
        .form
        .as_ref()
        .is_none_or(|form| form.generation != generation)
    {
        let fields = crate::doc::guarded(|| godpdf_core::form_fields(tab.session.bytes()))
            .map_err(|err| err.to_string());
        let values = fields
            .as_ref()
            .map(|fields| fields.iter().map(|field| field.value.clone()).collect())
            .unwrap_or_default();
        tab.form = Some(FormState {
            generation,
            fields,
            values,
        });
    }
    let Some(form) = tab.form.as_mut() else {
        return out;
    };
    let fields = match &form.fields {
        Ok(fields) if fields.is_empty() => {
            ui.label(RichText::new("This document has no form fields.").color(palette.text_dim));
            return out;
        }
        Ok(fields) => fields,
        Err(err) => {
            ui.label(RichText::new(format!("Cannot read the form: {err}")).color(palette.error));
            return out;
        }
    };
    let changes = changes(fields, &form.values);
    ui.horizontal(|ui| {
        let apply = egui::Button::new(
            RichText::new(format!("Fill in ({})", changes.len())).color(palette.on_accent),
        )
        .fill(palette.accent);
        if ui.add_enabled(!changes.is_empty(), apply).clicked() {
            out.fill = Some(changes.clone());
        }
        if ui
            .add_enabled(!changes.is_empty(), egui::Button::new("Undo typing"))
            .clicked()
        {
            form.values = fields.iter().map(|field| field.value.clone()).collect();
        }
    });
    ui.add_space(6.0);
    ScrollArea::vertical()
        .id_salt(("form", tab.id))
        .auto_shrink(false)
        .show(ui, |ui| {
            for (index, (field, value)) in fields.iter().zip(form.values.iter_mut()).enumerate() {
                let label = match field.page {
                    Some(page) => format!("{}  ·  page {}", field.label, page + 1),
                    None => field.label.clone(),
                };
                let response = ui.add(
                    egui::Label::new(RichText::new(label).color(palette.text_dim))
                        .sense(egui::Sense::click()),
                );
                if response.on_hover_text("Show its page").clicked() {
                    out.jump = field.page;
                }
                ui.add_enabled_ui(!field.read_only, |ui| input(ui, index, field, value));
                ui.add_space(6.0);
            }
        });
    out
}
