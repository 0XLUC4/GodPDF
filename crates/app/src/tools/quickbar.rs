//! The quick bar floating over the viewer's top-left corner: tool toggles, undo / redo, and
//! next to it the style card of what is being made or is selected (colours, width, font,
//! formats), or the property card of the selected picture.

use godpdf_core::{Annotation, CharStyle, Color, Font, StampLabel};
use gpui::component::Sizable;
use gpui::component::button::Button;
use gpui::component::input::{Input, InputEvent, InputState};
use gpui::component::menu::{DropdownMenu, PopupMenuItem};
use gpui::prelude::*;
use gpui::{
    Action, AnyElement, App, ClickEvent, Context, Div, ElementId, Entity, EntityId, PromptLevel,
    SharedString, Stateful, Subscription, Window, div, px,
};

use super::canvas::{self, rewrite};
use super::geom::{self, DRect, PictureField};
use super::picker::{self, Column, color_to_rgb};
use super::{
    Format, SWATCHES, Style, Tool, common_style, current_tool, editor, hsla, toggle_all,
    tool_state, with_active,
};
use crate::doc::DocTab;
use crate::theme::{self, Palette};
use crate::{fonts, icons, toasts, widgets};

/// Distance of the bar from the viewer's top-left corner.
const INSET: f32 = 12.0;
/// Room kept free on the viewer's right: the inset and the page scroll bar.
const RIGHT_CLEAR: f32 = INSET + 14.0;
/// Button side: tool toggles, format toggles, card columns; glyphs 18 in them.
const BUTTON: f32 = 32.0;
const GLYPH: f32 = 18.0;
/// Card padding and gap between the buttons of the bar; radius = button radius + padding.
const PAD: f32 = 6.0;
const BAR_GAP: f32 = 4.0;
const BUTTON_RADIUS: f32 = 6.0;
const CARD_RADIUS: f32 = BUTTON_RADIUS + PAD;
/// Between the bar and the cards beside it, and between the style card's columns.
const CARD_GAP: f32 = 8.0;
const COLUMN_GAP: f32 = 4.0;
/// Picture card: the glyph naming a field, and the field.
const LABEL_WIDTH: f32 = 20.0;
const FIELD_WIDTH: f32 = 96.0;

/// The fields of the picture card (created once, reused for every picture).
pub struct PictureCard {
    fields: Vec<(PictureField, Entity<InputState>)>,
    /// What the fields show: tab, page, annotation id, document generation.
    shown: Option<(EntityId, usize, String, u64)>,
    _subscriptions: Vec<Subscription>,
}

/// Which style controls a target has.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Colour + font size + font.
    Text,
    /// Colour + stroke width.
    Stroke,
    /// Colour + stroke width + fill colour (rectangles, ellipses).
    Shape,
    /// Colour only.
    Fill,
}

/// What the style card edits.
enum Target {
    /// New comments of this tool.
    Tool(Tool),
    /// The selected annotation: a copy of its style, and its formats for a text.
    Selected(Kind, Style, Option<CharStyle>),
    /// The text box being typed.
    TextBox(CharStyle),
    Picture {
        page: usize,
        id: String,
        rect: DRect,
        natural: (f32, f32),
    },
}

#[derive(Clone, Copy)]
enum Change {
    Color(Color),
    Fill(Option<Color>),
    /// One step up (+1) or down (-1) of the font size or line width.
    Step(f32),
    Font(Font),
    Format(Format),
}

fn target(tab: &mut DocTab, cx: &mut App) -> Target {
    if let Some(editor) = &tab.canvas.editor {
        return match editor.kind {
            editor::Kind::Text { .. } => Target::TextBox(editor.active(cx)),
            editor::Kind::Note { .. } => Target::Tool(Tool::Note),
        };
    }
    let style = |color: Color, width: f32, font_size: f32, font: Font| Style {
        color,
        width,
        font_size,
        font,
        fill: None,
    };
    match canvas::selected_item(tab, cx) {
        Some((page, item)) => match item.annotation {
            Some(Annotation::Image { picture, .. }) => Target::Picture {
                page,
                id: item.id,
                rect: item.rect,
                natural: canvas::natural_size(&picture),
            },
            Some(Annotation::FreeText {
                font_size,
                color,
                font,
                text,
                styles,
                ..
            }) => Target::Selected(
                Kind::Text,
                style(color, 2.0, font_size, font),
                Some(common_style(&styles, 0..text.chars().count())),
            ),
            Some(
                Annotation::Square {
                    color, width, fill, ..
                }
                | Annotation::Circle {
                    color, width, fill, ..
                },
            ) => Target::Selected(
                Kind::Shape,
                Style {
                    fill,
                    ..style(color, width, 14.0, Font::Helvetica)
                },
                None,
            ),
            Some(Annotation::Line { color, width, .. } | Annotation::Ink { color, width, .. }) => {
                Target::Selected(
                    Kind::Stroke,
                    style(color, width, 14.0, Font::Helvetica),
                    None,
                )
            }
            Some(
                Annotation::Highlight { color, .. }
                | Annotation::Underline { color, .. }
                | Annotation::StrikeOut { color, .. },
            ) => Target::Selected(Kind::Fill, style(color, 2.0, 14.0, Font::Helvetica), None),
            Some(Annotation::Note { .. } | Annotation::Stamp { .. }) | None => {
                Target::Tool(current_tool(cx))
            }
        },
        None => Target::Tool(current_tool(cx)),
    }
}

fn change_style(style: &mut Style, kind: Kind, change: Change) {
    match change {
        Change::Color(color) => style.color = color,
        Change::Fill(fill) => style.fill = fill,
        Change::Step(sign) if kind == Kind::Text => {
            style.font_size = (style.font_size + sign).clamp(6.0, 72.0);
        }
        Change::Step(sign) => style.width = (style.width + sign * 0.5).clamp(0.5, 12.0),
        Change::Font(font) => style.font = font,
        Change::Format(_) => {}
    }
}

