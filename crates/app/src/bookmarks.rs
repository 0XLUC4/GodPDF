//! Bookmarks panel: the tree with page numbers, a click goes to the page; bookmarks are
//! added, renamed, moved, nested and deleted right there.

use egui::{Align, Id, Key, Layout, RichText, ScrollArea, Sense, TextEdit, Ui, vec2};
use godpdf_core::OutlineItem;

use crate::commands::Cmd;
use crate::doc::DocTab;
use crate::theme::Palette;
use crate::widgets::{self, icon, with_icon};

/// Where a bookmark is in the tree: its index at each level.
type Path = Vec<usize>;

/// What the panel asks the app to do.
#[derive(Default)]
pub struct Output {
    pub jump: Option<usize>,
    /// New bookmarks to write, with the name of the change for the undo history.
    pub write: Option<(&'static str, Vec<OutlineItem>)>,
    pub run: Option<Cmd>,
}

/// A bookmark being renamed.
pub struct Renaming {
    path: Path,
    text: String,
    focused: bool,
}

impl Renaming {
    fn of(path: &[usize], title: &str) -> Self {
        Self {
            path: path.to_vec(),
            text: title.to_owned(),
            focused: false,
        }
    }
}

/// One change made from a row.
enum Change {
    Rename(Path, String),
    SetPage(Path),
    AddInside(Path),
    MoveUp(Path),
    MoveDown(Path),
    Indent(Path),
    Outdent(Path),
    Delete(Path),
}

pub fn show(ui: &mut Ui, tab: &mut DocTab) -> Output {
    let read = tab.outline.get_or_insert_with(|| {
        godpdf_core::outline(tab.session.bytes()).map_err(|err| err.to_string())
    });
    let items = match read {
        Ok(items) => items.clone(),
        Err(err) => {
            let palette = Palette::of(ui);
            ui.label(RichText::new(format!("Cannot read bookmarks: {err}")).color(palette.error));
            return Output::default();
        }
    };
    let mut out = Output::default();
    let current = tab.current_page;
    let page_count = tab.page_count();
    let bytes = tab.session.bytes();
    let page_title = || first_line(bytes, current);

    ui.horizontal(|ui| {
        let add = ui
            .button(with_icon(icon::BOOKMARK_SIMPLE, "Add"))
            .on_hover_text(format!("Bookmark page {}", current + 1));
        if add.clicked() {
            out.run = Some(Cmd::AddBookmark);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.menu_button(icon::DOTS_THREE_VERTICAL, |ui| {
                if ui
                    .button(with_icon(icon::LIST_BULLETS, "Contents page"))
                    .on_hover_text("A first page listing the bookmarks, each line a link")
                    .clicked()
                {
                    out.run = Some(Cmd::ContentsPage);
                }
                if ui
                    .button(with_icon(icon::FILES, "One bookmark per page"))
                    .clicked()
                {
                    let all = (0..page_count)
                        .map(|page| OutlineItem {
                            title: format!("Page {}", page + 1),
                            page: Some(page),
                            children: Vec::new(),
                        })
                        .collect();
                    out.write = Some(("Bookmark every page", all));
                }
                let clear = egui::Button::new(with_icon(icon::TRASH, "Remove all bookmarks"));
                if ui.add_enabled(!items.is_empty(), clear).clicked() {
                    out.write = Some(("Remove all bookmarks", Vec::new()));
                }
            });
        });
    });
    ui.add_space(6.0);

    if items.is_empty() {
        let palette = Palette::of(ui);
        ui.label(
            RichText::new("No bookmarks yet. “Add” marks the page you are reading.")
                .color(palette.text_dim),
        );
        return out;
    }

    let active = active_path(&items, current);
    let mut change = None;
    ScrollArea::vertical()
        .id_salt(("outline", tab.id))
        .auto_shrink(false)
        .show(ui, |ui| {
            let mut rows = Rows {
                active: active.as_deref(),
                renaming: &mut tab.bookmark_rename,
                jump: &mut out.jump,
                change: &mut change,
                current,
            };
            rows.level(ui, &items, &mut Vec::new(), Id::new(("outline", tab.id)));
        });

    if let Some(change) = change {
        let mut next = items;
        if let Some(label) = apply(&mut next, change, current, &page_title) {
            out.write = Some((label, next));
        }
    }
    out
}

