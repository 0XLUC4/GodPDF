//! Modal dialogs, command palette and the extracted-text window.
//!
//! Dialogs only edit their own fields and report an [`Outcome`]; the app performs the work.

use egui::{
    Button, Context, FontId, Id, Key, Modal, Response, RichText, ScrollArea, Sense, TextEdit,
    TextureHandle, Ui, pos2, vec2,
};

use crate::commands::Cmd;
use crate::platform::PickedFile;
use crate::theme::Palette;
use crate::widgets::{self, icon, with_icon};

pub enum Dialog {
    Password {
        file: PickedFile,
        password: String,
        error: Option<String>,
    },
    ConfirmClose {
        tab: u64,
    },
    ConfirmQuit,
    Merge {
        files: Vec<PickedFile>,
    },
    Extract {
        tab: u64,
        range: String,
        error: Option<String>,
    },
    Split {
        tab: u64,
        every: bool,
        n: usize,
        starts: String,
        error: Option<String>,
    },
    Encrypt {
        tab: u64,
        user: String,
        owner: String,
        confirm: String,
        error: Option<String>,
    },
    Properties {
        tab: u64,
        fields: [String; 6],
    },
    /// Sticky note text.
    CommentText {
        tab: u64,
        page: usize,
        rect: godpdf_core::Rect,
        text: String,
    },
    /// Page number typed by the user, of the `count` pages of the tab.
    GoToPage {
        tab: u64,
        page: String,
        count: usize,
        error: Option<String>,
    },
    /// Numbers to write on the pages of `range`.
    NumberPages {
        tab: u64,
        numbers: godpdf_core::PageNumbers,
        range: String,
        error: Option<String>,
    },
    /// A line of text to draw across the pages of `range`.
    Watermark {
        tab: u64,
        mark: godpdf_core::Watermark,
        range: String,
        error: Option<String>,
    },
    /// A page tool and the pages of `range` it works on.
    Tool {
        tab: u64,
        form: crate::pagetools::Form,
        range: String,
        error: Option<String>,
    },
    /// Files attached to the document; `action` is what the user just asked for.
    Attachments {
        tab: u64,
        files: Vec<godpdf_core::Attachment>,
        action: Option<AttachAction>,
    },
    /// A PDF whose pages go in turn with those of the tab.
    Interleave {
        tab: u64,
        file: PickedFile,
        reverse: bool,
    },
    /// The signature pad. `place` is where it goes once drawn; `saved` when one is kept.
    Signature {
        strokes: Vec<Vec<egui::Pos2>>,
        place: Option<(u64, usize, (f32, f32))>,
        saved: bool,
    },
    /// A field to add over `rect` of `page`.
    NewField {
        tab: u64,
        page: usize,
        rect: godpdf_core::Rect,
        kind: godpdf_core::NewField,
        name: String,
        error: Option<String>,
    },
    About,
    Updates,
    Preferences,
}

#[derive(Clone, Copy)]
pub enum AttachAction {
    Add,
    Save(usize),
    Remove(usize),
}

pub const META_LABELS: [&str; 6] = [
    "Title", "Author", "Subject", "Keywords", "Creator", "Producer",
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Keep,
    Cancel,
    Primary,
    Secondary,
}

pub struct Env<'a> {
    pub logo: Option<&'a TextureHandle>,
    pub dark: &'a mut bool,
    pub updates: &'a crate::updates::Checker,
}

pub fn show(ctx: &Context, dialog: &mut Dialog, env: Env<'_>) -> Outcome {
    let mut outcome = Outcome::Keep;
    let response = Modal::new(Id::new("dialog")).show(ctx, |ui| {
        ui.set_width(420.0);
        outcome = body(ui, dialog, env);
    });
    if outcome == Outcome::Keep && response.should_close() {
        Outcome::Cancel
    } else {
        outcome
    }
}

