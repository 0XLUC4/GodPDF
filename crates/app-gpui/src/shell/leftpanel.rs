//! Left tools panel (egui `doc_panels` + `tools::panel`): title (17) + close cross, then the
//! rows of the mode. Fill `bg`, margin 12 × 6, width 280, a `border` line on its right.
//! Hidden in Read mode or when closed (Menu > Tools panel brings it back).

use gpui::prelude::*;
use gpui::{Action, AnyElement, Context, SharedString, div, px};

use super::Mode;
use crate::app::{GodPdfApp, SaveAs};
use crate::dialogs::{
    AddWatermark, CommandPalette, DeletePage, ExtractPages, InsertBlankPage, MergeFiles,
    NumberPages, Properties, RotateAll, RotateRight, SplitDocument,
};
use crate::theme::{self, Palette, hex};
use crate::{fonts, icons, panels, widgets};

// shortcut: fixed width, egui lets it be dragged 220..=420; add a drag handle if wanted.
const WIDTH: f32 = 280.0;
/// "All tools" rows past this count hide behind "View more".
const FOLDED: usize = 11;

/// What a row does.
#[derive(Clone, Copy)]
enum Do {
    Run(fn() -> Box<dyn Action>),
    Mode(Mode),
    /// Not ported to GPUI yet: shown dimmed, does nothing.
    Missing,
}

struct Row {
    action: Do,
    glyph: &'static str,
    tint: u32,
    label: &'static str,
    /// Tooltip (egui `Cmd::description` / mode hint); empty = none.
    hint: &'static str,
}

const fn row(
    action: Do,
    glyph: &'static str,
    tint: u32,
    label: &'static str,
    hint: &'static str,
) -> Row {
    Row {
        action,
        glyph,
        tint,
        label,
        hint,
    }
}

use theme::{AMBER, BLUE, GREEN, PINK, PURPLE, RED, TEAL};

/// egui `tools::ALL_TOOLS`, with each command's icon and tint (`Cmd::icon`, `Cmd::tint`).
const ALL_TOOLS: [Row; 19] = [
    row(
        Do::Run(|| Box::new(SaveAs)),
        icons::EXPORT,
        TEAL,
        "Export a PDF",
        "",
    ),
    row(
        Do::Run(|| Box::new(MergeFiles)),
        icons::STACK,
        PURPLE,
        "Combine files",
        "Combine PDFs, images and text files into one",
    ),
    row(
        Do::Mode(Mode::Organize),
        icons::SQUARES_FOUR,
        GREEN,
        "Organize pages",
        "Reorder, rotate and delete pages in a grid",
    ),
    row(
        Do::Mode(Mode::Comment),
        icons::CHAT_TEXT,
        AMBER,
        "Add comments",
        "Highlight, notes, text, shapes and ink",
    ),
    row(
        Do::Mode(Mode::Protect),
        icons::SHIELD_CHECK,
        BLUE,
        "Protect a PDF",
        "Set or remove a password",
    ),
    // TODO(left panel): Edit PDF text, Insert from file and Extract text have no GPUI port.
    row(
        Do::Missing,
        icons::TEXT_AA,
        PINK,
        "Edit PDF text",
        "Click a line of the page to change its words",
    ),
    row(
        Do::Run(|| Box::new(RotateRight)),
        icons::ARROW_CLOCKWISE,
        GREEN,
        "Rotate page",
        "Turn the current page by 90°",
    ),
    row(
        Do::Run(|| Box::new(RotateAll)),
        icons::ARROWS_CLOCKWISE,
        GREEN,
        "Rotate all pages",
        "Turn every page by 90°",
    ),
    row(
        Do::Run(|| Box::new(InsertBlankPage)),
        icons::FILE_PLUS,
        GREEN,
        "Insert a blank page",
        "Add an empty page",
    ),
    row(
        Do::Missing,
        icons::FILE_ARROW_DOWN,
        GREEN,
        "Insert from file",
        "Insert another PDF's pages",
    ),
    row(
        Do::Run(|| Box::new(ExtractPages)),
        icons::EXPORT,
        TEAL,
        "Extract pages",
        "Copy pages into a new PDF",
    ),
    row(
        Do::Run(|| Box::new(SplitDocument)),
        icons::SCISSORS,
        GREEN,
        "Split a PDF",
        "Cut a PDF into several files",
    ),
    row(
        Do::Run(|| Box::new(DeletePage)),
        icons::TRASH,
        RED,
        "Delete pages",
        "Remove pages for good",
    ),
    row(
        Do::Run(|| Box::new(NumberPages)),
        icons::LIST_NUMBERS,
        GREEN,
        "Number pages",
        "Write its number on every page",
    ),
    row(
        Do::Run(|| Box::new(AddWatermark)),
        icons::STAMP,
        PINK,
        "Add a watermark",
        "Draw a line of text across the pages",
    ),
    row(
        Do::Run(|| Box::new(Properties)),
        icons::INFO,
        BLUE,
        "Properties",
        "Edit title, author, keywords",
    ),
    row(
        Do::Missing,
        icons::TEXT_ALIGN_LEFT,
        TEAL,
        "Extract text",
        "Copy all text out",
    ),
    row(
        Do::Run(|| Box::new(panels::Find)),
        icons::MAGNIFYING_GLASS,
        BLUE,
        "Search text",
        "Find words with per-page counts",
    ),
    row(
        Do::Run(|| Box::new(CommandPalette)),
        icons::COMMAND,
        BLUE,
        "All commands",
        "",
    ),
];

