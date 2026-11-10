//! The quick bar floating over the viewer's top-left corner: tool toggles, undo / redo, and
//! next to it the style card of what is being made or is selected (colours, width, font,
//! formats), or the property card of the selected picture.

use godpdf_core::{Annotation, CharStyle, Color, Font};
use gpui::assets::IconName;
use gpui::component::Sizable;
use gpui::component::checkbox::Checkbox;
use gpui::component::input::{Input, InputEvent, InputState};
use gpui::component::tooltip::Tooltip;
use gpui::prelude::*;
use gpui::{
    Action, AnyElement, App, ClickEvent, Context, Div, ElementId, Entity, EntityId, FontWeight,
    SharedString, Stateful, Subscription, Window, div, px,
};

use super::canvas::{self, rewrite};
use super::geom::{self, DRect, PictureField};
use super::{
    Format, SWATCHES, Style, Tool, common_style, current_tool, editor, hsla, toggle_all,
    tool_state, with_active,
};
use crate::doc::DocTab;
use crate::theme::Palette;
use crate::toasts;

/// Distance of the bar from the viewer's top-left corner.
const INSET: f32 = 16.0;
const BUTTON: f32 = 32.0;
const PICTURE_CARD_WIDTH: f32 = 204.0;
const LABEL_WIDTH: f32 = 52.0;
const FIELD_WIDTH: f32 = 116.0;
const HINT_CARD_WIDTH: f32 = 190.0;

/// The fields of the picture card (created once, reused for every picture).
pub struct PictureCard {
    fields: Vec<(PictureField, Entity<InputState>)>,
    /// What the fields show: tab, annotation id, document generation.
    shown: Option<(EntityId, String, u64)>,
    _subscriptions: Vec<Subscription>,
}

/// Which style controls a target has.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Colour + font size + font.
    Text,
    /// Colour + stroke width.
    Stroke,
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
    /// One step up (+1) or down (-1) of the font size or line width.
    Step(f32),
    NextFont,
    Format(Format),
}

fn target(tab: &mut DocTab, cx: &mut App) -> Target {
    if let Some(editor) = &tab.canvas.editor {
        return match editor.kind {
            editor::Kind::Text { .. } => Target::TextBox(editor.format),
            editor::Kind::Note { .. } => Target::Tool(Tool::Note),
        };
    }
    let style = |color: Color, width: f32, font_size: f32, font: Font| Style {
        color,
        width,
        font_size,
        font,
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
                Some(common_style(&styles, text.chars().count())),
            ),
            Some(
                Annotation::Square { color, width, .. }
                | Annotation::Circle { color, width, .. }
                | Annotation::Line { color, width, .. }
                | Annotation::Ink { color, width, .. },
            ) => Target::Selected(
                Kind::Stroke,
                style(color, width, 14.0, Font::Helvetica),
                None,
            ),
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
        Change::Step(sign) if kind == Kind::Text => {
            style.font_size = (style.font_size + sign).clamp(6.0, 72.0);
        }
        Change::Step(sign) => style.width = (style.width + sign * 0.5).clamp(0.5, 12.0),
        Change::NextFont => {
            let next = Font::ALL
                .iter()
                .position(|font| *font == style.font)
                .map_or(0, |index| (index + 1) % Font::ALL.len());
            style.font = Font::ALL[next];
        }
        Change::Format(_) => {}
    }
}

/// Apply a click of the style card to whatever it edits.
fn apply(tab: &mut DocTab, change: Change, window: &mut Window, cx: &mut Context<DocTab>) {
    match target(tab, cx) {
        Target::TextBox(_) => match change {
            Change::Format(format) => editor::toggle_format(tab, format, window, cx),
            _ => {
                change_style(&mut tool_state(cx).styles.text, Kind::Text, change);
                editor::refocus(tab, window, cx);
            }
        },
        Target::Selected(kind, mut style, _) => {
            let Some(selected) = tab.canvas.selected.clone() else {
                return;
            };
            change_style(&mut style, kind, change);
            let restyle =
                move |annotation: &Annotation, _: &_, _: &_, _| restyled(annotation, style, change);
            rewrite(tab, selected.page, &selected.id, "Restyle", restyle, cx);
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
        Annotation::Highlight { rect, .. } => Annotation::Highlight { rect, color },
        Annotation::Square { rect, .. } => Annotation::Square {
            rect,
            color,
            width: style.width,
        },
        Annotation::Circle { rect, .. } => Annotation::Circle {
            rect,
            color,
            width: style.width,
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
        Annotation::Underline { rect, .. } => Annotation::Underline { rect, color },
        Annotation::StrikeOut { rect, .. } => Annotation::StrikeOut { rect, color },
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
                toggle_all(&mut styles, text.chars().count(), format);
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
        Tool::Ink | Tool::Rectangle => Some(Kind::Stroke),
        Tool::Highlight => Some(Kind::Fill),
        _ => None,
    }
}

fn card(palette: &Palette) -> Div {
    div()
        .flex_none()
        .p(px(4.))
        .rounded(px(10.))
        .shadow_md()
        .bg(palette.panel)
        .border_1()
        .border_color(palette.border)
        .text_color(palette.text)
        // Clicks stay on the card; the wheel still scrolls the pages under it.
        .block_mouse_except_scroll()
}

fn divider(palette: &Palette) -> Div {
    div().h(px(1.)).mx(px(4.)).my(px(3.)).bg(palette.border)
}

fn with_tooltip(element: Stateful<Div>, text: impl Into<SharedString>) -> Stateful<Div> {
    let text: SharedString = text.into();
    element.tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx))
}

