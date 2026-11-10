//! Canvas tools:
//! the active [`Tool`], per-tool styles, the floating quick bar over the viewer and the
//! per-page interactions (select / move / resize / delete, draw, erase, pan, type, pictures).
//!
//! App-wide state is the [`ToolState`] global (tool, styles, picture card fields, the tab
//! shown). Per-document state is [`Canvas`], a field of `DocTab` (selection, gesture, text
//! editor, annotation cache), so it goes away with its tab.

mod canvas;
mod editor;
mod fields;
mod geom;
mod picker;
mod quickbar;
mod richtext;
mod sign;
mod stamp;

use std::ops::Range;

use gpui::{
    Action, AnyElement, App, Context, Global, Hsla, KeyBinding, Rgba, WeakEntity, Window, actions,
};

use godpdf_core::{CharStyle, Color, Font, NewField, Script, StampLabel, ZOrder};

use crate::doc::DocTab;
use crate::geometry::PageRect;
use crate::icons;

pub use canvas::Canvas;
pub use picker::parse_hex;
pub use sign::place_in as place_signature;

actions!(
    godpdf,
    [
        SelectTool,
        HandTool,
        TextTool,
        NoteTool,
        HighlightTool,
        InkTool,
        RectangleTool,
        EllipseTool,
        LineTool,
        ArrowTool,
        UnderlineTool,
        StrikeOutTool,
        StampTool,
        RedactTool,
        PictureTool,
        EraserTool,
        SignTool,
        FieldTool,
        DeleteSelection,
        Deselect,
        BringToFront,
        BringForward,
        SendBackward,
        SendToBack,
        ToggleBold,
        ToggleItalic,
        ToggleUnderline
    ]
);

/// Single-key shortcuts must not fire while typing in a text field.
const CANVAS_KEYS: &str = "GodPdf && !Input";
/// Key context of the text box being typed.
const TEXT_BOX: &str = "TextBox";

/// What a click or drag on the page does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    /// Select, move, resize and delete annotations. Never creates anything.
    #[default]
    Select,
    /// Drag to pan.
    Hand,
    Text,
    Note,
    Highlight,
    Ink,
    Rectangle,
    Ellipse,
    Line,
    Arrow,
    Underline,
    StrikeOut,
    Stamp,
    /// Mark areas to redact; nothing is removed until "Apply redactions".
    Redact,
    Picture,
    /// Chosen from the signature panel, not shown in the bar.
    Sign,
    /// Chosen from the form panel, not shown in the bar.
    Field,
    /// Click or drag over comments to remove them, or brush the page content away.
    Eraser,
}

impl Tool {
    /// The quick bar, top to bottom: the drawing tools, plus stamp and redact before the
    /// eraser. `Highlight` and `Rectangle` stand for their groups ([`Tool::group`]): the
    /// button shows the last tool picked in it, the style card picks another.
    pub const BAR: [Tool; 11] = [
        Tool::Select,
        Tool::Hand,
        Tool::Text,
        Tool::Note,
        Tool::Highlight,
        Tool::Ink,
        Tool::Rectangle,
        Tool::Picture,
        Tool::Stamp,
        Tool::Redact,
        Tool::Eraser,
    ];
    /// The shapes group.
    pub const SHAPES: [Tool; 4] = [Tool::Rectangle, Tool::Ellipse, Tool::Line, Tool::Arrow];
    /// The text markup group.
    pub const MARKUP: [Tool; 3] = [Tool::Highlight, Tool::Underline, Tool::StrikeOut];

    pub fn is_shape(self) -> bool {
        Self::SHAPES.contains(&self)
    }

