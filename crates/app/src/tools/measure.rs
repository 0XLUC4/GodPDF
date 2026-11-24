//! The scale of the measuring tools: one per open document (kept in its [`super::Canvas`]),
//! set in a card beside the style card, as "1 cm = 2 m".

use godpdf_core::Scale;
use gpui::component::Sizable;
use gpui::component::input::{Input, InputEvent, InputState};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, EntityId, Global, SharedString, Subscription, Window, div, px,
};

use super::quickbar::card;
use super::{Tool, current_tool, with_active};
use crate::doc::DocTab;
use crate::theme::Palette;
use crate::{fonts, icons, toasts, widgets};

/// Width of the scale field.
const FIELD_WIDTH: f32 = 128.0;

/// The scale field, created once and shared by every tab.
struct ScaleField {
    input: Entity<InputState>,
    /// The tab and scale the field shows; `None` to show the active tab's scale again.
    shown: Option<(EntityId, Scale)>,
    _subscription: Subscription,
}

impl Global for ScaleField {}

pub(super) fn is_measure_tool(tool: Tool) -> bool {
    Tool::MEASURE.contains(&tool)
}

/// The scale card, while a measuring tool is in use.
pub(super) fn scale_card(
    tab: &DocTab,
    palette: &Palette,
    window: &mut Window,
    cx: &mut Context<DocTab>,
) -> Option<AnyElement> {
    if !is_measure_tool(current_tool(cx)) || !tab.can_edit() {
        return None;
    }
    let input = field(window, cx);
    let key = (cx.entity_id(), tab.canvas.scale);
    let field = cx.global_mut::<ScaleField>();
    if field.shown != Some(key) {
        field.shown = Some(key);
        let text = tab.canvas.scale.to_string();
        input.update(cx, |state, cx| state.set_value(text, window, cx));
    }
    let heading = widgets::tooltip(
        div().id("scale-heading").child(widgets::with_icon(
            icons::RULER,
            "Scale",
            fonts::BODY,
            palette.text_dim,
        )),
        "Set scale: page length = real length (mm, cm, m, in, ft), then Enter",
    );
    Some(
        card(palette)
            .flex()
            .flex_col()
            .gap(px(6.))
            .text_size(px(fonts::BODY))
            .child(heading)
            .child(div().w(px(FIELD_WIDTH)).child(Input::new(&input).small()))
            .into_any_element(),
    )
}

/// The scale field, created the first time the card shows.
fn field(window: &mut Window, cx: &mut App) -> Entity<InputState> {
    if let Some(field) = cx.try_global::<ScaleField>() {
        return field.input.clone();
    }
    let input = cx.new(|cx| InputState::new(window, cx).placeholder("1 cm = 2 m"));
    let subscription = cx.subscribe(
        &input,
        |state: Entity<InputState>, event: &InputEvent, cx: &mut App| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                let text = state.read(cx).value().to_string();
                set_scale(&text, cx);
            }
        },
    );
    cx.set_global(ScaleField {
        input: input.clone(),
        shown: None,
        _subscription: subscription,
    });
    input
}

/// A scale typed in the field: becomes the scale of the document shown.
fn set_scale(text: &str, cx: &mut App) {
    let Some(scale) = Scale::parse(text) else {
        toasts::error(
            cx,
            format!(
                "“{}” is not a scale: write it as 1 cm = 2 m (mm, cm, m, in, ft)",
                text.trim()
            ),
        );
        // Back to the scale in use.
        cx.global_mut::<ScaleField>().shown = None;
        cx.refresh_windows();
        return;
    };
    let label = SharedString::from(format!("Scale set: {scale}"));
    with_active(cx, |tab, cx| {
        if tab.canvas.scale != scale {
            tab.canvas.scale = scale;
            toasts::success(cx, label);
        }
    });
}
