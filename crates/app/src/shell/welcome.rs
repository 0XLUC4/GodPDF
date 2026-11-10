//! Start screen: the recent files from the top left, 24 px in, each a bare first-page preview
//! (thin outline and shadow, no card) with its name and folder under it. Without recent files,
//! a small logo and a hint, centered. Open / New / Merge live in the Menu and the palette.
//! Shown without a document or when the home tab is in front. Fill `bg`.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant, SystemTime};

use gpui::component::menu::ContextMenuExt;
use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Global, Image, ImageFormat, ObjectFit, RenderImage, SharedString, Window,
    div, img, px,
};

use crate::app::GodPdfApp;
use crate::theme::Palette;
use crate::{icons, widgets};

pub static LOGO: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Png,
        include_bytes!("../../../../assets/logo.png").to_vec(),
    ))
});

/// Small text size.
const SMALL: f32 = 9.0;
/// Preview width, gap between files, padding of the screen.
const ITEM: f32 = 208.0;
const GAP: f32 = 20.0;
const PAD: f32 = 24.0;
/// The preview box: A4 proportions, whole pixels.
const PREVIEW: (f32, f32) = (ITEM, 294.0);

/// What a card shows in its preview box.
enum Preview {
    Rendering,
    Ready(Arc<RenderImage>),
    /// Encrypted: no preview without the password.
    Locked,
    Unreadable,
}

/// How long the modified times of the recent files are trusted before they are read again.
const STAT_EVERY: Duration = Duration::from_secs(2);

/// First-page previews of the recent files, by path and the modified time they show.
#[derive(Default)]
struct Previews {
    cards: HashMap<PathBuf, (SystemTime, Preview)>,
    /// Modified time of each recent file (`None` = not found), read off the UI thread.
    times: HashMap<PathBuf, Option<SystemTime>>,
    /// When `times` was last read, and whether a read is running.
    read_at: Option<Instant>,
    reading: bool,
    /// Images dropped from the cache; their GPU textures are freed on the next render.
    evicted: Vec<Arc<RenderImage>>,
}

impl Global for Previews {}

impl Previews {
    fn forget(&mut self, path: &Path) {
        if let Some((_, Preview::Ready(image))) = self.cards.remove(path) {
            self.evicted.push(image);
        }
    }
}

/// Render the first page of `path`, at most `max_px` a side (blocking: run it off the UI thread).
fn render_first_page(path: &Path, max_px: u32) -> Preview {
    let rendered = catch_unwind(AssertUnwindSafe(|| {
        let bytes = std::fs::read(path).map_err(|_| Preview::Unreadable)?;
        let doc =
            godpdf_render::Document::open(Arc::new(bytes), None).map_err(|err| match err {
                godpdf_render::Error::PasswordRequired => Preview::Locked,
                _ => Preview::Unreadable,
            })?;
        let bitmap = doc.thumbnail(0, max_px).map_err(|_| Preview::Unreadable)?;
        crate::doc::to_render_image(bitmap).map_err(|_| Preview::Unreadable)
    }));
    match rendered {
        Ok(Ok(image)) => Preview::Ready(Arc::new(image)),
        Ok(Err(preview)) => preview,
        Err(_) => Preview::Unreadable,
    }
}

/// Read the modified times of `recent` in the background when they are older than
/// [`STAT_EVERY`] (a file on a sleeping network drive can take seconds to answer).
fn refresh_times(recent: &[PathBuf], cx: &mut Context<GodPdfApp>) {
    let previews = cx.default_global::<Previews>();
    if previews.reading || previews.read_at.is_some_and(|at| at.elapsed() < STAT_EVERY) {
        return;
    }
    previews.reading = true;
    let paths = recent.to_vec();
    let read = cx.background_executor().spawn(async move {
        paths
            .into_iter()
            .map(|path| {
                let time = std::fs::metadata(&path)
                    .and_then(|meta| meta.modified())
                    .ok();
                (path, time)
            })
            .collect::<HashMap<_, _>>()
    });
    cx.spawn(async move |this, cx| {
        let times = read.await;
        let _ = this.update(cx, |_, cx| {
            let previews = cx.default_global::<Previews>();
            previews.times = times;
            previews.read_at = Some(Instant::now());
            previews.reading = false;
            cx.notify();
        });
    })
    .detach();
}

