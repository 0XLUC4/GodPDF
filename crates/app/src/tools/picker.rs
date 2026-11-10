//! Custom colour picker of the style card: a saturation / value square, a hue slider, a hex
//! field, a preview chip and the last custom colours (kept in the settings).
//!
//! Dragging only moves the picker; the colour is applied on release (or Enter in the field,
//! or a click on a recent colour), so each choice is one undo step.

use godpdf_core::Color;
use gpui::component::Sizable;
use gpui::component::input::{Input, InputEvent, InputState};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Bounds, ClickEvent, Context, CursorStyle, DispatchPhase, Entity,
    HitboxBehavior, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point,
    SharedString, Subscription, Window, div, linear_color_stop, linear_gradient, px,
};

use super::quickbar::{self, circle_radius};
use super::{hsla, tool_state};
use crate::doc::DocTab;
use crate::theme::Palette;
use crate::{fonts, icons, widgets};

/// Custom colours remembered.
pub const RECENT_MAX: usize = 6;
/// Saturation / value square and hue slider, whole pixels (6 hue segments of 32).
const AREA_WIDTH: f32 = 192.0;
const SQUARE_HEIGHT: f32 = 128.0;
const HUE_HEIGHT: f32 = 12.0;
const KNOB: f32 = 14.0;
const AREA_RADIUS: f32 = 6.0;

/// Which colour column of the style card the picker edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    /// The colour (stroke, text, highlight) column.
    Stroke,
    /// The fill of rectangles and ellipses.
    Fill,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Area {
    Square,
    Hue,
}

/// Picker state (in [`super::ToolState`]): open column, colour being picked.
#[derive(Default)]
pub struct Picker {
    pub open: Option<Column>,
    /// Hue 0..360, saturation and value 0..1. Kept as is while dragging, so a grey keeps its hue.
    hsv: [f32; 3],
    drag: Option<Area>,
    /// The hex field, created the first time the picker is shown.
    field: Option<(Entity<InputState>, Subscription)>,
    /// The field must show the picker's colour on the next frame.
    sync: bool,
}

/// Open the picker on `column` (showing `current`), or close it when it is open there.
pub fn toggle(column: Column, current: u32, cx: &mut App) {
    let picker = &mut tool_state(cx).picker;
    if picker.open == Some(column) {
        picker.open = None;
    } else {
        picker.open = Some(column);
        picker.hsv = rgb_to_hsv(current);
        picker.sync = true;
    }
}

/// `#RRGGBB`, `RRGGBB`, `#RGB` or `RGB` (spaces around ignored) as `0xRRGGBB`.
pub fn parse_hex(text: &str) -> Option<u32> {
    let digits = text.trim();
    let digits = digits.strip_prefix('#').unwrap_or(digits);
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let digits = match digits.len() {
        3 => digits.chars().flat_map(|c| [c, c]).collect(),
        6 => digits.to_owned(),
        _ => return None,
    };
    u32::from_str_radix(&digits, 16).ok()
}

pub fn format_hex(rgb: u32) -> String {
    format!("#{:06X}", rgb & 0xFF_FF_FF)
}

pub fn color_to_rgb(color: Color) -> u32 {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
    channel(color.r) << 16 | channel(color.g) << 8 | channel(color.b)
}

pub fn rgb_to_color(rgb: u32) -> Color {
    let channel = |shift: u32| ((rgb >> shift) & 0xFF) as f32 / 255.0;
    Color {
        r: channel(16),
        g: channel(8),
        b: channel(0),
    }
}

pub fn hsv_to_rgb([hue, saturation, value]: [f32; 3]) -> u32 {
    let chroma = value * saturation;
    let sector = hue.rem_euclid(360.0) / 60.0;
    let second = chroma * (1.0 - (sector % 2.0 - 1.0).abs());
    let (r, g, b) = match sector as u32 {
        0 => (chroma, second, 0.0),
        1 => (second, chroma, 0.0),
        2 => (0.0, chroma, second),
        3 => (0.0, second, chroma),
        4 => (second, 0.0, chroma),
        _ => (chroma, 0.0, second),
    };
    let lift = value - chroma;
    color_to_rgb(Color {
        r: r + lift,
        g: g + lift,
        b: b + lift,
    })
}

