//! Settings kept between sessions: theme, right panel, tools panel, recent files, window
//! size. JSON in `settings.json`, in the app data folder (`%APPDATA%\GodPDF\data` on Windows).
//!
//! The root view notes what it shows every frame ([`track`]); it is written when it changed,
//! every few seconds and when the app quits ([`save`]), so a crash loses little. The write is
//! atomic: a crash mid-write never truncates the file.

use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::{Value, json};

/// What is remembered.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub dark: bool,
    /// Right panel open.
    pub sidebar: bool,
    pub tools_panel: bool,
    /// Newest first.
    pub recent: Vec<PathBuf>,
    /// Size when not maximized, in logical pixels; `None` when never seen so.
    pub size: Option<(f32, f32)>,
    pub maximized: bool,
    /// Custom colours of the style card (`0xRRGGBB`), newest first.
    pub recent_colors: Vec<u32>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            dark: false,
            sidebar: true,
            tools_panel: true,
            recent: Vec::new(),
            size: None,
            maximized: false,
            recent_colors: Vec::new(),
        }
    }
}

/// Smallest window brought back (the window's minimum size).
pub const MIN_SIZE: (f32, f32) = (640.0, 420.0);

static LAST: Mutex<Option<Settings>> = Mutex::new(None);
/// What is in the file: no write while nothing changed.
static SAVED: Mutex<Option<Settings>> = Mutex::new(None);

/// Note what the app shows now. Cheap: call every frame.
pub fn track(seen: Settings) {
    if let Ok(mut last) = LAST.lock()
        && last.as_ref() != Some(&seen)
    {
        *last = Some(seen);
    }
}

/// Write down what [`track`] saw last, if it changed. Not worth an error message: defaults
/// come back. Blocking (disk): call it off the UI thread, or at quit.
pub fn save() {
    let Some(seen) = LAST.lock().ok().and_then(|last| last.clone()) else {
        return;
    };
    // Held during the write: a periodic save and the quit save never interleave.
    let Ok(mut saved) = SAVED.lock() else {
        return;
    };
    if saved.as_ref() == Some(&seen) {
        return;
    }
    if let Some(path) = file()
        && crate::files::write_atomic(&path, to_json(&seen).as_bytes()).is_ok()
    {
        *saved = Some(seen);
    }
}

/// What was written down, or the defaults.
pub fn load() -> Settings {
    file()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| parse(&text))
        .unwrap_or_default()
}

fn to_json(settings: &Settings) -> String {
    let recent: Vec<String> = settings
        .recent
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    let value = json!({
        "dark_mode": settings.dark,
        "sidebar": settings.sidebar,
        "tools_panel": settings.tools_panel,
        "recent": recent,
        "recent_colors": settings
            .recent_colors
            .iter()
            .map(|&rgb| format!("#{rgb:06X}"))
            .collect::<Vec<_>>(),
        "window": {
            "width": settings.size.map(|size| size.0),
            "height": settings.size.map(|size| size.1),
            "maximized": settings.maximized,
        },
    });
    serde_json::to_string_pretty(&value).unwrap_or_default()
}

/// Anything missing or broken falls back to its default; a size too small or absurd is
/// never brought back.
fn parse(text: &str) -> Settings {
    let defaults = Settings::default();
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return defaults;
    };
    let flag =
        |key: &str, default: bool| value.get(key).and_then(Value::as_bool).unwrap_or(default);
    let recent = value
        .get("recent")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
                .take(crate::shell::MAX_RECENT)
                .collect()
        })
        .unwrap_or_default();
    let recent_colors = value
        .get("recent_colors")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .filter_map(crate::tools::parse_hex)
                .collect()
        })
        .unwrap_or_default();
    let window = value.get("window");
    let number = |key: &str| {
        window
            .and_then(|window| window.get(key))
            .and_then(Value::as_f64)
            .map(|number| number as f32)
    };
    let usable = |value: f32, least: f32| value.is_finite() && (least..=20_000.0).contains(&value);
    let size = match (number("width"), number("height")) {
        (Some(w), Some(h)) if usable(w, MIN_SIZE.0) && usable(h, MIN_SIZE.1) => Some((w, h)),
        _ => None,
    };
    Settings {
        dark: flag("dark_mode", defaults.dark),
        sidebar: flag("sidebar", defaults.sidebar),
        tools_panel: flag("tools_panel", defaults.tools_panel),
        recent,
        size,
        maximized: window
            .and_then(|window| window.get("maximized"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        recent_colors,
    }
}

/// eframe `storage_dir(APP_NAME)`, created when missing.
fn dir() -> Option<PathBuf> {
    let env = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let dir = if cfg!(windows) {
        env("APPDATA")?.join(crate::APP_NAME).join("data")
    } else if cfg!(target_os = "macos") {
        env("HOME")?
            .join("Library")
            .join("Application Support")
            .join(crate::APP_NAME)
    } else {
        env("XDG_DATA_HOME")
            .filter(|path| path.is_absolute())
            .or_else(|| env("HOME").map(|home| home.join(".local").join("share")))?
            .join(crate::APP_NAME.to_lowercase())
    };
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn file() -> Option<PathBuf> {
    Some(dir()?.join("settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip() {
        let settings = Settings {
            dark: true,
            sidebar: false,
            tools_panel: false,
            recent: vec![PathBuf::from("C:/a.pdf"), PathBuf::from("/b c.pdf")],
            size: Some((1200.0, 800.0)),
            maximized: true,
            recent_colors: vec![0x1F5FD6, 0x000000],
        };
        assert_eq!(parse(&to_json(&settings)), settings);
    }

    #[test]
    fn broken_or_tiny_values_fall_back() {
        assert_eq!(parse(""), Settings::default());
        assert_eq!(parse("not json"), Settings::default());
        let tiny = parse(r#"{"window": {"width": 64, "height": 64, "maximized": true}}"#);
        assert_eq!(tiny.size, None);
        assert!(tiny.maximized);
        let missing = parse(r#"{"dark_mode": true, "window": {"width": 1000}}"#);
        assert!(missing.dark && missing.sidebar && missing.tools_panel);
        assert_eq!(missing.size, None);
        let colors = parse(r##"{"recent_colors": ["#FF0000", "nope", 3, "0f0"]}"##);
        assert_eq!(colors.recent_colors, [0xFF0000, 0x00FF00]);
    }
}