fn body(ui: &mut Ui, dialog: &mut Dialog, env: Env<'_>) -> Outcome {
    let palette = Palette::of(ui);
    let enter = ui.input(|input| input.key_pressed(Key::Enter));
    match dialog {
        Dialog::Password {
            file,
            password,
            error,
        } => {
            title(ui, &with_icon(icon::LOCK, "Password required"));
            ui.label(format!("“{}” is encrypted.", file.name));
            let edit = ui.add(
                TextEdit::singleline(password)
                    .margin(crate::widgets::FIELD_MARGIN)
                    .password(true)
                    .hint_text("Password")
                    .desired_width(f32::INFINITY),
            );
            edit.request_focus();
            error_line(ui, error.as_deref(), &palette);
            buttons(ui, "Unlock", Some("Cancel"), enter).map_or(Outcome::Keep, |primary| {
                if primary {
                    Outcome::Primary
                } else {
                    Outcome::Cancel
                }
            })
        }
        Dialog::ConfirmClose { .. } | Dialog::ConfirmQuit => {
            title(ui, "Unsaved changes");
            let quitting = matches!(dialog, Dialog::ConfirmQuit);
            ui.label(if quitting {
                "Some documents have unsaved changes. Quit anyway?"
            } else {
                "This document has unsaved changes."
            });
            ui.add_space(12.0);
            let mut outcome = Outcome::Keep;
            ui.horizontal(|ui| {
                if !quitting && ui.add(primary_button(ui, "Save")).clicked() {
                    outcome = Outcome::Primary;
                }
                if ui
                    .button(if quitting {
                        "Quit without saving"
                    } else {
                        "Discard"
                    })
                    .clicked()
                {
                    outcome = Outcome::Secondary;
                }
                if ui.button("Cancel").clicked() {
                    outcome = Outcome::Cancel;
                }
            });
            outcome
        }
        Dialog::Merge { files } => merge(ui, files, &palette),
        Dialog::Extract { range, error, .. } => {
            title(ui, &with_icon(icon::EXPORT, "Extract pages"));
            ui.label("Pages (e.g. 1-3, 5, 8-)");
            ui.add(
                TextEdit::singleline(range)
                    .margin(crate::widgets::FIELD_MARGIN)
                    .desired_width(f32::INFINITY),
            );
            error_line(ui, error.as_deref(), &palette);
            ui.add_space(12.0);
            let mut outcome = Outcome::Keep;
            ui.horizontal(|ui| {
                if ui.add(primary_button(ui, "Open in new tab")).clicked() {
                    outcome = Outcome::Primary;
                }
                if ui.button("Save as…").clicked() {
                    outcome = Outcome::Secondary;
                }
                if ui.button("Cancel").clicked() {
                    outcome = Outcome::Cancel;
                }
            });
            outcome
        }
        Dialog::Split {
            every,
            n,
            starts,
            error,
            ..
        } => {
            title(ui, &with_icon(icon::SCISSORS, "Split document"));
            ui.radio_value(every, true, "Every N pages");
            ui.add_enabled(
                *every,
                egui::DragValue::new(n).range(1..=10_000).prefix("N = "),
            );
            ui.radio_value(every, false, "Start a new part at pages");
            ui.add_enabled(
                !*every,
                TextEdit::singleline(starts)
                    .margin(crate::widgets::FIELD_MARGIN)
                    .hint_text("e.g. 4, 9")
                    .desired_width(f32::INFINITY),
            );
            error_line(ui, error.as_deref(), &palette);
            ui.add_space(12.0);
            let mut outcome = Outcome::Keep;
            ui.horizontal(|ui| {
                let label = if cfg!(target_arch = "wasm32") {
                    "Download parts"
                } else {
                    "Save to folder…"
                };
                if ui.add(primary_button(ui, label)).clicked() {
                    outcome = Outcome::Primary;
                }
                if ui.button("Open in new tabs").clicked() {
                    outcome = Outcome::Secondary;
                }
                if ui.button("Cancel").clicked() {
                    outcome = Outcome::Cancel;
                }
            });
            outcome
        }
        Dialog::Encrypt {
            user,
            owner,
            confirm,
            error,
            ..
        } => {
            title(ui, &with_icon(icon::LOCK, "Encrypt with password"));
            ui.label(RichText::new("Applied when you save (AES-256).").color(palette.text_dim));
            egui::Grid::new("encrypt-grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Open password");
                    ui.add(
                        TextEdit::singleline(user)
                            .margin(crate::widgets::FIELD_MARGIN)
                            .password(true),
                    );
                    ui.end_row();
                    ui.label("Confirm");
                    ui.add(
                        TextEdit::singleline(confirm)
                            .margin(crate::widgets::FIELD_MARGIN)
                            .password(true),
                    );
                    ui.end_row();
                    ui.label("Owner password");
                    ui.add(
                        TextEdit::singleline(owner)
                            .margin(crate::widgets::FIELD_MARGIN)
                            .password(true)
                            .hint_text("optional"),
                    );
                    ui.end_row();
                });
            error_line(ui, error.as_deref(), &palette);
            buttons(ui, "Encrypt on save", Some("Cancel"), enter).map_or(Outcome::Keep, |primary| {
                if primary {
                    Outcome::Primary
                } else {
                    Outcome::Cancel
                }
            })
        }
        Dialog::Properties { fields, .. } => {
            title(ui, &with_icon(icon::INFO, "Document properties"));
            egui::Grid::new("meta-grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    for (label, value) in META_LABELS.iter().zip(fields.iter_mut()) {
                        ui.label(*label);
                        ui.add(
                            TextEdit::singleline(value)
                                .margin(crate::widgets::FIELD_MARGIN)
                                .desired_width(300.0),
                        );
                        ui.end_row();
                    }
                });
            buttons(ui, "Apply", Some("Cancel"), false).map_or(Outcome::Keep, |primary| {
                if primary {
                    Outcome::Primary
                } else {
                    Outcome::Cancel
                }
            })
        }
        Dialog::CommentText { text, .. } => {
            title(ui, &with_icon(icon::NOTE, "Sticky note"));
            let edit = ui.add(
                TextEdit::multiline(text)
                    .margin(crate::widgets::FIELD_MARGIN)
                    .desired_rows(4)
                    .desired_width(f32::INFINITY),
            );
            if text.is_empty() {
                edit.request_focus();
            }
            buttons(ui, "Add", Some("Cancel"), false).map_or(Outcome::Keep, |primary| {
                if primary {
                    Outcome::Primary
                } else {
                    Outcome::Cancel
                }
            })
        }
        Dialog::GoToPage {
            page, count, error, ..
        } => {
            title(ui, &with_icon(icon::HASH, "Go to page"));
            ui.label(format!("Page number, from 1 to {count}"));
            let edit = ui.add(
                TextEdit::singleline(page)
                    .margin(crate::widgets::FIELD_MARGIN)
                    .desired_width(f32::INFINITY),
            );
            edit.request_focus();
            error_line(ui, error.as_deref(), &palette);
            buttons(ui, "Go", Some("Cancel"), enter).map_or(Outcome::Keep, |primary| {
                if primary {
                    Outcome::Primary
                } else {
                    Outcome::Cancel
                }
            })
        }
        Dialog::NumberPages {
            numbers,
            range,
            error,
            ..
        } => {
            title(ui, &with_icon(icon::LIST_NUMBERS, "Number pages"));
            egui::Grid::new("numbers-grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Position");
                    egui::ComboBox::from_id_salt("numbers-place")
                        .selected_text(numbers.place.label())
                        .show_ui(ui, |ui| {
                            for place in godpdf_core::Place::ALL {
                                ui.selectable_value(&mut numbers.place, place, place.label());
                            }
                        });
                    ui.end_row();
                    ui.label("Format");
                    egui::ComboBox::from_id_salt("numbers-format")
                        .selected_text(number_sample(&numbers.template))
                        .show_ui(ui, |ui| {
                            for template in NUMBER_FORMATS {
                                let chosen = numbers.template == template;
                                if ui
                                    .selectable_label(chosen, number_sample(template))
                                    .clicked()
                                {
                                    numbers.template = template.to_owned();
                                }
                            }
                        });
                    ui.end_row();
                    ui.label("First number");
                    ui.add(egui::DragValue::new(&mut numbers.first).range(0..=99_999));
                    ui.end_row();
                    ui.label("Size");
                    ui.add(
                        egui::DragValue::new(&mut numbers.font_size)
                            .range(6.0..=72.0)
                            .suffix(" pt"),
                    );
                    ui.end_row();
                    ui.label("Colour");
                    color_button(ui, &mut numbers.color);
                    ui.end_row();
                    ui.label("Pages");
                    range_field(ui, range);
                    ui.end_row();
                });
            error_line(ui, error.as_deref(), &palette);
            buttons(ui, "Number pages", Some("Cancel"), enter).map_or(Outcome::Keep, |primary| {
                if primary {
                    Outcome::Primary
                } else {
                    Outcome::Cancel
                }
            })
        }
        Dialog::Watermark {
            mark, range, error, ..
        } => {
            title(ui, &with_icon(icon::STAMP, "Add watermark"));
            egui::Grid::new("watermark-grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Text");
                    let edit = ui.add(
                        TextEdit::singleline(&mut mark.text)
                            .margin(crate::widgets::FIELD_MARGIN)
                            .hint_text("e.g. DRAFT")
                            .desired_width(280.0),
                    );
                    if mark.text.is_empty() {
                        edit.request_focus();
                    }
                    ui.end_row();
                    ui.label("Size");
                    ui.add(
                        egui::DragValue::new(&mut mark.font_size)
                            .range(8.0..=300.0)
                            .suffix(" pt"),
                    );
                    ui.end_row();
                    ui.label("Opacity");
                    ui.add(
                        egui::Slider::new(&mut mark.opacity, 0.05..=1.0)
                            .custom_formatter(|value, _| format!("{:.0} %", value * 100.0)),
                    );
                    ui.end_row();
                    ui.label("Angle");
                    ui.add(egui::Slider::new(&mut mark.angle, -90.0..=90.0).suffix("°"));
                    ui.end_row();
                    ui.label("Colour");
                    color_button(ui, &mut mark.color);
                    ui.end_row();
                    ui.label("From the centre");
                    ui.horizontal(|ui| {
                        let offset = |value| {
                            egui::DragValue::new(value)
                                .range(-2000.0..=2000.0)
                                .suffix(" pt")
                        };
                        ui.add(offset(&mut mark.offset.0).prefix("right "));
                        ui.add(offset(&mut mark.offset.1).prefix("up "));
                    });
                    ui.end_row();
                    ui.label("Pages");
                    range_field(ui, range);
                    ui.end_row();
                });
            error_line(ui, error.as_deref(), &palette);
            buttons(ui, "Add watermark", Some("Cancel"), enter).map_or(Outcome::Keep, |primary| {
                if primary {
                    Outcome::Primary
                } else {
                    Outcome::Cancel
                }
            })
        }
        Dialog::Tool {
            form, range, error, ..
        } => {
            let cmd = form.cmd();
            title(
                ui,
                &with_icon(cmd.icon(), cmd.label().trim_end_matches('…')),
            );
            ui.label(RichText::new(cmd.description()).color(palette.text_dim));
            ui.add_space(6.0);
            egui::Grid::new("tool-grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    form.ui(ui);
                    if form.has_range() {
                        ui.label("Pages");
                        range_field(ui, range);
                        ui.end_row();
                    }
                });
            error_line(ui, error.as_deref(), &palette);
            buttons(ui, form.button(), Some("Cancel"), enter).map_or(Outcome::Keep, |primary| {
                if primary {
                    Outcome::Primary
                } else {
                    Outcome::Cancel
                }
            })
        }
        Dialog::Attachments { files, action, .. } => {
            title(ui, &with_icon(icon::PAPERCLIP, "Attached files"));
            if files.is_empty() {
                ui.label(
                    RichText::new("No file is attached to this document.").color(palette.text_dim),
                );
            }
            ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                for (index, file) in files.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(with_icon(icon::FILE, &file.name));
                        ui.label(
                            RichText::new(size_text(file.bytes.len())).color(palette.text_dim),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::small_icon_button(ui, icon::TRASH, "Remove").clicked() {
                                *action = Some(AttachAction::Remove(index));
                            }
                            if widgets::small_icon_button(ui, icon::DOWNLOAD_SIMPLE, "Save a copy")
                                .clicked()
                            {
                                *action = Some(AttachAction::Save(index));
                            }
                        });
                    });
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .add(primary_button(ui, &with_icon(icon::PLUS, "Attach files…")))
                    .clicked()
                {
                    *action = Some(AttachAction::Add);
                }
                if ui.button("Close").clicked() {
                    return Outcome::Cancel;
                }
                if action.is_some() {
                    Outcome::Primary
                } else {
                    Outcome::Keep
                }
            })
            .inner
        }
        Dialog::Interleave { file, reverse, .. } => {
            title(ui, &with_icon(icon::SHUFFLE, "Interleave pages"));
            ui.label(format!(
                "The pages of this document and of “{}”, taken in turn.",
                file.name
            ));
            ui.add_space(6.0);
            ui.checkbox(reverse, "Read the second one from its last page")
                .on_hover_text(
                    "For the backs of sheets scanned as a pile, which come out in reverse",
                );
            buttons(ui, "Interleave", Some("Cancel"), enter).map_or(Outcome::Keep, |primary| {
                if primary {
                    Outcome::Primary
                } else {
                    Outcome::Cancel
                }
            })
        }
        Dialog::Signature { strokes, saved, .. } => {
            title(ui, &with_icon(icon::SIGNATURE, "Your signature"));
            ui.label(
                RichText::new("Draw it with the mouse or a pen. It is kept for the next time.")
                    .color(palette.text_dim),
            );
            ui.add_space(6.0);
            crate::signature::pad(ui, strokes);
            let drawn = strokes.iter().any(|stroke| !stroke.is_empty());
            ui.add_space(8.0);
            let mut outcome = Outcome::Keep;
            ui.horizontal(|ui| {
                let label = if drawn || !*saved {
                    "Use this signature"
                } else {
                    "Use my saved signature"
                };
                let ready = drawn || *saved;
                if ui.add_enabled(ready, primary_button(ui, label)).clicked() {
                    outcome = Outcome::Primary;
                }
                if ui.add_enabled(drawn, Button::new("Clear")).clicked() {
                    strokes.clear();
                }
                if ui.button("Cancel").clicked() {
                    outcome = Outcome::Cancel;
                }
            });
            outcome
        }
        Dialog::NewField {
            kind, name, error, ..
        } => {
            title(ui, &with_icon(icon::TEXTBOX, "New form field"));
            egui::Grid::new("field-grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Kind");
                    ui.horizontal(|ui| {
                        ui.selectable_value(kind, godpdf_core::NewField::Text, "Text");
                        ui.selectable_value(kind, godpdf_core::NewField::CheckBox, "Check box");
                    });
                    ui.end_row();
                    ui.label("Name");
                    let edit = ui.add(
                        TextEdit::singleline(name)
                            .margin(crate::widgets::FIELD_MARGIN)
                            .desired_width(240.0),
                    );
                    if name.is_empty() {
                        edit.request_focus();
                    }
                    ui.end_row();
                });
            error_line(ui, error.as_deref(), &palette);
            buttons(ui, "Add field", Some("Cancel"), enter).map_or(Outcome::Keep, |primary| {
                if primary {
                    Outcome::Primary
                } else {
                    Outcome::Cancel
                }
            })
        }
        Dialog::About => {
            ui.vertical_centered(|ui| {
                if let Some(logo) = env.logo {
                    ui.add(
                        egui::Image::new(logo)
                            .fit_to_exact_size(vec2(120.0, 120.0))
                            .corner_radius(egui::CornerRadius::same(26)),
                    );
                }
                ui.label(RichText::new(crate::APP_NAME).size(26.0).strong());
                ui.label(
                    RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                        .color(palette.text_dim),
                );
                ui.add_space(6.0);
                ui.label("Every PDF tool. Local. Pure Rust.");
                ui.label(
                    RichText::new("No account, no server, no telemetry.").color(palette.text_dim),
                );
                ui.label(
                    RichText::new("Built with egui, lopdf and hayro")
                        .small()
                        .color(palette.text_dim),
                );
            });
            buttons(ui, "Close", None, enter).map_or(Outcome::Keep, |_| Outcome::Cancel)
        }
        Dialog::Updates => {
            updates(ui, env.updates, &palette);
            buttons(ui, "Close", None, enter).map_or(Outcome::Keep, |_| Outcome::Cancel)
        }
        Dialog::Preferences => {
            title(ui, &with_icon(icon::GEAR, "Preferences"));
            ui.horizontal(|ui| {
                ui.label("Theme");
                ui.selectable_value(env.dark, true, with_icon(icon::MOON, "Dark"));
                ui.selectable_value(env.dark, false, with_icon(icon::SUN, "Light"));
            });
            buttons(ui, "Done", None, enter).map_or(Outcome::Keep, |_| Outcome::Cancel)
        }
    }
}

