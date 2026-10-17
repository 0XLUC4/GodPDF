//! Right panel views: page thumbnails, bookmarks (drawn by `bookmarks`), text search
//! (comments live in `comments`).

use egui::{
    Color32, CornerRadius, Id, Rect, RichText, ScrollArea, Sense, Stroke, StrokeKind, Ui, pos2,
    vec2,
};
use godpdf_core::OutlineItem;

use crate::doc::{Budget, DocTab};
use crate::theme::Palette;
use crate::widgets::{self, icon};

/// View shown in the right panel, picked from the icon rail.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Comments,
    Pages,
    Bookmarks,
    Search,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Comments, Tab::Pages, Tab::Bookmarks, Tab::Search];

    pub fn title(self) -> &'static str {
        match self {
            Self::Comments => "Comments",
            Self::Pages => "Pages",
            Self::Bookmarks => "Bookmarks",
            Self::Search => "Search",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Comments => icon::CHAT_TEXT,
            Self::Pages => icon::FILES,
            Self::Bookmarks => icon::BOOKMARK_SIMPLE,
            Self::Search => icon::MAGNIFYING_GLASS,
        }
    }
}

pub const SEARCH_ID: &str = "sidebar-search";

#[derive(Default)]
pub struct Output {
    /// Page to jump to.
    pub jump: Option<usize>,
    /// Arrow clicked on a thumbnail: move that page by -1 / +1.
    pub move_page: Option<(usize, isize)>,
    /// New bookmarks to write, named for the undo history.
    pub outline: Option<(&'static str, Vec<OutlineItem>)>,
    pub run: Option<crate::commands::Cmd>,
}

/// Pages, bookmarks or search (`Tab::Comments` is drawn by the caller).
pub fn show(ui: &mut Ui, tab: &mut DocTab, current: Tab, budget: &mut Budget) -> Output {
    match current {
        Tab::Pages => pages(ui, tab, budget),
        Tab::Bookmarks => {
            let marks = crate::bookmarks::show(ui, tab);
            Output {
                jump: marks.jump,
                outline: marks.write,
                run: marks.run,
                ..Output::default()
            }
        }
        Tab::Search => Output {
            jump: search(ui, tab),
            ..Output::default()
        },
        Tab::Comments => Output::default(),
    }
}

fn pages(ui: &mut Ui, tab: &mut DocTab, budget: &mut Budget) -> Output {
    let palette = Palette::of(ui);
    let row_height = 196.0;
    let count = tab.page_count();
    let mut out = Output::default();
    ScrollArea::vertical()
        .id_salt(("thumbs", tab.id))
        .auto_shrink(false)
        .show_rows(ui, row_height, tab.page_count(), |ui, rows| {
            for page in rows {
                let width = ui.available_width();
                let (rect, response) =
                    ui.allocate_exact_size(vec2(width, row_height), Sense::click());
                let current = page == tab.current_page;
                if current || response.hovered() {
                    let fill = if current {
                        palette.accent.gamma_multiply(0.18)
                    } else {
                        palette.hover
                    };
                    ui.painter()
                        .rect_filled(rect.shrink(2.0), CornerRadius::same(8), fill);
                }
                let box_rect = Rect::from_center_size(
                    pos2(rect.center().x, rect.min.y + 84.0),
                    vec2(width - 32.0, 156.0),
                );
                if let Some(texture) = tab.thumbnail(ui.ctx(), page, budget) {
                    let size = texture.size_vec2();
                    let scale = (box_rect.width() / size.x).min(box_rect.height() / size.y);
                    let image_rect = Rect::from_center_size(box_rect.center(), size * scale);
                    let painter = ui.painter();
                    painter.rect_filled(image_rect, 0.0, Color32::WHITE);
                    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                    painter.image(texture.id(), image_rect, uv, Color32::WHITE);
                    if current {
                        painter.rect_stroke(
                            image_rect,
                            0.0,
                            Stroke::new(2.0, palette.accent),
                            StrokeKind::Outside,
                        );
                    }
                } else {
                    ui.painter()
                        .rect_filled(box_rect.shrink(24.0), 2.0, palette.canvas);
                }
                ui.painter().text(
                    pos2(rect.center().x, rect.max.y - 16.0),
                    egui::Align2::CENTER_CENTER,
                    format!("{}", page + 1),
                    egui::FontId::proportional(13.0),
                    if current {
                        palette.accent_text
                    } else {
                        palette.text_dim
                    },
                );
                if ui.rect_contains_pointer(rect) {
                    let size = widgets::SMALL_ICON_BUTTON;
                    let x = rect.max.x - 10.0 - size.x;
                    let arrows = [
                        (page > 0, rect.min.y + 8.0, icon::CARET_UP, "Move up", -1),
                        (
                            page + 1 < count,
                            rect.min.y + 12.0 + size.y,
                            icon::CARET_DOWN,
                            "Move down",
                            1,
                        ),
                    ];
                    for (shown, y, glyph, tooltip, delta) in arrows {
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
                if response.clicked() {
                    out.jump = Some(page);
                }
            }
        });
    if budget.exhausted() {
        ui.ctx().request_repaint();
    }
    out
}

fn search(ui: &mut Ui, tab: &mut DocTab) -> Option<usize> {
    let palette = Palette::of(ui);
    let mut run = false;
    ui.horizontal(|ui| {
        let edit = egui::TextEdit::singleline(&mut tab.search.query)
            .margin(crate::widgets::FIELD_MARGIN)
            .id(Id::new(SEARCH_ID))
            .hint_text("Search text…")
            .desired_width(ui.available_width() - 40.0);
        let response = ui.add(edit);
        run = response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        run |= widgets::icon_button(ui, icon::MAGNIFYING_GLASS, "Search").clicked();
    });
    let query = tab.search.query.trim().to_owned();
    if run && !query.is_empty() && !tab.search.busy {
        // The app runs the search in the background.
        tab.search.request = Some(query);
    }

    if tab.search.busy || tab.search.request.is_some() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new("Searching…").color(palette.text_dim));
        });
        return None;
    }
    if let Some(err) = &tab.search.error {
        ui.label(RichText::new(err).color(palette.error));
        return None;
    }
    if tab.search.searched.is_empty() {
        ui.label(RichText::new("Press Enter to search all pages.").color(palette.text_dim));
        return None;
    }
    let total: usize = tab.search.hits.iter().map(|hit| hit.count).sum();
    ui.label(
        RichText::new(format!(
            "{total} matches on {} pages for “{}”",
            tab.search.hits.len(),
            tab.search.searched
        ))
        .color(palette.text_dim),
    );
    ui.add_space(4.0);
    let mut jump = None;
    ScrollArea::vertical()
        .id_salt(("hits", tab.id))
        .auto_shrink(false)
        .show(ui, |ui| {
            for hit in &tab.search.hits {
                let text = format!("Page {}   ·   {} ×", hit.page + 1, hit.count);
                let selected = hit.page == tab.current_page;
                if ui
                    .add_sized(
                        [ui.available_width(), 26.0],
                        egui::Button::selectable(selected, text),
                    )
                    .clicked()
                {
                    jump = Some(hit.page);
                }
            }
        });
    jump
}
