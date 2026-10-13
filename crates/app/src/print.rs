//! Printing on Windows: the print dialog of the system, then each page drawn by GodPDF itself,
//! so that no other PDF program is needed.

use std::io::{BufRead as _, BufReader, BufWriter};
use std::os::windows::process::CommandExt as _;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use godpdf_render::Document;
use image::ImageEncoder as _;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};

/// Dots per inch the pages are drawn at for the printer.
const DPI: f32 = 300.0;
/// How many drawn pages may wait for the printer: more would only fill the disk.
const AHEAD: usize = 3;
/// Shows the dialog and prints the pages: see what it says of itself.
const SCRIPT: &str = include_str!("print.ps1");

/// Print the PDF `bytes`, named `name`, through the print dialog. Returns what was done, to
/// tell the user: `None` when the dialog was closed without printing.
pub fn print(name: &str, bytes: Vec<u8>) -> Result<Option<&'static str>, String> {
    let doc =
        Document::open(Arc::new(bytes), None).map_err(|err| format!("Cannot print: {err}"))?;
    let dir = std::env::temp_dir().join(format!("godpdf-print-{}", std::process::id()));
    std::fs::create_dir_all(&dir)
        .map_err(|err| format!("Cannot write {}: {err}", dir.display()))?;
    let done = run(name, &doc, &dir);
    let _ = std::fs::remove_dir_all(&dir);
    done
}

fn run(name: &str, doc: &Document, dir: &Path) -> Result<Option<&'static str>, String> {
    let mut child = Command::new("powershell")
        .args(["-NoProfile", "-STA", "-Command", SCRIPT])
        .env("GODPDF_PRINT_DIR", dir)
        .env("GODPDF_PRINT_NAME", name)
        .env("GODPDF_PRINT_PAGES", doc.page_count().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        // No console window of its own.
        .creation_flags(0x0800_0000)
        .spawn()
        .map_err(|err| format!("Cannot open the print dialog: {err}"))?;
    let said = child.stdout.take().map(BufReader::new);
    let said = said.into_iter().flat_map(BufReader::lines);
    let mut done = Err("Printing stopped without saying why".to_owned());
    for line in said.map_while(Result::ok) {
        let line = line.trim();
        if line == "cancel" {
            done = Ok(None);
        } else if line == "done" {
            done = Ok(Some("Sent to the printer"));
        } else if let Some(why) = line.strip_prefix("error ") {
            done = Err(format!("Cannot print: {why}"));
        } else if let Some(pages) = line.strip_prefix("pages ") {
            let mut numbers = pages.split(' ').filter_map(|number| number.parse().ok());
            if let (Some(first), Some(last)) = (numbers.next(), numbers.next()) {
                draw_pages(doc, dir, first..=last, &mut child)?;
            }
        }
    }
    let _ = child.wait();
    done
}

/// Draw the pages `pages` (the first is 1) into `dir`, one file each, no more than [`AHEAD`]
/// before the printer takes them. Stops when the printing itself has.
fn draw_pages(
    doc: &Document,
    dir: &Path,
    pages: std::ops::RangeInclusive<usize>,
    printing: &mut Child,
) -> Result<(), String> {
    let stopped = |printing: &mut Child| !matches!(printing.try_wait(), Ok(None));
    for page in pages {
        let waiting = || std::fs::read_dir(dir).map_or(0, Iterator::count);
        while waiting() >= AHEAD {
            if stopped(printing) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        if stopped(printing) {
            return Ok(());
        }
        let failed = |err: &dyn std::fmt::Display| format!("Cannot print page {page}: {err}");
        let bitmap = doc
            .render(page.saturating_sub(1), DPI / 72.0)
            .map_err(|err| failed(&err))?;
        // Written under another name: the printer must not take half a file.
        let part = dir.join(format!("{page}.part"));
        let file = std::fs::File::create(&part).map_err(|err| failed(&err))?;
        PngEncoder::new_with_quality(BufWriter::new(file), CompressionType::Fast, FilterType::Sub)
            .write_image(
                &bitmap.rgba,
                bitmap.width,
                bitmap.height,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|err| failed(&err))?;
        std::fs::rename(&part, dir.join(format!("{page}.png"))).map_err(|err| failed(&err))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    /// Prints the PDF named by `GODPDF_TEST_PDF` to the file named by `GODPDF_PRINT_TO`, with
    /// the printer to PDF of Windows and no dialog.
    #[test]
    #[ignore = "needs the printer to PDF of Windows, and the two variables set"]
    fn a_document_is_printed_page_by_page() {
        let pdf = std::env::var("GODPDF_TEST_PDF").unwrap();
        let out = std::env::var("GODPDF_PRINT_TO").unwrap();
        let bytes = std::fs::read(&pdf).unwrap();
        assert_eq!(super::print("test", bytes), Ok(Some("Sent to the printer")));
        assert!(std::fs::metadata(&out).unwrap().len() > 1000);
    }
}