fn updates(ui: &mut Ui, checker: &crate::updates::Checker, palette: &Palette) {
    use crate::updates::{CAN_INSTALL, RELEASES_PAGE, State};
    title(ui, &with_icon(icon::ARROW_CIRCLE_UP, "Updates"));
    let current = env!("CARGO_PKG_VERSION");
    match checker.state() {
        State::Idle | State::Loading => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Looking for new versions…");
            });
        }
        State::Failed(err) => {
            ui.label(RichText::new(err).color(palette.error));
            if ui
                .button(with_icon(icon::ARROW_CLOCKWISE, "Try again"))
                .clicked()
            {
                checker.check(ui.ctx());
            }
        }
        State::Ready(releases) => {
            let newest = releases.iter().find(|release| release.is_newer());
            let status = match newest {
                Some(release) => format!("{} is out. You have version {current}.", release.tag),
                None => format!("You have the latest version ({current})."),
            };
            ui.label(RichText::new(status).strong());
            ui.add_space(6.0);
            if releases.is_empty() {
                ui.label(RichText::new("No release published yet.").color(palette.text_dim));
            }
            ScrollArea::vertical()
                .max_height(360.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for release in &releases {
                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&release.name).size(16.0).strong());
                            if release.is_newer() {
                                ui.label(RichText::new("New").color(palette.accent_text).strong());
                            }
                        });
                        ui.label(
                            RichText::new(format!("{} · {}", release.tag, release.date))
                                .small()
                                .color(palette.text_dim),
                        );
                        if !release.notes.is_empty() {
                            markdown(ui, &release.notes, palette);
                        }
                        let latest = newest.is_some_and(|newest| newest.tag == release.tag);
                        match (&release.asset, latest && CAN_INSTALL) {
                            (Some(asset), true) => install_row(ui, checker, asset, palette),
                            _ if !release.url.is_empty() => {
                                ui.hyperlink_to(
                                    with_icon(icon::ARROW_SQUARE_OUT, "View on GitHub"),
                                    &release.url,
                                );
                            }
                            _ => {}
                        }
                    }
                });
        }
    }
    ui.add_space(6.0);
    ui.hyperlink_to("All releases on GitHub", RELEASES_PAGE);
}

