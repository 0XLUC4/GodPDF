//! Command palette (Ctrl+K): type to find any command by name or description, Up / Down to
//! choose, Enter or a click to run.

use gpui::component::input::{Input, InputEvent, InputState};
use gpui::component::kbd::Kbd;
use gpui::prelude::*;
use gpui::{AnyElement, App, Entity, ScrollHandle, Subscription, Window, div, px};

use super::commands::{self, Command};
use super::{
    CommandPalette, DialogCancel, DialogConfirm, Open, PaletteDown, PaletteUp, card, close, opened,
    state,
};
use crate::theme::Palette;
use crate::widgets;

pub struct PaletteState {
    pub query: Entity<InputState>,
    selected: usize,
    scroll: ScrollHandle,
    _changed: Subscription,
}

pub fn new(window: &mut Window, cx: &mut App) -> PaletteState {
    let query = cx.new(|cx| InputState::new(window, cx).placeholder("Find tools and commands"));
    // A new query starts again from its best match.
    let changed = App::subscribe(cx, &query, |_, event: &InputEvent, cx: &mut App| {
        if matches!(event, InputEvent::Change) {
            with_palette(cx, |palette| palette.selected = 0);
        }
    });
    PaletteState {
        query,
        selected: 0,
        scroll: ScrollHandle::new(),
        _changed: changed,
    }
}

fn with_palette(cx: &mut App, change: impl FnOnce(&mut PaletteState)) {
    if let Some(Open::Palette(palette)) = &mut state(cx).open {
        change(palette);
    }
}

fn matches(palette: &PaletteState, cx: &App) -> Vec<&'static Command> {
    commands::search(&palette.query.read(cx).value())
}

fn step(down: bool, window: &mut Window, cx: &mut App) {
    let Some(Open::Palette(palette)) = opened(cx) else {
        return;
    };
    let count = matches(palette, cx).len();
    with_palette(cx, |palette| {
        palette.selected = if down {
            (palette.selected + 1).min(count.saturating_sub(1))
        } else {
            palette.selected.saturating_sub(1)
        };
        palette.scroll.scroll_to_item(palette.selected);
    });
    window.refresh();
}

/// Close the palette, then run `command` from where the focus was before it opened.
fn run(command: &'static Command, window: &mut Window, cx: &mut App) {
    close(window, cx);
    window.dispatch_action((command.action)(), cx);
}

fn run_selected(window: &mut Window, cx: &mut App) {
    let Some(Open::Palette(palette)) = opened(cx) else {
        return;
    };
    let command = matches(palette, cx).get(palette.selected).copied();
    match command {
        // Greyed (needs a document): the hint under the list already says so.
        Some(command) if command.needs_doc && super::active_tab(cx).is_none() => {}
        Some(command) => run(command, window, cx),
        None => close(window, cx),
    }
}

/// The icon and tint of a command in the palette.
fn look(command: &Command) -> (&'static str, u32) {
    use crate::theme::{AMBER, BLUE, GREEN, PINK, PURPLE, RED, TEAL};
    use commands::Group;
    let icon = crate::shell::header::look(command.label).0;
    let tint = match (command.label, command.group) {
        (
            "Delete pages" | "Remove blank pages" | "Remove all comments" | "Remove hidden data",
            _,
        ) => RED,
        ("Extract pages…" | "Export as images…", _) => TEAL,
        ("Properties…", _) => BLUE,
        (_, Group::Organize) => GREEN,
        (_, Group::Combine) => PURPLE,
        (_, Group::Comment) => AMBER,
        (_, Group::Protect | Group::Edit | Group::Help) => BLUE,
        (_, Group::File | Group::View) => TEAL,
        (_, Group::Document) => PINK,
    };
    (icon, tint)
}

/// Magnifier + field, rows of 36 (icon 20 in its tint centered 22
/// from the left, label 15 at 46, shortcut and family 13 dimmed 14 from the right), a
/// separator, then the hint (13, dimmed).
pub fn render(
    palette: &PaletteState,
    has_doc: bool,
    colors: &Palette,
    window: &Window,
    cx: &App,
) -> AnyElement {
    let colors = *colors;
    let found = matches(palette, cx);
    let selected = palette.selected.min(found.len().saturating_sub(1));
    let rows = found.iter().enumerate().map(|(index, command)| {
        let command: &'static Command = command;
        let ready = has_doc || !command.needs_doc;
        // The document's keys (tools, Alt+arrows) do not apply in the palette's own field.
        let action = (command.action)();
        let shortcut = Kbd::binding_for_action(&*action, Some("GodPdf"), window)
            .or_else(|| Kbd::binding_for_action(&*action, None, window));
        let (glyph, tint) = look(command);
        let (ink, tint) = if ready {
            (colors.text, crate::theme::hex(tint))
        } else {
            (colors.text_dim, colors.text_dim)
        };
        let hover = colors.hover;
        div()
            .id(("command", index))
            .flex_none()
            .flex()
            .items_center()
            .h(px(36.))
            .pr(px(14.))
            .rounded(px(8.))
            .when(index == selected, |row| row.bg(colors.selected))
            .when(index != selected && ready, |row| {
                row.hover(move |style| style.bg(hover))
            })
            .when(ready, |row| {
                row.cursor_pointer()
                    .on_click(move |_, window, cx| run(command, window, cx))
            })
            .child(
                div()
                    .w(px(44.))
                    .flex_none()
                    .flex()
                    .justify_center()
                    .child(widgets::icon(glyph, 20.0, tint)),
            )
            .child(
                div()
                    .flex_1()
                    .pl(px(2.))
                    .text_size(px(15.))
                    .text_color(ink)
                    .child(command.label),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(20.))
                    .text_size(px(13.))
                    .text_color(colors.text_dim)
                    .children(shortcut.map(|kbd| kbd.appearance(false)))
                    .child(command.group.title()),
            )
    });
    let hint = match found.get(selected) {
        Some(command) if command.needs_doc && !has_doc => "Open a PDF first",
        Some(command) if !command.description.is_empty() => command.description,
        _ => "",
    };
    card(&colors, 620.)
        .key_context("CommandPalette")
        .on_action(|_: &DialogConfirm, window, cx| run_selected(window, cx))
        .on_action(|_: &DialogCancel, window, cx| close(window, cx))
        .on_action(|_: &CommandPalette, window, cx| close(window, cx))
        .on_action(|_: &PaletteUp, window, cx| step(false, window, cx))
        .on_action(|_: &PaletteDown, window, cx| step(true, window, cx))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(crate::theme::ITEM_SPACING.0))
                .child(widgets::icon(
                    crate::icons::MAGNIFYING_GLASS,
                    20.0,
                    colors.text_dim,
                ))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(18.))
                        .child(Input::new(&palette.query)),
                ),
        )
        .child(
            div()
                .id("command-list")
                .flex()
                .flex_col()
                .gap(px(2.))
                .max_h(px(420.))
                .overflow_y_scroll()
                .track_scroll(&palette.scroll)
                .children(rows)
                .when(found.is_empty(), |list| {
                    list.child(
                        div()
                            .text_color(colors.text_dim)
                            .child("No matching command"),
                    )
                }),
        )
        .when(!found.is_empty(), |card| {
            card.child(div().h(px(1.)).bg(colors.border)).child(
                // Fixed height: rows without a description leave it empty, without a jump.
                div()
                    .h(px(21.))
                    .text_size(px(13.))
                    .text_color(colors.text_dim)
                    .child(hint),
            )
        })
        .into_any_element()
}
