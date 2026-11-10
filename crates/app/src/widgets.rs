//! Small shared controls in the palette of `theme.rs`.
//! Icons are Phosphor glyphs from `icons.rs`, drawn with [`icon`].

use gpui::component::tooltip::Tooltip;
use gpui::prelude::*;
use gpui::{Action, App, BoxShadow, Div, ElementId, Hsla, SharedString, Stateful, div, px};

use crate::fonts;
use crate::theme::{self, Palette};

/// Side of a standard icon-only button.
pub const ICON_BUTTON: f32 = 28.0;
/// Side of a compact icon-only button: tab close, thumbnail arrows (`SMALL_ICON_BUTTON`).
pub const SMALL_ICON_BUTTON: f32 = 22.0;
/// Side of a button of the right rail (`RAIL_BUTTON`).
pub const RAIL_BUTTON: f32 = 32.0;

/// A Phosphor glyph in the size and colour of the text around it.
pub fn glyph(glyph: &'static str) -> Div {
    div().flex_none().font_family(fonts::ICONS).child(glyph)
}

/// One Phosphor glyph (`icons::X`) at `size` px in `color`: a line box of
/// the glyph's size, so it centers like text.
pub fn icon(glyph: &'static str, size: f32, color: Hsla) -> Div {
    self::glyph(glyph)
        .text_size(px(size))
        .line_height(px(size))
        .text_color(color)
}

/// Glyph then label with a gap (two spaces of the label's font, ~`size`/2).
pub fn with_icon(
    glyph: &'static str,
    label: impl Into<SharedString>,
    size: f32,
    color: Hsla,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(size * 0.5))
        .text_size(px(size))
        .text_color(color)
        .child(icon(glyph, size, color))
        .child(label.into())
}

/// Attach a hover tooltip: a popup frame (panel fill,
/// border, radius 9, margin 8, popup shadow) around body text.
pub fn tooltip(element: Stateful<Div>, text: impl Into<SharedString>) -> Stateful<Div> {
    let text = text.into();
    element.tooltip(move |window, cx| {
        let palette = Palette::of(cx);
        Tooltip::new(text.clone())
            .bg(palette.panel)
            .border_color(palette.border)
            .rounded(px(theme::MENU_RADIUS))
            .p(px(theme::MENU_MARGIN))
            .text_size(px(fonts::BODY))
            .text_color(palette.text)
            .shadow(popup_shadow(&palette))
            .build(window, cx)
    })
}

/// A shadow offset `[0, y]`, `blur` wide, of black at `alpha` (0..1). GPUI's blur
/// radius is a gaussian deviation, about half the blur width.
pub fn shadow(y: f32, blur: f32, alpha: f32) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: gpui::hsla(0., 0., 0., alpha),
        offset: gpui::point(px(0.), px(y)),
        blur_radius: px(blur / 2.0),
        spread_radius: px(0.),
        inset: false,
    }]
}

/// Popup shadow: menus, tooltips, toasts, the command palette.
pub fn popup_shadow(palette: &Palette) -> Vec<BoxShadow> {
    shadow(6.0, 16.0, palette.shadow_alpha * 0.75)
}

/// Icon widget: square, frameless until hovered (hover fill, radius 7), glyph in the
/// text colour. Add `.on_click(..)`. `enabled = false` dims it and drops the hover.
pub fn icon_widget(
    id: impl Into<ElementId>,
    glyph: &'static str,
    side: f32,
    glyph_size: f32,
    enabled: bool,
    palette: &Palette,
) -> Stateful<Div> {
    let hover = palette.hover;
    let color = if enabled {
        palette.text
    } else {
        palette.disabled()
    };
    div()
        .id(id)
        .flex_none()
        .size(px(side))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme::RADIUS))
        .when(enabled, |button| {
            button.cursor_pointer().hover(move |style| style.bg(hover))
        })
        .child(icon(glyph, glyph_size, color))
}

/// Standard 28×28 icon button (16 px glyph) with a tooltip.
pub fn icon_button(
    id: impl Into<ElementId>,
    glyph: &'static str,
    tip: impl Into<SharedString>,
    palette: &Palette,
) -> Stateful<Div> {
    tooltip(
        icon_widget(id, glyph, ICON_BUTTON, 16.0, true, palette),
        tip,
    )
}

