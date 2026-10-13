//! Continuous vertical page viewer with zoom and comment gestures.

use egui::{
    Color32, CornerRadius, CursorIcon, Id, Pos2, Rect, RichText, ScrollArea, Sense, Shadow, Stroke,
    StrokeKind, Ui, pos2, vec2,
};

use crate::commands::CommentTool;
use crate::doc::{Budget, DocTab, Gesture};
use crate::lineedit::{self, LineEdit};
use crate::quickbar::{Styles, Tool};
use crate::selection;
use crate::textbox::{self, TextBox};
use crate::theme::Palette;
use crate::tools::CommentStyle;

pub const PAD: f32 = 28.0;
pub const GAP: f32 = 18.0;

/// A finished comment gesture, in displayed page points (origin top-left).
pub struct Finished {
    pub page: usize,
    pub points: Vec<Pos2>,
}

#[derive(Default)]
pub struct Output {
    pub gesture: Option<Finished>,
    /// Page and point (displayed page points) where a text box should open: a click with the
    /// Free text tool, or a double-click without tool (`new_allowed` false).
    pub open_text: Option<(usize, Pos2, bool)>,
    pub text_event: Option<textbox::Event>,
    /// Select tool: an annotation was moved or opened.
    pub select: Option<selection::Action>,
    /// Highlight tool over text: (page, one rectangle per line, displayed points).
    pub highlight: Option<(usize, Vec<Rect>)>,
    /// Eraser tool: (page, annotation id) to remove.
    pub erase: Option<(usize, String)>,
    /// First and last page on screen: the others can give their textures back.
    pub visible: Option<(usize, usize)>,
    /// Edit-text mode: a line was clicked. Its page, its box (displayed points), its words.
    pub edit_line: Option<(usize, Rect, String)>,
    pub line_event: Option<lineedit::Event>,
}

pub struct Editors<'a> {
    pub text_box: Option<&'a mut TextBox>,
    pub line: Option<&'a mut LineEdit>,
}

/// Zoom that fits the widest page in the viewer width.
pub fn fit_width_zoom(tab: &DocTab) -> f32 {
    (tab.viewport.x - 2.0 * PAD - 14.0) / tab.max_page_width()
}

pub fn fit_page_zoom(tab: &DocTab) -> f32 {
    let height = tab
        .page_sizes
        .get(tab.current_page)
        .map_or(792.0, |size| size.y);
    fit_width_zoom(tab).min((tab.viewport.y - 2.0 * PAD) / height.max(1.0))
}