pub const DOWNLOAD_BLUE: egui::Color32 = egui::Color32::from_rgb(0x25, 0x63, 0xEB);

fn install_row(ui: &mut Ui, checker: &crate::updates::Checker, asset: &str, palette: &Palette) {
    use crate::updates::Install;
    ui.add_space(4.0);
    match checker.install_state() {
        Install::Downloading | Install::Ready(_) => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Downloading the update… GodPDF restarts when it is done.");
            });
        }
        state => {
            let text = RichText::new(with_icon(icon::DOWNLOAD_SIMPLE, "Download and install"))
                .color(egui::Color32::WHITE)
                .strong();
            let button = Button::new(text)
                .fill(DOWNLOAD_BLUE)
                .min_size(vec2(0.0, 30.0));
            if ui
                .add(button)
                .on_hover_text("Replace GodPDF with this version and restart")
                .clicked()
            {
                checker.install(ui.ctx(), asset.to_owned());
            }
            if let Install::Failed(err) = state {
                ui.label(RichText::new(err).color(palette.error));
            }
        }
    }
}

fn markdown(ui: &mut Ui, text: &str, palette: &Palette) {
    use egui::text::{LayoutJob, TextFormat};
    use egui::{FontFamily, FontId};
    let size = 14.0;
    let line_job = |line: &str, size: f32, all_bold: bool, width: f32| {
        let mut job = LayoutJob::default();
        job.wrap.max_width = width;
        for (index, part) in line.split("**").enumerate() {
            let bold = all_bold || index % 2 == 1;
            let family = if bold {
                FontFamily::Name(crate::textbox::BOLD_FAMILY.into())
            } else {
                FontFamily::Proportional
            };
            job.append(
                &part.replace('`', ""),
                0.0,
                TextFormat {
                    font_id: FontId::new(size, family),
                    color: palette.text,
                    ..Default::default()
                },
            );
        }
        job
    };
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            ui.add_space(4.0);
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix('#') {
            let heading = heading.trim_start_matches('#').trim();
            ui.label(line_job(heading, size + 1.0, true, ui.available_width()));
            continue;
        }
        let bullet = ["- ", "* ", "+ "]
            .iter()
            .find_map(|marker| trimmed.strip_prefix(marker));
        match bullet {
            Some(item) => {
                ui.horizontal_top(|ui| {
                    ui.label(RichText::new("•").size(size).color(palette.text));
                    let width = ui.available_width();
                    // In a row a label runs on: this one is told to go to the next line.
                    ui.add(egui::Label::new(line_job(item, size, false, width)).wrap());
                });
            }
            None => {
                ui.label(line_job(trimmed, size, false, ui.available_width()));
            }
        }
    }
}