/// Apply a click of the style card to whatever it edits.
fn apply(tab: &mut DocTab, change: Change, window: &mut Window, cx: &mut Context<DocTab>) {
    match target(tab, cx) {
        Target::TextBox(_) => match change {
            Change::Format(format) => editor::toggle_format(tab, format, window, cx),
            _ => {
                change_style(editor::text_style_mut(tab, cx), Kind::Text, change);
                editor::refocus(tab, window, cx);
            }
        },
        Target::Selected(kind, mut style, _) => {
            let Some(selected) = tab.canvas.selected.clone() else {
                return;
            };
            let before = style;
            change_style(&mut style, kind, change);
            // The current swatch, a step at its limit: nothing to write.
            if style == before && !matches!(change, Change::Format(_)) {
                return;
            }
            let restyle =
                move |annotation: &Annotation, _: &_, _: &_, _| restyled(annotation, style, change);
            rewrite(tab, selected.page, &selected.id, "Restyle", restyle, cx);
        }
        // The custom colour picked in the redact card.
        Target::Tool(Tool::Redact) => {
            if let Change::Color(color) | Change::Fill(Some(color)) = change {
                tool_state(cx).redact_cover = Some(color);
            }
        }
        Target::Tool(tool) => {
            let kind = tool_kind(tool).unwrap_or(Kind::Text);
            change_style(tool_state(cx).styles.of_mut(tool), kind, change);
        }
        Target::Picture { .. } => {}
    }
    cx.notify();
}

/// `annotation` with the style card's `style` (and format `change` for a text).
fn restyled(annotation: &Annotation, style: Style, change: Change) -> Annotation {
    let color = style.color;
    match annotation.clone() {
        Annotation::Highlight { rect, quads, .. } => Annotation::Highlight { rect, color, quads },
        Annotation::Square { rect, .. } => Annotation::Square {
            rect,
            color,
            width: style.width,
            fill: style.fill,
        },
        Annotation::Circle { rect, .. } => Annotation::Circle {
            rect,
            color,
            width: style.width,
            fill: style.fill,
        },
        Annotation::Line {
            start, end, arrow, ..
        } => Annotation::Line {
            start,
            end,
            color,
            width: style.width,
            arrow,
        },
        Annotation::Underline { rect, quads, .. } => Annotation::Underline { rect, color, quads },
        Annotation::StrikeOut { rect, quads, .. } => Annotation::StrikeOut { rect, color, quads },
        Annotation::Ink { strokes, .. } => Annotation::Ink {
            strokes,
            color,
            width: style.width,
        },
        Annotation::FreeText {
            rect,
            text,
            rotation,
            mut styles,
            ..
        } => {
            if let Change::Format(format) = change {
                let len = text.chars().count();
                toggle_all(&mut styles, len, 0..len, format);
            }
            // Same top edge and width; core refits the height to the wrapped text.
            Annotation::FreeText {
                rect,
                text,
                font_size: style.font_size,
                color,
                rotation,
                font: style.font,
                styles,
            }
        }
        other @ (Annotation::Note { .. } | Annotation::Image { .. } | Annotation::Stamp { .. }) => {
            other
        }
    }
}

fn tool_kind(tool: Tool) -> Option<Kind> {
    match tool {
        Tool::Text => Some(Kind::Text),
        Tool::Ink | Tool::Line | Tool::Arrow => Some(Kind::Stroke),
        Tool::Rectangle | Tool::Ellipse => Some(Kind::Shape),
        shape if shape.is_shape() => Some(Kind::Stroke),
        Tool::Highlight | Tool::Underline | Tool::StrikeOut => Some(Kind::Fill),
        _ => None,
    }
}

/// A card: panel fill, 1 px border, radius 12, padding 6, shadow 0/4/16 at
/// black alpha 28 (both themes).
pub(super) fn card(palette: &Palette) -> Div {
    div()
        .flex_none()
        .p(px(PAD))
        .rounded(px(CARD_RADIUS))
        .shadow(widgets::shadow(4.0, 16.0, 28.0 / 255.0))
        .bg(palette.panel)
        .border_1()
        .border_color(palette.border)
        .text_color(palette.text)
        // Clicks stay on the card; the wheel still scrolls the pages under it.
        .block_mouse_except_scroll()
}

/// Divider between groups: a 1 px line, 6 px shorter than a button each side (the column
/// gap spaces it).
fn divider(palette: &Palette) -> Div {
    div()
        .flex_none()
        .w(px(BUTTON - 12.))
        .h(px(1.))
        .mx(px(6.))
        .bg(palette.border)
}

/// Square icon button; `active` gets the accent tint (blue glyph on a light blue square),
/// others the hover fill under the pointer, disabled ones are dimmed.
fn icon_button(
    id: impl Into<ElementId>,
    content: impl IntoElement,
    active: bool,
    enabled: bool,
    palette: &Palette,
) -> Stateful<Div> {
    let hover = palette.hover;
    let tint = theme::hex(theme::BLUE);
    div()
        .id(id)
        .flex_none()
        .size(px(BUTTON))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(BUTTON_RADIUS))
        .text_size(px(GLYPH))
        .line_height(px(GLYPH))
        .text_color(match (active, enabled) {
            (true, _) => tint,
            (false, true) => palette.text,
            (false, false) => palette.disabled(),
        })
        .when(active, |button| button.bg(tint.opacity(0.16)))
        .when(enabled && !active, |button| {
            button.cursor_pointer().hover(move |style| style.bg(hover))
        })
        .child(content)
}

fn dispatch(action: Box<dyn Action>) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx)
}

