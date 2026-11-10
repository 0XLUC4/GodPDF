//! Continuous vertical page viewer: `impl Render for DocTab`.
//!
//! Scrolling is ours (not a gpui scroll container): `DocTab::view.scroll` is the offset, and
//! only pages intersecting the view are built, absolutely placed. Coordinates: see
//! `geometry.rs`. Bitmaps are rendered at `zoom * window.scale_factor()` pixels per point.

use gpui::prelude::*;
use gpui::{
    Bounds, Context, MouseButton, MouseDownEvent, MouseMoveEvent, ObjectFit, Pixels, ScrollDelta,
    ScrollEvent, Window, canvas, div, img, px, white,
};

use crate::doc::DocTab;
use crate::geometry::ZOOM_STEP;
use crate::theme::Palette;
use crate::tools;

/// Logical pixels per wheel "line" (Windows sends 3 lines per notch by default).
const LINE_HEIGHT: f32 = 33.0;
/// Wheel lines in one notch, to turn Ctrl+wheel into zoom steps.
const LINES_PER_NOTCH: f32 = 3.0;
const SCROLLBAR_WIDTH: f32 = 10.0;
const MIN_THUMB: f32 = 24.0;

/// Scrollbar drag in progress: the view-space y where the thumb was grabbed, relative to
/// its top.
#[derive(Clone, Copy)]
pub struct ThumbDrag {
    grab_offset: f32,
}

impl Render for DocTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::of(cx);
        let this = cx.entity();
        // Learn the viewer size after layout; layout-dependent state is fixed up then.
        let measure = canvas(
            move |bounds, _, cx| {
                this.update(cx, |tab, cx| tab.set_bounds(bounds, cx));
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        let root = div()
            .id("viewer")
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(palette.canvas)
            .child(measure)
            .on_scroll(cx.listener(Self::on_wheel))
            .on_mouse_move(cx.listener(Self::on_drag_thumb))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|tab, _, _, cx| {
                    if tab.thumb_drag.take().is_some() {
                        cx.notify();
                    }
                }),
            );

        if let Some(err) = &self.render_error {
            return root.child(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(palette.error)
                    .child(format!("Cannot display this PDF: {err}")),
            );
        }

        let layout = self.layout();
        let (scroll_x, scroll_y) = self.view.scroll;
        let view_height = f32::from(self.view.bounds.size.height);
        let visible = layout.visible(scroll_y, view_height);
        let scale = self.view.zoom * window.scale_factor();
        let mut pages = Vec::with_capacity(visible.len());
        for page in visible.clone() {
            let rect = layout.rects[page];
            let image = self.page_image(page, scale, cx);
            // The part on screen, in displayed points (drawn sharp over a too large page).
            let zoom = self.view.zoom;
            let (left, top) = (rect.x - scroll_x, rect.y - scroll_y);
            let view_width = f32::from(self.view.bounds.size.width);
            let visible = [
                (-left).max(0.0) / zoom,
                (-top).max(0.0) / zoom,
                (view_width - left).min(rect.width) / zoom,
                (view_height - top).min(rect.height) / zoom,
            ];
            let detail = self
                .page_detail(page, scale, visible, cx)
                .map(|(image, part)| {
                    img(image)
                        .absolute()
                        .left(px(part[0] * zoom))
                        .top(px(part[1] * zoom))
                        .w(px((part[2] - part[0]) * zoom))
                        .h(px((part[3] - part[1]) * zoom))
                        .object_fit(ObjectFit::Fill)
                });
            let error = self
                .page_error(page)
                .map(|err| format!("Page {}: {err}", page + 1));
            let (hits, text_under, overlay, text_over) = if self.companion {
                (None, None, None, None)
            } else {
                (
                    crate::panels::page_overlay(self, page, cx),
                    crate::text::page_underlay(self, page, window, cx),
                    tools::page_overlay(self, page, rect, window, cx),
                    crate::text::page_overlay(self, page, window, cx),
                )
            };
            pages.push(
                div()
                    .absolute()
                    .left(px(rect.x - scroll_x))
                    .top(px(rect.y - scroll_y))
                    .w(px(rect.width))
                    .h(px(rect.height))
                    .bg(white())
                    .shadow_md()
                    .when_some(image, |page_box, image| {
                        page_box.child(img(image).size_full().object_fit(ObjectFit::Fill))
                    })
                    .children(detail)
                    .when_some(error, |page_box, error| {
                        page_box
                            .p(px(16.))
                            .text_sm()
                            .text_color(gpui::black())
                            .child(error)
                    })
                    .children(hits)
                    .children(text_under)
                    .children(overlay)
                    .children(text_over),
            );
        }
        self.trim_cache(visible, window);

        let quick_bar = if self.companion {
            None
        } else {
            tools::viewer_overlay(self, window, cx)
        };
        root.children(pages)
            .children(self.scrollbar(&palette, cx))
            .children(quick_bar)
    }
}

