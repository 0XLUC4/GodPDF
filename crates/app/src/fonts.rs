//! The fonts of the egui app, bundled: Ubuntu Light (all UI text), Hack (monospace) from
//! `epaint_default_fonts` 0.35, and the Phosphor icon font (regular) from `egui-phosphor` 0.13.

use std::borrow::Cow;

use gpui::{App, px};

/// UI text (egui `FontFamily::Proportional`): the typographic family of Ubuntu-Light.ttf.
/// Its only face is Light (300), so every weight resolves to it, bold included, as in egui
/// (egui has no bold either: `.strong()` only changes the colour).
pub const UI: &str = "Ubuntu";
/// The weight of [`UI`]: set it on the root so the match is exact.
pub const UI_WEIGHT: gpui::FontWeight = gpui::FontWeight::LIGHT;
/// egui `FontFamily::Monospace`.
pub const MONO: &str = "Hack";
/// Phosphor glyphs: see `icons.rs`.
pub const ICONS: &str = "Phosphor";

/// egui `TextStyle::Body` / `Button`: the size of text without an explicit size.
pub const BODY: f32 = 13.0;

pub const FILES: [&[u8]; 3] = [
    include_bytes!("../assets/fonts/Ubuntu-Light.ttf"),
    include_bytes!("../assets/fonts/Hack-Regular.ttf"),
    include_bytes!("../assets/fonts/Phosphor.ttf"),
];

/// Load the fonts into the text system. Call once, before the theme is set.
pub fn load(cx: &mut App) {
    let fonts = FILES.iter().map(|bytes| Cow::Borrowed(*bytes)).collect();
    if let Err(err) = cx.text_system().add_fonts(fonts) {
        eprintln!("{}: cannot load the bundled fonts: {err}", crate::APP_NAME);
    }
}

/// Make gpui-component widgets use the egui fonts and sizes. Called after every theme change
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
        let ui = gpui::Font {
            weight: UI_WEIGHT,
            ..gpui::font(UI)
        };
        text.font_id(&ui).unwrap();
        let icons = text.font_id(&gpui::font(ICONS)).unwrap();
        for glyph in [crate::icons::HOUSE, crate::icons::X, crate::icons::FILE_PDF] {
            let ch = glyph.chars().next().unwrap();
            assert!(text.glyph_for_char(icons, ch).is_some(), "{ch:?}");
        }
    }
}