pub fn render(
    tab: &mut DocTab,
    window: &mut Window,
    cx: &mut Context<DocTab>,
) -> Option<AnyElement> {
    tool_state(cx).active = Some(cx.entity().downgrade());
    if tab.page_count() == 0 {
        return None;
    }
    let palette = Palette::of(cx);
    let tool = current_tool(cx);
    let can_undo = tab.session.undo_label().is_some();
    let can_redo = tab.session.redo_label().is_some();
    let max_height = max_height(tab);

    // Too short a viewer: the bar scrolls.
    let mut tools = card(&palette)
        .id("quick-tools")
        .flex()
        .flex_col()
        .gap(px(BAR_GAP))
        .max_h(px(max_height))
        .overflow_y_scroll();
    let (shape, markup) = (tool_state(cx).shape, tool_state(cx).markup);
    for candidate in Tool::BAR {
        // A group button stands for the last tool picked in its group.
        let (candidate, active) = match candidate.group() {
            Some(group) if candidate.is_shape() => (shape, group.contains(&tool)),
            Some(group) => (markup, group.contains(&tool)),
            None => (candidate, candidate == tool),
        };
        // Groups: navigate | comment and insert | remove.
        if matches!(candidate, Tool::Text | Tool::Redact) {
            tools = tools.child(divider(&palette));
        }
        tools = tools.child(widgets::tooltip(
            icon_button(
                SharedString::from(format!("tool-{:?}", candidate)),
                widgets::glyph(candidate.icon()),
                active,
                true,
                &palette,
            )
            .on_click(dispatch(candidate.action())),
            candidate.tooltip(),
        ));
    }
    let undo_redo = |id: &'static str, glyph, enabled| {
        icon_button(id, widgets::glyph(glyph), false, enabled, &palette)
    };
    let mut undo = undo_redo("quick-undo", icons::ARROW_U_UP_LEFT, can_undo);
    if can_undo {
        undo = undo.on_click(dispatch(Box::new(crate::app::Undo)));
    }
    let mut redo = undo_redo("quick-redo", icons::ARROW_U_UP_RIGHT, can_redo);
    if can_redo {
        redo = redo.on_click(dispatch(Box::new(crate::app::Redo)));
    }
    tools = tools
        .child(divider(&palette))
        .child(widgets::tooltip(undo, "Undo (Ctrl+Z)"))
        .child(widgets::tooltip(redo, "Redo (Ctrl+Y)"));

    let style_card = style_card(tab, &palette, window, cx);
    Some(
        div()
            .absolute()
            .top(px(INSET))
            .left(px(INSET))
            .flex()
            .items_start()
            .gap(px(CARD_GAP))
            .child(tools)
            .children(style_card)
            .into_any_element(),
    )
}

/// Tallest a card may be: the viewer less the inset above and below (cards scroll past it).
fn max_height(tab: &DocTab) -> f32 {
    (f32::from(tab.view.bounds.size.height) - 2.0 * INSET).max(BUTTON * 3.0)
}

/// Width of a card of `columns` columns.
fn card_width(columns: usize) -> f32 {
    let columns = columns as f32;
    columns * BUTTON + (columns - 1.0).max(0.0) * COLUMN_GAP + 2.0 * PAD + 2.0
}

