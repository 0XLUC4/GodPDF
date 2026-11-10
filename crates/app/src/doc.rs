//! One open document (an `Entity<DocTab>`): editing session, parsed renderer, page bitmap
//! cache and view state. Every edit goes through [`DocTab::apply`] (undo history), then the
//! renderer is re-opened from the new bytes ([`DocTab::reload`]).

use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use godpdf_core::{PageGeometry, Session};
use godpdf_render::{Bitmap, Document};
use gpui::{App, Bounds, Context, Global, Pixels, RenderImage, Window};

use crate::geometry::{self, Layout};

pub(crate) const PANIC_MESSAGE: &str = "internal error while processing this PDF (malformed file?)";
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

/// Images of closed documents whose GPU textures are freed on the next window paint
/// (dropping an `Arc<RenderImage>` does not free its sprite-atlas texture).
#[derive(Default)]
struct Released(Vec<Arc<RenderImage>>);

impl Global for Released {}

/// Free the textures of `tab`'s images on the next paint: call before the tab is dropped.
pub fn release(tab: &gpui::Entity<DocTab>, cx: &mut App) {
    let images = tab.update(cx, |tab, _| tab.take_images());
    cx.default_global::<Released>().0.extend(images);
}

/// Free the textures [`release`] collected. Called from the main window's render.
pub fn free_released(window: &mut Window, cx: &mut App) {
    for image in std::mem::take(&mut cx.default_global::<Released>().0) {
        // Fails only for an image never painted: nothing to free then.
        let _ = window.drop_image(image);
    }
}

/// A rendered page plus what it was rendered from (older = stale but still shown).
pub struct CachedPage {
    pub image: Arc<RenderImage>,
    scale: f32,
    generation: u64,
}

/// A page rendered without one of its annotations: shown under that annotation while it is
/// moved or resized (and until the page is rendered again), so only the annotation moves.
struct Lifted {
    page: usize,
    id: String,
    generation: u64,
    scale: f32,
    image: Option<Arc<RenderImage>>,
}

/// A sharp picture of the part on screen of a page too large to be drawn whole at the zoom
/// in use: drawn over the page, which is then as large as it can be.
struct Detail {
    image: Arc<RenderImage>,
    scale: f32,
    generation: u64,
    /// The part of the page it shows (left, top, right, bottom), displayed points.
    region: [f32; 4],
}

/// The part of a page of `size` points to render at `scale` around `visible` (both
/// left, top, right, bottom in displayed points): what shows plus some room around it so a
/// little scrolling needs no new picture, on whole pixels, at most [`MAX_TEXTURE_SIDE`] a side.
pub fn detail_region(visible: [f32; 4], size: (f32, f32), scale: f32) -> [f32; 4] {
    let [left, top, right, bottom] = visible;
    let reach = MAX_TEXTURE_SIDE / scale;
    let grow = |low: f32, high: f32, extent: f32| {
        let room = (reach - (high - low)).max(0.0).min(high - low) / 2.0;
        let low = ((low - room) * scale).floor() / scale;
        let high = ((high + room) * scale).ceil() / scale;
        (low.max(0.0), high.min(extent))
    };
    let (left, right) = grow(left, right, size.0);
    let (top, bottom) = grow(top, bottom, size.1);
    [left, top, right, bottom]
}