/// Compact 22×22 icon button (13 px glyph) with a tooltip.
pub fn small_icon_button(
    id: impl Into<ElementId>,
    glyph: &'static str,
    tip: impl Into<SharedString>,
    palette: &Palette,
) -> Stateful<Div> {
    tooltip(
        icon_widget(id, glyph, SMALL_ICON_BUTTON, 13.0, true, palette),
        tip,
    )
}

/// 32×32 toggle of the right rail (18 px glyph); `selected` shows the `selected` square,
/// hover the `hover` one.
pub fn rail_button(
    id: impl Into<ElementId>,
    glyph: &'static str,
    tip: impl Into<SharedString>,
    selected: bool,
    enabled: bool,
    palette: &Palette,
) -> Stateful<Div> {
    let button = icon_widget(id, glyph, RAIL_BUTTON, 18.0, enabled, palette)
        .when(selected, |button| button.bg(palette.selected));
    tooltip(button, tip)
}

/// Full-width list row: 20 px glyph in `tint` centered 20 px from the left,
/// label (14) at 42 px, optional dimmed description (13) under it; hover fill radius 8.
/// Height 40, or 52 with a description.
pub fn tool_row(
    id: impl Into<ElementId>,
    glyph: &'static str,
    tint: Hsla,
    label: impl Into<SharedString>,
    description: Option<SharedString>,
    palette: &Palette,
) -> Stateful<Div> {
    let hover = palette.hover;
    let height = if description.is_some() { 52.0 } else { 40.0 };
    div()
        .id(id)
        .flex_none()
        .w_full()
        .h(px(height))
        .flex()
        .items_center()
        .pl(px(10.))
        .rounded(px(8.))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(div().w(px(32.)).flex_none().child(icon(glyph, 20.0, tint)))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    div()
                        .text_size(px(14.))
                        .text_color(palette.text)
                        .child(label.into()),
                )
                .children(description.map(|description| {
                    div()
                        .text_size(px(13.))
                        .text_color(palette.text_dim)
                        .child(description)
                })),
        )
}

/// A frameless text button: padding 10×5,
/// height 26, hover fill, radius 7. Add `.on_click(..)`, e.g. [`dispatch`].
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    palette: &Palette,
) -> Stateful<Div> {
    let hover = palette.hover;
    div()
        .id(id)
        .flex()
        .items_center()
        .px(px(theme::BUTTON_PADDING.0))
        .h(px(theme::INTERACT_HEIGHT))
        .rounded(px(theme::RADIUS))
        .text_size(px(fonts::BODY))
        .text_color(palette.text)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(label.into())
}

/// A framed button: surface fill, 1 px border, radius 7, padding 10×5,
/// height 26; hover fill. `enabled = false` dims the label and drops the hover.
pub fn framed_button(
    id: impl Into<ElementId>,
    label: impl IntoElement,
    enabled: bool,
    palette: &Palette,
) -> Stateful<Div> {
    let hover = palette.hover;
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(fonts::BODY * 0.5))
        .px(px(theme::BUTTON_PADDING.0))
        .h(px(theme::INTERACT_HEIGHT))
        .rounded(px(theme::RADIUS))
        .bg(palette.surface)
        .border_1()
        .border_color(palette.border)
        .text_size(px(fonts::BODY))
        .text_color(if enabled {
            palette.text
        } else {
            palette.disabled()
        })
        .when(enabled, |button| {
            button.cursor_pointer().hover(move |style| style.bg(hover))
        })
        .child(label)
}

/// The filled button of a dialog's main action: a
/// [`framed_button`] in the accent colour.
pub fn accent_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    enabled: bool,
    palette: &Palette,
) -> Stateful<Div> {
    let accent = palette.accent;
    framed_button(id, label.into(), enabled, palette)
        .bg(accent)
        .text_color(palette.on_accent)
        .when(!enabled, |button| button.opacity(0.5))
        .hover(move |style| style.bg(accent))
}

/// Click handler that dispatches `action`, so a button runs the same handler as its shortcut.
pub fn dispatch(
    action: impl Action + Clone,
) -> impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut App) + 'static {
    move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx)
}

/// The filled button of a screen's main action.
#[allow(dead_code)] // dialogs moved to `accent_button`; kept for the welcome screen.
pub fn primary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    palette: &Palette,
) -> Stateful<Div> {
    let (accent, on_accent) = (palette.accent, palette.on_accent);
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .px(px(18.))
        .h(px(38.))
        .rounded(px(9.))
        .bg(accent)
        .text_color(on_accent)
        .cursor_pointer()
        .hover(|style| style.opacity(0.85))
        .child(label.into())
}