fn style_card(
    tab: &mut DocTab,
    palette: &Palette,
    window: &mut Window,
    cx: &mut Context<DocTab>,
) -> Option<AnyElement> {
    let (kind, style, formats) = match target(tab, cx) {
        Target::Tool(tool) => match tool_kind(tool) {
            Some(kind) => (kind, tool_state(cx).styles.of(tool), None),
            None if tool == Tool::Stamp => return Some(stamp_card(palette, cx)),
            None if tool == Tool::Redact => {
                return Some(redact_card(tab, palette, window, cx));
            }
            None if tool == Tool::Eraser => return Some(eraser_card(palette, cx)),
            None => return None,
        },
        Target::Selected(kind, style, formats) => (kind, style, formats),
        Target::TextBox(format) => (Kind::Text, editor::text_style(tab, cx), Some(format)),
        Target::Picture {
            page,
            id,
            rect,
            natural,
        } => return picture_card(tab, page, id, rect, natural, palette, window, cx),
    };
    // Using a tool of a group (shapes, text markup): pick another one, Adobe style, in the
    // same card.
    let group = match target(tab, cx) {
        Target::Tool(tool) => tool.group().map(|group| (tool, group)),
        _ => None,
    };
    let picker_open = tool_state(cx).picker.open;
    let on = |change: Change| {
        cx.listener(move |tab, _: &ClickEvent, window, cx| apply(tab, change, window, cx))
    };

    let swatch_column = |column: Column| {
        // Presets, then the custom swatch: the current colour when it is none of them.
        let (current, id) = match column {
            Column::Stroke => (Some(style.color), "swatch"),
            Column::Fill => (style.fill, "fill"),
        };
        let (glyph, name) = match (column, kind) {
            (Column::Fill, _) => (icons::PAINT_BUCKET, "Fill"),
            (Column::Stroke, Kind::Shape) => (icons::PEN, "Stroke"),
            (Column::Stroke, _) => (icons::DROP, "Colour"),
        };
        let mut swatches = column_of(id, glyph, name, palette);
        if column == Column::Fill {
            swatches = swatches.child(widgets::tooltip(
                swatch("fill-none", None, style.fill.is_none(), palette)
                    .on_click(on(Change::Fill(None))),
                "No fill",
            ));
        }
        for (index, (color, name)) in SWATCHES.into_iter().enumerate() {
            let change = match column {
                Column::Stroke => Change::Color(color),
                Column::Fill => Change::Fill(Some(color)),
            };
            swatches = swatches.child(widgets::tooltip(
                swatch(
                    (id, index),
                    Some(color),
                    current.is_some_and(|current| same_color(color, current)),
                    palette,
                )
                .on_click(on(change)),
                name,
            ));
        }
        let custom = current.filter(|&current| {
            !SWATCHES
                .iter()
                .any(|&(preset, _)| same_color(preset, current))
        });
        let shown = current.map_or(0xFF_FF_FF, color_to_rgb);
        swatches.child(widgets::tooltip(
            custom_swatch(
                (id, SWATCHES.len()),
                custom,
                picker_open == Some(column),
                palette,
            )
            .on_click(move |_: &ClickEvent, _, cx: &mut App| {
                picker::toggle(column, shown, cx);
                cx.refresh_windows();
            }),
            match custom {
                Some(color) => format!("Custom colour {}", picker::format_hex(color_to_rgb(color))),
                None => "Custom colour…".to_owned(),
            },
        ))
    };
    let colors = swatch_column(Column::Stroke);
    let fills = (kind == Kind::Shape).then(|| swatch_column(Column::Fill));
    let mut controls = match kind {
        Kind::Text => column_of("size", icons::TEXT_AA, "Size", palette),
        _ => column_of("width", icons::RULER, "Width", palette),
    }
    .gap(px(4.));
    match kind {
        Kind::Text => {
            controls = stepper(
                controls,
                format!("{:.0}", style.font_size),
                "font size",
                palette,
                &on,
            );
            controls = controls
                .child(divider(palette))
                .child(font_menu(style.font, palette));
            if let Some(active) = formats {
                controls = controls.child(divider(palette));
                for format in super::Format::ALL {
                    let (icon, tooltip) = match format {
                        Format::Bold => (icons::TEXT_B, "Bold (Ctrl+B)"),
                        Format::Italic => (icons::TEXT_ITALIC, "Italic (Ctrl+I)"),
                        Format::Underline => (icons::TEXT_UNDERLINE, "Underline (Ctrl+U)"),
                        Format::Sub => (icons::TEXT_SUBSCRIPT, "Subscript"),
                        Format::Super => (icons::TEXT_SUPERSCRIPT, "Superscript"),
                    };
                    controls = controls.child(widgets::tooltip(
                        icon_button(
                            SharedString::from(format!("format-{format:?}")),
                            widgets::glyph(icon),
                            format.is_on(active),
                            true,
                            palette,
                        )
                        .on_click(on(Change::Format(format))),
                        tooltip,
                    ));
                }
            }
        }
        Kind::Stroke | Kind::Shape => {
            controls = stepper(
                controls,
                format!("{:.1}", style.width),
                "line width",
                palette,
                &on,
            );
        }
        Kind::Fill => {}
    }
    let group = group.map(|(tool, group)| {
        let mut column = column_of("type", icons::SHAPES, "Type", palette);
        for &member in group {
            column = column.child(widgets::tooltip(
                icon_button(
                    SharedString::from(format!("group-{member:?}")),
                    widgets::glyph(member.icon()),
                    member == tool,
                    true,
                    palette,
                )
                .on_click(dispatch(member.action())),
                member.tooltip(),
            ));
        }
        column
    });
    let columns = usize::from(group.is_some())
        + 1
        + usize::from(fills.is_some())
        + usize::from(kind != Kind::Fill);
    let style_card = card(palette)
        .id("style-card")
        .flex()
        .items_start()
        .gap(px(COLUMN_GAP))
        .max_h(px(max_height(tab)))
        .overflow_y_scroll()
        .children(group)
        .child(colors)
        .children(fills)
        .when(kind != Kind::Fill, |card| card.child(controls));
    let Some(column) = picker_open.filter(|&open| open == Column::Stroke || kind == Kind::Shape)
    else {
        return Some(style_card.into_any_element());
    };
    let picker = picker::card(column, palette, window, cx);
    // No room for both beside the bar: the picker stands in for the style card.
    let room = f32::from(tab.view.bounds.size.width) - INSET - RIGHT_CLEAR;
    let needed = card_width(1) + card_width(columns) + picker::CARD_WIDTH + 2.0 * CARD_GAP;
    if room < needed {
        return Some(picker);
    }
    Some(
        div()
            .flex()
            .items_start()
            .gap(px(CARD_GAP))
            .child(style_card)
            .child(picker)
            .into_any_element(),
    )
}

/// A column of the style card, one button wide, under its 12 px `text_dim` glyph (14 high)
/// whose tooltip names it.
fn column_of(id: &str, glyph: &'static str, name: &'static str, palette: &Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(2.))
        .w(px(BUTTON))
        .child(widgets::tooltip(
            div()
                .id(SharedString::from(format!("heading-{id}")))
                .h(px(14.))
                .flex()
                .items_center()
                .child(widgets::icon(glyph, 12.0, palette.text_dim)),
            name,
        ))
}

/// An icon toggle of a card with its tooltip; `on` gets the accent tint.
fn toggle(
    id: &'static str,
    glyph: &'static str,
    tip: &'static str,
    on: bool,
    palette: &Palette,
    click: impl Fn(&mut App) + 'static,
) -> Stateful<Div> {
    widgets::tooltip(
        icon_button(id, widgets::glyph(glyph), on, true, palette).on_click(move |_, _, cx| {
            click(cx);
            cx.refresh_windows();
        }),
        tip,
    )
}

/// A small 9 px number of a card (a size, a count), with its tooltip.
fn value_label(
    id: &'static str,
    value: String,
    tip: &'static str,
    palette: &Palette,
) -> Stateful<Div> {
    widgets::tooltip(
        div()
            .id(id)
            .text_size(px(9.))
            .line_height(px(14.))
            .text_color(palette.text)
            .child(value),
        tip,
    )
}

