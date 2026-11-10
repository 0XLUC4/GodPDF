//! Canvas tools (ported from `crates/app/src/quickbar.rs`, `selection.rs`, `textbox.rs`):
//! the active [`Tool`], per-tool styles, the floating quick bar over the viewer and the
//! per-page interactions (select / move / resize / delete, draw, erase, pan, type, pictures).
//!
//! App-wide state is the [`ToolState`] global (tool, styles, picture card fields, the tab
//! shown). Per-document state is [`Canvas`], a field of `DocTab` (selection, gesture, text
//! editor, annotation cache), so it goes away with its tab.

mod canvas;
mod editor;
mod geom;
mod quickbar;

use gpui::assets::IconName;
use gpui::{
    Action, AnyElement, App, Context, Global, Hsla, KeyBinding, Rgba, WeakEntity, Window, actions,
};

use godpdf_core::{CharStyle, Color, Font};

use crate::doc::DocTab;
use crate::geometry::PageRect;

pub use canvas::Canvas;

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
        PictureTool,
        EraserTool,
        DeleteSelection,
        Deselect,
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
    Picture,
    /// Click or drag over comments to remove them.
    Eraser,
}

impl Tool {
    pub const ALL: [Tool; 9] = [
        Tool::Select,
        Tool::Hand,
        Tool::Text,
        Tool::Note,
        Tool::Highlight,
        Tool::Ink,
        Tool::Rectangle,
        Tool::Picture,
        Tool::Eraser,
    ];

