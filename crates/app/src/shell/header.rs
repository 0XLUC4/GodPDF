//! Header row: Menu, the mode tabs,
//! then on the right the search pill and properties / save / export / print. Fill `bg`,
//! margin 10 × 0, height 46, a `border` line under it.

use std::path::PathBuf;

use gpui::component::button::{Button, ButtonVariants};
use gpui::component::menu::{DropdownMenu, PopupMenu, PopupMenuItem};
use gpui::prelude::*;
use gpui::{Action, AnyElement, Context, Div, SharedString, Stateful, WeakEntity, Window, div, px};

use super::{HEADER_HEIGHT, Mode, ReopenTab, ToggleToolsPanel};
use crate::app::{self, GodPdfApp, Save, SaveAs};
use crate::dialogs::{COMMANDS, CommandPalette, Group, Print, Properties};
use crate::theme::Palette;
use crate::widgets::{self, dispatch, icon};
use crate::{fonts, icons};

/// Narrower windows get a magnifier button instead of the pill, so the right buttons stay.
const SEARCH_PILL_FROM: f32 = 900.0;
const SEARCH_HINT_FROM: f32 = 1_080.0;

pub fn render(
    app: &GodPdfApp,
    palette: &Palette,
    window: &Window,
    cx: &mut Context<GodPdfApp>,
) -> AnyElement {
    let doc_view = app.doc_view();
    let undo_redo = app.active_tab().map_or((None, None), |tab| {
        let session = &tab.read(cx).session;
        let label = |label: Option<&str>| label.map(|label| SharedString::from(label.to_owned()));
        (label(session.undo_label()), label(session.redo_label()))
    });
    let state = MenuState {
        app: cx.weak_entity(),
        show_tools: app.shell.show_tools,
        doc_view,
        has_doc: !app.tabs.is_empty(),
        undo_redo,
        recent: app
            .shell
            .recent
            .iter()
            .map(|path| (path.clone(), !super::welcome::is_missing(path, cx)))
            .collect(),
        reopen: !app.shell.closed.is_empty(),
    };
    let menu = Button::new("menu")
        .ghost()
        .h(px(32.))
        .px(px(10.))
        .child(widgets::with_icon(
            icons::SIDEBAR_SIMPLE,
            "Menu",
            14.0,
            palette.text,
        ))
        .dropdown_menu(move |menu, window, cx| state.build(menu, window, cx));

    let tabs = Mode::ALL.map(|mode| {
        let enabled = doc_view || mode == Mode::AllTools;
        let active = if doc_view {
            app.shell.mode == mode
        } else {
            mode == Mode::AllTools
        };
        mode_tab(mode, active, enabled, palette).when(enabled, |tab| {
            tab.on_click(cx.listener(move |app, _, _, cx| app.set_mode(mode, cx)))
        })
    });

    let buttons = [
        (icons::INFO, "Document properties", "properties"),
        (icons::FLOPPY_DISK, "Save (Ctrl+S)", "save"),
        (
            icons::FLOPPY_DISK_BACK,
            "Save As… (Ctrl+Shift+S)",
            "save-as",
        ),
        (icons::PRINTER, "Print… (Ctrl+P)", "print"),
    ]
    .map(|(glyph, tip, id)| {
        let button = widgets::icon_widget(id, glyph, widgets::ICON_BUTTON, 16.0, doc_view, palette);
        let button = widgets::tooltip(button, tip);
        if !doc_view {
            return button;
        }
        match id {
            "properties" => button.on_click(dispatch(Properties)),
            "save" => button.on_click(dispatch(Save)),
            "save-as" => button.on_click(dispatch(SaveAs)),
            _ => button.on_click(dispatch(Print)),
        }
    });

    let viewport_width = f32::from(window.viewport_size().width);
    let search = if viewport_width < SEARCH_PILL_FROM {
        widgets::icon_button(
            "search-button",
            icons::MAGNIFYING_GLASS,
            "Command palette (Ctrl+K)",
            palette,
        )
        .on_click(dispatch(CommandPalette))
    } else {
        search_field(palette, viewport_width >= SEARCH_HINT_FROM)
    };

    div()
        .flex_none()
        .flex()
        .items_center()
        .h(px(HEADER_HEIGHT))
        .px(px(10.))
        .bg(palette.bg)
        .border_b_1()
        .border_color(palette.border)
        .text_size(px(fonts::BODY))
        .child(menu)
        // Item spacing 8 after the Menu button, then 14.
        .child(div().flex_none().w(px(8. + 14.)))
        // On a narrow window the mode tabs are clipped, never the buttons on the right.
        .child(
            div()
                .flex()
                .min_w_0()
                .overflow_hidden()
                .gap(px(2.))
                .children(tabs),
        )
        .child(
            div()
                .ml_auto()
                .pl(px(16.))
                .flex()
                .items_center()
                .gap(px(6.))
                .children(crate::services::working(palette, cx))
                .child(search)
                .child(
                    Button::new("share-pdf")
                        .ghost()
                        .child(crate::collaboration::button_label(app))
                        .on_click(cx.listener(|app, _, window, cx| {
                            crate::collaboration::toggle(app, window, cx);
                        })),
                )
                .child(div().w(px(0.)))
                .children(buttons),
        )
        .into_any_element()
}