/// The bookmarks of the tab with one more at the end, for the page being read, named after
/// its first line; its name is ready to be typed over in the panel.
pub fn appended(tab: &mut DocTab) -> Result<Vec<OutlineItem>, String> {
    let mut items = godpdf_core::outline(tab.session.bytes()).map_err(|err| err.to_string())?;
    let title = first_line(tab.session.bytes(), tab.current_page);
    tab.bookmark_rename = Some(Renaming::of(&[items.len()], &title));
    items.push(OutlineItem {
        title,
        page: Some(tab.current_page),
        children: Vec::new(),
    });
    Ok(items)
}

struct Rows<'a> {
    active: Option<&'a [usize]>,
    renaming: &'a mut Option<Renaming>,
    jump: &'a mut Option<usize>,
    change: &'a mut Option<Change>,
    current: usize,
}

impl Rows<'_> {
    fn level(&mut self, ui: &mut Ui, items: &[OutlineItem], path: &mut Path, parent: Id) {
        for (index, item) in items.iter().enumerate() {
            path.push(index);
            let id = parent.with(index);
            if item.children.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(ui.spacing().indent);
                    self.row(ui, item, path, items.len());
                });
            } else {
                // The branch holding the bookmark of the page being read starts open.
                let open = self.active.is_some_and(|active| active.starts_with(path));
                egui::collapsing_header::CollapsingState::load_with_default_open(
                    ui.ctx(),
                    id,
                    open,
                )
                .show_header(ui, |ui| self.row(ui, item, path, items.len()))
                .body(|ui| self.level(ui, &item.children, path, id));
            }
            path.pop();
        }
    }

    fn row(&mut self, ui: &mut Ui, item: &OutlineItem, path: &[usize], siblings: usize) {
        if self.renaming.as_ref().is_some_and(|r| r.path == path) {
            self.rename_field(ui, item, path);
            return;
        }
        let palette = Palette::of(ui);
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
        if self.active == Some(path) {
            ui.painter().rect_filled(rect, 4.0, palette.selected);
        } else if response.hovered() {
            ui.painter().rect_filled(rect, 4.0, palette.hover);
        }
        let number = item
            .page
            .map_or_else(|| "–".to_owned(), |page| (page + 1).to_string());
        let number =
            ui.painter()
                .layout_no_wrap(number, egui::FontId::proportional(12.5), palette.text_dim);
        let number_pos = egui::pos2(
            rect.right() - 6.0 - number.size().x,
            rect.center().y - number.size().y / 2.0,
        );
        // One line: what does not fit is cut, the whole title shows on hover.
        let mut job = egui::text::LayoutJob::simple_singleline(
            item.title.clone(),
            egui::FontId::proportional(14.0),
            palette.text,
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(
            (number_pos.x - rect.left() - 14.0).max(10.0),
        );
        let title = ui.fonts_mut(|fonts| fonts.layout_job(job));
        let title_pos = egui::pos2(rect.left() + 6.0, rect.center().y - title.size().y / 2.0);
        ui.painter().galley(title_pos, title, palette.text);
        ui.painter().galley(number_pos, number, palette.text_dim);

        let hint = match item.page {
            Some(page) => format!("{}\nPage {} · double-click to rename", item.title, page + 1),
            None => format!("{}\nGoes nowhere", item.title),
        };
        let response = response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(hint);
        if response.double_clicked() {
            *self.renaming = Some(Renaming::of(path, &item.title));
        } else if response.clicked()
            && let Some(page) = item.page
        {
            *self.jump = Some(page);
        }
        response.context_menu(|ui| self.menu(ui, item, path, siblings));
    }

    fn rename_field(&mut self, ui: &mut Ui, item: &OutlineItem, path: &[usize]) {
        let Some(renaming) = self.renaming.as_mut() else {
            return;
        };
        let edit = ui.add(
            TextEdit::singleline(&mut renaming.text)
                .margin(widgets::FIELD_MARGIN)
                .desired_width(ui.available_width()),
        );
        if !renaming.focused {
            edit.request_focus();
            renaming.focused = true;
        }
        let (enter, escape) = ui.input(|input| {
            (
                input.key_pressed(Key::Enter),
                input.key_pressed(Key::Escape),
            )
        });
        if escape {
            *self.renaming = None;
        } else if enter || edit.lost_focus() {
            let text = renaming.text.trim().to_owned();
            if !text.is_empty() && text != item.title {
                *self.change = Some(Change::Rename(path.to_vec(), text));
            }
            *self.renaming = None;
        }
    }

    fn menu(&mut self, ui: &mut Ui, item: &OutlineItem, path: &[usize], siblings: usize) {
        let index = path[path.len() - 1];
        let current = self.current;
        if ui
            .button(with_icon(icon::PENCIL_SIMPLE, "Rename"))
            .clicked()
        {
            *self.renaming = Some(Renaming::of(path, &item.title));
            ui.close();
        }
        let path = path.to_vec();
        let entries = [
            None,
            Some((
                item.page != Some(current),
                icon::CROSSHAIR,
                format!("Point to page {}", current + 1),
                Change::SetPage(path.clone()),
            )),
            Some((
                true,
                icon::PLUS,
                format!("Add inside, for page {}", current + 1),
                Change::AddInside(path.clone()),
            )),
            None,
            Some((
                index > 0,
                icon::ARROW_UP,
                "Move up".to_owned(),
                Change::MoveUp(path.clone()),
            )),
            Some((
                index + 1 < siblings,
                icon::ARROW_DOWN,
                "Move down".to_owned(),
                Change::MoveDown(path.clone()),
            )),
            Some((
                index > 0,
                icon::ARROW_RIGHT,
                "Put inside the one above".to_owned(),
                Change::Indent(path.clone()),
            )),
            Some((
                path.len() > 1,
                icon::ARROW_LEFT,
                "Take out one level".to_owned(),
                Change::Outdent(path.clone()),
            )),
            None,
            Some((true, icon::TRASH, "Delete".to_owned(), Change::Delete(path))),
        ];
        for entry in entries {
            let Some((enabled, glyph, label, change)) = entry else {
                ui.separator();
                continue;
            };
            let button = egui::Button::new(with_icon(glyph, &label));
            if ui.add_enabled(enabled, button).clicked() {
                *self.change = Some(change);
                ui.close();
            }
        }
    }
}

