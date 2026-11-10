//! Monochrome palette of the egui app (`crates/app/src/theme.rs`): black on white, white on
//! black, and the egui visuals it sets (`configure`) as constants. See DESIGN.md.

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
    /// Fill of selected / pressed controls.
    pub selected: Hsla,
    /// Viewer canvas behind pages.
    pub canvas: Hsla,
    pub success: Hsla,
    pub error: Hsla,
    /// Shadow of windows (dialogs) and popups: black at this alpha (egui 110 / 28 of 255).
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

// ---- egui visuals (`theme::configure`), same for both palettes.

/// `widgets.*.corner_radius`: buttons, fields, menus entries.
pub const RADIUS: f32 = 7.0;
/// `window_corner_radius`: dialogs.
pub const WINDOW_RADIUS: f32 = 12.0;
/// `menu_corner_radius`: popups and menus.
pub const MENU_RADIUS: f32 = 9.0;
/// `spacing.item_spacing` (x, y).
pub const ITEM_SPACING: (f32, f32) = (8.0, 6.0);
/// `spacing.button_padding` (x, y).
pub const BUTTON_PADDING: (f32, f32) = (10.0, 5.0);
/// `spacing.menu_margin` / `window_margin`.
pub const MENU_MARGIN: f32 = 8.0;
pub const WINDOW_MARGIN: f32 = 16.0;
/// `spacing.interact_size.y`: height of a standard button or field.
pub const INTERACT_HEIGHT: f32 = 26.0;

// ---- tool colours (`crates/app/src/commands.rs`), one per family.

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
            border: grey(0x2E),
            text: grey(0xF2),
            text_dim: grey(0x9A),
            accent: grey(0xF2),
            on_accent: grey(0x11),
            accent_text: grey(0xF2),
            selected: grey(0x33),
            canvas: grey(0x0A),
            success: grey(0xF2),
            error: grey(0xF2),
            shadow_alpha: 110.0 / 255.0,
        }
    }

    /// Default: white UI, light grey canvas so white pages stand out.
    pub fn light() -> Self {
        Self {
            bg: grey(0xFF),
            panel: grey(0xFF),
            surface: grey(0xF5),
            hover: grey(0xEF),
            border: grey(0xE4),
            text: grey(0x11),
            text_dim: grey(0x66),
            accent: grey(0x11),
            on_accent: grey(0xFF),
            accent_text: grey(0x11),
            selected: grey(0xE6),
            canvas: grey(0xF2),
            success: grey(0x11),
            error: grey(0x11),
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

    /// egui `gamma_multiply(0.5)` of `text_dim`: disabled text.
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
/// palette, with egui's widget radius, so they look like the egui ones.
fn apply_to_components(cx: &mut App) {
    let p = Palette::of(cx);
    gpui::component::Theme::update(cx, |theme| {
        let c = &mut theme.colors;
        (c.background, c.foreground, c.border, c.input) = (p.panel, p.text, p.border, p.border);
        (c.popover, c.popover_foreground) = (p.panel, p.text);
        (c.accent, c.accent_foreground) = (p.hover, p.text);
        (c.muted, c.muted_foreground) = (p.surface, p.text_dim);
        (c.ring, c.caret, c.selection) = (p.accent_text, p.text, p.selected);
        (c.list_hover, c.list_active) = (p.hover, p.selected);
        (c.button, c.button_hover, c.button_active) = (p.surface, p.hover, p.selected);
        (c.secondary, c.secondary_hover, c.secondary_active) = (p.surface, p.hover, p.selected);
        (c.button_foreground, c.secondary_foreground) = (p.text, p.text);
        (c.primary, c.primary_hover, c.primary_active) = (p.accent, p.accent, p.accent);
        (c.button_primary, c.button_primary_hover) = (p.accent, p.accent);
        c.button_primary_active = p.accent;
        (c.primary_foreground, c.button_primary_foreground) = (p.on_accent, p.on_accent);
        theme.radius = gpui::px(RADIUS);
        theme.radius_lg = gpui::px(MENU_RADIUS);
    });
}

pub fn toggle(cx: &mut App) {
    set_dark(!Palette::is_dark(cx), cx);
}