    /// The flyout group of the bar this tool is in, if any.
    pub fn group(self) -> Option<&'static [Tool]> {
        [&Self::SHAPES[..], &Self::MARKUP[..]]
            .into_iter()
            .find(|group| group.contains(&self))
    }

    /// Phosphor glyph.
    fn icon(self) -> &'static str {
        match self {
            Self::Select => icons::CURSOR,
            Self::Hand => icons::HAND,
            Self::Text => icons::TEXT_T,
            Self::Note => icons::NOTE,
            Self::Highlight => icons::HIGHLIGHTER,
            Self::Ink => icons::PENCIL_SIMPLE,
            Self::Rectangle => icons::SQUARE,
            Self::Ellipse => icons::CIRCLE,
            Self::Line => icons::LINE_SEGMENT,
            Self::Arrow => icons::ARROW_UP_RIGHT,
            Self::Underline => icons::TEXT_UNDERLINE,
            Self::StrikeOut => icons::TEXT_STRIKETHROUGH,
            Self::Stamp => icons::STAMP,
            Self::Redact => icons::EYE_SLASH,
            Self::Picture => icons::IMAGE,
            Self::Sign => icons::SIGNATURE,
            Self::Field => icons::TEXTBOX,
            Self::Eraser => icons::ERASER,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::Hand => "Hand",
            Self::Text => "Text box",
            Self::Note => "Sticky note",
            Self::Highlight => "Highlight",
            Self::Ink => "Draw",
            Self::Rectangle => "Rectangle",
            Self::Ellipse => "Ellipse",
            Self::Line => "Line",
            Self::Arrow => "Arrow",
            Self::Underline => "Underline",
            Self::StrikeOut => "Strikethrough",
            Self::Stamp => "Stamp",
            Self::Redact => "Redact",
            Self::Picture => "Picture",
            Self::Sign => "Sign",
            Self::Field => "Form field",
            Self::Eraser => "Eraser",
        }
    }

    /// Single-key shortcut shown in the tooltip (bound in [`register`]).
    fn key(self) -> Option<&'static str> {
        match self {
            Self::Select => Some("V"),
            Self::Hand => Some("H"),
            Self::Text => Some("T"),
            Self::Note => Some("N"),
            Self::Ink => Some("D"),
            Self::Rectangle => Some("R"),
            Self::Ellipse => Some("O"),
            Self::Line => Some("L"),
            Self::Arrow => Some("A"),
            Self::Underline => Some("U"),
            Self::StrikeOut => Some("K"),
            Self::Stamp => Some("S"),
            Self::Redact => Some("X"),
            Self::Picture => Some("I"),
            Self::Eraser => Some("E"),
            Self::Highlight | Self::Sign | Self::Field => None,
        }
    }

    fn action(self) -> Box<dyn Action> {
        match self {
            Self::Select => Box::new(SelectTool),
            Self::Hand => Box::new(HandTool),
            Self::Text => Box::new(TextTool),
            Self::Note => Box::new(NoteTool),
            Self::Highlight => Box::new(HighlightTool),
            Self::Ink => Box::new(InkTool),
            Self::Rectangle => Box::new(RectangleTool),
            Self::Ellipse => Box::new(EllipseTool),
            Self::Line => Box::new(LineTool),
            Self::Arrow => Box::new(ArrowTool),
            Self::Underline => Box::new(UnderlineTool),
            Self::StrikeOut => Box::new(StrikeOutTool),
            Self::Stamp => Box::new(StampTool),
            Self::Redact => Box::new(RedactTool),
            Self::Picture => Box::new(PictureTool),
            Self::Sign => Box::new(SignTool),
            Self::Field => Box::new(FieldTool),
            Self::Eraser => Box::new(EraserTool),
        }
    }

    fn tooltip(self) -> String {
        match self.key() {
            Some(key) => format!("{} ({key})", self.label()),
            None => self.label().to_owned(),
        }
    }

    /// One-line hint shown in the style card of tools without styles.
    fn hint(self) -> &'static str {
        match self {
            Self::Note => "Click where the sticky note goes",
            Self::Picture => "Click where the picture goes, or drag the box it fills",
            Self::Text => "Click on the page to type",
            Self::Highlight => "Drag over the area to highlight",
            Self::Ink => "Draw freehand on the page",
            Self::Rectangle => "Drag to draw a rectangle",
            Self::Ellipse => "Drag to draw an ellipse",
            Self::Line => "Drag to draw a line",
            Self::Arrow => "Drag to draw an arrow",
            Self::Underline => "Drag over the text to underline",
            Self::StrikeOut => "Drag over the text to strike through",
            Self::Stamp => "Pick a stamp, then click where it goes",
            Self::Redact => "Drag over what to remove; click a mark to drop it",
            Self::Sign => "Click where your signature goes",
            Self::Field => "Drag the box of the field, or click for one of standard size",
            Self::Hand => "Drag to move around",
            Self::Eraser => "Click or drag over comments to erase them, or brush page content away",
            Self::Select => "Click a comment to select it, drag to move it",
        }
    }
}

