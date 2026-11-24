//! Palette of the app: white (or dark grey) surfaces, a blue accent, and the shared visuals
//! as constants. See DESIGN.md.

use gpui::{App, Global, Hsla, Rgba, rgb};

/// Which palette is in use. Read it with [`Palette::of`]; flip it with [`toggle`].
#[derive(Clone, Copy, Default)]
pub struct Theme {
    pub dark: bool,
}

impl Global for Theme {}

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Hsla,
    pub panel: Hsla,
    pub surface: Hsla,
    pub hover: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub text_dim: Hsla,
    /// Emphasis color: strokes, primary buttons, current page.
    pub accent: Hsla,
    /// Text drawn on top of `accent`.
    pub on_accent: Hsla,
    /// Accent usable as text/link color on this background.
    pub accent_text: Hsla,
    /// Hovered primary button.
    pub accent_hover: Hsla,
    /// Fill of a secondary button, and hovered.
    pub button: Hsla,
    pub button_hover: Hsla,
    /// Outline of secondary buttons (white pills with a thin line).
    pub outline: Hsla,
    /// Fill of selected / pressed controls.
    pub selected: Hsla,
    /// Viewer canvas behind pages.
    pub canvas: Hsla,
    pub success: Hsla,
    pub error: Hsla,
    /// Shadow of windows (dialogs) and popups: black at this alpha (110 / 28 of 255).
    pub shadow_alpha: f32,
}

fn grey(level: u32) -> Hsla {
    let color: Rgba = rgb(level << 16 | level << 8 | level);
    color.into()
}

/// `0xRRGGBB` as a color.
pub fn hex(color: u32) -> Hsla {
    rgb(color).into()
}

// ---- visuals, same for both palettes.

/// Corner radius of fields, menu entries, icon buttons.
pub const RADIUS: f32 = 6.0;
/// Text buttons are pills: half their height.
pub const PILL_RADIUS: f32 = INTERACT_HEIGHT / 2.0;
/// `window_corner_radius`: dialogs.
pub const WINDOW_RADIUS: f32 = 12.0;
/// Popups and menus.
pub const MENU_RADIUS: f32 = 8.0;
/// `spacing.item_spacing` (x, y).
pub const ITEM_SPACING: (f32, f32) = (8.0, 6.0);
/// Button padding (x, y).
pub const BUTTON_PADDING: (f32, f32) = (12.0, 6.0);
/// `spacing.menu_margin` / `window_margin`.
pub const MENU_MARGIN: f32 = 8.0;
pub const WINDOW_MARGIN: f32 = 16.0;
/// Height of a standard button or field.
pub const INTERACT_HEIGHT: f32 = 28.0;

// ---- tool colours, one per family.

pub const GREEN: u32 = 0x3FB950;
pub const PURPLE: u32 = 0xA371F7;
pub const AMBER: u32 = 0xE3A008;
pub const BLUE: u32 = 0x4C8DF6;
pub const TEAL: u32 = 0x1FB5A5;
pub const PINK: u32 = 0xE85AAD;
pub const RED: u32 = 0xF05252;
/// Update dot on the tab row (`dialogs::DOWNLOAD_BLUE`).
pub const DOWNLOAD_BLUE: u32 = 0x2563EB;

impl Palette {
    pub fn dark() -> Self {
        Self {
            bg: grey(0x0E),
            panel: grey(0x14),
            surface: grey(0x1C),
            hover: grey(0x26),
            border: grey(0x24),
            text: grey(0xF2),
            text_dim: grey(0x9A),
            accent: hex(0x3B82F6),
            on_accent: grey(0xFF),
            accent_text: hex(0x60A5FA),
            accent_hover: hex(0x2563EB),
            button: grey(0x14),
            button_hover: grey(0x22),
            outline: grey(0x52),
            selected: hex(0x1E2A44),
            canvas: grey(0x0A),
            success: grey(0xF2),
            error: hex(0xF87171),
            shadow_alpha: 110.0 / 255.0,
        }
    }