/// Apply `change` to `items`; returns the name of the change, `None` when nothing changed.
fn apply(
    items: &mut Vec<OutlineItem>,
    change: Change,
    current: usize,
    page_title: &dyn Fn() -> String,
) -> Option<&'static str> {
    match change {
        Change::Rename(path, title) => {
            item_mut(items, &path)?.title = title;
            Some("Rename bookmark")
        }
        Change::SetPage(path) => {
            item_mut(items, &path)?.page = Some(current);
            Some("Move bookmark to page")
        }
        Change::AddInside(path) => {
            item_mut(items, &path)?.children.push(OutlineItem {
                title: page_title(),
                page: Some(current),
                children: Vec::new(),
            });
            Some("Add bookmark")
        }
        Change::MoveUp(path) => {
            let (index, siblings) = siblings_mut(items, &path)?;
            if index == 0 {
                return None;
            }
            siblings.swap(index, index - 1);
            Some("Move bookmark")
        }
        Change::MoveDown(path) => {
            let (index, siblings) = siblings_mut(items, &path)?;
            if index + 1 >= siblings.len() {
                return None;
            }
            siblings.swap(index, index + 1);
            Some("Move bookmark")
        }
        Change::Indent(path) => {
            let (index, siblings) = siblings_mut(items, &path)?;
            if index == 0 {
                return None;
            }
            let moved = siblings.remove(index);
            siblings[index - 1].children.push(moved);
            Some("Nest bookmark")
        }
        Change::Outdent(path) => {
            let (&index, parent) = path.split_last()?;
            let (parent_index, uncles) = siblings_mut(items, parent)?;
            let moved = uncles[parent_index].children.remove(index);
            uncles.insert(parent_index + 1, moved);
            Some("Unnest bookmark")
        }
        Change::Delete(path) => {
            let (index, siblings) = siblings_mut(items, &path)?;
            siblings.remove(index);
            Some("Delete bookmark")
        }
    }
}

