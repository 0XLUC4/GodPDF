//! Form fields filled in right on the page: a click on a text field opens a box to type in,
//! a click on a check box ticks it, a radio button or a list offers its options.

use egui::{Color32, Id, Key, Modifiers, Rect, Sense, Stroke, TextEdit, Ui, pos2};
use godpdf_core::{FieldKind, FieldValue};

use crate::doc::DocTab;
use crate::geometry;

/// A text field being typed in. `rect` is in displayed points.
pub struct FieldEdit {
    pub page: usize,
    pub name: String,
    pub rect: Rect,
    pub text: String,
    pub multiline: bool,
    focused: bool,
}

/// A value to write: page, field name, value.
pub type Fill = (usize, String, FieldValue);

const TINT: Color32 = Color32::from_rgba_premultiplied(30, 80, 200, 28);
const EDGE: Color32 = Color32::from_rgb(0x25, 0x63, 0xEB);

/// The fields of `page` shown at `page_rect`, ready to be clicked. Returns a value to write.
pub fn page(ui: &mut Ui, tab: &mut DocTab, page: usize, page_rect: Rect) -> Option<Fill> {
    let zoom = tab.zoom;
    let displayed = *tab.page_sizes.get(page)?;
    let geom = tab.page_geom(page).ok()?;
    let widgets = tab.field_widgets(page).to_vec();
    let mut fill = None;
    for (index, widget) in widgets.iter().enumerate() {
        if widget.field.read_only {
            continue;
        }
        let shown = geometry::from_pdf_rect(&geom, displayed, &widget.rect);
        let screen = Rect::from_min_max(
            page_rect.min + shown.min.to_vec2() * zoom,
            page_rect.min + shown.max.to_vec2() * zoom,
        );
        let editing = tab
            .field_edit
            .as_ref()
            .is_some_and(|edit| edit.page == page && edit.name == widget.field.name);
        if editing {
            continue;
        }
        let id = Id::new(("form-field", tab.id, page, index));
        let response = ui
            .interact(screen, id, Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(&widget.field.label);
        ui.painter().rect_filled(screen, 2.0, TINT);
        if response.hovered() {
            ui.painter().rect_stroke(
                screen,
                2.0,
                Stroke::new(1.0, EDGE),
                egui::StrokeKind::Outside,
            );
        }
        let name = widget.field.name.clone();
        match (&widget.field.kind, &widget.field.value) {
            (FieldKind::Text { multiline }, FieldValue::Text(text)) if response.clicked() => {
                tab.field_edit = Some(FieldEdit {
                    page,
                    name,
                    rect: shown,
                    text: text.clone(),
                    multiline: *multiline,
                    focused: false,
                });
            }
            (FieldKind::CheckBox, FieldValue::Checked(checked)) if response.clicked() => {
                fill = Some((page, name, FieldValue::Checked(!checked)));
            }
            (FieldKind::Radio { .. }, _) if response.clicked() => {
                if let Some(option) = &widget.option {
                    fill = Some((page, name, FieldValue::Choice(option.clone())));
                }
            }
            (FieldKind::Choice { options }, FieldValue::Choice(chosen)) => {
                egui::Popup::menu(&response).show(|ui| {
                    for option in options {
                        if ui.selectable_label(option == chosen, option).clicked() {
                            fill = Some((page, name.clone(), FieldValue::Choice(option.clone())));
                        }
                    }
                });
            }
            _ => {}
        }
    }
    fill
}

/// What the typing box says.
pub enum Event {
    Commit(Fill),
    Cancel,
}

/// The box to type a text field in, over the field.
pub fn show(ui: &mut Ui, edit: &mut FieldEdit, page_rect: Rect, zoom: f32) -> Option<Event> {
    let screen = Rect::from_min_max(
        page_rect.min + edit.rect.min.to_vec2() * zoom,
        page_rect.min + edit.rect.max.to_vec2() * zoom,
    );
    let font_size = if edit.multiline {
        10.0 * zoom
    } else {
        (edit.rect.height() * 0.6).clamp(6.0, 14.0) * zoom
    };
    let height = if edit.multiline {
        screen.height()
    } else {
        screen.height().max(font_size * 1.6)
    };
    let field = Rect::from_min_size(
        pos2(screen.left(), screen.center().y - height / 2.0),
        egui::vec2(screen.width().max(60.0), height),
    );
    ui.painter()
        .rect_filled(field.expand(1.0), 2.0, Color32::WHITE);
    ui.painter().rect_stroke(
        field.expand(1.0),
        2.0,
        Stroke::new(1.5, EDGE),
        egui::StrokeKind::Outside,
    );
    let id = Id::new(("form-field-edit", edit.page, &edit.name));
    let focused = ui.memory(|memory| memory.has_focus(id));
    let (enter, escape) = ui.input_mut(|input| {
        let enter = focused && !edit.multiline && input.consume_key(Modifiers::NONE, Key::Enter);
        (enter, input.key_pressed(Key::Escape))
    });
    let text_edit = if edit.multiline {
        TextEdit::multiline(&mut edit.text)
    } else {
        TextEdit::singleline(&mut edit.text)
    };
    let response = ui.put(
        field,
        text_edit
            .id(id)
            .frame(egui::Frame::NONE)
            .font(egui::FontId::proportional(font_size))
            .text_color(Color32::BLACK)
            .margin(egui::Margin::symmetric(3, 1))
            .desired_width(field.width()),
    );
    if !edit.focused {
        response.request_focus();
        edit.focused = true;
        return None;
    }
    if escape {
        return Some(Event::Cancel);
    }
    if enter || response.lost_focus() {
        let value = FieldValue::Text(edit.text.clone());
        return Some(Event::Commit((edit.page, edit.name.clone(), value)));
    }
    None
}