pub fn rgb_to_hsv(rgb: u32) -> [f32; 3] {
    let Color { r, g, b } = rgb_to_color(rgb);
    let max = r.max(g).max(b);
    let chroma = max - r.min(g).min(b);
    let hue = if chroma == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / chroma).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / chroma + 2.0)
    } else {
        60.0 * ((r - g) / chroma + 4.0)
    };
    let saturation = if max == 0.0 { 0.0 } else { chroma / max };
    [hue, saturation, max]
}

/// `rgb` first in `recent`, once, at most [`RECENT_MAX`] kept.
pub fn push_recent(recent: &mut Vec<u32>, rgb: u32) {
    recent.retain(|&other| other != rgb);
    recent.insert(0, rgb);
    recent.truncate(RECENT_MAX);
}

/// Apply `rgb` to the open column and remember it.
fn commit(rgb: u32, window: &mut Window, cx: &mut App) {
    let state = tool_state(cx);
    push_recent(&mut state.recent_colors, rgb);
    let fill = state.picker.open == Some(Column::Fill);
    quickbar::apply_custom(fill, rgb_to_color(rgb), window, cx);
}

fn hex_field(window: &mut Window, cx: &mut Context<DocTab>) -> Entity<InputState> {
    if let Some((field, _)) = &tool_state(cx).picker.field {
        return field.clone();
    }
    let field = cx.new(|cx| InputState::new(window, cx).placeholder("#RRGGBB"));
    let handle = window.window_handle();
    // App-wide: the picker serves every tab.
    let subscription = App::subscribe(
        cx,
        &field,
        move |field: Entity<InputState>, event: &InputEvent, cx: &mut App| {
            if !matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                return;
            }
            let text = field.read(cx).value().to_string();
            let picker = &mut tool_state(cx).picker;
            let Some(rgb) = parse_hex(&text) else {
                // Not a colour: show the picker's colour again.
                picker.sync = true;
                cx.refresh_windows();
                return;
            };
            if picker.open.is_none() || rgb == hsv_to_rgb(picker.hsv) {
                return;
            }
            picker.hsv = rgb_to_hsv(rgb);
            picker.sync = true;
            let _ = handle.update(cx, |_, window, cx| commit(rgb, window, cx));
        },
    );
    tool_state(cx).picker.field = Some((field.clone(), subscription));
    field
}

/// Picker position from the pointer over `area` at `bounds`.
fn pick(area: Area, bounds: Bounds<Pixels>, position: Point<Pixels>, cx: &mut App) {
    let fraction =
        |offset: Pixels, length: Pixels| (f32::from(offset) / f32::from(length)).clamp(0.0, 1.0);
    let x = fraction(position.x - bounds.origin.x, bounds.size.width);
    let picker = &mut tool_state(cx).picker;
    match area {
        Area::Square => {
            picker.hsv[1] = x;
            picker.hsv[2] = 1.0 - fraction(position.y - bounds.origin.y, bounds.size.height);
        }
        Area::Hue => picker.hsv[0] = x * 360.0,
    }
    picker.sync = true;
}

/// Invisible layer over `area`: press, drag (also outside it) and release pick.
fn drag_layer(area: Area) -> impl IntoElement {
    gpui::canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        move |bounds, hitbox, window, _| {
            window.set_cursor_style(CursorStyle::Crosshair, &hitbox);
            window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble
                    && event.button == MouseButton::Left
                    && hitbox.is_hovered(window)
                {
                    tool_state(cx).picker.drag = Some(area);
                    pick(area, bounds, event.position, cx);
                    window.refresh();
                }
            });
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble && tool_state(cx).picker.drag == Some(area) {
                    pick(area, bounds, event.position, cx);
                    window.refresh();
                }
            });
            window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble && tool_state(cx).picker.drag == Some(area) {
                    pick(area, bounds, event.position, cx);
                    let picker = &mut tool_state(cx).picker;
                    picker.drag = None;
                    let rgb = hsv_to_rgb(picker.hsv);
                    commit(rgb, window, cx);
                    window.refresh();
                }
            });
        },
    )
    .absolute()
    .inset_0()
}

