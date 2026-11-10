//! Command palette (Ctrl+K): type to find any command by name or description, Up / Down to
//! choose, Enter or a click to run (ported from `command_palette` in `crates/app/src/dialogs.rs`).

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
        Some(command) => run(command, window, cx),
        None => close(window, cx),
    }
}

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
        let shortcut = Kbd::binding_for_action(&*(command.action)(), None, window);
        let hover = colors.hover;
        div()
            .id(("command", index))
            .flex()
            .items_center()
            .gap(px(10.))
            .h(px(36.))
            .px(px(12.))
            .rounded(px(8.))
            .cursor_pointer()
            .text_color(if ready { colors.text } else { colors.text_dim })
            .when(index == selected, |row| row.bg(colors.selected))
            .when(index != selected, |row| {
                row.hover(move |style| style.bg(hover))
            })
            .on_click(move |_, window, cx| run(command, window, cx))
            .child(div().flex_1().child(command.label))
            .children(shortcut)
            .child(
                div()
                    .w(px(76.))
                    .text_right()
                    .text_sm()
                    .text_color(colors.text_dim)
                    .child(command.group.title()),
            )
    });
    let hint = match found.get(selected) {
        Some(command) if command.needs_doc && !has_doc => "Open a PDF first",
        Some(command) if !command.description.is_empty() => command.description,
        Some(_) => "Enter runs it",
        None => "No matching command",
    };
    card(&colors, 620.)
        .key_context("CommandPalette")
        .on_action(|_: &DialogConfirm, window, cx| run_selected(window, cx))
        .on_action(|_: &DialogCancel, window, cx| close(window, cx))
        .on_action(|_: &CommandPalette, window, cx| close(window, cx))
        .on_action(|_: &PaletteUp, window, cx| step(false, window, cx))
        .on_action(|_: &PaletteDown, window, cx| step(true, window, cx))
        .child(Input::new(&palette.query))
        .child(
            div()
                .id("command-list")
                .flex()
                .flex_col()
                .gap(px(2.))
                .max_h(px(420.))
                .overflow_y_scroll()
                .track_scroll(&palette.scroll)
                .children(rows),
        )
        .child(
            div()
                .pt(px(6.))
                .border_t_1()
                .border_color(colors.border)
                .text_sm()
                .text_color(colors.text_dim)
                .child(hint),
        )
        .into_any_element()
}
