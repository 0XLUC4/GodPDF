//! Page grid: multi-select, drag-and-drop reorder, page toolbar.

use egui::{
    Align2, Color32, CornerRadius, FontId, Id, Rect, RichText, ScrollArea, Sense, Stroke,
    StrokeKind, Ui, pos2, vec2,
};

use crate::commands::Cmd;
use crate::doc::{Budget, DocTab};
use crate::theme::Palette;
use crate::widgets::{self, icon};

const CARD: egui::Vec2 = vec2(168.0, 218.0);
const THUMB_BOX: egui::Vec2 = vec2(144.0, 180.0);

#[derive(Default)]
pub struct Output {
    pub cmd: Option<Cmd>,
    /// New page order (a permutation) after a drop, and the new indices of the moved pages.
    pub reorder: Option<(Vec<usize>, Vec<usize>)>,
    /// Double-clicked page: open it in the reader.
    pub open_page: Option<usize>,
    /// Arrow clicked on a card: move that page by -1 / +1.
    pub move_page: Option<(usize, isize)>,
}

const TOOLBAR: [Cmd; 8] = [
    Cmd::RotateLeft,
    Cmd::RotateRight,
    Cmd::MoveEarlier,
    Cmd::MoveLater,
    Cmd::DeletePages,
    Cmd::Extract,
    Cmd::InsertBlank,
    Cmd::InsertFile,
];

pub fn show(ui: &mut Ui, tab: &mut DocTab, budget: &mut Budget) -> Output {
    let palette = Palette::of(ui);
    let mut out = Output::default();
    let count = tab.page_count();

    egui::Frame::new()
        .fill(palette.panel)
        .inner_margin(egui::Margin::symmetric(16, 10))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for cmd in TOOLBAR {
                    let button = egui::Button::new(widgets::with_icon(cmd.icon(), cmd.label()))
                        .min_size(vec2(0.0, widgets::ICON_BUTTON.y));
                    let response = ui.add(button).on_hover_text(cmd.description());
                    if response.clicked() {
                        out.cmd = Some(cmd);
                    }
                }
                ui.separator();
                if widgets::icon_button(ui, icon::SELECTION_ALL, "Select all").clicked() {
                    tab.selection = (0..count).collect();
                }
                if widgets::icon_button(ui, icon::SELECTION_SLASH, "Clear selection").clicked() {
                    tab.selection.clear();
                }
                ui.label(
                    RichText::new(format!("{} of {count} selected", tab.selection.len()))
                        .color(palette.text_dim),
                );
            });
        });

    ScrollArea::vertical()
        .id_salt(("organize", tab.id))
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.add_space(16.0);
            ui.horizontal_wrapped(|ui| {
                ui.add_space(12.0);
                ui.spacing_mut().item_spacing = vec2(14.0, 14.0);
                for page in 0..count {
                    card(ui, tab, page, budget, &palette, &mut out);
                }
            });
            ui.add_space(24.0);
        });
    if budget.exhausted() {
        ui.ctx().request_repaint();
    }
    out
}

fn card(
    ui: &mut Ui,
    tab: &mut DocTab,
    page: usize,
    budget: &mut Budget,
    palette: &Palette,
    out: &mut Output,
) {
    let (rect, _) = ui.allocate_exact_size(CARD, Sense::hover());
    let response = ui.interact(
        rect,
        Id::new(("org-card", tab.id, page)),
        Sense::click_and_drag(),
    );
    response.dnd_set_drag_payload(page);

    let selected = tab.selection.contains(&page);
    let painter = ui.painter();
    let fill = if response.hovered() {
        palette.hover
    } else {
        palette.surface
    };
    let stroke = if selected {
        Stroke::new(2.0, palette.accent)
    } else {
        Stroke::new(1.0, palette.border)
    };
    painter.rect(
        rect,
        CornerRadius::same(10),
        fill,
        stroke,
        StrokeKind::Inside,
    );

    let thumb_area = Rect::from_center_size(
        pos2(rect.center().x, rect.min.y + 12.0 + THUMB_BOX.y / 2.0),
        THUMB_BOX,
    );
    if !ui.is_rect_visible(rect) {
        // Skip rendering off-screen thumbnails.
    } else if let Some(texture) = tab.thumbnail(ui.ctx(), page, budget) {
        let size = texture.size_vec2();
        let scale = (THUMB_BOX.x / size.x).min(THUMB_BOX.y / size.y);
        let image_rect = Rect::from_center_size(thumb_area.center(), size * scale);
        let painter = ui.painter();
        painter.rect_filled(image_rect, 0.0, Color32::WHITE);
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        painter.image(texture.id(), image_rect, uv, Color32::WHITE);
    } else {
        ui.painter()
            .rect_filled(thumb_area.shrink(18.0), 2.0, palette.canvas);
    }
    ui.painter().text(
        pos2(rect.center().x, rect.max.y - 14.0),
        Align2::CENTER_CENTER,
        format!("{}", page + 1),
        FontId::proportional(13.0),
        if selected {
            palette.accent_text
        } else {
            palette.text_dim
        },
    );

    if (selected || ui.rect_contains_pointer(rect)) && !response.dragged() {
        card_arrows(ui, rect, page, tab.page_count(), out);
    }

    if response.double_clicked() {
        out.open_page = Some(page);
    } else if response.clicked() {
        select(tab, page, ui.input(|input| input.modifiers));
    }

    let pointer_x = ui
        .ctx()
        .pointer_hover_pos()
        .map_or(rect.center().x, |pos| pos.x);
    let after = pointer_x > rect.center().x;
    if response.dnd_hover_payload::<usize>().is_some() {
        let x = if after {
            rect.max.x + 7.0
        } else {
            rect.min.x - 7.0
        };
        ui.painter().line_segment(
            [pos2(x, rect.min.y), pos2(x, rect.max.y)],
            Stroke::new(3.0, palette.accent),
        );
    }
    if let Some(dragged) = response.dnd_release_payload::<usize>()
        && let Some(reordered) = reorder(tab, *dragged, page, after)
    {
        // The app selects the moved pages once the reorder succeeded.
        out.reorder = Some(reordered);
    }
}

