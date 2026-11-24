//! Export pages as images and split documents in the background.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use godpdf_core as core;
use gpui::App;

use super::kinds::open_tab;
use crate::doc::guarded;
use crate::files::write_atomic;
use crate::toasts;

/// Ask where to save the images, then render and write them one page at a time.
pub fn export_images(
    pdf: Vec<u8>,
    pages: Vec<usize>,
    dpi: u32,
    jpeg: bool,
    base: String,
    cx: &mut App,
) {
    crate::files::pick_folder(cx, move |folder, cx| {
        let count = pages.len();
        let exported = crate::services::jobs::background(cx, move || {
            render_images(pdf, &pages, dpi, jpeg, &base, &folder)
        });
        cx.spawn(async move |cx| {
            let result = exported.await;
            cx.update(|cx| match result {
                Ok(()) => toasts::note(
                    cx,
                    format!(
                        "Exported {count} {}",
                        if count == 1 { "page" } else { "pages" }
                    ),
                ),
                Err(err) => toasts::error(cx, err),
            });
        })
        .detach();
    });
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
    let parts = crate::services::jobs::background(cx, move || {
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
                save_files(files, cx);
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

/// Render and save each page before allocating the bitmap for the next one.
fn render_images(
    pdf: Vec<u8>,
    pages: &[usize],
    dpi: u32,
    jpeg: bool,
    base: &str,
    folder: &Path,
) -> Result<(), String> {
    let doc = godpdf_render::Document::open(Arc::new(pdf), None).map_err(|err| err.to_string())?;
    let digits = doc.page_count().to_string().len();
    let extension = if jpeg { "jpg" } else { "png" };
    for (written, &page) in pages.iter().enumerate() {
        let result = (|| {
            let bitmap = doc
                .render(page, dpi as f32 / 72.0)
                .map_err(|err| err.to_string())?;
            let image = image::RgbaImage::from_raw(bitmap.width, bitmap.height, bitmap.rgba)
                .ok_or("The page could not be drawn")?;
            let mut bytes = std::io::Cursor::new(Vec::new());
            let encoded = if jpeg {
                image::DynamicImage::ImageRgba8(image)
                    .to_rgb8()
                    .write_to(&mut bytes, image::ImageFormat::Jpeg)
            } else {
                image.write_to(&mut bytes, image::ImageFormat::Png)
            };
            encoded.map_err(|err| err.to_string())?;
            let name = format!("{base} - page {:0digits$}.{extension}", page + 1);
            write_unique(folder, &name, bytes.get_ref())
        })();
        result.map_err(|err| {
            format!(
                "Cannot export page {} ({written} already saved): {err}",
                page + 1
            )
        })?;
    }
    Ok(())
}

/// Ask for a folder and write each `(name, bytes)` there in the background, never over an
/// existing file.
fn save_files(files: Vec<(String, Vec<u8>)>, cx: &mut App) {
    crate::files::pick_folder(cx, move |folder, cx| {
        let written = crate::services::jobs::background(cx, move || {
            files
                .iter()
                .try_for_each(|(name, bytes)| write_unique(&folder, name, bytes))
        });
        cx.spawn(async move |cx| {
            if let Err(err) = written.await {
                cx.update(|cx| toasts::error(cx, err));
            }
        })
        .detach();
    });
}

/// `report.pdf` + 2 -> `report_part2.pdf`.
fn part_name(base: &str, index: usize) -> String {
    format!("{}_part{index}.pdf", super::kinds::stem(base))
}

fn write_unique(folder: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    // Keep name selection and writing together when two exports use the same folder.
    static WRITING: Mutex<()> = Mutex::new(());
    let _writing = WRITING.lock().unwrap_or_else(|err| err.into_inner());
    write_atomic(&free_path(folder, name), bytes)
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

    #[test]
    fn a_failed_later_page_leaves_the_first_image_and_existing_file_intact() {
        let dir = std::env::temp_dir().join(format!("godpdf-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let original = dir.join("scan - page 1.png");
        std::fs::write(&original, b"existing").unwrap();
        let pdf = core::blank_document(12.0, 12.0).unwrap();

        let err = render_images(pdf, &[0, 1], 72, false, "scan", &dir).unwrap_err();

        assert!(err.contains("page 2 (1 already saved)"), "{err}");
        assert_eq!(std::fs::read(original).unwrap(), b"existing");
        let exported = image::open(dir.join("scan - page 1 (2).png")).unwrap();
        assert_eq!((exported.width(), exported.height()), (12, 12));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