/// Whether each recent file exists (true until first looked at); previews of files gone
/// from the list or changed on disk are dropped, missing ones start rendering.
fn sync_previews(
    recent: &[PathBuf],
    window: &mut Window,
    cx: &mut Context<GodPdfApp>,
) -> Vec<bool> {
    refresh_times(recent, cx);
    let max_px = (PREVIEW.1 * window.scale_factor()).round() as u32;
    let previews = cx.default_global::<Previews>();
    for image in previews.evicted.drain(..) {
        // Fails only for an image never painted: nothing to free then.
        let _ = window.drop_image(image);
    }
    let gone: Vec<PathBuf> = previews
        .cards
        .keys()
        .filter(|path| !recent.contains(path))
        .cloned()
        .collect();
    for path in gone {
        previews.forget(&path);
    }

    let mut exists = Vec::with_capacity(recent.len());
    for path in recent {
        let previews = cx.default_global::<Previews>();
        let Some(&time) = previews.times.get(path) else {
            // Not looked at yet (just opened): shown as there, without a preview.
            exists.push(true);
            continue;
        };
        exists.push(time.is_some());
        let Some(time) = time else {
            previews.forget(path);
            continue;
        };
        if previews
            .cards
            .get(path)
            .is_some_and(|(made, _)| *made == time)
        {
            continue;
        }
        previews.forget(path);
        previews
            .cards
            .insert(path.clone(), (time, Preview::Rendering));
        let job_path = path.clone();
        let rendered = cx
            .background_executor()
            .spawn(async move { render_first_page(&job_path, max_px) });
        let path = path.clone();
        cx.spawn(async move |this, cx| {
            let preview = rendered.await;
            let _ = this.update(cx, |_, cx| {
                let previews = cx.default_global::<Previews>();
                // Only if the file was not changed (or dropped) meanwhile.
                if let Some((made, slot)) = previews.cards.get_mut(&path)
                    && *made == time
                {
                    *slot = preview;
                    cx.notify();
                }
            });
        })
        .detach();
    }
    exists
}

pub fn render(
    app: &GodPdfApp,
    palette: &Palette,
    window: &mut Window,
    cx: &mut Context<GodPdfApp>,
) -> AnyElement {
    let recent = app.shell.recent.clone();
    let exists = sync_previews(&recent, window, cx);
    if recent.is_empty() {
        return div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .bg(palette.bg)
            .child(empty_state(palette))
            .into_any_element();
    }
    let items: Vec<AnyElement> = recent
        .iter()
        .zip(exists)
        .enumerate()
        .map(|(index, (path, exists))| item(index, path, exists, palette, cx))
        .collect();
    div()
        .id("welcome")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .bg(palette.bg)
        .p(px(PAD))
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(
            div()
                .text_size(px(SMALL))
                .text_color(palette.accent_text)
                .child("RECENT"),
        )
        .child(div().flex().flex_wrap().gap(px(GAP)).children(items))
        .into_any_element()
}

/// No recent file yet: the logo, small, and a hint.
fn empty_state(palette: &Palette) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(12.))
        .child(img(LOGO.clone()).size(px(56.)).rounded(px(12.)))
        .child(
            div()
                .text_size(px(15.))
                .text_color(palette.text_dim)
                .child("Open a PDF (Ctrl+O) to see it here"),
        )
        .into_any_element()
}

