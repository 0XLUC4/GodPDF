//! Small shared widgets: fixed-size icon buttons and the unsaved-changes dot.

use egui::{Button, Response, RichText, Sense, Ui, Vec2, vec2};

/// Phosphor icon glyphs (regular weight), installed as a fallback of the proportional font.
pub use egui_phosphor::regular as icon;

/// Side of a standard icon-only button.
pub const ICON_BUTTON: Vec2 = vec2(28.0, 28.0);
/// Side of a compact icon-only button (tab close, thumbnail arrows).
pub const SMALL_ICON_BUTTON: Vec2 = vec2(22.0, 22.0);

/// Frameless-until-hovered square button showing one glyph. Use with `ui.add_sized` / `ui.put`.
pub fn icon_widget(glyph: &str, size: f32) -> Button<'static> {
    Button::new(RichText::new(glyph.to_owned()).size(size))
        .frame_when_inactive(false)
        .stroke(egui::Stroke::NONE)
}

/// Standard 28×28 icon button with a tooltip.
pub fn icon_button(ui: &mut Ui, glyph: &str, tooltip: &str) -> Response {
    ui.add_sized(ICON_BUTTON, icon_widget(glyph, 16.0))
        .on_hover_text(tooltip)
}

/// Compact 22×22 icon button with a tooltip.
pub fn small_icon_button(ui: &mut Ui, glyph: &str, tooltip: &str) -> Response {
    ui.add_sized(SMALL_ICON_BUTTON, icon_widget(glyph, 13.0))
        .on_hover_text(tooltip)
}

/// Icon followed by a label, for menus and text buttons.
pub fn with_icon(glyph: &str, label: &str) -> String {
    format!("{glyph}  {label}")
}

/// Side of a rail button (right icon rail).
pub const RAIL_BUTTON: Vec2 = vec2(32.0, 32.0);

/// 32×32 icon toggle for the right rail; `selected` shows a light grey square.
pub fn rail_button(ui: &mut Ui, glyph: &str, tooltip: &str, selected: bool) -> Response {
    let text = RichText::new(glyph.to_owned()).size(18.0);
    ui.add_sized(
        RAIL_BUTTON,
        Button::selectable(selected, text)
            .frame_when_inactive(selected)
            .stroke(egui::Stroke::NONE),
    )
    .on_hover_text(tooltip)
}

/// Full-width list row: 20px icon in `tint` + label (+ dimmed description), hover background.
pub fn tool_row(
    ui: &mut Ui,
    glyph: &str,
    tint: egui::Color32,
    label: &str,
    description: Option<&str>,
) -> Response {
    let palette = crate::theme::Palette::of(ui);
    let height = if description.is_some() { 52.0 } else { 40.0 };
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, egui::CornerRadius::same(8), palette.hover);
    }
    let left = rect.min.x + 10.0;
    painter.text(
        egui::pos2(left + 10.0, rect.center().y),
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional(20.0),
        tint,
    );
    let text_x = left + 32.0;
    match description {
        Some(description) => {
            painter.text(
                egui::pos2(text_x, rect.center().y - 9.0),
                egui::Align2::LEFT_CENTER,
                label,
                egui::FontId::proportional(14.0),
                palette.text,
            );
            painter.text(
                egui::pos2(text_x, rect.center().y + 10.0),
                egui::Align2::LEFT_CENTER,
                description,
                egui::FontId::proportional(13.0),
                palette.text_dim,
            );
        }
        None => {
            painter.text(
                egui::pos2(text_x, rect.center().y),
                egui::Align2::LEFT_CENTER,
                label,
                egui::FontId::proportional(14.0),
                palette.text,
            );
        }
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Padding of text fields: as tall as a button, so the shared corner radius reads as a
/// rounded rectangle and not as a pill.
pub const FIELD_MARGIN: egui::Margin = egui::Margin::symmetric(8, 4);