/// Style of new comments of one kind.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub color: Color,
    pub width: f32,
    pub font_size: f32,
    pub font: Font,
    /// Interior of rectangles and ellipses; `None` leaves them hollow.
    pub fill: Option<Color>,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
    }
}

/// Content colors written into the PDF (the only colors the UI shows besides greys).
pub const SWATCHES: [(Color, &str); 7] = [
    (rgb(0, 0, 0), "Black"),
    (rgb(0x55, 0x55, 0x55), "Dark grey"),
    (rgb(0xD9, 0x2D, 0x20), "Red"),
    (rgb(0x1F, 0x5F, 0xD6), "Blue"),
    (rgb(0xFF, 0xF2, 0x00), "Fluo yellow"),
    (rgb(0x39, 0xFF, 0x14), "Fluo green"),
    (rgb(0xFF, 0x3E, 0xC8), "Fluo pink"),
];

pub fn hsla(color: Color) -> Hsla {
    Rgba {
        r: color.r,
        g: color.g,
        b: color.b,
        a: 1.0,
    }
    .into()
}

/// Style for new comments, one per tool kind (text defaults to black, highlight to yellow).
pub struct Styles {
    pub text: Style,
    pub highlight: Style,
    pub ink: Style,
    /// Every shape of the shapes group.
    pub shape: Style,
    pub underline: Style,
    pub strike_out: Style,
}

impl Default for Styles {
    fn default() -> Self {
        let style = |color| Style {
            color,
            width: 2.0,
            font_size: 14.0,
            font: Font::Helvetica,
            fill: None,
        };
        Self {
            text: style(SWATCHES[0].0),
            highlight: style(SWATCHES[4].0),
            ink: style(SWATCHES[0].0),
            shape: style(SWATCHES[0].0),
            underline: style(SWATCHES[3].0),
            strike_out: style(SWATCHES[2].0),
        }
    }
}

impl Styles {
    /// Style of the comments `tool` makes; tools without styles give the text style.
    pub fn of_mut(&mut self, tool: Tool) -> &mut Style {
        match tool {
            Tool::Highlight => &mut self.highlight,
            Tool::Ink => &mut self.ink,
            Tool::Rectangle | Tool::Ellipse | Tool::Line | Tool::Arrow => &mut self.shape,
            Tool::Underline => &mut self.underline,
            Tool::StrikeOut => &mut self.strike_out,
            _ => &mut self.text,
        }
    }

    pub fn of(&self, tool: Tool) -> Style {
        match tool {
            Tool::Highlight => self.highlight,
            Tool::Ink => self.ink,
            Tool::Rectangle | Tool::Ellipse | Tool::Line | Tool::Arrow => self.shape,
            Tool::Underline => self.underline,
            Tool::StrikeOut => self.strike_out,
            _ => self.text,
        }
    }
}

/// Character formats the style card toggles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Bold,
    Italic,
    Underline,
    Sub,
    Super,
}

impl Format {
    pub const ALL: [Format; 5] = [
        Format::Bold,
        Format::Italic,
        Format::Underline,
        Format::Sub,
        Format::Super,
    ];

    pub fn is_on(self, style: CharStyle) -> bool {
        match self {
            Self::Bold => style.bold,
            Self::Italic => style.italic,
            Self::Underline => style.underline,
            Self::Sub => style.script == Script::Sub,
            Self::Super => style.script == Script::Super,
        }
    }

    /// Subscript and superscript exclude each other; turning one off leaves the other.
    pub fn set(self, style: &mut CharStyle, on: bool) {
        match self {
            Self::Bold => style.bold = on,
            Self::Italic => style.italic = on,
            Self::Underline => style.underline = on,
            Self::Sub | Self::Super if !on => {
                if self.is_on(*style) {
                    style.script = Script::Normal;
                }
            }
            Self::Sub => style.script = Script::Sub,
            Self::Super => style.script = Script::Super,
        }
    }
}

