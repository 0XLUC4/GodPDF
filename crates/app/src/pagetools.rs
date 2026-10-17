//! Settings of the page tools that ask a few things first: crop, pages per sheet, page
//! size, halves, colours, export as pictures. One dialog shows any of them.

use egui::Ui;
use godpdf_core::{Color, Tint};

use crate::commands::Cmd;

/// Millimetres to points.
const MM: f32 = 72.0 / 25.4;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Paper {
    A3,
    A4,
    A5,
    Letter,
    Legal,
}

impl Paper {
    const ALL: [Paper; 5] = [Paper::A4, Paper::Letter, Paper::A3, Paper::A5, Paper::Legal];

    fn label(self) -> &'static str {
        match self {
            Self::A3 => "A3",
            Self::A4 => "A4",
            Self::A5 => "A5",
            Self::Letter => "Letter",
            Self::Legal => "Legal",
        }
    }

    /// Upright size in points.
    fn size(self) -> (f32, f32) {
        match self {
            Self::A3 => (842.0, 1191.0),
            Self::A4 => (595.0, 842.0),
            Self::A5 => (420.0, 595.0),
            Self::Letter => (612.0, 792.0),
            Self::Legal => (612.0, 1008.0),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Colours {
    Invert,
    Greyscale,
    Paper,
}

pub enum Form {
    /// Left, top, right, bottom in millimetres.
    Crop([f32; 4]),
    PerSheet(usize),
    PageSize {
        paper: Paper,
        landscape: bool,
    },
    Halve {
        across: bool,
    },
    Colours {
        kind: Colours,
        paper: Color,
    },
    Images {
        dpi: u32,
        jpeg: bool,
    },
}

impl Form {
    pub fn of(cmd: Cmd) -> Option<Self> {
        Some(match cmd {
            Cmd::CropPages => Self::Crop([10.0; 4]),
            Cmd::PagesPerSheet => Self::PerSheet(2),
            Cmd::PageSize => Self::PageSize {
                paper: Paper::A4,
                landscape: false,
            },
            Cmd::HalvePages => Self::Halve { across: true },
            Cmd::PageColours => Self::Colours {
                kind: Colours::Invert,
                paper: Color {
                    r: 1.0,
                    g: 0.96,
                    b: 0.84,
                },
            },
            Cmd::ExportImages => Self::Images {
                dpi: 150,
                jpeg: false,
            },
            _ => return None,
        })
    }

    pub fn cmd(&self) -> Cmd {
        match self {
            Self::Crop(_) => Cmd::CropPages,
            Self::PerSheet(_) => Cmd::PagesPerSheet,
            Self::PageSize { .. } => Cmd::PageSize,
            Self::Halve { .. } => Cmd::HalvePages,
            Self::Colours { .. } => Cmd::PageColours,
            Self::Images { .. } => Cmd::ExportImages,
        }
    }

    /// Works on chosen pages (else on the whole document).
    pub fn has_range(&self) -> bool {
        !matches!(self, Self::PerSheet(_))
    }

    pub fn button(&self) -> &'static str {
        match self {
            Self::Crop(_) => "Crop",
            Self::PerSheet(_) | Self::PageSize { .. } | Self::Halve { .. } => "Apply",
            Self::Colours { .. } => "Change colours",
            Self::Images { .. } => "Export",
        }
    }

    /// The fields of the tool, as rows of a two-column grid.
    pub fn ui(&mut self, ui: &mut Ui) {
        match self {
            Self::Crop(margins) => {
                for (value, label) in margins.iter_mut().zip(["Left", "Top", "Right", "Bottom"]) {
                    ui.label(label);
                    ui.add(
                        egui::DragValue::new(value)
                            .range(0.0..=500.0)
                            .speed(0.5)
                            .suffix(" mm"),
                    );
                    ui.end_row();
                }
            }
            Self::PerSheet(count) => {
                ui.label("Pages on a sheet");
                ui.horizontal(|ui| {
                    for choice in [2, 4, 6, 8, 9, 16] {
                        ui.selectable_value(count, choice, choice.to_string());
                    }
                });
                ui.end_row();
            }
            Self::PageSize { paper, landscape } => {
                ui.label("Size");
                ui.horizontal(|ui| {
                    for choice in Paper::ALL {
                        ui.selectable_value(paper, choice, choice.label());
                    }
                });
                ui.end_row();
                ui.label("Orientation");
                ui.horizontal(|ui| {
                    ui.selectable_value(landscape, false, "Portrait");
                    ui.selectable_value(landscape, true, "Landscape");
                });
                ui.end_row();
            }
            Self::Halve { across } => {
                ui.label("Cut");
                ui.horizontal(|ui| {
                    ui.selectable_value(across, true, "Left | right");
                    ui.selectable_value(across, false, "Top / bottom");
                });
                ui.end_row();
            }
            Self::Colours { kind, paper } => {
                ui.label("Colours");
                ui.horizontal(|ui| {
                    ui.selectable_value(kind, Colours::Invert, "Invert");
                    ui.selectable_value(kind, Colours::Greyscale, "Greyscale");
                    ui.selectable_value(kind, Colours::Paper, "Paper colour");
                });
                ui.end_row();
                if *kind == Colours::Paper {
                    ui.label("Paper");
                    let mut rgb = [paper.r, paper.g, paper.b];
                    if ui.color_edit_button_rgb(&mut rgb).changed() {
                        let [r, g, b] = rgb;
                        *paper = Color { r, g, b };
                    }
                    ui.end_row();
                }
            }
            Self::Images { dpi, jpeg } => {
                ui.label("Format");
                ui.horizontal(|ui| {
                    ui.selectable_value(jpeg, false, "PNG");
                    ui.selectable_value(jpeg, true, "JPEG");
                });
                ui.end_row();
                ui.label("Sharpness");
                ui.horizontal(|ui| {
                    for choice in [72, 150, 300] {
                        ui.selectable_value(dpi, choice, format!("{choice} dpi"));
                    }
                });
                ui.end_row();
            }
        }
    }

    /// The change to the document, for every tool but [`Form::Images`].
    pub fn edit(&self, pdf: &[u8], pages: &[usize]) -> godpdf_core::Result<Vec<u8>> {
        match *self {
            Self::Crop(margins) => godpdf_core::crop_pages(pdf, pages, margins.map(|mm| mm * MM)),
            Self::PerSheet(count) => godpdf_core::pages_per_sheet(pdf, count),
            Self::PageSize { paper, landscape } => {
                let (w, h) = paper.size();
                let (w, h) = if landscape { (h, w) } else { (w, h) };
                godpdf_core::resize_pages(pdf, pages, w, h)
            }
            Self::Halve { across } => godpdf_core::halve_pages(pdf, pages, across),
            Self::Colours { kind, paper } => {
                let tint = match kind {
                    Colours::Invert => Tint::Invert,
                    Colours::Greyscale => Tint::Greyscale,
                    Colours::Paper => Tint::Paper(paper),
                };
                godpdf_core::tint_pages(pdf, pages, tint)
            }
            Self::Images { .. } => Err(godpdf_core::Error::InvalidArgument(
                "pictures are not a change of the document".into(),
            )),
        }
    }

    pub fn done(&self) -> &'static str {
        match self {
            Self::Crop(_) => "Pages cropped",
            Self::PerSheet(_) => "Pages put together on sheets",
            Self::PageSize { .. } => "Pages resized",
            Self::Halve { .. } => "Pages cut in half",
            Self::Colours { .. } => "Colours changed",
            Self::Images { .. } => "Pages exported",
        }
    }
}