impl DocTab {
    fn set_bounds(&mut self, bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        if self.view.bounds == bounds {
            return;
        }
        self.view.bounds = bounds;
        if self.view.fit_pending && bounds.size.width > px(0.) && self.page_count() > 0 {
            self.view.fit_pending = false;
            let zoom =
                crate::geometry::fit_width_zoom(&self.page_sizes, f32::from(bounds.size.width));
            self.view.zoom = zoom.clamp(0.25, 1.5);
        }
        // Re-clamp the scroll offset to the new size.
        self.scroll_by((0.0, 0.0));
        cx.notify();
    }

    fn on_wheel(&mut self, event: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (lines_x, lines_y) = match event.delta {
            ScrollDelta::Lines(lines) => (lines.x, lines.y),
            ScrollDelta::Pixels(pixels) => (
                f32::from(pixels.x) / LINE_HEIGHT,
                f32::from(pixels.y) / LINE_HEIGHT,
            ),
        };
        if event.modifiers.control {
            let notches = (lines_y / LINES_PER_NOTCH).clamp(-3.0, 3.0);
            let origin = self.view.bounds.origin;
            let anchor = (
                f32::from(event.position.x - origin.x),
                f32::from(event.position.y - origin.y),
            );
            self.set_zoom(self.view.zoom * ZOOM_STEP.powf(notches), Some(anchor));
        } else {
            self.scroll_by((-lines_x * LINE_HEIGHT, -lines_y * LINE_HEIGHT));
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Thumb top and height in view space, `None` when everything fits.
    fn thumb(&self) -> Option<(f32, f32)> {
        let layout = self.layout();
        let view_height = f32::from(self.view.bounds.size.height);
        let max_scroll = layout.max_scroll((0.0, view_height)).1;
        if max_scroll <= 0.0 || view_height <= 0.0 {
            return None;
        }
        let height = (view_height * view_height / layout.content_height).max(MIN_THUMB);
        let top = self.view.scroll.1 / max_scroll * (view_height - height);
        Some((top, height))
    }

    fn on_drag_thumb(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (Some(drag), Some((_, height))) = (self.thumb_drag, self.thumb()) else {
            return;
        };
        if !event.dragging() {
            self.thumb_drag = None;
            return;
        }
        let view_height = f32::from(self.view.bounds.size.height);
        let max_scroll = self.layout().max_scroll((0.0, view_height)).1;
        let top = f32::from(event.position.y - self.view.bounds.origin.y) - drag.grab_offset;
        let fraction = top / (view_height - height).max(1.0);
        self.scroll_by((0.0, fraction * max_scroll - self.view.scroll.1));
        cx.notify();
    }

    fn scrollbar(&self, palette: &Palette, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let (top, height) = self.thumb()?;
        let thumb_color = if self.thumb_drag.is_some() {
            palette.text_dim
        } else {
            palette.text_dim.opacity(0.35)
        };
        Some(
            div()
                .id("scrollbar")
                .absolute()
                .top_0()
                .right_0()
                .h_full()
                .w(px(SCROLLBAR_WIDTH + 4.))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |tab, event: &MouseDownEvent, _, cx| {
                        let y = f32::from(event.position.y - tab.view.bounds.origin.y);
                        // Grabbing the thumb keeps its offset; clicking the track centers it.
                        let grab_offset = if (top..top + height).contains(&y) {
                            y - top
                        } else {
                            height / 2.0
                        };
                        tab.thumb_drag = Some(ThumbDrag { grab_offset });
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(top))
                        .right(px(2.))
                        .w(px(SCROLLBAR_WIDTH - 4.))
                        .h(px(height))
                        // Half the 6 px side, less 0.5: see quickbar's `circle_radius`.
                        .rounded(px(2.5))
                        .bg(thumb_color),
                ),
        )
    }
}
