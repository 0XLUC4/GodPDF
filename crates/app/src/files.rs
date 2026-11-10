//! Native file access: pick, read and write PDFs.

use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::App;
use rfd::{AsyncFileDialog, FileHandle};

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

/// What Open, Combine and Insert accept: pictures and text are made into PDFs (`import.rs`).
const OPENABLE: [&str; 5] = ["pdf", "jpg", "jpeg", "png", "txt"];

/// A file dialog is up: another one would stack on it (Ctrl+O twice opens two).
static ASKING: AtomicBool = AtomicBool::new(false);

/// Run the native dialog `dialog` makes without blocking, then `then` with its answer; nothing
/// while another one is up. A blocking dialog runs a nested Windows message loop, where gpui
/// tasks borrow the app again and abort the process.
// shortcut: the dialogs have no parent window (that needs its raw handle, kept as unsafe
// code), so one can fall behind the app; pass `set_parent` if that bothers.
pub fn ask<T: 'static, F: Future<Output = T> + 'static>(
    cx: &App,
    dialog: impl FnOnce() -> F,
    then: impl FnOnce(T, &mut App) + 'static,
) {
    if ASKING.swap(true, Ordering::SeqCst) {
        return;
    }
    let dialog = dialog();
    cx.spawn(async move |cx| {
        let answer = dialog.await;
        ASKING.store(false, Ordering::SeqCst);
        cx.update(|cx| then(answer, cx));
    })
    .detach();
}

fn paths(handles: Option<Vec<FileHandle>>) -> Vec<PathBuf> {
    handles
        .unwrap_or_default()
        .iter()
        .map(|handle| handle.path().to_path_buf())
        .collect()
}

/// Ask for files to open (PDFs, pictures, text); empty when cancelled.
pub fn pick_pdfs(cx: &App, then: impl FnOnce(Vec<PathBuf>, &mut App) + 'static) {
    let dialog = || {
        AsyncFileDialog::new()
            .add_filter("PDFs, images and text", &OPENABLE)
            .pick_files()
    };
    ask(cx, dialog, move |picked, cx| then(paths(picked), cx));
}

/// Ask for any files; empty when cancelled.
pub fn pick_any(cx: &App, then: impl FnOnce(Vec<PathBuf>, &mut App) + 'static) {
    let dialog = || AsyncFileDialog::new().pick_files();
    ask(cx, dialog, move |picked, cx| then(paths(picked), cx));
}

/// Ask for one file matching `filter` (`(label, extensions)`).
pub fn pick_file(
    cx: &App,
    filter: (&str, &[&str]),
    then: impl FnOnce(PathBuf, &mut App) + 'static,
) {
    let dialog = || {
        AsyncFileDialog::new()
            .add_filter(filter.0, filter.1)
            .pick_file()
    };
    ask(cx, dialog, move |picked, cx| {
        if let Some(handle) = picked {
            then(handle.path().to_path_buf(), cx);
        }
    });
}

/// Ask for a folder; nothing happens when cancelled.
pub fn pick_folder(cx: &App, then: impl FnOnce(PathBuf, &mut App) + 'static) {
    ask(
        cx,
        || AsyncFileDialog::new().pick_folder(),
        move |picked, cx| {
            if let Some(handle) = picked {
                then(handle.path().to_path_buf(), cx);
            }
        },
    );
}

/// Ask where to save `name` (`pdf` filter when `pdf`); nothing happens when cancelled.
pub fn pick_save(cx: &App, name: &str, pdf: bool, then: impl FnOnce(PathBuf, &mut App) + 'static) {
    let mut dialog = AsyncFileDialog::new();
    if pdf {
        dialog = dialog
            .add_filter("PDF document", &["pdf"])
            .set_file_name(pdf_name(name));
    } else {
        dialog = dialog.set_file_name(name);
    }
    ask(
        cx,
        || dialog.save_file(),
        move |picked, cx| {
            if let Some(handle) = picked {
                then(handle.path().to_path_buf(), cx);
            }
        },
    );
}

/// Ask where to save a new PDF and write it there; `done` gets the path once written.
pub fn save_new_pdf(
    cx: &App,
    name: &str,
    bytes: Vec<u8>,
    done: impl FnOnce(PathBuf, &mut App) + 'static,
) {
    pick_save(cx, name, true, move |target, cx| {
        match write_atomic(&target, &bytes) {
            Ok(()) => done(target, cx),
            Err(err) => crate::toasts::error(cx, err),
        }
    });
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
        let dir = std::env::temp_dir().join(format!("godpdf-{}", std::process::id()));
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
