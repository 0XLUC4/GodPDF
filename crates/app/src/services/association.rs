//! GodPDF as a PDF app of Windows: its file type (`GodPDF.Document`: icon, "open" command),
//! its entry in Settings > Default apps, and the Explorer thumbnails of PDF files.
//!
//! Windows lets only the user pick the default app; once GodPDF is picked, PDF files open on
//! a double click and Explorer shows page 1 with the GodPDF icon in the corner. All is
//! written under HKEY_CURRENT_USER (no admin rights) at every start, so a moved or updated
//! program stays registered.

use std::path::{Path, PathBuf};

use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegGetValueW, RegSetKeyValueW,
};
use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
use windows::core::{HSTRING, PCWSTR};

/// The thumbnail provider, built by `build.rs` from `crates/thumbs`.
static THUMBS_DLL: &[u8] = include_bytes!(env!("GODPDF_THUMBS_DLL"));
/// Must match `CLSID` in `crates/thumbs/src/lib.rs`.
const THUMBS_CLSID: &str = "{033B742F-20FF-421E-821F-D1B9C902AA8A}";
/// Shell extension key of thumbnail providers.
const THUMBNAIL_HANDLER: &str = "{e357fccd-a995-4576-b01f-234630154e96}";
const PROG_ID: &str = "GodPDF.Document";
const CAPABILITIES: &str = r"Software\GodPDF\Capabilities";

/// The page of Settings that sets GodPDF as the default app.
pub const DEFAULT_APPS_PAGE: &str = "ms-settings:defaultapps?registeredAppUser=GodPDF";

/// Register GodPDF and its thumbnails. Blocking (disk, registry): call it off the UI thread.
/// Debug builds leave the registration of the installed program alone.
pub fn register() {
    if cfg!(debug_assertions) {
        return;
    }
    if let Err(err) = try_register() {
        eprintln!("{}: cannot register as a PDF app: {err}", crate::APP_NAME);
    }
}

fn try_register() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|err| err.to_string())?;
    let dll_dir = super::settings::dir().ok_or("no app data folder")?;
    let dll = install_dll(&dll_dir).map_err(|err| format!("thumbnail DLL: {err}"))?;
    let changed = entries(&exe, &dll)
        .iter()
        .filter(|(key, name, value)| set(key, name, value))
        .count();
    // Explorer reads the file types again (icons, thumbnails) only when told.
    if changed > 0 {
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    }
    Ok(())
}