    /// Default: white UI, blue accent, light grey canvas so white pages stand out.
    pub fn light() -> Self {
        Self {
            bg: grey(0xFF),
            panel: grey(0xFF),
            surface: hex(0xF4F4F5),
            hover: hex(0xF1F1F3),
            border: hex(0xE4E4E7),
            text: hex(0x18181B),
            text_dim: hex(0x71717A),
            accent: hex(0x2563EB),
            on_accent: grey(0xFF),
            accent_text: hex(0x2563EB),
            accent_hover: hex(0x1D4ED8),
            button: grey(0xFF),
            button_hover: hex(0xF4F4F5),
            outline: hex(0x6B6B73),
            selected: hex(0xE8EFFD),
            canvas: hex(0xF0F0F2),
            success: grey(0x11),
            error: hex(0xD92D20),
            shadow_alpha: 28.0 / 255.0,
        }
    }

    pub fn is_dark(cx: &App) -> bool {
        cx.try_global::<Theme>().is_some_and(|theme| theme.dark)
    }

    pub fn of(cx: &App) -> Self {
        if Self::is_dark(cx) {
            Self::dark()
        } else {
            Self::light()
        }
    }

    /// `text_dim` at half opacity: disabled text.
    pub fn disabled(&self) -> Hsla {
        self.text_dim.opacity(0.5)
    }
}

/// Switch dark <-> light, for our palette and for gpui-component widgets.
pub fn set_dark(dark: bool, cx: &mut App) {
    cx.set_global(Theme { dark });
    let mode = if dark {
        gpui::component::ThemeMode::Dark
    } else {
        gpui::component::ThemeMode::Light
    };
    gpui::component::Theme::change(mode, None, cx);
    crate::fonts::apply_to_components(cx);
    apply_to_components(cx);
    cx.refresh_windows();
}

/// Paint gpui-component widgets (menus, tooltips, fields, checkboxes, buttons) in this
/// palette, with the app's widget radius, so they match the app's own widgets.
fn apply_to_components(cx: &mut App) {
    let p = Palette::of(cx);
    gpui::component::Theme::update(cx, |theme| {
        let c = &mut theme.colors;
        (c.background, c.foreground, c.border) = (p.panel, p.text, p.border);
        // Fields: white with a visible line, like the buttons.
        c.input = p.outline.opacity(0.45);
        (c.popover, c.popover_foreground) = (p.panel, p.text);
        (c.accent, c.accent_foreground) = (p.hover, p.text);
        (c.muted, c.muted_foreground) = (p.surface, p.text_dim);
        (c.ring, c.caret, c.selection) = (p.accent_text, p.text, p.selected);
        (c.list_hover, c.list_active) = (p.hover, p.selected);
        (c.button, c.button_hover, c.button_active) = (p.button, p.button_hover, p.button_hover);
        (c.secondary, c.secondary_hover, c.secondary_active) =
            (p.button, p.button_hover, p.selected);
        (c.button_secondary, c.button_secondary_hover) = (p.button, p.button_hover);
        c.button_secondary_active = p.selected;
        (c.button_foreground, c.secondary_foreground) = (p.text, p.text);
        c.button_secondary_foreground = p.text;
        (c.primary, c.primary_hover, c.primary_active) = (p.accent, p.accent_hover, p.accent_hover);
        (c.button_primary, c.button_primary_hover) = (p.accent, p.accent_hover);
        c.button_primary_active = p.accent_hover;
        (c.scrollbar, c.scrollbar_thumb) = (gpui::transparent_black(), p.text_dim.opacity(0.35));
        c.scrollbar_thumb_hover = p.text_dim.opacity(0.6);
        (c.primary_foreground, c.button_primary_foreground) = (p.on_accent, p.on_accent);
        theme.radius = gpui::px(RADIUS);
        theme.radius_lg = gpui::px(MENU_RADIUS);
    });
}

pub fn toggle(cx: &mut App) {
    set_dark(!Palette::is_dark(cx), cx);
}