/// Small earlier/later arrows in the card's bottom corners.
fn card_arrows(ui: &mut Ui, rect: Rect, page: usize, count: usize, out: &mut Output) {
    let size = widgets::SMALL_ICON_BUTTON;
    let y = rect.max.y - 8.0 - size.y;
    let arrows = [
        (
            page > 0,
            rect.min.x + 8.0,
            icon::CARET_LEFT,
            "Move earlier",
            -1,
        ),
        (
            page + 1 < count,
            rect.max.x - 8.0 - size.x,
            icon::CARET_RIGHT,
            "Move later",
            1,
        ),
    ];
    for (shown, x, glyph, tooltip, delta) in arrows {
        if !shown {
            continue;
        }
        let area = Rect::from_min_size(pos2(x, y), size);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area));
        if widgets::small_icon_button(&mut child, glyph, tooltip).clicked() {
            out.move_page = Some((page, delta));
        }
    }
}

/// New page order after moving every `selected` page one place (`delta` < 0: earlier,
/// > 0: later). Blocks move together; pages already at an end stay put.
pub fn move_selection(count: usize, selected: &[usize], delta: isize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..count).collect();
    let picked = |page: usize| selected.contains(&page);
    if delta < 0 {
        for index in 1..count {
            if picked(order[index]) && !picked(order[index - 1]) {
                order.swap(index - 1, index);
            }
        }
    } else if delta > 0 {
        for index in (0..count.saturating_sub(1)).rev() {
            if picked(order[index]) && !picked(order[index + 1]) {
                order.swap(index, index + 1);
            }
        }
    }
    order
}

fn select(tab: &mut DocTab, page: usize, modifiers: egui::Modifiers) {
    if modifiers.shift {
        let anchor = tab.anchor.unwrap_or(page);
        tab.selection = (anchor.min(page)..=anchor.max(page)).collect();
        return;
    }
    if modifiers.command {
        if !tab.selection.remove(&page) {
            tab.selection.insert(page);
        }
    } else {
        tab.selection.clear();
        tab.selection.insert(page);
    }
    tab.anchor = Some(page);
    tab.current_page = page;
}

/// Move the dragged page (or the whole selection containing it) next to `target`.
/// Returns the new order and the new indices of the moved pages; `None` if nothing changes.
fn reorder(
    tab: &DocTab,
    dragged: usize,
    target: usize,
    after: bool,
) -> Option<(Vec<usize>, Vec<usize>)> {
    let moved: Vec<usize> = if tab.selection.contains(&dragged) {
        tab.selection.iter().copied().collect()
    } else {
        vec![dragged]
    };
    if moved.contains(&target) {
        return None;
    }
    let mut order: Vec<usize> = (0..tab.page_count())
        .filter(|page| !moved.contains(page))
        .collect();
    let at = order.iter().position(|&page| page == target)? + usize::from(after);
    order.splice(at..at, moved.iter().copied());
    if order.iter().enumerate().all(|(index, &page)| index == page) {
        return None;
    }
    Some((order, (at..at + moved.len()).collect()))
}

#[cfg(test)]
mod tests {
    use super::move_selection;

    #[test]
    fn moves_single_page() {
        assert_eq!(move_selection(4, &[2], -1), vec![0, 2, 1, 3]);
        assert_eq!(move_selection(4, &[2], 1), vec![0, 1, 3, 2]);
    }

    #[test]
    fn moves_block_together() {
        assert_eq!(move_selection(4, &[1, 2], -1), vec![1, 2, 0, 3]);
        assert_eq!(move_selection(4, &[1, 2], 1), vec![0, 3, 1, 2]);
    }

    #[test]
    fn moves_scattered_pages() {
        assert_eq!(move_selection(5, &[1, 3], -1), vec![1, 0, 3, 2, 4]);
        assert_eq!(move_selection(5, &[1, 3], 1), vec![0, 2, 1, 4, 3]);
    }

    #[test]
    fn clamps_at_ends() {
        assert_eq!(move_selection(3, &[0], -1), vec![0, 1, 2]);
        assert_eq!(move_selection(3, &[0, 2], -1), vec![0, 2, 1]);
        assert_eq!(move_selection(3, &[2], 1), vec![0, 1, 2]);
        assert_eq!(move_selection(0, &[], 1), Vec::<usize>::new());
    }
}