/// Write the DLL under a versioned name: an older one may still be loaded by Explorer,
/// which locks it. Older ones are removed when free.
fn install_dll(dir: &Path) -> std::io::Result<PathBuf> {
    let name = format!("thumbs-{}.dll", env!("CARGO_PKG_VERSION"));
    let dll = dir.join(&name);
    let current = std::fs::metadata(&dll).is_ok_and(|meta| meta.len() == THUMBS_DLL.len() as u64);
    if !current {
        let partial = dll.with_extension("dll.part");
        std::fs::write(&partial, THUMBS_DLL)?;
        std::fs::rename(&partial, &dll)?;
    }
    for entry in std::fs::read_dir(dir)?.flatten() {
        let other = entry.file_name().to_string_lossy().into_owned();
        if other.starts_with("thumbs-") && other.ends_with(".dll") && other != name {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    Ok(dll)
}

/// (key under HKCU, value name ("" = default), data), all strings.
fn entries(exe: &Path, dll: &Path) -> Vec<(String, String, String)> {
    let exe_path = exe.display().to_string();
    let exe_name = exe
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let icon = format!("{exe_path},0");
    let open = format!("\"{exe_path}\" \"%1\"");
    let classes = r"Software\Classes";
    let prog = format!(r"{classes}\{PROG_ID}");
    // Also the key Windows makes when the program is picked with "Browse" in "Open with".
    let app = format!(r"{classes}\Applications\{exe_name}");
    let clsid = format!(r"{classes}\CLSID\{THUMBS_CLSID}");
    let entry =
        |key: &str, name: &str, value: &str| (key.to_owned(), name.to_owned(), value.to_owned());
    let mut entries = vec![
        entry(&clsid, "", "GodPDF thumbnails"),
        entry(
            &format!(r"{clsid}\InprocServer32"),
            "",
            &dll.display().to_string(),
        ),
        entry(
            &format!(r"{clsid}\InprocServer32"),
            "ThreadingModel",
            "Apartment",
        ),
        entry(&format!(r"{classes}\.pdf\OpenWithProgids"), PROG_ID, ""),
        entry(&prog, "", "PDF Document"),
        entry(CAPABILITIES, "ApplicationName", crate::APP_NAME),
        entry(
            CAPABILITIES,
            "ApplicationDescription",
            "Every PDF tool. Local.",
        ),
        entry(CAPABILITIES, "ApplicationIcon", &icon),
        entry(
            &format!(r"{CAPABILITIES}\FileAssociations"),
            ".pdf",
            PROG_ID,
        ),
        entry(
            r"Software\RegisteredApplications",
            crate::APP_NAME,
            CAPABILITIES,
        ),
        entry(&format!(r"{app}\SupportedTypes"), ".pdf", ""),
    ];
    for key in [&prog, &app] {
        entries.extend([
            entry(&format!(r"{key}\DefaultIcon"), "", &icon),
            entry(&format!(r"{key}\shell\open\command"), "", &open),
            entry(
                &format!(r"{key}\ShellEx\{THUMBNAIL_HANDLER}"),
                "",
                THUMBS_CLSID,
            ),
            // The icon Explorer draws in the corner of the thumbnails.
            entry(key, "TypeOverlay", &icon),
        ]);
    }
    entries
}

/// Write a string value unless it is already so; true when written.
fn set(key: &str, name: &str, value: &str) -> bool {
    if get(key, name).as_deref() == Some(value) {
        return false;
    }
    let data: Vec<u16> = value.encode_utf16().chain([0]).collect();
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(key),
            &HSTRING::from(name),
            REG_SZ.0,
            Some(data.as_ptr().cast()),
            (data.len() * 2) as u32,
        )
    };
    if status.is_err() {
        eprintln!(
            "{}: cannot write HKCU\\{key} {name}: {status:?}",
            crate::APP_NAME
        );
    }
    status.is_ok()
}

fn get(key: &str, name: &str) -> Option<String> {
    let (key, name) = (HSTRING::from(key), HSTRING::from(name));
    let read = |buffer: Option<&mut [u16]>, size: &mut u32| unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            buffer.map(|buffer| buffer.as_mut_ptr().cast()),
            Some(size),
        )
    };
    let mut size = 0u32;
    read(None, &mut size).ok().ok()?;
    let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
    read(Some(&mut buffer), &mut size).ok().ok()?;
    let text = String::from_utf16_lossy(&buffer);
    Some(text.trim_end_matches('\0').to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dll_is_embedded() {
        assert!(THUMBS_DLL.starts_with(b"MZ"));
    }

    #[test]
    fn entries_point_at_program_and_dll() {
        let exe = Path::new(r"C:\Apps\GodPDF.exe");
        let dll = Path::new(r"C:\Data\thumbs-1.0.0.dll");
        let entries = entries(exe, dll);
        let find = |key: &str, name: &str| {
            entries
                .iter()
                .find(|(k, n, _)| k == key && n == name)
                .map(|(_, _, value)| value.as_str())
        };
        let prog = r"Software\Classes\GodPDF.Document";
        assert_eq!(
            find(&format!(r"{prog}\shell\open\command"), ""),
            Some(r#""C:\Apps\GodPDF.exe" "%1""#)
        );
        assert_eq!(find(prog, "TypeOverlay"), Some(r"C:\Apps\GodPDF.exe,0"));
        assert_eq!(
            find(
                &format!(r"Software\Classes\Applications\GodPDF.exe\ShellEx\{THUMBNAIL_HANDLER}"),
                ""
            ),
            Some(THUMBS_CLSID)
        );
        assert_eq!(
            find(
                &format!(r"Software\Classes\CLSID\{THUMBS_CLSID}\InprocServer32"),
                ""
            ),
            Some(r"C:\Data\thumbs-1.0.0.dll")
        );
    }
}
