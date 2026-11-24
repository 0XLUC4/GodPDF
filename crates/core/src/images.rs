//! Pictures of a document: where the pages draw them, decoded, and made lighter.

use std::collections::{HashMap, HashSet};

use image::DynamicImage;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use lopdf::content::Content;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

use crate::create::jpeg_info;
use crate::pdf::{deref, dict_entry, inherited, page_ids};
use crate::retype::{IDENTITY, Matrix, then};

/// How deep forms drawn inside forms are followed.
const MAX_FORM_DEPTH: usize = 8;
/// Pictures with more pixels are left alone: decoding them would take too much memory.
const MAX_PIXELS: u64 = 60_000_000;
/// A picture is resampled only when it is this much sharper than asked.
const SLACK: f32 = 1.2;
/// A picture no page draws (in a comment, a pattern...) is taken to fill the long side
/// of an A4 page, in inches: large enough never to make it look blurred.
const FALLBACK_INCHES: f32 = 842.0 / 72.0;

/// How pictures are made lighter by [`crate::compress_with`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageOptions {
    /// Pictures sharper than this, as drawn on their page, are scaled down to it.
    pub dpi: u32,
    /// JPEG quality of the pictures written again, 1 to 100.
    pub quality: u8,
}

/// A picture decoded to 8-bit grey or RGB pixels.
pub(crate) struct Decoded {
    pub(crate) pixels: DynamicImage,
    /// Stored as a JPEG: its bytes can be used as they are.
    pub(crate) jpeg: bool,
}

/// Every picture `page` draws (forms included), with the matrix it is drawn with: the
/// unit square of the picture in page space.
pub(crate) fn page_images(doc: &Document, page: ObjectId) -> Vec<(ObjectId, Matrix)> {
    let mut found = Vec::new();
    let resources = inherited(doc, page, b"Resources");
    if let Some(resources) = resources
        .as_ref()
        .and_then(|r| deref(doc, r)?.as_dict().ok())
    {
        let content = doc.get_page_content(page);
        walk(doc, &content, resources, IDENTITY, 0, &mut found);
    }
    found
}

