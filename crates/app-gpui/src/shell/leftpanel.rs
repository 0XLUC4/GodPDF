//! Left tools panel (egui `doc_panels` + `tools::panel`): title (17) + close cross, then the
//! rows of the mode. Fill `bg`, margin 12 × 6, width 280, a `border` line on its right.
//! Hidden in Read mode or when closed (Menu > Tools panel brings it back).

use gpui::prelude::*;
use gpui::{Action, AnyElement, Context, SharedString, div, px};

use super::Mode;
use crate::app::{GodPdfApp, SaveAs};
use crate::dialogs::{
    AddWatermark, Booklet, CommandPalette, CropPages, DeletePage, ExtractPages, HalvePages,
    InsertBlankPage, JoinPages, MergeFiles, NumberPages, PageSize, PagesPerSheet, Properties,
    RemoveHiddenData, RemovePassword, ReversePages, RotateAll, RotateLeft, RotateRight,
    SetPassword, SplitDocument,
};
use crate::theme::{self, Palette, hex};
use crate::tools::{HighlightTool, InkTool, NoteTool, PictureTool, RectangleTool, TextTool};
use crate::{fonts, icons, panels, toasts, widgets};

// shortcut: fixed width, egui lets it be dragged 220..=420; add a drag handle if wanted.
const WIDTH: f32 = 280.0;
/// "All tools" rows past this count hide behind "View more".
const FOLDED: usize = 11;

