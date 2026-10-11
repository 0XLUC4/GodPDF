//! Page rasterization for GodPDF, backed by `hayro`. Page indices are 0-based.

use std::sync::Arc;

use hayro::hayro_interpret::font::{Glyph, GlyphRun, PositionedGlyph};
use hayro::hayro_interpret::hayro_cmap::BfString;
use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, InterpreterSettings, SoftMask, interpret_page,
};
use hayro::hayro_syntax::page::Page;
use hayro::hayro_syntax::{DecryptionError, LoadPdfError, Pdf};
use hayro::kurbo::{Affine, BezPath, Point, Rect, Shape};
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::vello_cpu::peniko::ImageAlphaType;
use hayro::vello_cpu::{Pixmap, RasterizerSettings, RenderContext, Resources, TargetInit};
use hayro::{RenderCache, RenderSettings, render_into};

/// Errors from opening or rendering.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid PDF: {0}")]
    Parse(String),
    /// Encrypted and no / wrong password given.
    #[error("password required")]
    PasswordRequired,
    #[error("page {page} out of range (document has {count} pages)")]
    PageOutOfRange { page: usize, count: usize },
}

pub type Result<T> = std::result::Result<T, Error>;

/// Straight (non-premultiplied) RGBA8, row-major, `width * height * 4` bytes.
#[derive(Debug, Clone)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Page size in points **after** applying /Rotate (as displayed).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSize {
    pub width: f32,
    pub height: f32,
}

/// Scale is clamped to this range (1.0 = 72 dpi).
const MIN_SCALE: f32 = 0.05;
const MAX_SCALE: f32 = 8.0;
/// Upper bound on output pixels (~160 MB of RGBA) to avoid OOM.
const MAX_PIXELS: f32 = 40_000_000.0;
/// Upper bound on a single side (vello_cpu uses u16 dimensions).
const MAX_SIDE: f32 = 16_384.0;

/// A parsed document ready to render. Re-open after every edit.
///
/// `Document` is `Send + Sync`: it can be moved to (or shared with) a background
/// render thread. Each render call builds its own caches, so calls are independent.
pub struct Document {
    pdf: Pdf,
}

impl Document {
    /// Parse `bytes`. `password` is used only if the document is encrypted.
    pub fn open(bytes: Arc<Vec<u8>>, password: Option<&str>) -> Result<Self> {
        match Pdf::new_with_password(bytes, password.unwrap_or("")) {
            Ok(pdf) => Ok(Self { pdf }),
            Err(LoadPdfError::Decryption(DecryptionError::PasswordProtected)) => {
                Err(Error::PasswordRequired)
            }
            Err(LoadPdfError::Decryption(e)) => Err(Error::Parse(format!("decryption: {e:?}"))),
            Err(LoadPdfError::Invalid) => Err(Error::Parse("unreadable PDF structure".into())),
        }
    }

    pub fn page_count(&self) -> usize {
        self.pdf.pages().len()
    }

    pub fn page_size(&self, page: usize) -> Result<PageSize> {
        let (width, height) = self.page(page)?.render_dimensions();
        Ok(PageSize { width, height })
    }

    /// Render at `scale` (1.0 = 72 dpi) on a white background.
    ///
    /// `scale` is clamped to `0.05..=8.0` and reduced further if the output would exceed
    /// 40 megapixels or 16384 px on a side.
    pub fn render(&self, page: usize, scale: f32) -> Result<Bitmap> {
        let page = self.page(page)?;
        let (page_w, page_h) = page.render_dimensions();
        let scale = effective_scale(page_w, page_h, scale);
        let width = ((page_w * scale).round() as u16).max(1);
        let height = ((page_h * scale).round() as u16).max(1);

        let cache = RenderCache::new();
        let mut ctx = RenderContext::new(width, height);
        let transform = Affine::scale(f64::from(scale)) * page.initial_transform(true).to_kurbo();
        render_into(
            page,
            &cache,
            &InterpreterSettings::default(),
            &RenderSettings::default(),
            &mut ctx,
            transform,
        );
        ctx.flush();

        let mut pixmap = Pixmap::new(width, height);
        ctx.render_with(
            &mut pixmap,
            &mut Resources::default(),
            RasterizerSettings {
                target_init: TargetInit::Clear(WHITE),
                ..Default::default()
            },
        );
        Ok(Bitmap {
            width: u32::from(width),
            height: u32::from(height),
            // vello_cpu is premultiplied; ask for straight alpha.
            rgba: pixmap.take_rgba8(ImageAlphaType::Alpha),
        })
    }

    /// Render so the longest side is `max_px` pixels (thumbnails).
    pub fn thumbnail(&self, page: usize, max_px: u32) -> Result<Bitmap> {
        let size = self.page_size(page)?;
        let longest = size.width.max(size.height).max(1.0);
        self.render(page, max_px as f32 / longest)
    }

    fn page(&self, page: usize) -> Result<&Page<'_>> {
        let pages = self.pdf.pages();
        pages.get(page).ok_or(Error::PageOutOfRange {
            page,
            count: pages.len(),
        })
    }
}