/// Flip `format` over the characters `range` of the `len` of `styles`: on unless all of
/// them have it already. An all-default result is the empty list, as core writes it.
pub fn toggle_all(styles: &mut Vec<CharStyle>, len: usize, range: Range<usize>, format: Format) {
    styles.resize(len, CharStyle::default());
    let range = range.start.min(len)..range.end.min(len);
    let on = !styles[range.clone()]
        .iter()
        .all(|style| format.is_on(*style));
    for style in &mut styles[range] {
        format.set(style, on);
    }
    if styles.iter().all(|style| *style == CharStyle::default()) {
        styles.clear();
    }
}

/// Formats every one of the characters `range` has.
pub fn common_style(styles: &[CharStyle], range: Range<usize>) -> CharStyle {
    let mut common = CharStyle::default();
    for format in Format::ALL {
        let on = !range.is_empty()
            && range
                .clone()
                .all(|index| format.is_on(godpdf_core::style_at(styles, index)));
        format.set(&mut common, on);
    }
    common
}

/// App-wide tool state.
pub struct ToolState {
    pub tool: Tool,
    pub styles: Styles,
    /// Picture card: width and height change together.
    pub keep_ratio: bool,
    /// The shape the shapes group button picks.
    pub shape: Tool,
    /// The markup (highlight, underline, strikethrough) its group button picks.
    pub markup: Tool,
    /// The stamp the stamp tool places, and whether it gets today's date.
    pub stamp: StampLabel,
    pub stamp_date: bool,
    /// The kind of field the field tool adds (picked in the form panel).
    pub field_kind: NewField,
    /// The eraser brushes the page content away instead of removing comments.
    pub erase_content: bool,
    /// Side of the page eraser's brush, in points.
    pub brush: f32,
    /// The document the viewer shows, set while it renders: key actions go to it.
    active: Option<WeakEntity<DocTab>>,
    picture_card: Option<quickbar::PictureCard>,
    /// The custom colour picker of the style card.
    picker: picker::Picker,
    /// Custom colours picked last (`0xRRGGBB`), newest first.
    recent_colors: Vec<u32>,
}

impl Default for ToolState {
    fn default() -> Self {
        Self {
            tool: Tool::Select,
            styles: Styles::default(),
            keep_ratio: true,
            shape: Tool::Rectangle,
            markup: Tool::Highlight,
            stamp: StampLabel::Approved,
            stamp_date: false,
            field_kind: NewField::Text,
            erase_content: false,
            brush: 12.0,
            active: None,
            picture_card: None,
            picker: picker::Picker::default(),
            recent_colors: Vec::new(),
        }
    }
}

impl Global for ToolState {}

fn tool_state(cx: &mut App) -> &mut ToolState {
    cx.default_global::<ToolState>()
}

/// Custom colours picked last (`0xRRGGBB`, newest first), for the settings.
pub fn recent_colors(cx: &App) -> Vec<u32> {
    cx.try_global::<ToolState>()
        .map_or_else(Vec::new, |state| state.recent_colors.clone())
}

/// Bring back the custom colours of the last session.
pub fn set_recent_colors(mut colors: Vec<u32>, cx: &mut App) {
    colors.truncate(picker::RECENT_MAX);
    tool_state(cx).recent_colors = colors;
}

fn current_tool(cx: &App) -> Tool {
    cx.try_global::<ToolState>()
        .map_or(Tool::Select, |state| state.tool)
}

/// Run `update` on the document the viewer shows, if any.
fn with_active(cx: &mut App, update: impl FnOnce(&mut DocTab, &mut Context<DocTab>)) {
    let Some(tab) = tool_state(cx)
        .active
        .clone()
        .and_then(|weak| weak.upgrade())
    else {
        return;
    };
    tab.update(cx, |tab, cx| {
        update(tab, cx);
        cx.notify();
    });
}

fn set_tool(tool: Tool, cx: &mut App) {
    with_active(cx, canvas::reset);
    let state = tool_state(cx);
    state.tool = tool;
    state.picker.open = None;
    if tool.is_shape() {
        state.shape = tool;
    } else if Tool::MARKUP.contains(&tool) {
        state.markup = tool;
    }
    cx.refresh_windows();
}