fn merge(ui: &mut Ui, files: &mut Vec<PickedFile>, palette: &Palette) -> Outcome {
    title(ui, &with_icon(icon::STACK, "Combine files"));
    ui.label(
        RichText::new("PDFs, images and text files are combined top to bottom.")
            .color(palette.text_dim),
    );
    ui.add_space(6.0);
    let mut action: Option<(usize, i32)> = None;
    ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
        if files.is_empty() {
            ui.label(RichText::new("No files yet.").color(palette.text_dim));
        }
        for (index, file) in files.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{}.", index + 1)).color(palette.text_dim));
                ui.label(&file.name);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::small_icon_button(ui, icon::X, "Remove").clicked() {
                        action = Some((index, 0));
                    }
                    let down = ui.add_enabled_ui(index + 1 < files.len(), |ui| {
                        widgets::small_icon_button(ui, icon::CARET_DOWN, "Move down")
                    });
                    if down.inner.clicked() {
                        action = Some((index, 1));
                    }
                    let up = ui.add_enabled_ui(index > 0, |ui| {
                        widgets::small_icon_button(ui, icon::CARET_UP, "Move up")
                    });
                    if up.inner.clicked() {
                        action = Some((index, -1));
                    }
                });
            });
        }
    });
    match action {
        Some((index, 0)) => {
            files.remove(index);
        }
        Some((index, 1)) => files.swap(index, index + 1),
        Some((index, _)) => files.swap(index, index - 1),
        None => {}
    }
    ui.add_space(12.0);
    let mut outcome = Outcome::Keep;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(files.len() >= 2, primary_button(ui, "Merge"))
            .clicked()
        {
            outcome = Outcome::Primary;
        }
        if ui.button("Add files…").clicked() {
            outcome = Outcome::Secondary;
        }
        if ui.button("Cancel").clicked() {
            outcome = Outcome::Cancel;
        }
    });
    outcome
}

