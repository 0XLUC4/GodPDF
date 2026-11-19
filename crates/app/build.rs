//! Embeds the app icon and version info into the Windows executable, and builds the Explorer
//! thumbnail DLL (`crates/thumbs`) that the executable carries (`services/association.rs`).

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=../../assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("../../assets/icon.ico");
    resource.set("ProductName", "GodPDF");
    resource.set("FileDescription", "GodPDF");
    if let Err(err) = resource.compile() {
        // A missing Windows SDK must not block the build; the exe just keeps the default icon.
        println!("cargo:warning=could not embed the Windows icon: {err}");
    }
    build_thumbnail_dll();
}

/// A cdylib cannot be a dependency: build it with a nested cargo, in its own target folder
/// (the outer build holds the lock of `target/`), always optimized.
fn build_thumbnail_dll() {
    println!("cargo:rerun-if-changed=../thumbs");
    println!("cargo:rerun-if-changed=../render/src");
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target_dir = workspace.join("target").join("thumbs");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let status = Command::new(cargo)
        .args([
            "build",
            "--release",
            "--locked",
            "-p",
            "godpdf-thumbs",
            "--manifest-path",
        ])
        .arg(workspace.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(&target_dir)
        // Let `.cargo/config.toml` give the flags, as for the app.
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .status();
    match status {
        Ok(status) if status.success() => {}
        Ok(status) => panic!("building the thumbnail DLL failed ({status})"),
        Err(err) => panic!("cannot run cargo to build the thumbnail DLL: {err}"),
    }
    let dll = target_dir.join("release").join("godpdf_thumbs.dll");
    println!("cargo:rustc-env=GODPDF_THUMBS_DLL={}", dll.display());
}
