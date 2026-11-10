//! Settings kept between runs (egui: eframe storage), as `key=value` lines in
//! `%APPDATA%/GodPDF/gpui-settings.txt` (`~/.config/GodPDF/` elsewhere). Failing to read or
//! write them is never an error: the app then starts with its defaults.

use std::path::PathBuf;

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("GodPDF").join("gpui-settings.txt"))
}

fn parse(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
        .collect()
}

/// `entries` with `key` set to `value` (one line per key).
fn with(mut entries: Vec<(String, String)>, key: &str, value: &str) -> String {
    entries.retain(|(known, _)| known != key);
    entries.push((key.to_owned(), value.to_owned()));
    entries
        .iter()
        .map(|(key, value)| format!("{key}={value}\n"))
        .collect()
}

fn read() -> Vec<(String, String)> {
    path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| parse(&text))
        .unwrap_or_default()
}

pub fn get(key: &str) -> Option<String> {
    read()
        .into_iter()
        .find(|(known, _)| known == key)
        .map(|(_, value)| value)
}

/// Remember `value` for `key`; best effort.
pub fn set(key: &str, value: &str) {
    let Some(path) = path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = crate::files::write_atomic(&path, with(read(), key, value).as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_replaced_and_kept() {
        let text = with(
            parse("dark_mode=0\nother = x\nnot a line\n"),
            "dark_mode",
            "1",
        );
        assert_eq!(text, "other=x\ndark_mode=1\n");
        assert_eq!(parse(&text)[1], ("dark_mode".to_owned(), "1".to_owned()));
    }
}
