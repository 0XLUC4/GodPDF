//! Header row (egui `GodApp::header_row`, `mode_tab`, `search_field`): Menu, the mode tabs,
//! then on the right the search pill and properties / save / export / print. Fill `bg`,
//! margin 10 × 0, height 46, a `border` line under it (egui panel separator).

use gpui::component::button::{Button, ButtonVariants};
use gpui::component::menu::DropdownMenu;
use gpui::prelude::*;
use gpui::{AnyElement, Context, Div, Stateful, div, px};

use super::{HEADER_HEIGHT, Mode, ToggleToolsPanel};
use crate::app::{GodPdfApp, Save, SaveAs};
use crate::dialogs::{self, CommandPalette, Print, Properties};
use crate::theme::Palette;
use crate::widgets::{self, dispatch, icon};
use crate::{fonts, icons};

/// Search pill: egui clamps `available - 40` to 140..=300.
const SEARCH_MAX: f32 = 300.0;
const SEARCH_MIN: f32 = 140.0;

pub fn render(app: &GodPdfApp, palette: &Palette, cx: &mut Context<GodPdfApp>) -> AnyElement {
    let doc_view = app.doc_view();
    let show_tools = app.shell.show_tools;
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
        .dropdown_menu(move |menu, window, cx| {
            let menu = menu
                .min_w(px(200.))
                .menu_with_check_and_disabled(
                    "Tools panel",
                    show_tools,
                    Box::new(ToggleToolsPanel),
                    !doc_view,
                )
                .separator();
            // TODO(header): egui's menus are File / Edit / View / Tools / Help with icons,
            // shortcuts and "Open recent"; this lists the GPUI command families.
            dialogs::menu_groups(menu, window, cx)
        });

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
        (icons::EXPORT, "Save a copy… (Ctrl+Shift+S)", "save-as"),
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

    // TODO(header): spinner "Working…" left of the pill while a background job runs.
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
        .child(div().w(px(14.)))
        .child(div().flex().gap(px(2.)).children(tabs))
        .child(
            div()
                .ml_auto()
                .pl(px(40.))
                .flex()
                .items_center()
                .gap(px(6.))
                .child(search_field(palette))
                .child(div().w(px(0.)))
                .children(buttons),
        )
        .into_any_element()
}

/// Text tab with a 2 px underline when active (egui `mode_tab`): text 15 + 11 px each side,
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

/// Pill that looks like a search box and opens the command palette (egui `search_field`):
/// 32 high, radius 16, `bg` fill, 1 px `border` (`text_dim` on hover), magnifier 15 centered
/// 18 px in, hint (14, dim) at 34 px, shortcut (12, dim) 14 px from the right.
fn search_field(palette: &Palette) -> Stateful<Div> {
    let hover = palette.text_dim;
    let pill = div()
        .id("search-pill")
        .flex_1()
        .min_w(px(SEARCH_MIN))
        .w(px(SEARCH_MAX))
        .max_w(px(SEARCH_MAX))
        .h(px(32.))
        .flex()
        .items_center()
        .pl(px(18. - 15. / 2.))
        .pr(px(14.))
        .rounded(px(16.))
        .bg(palette.bg)
        .border_1()
        .border_color(palette.border)
        .hover(move |style| style.border_color(hover))
        .cursor_text()
        .text_color(palette.text_dim)
        .child(icon(icons::MAGNIFYING_GLASS, 15.0, palette.text_dim))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .pl(px(34. - 18. - 15. / 2.))
                .text_size(px(14.))
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child("Find tools and commands"),
        )
        .child(div().pl(px(14.)).text_size(px(12.)).child(shortcut_hint()))
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
