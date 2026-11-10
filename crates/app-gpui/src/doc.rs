//! One open document (an `Entity<DocTab>`): editing session, parsed renderer, page bitmap
//! cache and view state. Every edit goes through [`DocTab::apply`] (undo history), then the
//! renderer is re-opened from the new bytes ([`DocTab::reload`]).

use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use godpdf_core::{PageGeometry, Session};
use godpdf_render::{Bitmap, Document};
use gpui::{Bounds, Context, Pixels, RenderImage, Window};

use crate::geometry::{self, Layout};

const PANIC_MESSAGE: &str = "internal error while processing this PDF (malformed file?)";
/// A cached page is re-rendered when its scale is off by more than this ratio.
const SCALE_TOLERANCE: f32 = 0.005;
/// Keep page bitmaps under typical GPU texture limits.
const MAX_TEXTURE_SIDE: f32 = 4096.0;
/// Pages rendering at once per tab (keeps the work close to what is on screen).
const MAX_IN_FLIGHT: usize = 4;
/// Pages kept rendered beyond those on screen, on each side.
const KEEP_AROUND: usize = 3;

/// Run a core call, turning a panic (library bug on a malformed PDF) into an error.
pub fn guarded<T>(work: impl FnOnce() -> godpdf_core::Result<T>) -> godpdf_core::Result<T> {
    catch_unwind(AssertUnwindSafe(work))
        .unwrap_or_else(|_| Err(godpdf_core::Error::Pdf(PANIC_MESSAGE.to_owned())))
}

/// A rendered page plus what it was rendered from (older = stale but still shown).
pub struct CachedPage {
    pub image: Arc<RenderImage>,
    scale: f32,
    generation: u64,
}

/// Where the viewer is in the document. Sizes in logical pixels.
pub struct ViewState {
    pub zoom: f32,
    /// Scroll offset (x, y) of the viewer content.
    pub scroll: (f32, f32),
    /// Viewer bounds in the window, as of the last paint.
    pub bounds: Bounds<Pixels>,
    /// Fit width once the viewer size is known.
    pub fit_pending: bool,
    pub current_page: usize,
}

pub struct DocTab {
    pub session: Session,
    render: Option<Arc<Document>>,
    pub render_error: Option<String>,
    /// Bumped on every reload; render results from older generations are stale.
    generation: u64,
    /// Displayed (post-rotation) page sizes in points.
    pub page_sizes: Vec<(f32, f32)>,
    pages: HashMap<usize, CachedPage>,
    page_errors: HashMap<usize, String>,
    in_flight: HashSet<usize>,
    /// Images evicted from the cache; their GPU textures are freed on the next paint.
    evicted: Vec<Arc<RenderImage>>,
    geometries: HashMap<usize, PageGeometry>,
    pub view: ViewState,
    /// Viewer scrollbar drag in progress.
    pub thumb_drag: Option<crate::viewer::ThumbDrag>,
    /// Canvas tools state: selection, gesture, text being typed.
    pub canvas: crate::tools::Canvas,
    /// Sidebar state of this document: thumbnails, search hits, bookmark being renamed.
    pub panels: crate::panels::TabPanels,
    /// Page text: positions, selection, line being edited.
    pub text: crate::text::TabText,
    /// Organize page grid: shown or not, selected pages, its thumbnails.
    pub grid: crate::organize::Grid,
    /// The second viewer of "Side by side": for reading only, no tools on it.
    pub companion: bool,
}

impl DocTab {
    pub fn new(session: Session) -> Self {
        let mut tab = Self {
            session,
            render: None,
            render_error: None,
            generation: 0,
            page_sizes: Vec::new(),
            pages: HashMap::new(),
            page_errors: HashMap::new(),
            in_flight: HashSet::new(),
            evicted: Vec::new(),
            geometries: HashMap::new(),
            thumb_drag: None,
            canvas: Default::default(),
            panels: Default::default(),
            text: Default::default(),
            grid: Default::default(),
            companion: false,
            view: ViewState {
                zoom: 1.0,
                scroll: (0.0, 0.0),
                bounds: Bounds::default(),
                fit_pending: true,
                current_page: 0,
            },
        };
        tab.reload();
        tab
    }

    pub fn title(&self) -> &str {
        &self.session.name
    }

    pub fn is_dirty(&self) -> bool {
        self.session.is_dirty()
    }