pub fn show(
    ui: &mut Ui,
    tab: &mut DocTab,
    tool: Tool,
    editors: Editors<'_>,
    styles: &Styles,
    budget: &mut Budget,
    edit_text: bool,
) -> Output {
    let palette = Palette::of(ui);
    let Editors {
        mut text_box,
        line: mut line_edit,
    } = editors;
    tab.viewport = ui.available_size();
    if tab.fit_pending && tab.page_count() > 0 {
        tab.fit_pending = false;
        tab.zoom = fit_width_zoom(tab).clamp(0.25, 1.5);
    }

    if let Some(err) = &tab.render_error {
        ui.centered_and_justified(|ui| {
            ui.label(
                RichText::new(format!("Cannot display this PDF:\n{err}")).color(palette.error),
            );
        });
        return Output::default();
    }

    let zoom_delta = ui.input(|input| input.zoom_delta());
    if zoom_delta != 1.0 && ui.rect_contains_pointer(ui.max_rect()) {
        tab.set_zoom(tab.zoom * zoom_delta);
    }

    let zoom = tab.zoom;
    let mut tops = Vec::with_capacity(tab.page_count());
    let mut y = PAD;
    for size in &tab.page_sizes {
        tops.push(y);
        y += size.y * zoom + GAP;
    }
    let total_height = y - GAP + PAD;
    let content_width = tab.max_page_width() * zoom + 2.0 * PAD;

    let mut area = ScrollArea::both()
        .id_salt(("viewer", tab.id, tab.second))
        .scroll_source(egui::scroll_area::ScrollSource {
            mouse_wheel: false,
            ..Default::default()
        })
        .auto_shrink(false);
    if let Some(page) = tab.scroll_to_page.take() {
        if let Some(top) = tops.get(page) {
            area = area.vertical_scroll_offset((top - PAD / 2.0).max(0.0));
        }
    } else if let Some(offset) = tab.pending_scroll.take() {
        area = area.vertical_scroll_offset(offset.max(0.0));
    }

    let ppp = ui.ctx().pixels_per_point();
    let mut out = Output::default();
    let output = area.show_viewport(ui, |ui, viewport| {
        let width = content_width.max(ui.available_width());
        let origin = ui.max_rect().min;
        ui.allocate_rect(
            Rect::from_min_size(origin, vec2(width, total_height)),
            Sense::hover(),
        );
        if ui.rect_contains_pointer(ui.clip_rect()) {
            let delta = wheel_delta(ui, viewport.height());
            if delta != egui::Vec2::ZERO {
                ui.scroll_with_delta(delta);
            }
        }
        if tool == Tool::Hand {
            let pan = ui
                .interact(
                    ui.clip_rect(),
                    Id::new(("pan", tab.id, tab.second)),
                    Sense::drag(),
                )
                .on_hover_cursor(CursorIcon::Grab);
            if pan.dragged() {
                ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                ui.scroll_with_delta(pan.drag_delta());
            }
        }

        let mut visible: Option<(usize, usize)> = None;
        let probe = viewport.min.y + viewport.height() * 0.35;
        for (page, &top) in tops.iter().enumerate() {
            let size = tab.page_sizes[page] * zoom;
            let local = Rect::from_min_size(pos2((width - size.x) / 2.0, top), size);
            if local.min.y <= probe {
                tab.current_page = page;
            }
            let rect = local.translate(origin.to_vec2());
            if viewport.expand(GAP).intersects(local) {
                visible = Some(visible.map_or((page, page), |(first, _)| (first, page)));
                let texture =
                    paint_page(ui, tab, page, rect, scale_for(zoom, ppp), budget, &palette);
                paint_search_hits(ui, tab, page, rect);
                // While a text box is open, clicks only edit or commit it.
                let editing = text_box.is_some() || line_edit.is_some();
                match tool {
                    // Changing the words of the page comes before any tool.
                    _ if edit_text => {
                        if !editing {
                            let clicked = text_line(ui, tab, page, rect, &palette);
                            out.edit_line = clicked.or(out.edit_line.take());
                        }
                    }
                    Tool::Hand => {}
                    Tool::Select if !editing => {
                        if let Some(action) = selection::page(ui, tab, page, rect, texture) {
                            out.select = Some(action);
                        }
                    }
                    Tool::Eraser if !editing => {
                        if let Some(id) = selection::erase(ui, tab, page, rect) {
                            out.erase = Some((page, id));
                        }
                    }
                    Tool::Select | Tool::Eraser => {}
                    Tool::Text => {
                        if let Some(point) = page_click(ui, tab, page, rect) {
                            out.open_text = Some((page, point, true));
                        }
                    }
                    Tool::Highlight => {
                        let style = &styles.highlight;
                        match selection::highlight_text(ui, tab, page, rect, style.color) {
                            selection::TextDrag::Done(bands) => {
                                out.highlight = Some((page, bands));
                            }
                            selection::TextDrag::Active => {}
                            // Not on text: drag a free rectangle.
                            selection::TextDrag::Off => {
                                let tool = CommentTool::Highlight;
                                if let Some(done) = interact(ui, tab, page, rect, tool, style) {
                                    out.gesture = Some(done);
                                }
                            }
                        }
                    }
                    Tool::Note | Tool::Ink | Tool::Rectangle => {
                        if let Some(comment) = tool.comment()
                            && let Some(done) =
                                interact(ui, tab, page, rect, comment, styles.get(comment))
                        {
                            out.gesture = Some(done);
                        }
                    }
                }
            }
            // The box is shown even off-screen so its text field keeps keyboard focus.
            if let Some(line) = line_edit.as_deref_mut()
                && line.page == page
            {
                out.line_event = lineedit::show(ui, line, rect, zoom).or(out.line_event);
            }
            if let Some(text_box) = text_box.as_deref_mut()
                && text_box.page == page
            {
                out.text_event =
                    textbox::show(ui, text_box, rect, zoom, &styles.text).or(out.text_event);
            }
        }
        out.visible = visible;
    });
    tab.scroll_y = output.state.offset.y;
    if budget.exhausted() {
        ui.ctx().request_repaint();
    }
    out
}

fn wheel_delta(ui: &Ui, page_height: f32) -> egui::Vec2 {
    let line = ui
        .ctx()
        .options(|options| options.input_options.line_scroll_speed);
    ui.input(|input| {
        input
            .events
            .iter()
            .filter_map(|event| match event {
                egui::Event::MouseWheel {
                    unit,
                    delta,
                    modifiers,
                    ..
                } if !modifiers.command && !modifiers.ctrl => {
                    let delta = match unit {
                        egui::MouseWheelUnit::Point => *delta,
                        egui::MouseWheelUnit::Line => *delta * line,
                        egui::MouseWheelUnit::Page => *delta * page_height,
                    };
                    Some(if modifiers.shift {
                        vec2(delta.x + delta.y, 0.0)
                    } else {
                        delta
                    })
                }
                _ => None,
            })
            .fold(egui::Vec2::ZERO, |total, delta| total + delta)
    })
}

