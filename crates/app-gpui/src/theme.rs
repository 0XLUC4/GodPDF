//! Monochrome palette of the egui app (`crates/app/src/theme.rs`): black on white, white on black.

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
    /// Fill of selected / pressed controls.
    pub selected: Hsla,
    /// Viewer canvas behind pages.
    pub canvas: Hsla,
    pub error: Hsla,
}

fn grey(level: u32) -> Hsla {
    let color: Rgba = rgb(level << 16 | level << 8 | level);
    color.into()
}

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
            selected: grey(0x33),
            canvas: grey(0x0A),
            error: grey(0xF2),
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
            selected: grey(0xE6),
            canvas: grey(0xF2),
            error: grey(0x11),
        }
    }

    pub fn of(cx: &App) -> Self {
        if cx.try_global::<Theme>().is_some_and(|theme| theme.dark) {
            Self::dark()
        } else {
            Self::light()
        }
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
    cx.refresh_windows();
}

pub fn toggle(cx: &mut App) {
    let dark = cx.try_global::<Theme>().is_some_and(|theme| theme.dark);
    set_dark(!dark, cx);
}