/// Text tab with a 2 px underline when active: text 15 + 11 px each side,
/// full header height; hover fill 8 px in from top and bottom, radius 6.
fn mode_tab(mode: Mode, active: bool, enabled: bool, palette: &Palette) -> Stateful<Div> {
    let color = if !enabled {
        palette.disabled()
    } else if active {
        palette.text
    } else {
        palette.text_dim
    };
    let group = mode.title();
    let hover = palette.hover;
    div()
        .id(mode.title())
        .group(group)
        .relative()
        .flex_none()
        .h(px(HEADER_HEIGHT))
        .px(px(11.))
        .flex()
        .items_center()
        .text_size(px(15.))
        .text_color(color)
        .when(enabled && !active, |tab| {
            tab.cursor_pointer().child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(8.))
                    .bottom(px(8.))
                    .rounded(px(6.))
                    .group_hover(group, move |style| style.bg(hover)),
            )
        })
        .child(div().relative().child(mode.title()))
        .when(active, |tab| {
            tab.child(
                div()
                    .absolute()
                    .left(px(11.))
                    .right(px(11.))
                    .bottom_0()
                    .h(px(2.))
                    .rounded(px(1.))
                    .bg(palette.text),
            )
        })
}

/// Pill that looks like a search box and opens the command palette:
/// 32 high, radius 16, `bg` fill, 1 px `border` (`text_dim` on hover), magnifier 15 centered
/// 18 px in, hint (14, dim) at 34 px, shortcut (12, dim) 14 px from the right.
fn search_field(palette: &Palette, show_shortcut: bool) -> Stateful<Div> {
    let hover = palette.text_dim;
    let pill = div()
        .id("search-pill")
        .flex_none()
        .w(px(if show_shortcut { 300.0 } else { 184.0 }))
        .h(px(32.))
        .flex()
        .items_center()
        .pl(px(10.))
        .pr(px(14.))
        .rounded(px(16.))
        .bg(palette.bg)
        .border_1()
        .border_color(palette.border)
        .hover(move |style| style.border_color(hover))
        .cursor_pointer()
        .text_color(palette.text_dim)
        .child(icon(icons::MAGNIFYING_GLASS, 16.0, palette.text_dim))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .pl(px(8.))
                .text_size(px(14.))
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(if show_shortcut {
                    "Find tools and commands"
                } else {
                    "Find tools"
                }),
        )
        .when(show_shortcut, |pill| {
            pill.child(div().pl(px(14.)).text_size(px(12.)).child(shortcut_hint()))
        })
        .on_click(dispatch(CommandPalette));
    widgets::tooltip(pill, "Command palette (Ctrl+K)")
}

fn shortcut_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘K"
    } else {
        "Ctrl+K"
    }
}

/// What the Menu popup shows.
struct MenuState {
    app: WeakEntity<GodPdfApp>,
    show_tools: bool,
    doc_view: bool,
    /// A document is open (the home tab may be in front).
    has_doc: bool,
    /// Labels of the undo / redo steps of the document in front.
    undo_redo: (Option<SharedString>, Option<SharedString>),
    /// Recent files, and whether each was last seen on disk.
    recent: Vec<(PathBuf, bool)>,
    /// A closed tab can be opened again.
    reopen: bool,
}

