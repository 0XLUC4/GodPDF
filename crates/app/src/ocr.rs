//! Text recognition of the pages. On Windows the recognizer of the system reads every
//! language installed there; elsewhere a model (12 MB) is downloaded on first use.
//!
//! Every page is read; words the page already holds as text are left out, so pages that
//! mix a scan with some typed text get the words of the scan only.

use std::sync::Arc;

use godpdf_core::Word;
use godpdf_render::TextChar;

/// Pages are read at 300 dots per inch, less when the picture would be too big.
const SCALE: f32 = 300.0 / 72.0;
/// Longest side of a page picture given to the recognizer.
const MAX_SIDE: f32 = 9_000.0;

/// Nothing has to be downloaded before reading.
pub fn ready() -> bool {
    engine::ready()
}

/// The words of every page that the page does not hold as text yet, page by page.
pub fn recognize(pdf: Vec<u8>) -> Result<Vec<(usize, Vec<Word>)>, String> {
    let reader = engine::Reader::new()?;
    let doc = godpdf_render::Document::open(Arc::new(pdf), None).map_err(|err| err.to_string())?;
    let mut pages = Vec::new();
    for page in 0..doc.page_count() {
        let size = doc.page_size(page).map_err(|err| err.to_string())?;
        let scale = SCALE.min(MAX_SIDE / size.width.max(size.height).max(1.0));
        let bitmap = doc.render(page, scale).map_err(|err| err.to_string())?;
        // The render may be smaller than asked on very large pages.
        let scale = bitmap.width as f32 / size.width.max(1.0);
        let typed = doc.text_chars(page).unwrap_or_default();
        let words: Vec<Word> = reader
            .read(&bitmap)
            .map_err(|err| format!("Page {}: {err}", page + 1))?
            .into_iter()
            .map(|(text, [l, t, r, b])| Word {
                text,
                rect: [l / scale, t / scale, r / scale, b / scale],
            })
            .filter(|word| !already_text(word, &typed))
            .collect();
        if !words.is_empty() {
            pages.push((page, words));
        }
    }
    Ok(pages)
}

/// The middle of `word` falls on a character the page already holds as text.
fn already_text(word: &Word, typed: &[TextChar]) -> bool {
    let [l, t, r, b] = word.rect;
    let (x, y) = ((l + r) / 2.0, (t + b) / 2.0);
    typed
        .iter()
        .any(|c| (c.x0..=c.x1).contains(&x) && (c.y0..=c.y1).contains(&y))
}

#[cfg(windows)]
mod engine {
    use godpdf_render::Bitmap;
    use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::Security::Cryptography::CryptographicBuffer;

    pub fn ready() -> bool {
        true
    }

    /// The recognizer of Windows, in the languages of the user.
    pub struct Reader(OcrEngine);

    impl Reader {
        pub fn new() -> Result<Self, String> {
            OcrEngine::TryCreateFromUserProfileLanguages()
                .map(Self)
                .map_err(|_| {
                    "Windows has no text recognition for your languages: add one in Settings, \
                     Time and language, Language"
                        .to_owned()
                })
        }

        /// Words and their boxes (left, top, right, bottom) in pixels.
        pub fn read(&self, bitmap: &Bitmap) -> Result<Vec<(String, [f32; 4])>, String> {
            let bgra: Vec<u8> = bitmap
                .rgba
                .chunks_exact(4)
                .flat_map(|px| [px[2], px[1], px[0], 255])
                .collect();
            let fail = |err: windows::core::Error| err.message().to_string();
            let buffer = CryptographicBuffer::CreateFromByteArray(&bgra).map_err(fail)?;
            let picture = SoftwareBitmap::CreateCopyFromBuffer(
                &buffer,
                BitmapPixelFormat::Bgra8,
                bitmap.width as i32,
                bitmap.height as i32,
            )
            .map_err(fail)?;
            let result = self
                .0
                .RecognizeAsync(&picture)
                .and_then(|operation| operation.join())
                .map_err(fail)?;
            let mut words = Vec::new();
            for line in result.Lines().map_err(fail)? {
                for word in line.Words().map_err(fail)? {
                    let rect = word.BoundingRect().map_err(fail)?;
                    let text = word.Text().map_err(fail)?.to_string();
                    words.push((
                        text,
                        [rect.X, rect.Y, rect.X + rect.Width, rect.Y + rect.Height],
                    ));
                }
            }
            Ok(words)
        }
    }
}