    /// Bumped on every reload: caches of what the bytes contain key on it.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn page_count(&self) -> usize {
        self.page_sizes.len()
    }

    /// Re-parse after the bytes changed. Old bitmaps stay visible as stale previews while
    /// the page count is unchanged.
    pub fn reload(&mut self) {
        self.generation += 1;
        self.in_flight.clear();
        self.page_errors.clear();
        self.geometries.clear();
        // egui: callers set the selection again after an edit that keeps the pages.
        self.grid.selection.clear();
        let old_count = self.page_count();
        let bytes = Arc::new(self.session.bytes().to_vec());
        let opened = catch_unwind(AssertUnwindSafe(|| {
            let doc = Document::open(bytes, None).map_err(|err| err.to_string())?;
            let sizes = (0..doc.page_count())
                .map(|page| {
                    doc.page_size(page)
                        .map_or((612.0, 792.0), |size| (size.width, size.height))
                })
                .collect::<Vec<_>>();
            Ok::<_, String>((doc, sizes))
        }))
        .unwrap_or_else(|_| Err(PANIC_MESSAGE.to_owned()));
        match opened {
            Ok((doc, sizes)) => {
                self.page_sizes = sizes;
                self.render = Some(Arc::new(doc));
                self.render_error = None;
            }
            Err(err) => {
                self.page_sizes.clear();
                self.render = None;
                self.render_error = Some(err);
            }
        }
        if self.page_count() != old_count {
            self.evicted
                .extend(self.pages.drain().map(|(_, cached)| cached.image));
        }
        self.view.current_page = self
            .view
            .current_page
            .min(self.page_count().saturating_sub(1));
    }

    /// Run a core edit through the session (undo history), then reload.
    pub fn apply(
        &mut self,
        label: &str,
        edit: impl FnOnce(&[u8]) -> godpdf_core::Result<Vec<u8>>,
    ) -> Result<(), String> {
        self.session
            .apply(label, |pdf| guarded(|| edit(pdf)))
            .map_err(|err| err.to_string())?;
        self.reload();
        Ok(())
    }

    /// Like [`Self::apply`] but folded into the latest undo step.
    pub fn amend(
        &mut self,
        label: &str,
        edit: impl FnOnce(&[u8]) -> godpdf_core::Result<Vec<u8>>,
    ) -> Result<(), String> {
        self.session
            .amend(label, |pdf| guarded(|| edit(pdf)))
            .map_err(|err| err.to_string())?;
        self.reload();
        Ok(())
    }

    /// Returns the label of the undone step, `None` when there was nothing to undo.
    pub fn undo(&mut self) -> Option<String> {
        let label = self.session.undo_label()?.to_owned();
        self.session.undo().then(|| {
            self.reload();
            label
        })
    }

    pub fn redo(&mut self) -> Option<String> {
        let label = self.session.redo_label()?.to_owned();
        self.session.redo().then(|| {
            self.reload();
            label
        })
    }

    /// The parsed document, `None` when it cannot be displayed.
    pub fn renderer(&self) -> Option<Arc<Document>> {
        self.render.clone()
    }

    /// Rotation and origin of `page`, parsed once per document revision.
    pub fn page_geom(&mut self, page: usize) -> Result<PageGeometry, String> {
        if let Some(geom) = self.geometries.get(&page) {
            return Ok(*geom);
        }
        let geom = guarded(|| godpdf_core::page_geometry(self.session.bytes(), page))
            .map_err(|err| err.to_string())?;
        self.geometries.insert(page, geom);
        Ok(geom)
    }

    /// Page layout at the current zoom and viewer width.
    pub fn layout(&self) -> Layout {
        Layout::new(
            &self.page_sizes,
            self.view.zoom,
            f32::from(self.view.bounds.size.width),
        )
    }

    fn view_size(&self) -> (f32, f32) {
        let size = self.view.bounds.size;
        (f32::from(size.width), f32::from(size.height))
    }

    /// Scroll by `delta` (logical pixels), clamped to the content.
    pub fn scroll_by(&mut self, delta: (f32, f32)) {
        let scroll = (self.view.scroll.0 + delta.0, self.view.scroll.1 + delta.1);
        self.set_scroll(&self.layout(), scroll);
    }

    pub fn scroll_to_page(&mut self, page: usize) {
        let layout = self.layout();
        if let Some(y) = layout.scroll_to_page(page) {
            self.set_scroll(&layout, (self.view.scroll.0, y));
        }
    }

    fn set_scroll(&mut self, layout: &Layout, scroll: (f32, f32)) {
        let view = self.view_size();
        let max = layout.max_scroll(view);
        self.view.scroll = (scroll.0.clamp(0.0, max.0), scroll.1.clamp(0.0, max.1));
        self.view.current_page = layout.current_page(self.view.scroll.1, view.1);
    }

    /// Zoom to `zoom`, keeping the content under `anchor` (view space; `None` = view center).
    pub fn set_zoom(&mut self, zoom: f32, anchor: Option<(f32, f32)>) {
        let zoom = zoom.clamp(geometry::MIN_ZOOM, geometry::MAX_ZOOM);
        if (zoom - self.view.zoom).abs() < f32::EPSILON {
            return;
        }
        let view = self.view_size();
        let anchor = anchor.unwrap_or((view.0 / 2.0, view.1 / 2.0));
        let old = self.layout();
        self.view.zoom = zoom;
        let new = self.layout();
        let scroll = geometry::anchored_scroll(&old, &new, self.view.scroll, anchor);
        self.set_scroll(&new, scroll);
    }

    pub fn fit_width(&mut self) {
        let width = f32::from(self.view.bounds.size.width);
        if width > 0.0 && !self.page_sizes.is_empty() {
            let page = self.view.current_page;
            self.set_zoom(geometry::fit_width_zoom(&self.page_sizes, width), None);
            self.scroll_to_page(page);
        }
    }

    /// Zoom so the current page fits whole in the view, and show it.
    pub fn fit_page(&mut self) {
        let size = self.view_size();
        let page = self.view.current_page;
        if let Some(&(width, height)) = self.page_sizes.get(page) {
            let zoom = geometry::fit_page_zoom((width, height), size);
            self.set_zoom(zoom, None);
            self.scroll_to_page(page);
        }
    }

    pub fn page_error(&self, page: usize) -> Option<&str> {
        self.page_errors.get(&page).map(String::as_str)
    }

    /// Bitmap for `page`, rendering it at `scale` (pixels per point) when the cached one is
    /// missing, stale or at another scale. May return the stale one meanwhile.
    pub fn page_image(
        &mut self,
        page: usize,
        scale: f32,
        cx: &mut Context<Self>,
    ) -> Option<Arc<RenderImage>> {
        let &(width, height) = self.page_sizes.get(page)?;
        let scale = scale.min(MAX_TEXTURE_SIDE / width.max(height).max(1.0));
        let fresh = self.pages.get(&page).is_some_and(|cached| {
            cached.generation == self.generation
                && (cached.scale / scale - 1.0).abs() <= SCALE_TOLERANCE
        });
        if !fresh && !self.page_errors.contains_key(&page) {
            self.request(page, scale, cx);
        }
        self.pages
            .get(&page)
            .map(|cached| Arc::clone(&cached.image))
    }

    fn request(&mut self, page: usize, scale: f32, cx: &mut Context<Self>) {
        let Some(doc) = self.render.clone() else {
            return;
        };
        if self.in_flight.len() >= MAX_IN_FLIGHT || !self.in_flight.insert(page) {
            return;
        }
        let generation = self.generation;
        let rendered = cx.background_executor().spawn(async move {
            catch_unwind(AssertUnwindSafe(|| doc.render(page, scale)))
                .unwrap_or_else(|_| Err(godpdf_render::Error::Parse(PANIC_MESSAGE.to_owned())))
                .map_err(|err| err.to_string())
                .and_then(to_render_image)
        });
        cx.spawn(async move |this, cx| {
            let result = rendered.await;
            // The tab may be closed meanwhile: nothing to do then.
            let _ = this.update(cx, |tab, cx| {
                if tab.generation != generation {
                    return;
                }
                tab.in_flight.remove(&page);
                match result {
                    Ok(image) => {
                        let cached = CachedPage {
                            image: Arc::new(image),
                            scale,
                            generation,
                        };
                        if let Some(old) = tab.pages.insert(page, cached) {
                            tab.evicted.push(old.image);
                        }
                    }
                    Err(err) => {
                        tab.page_errors.insert(page, err);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Drop bitmaps of pages far from `visible`, and free evicted GPU textures.
    pub fn trim_cache(&mut self, visible: std::ops::Range<usize>, window: &mut Window) {
        let keep = visible.start.saturating_sub(KEEP_AROUND)..visible.end + KEEP_AROUND;
        let far: Vec<usize> = self
            .pages
            .keys()
            .copied()
            .filter(|page| !keep.contains(page))
            .collect();
        for page in far {
            if let Some(cached) = self.pages.remove(&page) {
                self.evicted.push(cached.image);
            }
        }
        for image in self.evicted.drain(..) {
            // Fails only for an image never painted: nothing to free then.
            let _ = window.drop_image(image);
        }
    }
}

/// hayro gives straight RGBA; gpui wants BGRA.
pub fn to_render_image(bitmap: Bitmap) -> Result<RenderImage, String> {
    let Bitmap {
        width,
        height,
        mut rgba,
    } = bitmap;
    for pixel in rgba.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let buffer = image::RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| "rendered page has a wrong size".to_owned())?;
    Ok(RenderImage::new(vec![image::Frame::new(buffer)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swaps_red_and_blue() {
        let bitmap = Bitmap {
            width: 1,
            height: 1,
            rgba: vec![1, 2, 3, 4],
        };
        let image = to_render_image(bitmap).unwrap();
        assert_eq!(image.as_bytes(0), Some(&[3, 2, 1, 4][..]));
    }
}
