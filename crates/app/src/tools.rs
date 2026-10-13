//! Header modes ("All tools", "Read", …) and the left tools panel listing their commands.

use egui::{RichText, ScrollArea, Ui};

use crate::commands::{self, Cmd, Group};
use crate::theme::Palette;
use crate::widgets::{self, icon};

/// Style applied to new comments.
pub struct CommentStyle {
    pub color: egui::Color32,
    pub width: f32,
    pub font_size: f32,
    pub font: godpdf_core::Font,
}

impl Default for CommentStyle {
    fn default() -> Self {
        Self {
            color: egui::Color32::from_rgb(0xFF, 0xF2, 0x00),
            width: 2.0,
            font_size: 14.0,
            font: godpdf_core::Font::Helvetica,
        }
    }
}

/// Header tab: decides what the left panel lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    AllTools,
    Read,
    Organize,
    Comment,
    Protect,
}

impl Mode {
    pub const ALL: [Mode; 5] = [
        Mode::AllTools,
        Mode::Read,
        Mode::Organize,
        Mode::Comment,
        Mode::Protect,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::AllTools => "All tools",
            Self::Read => "Read",
            Self::Organize => "Organize",
            Self::Comment => "Comment",
            Self::Protect => "Protect",
        }
    }

    fn panel_title(self) -> &'static str {
        match self {
            Self::Organize => "Organize pages",
            Self::Comment => "Add comments",
            Self::Protect => "Protect a PDF",
            Self::AllTools | Self::Read => "All tools",
        }
    }
}

/// One row of the tools panel: run a command or switch to a mode.
#[derive(Clone, Copy)]
pub enum Action {
    Run(Cmd),
    Mode(Mode),
}

/// "All tools" list; rows past this count hide behind "View more".
const FOLDED: usize = 11;

const ALL_TOOLS: [(Action, &str, &str); 17] = [
    (Action::Run(Cmd::SaveAs), icon::EXPORT, "Export a PDF"),
    (Action::Run(Cmd::Merge), icon::STACK, "Combine files"),
    (
        Action::Mode(Mode::Organize),
        icon::SQUARES_FOUR,
        "Organize pages",
    ),
    (Action::Mode(Mode::Comment), icon::CHAT_TEXT, "Add comments"),
    (
        Action::Mode(Mode::Protect),
        icon::SHIELD_CHECK,
        "Protect a PDF",
    ),
    (Action::Run(Cmd::EditText), icon::TEXT_AA, "Edit PDF text"),
    (
        Action::Run(Cmd::RotateRight),
        icon::ARROW_CLOCKWISE,
        "Rotate page",
    ),
    (
        Action::Run(Cmd::RotateAll),
        icon::ARROWS_CLOCKWISE,
        "Rotate all pages",
    ),
    (
        Action::Run(Cmd::InsertBlank),
        icon::FILE_PLUS,
        "Insert a blank page",
    ),
    (
        Action::Run(Cmd::InsertFile),
        icon::FILE_ARROW_DOWN,
        "Insert from file",
    ),
    (Action::Run(Cmd::Extract), icon::EXPORT, "Extract pages"),
    (Action::Run(Cmd::Split), icon::SCISSORS, "Split a PDF"),
    (Action::Run(Cmd::DeletePages), icon::TRASH, "Delete pages"),
    (Action::Run(Cmd::Properties), icon::INFO, "Properties"),
    (
        Action::Run(Cmd::ExtractText),
        icon::TEXT_ALIGN_LEFT,
        "Extract text",
    ),
    (
        Action::Run(Cmd::Find),
        icon::MAGNIFYING_GLASS,
        "Search text",
    ),
    (Action::Run(Cmd::Palette), icon::COMMAND, "All commands"),
];

#[derive(Default)]
pub struct Output {
    pub action: Option<Action>,
    pub close: bool,
}

/// The left panel for `mode` (never shown for [`Mode::Read`]).
pub fn panel(ui: &mut Ui, mode: Mode, expanded: &mut bool) -> Output {
    let palette = Palette::of(ui);
    let mut out = Output::default();
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(mode.panel_title()).size(17.0).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            out.close = widgets::icon_button(ui, icon::X, "Close panel").clicked();
        });
    });
    ui.add_space(8.0);
    ScrollArea::vertical()
        .id_salt("tools-panel")
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let mut row = |ui: &mut Ui, action: Action, glyph: &str, label: &str| {
                let mut response = widgets::tool_row(ui, glyph, tint(action), label, None);
                if !hint(action).is_empty() {
                    response = response.on_hover_text(hint(action));
                }
                if response.clicked() {
                    out.action = Some(action);
                }
            };
            match mode {
                Mode::AllTools | Mode::Read => {
                    let shown = if *expanded { ALL_TOOLS.len() } else { FOLDED };
                    for (action, glyph, label) in &ALL_TOOLS[..shown] {
                        row(ui, *action, glyph, label);
                    }
                    ui.add_space(6.0);
                    let more = if *expanded { "View less" } else { "View more" };
                    let link = egui::Button::new(RichText::new(more).color(palette.text))
                        .frame_when_inactive(false)
                        .stroke(egui::Stroke::NONE);
                    if ui.add(link).clicked() {
                        *expanded = !*expanded;
                    }
                }
                Mode::Organize | Mode::Comment | Mode::Protect => {
                    let group = match mode {
                        Mode::Organize => Group::Organize,
                        Mode::Comment => Group::Comment,
                        _ => Group::Protect,
                    };
                    for cmd in commands::in_group(group) {
                        row(ui, Action::Run(cmd), cmd.icon(), cmd.short_label());
                    }
                    if mode == Mode::Organize {
                        row(
                            ui,
                            Action::Run(Cmd::Merge),
                            Cmd::Merge.icon(),
                            "Combine files",
                        );
                    }
                    if mode == Mode::Comment {
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new("Comments are listed in the panel on the right.")
                                .size(13.0)
                                .color(palette.text_dim),
                        );
                    }
                }
            }
        });
    out
}

/// The colour of the icon of a row: the one of the command, or of the tools a mode shows.
fn tint(action: Action) -> egui::Color32 {
    match action {
        Action::Run(cmd) => cmd.tint(),
        Action::Mode(Mode::Organize) => Group::Organize.tint(),
        Action::Mode(Mode::Comment) => Group::Comment.tint(),
        Action::Mode(Mode::Protect) => Group::Protect.tint(),
        Action::Mode(Mode::AllTools | Mode::Read) => Group::View.tint(),
    }
}

fn hint(action: Action) -> &'static str {
    match action {
        Action::Run(cmd) => cmd.description(),
        Action::Mode(Mode::Organize) => "Reorder, rotate and delete pages in a grid",
        Action::Mode(Mode::Comment) => "Highlight, notes, text, shapes and ink",
        Action::Mode(Mode::Protect) => "Set or remove a password",
        Action::Mode(_) => "",
    }
}