fn scale_for(zoom: f32, ppp: f32) -> f32 {
    zoom * ppp
}

fn paint_page(
    ui: &Ui,
    tab: &mut DocTab,
    page: usize,
    rect: Rect,
    scale: f32,
    budget: &mut Budget,
    palette: &Palette,
) -> Option<egui::TextureId> {
    let painter = ui.painter();
    let shadow = Shadow {
        offset: [0, 4],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(90),
    };
    painter.add(shadow.as_shape(rect, CornerRadius::same(2)));
    painter.rect_filled(rect, CornerRadius::same(2), Color32::WHITE);

    let texture = tab
        .page_texture(ui.ctx(), page, scale, budget)
        .map(|tex| (tex.id(), tex.size_vec2()));
    if let Some((texture, pixels)) = texture {
        // Drawn pixel for pixel: a page that starts between two pixels of the screen, or is
        // stretched by a fraction of one, has every letter smeared over its neighbours.
        let ppp = ui.ctx().pixels_per_point();
        let on_pixel = |at: f32| (at * ppp).round() / ppp;
        let min = pos2(on_pixel(rect.min.x), on_pixel(rect.min.y));
        // A page drawn for another zoom, shown while the right one is made, is stretched.
        let own = pixels / ppp;
        let fits = (own.x - rect.width()).abs() <= 1.0 && (own.y - rect.height()).abs() <= 1.0;
        let size = if fits { own } else { rect.size() };
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        painter.image(texture, Rect::from_min_size(min, size), uv, Color32::WHITE);
        // A page too large to be drawn whole this big is stretched: what shows of it is
        // drawn again over it, sharp.
        let zoom = scale / ppp;
        let seen = ui
            .clip_rect()
            .intersect(rect)
            .translate(-rect.min.to_vec2());
        let seen = Rect::from_min_max(
            (seen.min.to_vec2() / zoom).to_pos2(),
            (seen.max.to_vec2() / zoom).to_pos2(),
        );
        if let Some((detail, pixels, part)) = tab.page_detail(ui.ctx(), page, scale, seen, budget) {
            let corner = min + part.min.to_vec2() * zoom;
            let corner = pos2(on_pixel(corner.x), on_pixel(corner.y));
            // Made for another zoom, it is stretched until the right one comes.
            let (own, shown) = (pixels / ppp, part.size() * zoom);
            let fits = (own.x - shown.x).abs() <= 1.0 && (own.y - shown.y).abs() <= 1.0;
            let size = if fits { own } else { shown };
            painter.image(
                detail,
                Rect::from_min_size(corner, size),
                uv,
                Color32::WHITE,
            );
        }
    } else {
        let text = tab.page_error(page).map_or_else(
            || format!("Rendering page {}…", page + 1),
            |err| format!("Page {} failed to render:\n{err}", page + 1),
        );
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(14.0),
            palette.text_dim,
        );
    }
    texture.map(|(texture, _)| texture)
}

/// Edit-text mode: the line under the pointer is framed, and a click asks to change it.
/// Returns its page, its box in displayed points and its words.
fn text_line(
    ui: &mut Ui,
    tab: &mut DocTab,
    page: usize,
    rect: Rect,
    palette: &Palette,
) -> Option<(usize, Rect, String)> {
    let id = Id::new(("page-edit-text", tab.id, page));
    let response = ui.interact(rect, id, Sense::click());
    let chars = tab.text_chars(page);
    let zoom = tab.zoom;
    let pointer = ((response.hover_pos()? - rect.min) / zoom).to_pos2();
    let line = crate::textsel::line_of(&chars, crate::textsel::char_near(&chars, pointer)?);
    let bands = crate::textsel::bands(&chars, line.clone());
    let band = bands.into_iter().reduce(|all, band| all.union(band))?;
    let frame = Rect::from_min_size(rect.min + band.min.to_vec2() * zoom, band.size() * zoom);
    let stroke = Stroke::new(1.5, palette.accent);
    ui.painter()
        .rect_stroke(frame.expand(2.0), 2.0, stroke, StrokeKind::Outside);
    ui.ctx().set_cursor_icon(CursorIcon::Text);
    response
        .clicked()
        .then(|| (page, band, crate::textsel::text(&chars, line)))
}

