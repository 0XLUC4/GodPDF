//! Canvas tools: the active [`Tool`], per-tool styles and the floating quick bar drawn over
//! the left edge of the reader (tool toggles, undo/redo, contextual style card).

use egui::{
    Align2, Color32, CornerRadius, CursorIcon, Frame, Id, Key, Margin, Modifiers, Order, Pos2,
    Rect, RichText, Sense, Shadow, Stroke, StrokeKind, Ui, vec2,
};

use crate::commands::{Cmd, CommentTool};
use crate::theme::Palette;
use crate::tools::CommentStyle;
use crate::widgets::{self, icon};

/// What a click or drag on the page does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    /// Select, move and delete annotations. Never creates anything.
    #[default]
    Select,
    /// Drag to pan.
    Hand,
    Text,
    Note,
    Highlight,
    Ink,
    Rectangle,
    /// Click or drag over comments to remove them.
    Eraser,
}

impl Tool {
    pub const ALL: [Tool; 8] = [
        Tool::Select,
        Tool::Hand,
        Tool::Text,
        Tool::Note,
        Tool::Highlight,
        Tool::Ink,
        Tool::Rectangle,
        Tool::Eraser,
    ];

    /// The comment tool this is, if it creates annotations.
    pub fn comment(self) -> Option<CommentTool> {
        match self {
            Self::Select | Self::Hand | Self::Eraser => None,
            Self::Text => Some(CommentTool::FreeText),
            Self::Note => Some(CommentTool::Note),
            Self::Highlight => Some(CommentTool::Highlight),
            Self::Ink => Some(CommentTool::Ink),
            Self::Rectangle => Some(CommentTool::Rectangle),
        }
    }

    pub fn from_comment(tool: CommentTool) -> Self {
        match tool {
            CommentTool::FreeText => Self::Text,
            CommentTool::Note => Self::Note,
            CommentTool::Highlight => Self::Highlight,
            CommentTool::Ink => Self::Ink,
            CommentTool::Rectangle => Self::Rectangle,
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Select => icon::CURSOR,
            Self::Hand => icon::HAND,
            Self::Text => icon::TEXT_T,
            Self::Note => icon::NOTE,
            Self::Highlight => icon::HIGHLIGHTER,
            Self::Ink => icon::PENCIL_SIMPLE,
            Self::Rectangle => icon::SQUARE,
            Self::Eraser => icon::ERASER,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::Hand => "Hand",
            Self::Text => "Text box",
            Self::Note => "Sticky note",
            Self::Highlight => "Highlight",
            Self::Ink => "Draw",
            Self::Rectangle => "Rectangle",
            Self::Eraser => "Eraser",
        }
    }

    /// Single-key shortcut (no modifier).
    pub fn key(self) -> Option<Key> {
        match self {
            Self::Select => Some(Key::V),
            Self::Hand => Some(Key::H),
            Self::Text => Some(Key::T),
            Self::Note => Some(Key::N),
            Self::Ink => Some(Key::D),
            Self::Rectangle => Some(Key::R),
            Self::Eraser => Some(Key::E),
            Self::Highlight => None,
        }
    }

    fn tooltip(self) -> String {
        match self.key() {
            Some(key) => format!("{} ({})", self.label(), key.name()),
            None => self.label().to_owned(),
        }
    }

    /// One-line hint shown in the style card.
    pub fn hint(self) -> &'static str {
        match self.comment() {
            Some(tool) => tool.hint(),
            None if self == Self::Hand => "Drag to move around",
            None if self == Self::Eraser => "Click or drag over comments to erase them",
            None => "Click a comment to select it, drag to move it",
        }
    }
}

/// Tool chosen with a single key (V, H, T, N, D, R, E), consumed from the input.
/// Call only when no text field has keyboard focus.
pub fn key_tool(ctx: &egui::Context) -> Option<Tool> {
    ctx.input_mut(|input| {
        Tool::ALL.into_iter().find(|tool| {
            tool.key()
                .is_some_and(|key| input.consume_key(Modifiers::NONE, key))
        })
    })
}

/// Which style controls a target has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleKind {
    /// Color + font size.
    Text,
    /// Color + stroke width.
    Stroke,
    /// Color only.
    Fill,
}

/// Content colors written into the PDF (the only colors the UI shows besides greys).
pub const SWATCHES: [(Color32, &str); 5] = [
    (Color32::BLACK, "Black"),
    (Color32::from_rgb(0x55, 0x55, 0x55), "Dark grey"),
    (Color32::from_rgb(0xD9, 0x2D, 0x20), "Red"),
    (Color32::from_rgb(0x1F, 0x5F, 0xD6), "Blue"),
    (Color32::from_rgb(0xF2, 0xC9, 0x4C), "Yellow"),
];
const YELLOW: Color32 = SWATCHES[4].0;