/// One recent file: preview, name, folder; click opens it, the cross or the right-click
/// menu removes it from the list. A missing file is greyed and only removable.
fn item(
    index: usize,
    path: &Path,
    exists: bool,
    palette: &Palette,
    cx: &mut Context<GodPdfApp>,
) -> AnyElement {
    let palette = *palette;
    let name: SharedString = path
        .file_name()
        .map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
        .into();
    let below: SharedString = if exists {
        path.parent()
            .and_then(Path::file_name)
            .map_or_else(
                || path.parent().unwrap_or(path).display().to_string(),
                |folder| folder.to_string_lossy().into_owned(),
            )
            .into()
    } else {
        "File not found".into()
    };

    let placeholder = |glyph| widgets::icon(glyph, 28.0, palette.text_dim);
    let preview: AnyElement = match cx
        .try_global::<Previews>()
        .and_then(|previews| previews.cards.get(path))
    {
        _ if !exists => placeholder(icons::FILE_X).into_any_element(),
        Some((_, Preview::Ready(image))) => img(Arc::clone(image))
            .size_full()
            .object_fit(ObjectFit::Contain)
            .into_any_element(),
        Some((_, Preview::Locked)) => placeholder(icons::LOCK).into_any_element(),
        Some((_, Preview::Unreadable)) => placeholder(icons::FILE_PDF).into_any_element(),
        Some((_, Preview::Rendering)) | None => div().into_any_element(),
    };

    let remove = |app: &mut GodPdfApp, path: &Path, cx: &mut Context<GodPdfApp>| {
        app.shell.recent.retain(|recent| recent != path);
        cx.notify();
    };
    let cross = {
        let path = path.to_path_buf();
        widgets::small_icon_button(
            ("recent-remove", index),
            icons::X,
            "Remove from recent",
            &palette,
        )
        .absolute()
        .top(px(6.))
        .right(px(6.))
        .bg(palette.panel)
        .invisible()
        .group_hover("recent-card", |style| style.visible())
        .on_click(cx.listener(move |app, _, _, cx| {
            cx.stop_propagation();
            remove(app, &path, cx);
        }))
    };
    let menu_app = cx.weak_entity();
    let menu_path = path.to_path_buf();
    let open_path = path.to_path_buf();
    let accent = palette.accent;
    let card = div()
        .id(("recent", index))
        .group("recent-card")
        .relative()
        .flex_none()
        .w(px(ITEM))
        .flex()
        .flex_col()
        .gap(px(2.))
        .when(!exists, |card| card.opacity(0.5))
        .child(
            // The page itself: a thin outline and shadow so a white page shows on white.
            div()
                .w(px(PREVIEW.0))
                .h(px(PREVIEW.1))
                .mb(px(8.))
                .rounded(px(4.))
                .overflow_hidden()
                .bg(palette.canvas)
                .border_1()
                .border_color(palette.border)
                .shadow(widgets::shadow(2.0, 8.0, 20.0 / 255.0))
                .when(exists, |page| {
                    page.group_hover("recent-card", move |style| style.border_color(accent))
                })
                .flex()
                .items_center()
                .justify_center()
                .child(preview),
        )
        .child(
            div()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_size(px(13.))
                .text_color(palette.text)
                .child(name),
        )
        .child(
            div()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_size(px(11.))
                .text_color(palette.text_dim)
                .child(below),
        )
        .child(cross)
        .when(exists, |card| {
            card.cursor_pointer()
                .on_click(cx.listener(move |app, _, _, cx| {
                    app.shell.home = false;
                    app.open_path(&open_path, cx);
                }))
        });
    widgets::tooltip(card, path.display().to_string())
        .context_menu(move |menu, _, cx| {
            let (app, path) = (menu_app.clone(), menu_path.clone());
            menu.item(
                crate::panels::menu_entry(icons::X, "Remove from recent", true, &Palette::of(cx))
                    .on_click(move |_, _, cx| {
                        let _ = app.update(cx, |app, cx| remove(app, &path, cx));
                    }),
            )
        })
        .into_any_element()
}

/// The recent file `path` was last seen missing (from the times the home screen reads).
pub fn is_missing(path: &Path, cx: &gpui::App) -> bool {
    cx.try_global::<Previews>()
        .and_then(|previews| previews.times.get(path))
        .is_some_and(Option::is_none)
}