impl MenuState {
    fn build(
        &self,
        menu: PopupMenu,
        window: &mut Window,
        cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        let palette = Palette::of(cx);
        let tools = if self.show_tools {
            icons::CHECK_SQUARE
        } else {
            icons::SQUARE
        };
        let enabled = self.doc_view;
        let toggle = PopupMenuItem::element(move |_, _| {
            entry(tools, "Tools panel".into(), "", enabled, &palette)
        })
        .action(Box::new(ToggleToolsPanel))
        .disabled(!enabled);
        let has_doc = self.has_doc;
        let (undo, redo) = self.undo_redo.clone();
        let recent = self.recent.clone();
        let reopen = self.reopen;
        let app = self.app.clone();
        menu.min_w(px(200.))
            .item(toggle)
            .separator()
            .submenu("File", window, cx, move |menu, window, cx| {
                let menu = command(menu, "Open PDF…", has_doc, cx);
                let menu = command(menu, "New PDF", has_doc, cx);
                let (recent, app) = (recent.clone(), app.clone());
                let menu = menu.submenu("Open recent", window, cx, move |menu, _, cx| {
                    if recent.is_empty() {
                        return menu.label("Nothing yet");
                    }
                    let palette = Palette::of(cx);
                    recent.iter().fold(menu, |menu, (path, exists)| {
                        let (app, path, exists) = (app.clone(), path.clone(), *exists);
                        let (name, folder) = name_and_folder(&path);
                        menu.item(
                            PopupMenuItem::element(move |_, _| {
                                recent_entry(name.clone(), folder.clone(), exists, &palette)
                            })
                            .disabled(!exists)
                            .on_click(move |_, _, cx| {
                                let _ = app.update(cx, |app, cx| app.open_path(&path, cx));
                            }),
                        )
                    })
                });
                let mut menu = command(menu, "Combine files…", has_doc, cx).separator();
                for label in [
                    "Save",
                    "Save As…",
                    "Print…",
                    "Extract pages…",
                    "Export as images…",
                ] {
                    menu = command(menu, label, has_doc, cx);
                }
                let menu = command(menu.separator(), "Close tab", has_doc, cx);
                let menu = entry_item(
                    menu,
                    icons::CLOCK_COUNTER_CLOCKWISE,
                    "Reopen closed tab".into(),
                    "Ctrl+Shift+T",
                    Box::new(ReopenTab),
                    reopen,
                    cx,
                );
                command(menu, "Quit", has_doc, cx)
            })
            .submenu("Edit", window, cx, move |mut menu, _, cx| {
                let steps = [
                    (
                        icons::ARROW_U_UP_LEFT,
                        "Undo",
                        "Ctrl+Z",
                        undo.clone(),
                        Box::new(app::Undo) as Box<dyn Action>,
                    ),
                    (
                        icons::ARROW_U_UP_RIGHT,
                        "Redo",
                        "Ctrl+Y",
                        redo.clone(),
                        Box::new(app::Redo),
                    ),
                ];
                for (glyph, name, keys, label, action) in steps {
                    let text: SharedString = match &label {
                        Some(label) => format!("{name} \u{201C}{label}\u{201D}").into(),
                        None => name.to_owned().into(),
                    };
                    menu = entry_item(menu, glyph, text, keys, action, label.is_some(), cx);
                }
                let palette = Box::new(CommandPalette);
                let menu = entry_item(
                    menu.separator(),
                    icons::COMMAND,
                    "Command palette".into(),
                    "Ctrl+K",
                    palette,
                    true,
                    cx,
                );
                entry_item(
                    menu,
                    icons::GEAR,
                    "Preferences…".into(),
                    "",
                    Box::new(crate::services::Preferences),
                    true,
                    cx,
                )
            })
            .submenu("View", window, cx, move |mut menu, _, cx| {
                for cmd in COMMANDS.iter().filter(|cmd| cmd.group == Group::View) {
                    menu = command(menu, cmd.label, has_doc, cx);
                }
                command(menu, "Organize pages", has_doc, cx)
            })
            .submenu("Tools", window, cx, move |menu, window, cx| {
                let groups = [
                    Group::Organize,
                    Group::Combine,
                    Group::Comment,
                    Group::Protect,
                    Group::Document,
                ];
                groups.into_iter().fold(menu, |menu, group| {
                    menu.submenu(group.title(), window, cx, move |mut menu, _, cx| {
                        for cmd in COMMANDS.iter().filter(|cmd| cmd.group == group) {
                            menu = command(menu, cmd.label, has_doc, cx);
                        }
                        menu
                    })
                })
            })
            .submenu("Help", window, cx, move |menu, _, cx| {
                let menu = command(menu, "Updates", has_doc, cx);
                command(menu, "About GodPDF", has_doc, cx)
            })
    }
}

