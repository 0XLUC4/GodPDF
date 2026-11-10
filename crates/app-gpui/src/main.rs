//! GodPDF desktop app on GPUI. All PDF work is in `godpdf-core` / `godpdf-render`;
//! this crate is UI only. See ARCHITECTURE.md.

// No console window behind the app in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

/// Ctrl on Windows/Linux, Cmd on macOS, as a key binding prefix:
/// `KeyBinding::new` cannot resolve gpui's `secondary-` and panics on it.
macro_rules! secondary {
    ($keys:literal) => {
        if cfg!(target_os = "macos") {
            concat!("cmd-", $keys)
        } else {
            concat!("ctrl-", $keys)
        }
    };
}

mod app;
mod dialogs;
// Edit, PDF-space and palette API not used yet: it is for the tools and panels to come.
#[allow(dead_code)]
mod doc;
mod files;
mod fonts;
#[allow(dead_code)]
mod geometry;
mod icons;
mod import;
mod organize;
mod panels;
mod shell;
mod signature;
mod text;
#[allow(dead_code)]
mod theme;
mod toasts;
mod tools;
mod viewer;
mod widgets;

use gpui::{App, AppContext, Bounds, WindowBounds, WindowOptions, px, size};

pub const APP_NAME: &str = "GodPDF";

fn main() {
    // Files passed on the command line ("Open with") open at start.
    let paths: Vec<std::path::PathBuf> = std::env::args_os().skip(1).map(Into::into).collect();
    gpui::application()
        .with_assets(gpui::assets::Assets)
        .run(move |cx: &mut App| {
            gpui::init(cx);
            fonts::load(cx);
            theme::set_dark(false, cx);
            app::register(cx);
            let bounds = Bounds::centered(None, size(px(1280.), px(860.)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(640.), px(420.))),
                ..Default::default()
            };
            let opened = gpui::open_window(options, cx, |window, cx| {
                let root = cx.new(|cx| app::GodPdfApp::new(paths, window, cx));
                let weak = root.downgrade();
                window.on_window_should_close(cx, move |window, cx| {
                    weak.update(cx, |app, cx| app.confirm_quit(window, cx))
                        .unwrap_or(true)
                });
                root
            });
            match opened {
                Ok(_) => cx.activate(true),
                Err(err) => {
                    eprintln!("{APP_NAME}: cannot open the window: {err}");
                    cx.quit();
                }
            }
            // Closing the last window ends the app.
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
        });
}

#[cfg(test)]
mod tests {
    #[test]
    fn secondary_bindings_parse() {
        for keys in [
            secondary!("o"),
            secondary!("shift-s"),
            secondary!("="),
            secondary!("-"),
        ] {
            gpui::Keystroke::parse(keys).unwrap();
        }
    }
}