/// What a row does.
#[derive(Clone, Copy)]
enum Do {
    Run(fn() -> Box<dyn Action>),
    Mode(Mode),
    /// Not ported to GPUI yet: listed as in egui, a click says so in a toast.
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

use Do::{Missing, Run};
use theme::{AMBER, BLUE, GREEN, PINK, PURPLE, RED, TEAL};

/// egui `tools::ALL_TOOLS`, with each command's icon and tint (`Cmd::icon`, `Cmd::tint`).
const ALL_TOOLS: &[Row] = &[
    row(
        Run(|| Box::new(SaveAs)),
        icons::EXPORT,
        TEAL,
        "Export a PDF",
        "",
    ),
    row(
        Run(|| Box::new(MergeFiles)),
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
    row(
        Missing,
        icons::TEXT_AA,
        PINK,
        "Edit PDF text",
        "Click a line of the page to change its words",
    ),
    row(
        Run(|| Box::new(RotateRight)),
        icons::ARROW_CLOCKWISE,
        GREEN,
        "Rotate page",
        "Turn the current page by 90°",
    ),
    row(
        Run(|| Box::new(RotateAll)),
        icons::ARROWS_CLOCKWISE,
        GREEN,
        "Rotate all pages",
        "Turn every page by 90°",
    ),
    row(
        Run(|| Box::new(InsertBlankPage)),
        icons::FILE_PLUS,
        GREEN,
        "Insert a blank page",
        "Add an empty page",
    ),
    row(
        Missing,
        icons::FILE_ARROW_DOWN,
        GREEN,
        "Insert from file",
        "Insert another PDF's pages",
    ),
    row(
        Run(|| Box::new(ExtractPages)),
        icons::EXPORT,
        TEAL,
        "Extract pages",
        "Copy pages into a new PDF",
    ),
    row(
        Run(|| Box::new(SplitDocument)),
        icons::SCISSORS,
        GREEN,
        "Split a PDF",
        "Cut a PDF into several files",
    ),
    row(
        Run(|| Box::new(DeletePage)),
        icons::TRASH,
        RED,
        "Delete pages",
        "Remove pages for good",
    ),
    row(
        Run(|| Box::new(NumberPages)),
        icons::LIST_NUMBERS,
        GREEN,
        "Number pages",
        "Write its number on every page",
    ),
    row(
        Run(|| Box::new(AddWatermark)),
        icons::STAMP,
        PINK,
        "Add a watermark",
        "Draw a line of text across the pages",
    ),
    row(
        Run(|| Box::new(Properties)),
        icons::INFO,
        BLUE,
        "Properties",
        "Edit title, author, keywords",
    ),
    row(
        Missing,
        icons::TEXT_ALIGN_LEFT,
        TEAL,
        "Extract text",
        "Copy all text out",
    ),
    row(
        Run(|| Box::new(panels::Find)),
        icons::MAGNIFYING_GLASS,
        BLUE,
        "Search text",
        "Find words with per-page counts",
    ),
    row(
        Run(|| Box::new(CommandPalette)),
        icons::COMMAND,
        BLUE,
        "All commands",
        "",
    ),
];

/// egui `commands::in_group(Group::Organize)` (`Cmd::short_label`), then "Combine files".
const ORGANIZE: &[Row] = &[
    // TODO(organize): egui toggles the page grid here; GPUI has no grid yet.
    row(
        Missing,
        icons::SQUARES_FOUR,
        GREEN,
        "Organize",
        "Reorder, rotate and delete pages in a grid",
    ),
    row(
        Run(|| Box::new(RotateLeft)),
        icons::ARROW_COUNTER_CLOCKWISE,
        GREEN,
        "Rotate left",
        "Turn the current page by 90°",
    ),
    row(
        Run(|| Box::new(RotateRight)),
        icons::ARROW_CLOCKWISE,
        GREEN,
        "Rotate right",
        "Turn the current page by 90°",
    ),
    row(
        Run(|| Box::new(RotateAll)),
        icons::ARROWS_CLOCKWISE,
        GREEN,
        "Rotate all pages",
        "Turn every page by 90°",
    ),
    row(
        Run(|| Box::new(DeletePage)),
        icons::TRASH,
        RED,
        "Delete",
        "Remove pages for good",
    ),
    row(
        Missing,
        icons::CARET_LEFT,
        GREEN,
        "Move earlier",
        "Move the selected pages one place earlier",
    ),
    row(
        Missing,
        icons::CARET_RIGHT,
        GREEN,
        "Move later",
        "Move the selected pages one place later",
    ),
    row(
        Run(|| Box::new(InsertBlankPage)),
        icons::FILE_PLUS,
        GREEN,
        "Blank page",
        "Add an empty page",
    ),
    row(
        Missing,
        icons::FILE_ARROW_DOWN,
        GREEN,
        "Insert file",
        "Insert another PDF's pages",
    ),
    row(
        Run(|| Box::new(ExtractPages)),
        icons::EXPORT,
        TEAL,
        "Extract",
        "Copy pages into a new PDF",
    ),
    row(
        Run(|| Box::new(SplitDocument)),
        icons::SCISSORS,
        GREEN,
        "Split",
        "Cut a PDF into several files",
    ),
    row(
        Run(|| Box::new(ReversePages)),
        icons::SORT_DESCENDING,
        GREEN,
        "Reverse",
        "Last page first",
    ),
    row(
        Run(|| Box::new(CropPages)),
        icons::CROP,
        GREEN,
        "Crop pages",
        "Hide the margins of the pages",
    ),
    row(
        Run(|| Box::new(PageSize)),
        icons::RESIZE,
        GREEN,
        "Change page size",
        "A4, Letter, A3… what the pages show is scaled to fit",
    ),
    row(
        Run(|| Box::new(HalvePages)),
        icons::SQUARE_SPLIT_HORIZONTAL,
        GREEN,
        "Cut in half",
        "Each page into two: scans of two pages side by side",
    ),
    row(
        Run(|| Box::new(PagesPerSheet)),
        icons::SQUARES_FOUR,
        GREEN,
        "Per sheet",
        "2, 4, 6, 8, 9 or 16 pages on each sheet",
    ),
    row(
        Run(|| Box::new(Booklet)),
        icons::BOOK_OPEN,
        GREEN,
        "Make a booklet",
        "Print both sides, fold in the middle",
    ),
    row(
        Run(|| Box::new(JoinPages)),
        icons::ROWS,
        GREEN,
        "One page",
        "Every page one under the other, on one page",
    ),
    row(
        Missing,
        icons::FILE_X,
        RED,
        "Blank pages",
        "Find the empty pages and take them out",
    ),
    row(
        Run(|| Box::new(NumberPages)),
        icons::LIST_NUMBERS,
        GREEN,
        "Number pages",
        "Write its number on every page",
    ),
    row(
        Run(|| Box::new(MergeFiles)),
        icons::STACK,
        PURPLE,
        "Combine files",
        "Combine PDFs, images and text files into one",
    ),
];

/// egui `commands::in_group(Group::Comment)`.
const COMMENT: &[Row] = &[
    row(
        Run(|| Box::new(HighlightTool)),
        icons::HIGHLIGHTER,
        AMBER,
        "Highlight",
        "Mark important passages",
    ),
    row(
        Run(|| Box::new(NoteTool)),
        icons::NOTE,
        AMBER,
        "Sticky note",
        "Pin a sticky note",
    ),
    row(
        Run(|| Box::new(TextTool)),
        icons::TEXT_T,
        AMBER,
        "Free text",
        "Type text onto the page",
    ),
    row(
        Run(|| Box::new(RectangleTool)),
        icons::RECTANGLE,
        AMBER,
        "Rectangle",
        "Frame an area",
    ),
    row(
        Run(|| Box::new(InkTool)),
        icons::PENCIL_SIMPLE,
        AMBER,
        "Ink",
        "Draw freehand",
    ),
    row(
        Run(|| Box::new(PictureTool)),
        icons::IMAGE,
        AMBER,
        "Picture",
        "Put a PNG or JPEG on the page, then move and resize it",
    ),
    row(
        Missing,
        icons::SIGNATURE,
        AMBER,
        "Sign",
        "Draw your signature once, then click where it goes",
    ),
    row(
        Missing,
        icons::TEXTBOX,
        AMBER,
        "Form field",
        "Add a text box or a check box to fill in",
    ),
];

/// egui `commands::in_group(Group::Protect)`.
const PROTECT: &[Row] = &[
    row(
        Run(|| Box::new(SetPassword)),
        icons::LOCK,
        BLUE,
        "Set password",
        "Protect with a password",
    ),
    row(
        Run(|| Box::new(RemovePassword)),
        icons::LOCK_OPEN,
        BLUE,
        "Remove password",
        "Save without a password",
    ),
    row(
        Run(|| Box::new(RemoveHiddenData)),
        icons::BROOM,
        RED,
        "Hidden data",
        "Properties, scripts and attached files go",
    ),
];

/// The rows `mode` lists (egui `tools::panel`).
fn rows_of(mode: Mode) -> &'static [Row] {
    match mode {
        Mode::AllTools | Mode::Read => ALL_TOOLS,
        Mode::Organize => ORGANIZE,
        Mode::Comment => COMMENT,
        Mode::Protect => PROTECT,
    }
}

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
    let all = rows_of(mode);
    let shown = if mode == Mode::AllTools && !shell.tools_expanded {
        FOLDED
    } else {
        all.len()
    };
    let mut rows: Vec<AnyElement> = all[..shown]
        .iter()
        .enumerate()
        .map(|(index, row)| tool_row(index, row, palette, cx))
        .collect();
    match mode {
        Mode::AllTools => {
            let more = if shell.tools_expanded {
                "View less"
            } else {
                "View more"
            };
            // egui: 6 px of space on top of the 2 px item spacing; the button hugs its text.
            rows.push(
                div()
                    .pt(px(6.))
                    .flex()
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
        }
        Mode::Comment => rows.push(
            div()
                .pt(px(10.))
                .text_size(px(13.))
                .text_color(palette.text_dim)
                .child("Comments are listed in the panel on the right.")
                .into_any_element(),
        ),
        _ => {}
    }

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
                // egui: 10 px, the title row, item spacing 6 + 8 px, then the list.
                div()
                    .mt(px(10.))
                    .mb(px(14.))
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
                    .id(("tools-list", mode as usize))
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
    let element = widgets::tool_row(
        ("tool-row", index),
        row.glyph,
        hex(row.tint),
        row.label,
        None,
        palette,
    );
    let element = match row.action {
        Run(action) => element.on_click(move |_, window, cx| window.dispatch_action(action(), cx)),
        Do::Mode(mode) => {
            element.on_click(cx.listener(move |app, _, _, cx| app.set_mode(mode, cx)))
        }
        Missing => element.on_click(|_, _, cx| toasts::info(cx, "Not available yet")),
    };
    if row.hint.is_empty() {
        return element.into_any_element();
    }
    widgets::tooltip(element, SharedString::from(row.hint)).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_match_egui() {
        assert_eq!(ALL_TOOLS.len(), 19);
        assert_eq!(ALL_TOOLS[FOLDED - 1].label, "Extract pages");
        // 20 Organize commands + "Combine files", 8 Comment, 3 Protect in egui.
        assert_eq!(ORGANIZE.len(), 21);
        assert_eq!(COMMENT.len(), 8);
        assert_eq!(PROTECT.len(), 3);
    }
}