/// A menu entry for the command labeled `label` in [`COMMANDS`], with its icon and
/// shortcut; greyed when it needs a document and none is open.
fn command(menu: PopupMenu, label: &str, has_doc: bool, cx: &mut Context<PopupMenu>) -> PopupMenu {
    let Some(cmd) = COMMANDS.iter().find(|cmd| cmd.label == label) else {
        return menu;
    };
    let (glyph, keys) = look(cmd.label);
    let enabled = has_doc || !cmd.needs_doc;
    entry_item(
        menu,
        glyph,
        cmd.label.into(),
        keys,
        (cmd.action)(),
        enabled,
        cx,
    )
}

fn entry_item(
    menu: PopupMenu,
    glyph: &'static str,
    label: SharedString,
    keys: &'static str,
    action: Box<dyn Action>,
    enabled: bool,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let palette = Palette::of(cx);
    menu.item(
        PopupMenuItem::element(move |_, _| entry(glyph, label.clone(), keys, enabled, &palette))
            .action(action)
            .disabled(!enabled),
    )
}

/// File name and its folder, for a recent file entry.
fn name_and_folder(path: &std::path::Path) -> (SharedString, SharedString) {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let folder = path
        .parent()
        .map(|folder| folder.display().to_string())
        .unwrap_or_default();
    (name.into(), folder.into())
}

/// A recent file entry: `FILE_PDF`, the name, then its folder dimmed, cut with "…" at 360.
fn recent_entry(name: SharedString, folder: SharedString, exists: bool, palette: &Palette) -> Div {
    let (color, dim) = if exists {
        (palette.text, palette.text_dim)
    } else {
        (palette.disabled(), palette.disabled())
    };
    div()
        .max_w(px(360.))
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(7.))
        .text_size(px(fonts::BODY))
        .child(icon(icons::FILE_PDF, fonts::BODY, color))
        .child(div().flex_none().text_color(color).child(name))
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_color(dim)
                .child(folder),
        )
}

/// A menu entry with a shortcut: glyph and label
/// at 13, the shortcut dimmed on the right.
fn entry(
    glyph: &'static str,
    label: SharedString,
    keys: &'static str,
    enabled: bool,
    palette: &Palette,
) -> Div {
    let color = if enabled {
        palette.text
    } else {
        palette.disabled()
    };
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(16.))
        .child(widgets::with_icon(glyph, label, fonts::BODY, color))
        .child(div().text_color(palette.text_dim).child(keys))
}