fn contains(outer: [f32; 4], inner: [f32; 4]) -> bool {
    outer[0] <= inner[0] && outer[1] <= inner[1] && outer[2] >= inner[2] && outer[3] >= inner[3]
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
    /// The bytes `render` was opened from (shared with background renders).
    bytes: Arc<Vec<u8>>,
    pub render_error: Option<String>,
    /// Bumped on every reload; render results from older generations are stale.
    generation: u64,
    /// Displayed (post-rotation) page sizes in points.
    pub page_sizes: Vec<(f32, f32)>,
    pages: HashMap<usize, CachedPage>,
    page_errors: HashMap<usize, String>,
    in_flight: HashSet<usize>,
    /// Sharp parts of pages too large to render whole, and those being rendered.
    details: HashMap<usize, Detail>,
    details_in_flight: HashSet<usize>,
    /// Images evicted from the cache; their GPU textures are freed on the next paint.
    evicted: Vec<Arc<RenderImage>>,
    geometries: HashMap<usize, PageGeometry>,
    lifted: Option<Lifted>,
    /// The page and annotation the tools want lifted next, and whether one is rendering.
    lift_wanted: Option<(usize, String)>,
    lift_busy: bool,
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
            bytes: Arc::default(),
            render_error: None,
            generation: 0,
            page_sizes: Vec::new(),
            pages: HashMap::new(),
            page_errors: HashMap::new(),
            in_flight: HashSet::new(),
            details: HashMap::new(),
            details_in_flight: HashSet::new(),
            evicted: Vec::new(),
            geometries: HashMap::new(),
            lifted: None,
            lift_wanted: None,
            lift_busy: false,
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
        self.details_in_flight.clear();
        self.page_errors.clear();
        self.geometries.clear();
        // Callers set the selection again after an edit that keeps the pages.
        self.grid.selection.clear();
        let old_count = self.page_count();
        let bytes = Arc::new(self.session.bytes().to_vec());
        self.bytes = Arc::clone(&bytes);
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
            self.evicted
                .extend(self.details.drain().map(|(_, detail)| detail.image));
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

    /// Rotation and origin of `page`, every page parsed at once per document revision.
    pub fn page_geom(&mut self, page: usize) -> Result<PageGeometry, String> {
        if self.geometries.is_empty() {
            let all = guarded(|| godpdf_core::page_geometries(self.session.bytes()))
                .map_err(|err| err.to_string())?;
            self.geometries = all.into_iter().enumerate().collect();
        }
        self.geometries
            .get(&page)
            .copied()
            .ok_or_else(|| format!("no page {}", page + 1))
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

    /// The bitmap shown for `page`, fresh or not, without asking for a render.
    pub fn cached_page(&self, page: usize) -> Option<Arc<RenderImage>> {
        self.pages
            .get(&page)
            .map(|cached| Arc::clone(&cached.image))
    }

    /// `page` shows the current document (or cannot be rendered at all).
    pub fn page_settled(&self, page: usize) -> bool {
        self.page_errors.contains_key(&page)
            || self
                .pages
                .get(&page)
                .is_some_and(|cached| cached.generation == self.generation)
    }

    /// Have `page` rendered without the annotation `id` in the background (see
    /// [`Self::lifted`]). Cheap to call on every pointer move: it renders only when the
    /// target, the document or the zoom changed, one at a time.
    pub fn lift(&mut self, page: usize, id: &str, cx: &mut Context<Self>) {
        self.lift_wanted = Some((page, id.to_owned()));
        self.pump_lift(cx);
    }

    /// A lift of the current document is rendered or rendering (whatever its target).
    pub fn lift_current(&self) -> bool {
        self.lifted
            .as_ref()
            .is_some_and(|lifted| lifted.generation == self.generation)
    }

    /// Every image this tab holds (pages, details, lifted page, picture, thumbnails).
    fn take_images(&mut self) -> Vec<Arc<RenderImage>> {
        let mut images: Vec<_> = self.pages.drain().map(|(_, cached)| cached.image).collect();
        images.extend(self.details.drain().map(|(_, detail)| detail.image));
        images.append(&mut self.evicted);
        images.extend(self.lifted.take().and_then(|lifted| lifted.image));
        images.extend(self.canvas.take_picture());
        images.extend(self.panels.thumbs.take_images());
        images.extend(self.grid.thumbs.take_images());
        images
    }

    /// Free the GPU texture of `image` on the next paint.
    pub fn evict(&mut self, image: Arc<RenderImage>) {
        self.evicted.push(image);
    }

    /// `page` without the annotation `id`, once [`Self::lift`] has rendered it.
    pub fn lifted(&self, page: usize, id: &str) -> Option<Arc<RenderImage>> {
        let lifted = self.lifted.as_ref()?;
        (lifted.page == page && lifted.id == id)
            .then(|| lifted.image.clone())
            .flatten()
    }

    fn pump_lift(&mut self, cx: &mut Context<Self>) {
        let Some((page, id)) = self.lift_wanted.clone() else {
            return;
        };
        let Some(scale) = self.pages.get(&page).map(|cached| cached.scale) else {
            return;
        };
        let current = self.lifted.as_ref().is_some_and(|lifted| {
            lifted.page == page
                && lifted.id == id
                && lifted.generation == self.generation
                && (lifted.scale / scale - 1.0).abs() <= SCALE_TOLERANCE
        });
        if current || self.lift_busy {
            return;
        }
        self.lift_busy = true;
        let generation = self.generation;
        if let Some(old) = self.lifted.take().and_then(|lifted| lifted.image) {
            self.evicted.push(old);
        }
        self.lifted = Some(Lifted {
            page,
            id: id.clone(),
            generation,
            scale,
            image: None,
        });
        let bytes = Arc::clone(&self.bytes);
        let wanted = id.clone();
        let rendered = cx.background_executor().spawn(async move {
            catch_unwind(AssertUnwindSafe(|| {
                let pdf = godpdf_core::remove_annotation(&bytes, page, &wanted)
                    .map_err(|err| err.to_string())?;
                let doc = Document::open(Arc::new(pdf), None).map_err(|err| err.to_string())?;
                doc.render(page, scale)
                    .map_err(|err| err.to_string())
                    .and_then(to_render_image)
            }))
            .unwrap_or_else(|_| Err(PANIC_MESSAGE.to_owned()))
        });
        cx.spawn(async move |this, cx| {
            let result = rendered.await;
            let _ = this.update(cx, |tab, cx| {
                tab.lift_busy = false;
                // Without it the annotation simply shows under its stand-in: nothing to say.
                if let (Ok(image), Some(lifted)) = (result, tab.lifted.as_mut())
                    && (lifted.page, &lifted.id, lifted.generation) == (page, &id, generation)
                {
                    lifted.image = Some(Arc::new(image));
                }
                tab.pump_lift(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// The sharp part of `page` on screen when it is too large to be drawn whole at `scale`:
    /// its image and the part of the page it shows. `visible` (left, top, right, bottom in
    /// displayed points) is what shows. May be stale, as page images.
    pub fn page_detail(
        &mut self,
        page: usize,
        scale: f32,
        visible: [f32; 4],
        cx: &mut Context<Self>,
    ) -> Option<(Arc<RenderImage>, [f32; 4])> {
        let &(width, height) = self.page_sizes.get(page)?;
        let whole = MAX_TEXTURE_SIDE / width.max(height).max(1.0);
        let shows = visible[2] > visible[0] && visible[3] > visible[1];
        if scale <= whole * (1.0 + SCALE_TOLERANCE) || !shows {
            if let Some(detail) = self.details.remove(&page) {
                self.evicted.push(detail.image);
            }
            return None;
        }
        let wanted = detail_region(visible, (width, height), scale);
        let fresh = self.details.get(&page).is_some_and(|detail| {
            detail.generation == self.generation
                && (detail.scale / scale - 1.0).abs() <= SCALE_TOLERANCE
                && contains(detail.region, visible)
        });
        if !fresh {
            self.request_detail(page, scale, wanted, cx);
        }
        self.details
            .get(&page)
            .map(|detail| (Arc::clone(&detail.image), detail.region))
    }

    fn request_detail(
        &mut self,
        page: usize,
        scale: f32,
        region: [f32; 4],
        cx: &mut Context<Self>,
    ) {
        let Some(doc) = self.render.clone() else {
            return;
        };
        // One at a time per page; once it is in, a view that moved on asks again.
        if !self.details_in_flight.insert(page) {
            return;
        }
        let generation = self.generation;
        let rendered = cx.background_executor().spawn(async move {
            catch_unwind(AssertUnwindSafe(|| doc.render_region(page, scale, region)))
                .unwrap_or_else(|_| Err(godpdf_render::Error::Parse(PANIC_MESSAGE.to_owned())))
                .map_err(|err| err.to_string())
                .and_then(to_render_image)
        });
        cx.spawn(async move |this, cx| {
            let result = rendered.await;
            let _ = this.update(cx, |tab, cx| {
                if tab.generation != generation {
                    return;
                }
                tab.details_in_flight.remove(&page);
                // A failed part leaves the whole page shown, less sharp: nothing to say.
                if let Ok(image) = result {
                    let detail = Detail {
                        image: Arc::new(image),
                        scale,
                        generation,
                        region,
                    };
                    if let Some(old) = tab.details.insert(page, detail) {
                        tab.evicted.push(old.image);
                    }
                }
                cx.notify();
            });
        })
        .detach();
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
        let gone: Vec<usize> = self
            .details
            .keys()
            .copied()
            .filter(|page| !visible.contains(page))
            .collect();
        for page in gone {
            if let Some(detail) = self.details.remove(&page) {
                self.evicted.push(detail.image);
            }
        }
        // The page without a lifted annotation, once off screen or once its page shows the
        // newer document (a drop has settled).
        let lift_done = self.lifted.as_ref().is_some_and(|lifted| {
            !keep.contains(&lifted.page)
                || (lifted.generation != self.generation && self.page_settled(lifted.page))
        });
        if lift_done {
            self.lift_wanted = None;
            if let Some(image) = self.lifted.take().and_then(|lifted| lifted.image) {
                self.evicted.push(image);
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
    fn detail_covers_what_shows_with_room_on_whole_pixels() {
        // A0 at 4 px per point: 2384 x 3370 points, only 400 x 300 show.
        let region = detail_region([1000.0, 500.0, 1400.0, 800.0], (2384.0, 3370.0), 4.0);
        assert!(contains(region, [1000.0, 500.0, 1400.0, 800.0]));
        assert_eq!(region, [800.0, 350.0, 1600.0, 950.0]);
        assert!((region[2] - region[0]) * 4.0 <= MAX_TEXTURE_SIDE);
        // At the page corner it stops at the sheet.
        let corner = detail_region([0.0, 0.0, 100.0, 100.0], (2384.0, 3370.0), 4.0);
        assert_eq!(corner, [0.0, 0.0, 150.0, 150.0]);
    }

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
