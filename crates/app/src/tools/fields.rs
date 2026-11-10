//! Form fields filled in right on the page (ported from the egui `src/fieldedit.rs`): with
//! the Select tool a click on a text field opens a box to type in, a click on a check box
//! ticks it, a radio button or a list offers its options. Also the Field tool's box.

use std::collections::HashMap;
use std::rc::Rc;

use godpdf_core::{FieldKind, FieldValue, FieldWidget};
use gpui::component::button::{Button, ButtonVariants};
use gpui::component::input::{Escape, Input, InputEvent, InputState, Textarea, TextareaState};
use gpui::component::menu::{DropdownMenu, PopupMenuItem};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, CursorStyle, ElementId, Entity, Hsla, MouseButton, Rgba,
    Subscription, Window, black, div, px, white,
};

use super::canvas::{boxed, page_frame};
use super::geom::DRect;
use crate::doc::{DocTab, guarded};
use crate::toasts;

/// Fill over a field: egui's (30, 80, 200, 28), read as straight alpha so it shows on white.
const TINT: Rgba = Rgba {
    r: 30.0 / 255.0,
    g: 80.0 / 255.0,
    b: 200.0 / 255.0,
    a: 28.0 / 255.0,
};
/// Outline of a hovered or typed field: #2563EB.
const EDGE: u32 = 0x2563EB;
/// Box of a field added with a click (no drag), in points.
const STANDARD: (f32, f32) = (160.0, 22.0);

/// The field boxes of each page, read once per document revision, and the text field being
/// typed in.
#[derive(Default)]
pub struct Fields {
    generation: u64,
    widgets: HashMap<usize, Rc<Vec<FieldWidget>>>,
    edit: Option<FieldEdit>,
}

/// A text field being typed in. `rect` is in displayed points.
struct FieldEdit {
    page: usize,
    name: String,
    rect: DRect,
    multiline: bool,
    typed: Typed,
    _events: Subscription,
}

enum Typed {
    Line(Entity<InputState>),
    Area(Entity<TextareaState>),
}

impl Typed {
    fn value(&self, cx: &App) -> String {
        match self {
            Self::Line(state) => state.read(cx).value().to_string(),
            Self::Area(state) => state.read(cx).value().to_string(),
        }
    }
}

/// The field boxes of `page` (egui `DocTab::field_widgets`).
fn widgets(tab: &mut DocTab, page: usize) -> Rc<Vec<FieldWidget>> {
    let generation = tab.generation();
    let fields = &mut tab.canvas.fields;
    if fields.generation != generation {
        fields.widgets.clear();
        fields.generation = generation;
    }
    let bytes = tab.session.bytes();
    Rc::clone(fields.widgets.entry(page).or_insert_with(|| {
        Rc::new(guarded(|| godpdf_core::field_widgets(bytes, page)).unwrap_or_default())
    }))
}

/// Write `value` into the field `name`: one undo step.
fn fill(tab: &mut DocTab, name: String, value: FieldValue, cx: &mut Context<DocTab>) {
    tab.canvas.fields.edit = None;
    let changes = [(name, value)];
    let result = tab.apply("Fill in form", |pdf| godpdf_core::fill_form(pdf, &changes));
    if let Err(err) = result {
        toasts::error(cx, err);
    }
    cx.notify();
}

/// The box being typed is done: write what it says.
fn commit(tab: &mut DocTab, cx: &mut Context<DocTab>) {
    let Some(edit) = tab.canvas.fields.edit.take() else {
        return;
    };
    let value = edit.typed.value(cx);
    fill(tab, edit.name, FieldValue::Text(value), cx);
}

/// Open the typing box over the text field `name`.
#[allow(clippy::too_many_arguments)]
fn open(
    tab: &mut DocTab,
    page: usize,
    name: String,
    rect: DRect,
    text: String,
    multiline: bool,
    window: &mut Window,
    cx: &mut Context<DocTab>,
) {
    commit(tab, cx);
    let (typed, events) = if multiline {
        let state = cx.new(|cx| TextareaState::new(window, cx).default_value(text));
        state.update(cx, |state, cx| state.focus(window, cx));
        let events = cx.subscribe(&state, |tab, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Blur) {
                commit(tab, cx);
            }
        });
        (Typed::Area(state), events)
    } else {
        let state = cx.new(|cx| InputState::new(window, cx).default_value(text));
        state.update(cx, |state, cx| state.focus(window, cx));
        let events = cx.subscribe(&state, |tab, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                commit(tab, cx);
            }
        });
        (Typed::Line(state), events)
    };
    // Keep the focus on the box: the window root would take it on this same click.
    window.prevent_default();
    tab.canvas.fields.edit = Some(FieldEdit {
        page,
        name,
        rect,
        multiline,
        typed,
        _events: events,
    });
}