/// The stamp tool's preset picker/// The stamp tool's preset picker: one row per label, in the stamp's colour, and the date.
fn stamp_card(palette: &Palette, cx: &mut Context<DocTab>) -> AnyElement {
    let state = tool_state(cx);
    let (current, dated) = (state.stamp, state.stamp_date);
    let mut list = div().flex().flex_col().gap(px(2.));
    for label in StampLabel::ALL {
        let hover = palette.hover;
        list = list.child(
            div()
                .id(SharedString::from(format!("stamp-{label:?}")))
                .h(px(BUTTON))
                .px(px(10.))
                .flex()
                .items_center()
                .rounded(px(6.))
                .border_1()
                .border_color(hsla(label.color()))
                .text_color(hsla(label.color()))
                .cursor_pointer()
                .when(label == current, |row| row.bg(palette.selected))
                .when(label != current, |row| {
                    row.hover(move |style| style.bg(hover))
                })
                .child(label.text())
                .on_click(move |_, _, cx: &mut App| {
                    tool_state(cx).stamp = label;
                    cx.refresh_windows();
                }),
        );
    }
    card(palette)
        .w(px(card_width(4)))
        .flex()
        .flex_col()
        .gap(px(BAR_GAP))
        .text_size(px(fonts::BODY))
        .child(list)
        .child(toggle(
            "stamp-date",
            icons::CALENDAR_CHECK,
            "Add today's date",
            dated,
            palette,
            move |cx| tool_state(cx).stamp_date = !dated,
        ))
        .into_any_element()
}

/// Choices of the redact card's cover column besides a custom colour, with their tooltips.
const REDACT_COVERS: [(Option<Color>, &str); 3] = [
    (Some(SWATCHES[0].0), "Black"),
    (Some(super::rgb(0xFF, 0xFF, 0xFF)), "White"),
    (None, "No fill: remove the content, paint nothing"),
];

/// The redact tool's card: what covers the redacted areas (black, white, a custom colour or
/// nothing) and, once areas are marked, their count, apply them for good after a warning, or
/// clear them.
fn redact_card(
    tab: &DocTab,
    palette: &Palette,
    window: &mut Window,
    cx: &mut Context<DocTab>,
) -> AnyElement {
    let cover = tool_state(cx).redact_cover;
    let picker_open = tool_state(cx).picker.open == Some(Column::Stroke);
    let is_cover = |choice: Option<Color>| match (choice, cover) {
        (Some(choice), Some(cover)) => same_color(choice, cover),
        (choice, cover) => choice.is_none() && cover.is_none(),
    };
    let mut covers = column_of("redact-cover", icons::PAINT_BUCKET, "Cover", palette);
    for (index, (choice, name)) in REDACT_COVERS.into_iter().enumerate() {
        covers = covers.child(widgets::tooltip(
            swatch(("redact-cover", index), choice, is_cover(choice), palette).on_click(
                move |_: &ClickEvent, _, cx: &mut App| {
                    tool_state(cx).redact_cover = choice;
                    cx.refresh_windows();
                },
            ),
            name,
        ));
    }
    let custom = cover.filter(|_| !REDACT_COVERS.iter().any(|(choice, _)| is_cover(*choice)));
    let shown = cover.map_or(0, color_to_rgb);
    covers = covers.child(widgets::tooltip(
        custom_swatch(
            ("redact-cover", REDACT_COVERS.len()),
            custom,
            picker_open,
            palette,
        )
        .on_click(move |_: &ClickEvent, _, cx: &mut App| {
            picker::toggle(Column::Stroke, shown, cx);
            cx.refresh_windows();
        }),
        match custom {
            Some(color) => format!("Custom colour {}", picker::format_hex(color_to_rgb(color))),
            None => "Custom colour…".to_owned(),
        },
    ));
    let count = tab.canvas.redactions.len();
    let marks = (count > 0).then(|| {
        let apply = icon_button(
            "redact-apply",
            widgets::glyph(icons::CHECK),
            false,
            true,
            palette,
        )
        .text_color(theme::hex(theme::RED))
        .on_click(cx.listener(|_, _: &ClickEvent, window, cx| confirm_redactions(window, cx)));
        let clear = icon_button(
            "redact-clear",
            widgets::glyph(icons::X),
            false,
            true,
            palette,
        )
        .on_click(cx.listener(|tab, _: &ClickEvent, _, cx| {
            tab.canvas.redactions.clear();
            cx.notify();
        }));
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(2.))
            .w(px(BUTTON))
            .child(value_label(
                "redact-count",
                count.to_string(),
                "Areas marked",
                palette,
            ))
            .child(widgets::tooltip(apply, "Apply redactions…"))
            .child(widgets::tooltip(clear, "Clear marks"))
    });
    let columns = 1 + usize::from(marks.is_some());
    let redact_card = card(palette)
        .flex()
        .items_start()
        .gap(px(COLUMN_GAP))
        .child(covers)
        .children(marks);
    if !picker_open {
        return redact_card.into_any_element();
    }
    let picker = picker::card(Column::Stroke, palette, window, cx);
    // No room for both beside the bar: the picker stands in for the card.
    let room = f32::from(tab.view.bounds.size.width) - INSET - RIGHT_CLEAR;
    let needed = card_width(1) + card_width(columns) + picker::CARD_WIDTH + 2.0 * CARD_GAP;
    if room < needed {
        return picker;
    }
    div()
        .flex()
        .items_start()
        .gap(px(CARD_GAP))
        .child(redact_card)
        .child(picker)
        .into_any_element()
}

/// Smallest and largest page eraser brush, and its step, in points.
const BRUSH_RANGE: (f32, f32, f32) = (4.0, 64.0, 4.0);