/// Round handle centred on (`x`, `y`) of its area, whole pixels, filled with `color`.
fn knob(x: f32, y: f32, color: u32) -> impl IntoElement {
    div()
        .absolute()
        .left(px((x - KNOB / 2.0).round()))
        .top(px((y - KNOB / 2.0).round()))
        .size(px(KNOB))
        .rounded(px(circle_radius(KNOB)))
        .border(px(2.))
        .border_color(gpui::white())
        .bg(hsla(rgb_to_color(color)))
        .shadow(widgets::shadow(1.0, 4.0, 0.45))
}

fn pure_hue(hue: f32) -> gpui::Hsla {
    hsla(rgb_to_color(hsv_to_rgb([hue, 1.0, 1.0])))
}

/// Saturation left to right, value top to bottom: the pure hue under a white-to-clear and a
/// clear-to-black gradient.
fn square(hsv: [f32; 3]) -> impl IntoElement {
    let layer = || div().absolute().inset_0().rounded(px(AREA_RADIUS));
    let white = gpui::white();
    let black = gpui::black();
    div()
        .relative()
        .flex_none()
        .w(px(AREA_WIDTH))
        .h(px(SQUARE_HEIGHT))
        .child(layer().bg(pure_hue(hsv[0])))
        .child(layer().bg(linear_gradient(
            90.,
            linear_color_stop(white, 0.),
            linear_color_stop(white.opacity(0.), 1.),
        )))
        .child(layer().bg(linear_gradient(
            180.,
            linear_color_stop(black.opacity(0.), 0.),
            linear_color_stop(black, 1.),
        )))
        .child(knob(
            hsv[1] * AREA_WIDTH,
            (1.0 - hsv[2]) * SQUARE_HEIGHT,
            hsv_to_rgb(hsv),
        ))
        .child(drag_layer(Area::Square))
}

/// Hue slider: six 32 px segments, each a gradient between two neighbouring primaries.
fn hue_slider(hue: f32) -> impl IntoElement {
    let segment_width = AREA_WIDTH / 6.0;
    let mut segments = div().absolute().inset_0().flex();
    for index in 0..6 {
        let from = pure_hue(index as f32 * 60.0);
        let to = pure_hue((index + 1) as f32 * 60.0);
        segments = segments.child(
            div()
                .flex_none()
                .w(px(segment_width))
                .h_full()
                .when(index == 0, |segment| segment.rounded_l(px(AREA_RADIUS)))
                .when(index == 5, |segment| segment.rounded_r(px(AREA_RADIUS)))
                .bg(linear_gradient(
                    90.,
                    linear_color_stop(from, 0.),
                    linear_color_stop(to, 1.),
                )),
        );
    }
    div()
        .relative()
        .flex_none()
        .w(px(AREA_WIDTH))
        .h(px(HUE_HEIGHT))
        .child(segments)
        .child(knob(
            hue / 360.0 * AREA_WIDTH,
            HUE_HEIGHT / 2.0,
            hsv_to_rgb([hue, 1.0, 1.0]),
        ))
        .child(drag_layer(Area::Hue))
}

