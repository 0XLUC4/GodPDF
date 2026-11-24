//! The bundled fonts: Inter 4.1 Regular and SemiBold (all UI text, OFL), Hack (monospace)
//! from `epaint_default_fonts` 0.35, and the Phosphor icon font (light weight) 2.1.

use std::borrow::Cow;

use gpui::{App, px};

/// UI text: Inter, Regular for text and SemiBold ([`STRONG`]) for titles and names.
pub const UI: &str = "Inter";
/// The weight of [`UI`] text: set it on the root.
pub const UI_WEIGHT: gpui::FontWeight = gpui::FontWeight::NORMAL;
/// Titles, author names, the active tab.
pub const STRONG: gpui::FontWeight = gpui::FontWeight::SEMIBOLD;
/// Monospace text.
pub const MONO: &str = "Hack";
/// Phosphor glyphs, light weight (thin outlines): see `icons.rs`.
pub const ICONS: &str = "Phosphor";

/// The size of text without an explicit size (body and buttons).
pub const BODY: f32 = 13.0;

pub const FILES: [&[u8]; 4] = [
    include_bytes!("../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../assets/fonts/Hack-Regular.ttf"),
    include_bytes!("../assets/fonts/Phosphor-Light.ttf"),
];

/// Load the fonts into the text system. Call once, before the theme is set.
pub fn load(cx: &mut App) {
    let fonts = FILES.iter().map(|bytes| Cow::Borrowed(*bytes)).collect();
    if let Err(err) = cx.text_system().add_fonts(fonts) {
        eprintln!("{}: cannot load the bundled fonts: {err}", crate::APP_NAME);
    }
}

/// Make gpui-component widgets use the app fonts and sizes. Called after every theme change
/// (`Theme::change` reloads the theme file, fonts included).
pub fn apply_to_components(cx: &mut App) {
    gpui::component::Theme::update(cx, |theme| {
        theme.font_family = UI.into();
        theme.font_size = px(BODY);
        theme.mono_font_family = MONO.into();
        theme.mono_font_size = px(BODY);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The family names above match the bundled files, and the icon glyphs are in Phosphor.
    #[test]
    fn bundled_fonts_resolve() {
        // Not headless: a headless platform has no real text system. No window is opened.
        let platform = gpui::platform::current_platform(false);
        let text = platform.text_system();
        text.add_fonts(FILES.iter().map(|bytes| Cow::Borrowed(*bytes)).collect())
            .unwrap();
        let names = text.all_font_names();
        for family in [UI, MONO, ICONS] {
            assert!(
                names.iter().any(|name| name == family),
                "{family} not in {names:?}"
            );
            text.font_id(&gpui::font(family)).unwrap();
        }
        for weight in [UI_WEIGHT, STRONG] {
            let ui = gpui::Font {
                weight,
                ..gpui::font(UI)
            };
            text.font_id(&ui).unwrap();
        }
        let icons = text.font_id(&gpui::font(ICONS)).unwrap();
        for glyph in [crate::icons::HOUSE, crate::icons::X, crate::icons::FILE_PDF] {
            let ch = glyph.chars().next().unwrap();
            assert!(text.glyph_for_char(icons, ch).is_some(), "{ch:?}");
        }
    }
}