/// Mark every place the searched words are written on `page`, while the search shows them.
fn paint_search_hits(ui: &Ui, tab: &mut DocTab, page: usize, rect: Rect) {
    let Some(words) = tab.search.marked().map(str::to_owned) else {
        return;
    };
    let chars = tab.text_chars(page);
    let zoom = tab.zoom;
    let fill = Color32::from_rgba_unmultiplied(255, 196, 0, 96);
    for found in crate::textsel::find(&chars, &words) {
        for band in crate::textsel::bands(&chars, found) {
            let min = rect.min + band.min.to_vec2() * zoom;
            let band = Rect::from_min_size(min, band.size() * zoom);
            ui.painter().rect_filled(band.expand(1.0), 2.0, fill);
        }
    }
}

/// Click on a page with the Text tool, in displayed page points.
fn page_click(ui: &mut Ui, tab: &DocTab, page: usize, rect: Rect) -> Option<Pos2> {
    let response = ui
        .interact(rect, Id::new(("page-click", tab.id, page)), Sense::click())
        .on_hover_cursor(CursorIcon::Text);
    let pos = response
        .interact_pointer_pos()
        .filter(|_| response.clicked())?;
    Some(((pos - rect.min) / tab.zoom).to_pos2())
}

fn interact(
    ui: &mut Ui,
    tab: &mut DocTab,
    page: usize,
    rect: Rect,
    tool: CommentTool,
    style: &CommentStyle,
) -> Option<Finished> {
    let response = ui
        .interact(
            rect,
            Id::new(("page-input", tab.id, page)),
            Sense::click_and_drag(),
        )
        .on_hover_cursor(CursorIcon::Crosshair);
    let zoom = tab.zoom;
    let to_page = |pos: Pos2| {
        let local = (pos - rect.min) / zoom;
        pos2(
            local.x.clamp(0.0, rect.width() / zoom),
            local.y.clamp(0.0, rect.height() / zoom),
        )
    };

    if tool == CommentTool::Note && response.clicked() {
        let pos = response.interact_pointer_pos()?;
        return Some(Finished {
            page,
            points: vec![to_page(pos)],
        });
    }
    if response.drag_started()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let origin = ui
            .input(|input| input.pointer.press_origin())
            .filter(|origin| rect.contains(*origin))
            .unwrap_or(pos);
        let mut points = vec![to_page(origin)];
        if tool == CommentTool::Ink && origin != pos {
            points.push(to_page(pos));
        }
        tab.gesture = Some(Gesture { page, points });
    }
    let gesture = tab
        .gesture
        .as_mut()
        .filter(|gesture| gesture.page == page)?;
    if response.dragged() && tool == CommentTool::Ink {
        let moves: Vec<Pos2> = ui.input(|input| {
            input
                .events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::PointerMoved(pos) => Some(*pos),
                    _ => None,
                })
                .collect()
        });
        let moves = if moves.is_empty() {
            response.interact_pointer_pos().into_iter().collect()
        } else {
            moves
        };
        for pos in moves {
            let point = to_page(pos);
            if gesture.points.last() != Some(&point) {
                gesture.points.push(point);
            }
        }
    } else if response.dragged()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let point = to_page(pos);
        if gesture.points.len() < 2 {
            gesture.points.push(point);
        } else if let Some(last) = gesture.points.last_mut() {
            *last = point;
        }
    }
    paint_preview(ui, rect.min, zoom, &gesture.points, tool, style);
    if response.drag_stopped() {
        let gesture = tab.gesture.take()?;
        return Some(Finished {
            page,
            points: gesture.points,
        });
    }
    None
}

/// Show the comment being drawn the way it will look once written.
fn paint_preview(
    ui: &Ui,
    origin: Pos2,
    zoom: f32,
    points: &[Pos2],
    tool: CommentTool,
    style: &CommentStyle,
) {
    let to_screen = |p: &Pos2| origin + p.to_vec2() * zoom;
    let painter = ui.painter();
    let stroke = Stroke::new(style.width * zoom, style.color);
    if tool == CommentTool::Ink {
        painter.line(points.iter().map(to_screen).collect(), stroke);
        return;
    }
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return;
    };
    let rect = Rect::from_two_pos(to_screen(first), to_screen(last));
    if tool == CommentTool::Highlight {
        let fill = style.color.gamma_multiply(selection::HIGHLIGHT_OPACITY);
        painter.rect_filled(rect, 0.0, fill);
    } else {
        painter.rect_stroke(rect, 0.0, stroke, StrokeKind::Inside);
    }
}