/// The icon and shortcut text of a command by label.
/// shortcut: keyed by label, move into `dialogs::Command` once it carries icons.
pub(crate) fn look(label: &str) -> (&'static str, &'static str) {
    match label {
        "Open PDF…" => (icons::FOLDER_OPEN, "Ctrl+O"),
        "New PDF" => (icons::FILE_PLUS, ""),
        "Save" => (icons::FLOPPY_DISK, "Ctrl+S"),
        "Undo" => (icons::ARROW_U_UP_LEFT, "Ctrl+Z"),
        "Redo" => (icons::ARROW_U_UP_RIGHT, "Ctrl+Y"),
        "Save As…" => (icons::FLOPPY_DISK_BACK, "Ctrl+Shift+S"),
        "Print…" => (icons::PRINTER, "Ctrl+P"),
        "Export as images…" => (icons::FILE_IMAGE, ""),
        "Close tab" => (icons::X, "Ctrl+W"),
        "Quit" => (icons::SIGN_OUT, ""),
        "Toggle dark mode" => (icons::CIRCLE_HALF, ""),
        "Zoom in" => (icons::MAGNIFYING_GLASS_PLUS, "Ctrl++"),
        "Zoom out" => (icons::MAGNIFYING_GLASS_MINUS, "Ctrl+-"),
        "Fit width" => (icons::ARROWS_OUT_LINE_HORIZONTAL, "Ctrl+0"),
        "Fit page" => (icons::FRAME_CORNERS, ""),
        "Toggle sidebar" => (icons::SIDEBAR_SIMPLE, ""),
        "Reopen closed tab" => (icons::CLOCK_COUNTER_CLOCKWISE, "Ctrl+Shift+T"),
        "Command palette" => (icons::COMMAND, "Ctrl+K"),
        "Search text" => (icons::MAGNIFYING_GLASS, "Ctrl+F"),
        "Bookmark this page" => (icons::BOOKMARK_SIMPLE, "Ctrl+D"),
        "Sign…" => (icons::SIGNATURE, ""),
        "Form field" => (icons::TEXTBOX, ""),
        "Fill in form…" => (icons::LIST_CHECKS, ""),
        "Go to page…" => (icons::HASH, "Ctrl+G"),
        "Next tab" => (icons::CARET_RIGHT, "Ctrl+Tab"),
        "Previous tab" => (icons::CARET_LEFT, "Ctrl+Shift+Tab"),
        "Rotate left" => (icons::ARROW_COUNTER_CLOCKWISE, ""),
        "Rotate right" => (icons::ARROW_CLOCKWISE, ""),
        "Rotate all pages" => (icons::ARROWS_CLOCKWISE, ""),
        "Delete pages" => (icons::TRASH, ""),
        "Organize pages" => (icons::SQUARES_FOUR, ""),
        "Side by side" => (icons::COLUMNS, ""),
        "Move earlier" => (icons::CARET_LEFT, "Alt+Left"),
        "Move later" => (icons::CARET_RIGHT, "Alt+Right"),
        "Insert from file…" => (icons::FILE_ARROW_DOWN, ""),
        "Remove blank pages" => (icons::FILE_X, ""),
        "Interleave with…" => (icons::SHUFFLE, ""),
        "Insert blank page" => (icons::FILE_PLUS, ""),
        "Extract pages…" => (icons::EXPORT, ""),
        "Split…" => (icons::SCISSORS, ""),
        "Reverse page order" => (icons::SORT_DESCENDING, ""),
        "Crop pages…" => (icons::CROP, ""),
        "Change page size…" => (icons::RESIZE, ""),
        "Cut pages in half…" => (icons::SQUARE_SPLIT_HORIZONTAL, ""),
        "Pages per sheet…" => (icons::GRID_FOUR, ""),
        "Make a booklet" => (icons::BOOK_OPEN, ""),
        "Join into one long page" => (icons::ROWS, ""),
        "Number pages…" => (icons::LIST_NUMBERS, ""),
        "Combine files…" => (icons::STACK, ""),
        "Select" => (icons::CURSOR, "V"),
        "Hand" => (icons::HAND, "H"),
        "Highlight" => (icons::HIGHLIGHTER, ""),
        "Sticky note" => (icons::NOTE, "N"),
        "Free text" => (icons::TEXT_T, "T"),
        "Rectangle" => (icons::RECTANGLE, "R"),
        "Ink" => (icons::PENCIL_SIMPLE, "D"),
        "Picture" => (icons::IMAGE, "I"),
        "Eraser" => (icons::ERASER, "E"),
        "Bring to front" => (icons::ARROW_CIRCLE_UP, ""),
        "Bring forward" => (icons::ARROW_UP, ""),
        "Send backward" => (icons::ARROW_DOWN, ""),
        "Send to back" => (icons::STACK_SIMPLE, ""),
        "Encrypt with password…" => (icons::LOCK, ""),
        "Remove password" => (icons::LOCK_OPEN, ""),
        "Remove hidden data" => (icons::BROOM, ""),
        "Sign with certificate…" => (icons::SIGNATURE, ""),
        "Signatures…" => (icons::CHECK_CIRCLE, ""),
        "Properties…" => (icons::INFO, ""),
        "Add watermark…" => (icons::STAMP, ""),
        "Remove all links" => (icons::LINK_BREAK, ""),
        "Contents page" => (icons::LIST_BULLETS, ""),
        "Page colours…" => (icons::PALETTE, ""),
        "Remove all comments" => (icons::ERASER, ""),
        "Flatten comments and forms" => (icons::STACK_SIMPLE, ""),
        "Attached files…" => (icons::PAPERCLIP, ""),
        "Compress" => (icons::ARROWS_IN, ""),
        "Compare with…" => (icons::GIT_DIFF, ""),
        "Recognize text (OCR)" => (icons::SCAN, ""),
        "Edit PDF text" => (icons::TEXT_AA, ""),
        "About GodPDF" => (icons::QUESTION, ""),
        "Updates" => (icons::ARROW_CIRCLE_UP, ""),
        _ => (icons::FILE, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_has_its_own_icon() {
        let missing: Vec<&str> = crate::dialogs::COMMANDS
            .iter()
            .map(|command| command.label)
            .filter(|label| look(label).0 == icons::FILE)
            .collect();
        assert!(missing.is_empty(), "no icon for {missing:?}");
    }
}
