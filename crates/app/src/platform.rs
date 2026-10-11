//! File I/O. Native: synchronous rfd dialogs + `std::fs`. Web: async rfd + browser downloads.
//!
//! Picked files always arrive through an [`Inbox`] so the app handles native (immediate)
//! and web (async) the same way: drain the inbox once per frame.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// A PDF loaded from disk / the browser.
pub struct PickedFile {
    pub name: String,
    pub path: Option<PathBuf>,
    pub bytes: Vec<u8>,
}

/// Why the user picked files.
#[derive(Clone, Copy, Debug)]
pub enum Purpose {
    Open,
    MergeAdd,
    Insert { tab: u64, at: usize },
}

type Batch = (Purpose, Vec<PickedFile>);

/// Thread-safe queue of picked files.
#[derive(Clone, Default)]
pub struct Inbox(Arc<Mutex<Vec<Batch>>>);

impl Inbox {
    pub fn push(&self, purpose: Purpose, files: Vec<PickedFile>) {
        if files.is_empty() {
            return;
        }
        if let Ok(mut queue) = self.0.lock() {
            queue.push((purpose, files));
        }
    }

    pub fn drain(&self) -> Vec<Batch> {
        self.0
            .lock()
            .map(|mut queue| std::mem::take(&mut *queue))
            .unwrap_or_default()
    }
}

/// Result of a save request.
pub enum Saved {
    /// Written. Path is `None` on web (downloaded).
    To(Option<PathBuf>),
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Cancelled,
}

fn pdf_name(name: &str) -> String {
    if name.to_ascii_lowercase().ends_with(".pdf") {
        name.to_owned()
    } else {
        format!("{name}.pdf")
    }
}