/// The picker card for `column`, beside the style card.
pub fn card(
    column: Column,
    palette: &Palette,
    window: &mut Window,
    cx: &mut Context<DocTab>,
) -> AnyElement {
    let field = hex_field(window, cx);
    let state = tool_state(cx);
    let hsv = state.picker.hsv;
    let rgb = hsv_to_rgb(hsv);
    let recent = state.recent_colors.clone();
    if std::mem::take(&mut state.picker.sync) {
        field.update(cx, |field, cx| field.set_value(format_hex(rgb), window, cx));
    }

    let title = match column {
        Column::Stroke => "Custom colour",
        Column::Fill => "Custom fill",
    };
    let close = widgets::small_icon_button("picker-close", icons::X, "Close", palette).on_click(
        |_: &ClickEvent, _, cx: &mut App| {
            tool_state(cx).picker.open = None;
            cx.refresh_windows();
        },
    );
    let header = div()
        .flex()
        .items_center()
        .justify_between()
        .h(px(widgets::SMALL_ICON_BUTTON))
        .child(div().pl(px(2.)).text_size(px(fonts::BODY)).child(title))
        .child(close);
    let preview = div()
        .flex_none()
        .w(px(40.))
        .h(px(crate::theme::INTERACT_HEIGHT))
        .rounded(px(6.))
        .border_1()
        .border_color(palette.border)
        .bg(hsla(rgb_to_color(rgb)));
    let hex_row = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(preview)
        .child(div().flex_1().child(Input::new(&field).small()));
    let mut swatches = div().flex().gap(px(2.));
    for (index, &color) in recent.iter().enumerate() {
        swatches = swatches.child(widgets::tooltip(
            quickbar::swatch(
                ("recent-color", index),
                Some(rgb_to_color(color)),
                color == rgb,
                palette,
            )
            .on_click(move |_: &ClickEvent, window, cx: &mut App| {
                let picker = &mut tool_state(cx).picker;
                picker.hsv = rgb_to_hsv(color);
                picker.sync = true;
                commit(color, window, cx);
                cx.refresh_windows();
            }),
            SharedString::from(format_hex(color)),
        ));
    }
    let recent_row = div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(
            div()
                .text_size(px(9.))
                .text_color(palette.text_dim)
                .child("Recent"),
        )
        .child(if recent.is_empty() {
            div()
                .h(px(26.))
                .flex()
                .items_center()
                .text_size(px(11.))
                .text_color(palette.text_dim)
                .child("Colours you pick show here")
        } else {
            swatches
        });
    quickbar::card(palette)
        .id("color-picker")
        .p(px(10.))
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(header)
        .child(square(hsv))
        .child(hue_slider(hsv[0]))
        .child(hex_row)
        .child(recent_row)
        .into_any_element()
}

/// Card width: area, padding 10 each side, 1 px border each side.
pub const CARD_WIDTH: f32 = AREA_WIDTH + 22.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parses_with_or_without_hash_and_shorthand() {
        assert_eq!(parse_hex("#1F5FD6"), Some(0x1F5FD6));
        assert_eq!(parse_hex("1f5fd6"), Some(0x1F5FD6));
        assert_eq!(parse_hex("  #abc "), Some(0xAABBCC));
        assert_eq!(parse_hex("F00"), Some(0xFF0000));
        for bad in [
            "", "#", "#12345", "#1234567", "12g456", "+12345", "#-12", "##123",
        ] {
            assert_eq!(parse_hex(bad), None, "{bad:?}");
        }
        assert_eq!(format_hex(0x0A0B0C), "#0A0B0C");
        assert_eq!(parse_hex(&format_hex(0xD92D20)), Some(0xD92D20));
    }

    #[test]
    fn hsv_round_trips_every_rgb_sample() {
        for r in (0..=255).step_by(15) {
            for g in (0..=255).step_by(15) {
                for b in (0..=255).step_by(15) {
                    let rgb = r << 16 | g << 8 | b;
                    assert_eq!(hsv_to_rgb(rgb_to_hsv(rgb)), rgb, "{rgb:06X}");
                }
            }
        }
        assert_eq!(hsv_to_rgb([0.0, 1.0, 1.0]), 0xFF0000);
        assert_eq!(hsv_to_rgb([120.0, 1.0, 1.0]), 0x00FF00);
        assert_eq!(hsv_to_rgb([360.0, 1.0, 1.0]), 0xFF0000);
        assert_eq!(hsv_to_rgb([200.0, 0.0, 1.0]), 0xFFFFFF);
        assert_eq!(rgb_to_hsv(0x000000), [0.0, 0.0, 0.0]);
        assert_eq!(color_to_rgb(rgb_to_color(0x39FF14)), 0x39FF14);
    }

    #[test]
    fn recent_colours_are_deduplicated_and_capped() {
        let mut recent = Vec::new();
        for rgb in 1..=8 {
            push_recent(&mut recent, rgb);
        }
        assert_eq!(recent, [8, 7, 6, 5, 4, 3]);
        push_recent(&mut recent, 5);
        assert_eq!(recent, [5, 8, 7, 6, 4, 3]);
        push_recent(&mut recent, 5);
        assert_eq!(recent.len(), RECENT_MAX);
    }
}