/// `pages` rendered at `dpi`, each as a PNG or JPEG named after `base` and its number.
pub fn render_images(
    pdf: Vec<u8>,
    pages: &[usize],
    dpi: u32,
    jpeg: bool,
    base: &str,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let doc = godpdf_render::Document::open(std::sync::Arc::new(pdf), None)
        .map_err(|err| err.to_string())?;
    let digits = doc.page_count().to_string().len();
    let extension = if jpeg { "jpg" } else { "png" };
    pages
        .iter()
        .map(|&page| {
            let bitmap = doc
                .render(page, dpi as f32 / 72.0)
                .map_err(|err| err.to_string())?;
            let image = image::RgbaImage::from_raw(bitmap.width, bitmap.height, bitmap.rgba)
                .ok_or("The page could not be drawn")?;
            let mut bytes = std::io::Cursor::new(Vec::new());
            let written = if jpeg {
                image::DynamicImage::ImageRgba8(image)
                    .to_rgb8()
                    .write_to(&mut bytes, image::ImageFormat::Jpeg)
            } else {
                image.write_to(&mut bytes, image::ImageFormat::Png)
            };
            written.map_err(|err| format!("Cannot write page {}: {err}", page + 1))?;
            let name = format!("{base} - page {:0digits$}.{extension}", page + 1);
            Ok((name, bytes.into_inner()))
        })
        .collect()
}

/// Pages on which nearly nothing is drawn: fewer than 1 pixel in 500 is not white.
pub fn blank_pages(pdf: Vec<u8>) -> Result<Vec<usize>, String> {
    let doc = godpdf_render::Document::open(std::sync::Arc::new(pdf), None)
        .map_err(|err| err.to_string())?;
    let mut blank = Vec::new();
    for page in 0..doc.page_count() {
        let bitmap = doc.thumbnail(page, 300).map_err(|err| err.to_string())?;
        let pixels = bitmap.rgba.chunks_exact(4);
        let total = pixels.len().max(1);
        let inked = pixels
            .filter(|px| px[0].min(px[1]).min(px[2]) < 225)
            .count();
        if inked * 500 < total {
            blank.push(page);
        }
    }
    Ok(blank)
}