/// Style for new comments, one per tool kind (text defaults to black, highlight to yellow).
pub struct Styles {
    pub text: CommentStyle,
    pub highlight: CommentStyle,
    pub ink: CommentStyle,
    pub rectangle: CommentStyle,
}

impl Default for Styles {
    fn default() -> Self {
        let style = |color| CommentStyle {
            color,
            width: 2.0,
            font_size: 14.0,
        };
        Self {
            text: style(Color32::BLACK),
            highlight: style(YELLOW),
            ink: style(Color32::BLACK),
            rectangle: style(Color32::BLACK),
        }
    }
}

impl Styles {
    /// Style of a comment tool; `None` (and notes, which have a fixed look) give the text style.
    pub fn get_mut(&mut self, tool: Option<CommentTool>) -> &mut CommentStyle {
        match tool {
            Some(CommentTool::Highlight) => &mut self.highlight,
            Some(CommentTool::Ink) => &mut self.ink,
            Some(CommentTool::Rectangle) => &mut self.rectangle,
            Some(CommentTool::FreeText | CommentTool::Note) | None => &mut self.text,
        }
    }

    pub fn get(&self, tool: CommentTool) -> &CommentStyle {
        match tool {
            CommentTool::Highlight => &self.highlight,
            CommentTool::Ink => &self.ink,
            CommentTool::Rectangle => &self.rectangle,
            CommentTool::FreeText | CommentTool::Note => &self.text,
        }
    }
}

/// What the style card edits.
pub enum StyleTarget {
    /// New comments of this tool.
    Tool(Tool),
    /// The selected annotation: a copy of its style; edits are returned as `restyle`.
    Selected(StyleKind, CommentStyle),
    /// The text box being typed (edits the text style live).
    TextBox,
}

#[derive(Default)]
pub struct Output {
    pub tool: Option<Tool>,
    pub cmd: Option<Cmd>,
    /// New style for the selected annotation.
    pub restyle: Option<CommentStyle>,
}

/// Distance of the bar from the canvas' top-left corner.
const INSET: f32 = 16.0;
const CARD_RADIUS: u8 = 10;

/// Draw the quick bar over `canvas` (the reader's rect).
pub fn show(
    ui: &Ui,
    canvas: Rect,
    tool: Tool,
    styles: &mut Styles,
    target: StyleTarget,
    can_undo: bool,
    can_redo: bool,
) -> Output {
    let palette = Palette::of(ui);
    let mut out = Output::default();
    egui::Area::new(Id::new("quick-bar"))
        .order(Order::Middle)
        .fixed_pos(canvas.min + vec2(INSET, INSET))
        .constrain_to(canvas)
        .show(ui.ctx(), |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 6.0);
            card(&palette).show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
                ui.vertical(|ui| {
                    for candidate in Tool::ALL {
                        if tool_toggle(ui, candidate, candidate == tool, &palette).clicked() {
                            out.tool = Some(candidate);
                        }
                    }
                    divider(ui, &palette);
                    let undo = ui.add_enabled_ui(can_undo, |ui| {
                        widgets::icon_button(ui, icon::ARROW_U_UP_LEFT, "Undo (Ctrl+Z)")
                    });
                    if undo.inner.clicked() {
                        out.cmd = Some(Cmd::Undo);
                    }
                    let redo = ui.add_enabled_ui(can_redo, |ui| {
                        widgets::icon_button(ui, icon::ARROW_U_UP_RIGHT, "Redo (Ctrl+Y)")
                    });
                    if redo.inner.clicked() {
                        out.cmd = Some(Cmd::Redo);
                    }
                });
            });
            style_card(ui, target, styles, &palette, &mut out);
        });
    out
}

fn card(palette: &Palette) -> Frame {
    Frame::new()
        .fill(palette.panel)
        .stroke(Stroke::new(1.0, palette.border))
        .corner_radius(CornerRadius::same(CARD_RADIUS))
        .inner_margin(Margin::same(4))
        .shadow(Shadow {
            offset: [0, 4],
            blur: 16,
            spread: 0,
            color: Color32::from_black_alpha(28),
        })
}

/// Square icon toggle; the active tool sits on a light grey square.
fn tool_toggle(ui: &mut Ui, tool: Tool, active: bool, palette: &Palette) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(widgets::ICON_BUTTON, Sense::click());
    let fill = if active {
        palette.selected
    } else if response.hovered() {
        palette.hover
    } else {
        Color32::TRANSPARENT
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(6), fill);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        tool.icon(),
        egui::FontId::proportional(16.0),
        palette.text,
    );
    response
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(tool.tooltip())
}