#[cfg(not(windows))]
mod engine {
    use std::path::{Path, PathBuf};

    use godpdf_render::Bitmap;
    use ocrs::{ImageSource, OcrEngine, OcrEngineParams, TextItem};
    use rten::Model;

    const MODELS: [(&str, &str); 2] = [
        (
            "text-detection.onnx",
            "https://ocrs-models.s3-accelerate.amazonaws.com/text-detection.onnx",
        ),
        (
            "text-recognition.onnx",
            "https://ocrs-models.s3-accelerate.amazonaws.com/text-recognition.onnx",
        ),
    ];

    fn folder() -> Option<PathBuf> {
        eframe::storage_dir(crate::APP_NAME).map(|dir| dir.join("ocr"))
    }

    pub fn ready() -> bool {
        folder().is_some_and(|dir| MODELS.iter().all(|(name, _)| dir.join(name).is_file()))
    }

    pub struct Reader(OcrEngine);

    impl Reader {
        pub fn new() -> Result<Self, String> {
            let dir = folder().ok_or("No folder to keep the text recognition models in")?;
            for (name, url) in MODELS {
                let path = dir.join(name);
                if !path.is_file() {
                    download(url, &path)?;
                }
            }
            let load = |name: &str| {
                Model::load_file(dir.join(name))
                    .map_err(|err| format!("Cannot load the text recognition model: {err}"))
            };
            OcrEngine::new(OcrEngineParams {
                detection_model: Some(load(MODELS[0].0)?),
                recognition_model: Some(load(MODELS[1].0)?),
                ..Default::default()
            })
            .map(Self)
            .map_err(|err| err.to_string())
        }

        pub fn read(&self, bitmap: &Bitmap) -> Result<Vec<(String, [f32; 4])>, String> {
            self.lines(bitmap).map_err(|err| err.to_string())
        }

        fn lines(&self, bitmap: &Bitmap) -> anyhow::Result<Vec<(String, [f32; 4])>> {
            let rgb: Vec<u8> = bitmap
                .rgba
                .chunks_exact(4)
                .flat_map(|px| [px[0], px[1], px[2]])
                .collect();
            let source = ImageSource::from_bytes(&rgb, (bitmap.width, bitmap.height))?;
            let input = self.0.prepare_input(source)?;
            let found = self.0.detect_words(&input)?;
            let lines = self.0.find_text_lines(&input, &found);
            let mut words = Vec::new();
            for line in self.0.recognize_text(&input, &lines)?.into_iter().flatten() {
                for word in line.words() {
                    let text = word.to_string();
                    // Lone marks are mostly specks of the scan.
                    if !text.chars().any(char::is_alphanumeric) {
                        continue;
                    }
                    let rect = word.bounding_rect();
                    words.push((
                        text,
                        [
                            rect.left() as f32,
                            rect.top() as f32,
                            rect.right() as f32,
                            rect.bottom() as f32,
                        ],
                    ));
                }
            }
            Ok(words)
        }
    }

    fn download(url: &str, path: &Path) -> Result<(), String> {
        let fail = |err: String| format!("Cannot download the text recognition model: {err}");
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|err| fail(err.to_string()))?;
        }
        let mut response = ureq::get(url)
            .header("User-Agent", concat!("GodPDF/", env!("CARGO_PKG_VERSION")))
            .call()
            .map_err(|err| fail(err.to_string()))?;
        let part = path.with_extension("part");
        let mut file = std::fs::File::create(&part).map_err(|err| fail(err.to_string()))?;
        let copied = std::io::copy(&mut response.body_mut().as_reader(), &mut file);
        drop(file);
        match copied {
            Ok(bytes) if bytes > 0 => {
                std::fs::rename(&part, path).map_err(|err| fail(err.to_string()))
            }
            Ok(_) => {
                let _ = std::fs::remove_file(&part);
                Err(fail("the download is empty".to_owned()))
            }
            Err(err) => {
                let _ = std::fs::remove_file(&part);
                Err(fail(err.to_string()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_over_typed_text_are_left_out() {
        let typed = [TextChar {
            text: "A".into(),
            x0: 10.0,
            y0: 10.0,
            x1: 20.0,
            y1: 22.0,
        }];
        let word = |rect| Word {
            text: "A".into(),
            rect,
        };
        assert!(already_text(&word([9.0, 9.0, 21.0, 23.0]), &typed));
        assert!(!already_text(&word([40.0, 9.0, 60.0, 23.0]), &typed));
    }
}
