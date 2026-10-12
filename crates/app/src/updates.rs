use std::sync::{Arc, Mutex};

use egui::Context;

pub const RELEASES_PAGE: &str = "https://github.com/0XLUC4/GodPDF/releases";
#[cfg(not(target_arch = "wasm32"))]
const RELEASES_API: &str = "https://api.github.com/repos/0XLUC4/GodPDF/releases";

#[derive(Clone, Debug)]
pub struct Release {
    pub tag: String,
    pub name: String,
    pub date: String,
    pub notes: String,
    pub url: String,
    pub asset: Option<String>,
}

pub const ASSET_NAME: &str = "GodPDF-windows-x64.exe";
pub const CAN_INSTALL: bool = cfg!(windows);

#[derive(Clone)]
pub enum Install {
    Idle,
    Downloading,
    Ready(std::path::PathBuf),
    Failed(String),
}

impl Release {
    pub fn is_newer(&self) -> bool {
        is_newer(&self.tag, env!("CARGO_PKG_VERSION"))
    }
}

#[derive(Clone)]
pub enum State {
    Idle,
    Loading,
    Ready(Vec<Release>),
    Failed(String),
}

#[derive(Clone)]
pub struct Checker(Arc<Mutex<State>>, Arc<Mutex<Install>>);

impl Default for Checker {
    fn default() -> Self {
        Self(
            Arc::new(Mutex::new(State::Idle)),
            Arc::new(Mutex::new(Install::Idle)),
        )
    }
}

impl Checker {
    pub fn state(&self) -> State {
        self.0.lock().map_or(State::Idle, |state| state.clone())
    }

    pub fn install_state(&self) -> Install {
        self.1.lock().map_or(Install::Idle, |state| state.clone())
    }

    pub fn take_installed(&self) -> Option<std::path::PathBuf> {
        let mut state = self.1.lock().ok()?;
        match std::mem::replace(&mut *state, Install::Idle) {
            Install::Ready(path) => Some(path),
            other => {
                *state = other;
                None
            }
        }
    }

    pub fn install(&self, ctx: &Context, url: String) {
        if let Ok(mut state) = self.1.lock() {
            if matches!(*state, Install::Downloading | Install::Ready(_)) {
                return;
            }
            *state = Install::Downloading;
        }
        let shared = Arc::clone(&self.1);
        let ctx = ctx.clone();
        let work = move || {
            let result = match replace_program(&url) {
                Ok(path) => Install::Ready(path),
                Err(err) => Install::Failed(err),
            };
            if let Ok(mut state) = shared.lock() {
                *state = result;
            }
            ctx.request_repaint();
        };
        #[cfg(not(target_arch = "wasm32"))]
        if std::thread::Builder::new()
            .name("install".to_owned())
            .spawn(work)
            .is_err()
            && let Ok(mut state) = self.1.lock()
        {
            *state = Install::Failed("Cannot start the download".to_owned());
        }
        #[cfg(target_arch = "wasm32")]
        work();
    }

    pub fn newest(&self) -> Option<Release> {
        match self.state() {
            State::Ready(releases) => releases.into_iter().find(Release::is_newer),
            _ => None,
        }
    }

    pub fn check(&self, ctx: &Context) {
        if let Ok(mut state) = self.0.lock() {
            if matches!(*state, State::Loading) {
                return;
            }
            *state = State::Loading;
        }
        let shared = Arc::clone(&self.0);
        let ctx = ctx.clone();
        let work = move || {
            let result = match fetch() {
                Ok(releases) => State::Ready(releases),
                Err(err) => State::Failed(err),
            };
            if let Ok(mut state) = shared.lock() {
                *state = result;
            }
            ctx.request_repaint();
        };
        #[cfg(not(target_arch = "wasm32"))]
        if std::thread::Builder::new()
            .name("updates".to_owned())
            .spawn(work)
            .is_err()
            && let Ok(mut state) = self.0.lock()
        {
            *state = State::Failed("Cannot start the update check".to_owned());
        }
        #[cfg(target_arch = "wasm32")]
        work();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn fetch() -> Result<Vec<Release>, String> {
    let body = ureq::get(RELEASES_API)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", concat!("GodPDF/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|err| format!("Cannot reach GitHub: {err}"))?
        .body_mut()
        .read_to_string()
        .map_err(|err| format!("Cannot read the releases: {err}"))?;
    parse(&body)
}

#[cfg(target_arch = "wasm32")]
fn fetch() -> Result<Vec<Release>, String> {
    Err("Updates are listed on the releases page".to_owned())
}

#[cfg(not(target_arch = "wasm32"))]
fn old_program(exe: &std::path::Path) -> std::path::PathBuf {
    exe.with_extension("old.exe")
}

#[cfg(not(target_arch = "wasm32"))]
pub fn remove_old_program() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(old_program(&exe));
    }
}

#[cfg(target_arch = "wasm32")]
pub fn remove_old_program() {}

