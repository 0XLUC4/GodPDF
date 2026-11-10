//! Native file access: pick, read and write PDFs (ported from `crates/app/src/platform.rs`).

use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// A file read from disk.
pub struct PickedFile {
    pub name: String,
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

pub fn read_file(path: &Path) -> Result<PickedFile, String> {
    let bytes =
        std::fs::read(path).map_err(|err| format!("Cannot read {}: {err}", path.display()))?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "document.pdf".to_owned());
    Ok(PickedFile {
        name,
        path: path.to_path_buf(),
        bytes,
    })
}

/// Ask for PDF files to open; empty when cancelled. Blocks until the dialog closes.
pub fn pick_pdfs() -> Vec<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("PDF document", &["pdf"])
        .pick_files()
        .unwrap_or_default()
}

/// Write to `path` when given, otherwise ask where. `Ok(None)` when the user cancelled.
pub fn save_pdf(name: &str, path: Option<&Path>, bytes: &[u8]) -> Result<Option<PathBuf>, String> {
    let target = match path {
        Some(path) => path.to_path_buf(),
        None => match rfd::FileDialog::new()
            .add_filter("PDF document", &["pdf"])
            .set_file_name(pdf_name(name))
            .save_file()
        {
            Some(path) => path,
            None => return Ok(None),
        },
    };
    write_atomic(&target, bytes)?;
    Ok(Some(target))
}

fn pdf_name(name: &str) -> String {
    if name.to_ascii_lowercase().ends_with(".pdf") {
        name.to_owned()
    } else {
        format!("{name}.pdf")
    }
}

/// Write to a temp file next to `target`, then rename: a failed write never truncates it.
pub fn write_atomic(target: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut temp_name = target
        .file_name()
        .map(OsString::from)
        .unwrap_or_else(|| OsString::from("document.pdf"));
    temp_name.push(".godpdf.tmp");
    let temp = target.with_file_name(temp_name);
    let written = std::fs::File::create(&temp)
        .and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        })
        .and_then(|()| std::fs::rename(&temp, target));
    written.map_err(|err| {
        // Best effort: the temp file may not exist.
        let _ = std::fs::remove_file(&temp);
        format!("Cannot write {}: {err}", target.display())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_the_file() {
        let dir = std::env::temp_dir().join(format!("godpdf-gpui-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("a.pdf");
        std::fs::write(&target, b"old").unwrap();
        write_atomic(&target, b"new").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert!(!dir.join("a.pdf.godpdf.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn names_get_a_pdf_extension() {
        assert_eq!(pdf_name("report"), "report.pdf");
        assert_eq!(pdf_name("a.PDF"), "a.PDF");
    }
}