/// Clamp `scale` to the sane range, then shrink it to respect pixel and side limits.
fn effective_scale(page_w: f32, page_h: f32, scale: f32) -> f32 {
    let scale = if scale.is_finite() { scale } else { 1.0 };
    let mut scale = scale.clamp(MIN_SCALE, MAX_SCALE);
    let (w, h) = (page_w.max(1.0), page_h.max(1.0));
    let pixels = w * h * scale * scale;
    if pixels > MAX_PIXELS {
        scale *= (MAX_PIXELS / pixels).sqrt();
    }
    scale.min(MAX_SIDE / w.max(h))
}

/// A piece of page text (usually one character; ligatures give several) and where it is drawn.
///
/// Coordinates are displayed page points: origin top-left, after /Rotate — the same space as
/// [`PageSize`] and a bitmap rendered at scale 1.0.
#[derive(Debug, Clone, PartialEq)]
pub struct TextChar {
    pub text: String,
    /// Left edge of the glyph cell.
    pub x0: f32,
    /// Top of the line box (baseline minus ascent).
    pub y0: f32,
    /// Right edge: left edge plus advance width.
    pub x1: f32,
    /// Bottom of the line box (baseline plus descent).
    pub y1: f32,
}

impl Document {
    /// Text of `page` in content-stream order, one entry per drawn glyph that maps to Unicode.
    /// Invisible text (render mode 3, e.g. OCR layers) is included; annotations are not.
    pub fn text_chars(&self, page: usize) -> Result<Vec<TextChar>> {
        let page = self.page(page)?;
        let (page_w, page_h) = page.render_dimensions();
        let cache = InterpreterCache::new();
        let settings = InterpreterSettings {
            render_annotations: false,
            ..Default::default()
        };
        let mut context = Context::new(
            page.initial_transform(true).to_kurbo(),
            Rect::new(0.0, 0.0, f64::from(page_w), f64::from(page_h)),
            &cache,
            page.xref(),
            settings,
        );
        let mut collector = TextCollector::default();
        interpret_page(page, &mut context, &mut collector);
        Ok(collector.chars)
    }
}

/// Glyph space is 1000 units per em (hayro normalizes every font to this).
const EM: f64 = 1000.0;
/// Line box used for every font: hayro does not expose real ascent/descent.
const ASCENT: f64 = 0.8 * EM;
const DESCENT: f64 = -0.2 * EM;
/// Cell width when a glyph has neither an advance width nor an outline.
const FALLBACK_ADVANCE: f64 = 0.5 * EM;

/// A [`Device`] that records glyph cells and ignores all other drawing.
#[derive(Default)]
struct TextCollector {
    chars: Vec<TextChar>,
    /// First glyph transform and glyph count of the last filled run, to drop the stroke pass
    /// that hayro emits for the same glyphs in fill+stroke render modes.
    last_fill: Option<(Affine, usize)>,
}

impl<'a> Device<'a> for TextCollector {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, mode: &DrawMode) {
        let glyphs = run.glyphs();
        let Some(first) = glyphs.first() else { return };
        let key = (props.transform * first.transform(), glyphs.len());
        match mode {
            DrawMode::Stroke(_) if self.last_fill == Some(key) => {
                self.last_fill = None;
                return;
            }
            DrawMode::Fill(_) => self.last_fill = Some(key),
            _ => self.last_fill = None,
        }

        for (index, glyph) in glyphs.iter().enumerate() {
            let text = match glyph.as_unicode() {
                Some(BfString::Char(c)) => c.to_string(),
                Some(BfString::String(s)) => s,
                None => continue,
            };
            if text.is_empty() {
                continue;
            }
            // Glyph space (1000 upem, y up, origin on the baseline) -> displayed page points.
            let to_page = props.transform * glyph.transform();
            let advance = advance_width(glyph, glyphs.get(index + 1));
            let cell = to_page.transform_rect_bbox(Rect::new(0.0, DESCENT, advance, ASCENT));
            if cell.is_finite() {
                self.chars.push(TextChar {
                    text,
                    x0: cell.x0 as f32,
                    y0: cell.y0 as f32,
                    x1: cell.x1 as f32,
                    y1: cell.y1 as f32,
                });
            }
        }
    }
}

/// Horizontal advance of `glyph` in glyph space.
fn advance_width(glyph: &PositionedGlyph<'_>, next: Option<&PositionedGlyph<'_>>) -> f64 {
    let known = match &**glyph {
        Glyph::Outline(outline) => outline
            .advance_width()
            .map(f64::from)
            .or_else(|| Some(outline.outline().bounding_box().x1)),
        // Type3 widths are not exposed: measure up to the next glyph of the run instead.
        Glyph::Type3(_) => next.map(|next| {
            let origin = glyph.transform().inverse() * (next.transform() * Point::ZERO);
            if origin.y.abs() < 1.0 { origin.x } else { 0.0 }
        }),
    };
    known
        .filter(|width| width.is_finite() && *width > 0.0 && *width <= 4.0 * EM)
        .unwrap_or(FALLBACK_ADVANCE)
}
