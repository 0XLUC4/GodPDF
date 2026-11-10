//! New versions on GitHub: the releases list, the
//! blue dot and popover of the tab row, and "Update and restart", which swaps the program
//! file and restarts on the new one.

use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui::{App, Global};

pub const RELEASES_PAGE: &str = "https://github.com/0XLUC4/GodPDF/releases";
const RELEASES_API: &str = "https://api.github.com/repos/0XLUC4/GodPDF/releases";
pub const ASSET_NAME: &str = "GodPDF-windows-x64.exe";
pub const CAN_INSTALL: bool = cfg!(windows);

#[derive(Clone, Debug)]
pub struct Release {
    pub tag: String,
    pub notes: String,
    pub url: String,
    pub asset: Option<String>,
}

impl Release {
    pub fn is_newer(&self) -> bool {
        is_newer(&self.tag, env!("CARGO_PKG_VERSION"))
    }

    /// The tag without its leading "v": "1.2.0".
    pub fn version(&self) -> &str {
        self.tag.trim_start_matches(['v', 'V'])
    }
}

/// The bullet lines (`- ` or `* `) of release notes, without their marker and the
/// `**bold**` / `code` marks.
pub fn bullets(notes: &str) -> Vec<String> {
    notes
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            line.strip_prefix("- ").or_else(|| line.strip_prefix("* "))
        })
        .map(|item| item.replace("**", "").replace('`', "").trim().to_owned())
        .filter(|item| !item.is_empty())
        .collect()
}

#[derive(Clone, Default)]
pub enum State {
    #[default]
    Idle,
    Loading,
    Ready(Vec<Release>),
    Failed(String),
}

#[derive(Clone, Default)]
pub enum Install {
    #[default]
    Idle,
    Downloading,
}

#[derive(Default)]
pub struct Updates {
    pub state: State,
    pub install: Install,
    /// Installed: start this program when the app quits.
    pub restart: Option<PathBuf>,
    /// The popover under the Updates button is open.
    pub open: bool,
}

impl Global for Updates {}

pub fn get(cx: &App) -> Option<&Updates> {
    cx.try_global::<Updates>()
}

/// The newest release above this version, if any (the blue dot).
pub fn newest(cx: &App) -> Option<Release> {
    match &get(cx)?.state {
        State::Ready(releases) => releases.iter().find(|release| release.is_newer()).cloned(),
        _ => None,
    }
}

/// Look for releases in the background (no-op while already looking).
pub fn check(cx: &mut App) {
    let updates = cx.default_global::<Updates>();
    if matches!(updates.state, State::Loading) {
        return;
    }
    updates.state = State::Loading;
    let fetched = cx.background_executor().spawn(async { fetch() });
    cx.spawn(async move |cx| {
        let result = fetched.await;
        cx.update(|cx| {
            cx.default_global::<Updates>().state = match result {
                Ok(releases) => State::Ready(releases),
                // The detail goes to the log; the 256 px card says what to do.
                Err(err) => {
                    eprintln!("{}: update check failed: {err}", crate::APP_NAME);
                    State::Failed("Check your connection and try again.".to_owned())
                }
            };
            cx.refresh_windows();
        });
    })
    .detach();
}

/// Download `url` over the program, then quit (asking about unsaved work) to restart on it.
/// A failure shows in the error bar. Already installed (the quit was cancelled): quit again.
pub fn install(url: String, cx: &mut App) {
    let updates = cx.default_global::<Updates>();
    if updates.restart.is_some() {
        quit(cx);
        return;
    }
    if matches!(updates.install, Install::Downloading) {
        return;
    }
    updates.install = Install::Downloading;
    let replaced = cx
        .background_executor()
        .spawn(async move { replace_program(&url) });
    cx.spawn(async move |cx| {
        let result = replaced.await;
        cx.update(|cx| {
            let updates = cx.default_global::<Updates>();
            updates.install = Install::Idle;
            match result {
                Ok(program) => {
                    updates.restart = Some(program);
                    updates.open = false;
                    quit(cx);
                }
                Err(err) => crate::toasts::error(cx, err),
            }
            cx.refresh_windows();
        });
    })
    .detach();
}

/// Quit through the Quit action, so unsaved documents are asked about first.
fn quit(cx: &mut App) {
    if let Some(window) = cx.active_window().or_else(|| cx.windows().first().copied()) {
        let _ = window.update(cx, |_, window, cx| {
            window.dispatch_action(Box::new(crate::app::Quit), cx)
        });
    }
}

/// The app is quitting: start the new program when an update was installed.
pub fn restart_if_installed(cx: &mut App) {
    if let Some(program) = get(cx).and_then(|updates| updates.restart.clone())
        && let Err(err) = std::process::Command::new(&program).spawn()
    {
        eprintln!("{}: updated, start it again ({err})", crate::APP_NAME);
    }
}

fn user_agent() -> &'static str {
    concat!("GodPDF/", env!("CARGO_PKG_VERSION"))
}

/// An HTTP client that gives up: 15 s to connect, `total` for the whole call (a captive
/// portal or a stalled link must not leave "Checking…" spinning forever).
fn agent(total: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_global(Some(total))
        .build()
        .into()
}

fn fetch() -> Result<Vec<Release>, String> {
    let body = agent(Duration::from_secs(20))
        .get(RELEASES_API)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", user_agent())
        .call()
        .map_err(|err| format!("Cannot reach GitHub: {err}"))?
        .body_mut()
        .read_to_string()
        .map_err(|err| format!("Cannot read the releases: {err}"))?;
    parse(&body)
}

fn old_program(exe: &Path) -> PathBuf {
    exe.with_extension("old.exe")
}

/// The program replaced by the last update, left beside it: removed at start.
pub fn remove_old_program() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(old_program(&exe));
    }
}

/// Download beside the program, then rename: the running file becomes `.old.exe`.
fn replace_program(url: &str) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|err| format!("Cannot find GodPDF: {err}"))?;
    let new = exe.with_extension("new.exe");
    let old = old_program(&exe);
    let mut response = agent(Duration::from_secs(10 * 60))
        .get(url)
        .header("User-Agent", user_agent())
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

fn parse(body: &str) -> Result<Vec<Release>, String> {
    use serde_json::Value;
    let list: Vec<Value> = serde_json::from_str(body)
        .map_err(|err| format!("Unexpected answer from GitHub: {err}"))?;
    let text = |value: &Value, key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned()
    };
    let mut releases: Vec<Release> = list
        .iter()
        .filter(|value| !value.get("draft").and_then(Value::as_bool).unwrap_or(false))
        .map(|value| {
            let tag = text(value, "tag_name");
            let assets = value
                .get("assets")
                .and_then(Value::as_array)
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
                notes: text(value, "body").replace("\r\n", "\n"),
                url: text(value, "html_url"),
                tag,
            }
        })
        .collect();
    releases.sort_by_key(|release| std::cmp::Reverse(version(&release.tag)));
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

    #[test]
    fn release_notes_bullets() {
        let notes = "## What's new
- Faster **thumbnails**
  * `Ctrl+K` palette
plain line
-not a bullet
- 
";
        assert_eq!(bullets(notes), vec!["Faster thumbnails", "Ctrl+K palette"]);
        assert!(bullets("").is_empty());
    }

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
        assert_eq!(releases[0].tag, "v1.2.0");
        assert_eq!(releases[0].notes, "Second\nline");
        assert_eq!(
            releases[0].asset.as_deref(),
            Some("https://example.com/exe")
        );
        assert_eq!(releases[1].tag, "v1.0.0");
        assert_eq!(releases[1].asset, None);
    }
}