#[cfg(not(target_arch = "wasm32"))]
fn replace_program(url: &str) -> Result<std::path::PathBuf, String> {
    let exe = std::env::current_exe().map_err(|err| format!("Cannot find GodPDF: {err}"))?;
    let new = exe.with_extension("new.exe");
    let old = old_program(&exe);
    let mut response = ureq::get(url)
        .header("User-Agent", concat!("GodPDF/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|err| format!("Cannot download the update: {err}"))?;
    let mut file =
        std::fs::File::create(&new).map_err(|err| format!("Cannot write the update: {err}"))?;
    let written = std::io::copy(&mut response.body_mut().as_reader(), &mut file)
        .map_err(|err| format!("Download interrupted: {err}"));
    drop(file);
    match written {
        Ok(bytes) if bytes > 0 => {}
        Ok(_) => {
            let _ = std::fs::remove_file(&new);
            return Err("The download is empty".to_owned());
        }
        Err(err) => {
            let _ = std::fs::remove_file(&new);
            return Err(err);
        }
    }
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).map_err(|err| {
        let _ = std::fs::remove_file(&new);
        format!("Cannot replace GodPDF: {err}")
    })?;
    if let Err(err) = std::fs::rename(&new, &exe) {
        let _ = std::fs::rename(&old, &exe);
        let _ = std::fs::remove_file(&new);
        return Err(format!("Cannot replace GodPDF: {err}"));
    }
    Ok(exe)
}

#[cfg(target_arch = "wasm32")]
fn replace_program(_url: &str) -> Result<std::path::PathBuf, String> {
    Err("Download the update from the releases page".to_owned())
}

#[cfg(not(target_arch = "wasm32"))]
fn parse(body: &str) -> Result<Vec<Release>, String> {
    let list: Vec<serde_json::Value> = serde_json::from_str(body)
        .map_err(|err| format!("Unexpected answer from GitHub: {err}"))?;
    let text = |value: &serde_json::Value, key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned()
    };
    let mut releases: Vec<Release> = list
        .iter()
        .filter(|value| {
            !value
                .get("draft")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
        })
        .map(|value| {
            let tag = text(value, "tag_name");
            let name = text(value, "name");
            let assets = value
                .get("assets")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default();
            let asset = assets
                .iter()
                .find(|asset| text(asset, "name") == ASSET_NAME)
                .or_else(|| {
                    assets
                        .iter()
                        .find(|asset| text(asset, "name").to_ascii_lowercase().ends_with(".exe"))
                })
                .map(|asset| text(asset, "browser_download_url"))
                .filter(|url| !url.is_empty());
            Release {
                asset,
                name: if name.is_empty() { tag.clone() } else { name },
                date: text(value, "published_at").chars().take(10).collect(),
                notes: text(value, "body").replace("\r\n", "\n"),
                url: text(value, "html_url"),
                tag,
            }
        })
        .collect();
    releases.sort_by(|a, b| version(&b.tag).cmp(&version(&a.tag)));
    Ok(releases)
}

fn version(tag: &str) -> Vec<u64> {
    tag.trim_start_matches(|ch: char| !ch.is_ascii_digit())
        .split(|ch: char| !ch.is_ascii_digit())
        .take(3)
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

fn is_newer(tag: &str, current: &str) -> bool {
    let tag = version(tag);
    !tag.is_empty() && tag > version(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(is_newer("v1.1.0", "1.0.0"));
        assert!(is_newer("1.0.10", "1.0.9"));
        assert!(is_newer("v2", "1.9.9"));
        assert!(!is_newer("v1.0.0", "1.0.0"));
        assert!(!is_newer("v0.9.0", "1.0.0"));
        assert!(!is_newer("nightly", "1.0.0"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn parses_releases_newest_first() {
        let body = r#"[
            {"tag_name": "v1.0.0", "name": "", "published_at": "2026-09-01T10:00:00Z",
             "body": "First", "html_url": "https://example.com/1", "draft": false},
            {"tag_name": "v1.2.0", "name": "Big one", "published_at": "2026-10-01T10:00:00Z",
             "body": "Second\r\nline", "html_url": "https://example.com/2", "draft": false,
             "assets": [{"name": "notes.txt", "browser_download_url": "https://example.com/n"},
                        {"name": "GodPDF-windows-x64.exe", "browser_download_url": "https://example.com/exe"}]},
            {"tag_name": "v9.0.0", "name": "Draft", "draft": true}
        ]"#;
        let releases = parse(body).unwrap();
        assert_eq!(releases.len(), 2);
        assert_eq!(releases[0].name, "Big one");
        assert_eq!(releases[0].date, "2026-10-01");
        assert_eq!(releases[0].notes, "Second\nline");
        assert_eq!(
            releases[0].asset.as_deref(),
            Some("https://example.com/exe")
        );
        assert_eq!(releases[1].name, "v1.0.0");
        assert_eq!(releases[1].asset, None);
    }
}
