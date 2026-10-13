//! Modal dialogs, command palette and the extracted-text window.
//!
//! Dialogs only edit their own fields and report an [`Outcome`]; the app performs the work.

use egui::{
    Button, Context, Id, Key, Modal, RichText, ScrollArea, TextEdit, TextureHandle, Ui, vec2,
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
    About,
    Updates,
    Preferences,
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
    title(ui, &with_icon(icon::STACK, "Merge PDFs"));
    ui.label(RichText::new("Files are combined top to bottom.").color(palette.text_dim));
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

pub fn command_palette(ctx: &Context, state: &mut PaletteState) -> PaletteOutcome {
    let query = state.query.to_lowercase();
    let matches: Vec<Cmd> = Cmd::ALL
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
            query.split_whitespace().all(|word| haystack.contains(word))
        })
        .collect();
    let (down, up, enter) = ctx.input_mut(|input| {
        (
            input.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
            input.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
            input.key_pressed(Key::Enter),
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
        ui.set_width(520.0);
        let edit = ui.add(
            TextEdit::singleline(&mut state.query)
                .margin(crate::widgets::FIELD_MARGIN)
                .hint_text("Type a command…")
                .desired_width(f32::INFINITY)
                .font(egui::TextStyle::Heading),
        );
        edit.request_focus();
        if edit.changed() {
            state.selected = 0;
        }
        ui.add_space(6.0);
        ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            for (index, cmd) in matches.iter().enumerate() {
                let selected = index == state.selected;
                let shortcut = cmd
                    .shortcut()
                    .map(|sc| ui.ctx().format_shortcut(&sc))
                    .unwrap_or_default();
                let text = format!(
                    "{}   {}  ·  {}",
                    cmd.icon(),
                    cmd.label(),
                    cmd.group().title()
                );
                let response = ui.add_sized(
                    [ui.available_width(), 28.0],
                    Button::selectable(selected, text).shortcut_text(shortcut),
                );
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
    });
    if matches!(outcome, PaletteOutcome::Keep) && response.should_close() {
        PaletteOutcome::Close
    } else {
        outcome
    }
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