/// Register tool actions and their key bindings.
pub fn register(cx: &mut App) {
    cx.set_global(ToolState::default());
    let keys = Some(CANVAS_KEYS);
    cx.bind_keys([
        KeyBinding::new("v", SelectTool, keys),
        KeyBinding::new("h", HandTool, keys),
        KeyBinding::new("t", TextTool, keys),
        KeyBinding::new("n", NoteTool, keys),
        KeyBinding::new("d", InkTool, keys),
        KeyBinding::new("r", RectangleTool, keys),
        KeyBinding::new("o", EllipseTool, keys),
        KeyBinding::new("l", LineTool, keys),
        KeyBinding::new("a", ArrowTool, keys),
        KeyBinding::new("u", UnderlineTool, keys),
        KeyBinding::new("k", StrikeOutTool, keys),
        KeyBinding::new("s", StampTool, keys),
        KeyBinding::new("x", RedactTool, keys),
        KeyBinding::new("i", PictureTool, keys),
        KeyBinding::new("e", EraserTool, keys),
        KeyBinding::new("delete", DeleteSelection, keys),
        KeyBinding::new("backspace", DeleteSelection, keys),
        KeyBinding::new("escape", Deselect, keys),
        // Shifted brackets arrive as braces on some layouts: both are bound.
        KeyBinding::new(secondary!("shift-]"), BringToFront, keys),
        KeyBinding::new(secondary!("}"), BringToFront, keys),
        KeyBinding::new(secondary!("]"), BringForward, keys),
        KeyBinding::new(secondary!("["), SendBackward, keys),
        KeyBinding::new(secondary!("shift-["), SendToBack, keys),
        KeyBinding::new(secondary!("{"), SendToBack, keys),
        KeyBinding::new(secondary!("b"), ToggleBold, Some(TEXT_BOX)),
        KeyBinding::new(secondary!("i"), ToggleItalic, Some(TEXT_BOX)),
        KeyBinding::new(secondary!("u"), ToggleUnderline, Some(TEXT_BOX)),
    ]);
    cx.on_action(|_: &SelectTool, cx| set_tool(Tool::Select, cx));
    cx.on_action(|_: &HandTool, cx| set_tool(Tool::Hand, cx));
    cx.on_action(|_: &TextTool, cx| set_tool(Tool::Text, cx));
    cx.on_action(|_: &NoteTool, cx| set_tool(Tool::Note, cx));
    cx.on_action(|_: &HighlightTool, cx| set_tool(Tool::Highlight, cx));
    cx.on_action(|_: &InkTool, cx| set_tool(Tool::Ink, cx));
    cx.on_action(|_: &RectangleTool, cx| set_tool(Tool::Rectangle, cx));
    cx.on_action(|_: &EllipseTool, cx| set_tool(Tool::Ellipse, cx));
    cx.on_action(|_: &LineTool, cx| set_tool(Tool::Line, cx));
    cx.on_action(|_: &ArrowTool, cx| set_tool(Tool::Arrow, cx));
    cx.on_action(|_: &UnderlineTool, cx| set_tool(Tool::Underline, cx));
    cx.on_action(|_: &StrikeOutTool, cx| set_tool(Tool::StrikeOut, cx));
    cx.on_action(|_: &StampTool, cx| set_tool(Tool::Stamp, cx));
    cx.on_action(|_: &RedactTool, cx| set_tool(Tool::Redact, cx));
    cx.on_action(|_: &PictureTool, cx| set_tool(Tool::Picture, cx));
    cx.on_action(|_: &EraserTool, cx| set_tool(Tool::Eraser, cx));
    cx.on_action(|_: &SignTool, cx| set_tool(Tool::Sign, cx));
    cx.on_action(|_: &FieldTool, cx| set_tool(Tool::Field, cx));
    cx.on_action(|_: &DeleteSelection, cx| with_active(cx, canvas::delete_selected));
    cx.on_action(|_: &Deselect, cx| with_active(cx, canvas::reset));
    let reorder = |to: ZOrder| {
        move |tab: &mut DocTab, cx: &mut Context<DocTab>| canvas::reorder_selected(tab, to, cx)
    };
    cx.on_action(move |_: &BringToFront, cx| with_active(cx, reorder(ZOrder::Front)));
    cx.on_action(move |_: &BringForward, cx| with_active(cx, reorder(ZOrder::Forward)));
    cx.on_action(move |_: &SendBackward, cx| with_active(cx, reorder(ZOrder::Backward)));
    cx.on_action(move |_: &SendToBack, cx| with_active(cx, reorder(ZOrder::Back)));
}