fn divider(ui: &mut Ui, palette: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(widgets::ICON_BUTTON.x, 7.0), Sense::hover());
    ui.painter().hline(
        rect.x_range().shrink(4.0),
        rect.center().y,
        Stroke::new(1.0, palette.border),
    );
}

fn style_card(
    ui: &mut Ui,
    target: StyleTarget,
    styles: &mut Styles,
    palette: &Palette,
    out: &mut Output,
) {
    let (kind, hint, mut selected_copy) = match target {
        StyleTarget::Tool(tool) => {
            let kind = match tool {
                Tool::Text => StyleKind::Text,
                Tool::Ink | Tool::Rectangle => StyleKind::Stroke,
                Tool::Highlight => StyleKind::Fill,
                Tool::Note => {
                    hint_card(ui, tool.hint(), palette);
                    return;
                }
                Tool::Select | Tool::Hand | Tool::Eraser => return,
            };
            (kind, Some(tool), None)
        }
        StyleTarget::Selected(kind, style) => (kind, None, Some(style)),
        StyleTarget::TextBox => (StyleKind::Text, None, None),
    };
    let style: &mut CommentStyle = match (&mut selected_copy, hint) {
        (Some(copy), _) => copy,
        (None, Some(tool)) => styles.get_mut(tool.comment()),
        (None, None) => &mut styles.text,
    };
    let before = (style.color, style.width, style.font_size);
    card(palette).show(ui, |ui| {
        ui.set_width(widgets::ICON_BUTTON.x);
        ui.spacing_mut().item_spacing = vec2(0.0, 4.0);
        ui.vertical_centered(|ui| {
            for (color, name) in SWATCHES {
                if swatch(ui, color, style.color == color, palette)
                    .on_hover_text(name)
                    .clicked()
                {
                    style.color = color;
                }
            }
            divider(ui, palette);
            match kind {
                StyleKind::Text => stepper(ui, &mut style.font_size, 6.0..=72.0, 1.0, "Font size"),
                StyleKind::Stroke => stepper(ui, &mut style.width, 0.5..=12.0, 0.5, "Line width"),
                StyleKind::Fill => {}
            }
        });
    });
    if let Some(copy) = selected_copy
        && before != (copy.color, copy.width, copy.font_size)
    {
        out.restyle = Some(copy);
    }
}

fn hint_card(ui: &mut Ui, text: &str, palette: &Palette) {
    card(palette).show(ui, |ui| {
        ui.set_max_width(widgets::ICON_BUTTON.x);
        ui.label(RichText::new(icon::INFO).color(palette.text_dim))
            .on_hover_text(text);
    });
}

/// Round color dot; the current one gets a ring.
fn swatch(ui: &mut Ui, color: Color32, current: bool, palette: &Palette) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
    let painter = ui.painter();
    let center = rect.center();
    if current {
        painter.circle_stroke(center, 10.0, Stroke::new(1.5, palette.text));
    }
    painter.circle(center, 7.0, color, Stroke::new(1.0, palette.border));
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// Vertical `+ / value / −` stepper.
fn stepper(
    ui: &mut Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    step: f32,
    what: &str,
) {
    if widgets::small_icon_button(ui, icon::PLUS, &format!("Larger {}", what.to_lowercase()))
        .clicked()
    {
        *value = (*value + step).min(*range.end());
    }
    let text = if step < 1.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.0}")
    };
    ui.add(egui::Label::new(RichText::new(text).small()).selectable(false))
        .on_hover_text(what);
    if widgets::small_icon_button(ui, icon::MINUS, &format!("Smaller {}", what.to_lowercase()))
        .clicked()
    {
        *value = (*value - step).max(*range.start());
    }
}

/// Paint a dashed rectangle outline (selection).
pub fn dashed_rect(painter: &egui::Painter, rect: Rect, stroke: Stroke) {
    let points: Vec<Pos2> = vec![
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ];
    painter.extend(egui::Shape::dashed_line(&points, stroke, 4.0, 3.0));
}

/// Small square handles at the corners of a selection.
pub fn handles(painter: &egui::Painter, rect: Rect) {
    for corner in [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ] {
        let handle = Rect::from_center_size(corner, vec2(6.0, 6.0));
        painter.rect(
            handle,
            0.0,
            Color32::WHITE,
            Stroke::new(1.0, Color32::BLACK),
            StrokeKind::Inside,
        );
    }
}