/// How page numbers can be written: `{n}` is the page, `{total}` the last one.
const NUMBER_FORMATS: [&str; 5] = [
    "{n}",
    "Page {n}",
    "{n} / {total}",
    "Page {n} of {total}",
    "- {n} -",
];

/// A number format as it reads on the first of twelve pages.
fn number_sample(template: &str) -> String {
    template.replace("{n}", "1").replace("{total}", "12")
}

fn color_button(ui: &mut Ui, color: &mut godpdf_core::Color) {
    let mut rgb = [color.r, color.g, color.b];
    if ui.color_edit_button_rgb(&mut rgb).changed() {
        let [r, g, b] = rgb;
        *color = godpdf_core::Color { r, g, b };
    }
}

/// `1.2 MB`, `340 KB`, `12 bytes`.
pub fn size_text(bytes: usize) -> String {
    match bytes {
        0..1_000 => format!("{bytes} bytes"),
        1_000..1_000_000 => format!("{:.0} KB", bytes as f64 / 1e3),
        _ => format!("{:.1} MB", bytes as f64 / 1e6),
    }
}

fn range_field(ui: &mut Ui, range: &mut String) {
    ui.add(
        TextEdit::singleline(range)
            .margin(crate::widgets::FIELD_MARGIN)
            .hint_text("e.g. 1-3, 5, 8-")
            .desired_width(280.0),
    );
}

