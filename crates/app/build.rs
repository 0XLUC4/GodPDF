//! Generates icons from the single SVG source and embeds the Windows executable icon.
use std::{env,fs,path::PathBuf};
const SVG: &str = include_str!("../../assets/icon.svg");

fn png(tree: &resvg::usvg::Tree, size: u32, path: &std::path::Path) -> Vec<u8> {
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).expect("icon pixmap");
    let bounds = tree.size();
    let scale = resvg::tiny_skia::Transform::from_scale(
        size as f32 / bounds.width(),
        size as f32 / bounds.height(),
    );
    resvg::render(tree, scale, &mut pixmap.as_mut());
    // tiny-skia stores premultiplied RGBA; unpremultiply for PNG/ICO.
    let mut rgba = pixmap.data().to_vec();
    for pixel in rgba.chunks_exact_mut(4) {
        let alpha = pixel[3] as u32;
        if alpha != 0 && alpha != 255 {
            for channel in &mut pixel[..3] {
                *channel = ((*channel as u32 * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    image::save_buffer_with_format(
        path, &rgba, size, size, image::ColorType::Rgba8, image::ImageFormat::Png,
    ).expect("write generated app icon");
    rgba
}

fn main() {
    println!("cargo:rerun-if-changed=../../assets/icon.svg");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let options = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_str(SVG, &options).expect("valid icon SVG");
    png(&tree, 512, &out.join("godpdf-logo.png"));
    png(&tree, 256, &out.join("godpdf-favicon.png"));

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let mut directory = ico::IconDir::new(ico::ResourceType::Icon);
    for size in [16, 24, 32, 48, 64, 128, 256] {
        let pixels = png(&tree, size, &out.join(format!("godpdf-{size}.png")));
        let icon = ico::IconImage::from_rgba_data(size, size, pixels);
        directory.add_entry(ico::IconDirEntry::encode(&icon).expect("encode ICO entry"));
    }
    let icon_path = out.join("godpdf.ico");
    let mut file = fs::File::create(&icon_path).expect("create ICO");
    directory.write(&mut file).expect("write ICO");

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(icon_path.to_str().expect("UTF-8 icon path"));
    resource.set("ProductName", "GodPDF");
    resource.set("FileDescription", "GodPDF");
    if let Err(err) = resource.compile() {
        println!("cargo:warning=could not embed the Windows icon: {err}");
    }
}
