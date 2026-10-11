//! Start screen shown when no document is open. Calls nothing from core/render.

use std::path::PathBuf;

use egui::{
    Button, CornerRadius, Frame, Margin, RichText, ScrollArea, Sense, Stroke, TextureHandle, Ui,
    vec2,
};

use crate::commands::{self, Cmd, Group};
use crate::theme::Palette;
use crate::widgets::{icon, with_icon};

#[derive(Default)]
pub struct Output {
    pub cmd: Option<Cmd>,
    pub open_recent: Option<PathBuf>,
    /// Start a new document with one empty page.
    pub new_pdf: bool,
}

pub fn show(ui: &mut Ui, logo: Option<&TextureHandle>, recent: &[PathBuf]) -> Output {
    let palette = Palette::of(ui);
    let mut out = Output::default();
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.set_max_width(980.0);
            ui.add_space(36.0);
            if let Some(logo) = logo {
                ui.add(
                    egui::Image::new(logo)
                        .fit_to_exact_size(vec2(120.0, 120.0))
                        .corner_radius(CornerRadius::same(26)),
                );
            }
            ui.add_space(10.0);
            ui.label(
                RichText::new(crate::APP_NAME)
                    .size(40.0)
                    .strong()
                    .color(palette.text),
            );
            ui.label(
                RichText::new("Every PDF tool. Local. Pure Rust.")
                    .size(17.0)
                    .color(palette.accent_text),
            );
            let (bar, _) = ui.allocate_exact_size(vec2(56.0, 3.0), egui::Sense::hover());
            ui.painter()
                .rect_filled(bar, CornerRadius::same(2), palette.accent);
            ui.add_space(22.0);

            ui.horizontal(|ui| {
                // Center the three primary buttons.
                let width = 3.0 * 170.0 + 2.0 * ui.spacing().item_spacing.x;
                ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
                let open = Button::new(
                    RichText::new(with_icon(icon::FOLDER_OPEN, "Open PDF"))
                        .size(16.0)
                        .color(palette.on_accent),
                )
                .fill(palette.accent)
                .corner_radius(CornerRadius::same(10))
                .min_size(vec2(170.0, 44.0));
                if ui.add(open).clicked() {
                    out.cmd = Some(Cmd::Open);
                }
                let new =
                    Button::new(RichText::new(with_icon(icon::FILE_PLUS, "New PDF")).size(16.0))
                        .stroke(Stroke::new(1.0, palette.accent))
                        .corner_radius(CornerRadius::same(10))
                        .min_size(vec2(170.0, 44.0));
                if ui.add(new).clicked() {
                    out.new_pdf = true;
                }
                let merge =
                    Button::new(RichText::new(with_icon(icon::STACK, "Merge PDFs")).size(16.0))
                        .stroke(Stroke::new(1.0, palette.accent))
                        .corner_radius(CornerRadius::same(10))
                        .min_size(vec2(170.0, 44.0));
                if ui.add(merge).clicked() {
                    out.cmd = Some(Cmd::Merge);
                }
            });
            ui.label(
                RichText::new("…or drop PDF files anywhere on this window").color(palette.text_dim),
            );

            if !recent.is_empty() {
                ui.add_space(26.0);
                section_title(ui, "RECENT", &palette);
                for path in recent {
                    let name = path.file_name().map_or_else(
                        || path.display().to_string(),
                        |n| n.to_string_lossy().into_owned(),
                    );
                    let response = ui
                        .add(Button::new(with_icon(icon::FILE_PDF, &name)).frame(false))
                        .on_hover_text(path.display().to_string());
                    if response.clicked() {
                        out.open_recent = Some(path.clone());
                    }
                }
            }

            ui.add_space(30.0);
            section_title(ui, "TOOLS", &palette);
            ui.add_space(6.0);
            tool_grid(ui, &palette, &mut out);
            ui.add_space(30.0);
        });
    });
    out
}

fn section_title(ui: &mut Ui, title: &str, palette: &Palette) {
    ui.label(
        RichText::new(title)
            .small()
            .strong()
            .color(palette.accent_text),
    );
}

fn tool_grid(ui: &mut Ui, palette: &Palette, out: &mut Output) {
    let card_size = vec2(220.0, 84.0);
    let gap = 12.0;
    let columns = ((ui.available_width() + gap) / (card_size.x + gap))
        .floor()
        .max(1.0) as usize;
    let tools: Vec<Cmd> = Group::TOOLS
        .iter()
        .flat_map(|group| commands::in_group(*group))
        .filter(|cmd| !matches!(cmd, Cmd::RotateRight | Cmd::MoveEarlier | Cmd::MoveLater))
        .collect();
    for row in tools.chunks(columns) {
        ui.horizontal(|ui| {
            let used = row.len() as f32 * (card_size.x + gap) - gap;
            ui.add_space(((ui.available_width() - used) / 2.0).max(0.0));
            ui.spacing_mut().item_spacing.x = gap;
            for cmd in row {
                if card(ui, *cmd, card_size, palette) {
                    out.cmd = Some(*cmd);
                }
            }
        });
        ui.add_space(gap);
    }
}

fn card(ui: &mut Ui, cmd: Cmd, size: egui::Vec2, palette: &Palette) -> bool {
    let response = Frame::new()
        .fill(palette.surface)
        .stroke(Stroke::new(1.0, palette.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_min_size(size - vec2(24.0, 24.0));
            ui.set_max_size(size - vec2(24.0, 24.0));
            ui.horizontal(|ui| {
                ui.label(RichText::new(cmd.icon()).size(22.0).color(palette.accent));
                ui.vertical(|ui| {
                    ui.label(RichText::new(cmd.label().trim_end_matches('…')).strong());
                    ui.label(
                        RichText::new(cmd.description())
                            .small()
                            .color(palette.text_dim),
                    );
                });
            });
        })
        .response
        .interact(Sense::click());
    if response.hovered() {
        ui.painter().rect_stroke(
            response.rect,
            CornerRadius::same(12),
            Stroke::new(1.5, palette.accent),
            egui::StrokeKind::Inside,
        );
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}