pub fn render(
    app: &GodPdfApp,
    palette: &Palette,
    cx: &mut Context<GodPdfApp>,
) -> Option<AnyElement> {
    let shell = &app.shell;
    if !shell.show_tools || shell.mode == Mode::Read {
        return None;
    }
    let mode = shell.mode;
    let rows: Vec<AnyElement> = match mode {
        Mode::AllTools | Mode::Read => {
            let shown = if shell.tools_expanded {
                ALL_TOOLS.len()
            } else {
                FOLDED
            };
            let more = if shell.tools_expanded {
                "View less"
            } else {
                "View more"
            };
            let mut rows: Vec<AnyElement> = ALL_TOOLS[..shown]
                .iter()
                .enumerate()
                .map(|(index, row)| tool_row(index, row, palette, cx))
                .collect();
            rows.push(
                div()
                    .pt(px(6.))
                    .child(
                        widgets::button("view-more", more, palette).on_click(cx.listener(
                            |app, _, _, cx| {
                                app.shell.tools_expanded = !app.shell.tools_expanded;
                                cx.notify();
                            },
                        )),
                    )
                    .into_any_element(),
            );
            rows
        }
        // TODO(left panel): egui lists `commands::in_group(group)` here, each with its own
        // icon, tint and `short_label` (+ "Combine files" in Organize, and the note
        // "Comments are listed in the panel on the right." in Comment).
        Mode::Organize | Mode::Comment | Mode::Protect => {
            vec![
                div()
                    .px(px(10.))
                    .text_size(px(13.))
                    .text_color(palette.text_dim)
                    .child("Use Menu or Ctrl+K for these tools for now.")
                    .into_any_element(),
            ]
        }
    };

    Some(
        div()
            .id("tools-panel")
            .w(px(WIDTH))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .px(px(12.))
            .py(px(6.))
            .bg(palette.bg)
            .border_r_1()
            .border_color(palette.border)
            .text_size(px(fonts::BODY))
            .child(
                div()
                    .mt(px(10.))
                    .mb(px(8.))
                    .flex()
                    .items_center()
                    .child(div().flex_1().text_size(px(17.)).child(mode.panel_title()))
                    .child(
                        widgets::icon_button("close-tools", icons::X, "Close panel", palette)
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.shell.show_tools = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("tools-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .children(rows),
            )
            .into_any_element(),
    )
}

fn tool_row(index: usize, row: &Row, palette: &Palette, cx: &mut Context<GodPdfApp>) -> AnyElement {
    let missing = matches!(row.action, Do::Missing);
    let tint = if missing {
        palette.disabled()
    } else {
        hex(row.tint)
    };
    let element = widgets::tool_row(
        ("tool-row", index),
        row.glyph,
        tint,
        row.label,
        None,
        palette,
    )
    .when(missing, |element| element.opacity(0.5).cursor_default());
    let element = match row.action {
        Do::Run(action) => {
            element.on_click(move |_, window, cx| window.dispatch_action(action(), cx))
        }
        Do::Mode(mode) => {
            element.on_click(cx.listener(move |app, _, _, cx| app.set_mode(mode, cx)))
        }
        Do::Missing => element,
    };
    let hint: SharedString = match (missing, row.hint) {
        (true, hint) => format!("{hint} (not available yet)")
            .trim_start()
            .to_owned()
            .into(),
        (false, "") => return element.into_any_element(),
        (false, hint) => hint.into(),
    };
    widgets::tooltip(element, hint).into_any_element()
}
