//! Start screen: the recent files as a grid of cards (first page preview, name, folder),
//! then Open / New / Merge buttons (170 × 44, radius 10, text 16). Without recent files, a
//! small logo and "Open a PDF to see it here" instead of the grid.
//! Shown without a document or when the home tab is in front. Fill `bg`, centered in the view.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::SystemTime;

use gpui::component::menu::ContextMenuExt;
use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Global, Image, ImageFormat, ObjectFit, RenderImage, SharedString, Window,
    div, img, px,
};

use crate::app::{GodPdfApp, Open};
use crate::dialogs::MergeFiles;
use crate::theme::Palette;
use crate::widgets::{self, dispatch};
use crate::{icons, toasts};

pub static LOGO: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Png,
        include_bytes!("../../../../assets/logo.png").to_vec(),
    ))
});

/// Small text size.
const SMALL: f32 = 9.0;
/// Card width, gap between cards, inner padding.
const CARD: f32 = 160.0;
const GAP: f32 = 14.0;
const CARD_PAD: f32 = 8.0;
/// The preview box inside a card: A4 proportions.
const PREVIEW: (f32, f32) = (CARD - 2.0 * CARD_PAD, (CARD - 2.0 * CARD_PAD) * 1.414);
/// Widest the column gets.
const MAX_WIDTH: f32 = 980.0;

/// What a card shows in its preview box.
enum Preview {
    Rendering,
    Ready(Arc<RenderImage>),
    /// Encrypted: no preview without the password.
    Locked,
    Unreadable,
}

/// First-page previews of the recent files, by path and the modified time they show.
#[derive(Default)]
struct Previews {
    cards: HashMap<PathBuf, (SystemTime, Preview)>,
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

/// How many cards fit in a row of `width` px, at least 1 and no more than `count`.
fn columns(width: f32, count: usize) -> usize {
    let fit = ((width + GAP) / (CARD + GAP)).floor().max(1.0) as usize;
    fit.min(count.max(1))
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

/// The modified time of each recent file (`None` = not found); previews of files gone from
/// the list or changed on disk are dropped, missing ones start rendering.
fn sync_previews(
    recent: &[PathBuf],
    window: &mut Window,
    cx: &mut Context<GodPdfApp>,
) -> Vec<Option<SystemTime>> {
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

    let mut modified = Vec::with_capacity(recent.len());
    for path in recent {
        // shortcut: one stat per file per frame, fine for 8 local files; cache it if a slow
        // (network) drive makes the welcome screen lag.
        let time = std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok();
        modified.push(time);
        let previews = cx.default_global::<Previews>();
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
    modified
}

pub fn render(
    app: &GodPdfApp,
    palette: &Palette,
    window: &mut Window,
    cx: &mut Context<GodPdfApp>,
) -> AnyElement {
    let button = |id: &'static str, glyph, label: &'static str, primary: bool| {
        // Open has a fixed `accent` fill; the others the widget fills (surface, hover,
        // pressed `selected`) inside a 1 px `accent` stroke.
        let (hover, pressed) = (palette.hover, palette.selected);
        let text = if primary {
            palette.on_accent
        } else {
            palette.text
        };
        div()
            .id(id)
            .w(px(170.))
            .h(px(44.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(10.))
            .when(primary, |button| button.bg(palette.accent))
            .when(!primary, |button| {
                button
                    .bg(palette.surface)
                    .border_1()
                    .border_color(palette.accent)
                    .hover(move |style| style.bg(hover))
                    .active(move |style| style.bg(pressed))
            })
            .child(widgets::with_icon(glyph, label, 16.0, text))
    };
    let new_pdf = cx.listener(|app, _, _, cx| {
        // One empty A4 page to write, draw and insert into.
        match godpdf_core::blank_document(595.0, 842.0) {
            Ok(bytes) => {
                app.open_bytes("Untitled.pdf".to_owned(), None, bytes, cx);
                app.shell.home = false;
            }
            Err(err) => toasts::error(cx, err.to_string()),
        }
    });
    let open = cx.listener(|app, _, window, cx| {
        app.shell.home = false;
        window.dispatch_action(Box::new(Open), cx);
    });
    let buttons = div()
        .flex()
        .gap(px(8.))
        .child(button("welcome-open", icons::FOLDER_OPEN, "Open PDF", true).on_click(open))
        .child(button("welcome-new", icons::FILE_PLUS, "New PDF", false).on_click(new_pdf))
        .child(
            button("welcome-merge", icons::STACK, "Merge PDFs", false)
                .on_click(dispatch(MergeFiles)),
        );

    let recent = app.shell.recent.clone();
    let modified = sync_previews(&recent, window, cx);
    let top = if recent.is_empty() {
        empty_state(palette)
    } else {
        let width = (f32::from(window.viewport_size().width) - 2.0 * 32.0).min(MAX_WIDTH);
        let cols = columns(width, recent.len());
        let grid_width = cols as f32 * CARD + (cols - 1) as f32 * GAP;
        let cards: Vec<AnyElement> = recent
            .iter()
            .zip(modified)
            .enumerate()
            .map(|(index, (path, time))| card(index, path, time.is_some(), palette, cx))
            .collect();
        div()
            .w(px(grid_width))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .text_size(px(SMALL))
                    .text_color(palette.accent_text)
                    .child("RECENT"),
            )
            .child(div().flex().flex_wrap().gap(px(GAP)).children(cards))
            .into_any_element()
    };

    div()
        .id("welcome")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .bg(palette.bg)
        .child(
            // The column is centered in the view, 24 px at least from the top.
            div()
                .min_h_full()
                .py(px(24.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(24.))
                .child(top)
                .child(buttons),
        )
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
                .child("Open a PDF to see it here"),
        )
        .into_any_element()
}

/// One recent file: preview, name, folder; click opens it, the cross or the right-click
/// menu removes it from the list. A missing file is greyed and only removable.
fn card(
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
        .top(px(4.))
        .right(px(4.))
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
    let hover = palette.hover;
    let card = div()
        .id(("recent", index))
        .group("recent-card")
        .relative()
        .flex_none()
        .w(px(CARD))
        .p(px(CARD_PAD))
        .flex()
        .flex_col()
        .gap(px(2.))
        .rounded(px(10.))
        .bg(palette.surface)
        .border_1()
        .border_color(palette.border)
        .hover(move |style| style.bg(hover))
        .when(!exists, |card| card.opacity(0.5))
        .child(
            div()
                .w(px(PREVIEW.0))
                .h(px(PREVIEW.1))
                .mb(px(6.))
                .rounded(px(4.))
                .overflow_hidden()
                .bg(palette.canvas)
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

#[cfg(test)]
mod tests {
    use super::{CARD, GAP, columns};

    #[test]
    fn grid_columns_follow_the_width() {
        assert_eq!(columns(CARD, 8), 1);
        assert_eq!(columns(10.0, 8), 1);
        assert_eq!(columns(2.0 * CARD + GAP, 8), 2);
        assert_eq!(columns(2.0 * CARD + GAP - 1.0, 8), 1);
        assert_eq!(columns(980.0, 8), 5);
        // Never more columns than cards.
        assert_eq!(columns(980.0, 2), 2);
        assert_eq!(columns(980.0, 0), 1);
    }
}