/// The eraser's card: what it erases (two icon toggles), and the page brush size.
fn eraser_card(palette: &Palette, cx: &mut Context<DocTab>) -> AnyElement {
    let state = tool_state(cx);
    let (content, brush) = (state.erase_content, state.brush);
    let mode = |id, glyph, tip, on: bool| {
        toggle(id, glyph, tip, on == content, palette, move |cx| {
            tool_state(cx).erase_content = on;
        })
    };
    let modes = div()
        .flex()
        .flex_col()
        .gap(px(BAR_GAP))
        .child(mode(
            "erase-elements",
            icons::STACK,
            "Erase comments",
            false,
        ))
        .child(mode(
            "erase-content",
            icons::ERASER,
            "Erase page content",
            true,
        ));
    let step = |id: &'static str, glyph: &'static str, tip: &'static str, delta: f32| {
        let (low, high, _) = BRUSH_RANGE;
        widgets::tooltip(
            widgets::icon_widget(id, glyph, widgets::SMALL_ICON_BUTTON, 14.0, true, palette)
                .on_click(move |_, _, cx: &mut App| {
                    let state = tool_state(cx);
                    state.brush = (state.brush + delta).clamp(low, high);
                    cx.refresh_windows();
                }),
            tip,
        )
    };
    let size = div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(4.))
        .w(px(BUTTON))
        .child(step(
            "brush-larger",
            icons::PLUS,
            "Larger brush",
            BRUSH_RANGE.2,
        ))
        .child(value_label(
            "brush-size",
            format!("{brush:.0}"),
            "Brush size (points)",
            palette,
        ))
        .child(step(
            "brush-smaller",
            icons::MINUS,
            "Smaller brush",
            -BRUSH_RANGE.2,
        ));
    card(palette)
        .flex()
        .items_start()
        .gap(px(COLUMN_GAP))
        .child(modes)
        .when(content, |card| card.child(size))
        .into_any_element()
}

/// Ask before redacting: it cannot be taken back once the file is saved.
fn confirm_redactions(window: &mut Window, cx: &mut Context<DocTab>) {
    let answer = window.prompt(
        PromptLevel::Warning,
        "Apply redactions?",
        Some(
            "The text, images and comments under the marked areas are removed and the areas \
             are covered as chosen. This is permanent once the file is saved.",
        ),
        &["Apply redactions", "Cancel"],
        cx,
    );
    cx.spawn(async move |tab, cx| {
        if let Ok(0) = answer.await {
            let _ = tab.update(cx, |tab, cx| {
                canvas::apply_redactions(tab, cx);
                cx.notify();
            });
        }
    })
    .detach();
}

fn same_color(a: Color, b: Color) -> bool {
    let close = |x: f32, y: f32| (x - y).abs() < 1.0 / 255.0;
    close(a.r, b.r) && close(a.g, b.g) && close(a.b, b.b)
}

/// Colour dot side and the ring around it (2 px stroke, 2 px gap), whole pixels.
const DOT: f32 = 18.0;
const RING: f32 = 26.0;

/// Corner radius of a circle `side` px wide. Exactly half: a smaller one leaves flat sides
/// that look jagged, and gpui clamps a radius to the painted size itself.
pub(super) fn circle_radius(side: f32) -> f32 {
    side / 2.0
}

/// Light colours get a hairline so they still show on the card; others none, which would
/// only blur their edge.
fn needs_outline(color: Color) -> bool {
    0.2126 * color.r + 0.7152 * color.g + 0.0722 * color.b > 0.85
}

/// Round colour dot; the current one gets a `text` ring, hovered ones a `border` ring. The
/// ring is filled with the card colour so it paints as one quad (a border-only quad is split
/// into strips).
pub(super) fn swatch(
    id: impl Into<ElementId>,
    color: Option<Color>,
    current: bool,
    palette: &Palette,
) -> Stateful<Div> {
    let dot = div().size(px(DOT)).rounded(px(circle_radius(DOT)));
    let outlined = |dot: Div| dot.border_1().border_color(palette.border);
    let dot = match color {
        Some(color) if needs_outline(color) => outlined(dot).bg(hsla(color)),
        Some(color) => dot.bg(hsla(color)),
        // No colour: white with a red slash, as in design tools.
        None => outlined(dot).bg(gpui::white()).child(no_color_slash()),
    };
    ring(id, current, palette).child(dot)
}

/// The 26 px ring of a swatch: grey around the current one, `border` when hovered.
fn ring(id: impl Into<ElementId>, current: bool, palette: &Palette) -> Stateful<Div> {
    let hover = palette.border;
    div()
        .id(id)
        .flex_none()
        .size(px(RING))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(circle_radius(RING)))
        .bg(palette.panel)
        .border(px(2.))
        .border_color(if current {
            palette.text_dim
        } else {
            palette.panel
        })
        .cursor_pointer()
        .when(!current, |ring| {
            ring.hover(move |style| style.border_color(hover))
        })
}

/// Last swatch of a colour column: a `+` on the surface grey that opens the picker, or the
/// current custom colour (ringed as the current swatch). Ringed in `border` while the picker
/// is open on it.
fn custom_swatch(
    id: impl Into<ElementId>,
    custom: Option<Color>,
    open: bool,
    palette: &Palette,
) -> Stateful<Div> {
    match custom {
        Some(_) => swatch(id, custom, true, palette),
        None => ring(id, false, palette)
            .when(open, |ring| ring.border_color(palette.text_dim))
            .child(
                div()
                    .size(px(DOT))
                    .rounded(px(circle_radius(DOT)))
                    .border_1()
                    .border_color(palette.border)
                    .bg(palette.surface)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(widgets::icon(icons::PLUS, 12.0, palette.text)),
            ),
    }
}

