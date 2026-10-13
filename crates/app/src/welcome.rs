//! Start screen shown when no document is open. Calls nothing from core/render.

use std::path::PathBuf;

use egui::{Button, CornerRadius, RichText, ScrollArea, Stroke, TextureHandle, Ui, vec2};

use crate::commands::Cmd;
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
    // The logo, the buttons and the list sit in the middle of the view: how tall they are is
    // known from the frame before.
    let id = ui.id().with("welcome-height");
    let view = ui.available_height();
    let known: f32 = ui.data(|data| data.get_temp(id)).unwrap_or(0.0);
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.set_max_width(980.0);
            ui.add_space(((view - known) / 2.0).max(16.0));
            let top = ui.cursor().min.y;
            if let Some(logo) = logo {
                ui.add(
                    egui::Image::new(logo)
                        .fit_to_exact_size(vec2(112.0, 112.0))
                        .corner_radius(CornerRadius::same(24)),
                );
            }
            ui.add_space(10.0);
            ui.label(RichText::new(crate::APP_NAME).size(40.0).strong());
            let tagline = "Every PDF tool. Local. Pure Rust.";
            ui.label(RichText::new(tagline).size(17.0).color(palette.text_dim));
            ui.add_space(24.0);
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

            let height = ui.cursor().min.y - top;
            if (height - known).abs() > 0.5 {
                ui.data_mut(|data| data.insert_temp(id, height));
                ui.ctx().request_repaint();
            }
            ui.add_space(16.0);
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