/// `report.pdf` + 2 → `report_part2.pdf`.
pub fn part_name(base: &str, index: usize) -> String {
    let stem = base
        .strip_suffix(".pdf")
        .or_else(|| base.strip_suffix(".PDF"))
        .unwrap_or(base);
    format!("{stem}_part{index}.pdf")
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::*;
#[cfg(target_arch = "wasm32")]
pub use web::*;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::{Inbox, PickedFile, Purpose, Saved, part_name, pdf_name};
    use std::ffi::OsString;
    use std::io::Write as _;
    use std::path::{Path, PathBuf};

    fn dialog() -> rfd::FileDialog {
        rfd::FileDialog::new().add_filter("PDF document", &["pdf"])
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
            path: Some(path.to_path_buf()),
            bytes,
        })
    }

    pub fn pick_pdfs(
        multiple: bool,
        purpose: Purpose,
        inbox: &Inbox,
        _ctx: &egui::Context,
    ) -> Result<(), String> {
        let paths = if multiple {
            dialog().pick_files().unwrap_or_default()
        } else {
            dialog().pick_file().into_iter().collect()
        };
        let files = paths
            .iter()
            .map(|path| read_file(path))
            .collect::<Result<Vec<_>, _>>()?;
        inbox.push(purpose, files);
        Ok(())
    }

    /// Write to `path` when given, otherwise ask where.
    pub fn save_pdf(name: &str, path: Option<&Path>, bytes: &[u8]) -> Result<Saved, String> {
        let target = match path {
            Some(path) => path.to_path_buf(),
            None => match dialog().set_file_name(pdf_name(name)).save_file() {
                Some(path) => path,
                None => return Ok(Saved::Cancelled),
            },
        };
        write_atomic(&target, bytes)?;
        Ok(Saved::To(Some(target)))
    }

    /// Hand `bytes` to the system to print, as a file of its own: what was changed here need
    /// not be saved first. Returns what was done, to tell the user.
    pub fn print_pdf(name: &str, bytes: &[u8]) -> Result<&'static str, String> {
        let path = std::env::temp_dir().join(format!("godpdf-print-{}", pdf_name(name)));
        std::fs::write(&path, bytes)
            .map_err(|err| format!("Cannot write {}: {err}", path.display()))?;
        hand_to_printer(&path)
    }

    /// The program that opens PDFs prints the file when it says it can; otherwise it opens
    /// it, to print from there.
    #[cfg(windows)]
    fn hand_to_printer(path: &Path) -> Result<&'static str, String> {
        use std::os::windows::process::CommandExt as _;
        // The path goes by the environment: no quoting to get wrong.
        let start = |verb: &str| {
            let script = format!("Start-Process -FilePath $env:GODPDF_FILE {verb}");
            std::process::Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", &script])
                .env("GODPDF_FILE", path)
                // No console window of its own.
                .creation_flags(0x0800_0000)
                .status()
                .is_ok_and(|status| status.success())
        };
        if start("-Verb Print") {
            Ok("Sent to the printer by your PDF program")
        } else if start("") {
            Ok("Opened in your PDF program: print from there (Ctrl+P)")
        } else {
            Err("No program on this PC opens PDF files to print them".to_owned())
        }
    }

    #[cfg(not(windows))]
    fn hand_to_printer(path: &Path) -> Result<&'static str, String> {
        let sent = std::process::Command::new("lp").arg(path).status();
        if sent.is_ok_and(|status| status.success()) {
            Ok("Sent to the default printer")
        } else {
            Err("Could not print: is the `lp` command installed?".to_owned())
        }
    }

    /// Ask for a folder and write `base_part1.pdf`, ... Returns the folder, `None` if cancelled.
    pub fn save_parts(base: &str, parts: &[Vec<u8>]) -> Result<Option<String>, String> {
        let Some(folder) = rfd::FileDialog::new().pick_folder() else {
            return Ok(None);
        };
        for (index, bytes) in parts.iter().enumerate() {
            write_atomic(&free_path(&folder, &part_name(base, index + 1)), bytes)?;
        }
        Ok(Some(folder.display().to_string()))
    }

    /// Write to `<name>.godpdf.tmp` next to `target`, then rename over it, so a failed
    /// write never leaves a truncated file behind.
    fn write_atomic(target: &Path, bytes: &[u8]) -> Result<(), String> {
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

    /// `folder/name.pdf`, or `folder/name (2).pdf`, ... if taken.
    fn free_path(folder: &Path, name: &str) -> PathBuf {
        let stem = name.strip_suffix(".pdf").unwrap_or(name);
        let mut path = folder.join(name);
        let mut copy = 2;
        while path.exists() {
            path = folder.join(format!("{stem} ({copy}).pdf"));
            copy += 1;
        }
        path
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use super::{Inbox, PickedFile, Purpose, Saved, part_name, pdf_name};
    use std::path::Path;
    use wasm_bindgen::JsCast as _;

    pub fn read_file(path: &Path) -> Result<PickedFile, String> {
        Err(format!("Cannot open {} in the browser", path.display()))
    }

    pub fn pick_pdfs(
        multiple: bool,
        purpose: Purpose,
        inbox: &Inbox,
        ctx: &egui::Context,
    ) -> Result<(), String> {
        let inbox = inbox.clone();
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let dialog = rfd::AsyncFileDialog::new().add_filter("PDF document", &["pdf"]);
            let handles = if multiple {
                dialog.pick_files().await.unwrap_or_default()
            } else {
                dialog.pick_file().await.into_iter().collect()
            };
            let mut files = Vec::with_capacity(handles.len());
            for handle in handles {
                files.push(PickedFile {
                    name: handle.file_name(),
                    path: None,
                    bytes: handle.read().await,
                });
            }
            inbox.push(purpose, files);
            ctx.request_repaint();
        });
        Ok(())
    }

    fn download(name: &str, bytes: &[u8]) -> Result<(), String> {
        let fail = |what: &str| format!("Download failed: {what}");
        let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
        let options = web_sys::BlobPropertyBag::new();
        options.set_type("application/pdf");
        let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options)
            .map_err(|_| fail("blob"))?;
        let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(|_| fail("url"))?;
        let anchor = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.create_element("a").ok())
            .and_then(|element| element.dyn_into::<web_sys::HtmlAnchorElement>().ok())
            .ok_or_else(|| fail("no document"))?;
        anchor.set_href(&url);
        anchor.set_download(name);
        anchor.click();
        let _ = web_sys::Url::revoke_object_url(&url);
        Ok(())
    }

    pub fn save_pdf(name: &str, _path: Option<&Path>, bytes: &[u8]) -> Result<Saved, String> {
        download(&pdf_name(name), bytes)?;
        Ok(Saved::To(None))
    }

    pub fn print_pdf(_name: &str, _bytes: &[u8]) -> Result<&'static str, String> {
        Err("Printing is not available in the browser: save the file and print it".to_owned())
    }

    pub fn save_parts(base: &str, parts: &[Vec<u8>]) -> Result<Option<String>, String> {
        for (index, bytes) in parts.iter().enumerate() {
            download(&part_name(base, index + 1), bytes)?;
        }
        Ok(Some("Downloads".to_owned()))
    }
}