/// Square icon button; `active` sits on the selected grey, disabled ones are dimmed.
fn icon_button(
    id: impl Into<ElementId>,
    content: impl IntoElement,
    active: bool,
    enabled: bool,
    palette: &Palette,
) -> Stateful<Div> {
    let hover = palette.hover;
    div()
        .id(id)
        .flex_none()
        .size(px(BUTTON))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .text_size(px(16.))
        .text_color(if enabled {
            palette.text
        } else {
            palette.text_dim
        })
        .when(active, |button| button.bg(palette.selected))
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
    let view_height = f32::from(tab.view.bounds.size.height);

    let mut tools = card(&palette)
        .id("quick-tools")
        .flex()
        .flex_col()
        .gap(px(2.))
        .max_h(px((view_height - 2.0 * INSET).max(BUTTON * 3.0)))
        .overflow_y_scroll();
    for candidate in Tool::ALL {
        tools = tools.child(with_tooltip(
            icon_button(
                SharedString::from(format!("tool-{:?}", candidate)),
                candidate.icon(),
                candidate == tool,
                true,
                &palette,
            )
            .on_click(dispatch(candidate.action())),
            candidate.tooltip(),
        ));
    }
    let mut undo = icon_button("quick-undo", IconName::Undo2, false, can_undo, &palette);
    if can_undo {
        undo = undo.on_click(dispatch(Box::new(crate::app::Undo)));
    }
    let mut redo = icon_button("quick-redo", IconName::Redo2, false, can_redo, &palette);
    if can_redo {
        redo = redo.on_click(dispatch(Box::new(crate::app::Redo)));
    }
    tools = tools
        .child(divider(&palette))
        .child(with_tooltip(undo, "Undo (Ctrl+Z)"))
        .child(with_tooltip(redo, "Redo (Ctrl+Y)"));

    let style_card = style_card(tab, &palette, window, cx);
    Some(
        div()
            .absolute()
            .top(px(INSET))
            .left(px(INSET))
            .flex()
            .items_start()
            .gap(px(6.))
            .child(tools)
            .children(style_card)
            .into_any_element(),
    )
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
            None if matches!(tool, Tool::Note | Tool::Picture) => {
                return Some(hint_card(tool.hint(), palette));
            }
            None => return None,
        },
        Target::Selected(kind, style, formats) => (kind, style, formats),
        Target::TextBox(format) => (Kind::Text, tool_state(cx).styles.text, Some(format)),
        Target::Picture {
            page,
            id,
            rect,
            natural,
        } => return picture_card(tab, page, id, rect, natural, palette, window, cx),
    };
    let on = |change: Change| {
        cx.listener(move |tab, _: &ClickEvent, window, cx| apply(tab, change, window, cx))
    };

    let mut colors = div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(4.))
        .w(px(BUTTON));
    for (index, (color, name)) in SWATCHES.into_iter().enumerate() {
        colors = colors.child(with_tooltip(
            swatch(
                ("swatch", index),
                color,
                same_color(color, style.color),
                palette,
            )
            .on_click(on(Change::Color(color))),
            name,
        ));
    }
    let mut controls = div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(4.))
        .w(px(BUTTON));
    match kind {
        Kind::Text => {
            controls = stepper(
                controls,
                format!("{:.0}", style.font_size),
                "font size",
                palette,
                &on,
            );
            let (family, name) = editor::font_label(style.font);
            controls = controls
                .child(divider(palette).w(px(BUTTON - 8.)))
                .child(with_tooltip(
                    icon_button(
                        "font",
                        div().font_family(family).child("Aa"),
                        false,
                        true,
                        palette,
                    )
                    .on_click(on(Change::NextFont)),
                    format!("Font: {name} (click for the next one)"),
                ));
            if let Some(active) = formats {
                controls = controls.child(divider(palette).w(px(BUTTON - 8.)));
                for format in super::Format::ALL {
                    let (icon, tooltip) = match format {
                        Format::Bold => (IconName::Bold, "Bold (Ctrl+B)"),
                        Format::Italic => (IconName::Italic, "Italic (Ctrl+I)"),
                        Format::Underline => (IconName::Underline, "Underline (Ctrl+U)"),
                    };
                    controls = controls.child(with_tooltip(
                        icon_button(
                            SharedString::from(format!("format-{format:?}")),
                            icon,
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
        Kind::Stroke => {
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
    Some(
        card(palette)
            .flex()
            .items_start()
            .gap(px(4.))
            .child(colors)
            .when(kind != Kind::Fill, |card| card.child(controls))
            .into_any_element(),
    )
}

fn same_color(a: Color, b: Color) -> bool {
    let close = |x: f32, y: f32| (x - y).abs() < 1.0 / 255.0;
    close(a.r, b.r) && close(a.g, b.g) && close(a.b, b.b)
}

/// Round colour dot; the current one gets a ring.
fn swatch(
    id: impl Into<ElementId>,
    color: Color,
    current: bool,
    palette: &Palette,
) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(22.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .cursor_pointer()
        .when(current, |ring| {
            ring.border(px(1.5)).border_color(palette.text)
        })
        .child(
            div()
                .size(px(14.))
                .rounded_full()
                .bg(hsla(color))
                .border_1()
                .border_color(palette.border),
        )
}

/// Vertical `+ / value / -` stepper.
fn stepper<F, L>(column: Div, value: String, what: &str, palette: &Palette, on: &F) -> Div
where
    F: Fn(Change) -> L,
    L: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let small = |id: &str, icon: IconName, tooltip: String, change: Change| {
        with_tooltip(
            icon_button(
                SharedString::from(id.to_owned()),
                icon,
                false,
                true,
                palette,
            )
            .h(px(22.))
            .text_size(px(13.))
            .on_click(on(change)),
            tooltip,
        )
    };
    column
        .child(small(
            &format!("larger-{what}"),
            IconName::Plus,
            format!("Larger {what}"),
            Change::Step(1.0),
        ))
        .child(div().text_xs().text_color(palette.text).child(value))
        .child(small(
            &format!("smaller-{what}"),
            IconName::Minus,
            format!("Smaller {what}"),
            Change::Step(-1.0),
        ))
}

fn hint_card(text: &'static str, palette: &Palette) -> AnyElement {
    card(palette)
        .w(px(HINT_CARD_WIDTH))
        .p(px(10.))
        .flex()
        .items_start()
        .gap(px(8.))
        .child(
            div()
                .flex_none()
                .text_size(px(14.))
                .text_color(palette.text_dim)
                .child(IconName::Info),
        )
        .child(div().text_sm().text_color(palette.text_dim).child(text))
        .into_any_element()
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
    let shown = card.shown.clone();
    // Show the picture as it is on the next frame (an invalid entry goes back to it).
    card.shown = None;
    with_active(cx, |tab, cx| {
        let Some((page, item)) = canvas::selected_item(tab, cx) else {
            return;
        };
        // The fields were showing another picture (the selection just changed).
        if shown.is_none_or(|(tab_id, id, _)| tab_id != cx.entity_id() || id != item.id) {
            return;
        }
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
    let key = (cx.entity_id(), id.clone(), tab.generation());
    let card_state = tool_state(cx).picture_card.as_mut()?;
    if card_state.shown.as_ref() != Some(&key) {
        card_state.shown = Some(key);
        for (field, state) in &fields {
            let text = format!("{:.1}", field.value_mm(rect));
            state.update(cx, |state, cx| state.set_value(text, window, cx));
        }
    }
    let keep = tool_state(cx).keep_ratio;
    let mut rows = div().flex().flex_col().gap(px(6.));
    for (field, state) in &fields {
        rows = rows.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .w(px(LABEL_WIDTH))
                        .flex_none()
                        .text_sm()
                        .text_color(palette.text_dim)
                        .child(field.label()),
                )
                .child(
                    div().w(px(FIELD_WIDTH)).flex_none().child(
                        Input::new(state)
                            .small()
                            .suffix(div().text_xs().text_color(palette.text_dim).child("mm")),
                    ),
                ),
        );
    }
    let original = DRect {
        x0: rect.x0,
        y0: rect.y0,
        x1: rect.x0 + natural.0,
        y1: rect.y0 + natural.1,
    };
    let hover = palette.hover;
    let original_size = div()
        .id("picture-original-size")
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .h(px(28.))
        .rounded(px(7.))
        .border_1()
        .border_color(palette.border)
        .text_sm()
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(div().text_size(px(14.)).child(IconName::Maximize2))
        .child("Original size")
        .on_click(cx.listener(move |tab, _: &ClickEvent, _, cx| {
            set_picture_rect(tab, page, &id, original, cx);
            cx.notify();
        }));
    Some(
        card(palette)
            .w(px(PICTURE_CARD_WIDTH))
            .p(px(10.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .child("Picture"),
            )
            .child(rows)
            .child(
                div()
                    .text_xs()
                    .text_color(palette.text_dim)
                    .child("X and Y from the page's top-left corner"),
            )
            .child(
                Checkbox::new("picture-keep-ratio")
                    .label("Keep proportions")
                    .checked(keep)
                    .on_click(|checked: &bool, _, cx: &mut App| {
                        tool_state(cx).keep_ratio = *checked;
                        cx.refresh_windows();
                    }),
            )
            .child(with_tooltip(original_size, "96 pixels to the inch"))
            .into_any_element(),
    )
}