fn title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(19.0).strong());
    ui.add_space(8.0);
}

fn error_line(ui: &mut Ui, error: Option<&str>, palette: &Palette) {
    if let Some(error) = error {
        ui.label(RichText::new(with_icon(icon::WARNING_CIRCLE, error)).color(palette.error));
    }
}

fn primary_button(ui: &Ui, text: &str) -> Button<'static> {
    let palette = Palette::of(ui);
    Button::new(RichText::new(text.to_owned()).color(palette.on_accent)).fill(palette.accent)
}

/// Primary (+ optional cancel) buttons. `Some(true)` = primary, `Some(false)` = cancel.
fn buttons(ui: &mut Ui, primary: &str, cancel: Option<&str>, enter: bool) -> Option<bool> {
    ui.add_space(12.0);
    let mut result = None;
    ui.horizontal(|ui| {
        if ui.add(primary_button(ui, primary)).clicked() || enter {
            result = Some(true);
        }
        if let Some(cancel) = cancel
            && ui.button(cancel).clicked()
        {
            result = Some(false);
        }
    });
    result
}

// ---------------------------------------------------------------- command palette

#[derive(Default)]
pub struct PaletteState {
    pub query: String,
    pub selected: usize,
}

pub enum PaletteOutcome {
    Keep,
    Close,
    Run(Cmd),
}

