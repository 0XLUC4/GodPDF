//! Files that are not PDFs but become one when opened: pictures and plain text.

use std::path::Path;

use godpdf_core as core;

use crate::doc::guarded;
use crate::platform::PickedFile;

/// `file` as a PDF, and whether it had to be made into one. A file made into a PDF has no
/// path: saving it asks where, and never writes over the picture or the text it came from.
pub fn to_pdf(file: PickedFile) -> Result<(PickedFile, bool), String> {
    let extension = Path::new(&file.name)
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase());
    let made = if file.bytes.starts_with(&[0xFF, 0xD8]) {
        guarded(|| core::jpeg_document(&file.bytes)).map_err(|err| err.to_string())
    } else if file.bytes.starts_with(b"\x89PNG") {
        png_document(&file.bytes)
    } else if extension.as_deref() == Some("txt") {
        guarded(|| core::text_document(&String::from_utf8_lossy(&file.bytes)))
            .map_err(|err| err.to_string())
    } else {
        return Ok((file, false));
    };
    let bytes = made.map_err(|err| format!("Cannot make a PDF of “{}”: {err}", file.name))?;
    let name = Path::new(&file.name)
        .with_extension("pdf")
        .to_string_lossy()
        .into_owned();
    Ok((
        PickedFile {
            name,
            path: None,
            bytes,
        },
        true,
    ))
}

/// A PNG or JPEG file as a picture to put on a page. A JPEG is kept as it is.
pub fn picture(bytes: &[u8]) -> Result<core::Picture, String> {
    if bytes.starts_with(&[0xFF, 0xD8])
        && let Ok(picture) = guarded(|| core::Picture::jpeg(bytes))
    {
        return Ok(picture);
    }
    let image = image::load_from_memory(bytes)
        .map_err(|err| err.to_string())?
        .into_rgba8();
    let (width, height) = image.dimensions();
    guarded(|| core::Picture::rgba(width, height, image.as_raw())).map_err(|err| err.to_string())
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

    fn file(name: &str, bytes: &[u8]) -> PickedFile {
        PickedFile {
            name: name.to_owned(),
            path: Some(name.into()),
            bytes: bytes.to_vec(),
        }
    }

    #[test]
    fn a_pdf_is_left_as_it_is() {
        let (out, made) = to_pdf(file("a.pdf", b"%PDF-1.7")).unwrap();
        assert!(!made && out.path.is_some() && out.bytes == b"%PDF-1.7");
    }

    #[test]
    fn text_becomes_a_pdf_to_save_elsewhere() {
        let (out, made) = to_pdf(file("notes.txt", b"hello")).unwrap();
        assert!(made && out.path.is_none());
        assert_eq!(out.name, "notes.pdf");
        assert_eq!(core::page_count(&out.bytes).unwrap(), 1);
    }

    #[test]
    fn a_png_becomes_a_page_and_its_holes_are_white() {
        let mut png = Vec::new();
        let pixels = image::RgbaImage::from_pixel(4, 2, image::Rgba([0, 0, 0, 0]));
        image::DynamicImage::ImageRgba8(pixels)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let (out, made) = to_pdf(file("shot.png", &png)).unwrap();
        assert!(made);
        assert_eq!(core::page_count(&out.bytes).unwrap(), 1);
        assert!(to_pdf(file("bad.png", b"\x89PNG nothing")).is_err());
    }
}