/// The clickable boxes of the fields of `page` (Select tool), over the comments.
pub fn render(
    tab: &mut DocTab,
    page: usize,
    zoom: f32,
    cx: &mut Context<DocTab>,
) -> Vec<AnyElement> {
    let Some((geom, displayed)) = page_frame(tab, page) else {
        return Vec::new();
    };
    let edge: Hsla = gpui::rgb(EDGE).into();
    let editing = tab
        .canvas
        .fields
        .edit
        .as_ref()
        .filter(|edit| edit.page == page)
        .map(|edit| edit.name.clone());
    let mut boxes = Vec::new();
    for (index, widget) in widgets(tab, page).iter().enumerate() {
        let field = &widget.field;
        if field.read_only || editing.as_ref() == Some(&field.name) {
            continue;
        }
        let shown = DRect::from_pdf(&geom, displayed, &widget.rect);
        let id = ElementId::Name(format!("form-field-{page}-{index}").into());
        let name = field.name.clone();
        let element = match (&field.kind, &field.value) {
            (FieldKind::Choice { options }, FieldValue::Choice(chosen)) => {
                let tab_entity = cx.entity().downgrade();
                let (options, chosen) = (options.clone(), chosen.clone());
                // The page under the list must not get the click.
                boxed(shown, zoom, 0.0)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        Button::new(id)
                            .ghost()
                            .size_full()
                            .rounded(px(2.))
                            .bg(TINT)
                            .tooltip(field.label.clone())
                            .dropdown_menu(move |mut menu, _, _| {
                                for option in &options {
                                    let (tab, name, value) =
                                        (tab_entity.clone(), name.clone(), option.clone());
                                    menu = menu.item(
                                        PopupMenuItem::new(option.clone())
                                            .checked(*option == chosen)
                                            .on_click(move |_, _, cx: &mut App| {
                                                let Some(tab) = tab.upgrade() else { return };
                                                let value = FieldValue::Choice(value.clone());
                                                let name = name.clone();
                                                tab.update(cx, |tab, cx| {
                                                    fill(tab, name, value, cx)
                                                });
                                            }),
                                    );
                                }
                                menu
                            }),
                    )
                    .into_any_element()
            }
            (kind, value) => {
                let kind = kind.clone();
                let value = value.clone();
                let option = widget.option.clone();
                let text_box = div()
                    .id(id)
                    .absolute()
                    .left(px(shown.x0 * zoom))
                    .top(px(shown.y0 * zoom))
                    .w(px(shown.width() * zoom))
                    .h(px(shown.height() * zoom))
                    .rounded(px(2.))
                    .bg(TINT)
                    .cursor(CursorStyle::PointingHand)
                    .hover(move |style| style.border_1().border_color(edge))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |tab, _, window, cx| {
                            cx.stop_propagation();
                            let name = name.clone();
                            match (&kind, &value) {
                                (FieldKind::Text { multiline }, FieldValue::Text(text)) => {
                                    open(
                                        tab,
                                        page,
                                        name,
                                        shown,
                                        text.clone(),
                                        *multiline,
                                        window,
                                        cx,
                                    );
                                }
                                (FieldKind::CheckBox, FieldValue::Checked(checked)) => {
                                    fill(tab, name, FieldValue::Checked(!checked), cx);
                                }
                                (FieldKind::Radio { .. }, _) => {
                                    if let Some(option) = option.clone() {
                                        fill(tab, name, FieldValue::Choice(option), cx);
                                    }
                                }
                                _ => {}
                            }
                            cx.notify();
                        }),
                    );
                crate::widgets::tooltip(text_box, field.label.clone()).into_any_element()
            }
        };
        boxes.push(element);
    }
    boxes
}

/// The box to type a text field in, over the field (egui `fieldedit::show`).
pub fn editor(
    tab: &DocTab,
    page: usize,
    zoom: f32,
    cx: &mut Context<DocTab>,
) -> Option<AnyElement> {
    let edit = tab
        .canvas
        .fields
        .edit
        .as_ref()
        .filter(|edit| edit.page == page)?;
    let rect = edit.rect;
    let font_size = if edit.multiline {
        10.0 * zoom
    } else {
        (rect.height() * 0.6).clamp(6.0, 14.0) * zoom
    };
    let height = if edit.multiline {
        rect.height() * zoom
    } else {
        (rect.height() * zoom).max(font_size * 1.6)
    };
    let center_y = (rect.y0 + rect.y1) / 2.0 * zoom;
    let input = match &edit.typed {
        Typed::Line(state) => Input::new(state)
            .appearance(false)
            .text_size(px(font_size))
            .text_color(black())
            .into_any_element(),
        Typed::Area(state) => Textarea::new(state)
            .appearance(false)
            .h(px(height - 2.0))
            .text_size(px(font_size))
            .text_color(black())
            .into_any_element(),
    };
    Some(
        div()
            .id("form-field-edit")
            .absolute()
            .left(px(rect.x0 * zoom - 1.0))
            .top(px(center_y - height / 2.0 - 1.0))
            .w(px((rect.width() * zoom).max(60.0) + 2.0))
            .h(px(height + 2.0))
            .px(px(3.))
            .py(px(1.))
            .rounded(px(2.))
            .bg(white())
            .border(px(1.5))
            .border_color(gpui::rgb(EDGE))
            .occlude()
            .on_action(cx.listener(|tab, _: &Escape, _, cx| {
                tab.canvas.fields.edit = None;
                cx.notify();
            }))
            .child(input)
            .into_any_element(),
    )
}

/// The Field tool's drag (or click, for a box of standard size) is done: ask kind and name.
pub fn new_field(
    tab: &mut DocTab,
    page: usize,
    first: (f32, f32),
    last: (f32, f32),
    tiny: bool,
    cx: &mut Context<DocTab>,
) {
    let Some((geom, displayed)) = page_frame(tab, page) else {
        return;
    };
    let end = if tiny {
        (first.0 + STANDARD.0, first.1 + STANDARD.1)
    } else {
        last
    };
    let rect = crate::geometry::to_pdf_rect(&geom, displayed, first, end);
    crate::dialogs::new_field(page, rect, cx);
}
