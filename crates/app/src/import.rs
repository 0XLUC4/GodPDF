//! Files that are not PDFs but become one when opened: pictures and plain text (ported from
//! the egui `src/import.rs`).

use std::path::Path;

use godpdf_core as core;

use crate::doc::guarded;
use crate::files::{self, PickedFile};
use crate::toasts;

/// `name`'s `bytes` as a PDF: its name, its bytes and whether it had to be made into one.
/// A PDF (or any other file) is left as it is.
pub fn to_pdf(name: &str, bytes: Vec<u8>) -> Result<(String, Vec<u8>, bool), String> {
    let extension = Path::new(name)
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase());
    let made = if bytes.starts_with(&[0xFF, 0xD8]) {
        guarded(|| core::jpeg_document(&bytes)).map_err(|err| err.to_string())
    } else if bytes.starts_with(b"\x89PNG") {
        png_document(&bytes)
    } else if extension.as_deref() == Some("txt") {
        guarded(|| core::text_document(&String::from_utf8_lossy(&bytes)))
            .map_err(|err| err.to_string())
    } else {
        return Ok((name.to_owned(), bytes, false));
    };
    let made = made.map_err(|err| format!("Cannot make a PDF of “{name}”: {err}"))?;
    let pdf_name = Path::new(name)
        .with_extension("pdf")
        .to_string_lossy()
        .into_owned();
    Ok((pdf_name, made, true))
}

/// Ask for PDFs, pictures or text files and read them, each made into a PDF. What cannot be
/// read or made into one is said in a toast and left out.
pub fn pick_as_pdfs(cx: &mut gpui::App) -> Vec<PickedFile> {
    files::pick_pdfs()
        .iter()
        .filter_map(|path| {
            let file = files::read_file(path)
                .and_then(|file| to_pdf(&file.name, file.bytes).map(|made| (file.path, made)));
            match file {
                Ok((path, (name, bytes, _))) => Some(PickedFile { name, path, bytes }),
                Err(err) => {
                    toasts::error(cx, err);
                    None
                }
            }
        })
        .collect()
}

/// A PNG on a page: what is see-through shows the white of the paper.
fn png_document(png: &[u8]) -> Result<Vec<u8>, String> {
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .map_err(|err| err.to_string())?
        .into_rgba8();
    let (width, height) = image.dimensions();
    let rgb: Vec<u8> = image
        .pixels()
        .flat_map(|pixel| {
            let [r, g, b, a] = pixel.0.map(u16::from);
            [r, g, b].map(|channel| ((channel * a + 255 * (255 - a)) / 255) as u8)
        })
        .collect();
    guarded(|| core::rgb_document(width, height, &rgb)).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pdf_is_left_as_it_is() {
        let (name, bytes, made) = to_pdf("a.pdf", b"%PDF-1.7".to_vec()).unwrap();
        assert!(!made && name == "a.pdf" && bytes == b"%PDF-1.7");
    }

    #[test]
    fn text_becomes_a_pdf() {
        let (name, bytes, made) = to_pdf("notes.txt", b"hello".to_vec()).unwrap();
        assert!(made);
        assert_eq!(name, "notes.pdf");
        assert_eq!(core::page_count(&bytes).unwrap(), 1);
    }

    #[test]
    fn a_png_becomes_a_page_and_a_broken_one_an_error() {
        let mut png = Vec::new();
        let pixels = image::RgbaImage::from_pixel(4, 2, image::Rgba([0, 0, 0, 0]));
        image::DynamicImage::ImageRgba8(pixels)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let (_, bytes, made) = to_pdf("shot.png", png).unwrap();
        assert!(made);
        assert_eq!(core::page_count(&bytes).unwrap(), 1);
        assert!(to_pdf("bad.png", b"\x89PNG nothing".to_vec()).is_err());
    }
}