fn item_mut<'a>(items: &'a mut [OutlineItem], path: &[usize]) -> Option<&'a mut OutlineItem> {
    let (&first, rest) = path.split_first()?;
    let item = items.get_mut(first)?;
    if rest.is_empty() {
        Some(item)
    } else {
        item_mut(&mut item.children, rest)
    }
}

/// The list holding the bookmark at `path`, and its index there.
fn siblings_mut<'a>(
    items: &'a mut Vec<OutlineItem>,
    path: &[usize],
) -> Option<(usize, &'a mut Vec<OutlineItem>)> {
    let (&last, parents) = path.split_last()?;
    let list = if parents.is_empty() {
        items
    } else {
        &mut item_mut(items, parents)?.children
    };
    (last < list.len()).then_some((last, list))
}

/// The bookmark of the part being read: the one leading to the nearest page at or before
/// `page`, the last in reading order when several lead there.
fn active_path(items: &[OutlineItem], page: usize) -> Option<Path> {
    fn walk(items: &[OutlineItem], page: usize, path: &mut Path, best: &mut Option<(usize, Path)>) {
        for (index, item) in items.iter().enumerate() {
            path.push(index);
            if let Some(target) = item.page
                && target <= page
                && best.as_ref().is_none_or(|(found, _)| target >= *found)
            {
                *best = Some((target, path.clone()));
            }
            walk(&item.children, page, path, best);
            path.pop();
        }
    }
    let mut best = None;
    walk(items, page, &mut Vec::new(), &mut best);
    best.map(|(_, path)| path)
}

/// The first line of text of `page`, to name a new bookmark; "Page N" when it has none.
fn first_line(pdf: &[u8], page: usize) -> String {
    let line = godpdf_core::extract_text(pdf, page).ok().and_then(|text| {
        text.lines()
            .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
            .find(|line| line.chars().filter(|c| c.is_alphanumeric()).count() >= 3)
    });
    match line {
        Some(line) if line.chars().count() > 60 => {
            let cut: String = line.chars().take(57).collect();
            format!("{}…", cut.trim_end())
        }
        Some(line) => line,
        None => format!("Page {}", page + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(title: &str, page: usize, children: Vec<OutlineItem>) -> OutlineItem {
        OutlineItem {
            title: title.to_owned(),
            page: Some(page),
            children,
        }
    }

    fn titles(items: &[OutlineItem]) -> String {
        items
            .iter()
            .map(|item| {
                if item.children.is_empty() {
                    item.title.clone()
                } else {
                    format!("{}({})", item.title, titles(&item.children))
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn tree() -> Vec<OutlineItem> {
        vec![
            item("A", 0, vec![]),
            item("B", 2, vec![item("B1", 3, vec![]), item("B2", 5, vec![])]),
            item("C", 8, vec![]),
        ]
    }

    fn changed(change: Change) -> Option<String> {
        let mut items = tree();
        apply(&mut items, change, 4, &|| "New".to_owned())?;
        Some(titles(&items))
    }

    #[test]
    fn rows_move_nest_and_go() {
        let expect = |change, text: &str| assert_eq!(changed(change).as_deref(), Some(text));
        expect(Change::MoveUp(vec![1]), "B(B1 B2) A C");
        expect(Change::MoveDown(vec![1, 0]), "A B(B2 B1) C");
        expect(Change::Indent(vec![2]), "A B(B1 B2 C)");
        expect(Change::Outdent(vec![1, 0]), "A B(B2) B1 C");
        expect(Change::Delete(vec![1]), "A C");
        expect(Change::AddInside(vec![0]), "A(New) B(B1 B2) C");
        expect(Change::Rename(vec![2], "Z".into()), "A B(B1 B2) Z");
    }

    #[test]
    fn impossible_moves_change_nothing() {
        assert!(changed(Change::MoveUp(vec![0])).is_none());
        assert!(changed(Change::MoveDown(vec![2])).is_none());
        assert!(changed(Change::Indent(vec![0])).is_none());
        assert!(changed(Change::Outdent(vec![1])).is_none());
    }

    #[test]
    fn the_part_being_read_is_found() {
        let items = tree();
        assert_eq!(active_path(&items, 4), Some(vec![1, 0]));
        assert_eq!(active_path(&items, 9), Some(vec![2]));
        assert_eq!(active_path(&items, 2), Some(vec![1]));
    }
}