/// Red diagonal from the dot's bottom-left to its top-right rim.
fn no_color_slash() -> impl IntoElement {
    gpui::canvas(
        |_, _, _| {},
        |bounds, _, window, _| {
            let side = f32::from(bounds.size.width);
            let (center, reach) = (
                side / 2.0,
                (side / 2.0 - 1.0) * std::f32::consts::FRAC_1_SQRT_2,
            );
            let at = |x: f32, y: f32| bounds.origin + gpui::point(px(x), px(y));
            let mut path = gpui::PathBuilder::stroke(px(1.5));
            path.move_to(at(center - reach, center + reach));
            path.line_to(at(center + reach, center - reach));
            if let Ok(path) = path.build() {
                window.paint_path(path, gpui::rgb(0xD9_2D_20));
            }
        },
    )
    .size_full()
}

/// Vertical `+ / value / -` stepper.
fn stepper<F, L>(column: Div, value: String, what: &str, palette: &Palette, on: &F) -> Div
where
    F: Fn(Change) -> L,
    L: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let small = |id: &str, icon: &'static str, tooltip: String, change: Change| {
        widgets::tooltip(
            widgets::icon_widget(
                SharedString::from(id.to_owned()),
                icon,
                widgets::SMALL_ICON_BUTTON,
                14.0,
                true,
                palette,
            )
            .on_click(on(change)),
            tooltip,
        )
    };
    column
        .child(small(
            &format!("larger-{what}"),
            icons::PLUS,
            format!("Larger {what}"),
            Change::Step(1.0),
        ))
        // Small text: 9 px.
        .child(
            div()
                .text_size(px(9.))
                .text_color(palette.text)
                .child(value),
        )
        .child(small(
            &format!("smaller-{what}"),
            icons::MINUS,
            format!("Smaller {what}"),
            Change::Step(-1.0),
        ))
}

/// Font menu: "Aa" in the current font on a framed button; the menu lists every font
/// in its own face.
fn font_menu(current: Font, palette: &Palette) -> AnyElement {
    let (family, name) = editor::font_label(current);
    let label = div().font_family(family).text_size(px(15.)).child("Aa");
    Button::new("font-menu")
        .w(px(BUTTON))
        .h(px(BUTTON))
        .px(px(0.))
        .text_color(palette.text)
        .child(label)
        .tooltip(format!("Font: {name}"))
        .dropdown_menu(move |mut menu, _, _| {
            for font in Font::ALL {
                let (family, name) = editor::font_label(font);
                menu = menu.item(
                    PopupMenuItem::element(move |_, _| {
                        div().font_family(family).text_size(px(15.)).child(name)
                    })
                    .checked(font == current)
                    .on_click(move |_, window, cx| choose_font(font, window, cx)),
                );
            }
            menu
        })
        .into_any_element()
}

/// A font picked in the font menu: applied to whatever the style card edits.
fn choose_font(font: Font, window: &mut Window, cx: &mut App) {
    apply_to_active(Change::Font(font), window, cx);
}

/// A colour of the picker: applied like a preset swatch of the colour (or fill) column.
pub(super) fn apply_custom(fill: bool, color: Color, window: &mut Window, cx: &mut App) {
    let change = if fill {
        Change::Fill(Some(color))
    } else {
        Change::Color(color)
    };
    apply_to_active(change, window, cx);
}

fn apply_to_active(change: Change, window: &mut Window, cx: &mut App) {
    let Some(tab) = tool_state(cx).active.clone().and_then(|tab| tab.upgrade()) else {
        return;
    };
    tab.update(cx, |tab, cx| apply(tab, change, window, cx));
}

/// The fields of the picture card, created the first time one is shown.
fn picture_fields(
    window: &mut Window,
    cx: &mut Context<DocTab>,
) -> Vec<(PictureField, Entity<InputState>)> {
    if let Some(card) = &tool_state(cx).picture_card {
        return card.fields.clone();
    }
    let mut fields = Vec::new();
    let mut subscriptions = Vec::new();
    for field in PictureField::ALL {
        let state = cx.new(|cx| InputState::new(window, cx));
        // App-wide: the card serves every tab.
        subscriptions.push(App::subscribe(
            cx,
            &state,
            move |state: Entity<InputState>, event: &InputEvent, cx: &mut App| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let text = state.read(cx).value().to_string();
                    edit_picture_field(field, &text, cx);
                }
            },
        ));
        fields.push((field, state));
    }
    tool_state(cx).picture_card = Some(PictureCard {
        fields: fields.clone(),
        shown: None,
        _subscriptions: subscriptions,
    });
    fields
}

/// A field of the picture card was confirmed (Enter, or left): move or resize the picture.
fn edit_picture_field(field: PictureField, text: &str, cx: &mut App) {
    let state = tool_state(cx);
    let keep = state.keep_ratio;
    let Some(card) = state.picture_card.as_mut() else {
        return;
    };
    // Show the picture as it is on the next frame (an invalid entry goes back to it).
    let Some((tab_id, page, id, _)) = card.shown.take() else {
        return;
    };
    with_active(cx, |tab, cx| {
        // The picture the fields were showing, even if a click on the page just changed the
        // selection before the field lost the focus.
        if tab_id != cx.entity_id() {
            return;
        }
        let Some(item) = canvas::items(tab, page, cx)
            .and_then(|items| items.into_iter().find(|item| item.id == id))
        else {
            return;
        };
        let Some(mm) = geom::parse_mm(text) else {
            toasts::error(
                cx,
                format!("“{}” is not a number of millimetres", text.trim()),
            );
            return;
        };
        let Some(&size) = tab.page_sizes.get(page) else {
            return;
        };
        let edited = geom::edit_picture(item.rect, field, mm, keep);
        // Stay on the page.
        let edited = edited.translate(geom::clamp_offset(edited, size, (0.0, 0.0)));
        let moved = [edited.x0 - item.rect.x0, edited.y0 - item.rect.y0];
        let grown = [
            edited.width() - item.rect.width(),
            edited.height() - item.rect.height(),
        ];
        if moved.iter().chain(&grown).all(|delta| delta.abs() < 0.01) {
            return;
        }
        set_picture_rect(tab, page, &item.id, edited, cx);
    });
}