    fn icon(self) -> IconName {
        match self {
            Self::Select => IconName::MousePointer,
            Self::Hand => IconName::Hand,
            Self::Text => IconName::Type,
            Self::Note => IconName::StickyNote,
            Self::Highlight => IconName::Highlighter,
            Self::Ink => IconName::Pencil,
            Self::Rectangle => IconName::Square,
            Self::Picture => IconName::Image,
            Self::Eraser => IconName::Eraser,
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
            Self::Picture => "Picture",
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
            Self::Picture => Some("I"),
            Self::Eraser => Some("E"),
            Self::Highlight => None,
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
            Self::Picture => Box::new(PictureTool),
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
            Self::Hand => "Drag to move around",
            Self::Eraser => "Click or drag over comments to erase them",
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
    pub rectangle: Style,
}

impl Default for Styles {
    fn default() -> Self {
        let style = |color| Style {
            color,
            width: 2.0,
            font_size: 14.0,
            font: Font::Helvetica,
        };
        Self {
            text: style(SWATCHES[0].0),
            highlight: style(SWATCHES[4].0),
            ink: style(SWATCHES[0].0),
            rectangle: style(SWATCHES[0].0),
        }
    }
}

impl Styles {
    /// Style of the comments `tool` makes; tools without styles give the text style.
    pub fn of_mut(&mut self, tool: Tool) -> &mut Style {
        match tool {
            Tool::Highlight => &mut self.highlight,
            Tool::Ink => &mut self.ink,
            Tool::Rectangle => &mut self.rectangle,
            _ => &mut self.text,
        }
    }

    pub fn of(&self, tool: Tool) -> Style {
        match tool {
            Tool::Highlight => self.highlight,
            Tool::Ink => self.ink,
            Tool::Rectangle => self.rectangle,
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
}

impl Format {
    pub const ALL: [Format; 3] = [Format::Bold, Format::Italic, Format::Underline];

    pub fn is_on(self, style: CharStyle) -> bool {
        match self {
            Self::Bold => style.bold,
            Self::Italic => style.italic,
            Self::Underline => style.underline,
        }
    }

    pub fn set(self, style: &mut CharStyle, on: bool) {
        match self {
            Self::Bold => style.bold = on,
            Self::Italic => style.italic = on,
            Self::Underline => style.underline = on,
        }
    }
}

/// Flip `format` over the `len` characters of `styles`: on unless all have it already.
/// An all-default result is the empty list, as core writes it.
pub fn toggle_all(styles: &mut Vec<CharStyle>, len: usize, format: Format) {
    styles.resize(len, CharStyle::default());
    let on = !styles.iter().all(|style| format.is_on(*style));
    for style in styles.iter_mut() {
        format.set(style, on);
    }
    if styles.iter().all(|style| *style == CharStyle::default()) {
        styles.clear();
    }
}

/// Formats every one of the `len` characters has.
pub fn common_style(styles: &[CharStyle], len: usize) -> CharStyle {
    let mut common = CharStyle::default();
    for format in Format::ALL {
        let on =
            len > 0 && (0..len).all(|index| format.is_on(godpdf_core::style_at(styles, index)));
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
    /// The document the viewer shows, set while it renders: key actions go to it.
    active: Option<WeakEntity<DocTab>>,
    picture_card: Option<quickbar::PictureCard>,
}

impl Default for ToolState {
    fn default() -> Self {
        Self {
            tool: Tool::Select,
            styles: Styles::default(),
            keep_ratio: true,
            active: None,
            picture_card: None,
        }
    }
}

impl Global for ToolState {}

fn tool_state(cx: &mut App) -> &mut ToolState {
    cx.default_global::<ToolState>()
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
    tool_state(cx).tool = tool;
    cx.refresh_windows();
}

/// Register tool actions and their key bindings (same keys as the egui app).
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
        KeyBinding::new("i", PictureTool, keys),
        KeyBinding::new("e", EraserTool, keys),
        KeyBinding::new("delete", DeleteSelection, keys),
        KeyBinding::new("backspace", DeleteSelection, keys),
        KeyBinding::new("escape", Deselect, keys),
        KeyBinding::new("secondary-b", ToggleBold, Some(TEXT_BOX)),
        KeyBinding::new("secondary-i", ToggleItalic, Some(TEXT_BOX)),
        KeyBinding::new("secondary-u", ToggleUnderline, Some(TEXT_BOX)),
    ]);
    cx.on_action(|_: &SelectTool, cx| set_tool(Tool::Select, cx));
    cx.on_action(|_: &HandTool, cx| set_tool(Tool::Hand, cx));
    cx.on_action(|_: &TextTool, cx| set_tool(Tool::Text, cx));
    cx.on_action(|_: &NoteTool, cx| set_tool(Tool::Note, cx));
    cx.on_action(|_: &HighlightTool, cx| set_tool(Tool::Highlight, cx));
    cx.on_action(|_: &InkTool, cx| set_tool(Tool::Ink, cx));
    cx.on_action(|_: &RectangleTool, cx| set_tool(Tool::Rectangle, cx));
    cx.on_action(|_: &PictureTool, cx| set_tool(Tool::Picture, cx));
    cx.on_action(|_: &EraserTool, cx| set_tool(Tool::Eraser, cx));
    cx.on_action(|_: &DeleteSelection, cx| with_active(cx, canvas::delete_selected));
    cx.on_action(|_: &Deselect, cx| with_active(cx, canvas::reset));
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

    #[test]
    fn toggling_formats_over_all_characters() {
        let mut styles = Vec::new();
        toggle_all(&mut styles, 3, Format::Bold);
        assert!(styles.len() == 3 && styles.iter().all(|style| style.bold));
        assert!(common_style(&styles, 3).bold);
        toggle_all(&mut styles, 3, Format::Bold);
        assert!(styles.is_empty());
        // Partly bold: the toggle makes all of it bold.
        let bold = CharStyle {
            bold: true,
            ..CharStyle::default()
        };
        let mut mixed = vec![bold, CharStyle::default()];
        assert!(!common_style(&mixed, 2).bold);
        toggle_all(&mut mixed, 2, Format::Bold);
        assert!(common_style(&mixed, 2).bold);
        assert_eq!(common_style(&[], 0), CharStyle::default());
    }
}