/// The tool bar row under the main tool bar: unused, the quick bar floats over the viewer.
pub fn toolbar(_window: &mut Window, _cx: &mut App) -> Option<AnyElement> {
    None
}

/// Interactive layer over one page. `rect` is the page in content space (logical pixels).
pub fn page_overlay(
    tab: &mut DocTab,
    page: usize,
    rect: PageRect,
    window: &mut Window,
    cx: &mut Context<DocTab>,
) -> Option<AnyElement> {
    canvas::page_overlay(tab, page, rect, window, cx)
}

/// The floating quick bar over the viewer's top-left corner, with its style card.
pub fn viewer_overlay(
    tab: &mut DocTab,
    window: &mut Window,
    cx: &mut Context<DocTab>,
) -> Option<AnyElement> {
    quickbar::render(tab, window, cx)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bracket shortcuts parse (a bad one panics at startup).
    #[test]
    fn drawing_order_keys_parse() {
        KeyBinding::new(secondary!("shift-]"), BringToFront, None);
        KeyBinding::new(secondary!("}"), BringToFront, None);
        KeyBinding::new(secondary!("]"), BringForward, None);
        KeyBinding::new(secondary!("["), SendBackward, None);
        KeyBinding::new(secondary!("{"), SendToBack, None);
    }

    /// Every tool is on the bar, alone or through the group button of its group.
    #[test]
    fn every_tool_is_reachable_from_the_bar() {
        let reachable = |tool: Tool| {
            Tool::BAR.contains(&tool)
                || tool
                    .group()
                    .is_some_and(|group| Tool::BAR.contains(&group[0]))
        };
        let tools = [Tool::SHAPES.as_slice(), &Tool::MARKUP, &Tool::BAR].concat();
        assert!(tools.into_iter().all(reachable));
        // Chosen from the signature and form panels.
        assert!(!reachable(Tool::Sign) && !reachable(Tool::Field));
        assert_eq!(Tool::Underline.group(), Some(&Tool::MARKUP[..]));
        assert_eq!(Tool::Select.group(), None);
    }

    #[test]
    fn toggling_formats_over_all_characters() {
        let mut styles = Vec::new();
        toggle_all(&mut styles, 3, 0..3, Format::Bold);
        assert!(styles.len() == 3 && styles.iter().all(|style| style.bold));
        assert!(common_style(&styles, 0..3).bold);
        toggle_all(&mut styles, 3, 0..3, Format::Bold);
        assert!(styles.is_empty());
        // Partly bold: the toggle makes all of it bold.
        let bold = CharStyle {
            bold: true,
            ..CharStyle::default()
        };
        let mut mixed = vec![bold, CharStyle::default()];
        assert!(!common_style(&mixed, 0..2).bold);
        toggle_all(&mut mixed, 2, 0..2, Format::Bold);
        assert!(common_style(&mixed, 0..2).bold);
        assert_eq!(common_style(&[], 0..0), CharStyle::default());
        // Only the selected characters: "H2O" with the 2 lowered, then raised instead.
        let mut chemistry = Vec::new();
        toggle_all(&mut chemistry, 3, 1..2, Format::Sub);
        assert_eq!(chemistry[1].script, Script::Sub);
        assert_eq!(common_style(&chemistry, 0..1), CharStyle::default());
        toggle_all(&mut chemistry, 3, 1..2, Format::Super);
        assert_eq!(common_style(&chemistry, 1..2).script, Script::Super);
        toggle_all(&mut chemistry, 3, 1..2, Format::Super);
        assert!(chemistry.is_empty());
        // Superscript replaces subscript; turning superscript off then leaves it normal.
        let mut style = CharStyle::default();
        Format::Sub.set(&mut style, true);
        Format::Super.set(&mut style, true);
        assert!(Format::Super.is_on(style) && !Format::Sub.is_on(style));
        Format::Sub.set(&mut style, false);
        assert!(Format::Super.is_on(style));
        Format::Super.set(&mut style, false);
        assert_eq!(style, CharStyle::default());
    }
}
