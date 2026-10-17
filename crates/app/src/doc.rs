//! One open document tab: editing session + render state + view state.
//!
//! Native: pages and thumbnails are rasterized on a per-tab worker thread; the UI uploads
//! textures when results arrive. Web: rendered inline under a per-frame [`Budget`].

use std::collections::{BTreeSet, HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use egui::{ColorImage, Context, TextureHandle, TextureOptions, Vec2, vec2};
use godpdf_core::{OutlineItem, PageGeometry, PlacedAnnotation, SearchHit, Session};
use godpdf_render::{Bitmap, Document, TextChar};

pub const MIN_ZOOM: f32 = 0.1;
pub const MAX_ZOOM: f32 = 6.0;
/// Longest side of thumbnails in points; rendered at this times pixels-per-point.
pub const THUMB_PX: u32 = 220;
/// A cached page texture is re-rendered when its scale is off by more than this ratio.
const SCALE_TOLERANCE: f32 = 0.005;
/// Keep page bitmaps under typical GPU texture limits.
const MAX_TEXTURE_SIDE: f32 = 4096.0;
/// Render requests queued per tab at once (keeps the worker close to what is on screen).
const MAX_IN_FLIGHT: usize = 4;
const PANIC_MESSAGE: &str = "internal error while processing this PDF (malformed file?)";

/// Run a core call, turning a panic (library bug on a malformed PDF) into an error.
pub fn guarded<T>(work: impl FnOnce() -> godpdf_core::Result<T>) -> godpdf_core::Result<T> {
    catch_unwind(AssertUnwindSafe(work))
        .unwrap_or_else(|_| Err(godpdf_core::Error::Pdf(PANIC_MESSAGE.to_owned())))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Read,
    Organize,
}

#[derive(Default)]
pub struct SearchState {
    pub query: String,
    /// Query the hits belong to.
    pub searched: String,
    pub hits: Vec<SearchHit>,
    pub error: Option<String>,
    /// Query the sidebar wants searched; the app starts the job.
    pub request: Option<String>,
    /// A search job is running.
    pub busy: bool,
    /// The search panel is on screen.
    pub open: bool,
}

impl SearchState {
    /// The words to mark on the pages: those searched, for as long as the search panel
    /// shows and its field still reads them.
    pub fn marked(&self) -> Option<&str> {
        let current = !self.searched.is_empty() && self.query.trim() == self.searched;
        (self.open && current).then_some(&self.searched)
    }
}

/// An in-progress comment gesture, in displayed page points (origin top-left).
pub struct Gesture {
    pub page: usize,
    pub points: Vec<egui::Pos2>,
}

/// A texture plus the document generation it was rendered from (older = stale but shown).
struct Cached {
    texture: TextureHandle,
    scale: f32,
    generation: u64,
}

/// A sharp picture of the part on screen of a page too large to be drawn whole at the zoom
/// in use. It is shown over the page, which is then drawn as large as it can be.
struct Detail {
    texture: TextureHandle,
    scale: f32,
    generation: u64,
    /// The part of the page it shows, in displayed points.
    region: egui::Rect,
}

/// Per-frame render allowance so a frame never starts too many renders.
pub struct Budget(pub u32);

impl Budget {
    fn take(&mut self) -> bool {
        if self.0 == 0 {
            return false;
        }
        self.0 -= 1;
        true
    }

    pub fn exhausted(&self) -> bool {
        self.0 == 0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Page,
    Thumb,
    /// A page without one of its annotations (see [`Lifted`]).
    Lifted,
    /// A part of a page (see [`Detail`]), for the first or the second viewer.
    Detail {
        second: bool,
    },
}

enum Source {
    Doc(Arc<Document>),
    /// The document `bytes` once the annotation `id` is removed from the requested page.
    Without {
        bytes: Arc<Vec<u8>>,
        id: String,
    },
}

struct Request {
    source: Source,
    /// Document generation; for [`Kind::Lifted`] the serial of the lift.
    generation: u64,
    page: usize,
    kind: Kind,
    scale: f32,
    /// For [`Kind::Detail`]: the part of the page to draw (left, top, right, bottom).
    region: Option<[f32; 4]>,
}

struct Rendered {
    generation: u64,
    page: usize,
    kind: Kind,
    scale: f32,
    region: Option<[f32; 4]>,
    result: Result<Bitmap, String>,
}

impl Request {
    fn run(self) -> Rendered {
        let Self {
            source,
            generation,
            page,
            kind,
            scale,
            region,
        } = self;
        let result = catch_unwind(AssertUnwindSafe(|| {
            let doc = match source {
                Source::Doc(doc) => doc,
                Source::Without { bytes, id } => {
                    let pdf = godpdf_core::remove_annotation(&bytes, page, &id)
                        .map_err(|err| err.to_string())?;
                    let doc = Document::open(Arc::new(pdf), None).map_err(|err| err.to_string())?;
                    Arc::new(doc)
                }
            };
            match (kind, region) {
                (Kind::Detail { .. }, Some(region)) => doc.render_region(page, scale, region),
                (Kind::Page | Kind::Lifted | Kind::Detail { .. }, _) => doc.render(page, scale),
                // For thumbnails `scale` is the longest side in pixels.
                (Kind::Thumb, _) => doc.thumbnail(page, scale.round().max(1.0) as u32),
            }
            .map_err(|err| err.to_string())
        }))
        .unwrap_or_else(|_| Err(PANIC_MESSAGE.to_owned()));
        Rendered {
            generation,
            page,
            kind,
            scale,
            region,
            result,
        }
    }
}

/// Background render thread. Exits when the tab (and its sender) is dropped.
#[cfg(not(target_arch = "wasm32"))]
struct Worker {
    requests: std::sync::mpsc::Sender<Request>,
    results: std::sync::mpsc::Receiver<Rendered>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Worker {
    fn spawn(ctx: Context) -> Option<Self> {
        let (requests, inbox) = std::sync::mpsc::channel::<Request>();
        let (outbox, results) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("render".to_owned())
            .stack_size(16 << 20)
            .spawn(move || {
                for request in inbox {
                    if outbox.send(request.run()).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .ok()?;
        Some(Self { requests, results })
    }
}

/// A page rendered without one annotation: shown under that annotation while it is moved or
/// resized, so only the annotation itself appears to move.
struct Lifted {
    page: usize,
    id: String,
    /// Document generation it was made from.
    generation: u64,
    serial: u64,
    scale: f32,
    texture: Option<TextureHandle>,
}

pub struct DocTab {
    pub id: u64,
    pub session: Session,
    /// The bytes `render` was opened from.
    bytes: Arc<Vec<u8>>,
    render: Option<Arc<Document>>,
    pub render_error: Option<String>,
    /// Bumped on every reload; render/search results from older generations are stale.
    generation: u64,
    /// Displayed (post-rotation) page sizes in points.
    pub page_sizes: Vec<Vec2>,
    pages: HashMap<usize, Cached>,
    /// By page and viewer (true for the second one).
    details: HashMap<(usize, bool), Detail>,
    thumbs: HashMap<usize, Cached>,
    page_errors: HashMap<usize, String>,
    in_flight: HashSet<(usize, Kind)>,
    lifted: Option<Lifted>,
    lift_serial: u64,
    /// A lift is rendering; the next one waits for it.
    lift_busy: bool,
    #[cfg(not(target_arch = "wasm32"))]
    worker: Option<Worker>,
    pub outline: Option<Result<Vec<OutlineItem>, String>>,
    /// The bookmark whose name is being typed in the bookmarks panel.
    pub bookmark_rename: Option<crate::bookmarks::Renaming>,
    /// The form shown in the form panel, read once per revision.
    pub form: Option<crate::formfill::FormState>,
    /// Parsed annotations per page for hit-testing; cleared on reload (new generation).
    annotations: HashMap<usize, Vec<PlacedAnnotation>>,
    /// Page rotation / origin per page; cleared on reload.
    geometries: HashMap<usize, PageGeometry>,
    /// Text with positions per page, for text selection; cleared on reload.
    texts: HashMap<usize, Arc<Vec<TextChar>>>,
    /// Selected / dragged annotation (Select tool).
    pub canvas: crate::selection::CanvasState,
    pub search: SearchState,
    pub current_page: usize,
    pub zoom: f32,
    pub scroll_to_page: Option<usize>,
    /// Vertical scroll offset to apply next frame (after zoom).
    pub pending_scroll: Option<f32>,
    pub scroll_y: f32,
    /// Size of the viewer area last frame, for fit width / fit page.
    pub viewport: Vec2,
    pub selection: BTreeSet<usize>,
    pub anchor: Option<usize>,
    pub mode: ViewMode,
    /// (user, owner) passwords applied when saving.
    pub encrypt_on_save: Option<(String, String)>,
    /// `encrypt_on_save` as of the last save / open; a difference counts as unsaved.
    pub saved_encryption: Option<(String, String)>,
    pub gesture: Option<Gesture>,
    /// Created by the app (merge, extract, split) and never saved: counts as unsaved.
    pub derived: bool,
    /// Fit width once the viewer size is known.
    pub fit_pending: bool,
    /// When its file was last written, as of the last open or save: a later time means
    /// another program changed it.
    pub disk_time: Option<std::time::SystemTime>,
    /// Where the second viewer of this document is, when it shows twice side by side.
    other_place: Place,
    /// The place in use is the second viewer's (see [`Self::swap_place`]).
    pub second: bool,
}

/// Where a viewer is in a document.
#[derive(Clone, Copy, Default)]
struct Place {
    scroll_y: f32,
    current_page: usize,
    scroll_to_page: Option<usize>,
    pending_scroll: Option<f32>,
    viewport: Vec2,
}

impl DocTab {
    pub fn new(id: u64, session: Session) -> Self {
        let mut tab = Self {
            id,
            session,
            bytes: Arc::default(),
            render: None,
            render_error: None,
            generation: 0,
            page_sizes: Vec::new(),
            pages: HashMap::new(),
            details: HashMap::new(),
            thumbs: HashMap::new(),
            page_errors: HashMap::new(),
            in_flight: HashSet::new(),
            lifted: None,
            lift_serial: 0,
            lift_busy: false,
            #[cfg(not(target_arch = "wasm32"))]
            worker: None,
            outline: None,
            bookmark_rename: None,
            form: None,
            annotations: HashMap::new(),
            geometries: HashMap::new(),
            texts: HashMap::new(),
            canvas: crate::selection::CanvasState::default(),
            search: SearchState::default(),
            current_page: 0,
            zoom: 1.0,
            scroll_to_page: None,
            pending_scroll: None,
            scroll_y: 0.0,
            viewport: vec2(800.0, 600.0),
            selection: BTreeSet::new(),
            anchor: None,
            mode: ViewMode::Read,
            encrypt_on_save: None,
            saved_encryption: None,
            gesture: None,
            derived: false,
            fit_pending: true,
            disk_time: None,
            other_place: Place::default(),
            second: false,
        };
        tab.reload();
        tab
    }

    /// Trade the place in use for the one of the other viewer showing this document. Called
    /// around the second viewer, so everything else keeps seeing the first.
    pub fn swap_place(&mut self) {
        let other = &mut self.other_place;
        std::mem::swap(&mut self.scroll_y, &mut other.scroll_y);
        std::mem::swap(&mut self.current_page, &mut other.current_page);
        std::mem::swap(&mut self.scroll_to_page, &mut other.scroll_to_page);
        std::mem::swap(&mut self.pending_scroll, &mut other.pending_scroll);
        std::mem::swap(&mut self.viewport, &mut other.viewport);
        self.second = !self.second;
    }

    /// The second viewer starts on the page the first is on.
    pub fn start_second_view(&mut self) {
        self.other_place = Place {
            scroll_y: self.scroll_y,
            current_page: self.current_page,
            scroll_to_page: Some(self.current_page),
            pending_scroll: None,
            viewport: self.viewport,
        };
    }

    pub fn title(&self) -> &str {
        &self.session.name
    }

    pub fn is_dirty(&self) -> bool {
        self.derived || self.session.is_dirty() || self.encrypt_on_save != self.saved_encryption
    }

    pub fn page_count(&self) -> usize {
        self.page_sizes.len()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Re-parse after the bytes changed. Old textures stay visible (as stale previews) while
    /// the page count is unchanged; selection is cleared (callers re-set it after an edit).
    pub fn reload(&mut self) {
        let old_count = self.page_count();
        self.generation += 1;
        self.in_flight.clear();
        self.page_errors.clear();
        self.outline = None;
        self.annotations.clear();
        self.geometries.clear();
        self.texts.clear();
        self.canvas.drag = None;
        self.canvas.resize = None;
        self.canvas.text = None;
        self.search.hits.clear();
        self.search.searched.clear();
        self.search.busy = false;
        self.gesture = None;
        self.selection.clear();
        self.anchor = None;
        self.page_sizes.clear();

        let bytes = Arc::new(self.session.bytes().to_vec());
        self.bytes = Arc::clone(&bytes);
        let opened = catch_unwind(AssertUnwindSafe(|| {
            let doc = Document::open(bytes, None).map_err(|err| err.to_string())?;
            let sizes: Vec<Vec2> = (0..doc.page_count())
                .map(|page| {
                    doc.page_size(page)
                        .map_or(vec2(612.0, 792.0), |size| vec2(size.width, size.height))
                })
                .collect();
            Ok((doc, sizes))
        }))
        .unwrap_or_else(|_| Err(PANIC_MESSAGE.to_owned()));
        match opened {
            Ok((doc, sizes)) => {
                self.page_sizes = sizes;
                self.render = Some(Arc::new(doc));
                self.render_error = None;
            }
            Err(err) => {
                self.render = None;
                self.render_error = Some(err);
            }
        }
        let count = self.page_count();
        if count != old_count || self.render.is_none() {
            self.pages.clear();
            self.details.clear();
            self.thumbs.clear();
            self.lifted = None;
            self.canvas.ghost = None;
        }
        self.current_page = self.current_page.min(count.saturating_sub(1));
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

    /// Annotations of `page`, parsed once per document revision.
    pub fn annotations(&mut self, page: usize) -> Result<&[PlacedAnnotation], String> {
        if !self.annotations.contains_key(&page) {
            let list = guarded(|| godpdf_core::annotations(self.session.bytes(), page))
                .map_err(|err| err.to_string())?;
            self.annotations.insert(page, list);
        }
        Ok(self.annotations.get(&page).map_or(&[], Vec::as_slice))
    }

    /// Text of `page` with positions (displayed points), read once per document revision.
    /// Empty when the page has no text or cannot be read.
    pub fn text_chars(&mut self, page: usize) -> Arc<Vec<TextChar>> {
        if let Some(chars) = self.texts.get(&page) {
            return Arc::clone(chars);
        }
        let chars = self
            .render
            .as_ref()
            .and_then(|doc| {
                let doc = Arc::clone(doc);
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                    doc.text_chars(page).ok()
                }))
                .ok()?
            })
            .unwrap_or_default();
        let chars = Arc::new(chars);
        self.texts.insert(page, Arc::clone(&chars));
        chars
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

    pub fn undo(&mut self) -> bool {
        let done = self.session.undo();
        if done {
            self.reload();
        }
        done
    }

    pub fn redo(&mut self) -> bool {
        let done = self.session.redo();
        if done {
            self.reload();
        }
        done
    }

    /// Pages an organize command acts on: the selection in Organize view, else the current page.
    pub fn target_pages(&self) -> Vec<usize> {
        if self.mode == ViewMode::Organize && !self.selection.is_empty() {
            self.selection.iter().copied().collect()
        } else if self.page_count() > 0 {
            vec![self.current_page]
        } else {
            Vec::new()
        }
    }

    pub fn set_zoom(&mut self, zoom: f32) {
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        if (zoom - self.zoom).abs() < f32::EPSILON {
            return;
        }
        self.pending_scroll = Some(self.scroll_y * zoom / self.zoom);
        self.zoom = zoom;
    }

    pub fn max_page_width(&self) -> f32 {
        self.page_sizes
            .iter()
            .map(|size| size.x)
            .fold(1.0, f32::max)
    }

    pub fn page_error(&self, page: usize) -> Option<&str> {
        self.page_errors.get(&page).map(String::as_str)
    }

    /// Texture for `page` near `scale`. May return a stale texture (old resolution or old
    /// revision) while the fresh one renders; `None` means "nothing to show yet".
    pub fn page_texture(
        &mut self,
        ctx: &Context,
        page: usize,
        scale: f32,
        budget: &mut Budget,
    ) -> Option<&TextureHandle> {
        self.poll(ctx);
        if self.page_errors.contains_key(&page) {
            return None;
        }
        let size = self.page_sizes.get(page).copied()?;
        let scale = scale.min(MAX_TEXTURE_SIDE / size.x.max(size.y).max(1.0));
        let fresh = self
            .pages
            .get(&page)
            .is_some_and(|tex| tex.generation == self.generation && same_scale(tex.scale, scale));
        if !fresh {
            self.request(ctx, page, Kind::Page, scale, None, budget);
        }
        self.pages.get(&page).map(|tex| &tex.texture)
    }

    /// A sharp picture of the part of `page` on screen, when the page is too large to be drawn
    /// whole at `scale`: its texture, its size in pixels and the part of the page it shows.
    /// `visible` is the part on screen, in displayed points. May be stale, as page textures.
    pub fn page_detail(
        &mut self,
        ctx: &Context,
        page: usize,
        scale: f32,
        visible: egui::Rect,
        budget: &mut Budget,
    ) -> Option<(egui::TextureId, Vec2, egui::Rect)> {
        let key = (page, self.second);
        let size = self.page_sizes.get(page).copied()?;
        let sheet = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        let visible = visible.intersect(sheet);
        let whole = MAX_TEXTURE_SIDE / size.x.max(size.y).max(1.0);
        if scale <= whole * (1.0 + SCALE_TOLERANCE) || !visible.is_positive() {
            self.details.remove(&key);
            return None;
        }
        // Some of the page around what shows: scrolling a little needs no new picture.
        let room = (Vec2::splat(MAX_TEXTURE_SIDE / scale) - visible.size()).max(Vec2::ZERO);
        let around = visible.expand2(room.min(visible.size()) / 2.0);
        // On whole pixels, to be drawn pixel for pixel like the page under it.
        let min = (around.min.to_vec2() * scale).floor() / scale;
        let max = (around.max.to_vec2() * scale).ceil() / scale;
        let wanted = egui::Rect::from_min_max(min.to_pos2(), max.to_pos2()).intersect(sheet);
        let fresh = self.details.get(&key).is_some_and(|detail| {
            detail.generation == self.generation
                && same_scale(detail.scale, scale)
                && detail.region.contains_rect(visible.intersect(wanted))
        });
        if !fresh {
            let kind = Kind::Detail {
                second: self.second,
            };
            let region = [wanted.min.x, wanted.min.y, wanted.max.x, wanted.max.y];
            self.request(ctx, page, kind, scale, Some(region), budget);
        }
        let detail = self.details.get(&key)?;
        Some((
            detail.texture.id(),
            detail.texture.size_vec2(),
            detail.region,
        ))
    }

    /// True when the texture of `page` shows the current document revision.
    pub fn page_fresh(&self, page: usize) -> bool {
        self.pages
            .get(&page)
            .is_some_and(|tex| tex.generation == self.generation)
    }

    /// Prepare `page` rendered without the annotation `id` (see [`Self::lifted`]). Cheap to
    /// call every frame: it only renders when the target or the document changed.
    pub fn lift(&mut self, ctx: &Context, page: usize, id: &str) {
        let Some(scale) = self.pages.get(&page).map(|tex| tex.scale) else {
            return;
        };
        let current = self.lifted.as_ref().is_some_and(|lifted| {
            lifted.page == page
                && lifted.id == id
                && lifted.generation == self.generation
                && same_scale(lifted.scale, scale)
        });
        if current || self.lift_busy {
            return;
        }
        self.lift_serial += 1;
        self.lift_busy = true;
        self.lifted = Some(Lifted {
            page,
            id: id.to_owned(),
            generation: self.generation,
            serial: self.lift_serial,
            scale,
            texture: None,
        });
        self.send(
            ctx,
            Request {
                source: Source::Without {
                    bytes: Arc::clone(&self.bytes),
                    id: id.to_owned(),
                },
                generation: self.lift_serial,
                page,
                kind: Kind::Lifted,
                scale,
                region: None,
            },
        );
    }

    /// `page` without the annotation `id`, once [`Self::lift`] has rendered it.
    pub fn lifted(&self, page: usize, id: &str) -> Option<egui::TextureId> {
        let lifted = self
            .lifted
            .as_ref()
            .filter(|lifted| lifted.page == page && lifted.id == id)?;
        lifted.texture.as_ref().map(TextureHandle::id)
    }

    pub fn thumbnail(
        &mut self,
        ctx: &Context,
        page: usize,
        budget: &mut Budget,
    ) -> Option<&TextureHandle> {
        self.poll(ctx);
        if self.page_errors.contains_key(&page) {
            return None;
        }
        // Device pixels so thumbnails stay sharp on HiDPI screens.
        let pixels = THUMB_PX as f32 * ctx.pixels_per_point();
        let fresh = self
            .thumbs
            .get(&page)
            .is_some_and(|tex| tex.generation == self.generation && same_scale(tex.scale, pixels));
        if !fresh {
            self.request(ctx, page, Kind::Thumb, pixels, None, budget);
        }
        self.thumbs.get(&page).map(|tex| &tex.texture)
    }

    /// Free full-size textures far from what is on screen.
    pub fn retain_pages(&mut self, keep: std::ops::RangeInclusive<usize>) {
        self.pages.retain(|page, _| keep.contains(page));
        self.details.retain(|(page, _), _| keep.contains(page));
    }

    fn request(
        &mut self,
        ctx: &Context,
        page: usize,
        kind: Kind,
        scale: f32,
        region: Option<[f32; 4]>,
        budget: &mut Budget,
    ) {
        if self.in_flight.contains(&(page, kind))
            || self.in_flight.len() >= MAX_IN_FLIGHT
            || !budget.take()
        {
            return;
        }
        let Some(doc) = self.render.clone() else {
            return;
        };
        self.in_flight.insert((page, kind));
        self.send(
            ctx,
            Request {
                source: Source::Doc(doc),
                generation: self.generation,
                page,
                kind,
                scale,
                region,
            },
        );
    }

    /// Hand a request to the worker, or run it right here without one.
    fn send(&mut self, ctx: &Context, request: Request) {
        #[cfg(not(target_arch = "wasm32"))]
        let request = {
            if self.worker.is_none() {
                self.worker = Worker::spawn(ctx.clone());
            }
            let unsent = match &self.worker {
                Some(worker) => worker.requests.send(request).err().map(|err| err.0),
                None => Some(request),
            };
            let Some(request) = unsent else {
                return;
            };
            // No worker (spawn failed or it died): render inline.
            self.worker = None;
            request
        };
        let rendered = request.run();
        self.store(ctx, rendered);
    }

    /// Upload finished renders from the worker.
    fn poll(&mut self, ctx: &Context) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let done: Vec<Rendered> = self
                .worker
                .as_ref()
                .map(|worker| worker.results.try_iter().collect())
                .unwrap_or_default();
            for rendered in done {
                self.store(ctx, rendered);
            }
        }
        #[cfg(target_arch = "wasm32")]
        let _ = ctx;
    }

    fn store(&mut self, ctx: &Context, rendered: Rendered) {
        if rendered.kind == Kind::Lifted {
            self.lift_busy = false;
            if let Some(lifted) = self
                .lifted
                .as_mut()
                .filter(|lifted| lifted.serial == rendered.generation)
                && let Ok(bitmap) = &rendered.result
            {
                let name = format!("lifted-{}", self.id);
                lifted.texture = Some(upload(ctx, name, bitmap));
            }
            return;
        }
        if rendered.generation != self.generation {
            return;
        }
        let page = rendered.page;
        self.in_flight.remove(&(page, rendered.kind));
        if let Kind::Detail { second } = rendered.kind {
            // A part that cannot be drawn leaves the page as it shows.
            if let (Ok(bitmap), Some([left, top, right, bottom])) =
                (&rendered.result, rendered.region)
            {
                let name = format!("detail-{}-{page}-{second}", self.id);
                let corners = (egui::pos2(left, top), egui::pos2(right, bottom));
                let detail = Detail {
                    texture: upload(ctx, name, bitmap),
                    scale: rendered.scale,
                    generation: rendered.generation,
                    region: egui::Rect::from_min_max(corners.0, corners.1),
                };
                self.details.insert((page, second), detail);
            }
            return;
        }
        let bitmap = match rendered.result {
            Ok(bitmap) => bitmap,
            Err(err) => {
                self.page_errors.insert(page, err);
                return;
            }
        };
        let (prefix, cache) = match rendered.kind {
            Kind::Page => ("page", &mut self.pages),
            Kind::Thumb => ("thumb", &mut self.thumbs),
            Kind::Lifted | Kind::Detail { .. } => return,
        };
        let texture = upload(ctx, format!("{prefix}-{}-{page}", self.id), &bitmap);
        cache.insert(
            page,
            Cached {
                texture,
                scale: rendered.scale,
                generation: rendered.generation,
            },
        );
    }
}

/// Scales close enough that re-rendering would not change a pixel.
fn same_scale(cached: f32, wanted: f32) -> bool {
    (cached - wanted).abs() <= wanted.abs() * SCALE_TOLERANCE
}

fn upload(ctx: &Context, name: String, bitmap: &Bitmap) -> TextureHandle {
    let size = [bitmap.width as usize, bitmap.height as usize];
    let image = if bitmap.rgba.len() == size[0] * size[1] * 4 {
        ColorImage::from_rgba_unmultiplied(size, &bitmap.rgba)
    } else {
        // Defensive: a malformed bitmap would panic in ColorImage.
        ColorImage::filled([1, 1], egui::Color32::WHITE)
    };
    ctx.load_texture(name, image, TextureOptions::LINEAR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn searched_words_are_marked_only_while_the_search_shows_them() {
        let mut search = SearchState {
            query: " fluo ".to_owned(),
            searched: "fluo".to_owned(),
            open: true,
            ..SearchState::default()
        };
        assert_eq!(search.marked(), Some("fluo"));
        // Other words in the field: the marks of the old ones go.
        search.query = "flu".to_owned();
        assert_eq!(search.marked(), None);
        // The panel closed: no marks, and they come back with it.
        search.query = "fluo".to_owned();
        search.open = false;
        assert_eq!(search.marked(), None);
        search.open = true;
        assert_eq!(search.marked(), Some("fluo"));
        search.searched.clear();
        search.query.clear();
        assert_eq!(search.marked(), None);
    }
}