fn set_picture_rect(tab: &mut DocTab, page: usize, id: &str, rect: DRect, cx: &mut App) {
    let change = move |annotation: &Annotation, _: &_, geom: &_, displayed| match annotation {
        Annotation::Image { picture, .. } => Annotation::Image {
            rect: rect.to_pdf(geom, displayed),
            picture: picture.clone(),
        },
        other => other.clone(),
    };
    rewrite(tab, page, id, "Resize", change, cx);
}

/// Width, height and position of the selected picture in millimetres: a label column and a
/// field column of fixed widths, so nothing overlaps nor gets cut.
#[allow(clippy::too_many_arguments)]
fn picture_card(
    tab: &mut DocTab,
    page: usize,
    id: String,
    rect: DRect,
    natural: (f32, f32),
    palette: &Palette,
    window: &mut Window,
    cx: &mut Context<DocTab>,
) -> Option<AnyElement> {
    let fields = picture_fields(window, cx);
    let key = (cx.entity_id(), page, id.clone(), tab.generation());
    let card_state = tool_state(cx).picture_card.as_mut()?;
    if card_state.shown.as_ref() != Some(&key) {
        card_state.shown = Some(key);
        for (field, state) in &fields {
            let text = format!("{:.1}", field.value_mm(rect));
            state.update(cx, |state, cx| state.set_value(text, window, cx));
        }
    }
    let keep = tool_state(cx).keep_ratio;
    // Grid spacing 8 × 6.
    let mut rows = div().flex().flex_col().gap(px(6.));
    for (field, state) in &fields {
        let (glyph, tip) = match field {
            PictureField::Width => (icons::ARROWS_HORIZONTAL, "Width (mm)"),
            PictureField::Height => (icons::ARROWS_VERTICAL, "Height (mm)"),
            PictureField::X => (icons::ARROW_LINE_RIGHT, "X from the page's left edge (mm)"),
            PictureField::Y => (icons::ARROW_LINE_DOWN, "Y from the page's top edge (mm)"),
        };
        rows = rows.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(widgets::tooltip(
                    div()
                        .id(SharedString::from(format!("picture-{}", field.label())))
                        .w(px(LABEL_WIDTH))
                        .flex_none()
                        .flex()
                        .justify_center()
                        .child(widgets::icon(glyph, 16.0, palette.text_dim)),
                    tip,
                ))
                .child(
                    div()
                        .w(px(FIELD_WIDTH))
                        .flex_none()
                        .child(Input::new(state).small()),
                ),
        );
    }
    // Its own size from its top-left corner, smaller when the page has no room, on the page.
    let page_size = tab.page_sizes.get(page).copied().unwrap_or_default();
    let original = geom::place_picture(natural, page_size, (rect.x0, rect.y0), None);
    let original = original.translate(geom::clamp_offset(original, page_size, (0.0, 0.0)));
    let original_size = icon_button(
        "picture-original-size",
        widgets::glyph(icons::ARROWS_OUT_SIMPLE),
        false,
        true,
        palette,
    )
    .on_click(cx.listener(move |tab, _: &ClickEvent, _, cx| {
        set_picture_rect(tab, page, &id, original, cx);
        cx.notify();
    }));
    let buttons = div()
        .flex()
        .gap(px(BAR_GAP))
        .child(toggle(
            "picture-keep-ratio",
            icons::LINK_SIMPLE,
            "Keep proportions",
            keep,
            palette,
            move |cx| tool_state(cx).keep_ratio = !keep,
        ))
        .child(widgets::tooltip(
            original_size,
            "Original size (96 pixels to the inch)",
        ));
    Some(
        card(palette)
            .flex()
            .flex_col()
            .gap(px(6.))
            .text_size(px(fonts::BODY))
            .child(rows)
            .child(buttons)
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::{DOT, RING, circle_radius, needs_outline};

    #[test]
    fn circles_are_round_and_only_light_colours_are_outlined() {
        use godpdf_core::Color;
        assert_eq!(circle_radius(DOT), DOT / 2.0);
        assert_eq!(circle_radius(RING), RING / 2.0);
        let rgb = |r, g, b| Color { r, g, b };
        assert!(needs_outline(rgb(1.0, 1.0, 1.0)));
        assert!(needs_outline(rgb(1.0, 1.0, 0.0)));
        assert!(!needs_outline(rgb(0.9, 0.1, 0.1)));
        assert!(!needs_outline(rgb(0.0, 0.0, 0.0)));
    }

    #[test]
    fn fill_swatch_restyles_rectangles_and_ellipses_only() {
        use super::{Change, Kind, Style, Tool, change_style, restyled, tool_kind};
        use godpdf_core::{Annotation, Color, Font, Rect};
        let blue = Color {
            r: 0.0,
            g: 0.0,
            b: 1.0,
        };
        let mut style = Style {
            color: blue,
            width: 2.0,
            font_size: 14.0,
            font: Font::Helvetica,
            fill: None,
        };
        change_style(&mut style, Kind::Shape, Change::Fill(Some(blue)));
        assert_eq!(style.fill, Some(blue));
        let rect = Rect {
            x0: 0.0,
            y0: 0.0,
            x1: 10.0,
            y1: 10.0,
        };
        let square = Annotation::Square {
            rect,
            color: blue,
            width: 1.0,
            fill: None,
        };
        let Annotation::Square { fill, width, .. } =
            restyled(&square, style, Change::Fill(Some(blue)))
        else {
            panic!("not a rectangle");
        };
        assert_eq!((fill, width), (Some(blue), 2.0));
        assert!(tool_kind(Tool::Ellipse) == Some(Kind::Shape));
        assert!(tool_kind(Tool::Line) == Some(Kind::Stroke));
    }
}