fn walk(
    doc: &Document,
    content: &[u8],
    resources: &Dictionary,
    base: Matrix,
    depth: usize,
    found: &mut Vec<(ObjectId, Matrix)>,
) {
    let Ok(content) = Content::decode(content) else {
        return;
    };
    let xobjects = dict_entry(doc, resources, b"XObject");
    let mut ctm = base;
    let mut saved = Vec::new();
    for operation in &content.operations {
        match operation.operator.as_str() {
            "q" => saved.push(ctm),
            "Q" => ctm = saved.pop().unwrap_or(base),
            "cm" => {
                if let Some(matrix) = matrix(doc, &operation.operands) {
                    ctm = then(matrix, ctm);
                }
            }
            "Do" => {
                let id = operation
                    .operands
                    .first()
                    .and_then(|name| name.as_name().ok())
                    .and_then(|name| xobjects?.get(name).ok()?.as_reference().ok());
                let Some(id) = id else { continue };
                let Ok(stream) = doc.get_object(id).and_then(Object::as_stream) else {
                    continue;
                };
                match stream.dict.get(b"Subtype").and_then(Object::as_name) {
                    Ok(b"Image") => found.push((id, ctm)),
                    Ok(b"Form") if depth < MAX_FORM_DEPTH => {
                        let form_matrix = stream
                            .dict
                            .get(b"Matrix")
                            .ok()
                            .and_then(|m| deref(doc, m)?.as_array().ok())
                            .and_then(|m| matrix(doc, m))
                            .unwrap_or(IDENTITY);
                        let inner =
                            dict_entry(doc, &stream.dict, b"Resources").unwrap_or(resources);
                        if let Ok(bytes) = stream.decompressed_content() {
                            walk(doc, &bytes, inner, then(form_matrix, ctm), depth + 1, found);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

fn matrix(doc: &Document, operands: &[Object]) -> Option<Matrix> {
    let numbers: Vec<f32> = operands
        .iter()
        .map(|n| deref(doc, n)?.as_float().ok())
        .collect::<Option<_>>()?;
    numbers.try_into().ok()
}

/// Width and height in points of the unit square drawn with `matrix`.
pub(crate) fn drawn_size(matrix: Matrix) -> (f32, f32) {
    let [a, b, c, d, ..] = matrix;
    (a.hypot(b), c.hypot(d))
}

/// Make the pictures of `doc` lighter: those sharper than `options.dpi` where they are
/// drawn are scaled down, then written as JPEG when that is smaller. Pictures that cannot
/// be decoded safely (masks, CMYK, other bit depths or filters) are kept as they are.
/// Returns how many were changed.
pub(crate) fn downsample(doc: &mut Document, options: ImageOptions) -> usize {
    let target = options.dpi.max(1) as f32;
    let mut sharpness: HashMap<ObjectId, f32> = HashMap::new();
    for page in page_ids(doc) {
        for (id, matrix) in page_images(doc, page) {
            let Some(dpi) = placed_dpi(doc, id, matrix) else {
                continue;
            };
            let entry = sharpness.entry(id).or_insert(dpi);
            *entry = entry.min(dpi);
        }
    }
    let masks = mask_ids(doc);
    let pictures: Vec<ObjectId> = doc
        .objects
        .iter()
        .filter(|(id, object)| !masks.contains(id) && is_image(object))
        .map(|(id, _)| *id)
        .collect();
    let mut changed = 0;
    for id in pictures {
        let Some(Object::Stream(stream)) = doc.objects.get(&id) else {
            continue;
        };
        if has_mask(&stream.dict) {
            continue;
        }
        let Some(decoded) = decode(doc, stream) else {
            continue;
        };
        let (width, height) = (decoded.pixels.width(), decoded.pixels.height());
        let dpi = sharpness
            .get(&id)
            .copied()
            .unwrap_or(width.max(height) as f32 / FALLBACK_INCHES);
        let resized = dpi > target * SLACK;
        let pixels = if resized {
            let scale = target / dpi;
            let w = ((width as f32 * scale).round() as u32).max(1);
            let h = ((height as f32 * scale).round() as u32).max(1);
            decoded.pixels.resize_exact(w, h, FilterType::Triangle)
        } else {
            decoded.pixels
        };
        // Not resampled: a JPEG again only when much smaller. Flate pictures are often
        // drawings, which JPEG blurs: they need a large gain.
        let worth = match (resized, decoded.jpeg) {
            (true, _) => 1.0,
            (false, true) => 0.9,
            (false, false) => 0.5,
        };
        let Some(jpeg) = encode_jpeg(&pixels, options.quality) else {
            continue;
        };
        if (jpeg.len() as f32) >= stream.content.len() as f32 * worth {
            continue;
        }
        let mut dict = stream.dict.clone();
        dict.set("Width", i64::from(pixels.width()));
        dict.set("Height", i64::from(pixels.height()));
        dict.set("BitsPerComponent", 8);
        dict.set("Filter", "DCTDecode");
        dict.remove(b"DecodeParms");
        doc.objects
            .insert(id, Object::Stream(Stream::new(dict, jpeg)));
        changed += 1;
    }
    changed
}

/// Pixels per inch of picture `id` drawn with `matrix`: the lower of both directions.
fn placed_dpi(doc: &Document, id: ObjectId, matrix: Matrix) -> Option<f32> {
    let dict = &doc.get_object(id).ok()?.as_stream().ok()?.dict;
    let width = dict.get(b"Width").ok()?.as_float().ok()?;
    let height = dict.get(b"Height").ok()?.as_float().ok()?;
    let (w, h) = drawn_size(matrix);
    if w < 0.01 || h < 0.01 {
        return None;
    }
    Some((width / (w / 72.0)).min(height / (h / 72.0)))
}

fn is_image(object: &Object) -> bool {
    object.as_stream().is_ok_and(|stream| {
        matches!(
            stream.dict.get(b"Subtype").and_then(Object::as_name),
            Ok(b"Image")
        )
    })
}

fn has_mask(dict: &Dictionary) -> bool {
    dict.has(b"SMask") || dict.has(b"Mask") || dict.has(b"SMaskInData")
}

/// Pictures used as the mask of another one: their size must match it.
fn mask_ids(doc: &Document) -> HashSet<ObjectId> {
    doc.objects
        .values()
        .filter_map(|object| object.as_stream().ok())
        .flat_map(|stream| [b"SMask".as_slice(), b"Mask"].map(|key| stream.dict.get(key)))
        .filter_map(|mask| mask.ok()?.as_reference().ok())
        .collect()
}

/// The pixels of picture `stream` when it is 8-bit grey or RGB, stored as JPEG, Flate or
/// not compressed.
/// Masks are not applied.
pub(crate) fn decode(doc: &Document, stream: &Stream) -> Option<Decoded> {
    let dict = &stream.dict;
    let flag = |key: &[u8]| dict.get(key).and_then(Object::as_bool).unwrap_or(false);
    if flag(b"ImageMask") || dict.has(b"Decode") {
        return None;
    }
    let bits = dict
        .get(b"BitsPerComponent")
        .and_then(Object::as_i64)
        .ok()?;
    let width = u32::try_from(dict.get(b"Width").and_then(Object::as_i64).ok()?).ok()?;
    let height = u32::try_from(dict.get(b"Height").and_then(Object::as_i64).ok()?).ok()?;
    if bits != 8 || width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return None;
    }
    let channels = channels(doc, dict.get(b"ColorSpace").ok()?)?;
    let filter = match dict.get(b"Filter").ok().map(|filter| deref(doc, filter)) {
        None => None,
        Some(Some(Object::Name(name))) => Some(name.as_slice()),
        Some(Some(Object::Array(filters))) if filters.len() == 1 => {
            Some(filters[0].as_name().ok()?)
        }
        _ => return None,
    };
    match filter {
        Some(b"DCTDecode") => {
            let info = jpeg_info(&stream.content)?;
            if info.components != channels || dict.has(b"DecodeParms") {
                return None;
            }
            let pixels =
                image::load_from_memory_with_format(&stream.content, image::ImageFormat::Jpeg)
                    .ok()?;
            let expected = if channels == 1 {
                image::ColorType::L8
            } else {
                image::ColorType::Rgb8
            };
            if pixels.color() != expected || (pixels.width(), pixels.height()) != (width, height) {
                return None;
            }
            Some(Decoded { pixels, jpeg: true })
        }
        // Not compressed, or with Flate.
        None | Some(b"FlateDecode") => {
            let size = width as usize * height as usize * usize::from(channels);
            let mut raw = stream.decompressed_content_with_limit(size + 4096).ok()?;
            if raw.len() < size {
                return None;
            }
            raw.truncate(size);
            let pixels = if channels == 1 {
                DynamicImage::ImageLuma8(image::GrayImage::from_raw(width, height, raw)?)
            } else {
                DynamicImage::ImageRgb8(image::RgbImage::from_raw(width, height, raw)?)
            };
            Some(Decoded {
                pixels,
                jpeg: false,
            })
        }
        _ => None,
    }
}

/// 1 for grey, 3 for RGB (device or ICC based); `None` for any other colour space.
fn channels(doc: &Document, space: &Object) -> Option<u8> {
    match deref(doc, space)? {
        Object::Name(name) if name == b"DeviceRGB" => Some(3),
        Object::Name(name) if name == b"DeviceGray" => Some(1),
        Object::Array(items) if items.first()?.as_name().ok()? == b"ICCBased" => {
            let profile = deref(doc, items.get(1)?)?.as_stream().ok()?;
            match profile.dict.get(b"N").and_then(Object::as_i64).ok()? {
                1 => Some(1),
                3 => Some(3),
                _ => None,
            }
        }
        _ => None,
    }
}

pub(crate) fn encode_jpeg(pixels: &DynamicImage, quality: u8) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100))
        .encode_image(pixels)
        .ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compress_with;
    use lopdf::dictionary;

    /// A page drawing `image` over `size` points square.
    fn document(image: Stream, size: f32) -> Vec<u8> {
        let mut doc = Document::with_version("1.7");
        let pages = doc.new_object_id();
        let image = doc.add_object(image);
        let ops = format!("q {size} 0 0 {size} 0 0 cm /Im0 Do Q");
        let contents = doc.add_object(Stream::new(Dictionary::new(), ops.into_bytes()));
        let page = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages,
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            "Contents" => contents,
            "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => image } },
        });
        doc.objects.insert(
            pages,
            Object::Dictionary(
                dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 },
            ),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", catalog);
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    /// A noisy photo-like RGB picture, Flate compressed (raw would be larger still).
    fn photo(width: u32, height: u32) -> Stream {
        let mut pixels = Vec::new();
        let mut seed = 7u32;
        for y in 0..height {
            for x in 0..width {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                let noise = (seed >> 24) as u8 / 8;
                pixels.extend([(x % 256) as u8 ^ noise, (y % 256) as u8, noise]);
            }
        }
        let mut stream = Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => i64::from(width),
                "Height" => i64::from(height),
                "BitsPerComponent" => 8,
                "ColorSpace" => "DeviceRGB",
            },
            pixels,
        );
        stream.compress().unwrap();
        stream
    }

    fn first_image(pdf: &[u8]) -> Stream {
        let doc = Document::load_mem(pdf).unwrap();
        let page = page_ids(&doc)[0];
        let (id, _) = page_images(&doc, page)[0];
        doc.get_object(id).unwrap().as_stream().unwrap().clone()
    }

    #[test]
    fn a_sharp_picture_gets_smaller() {
        // 1200 pixels over 2 inches: 600 dpi.
        let pdf = document(photo(1200, 1200), 144.0);
        let options = ImageOptions {
            dpi: 150,
            quality: 75,
        };
        let out = compress_with(&pdf, Some(options)).unwrap();
        assert!(out.len() < pdf.len() / 4, "{} -> {}", pdf.len(), out.len());
        let image = first_image(&out);
        assert_eq!(image.dict.get(b"Width").unwrap().as_i64().unwrap(), 300);
        assert_eq!(
            image.dict.get(b"Filter").unwrap().as_name().unwrap(),
            b"DCTDecode"
        );
        // Still a PDF that opens, with its page and a decodable picture.
        let doc = Document::load_mem(&out).unwrap();
        assert_eq!(doc.page_iter().count(), 1);
        assert!(decode(&doc, &image).is_some());
    }

    #[test]
    fn a_picture_that_cannot_be_decoded_is_kept() {
        let mut cmyk = photo(400, 400);
        cmyk.dict.set("ColorSpace", "DeviceCMYK");
        let mut masked = photo(400, 400);
        masked.dict.set("SMask", Object::Reference((99, 0)));
        for stream in [cmyk, masked] {
            let before = stream.content.clone();
            let pdf = document(stream, 20.0);
            let options = ImageOptions {
                dpi: 72,
                quality: 50,
            };
            let out = compress_with(&pdf, Some(options)).unwrap_or(pdf);
            let image = first_image(&out);
            assert_eq!(image.content, before);
            assert_eq!(image.dict.get(b"Width").unwrap().as_i64().unwrap(), 400);
        }
    }

    #[test]
    fn the_largest_placement_sets_the_sharpness() {
        let pdf = document(photo(300, 300), 72.0);
        let doc = Document::load_mem(&pdf).unwrap();
        let page = page_ids(&doc)[0];
        let (id, matrix) = page_images(&doc, page)[0];
        assert_eq!(drawn_size(matrix), (72.0, 72.0));
        assert_eq!(placed_dpi(&doc, id, matrix), Some(300.0));
    }
}
