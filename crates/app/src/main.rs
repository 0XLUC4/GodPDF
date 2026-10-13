//! GodPDF: desktop (and later web) entry point.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod branding;
mod commands;
mod comments;
mod dialogs;
mod doc;
mod geometry;
mod jobs;
mod lineedit;
mod organize;
mod platform;
#[cfg(windows)]
mod print;
mod quickbar;
mod ranges;
mod selection;
mod sidebar;
mod textbox;
mod textsel;
mod theme;
mod toasts;
mod tools;
mod updates;
mod viewer;
mod welcome;
mod widgets;

pub const APP_NAME: &str = "GodPDF";

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(APP_NAME)
        .with_app_id("godpdf")
        .with_inner_size([1320.0, 860.0])
        .with_min_inner_size([760.0, 480.0])
        .with_drag_and_drop(true);
    if let Some(icon) = branding::icon_data() {
        viewport = viewport.with_icon(icon);
    }
    let files = std::env::args_os().skip(1).map(Into::into).collect();
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(|cc| Ok(Box::new(app::GodApp::new(cc, files)))),
    )
}

/// Web entry: expects a `<canvas id="godpdf_canvas">` in the host page.
#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast as _;

    wasm_bindgen_futures::spawn_local(async {
        let canvas = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.get_element_by_id("godpdf_canvas"))
            .and_then(|element| element.dyn_into::<web_sys::HtmlCanvasElement>().ok());
        let Some(canvas) = canvas else {
            return;
        };
        // Nothing useful to do with a start error in a headless page; the canvas stays blank.
        let _ = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| Ok(Box::new(app::GodApp::new(cc, Vec::new())))),
            )
            .await;
    });
}
