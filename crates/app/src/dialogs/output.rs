//! Work that writes other files: pages as pictures, split parts (ported from
//! the egui `src/pagetools.rs` and `platform.rs`). The heavy part runs in the background;
//! the folder is asked for once it is done.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use godpdf_core as core;
use gpui::App;

use super::kinds::open_tab;
use crate::doc::guarded;
use crate::files::write_atomic;
use crate::toasts;

/// Render `pages` at `dpi` as PNG or JPEG, then ask for a folder to write them to.
pub fn export_images(
    pdf: Vec<u8>,
    pages: Vec<usize>,
    dpi: u32,
    jpeg: bool,
    base: String,
    cx: &mut App,
) {
    toasts::info(cx, "Drawing the pages…");
    let rendered = crate::services::jobs::background(cx, async move {
        render_images(pdf, &pages, dpi, jpeg, &base)
    });
    cx.spawn(async move |cx| {
        let result = rendered.await;
        cx.update(|cx| {
            match result.and_then(|files| save_files(&files).map(|f| f.map(|f| (files.len(), f)))) {
                Ok(Some((count, folder))) => {
                    toasts::success(cx, format!("Saved {count} pictures to {folder}"));
                }
                Ok(None) => {}
                Err(err) => toasts::error(cx, err),
            }
        });
    })
    .detach();
}

/// Split every `n` pages, or at `starts`; save the parts to a folder or open them as tabs.
pub fn split(
    pdf: Vec<u8>,
    every: bool,
    n: usize,
    starts: Vec<usize>,
    base: String,
    to_disk: bool,
    cx: &mut App,
) {
    let parts = crate::services::jobs::background(cx, async move {
        guarded(|| {
            if every {
                core::split_every(&pdf, n)
            } else {
                core::split_at(&pdf, &starts)
            }
        })
        .map_err(|err| format!("Split failed: {err}"))
    });
    cx.spawn(async move |cx| {
        let parts = parts.await;
        cx.update(|cx| match parts {
            Ok(parts) if to_disk => {
                let files: Vec<(String, Vec<u8>)> = parts
                    .into_iter()
                    .enumerate()
                    .map(|(index, bytes)| (part_name(&base, index + 1), bytes))
                    .collect();
                match save_files(&files) {
                    Ok(Some(folder)) => {
                        toasts::success(cx, format!("Saved {} files to {folder}", files.len()));
                    }
                    Ok(None) => {}
                    Err(err) => toasts::error(cx, err),
                }
            }
            Ok(parts) => {
                let count = parts.len();
                for (index, bytes) in parts.into_iter().enumerate() {
                    open_tab(part_name(&base, index + 1), bytes, cx);
                }
                toasts::success(cx, format!("Opened {count} parts"));
            }
            Err(err) => toasts::error(cx, err),
        });
    })
    .detach();
}

/// `pages` rendered at `dpi`, each as a PNG or JPEG named after `base` and its number.
fn render_images(
    pdf: Vec<u8>,
    pages: &[usize],
    dpi: u32,
    jpeg: bool,
    base: &str,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let doc = godpdf_render::Document::open(Arc::new(pdf), None).map_err(|err| err.to_string())?;
    let digits = doc.page_count().to_string().len();
    let extension = if jpeg { "jpg" } else { "png" };
    pages
        .iter()
        .map(|&page| {
            let bitmap = doc
                .render(page, dpi as f32 / 72.0)
                .map_err(|err| err.to_string())?;
            let image = image::RgbaImage::from_raw(bitmap.width, bitmap.height, bitmap.rgba)
                .ok_or("The page could not be drawn")?;
            let mut bytes = std::io::Cursor::new(Vec::new());
            let written = if jpeg {
                image::DynamicImage::ImageRgba8(image)
                    .to_rgb8()
                    .write_to(&mut bytes, image::ImageFormat::Jpeg)
            } else {
                image.write_to(&mut bytes, image::ImageFormat::Png)
            };
            written.map_err(|err| format!("Cannot write page {}: {err}", page + 1))?;
            let name = format!("{base} - page {:0digits$}.{extension}", page + 1);
            Ok((name, bytes.into_inner()))
        })
        .collect()
}

/// Ask for a folder and write each `(name, bytes)` there, never over an existing file.
/// Returns the folder, `None` if cancelled.
fn save_files(files: &[(String, Vec<u8>)]) -> Result<Option<String>, String> {
    let Some(folder) = rfd::FileDialog::new().pick_folder() else {
        return Ok(None);
    };
    for (name, bytes) in files {
        write_atomic(&free_path(&folder, name), bytes)?;
    }
    Ok(Some(folder.display().to_string()))
}

/// `report.pdf` + 2 -> `report_part2.pdf`.
fn part_name(base: &str, index: usize) -> String {
    format!("{}_part{index}.pdf", super::kinds::stem(base))
}

/// `folder/name`, or `name (2)`, `name (3)`... when taken.
fn free_path(folder: &Path, name: &str) -> PathBuf {
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, format!(".{extension}")),
        _ => (name, String::new()),
    };
    let mut path = folder.join(name);
    let mut copy = 2;
    while path.exists() {
        path = folder.join(format!("{stem} ({copy}){extension}"));
        copy += 1;
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_are_numbered() {
        assert_eq!(part_name("report.pdf", 2), "report_part2.pdf");
        assert_eq!(part_name("scan", 1), "scan_part1.pdf");
    }

    #[test]
    fn taken_names_get_a_copy_number() {
        let dir = std::env::temp_dir().join(format!("godpdf-free-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(free_path(&dir, "a.png"), dir.join("a.png"));
        std::fs::write(dir.join("a.png"), b"x").unwrap();
        assert_eq!(free_path(&dir, "a.png"), dir.join("a (2).png"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