pub fn command_palette(ctx: &Context, state: &mut PaletteState, has_doc: bool) -> PaletteOutcome {
    let query = state.query.to_lowercase();
    let words: Vec<&str> = query.split_whitespace().collect();
    let mut matches: Vec<Cmd> = Cmd::ALL
        .into_iter()
        .filter(|cmd| *cmd != Cmd::Palette)
        .filter(|cmd| {
            let haystack = format!(
                "{} {} {}",
                cmd.group().title(),
                cmd.label(),
                cmd.description()
            )
            .to_lowercase();
            words.iter().all(|word| haystack.contains(word))
        })
        .collect();
    // Those the words name come before those the words only describe.
    matches.sort_by_key(|cmd| {
        let label = cmd.label().to_lowercase();
        !words.iter().all(|word| label.contains(word))
    });
    let (down, up, enter) = ctx.input_mut(|input| {
        (
            input.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
            input.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
            // Taken, so the dialog the command opens does not see it as its own Enter.
            input.consume_key(egui::Modifiers::NONE, Key::Enter),
        )
    });
    if down {
        state.selected += 1;
    }
    if up {
        state.selected = state.selected.saturating_sub(1);
    }
    state.selected = state.selected.min(matches.len().saturating_sub(1));
    if enter {
        return matches
            .get(state.selected)
            .map_or(PaletteOutcome::Close, |cmd| PaletteOutcome::Run(*cmd));
    }

    let mut outcome = PaletteOutcome::Keep;
    let area =
        Modal::default_area(Id::new("palette")).anchor(egui::Align2::CENTER_TOP, vec2(0.0, 90.0));
    let response = Modal::new(Id::new("palette")).area(area).show(ctx, |ui| {
        let palette = Palette::of(ui);
        ui.set_width(620.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(icon::MAGNIFYING_GLASS)
                    .size(20.0)
                    .color(palette.text_dim),
            );
            let edit = ui.add(
                TextEdit::singleline(&mut state.query)
                    .margin(crate::widgets::FIELD_MARGIN)
                    .hint_text("Find tools and commands")
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Heading),
            );
            edit.request_focus();
            if edit.changed() {
                state.selected = 0;
            }
        });
        ui.add_space(6.0);
        ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (index, cmd) in matches.iter().enumerate() {
                let selected = index == state.selected;
                let ready = has_doc || !cmd.needs_doc();
                let response = palette_row(ui, *cmd, selected, ready, &palette);
                if selected && (down || up) {
                    response.scroll_to_me(None);
                }
                if response.clicked() {
                    outcome = PaletteOutcome::Run(*cmd);
                }
            }
            if matches.is_empty() {
                ui.label(RichText::new("No matching command").color(palette.text_dim));
            }
        });
        if let Some(cmd) = matches.get(state.selected) {
            ui.separator();
            let hint = if !has_doc && cmd.asks_for_doc() {
                "Enter asks for a PDF, then runs this on it"
            } else if !has_doc && cmd.needs_doc() {
                "Open a PDF first"
            } else if cmd.description().is_empty() {
                "Enter runs it"
            } else {
                cmd.description()
            };
            ui.label(RichText::new(hint).size(13.0).color(palette.text_dim));
        }
    });
    if matches!(outcome, PaletteOutcome::Keep) && response.should_close() {
        PaletteOutcome::Close
    } else {
        outcome
    }
}

/// A command of the palette: its icon in the colour of its family, its name, and on the
/// right its shortcut and family. Greyed while it waits for a document to be open.
fn palette_row(ui: &mut Ui, cmd: Cmd, selected: bool, ready: bool, palette: &Palette) -> Response {
    let size = vec2(ui.available_width(), 36.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, egui::CornerRadius::same(8), palette.selected);
    } else if response.hovered() {
        painter.rect_filled(rect, egui::CornerRadius::same(8), palette.hover);
    }
    let (ink, tint) = if ready {
        (palette.text, cmd.tint())
    } else {
        (palette.text_dim, palette.text_dim)
    };
    let mid = rect.center().y;
    painter.text(
        pos2(rect.min.x + 22.0, mid),
        egui::Align2::CENTER_CENTER,
        cmd.icon(),
        FontId::proportional(19.0),
        tint,
    );
    painter.text(
        pos2(rect.min.x + 46.0, mid),
        egui::Align2::LEFT_CENTER,
        cmd.label(),
        FontId::proportional(15.0),
        ink,
    );
    let family = cmd.group().title();
    let side = match cmd.shortcut() {
        Some(shortcut) => format!("{}      {family}", ui.ctx().format_shortcut(&shortcut)),
        None => family.to_owned(),
    };
    painter.text(
        pos2(rect.max.x - 14.0, mid),
        egui::Align2::RIGHT_CENTER,
        side,
        FontId::proportional(13.0),
        palette.text_dim,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

// ---------------------------------------------------------------- extracted text

pub struct TextView {
    pub title: String,
    pub text: String,
}

/// Returns false when closed.
pub fn text_window(ctx: &Context, view: &TextView) -> bool {
    let mut open = true;
    egui::Window::new(&view.title)
        .id(Id::new("text-view"))
        .open(&mut open)
        .default_size([560.0, 520.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button(with_icon(icon::COPY, "Copy all")).clicked() {
                    ui.ctx().copy_text(view.text.clone());
                }
                ui.label(RichText::new(format!("{} characters", view.text.chars().count())).weak());
            });
            ui.separator();
            ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                let mut text = view.text.as_str();
                ui.add(
                    TextEdit::multiline(&mut text)
                        .margin(crate::widgets::FIELD_MARGIN)
                        .desired_width(f32::INFINITY)
                        .code_editor(),
                );
            });
        });
    open
}
