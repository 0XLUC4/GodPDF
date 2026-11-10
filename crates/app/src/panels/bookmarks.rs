//! Bookmarks panel: the tree with page numbers;
//! a click goes to the page, a double-click renames, right-click moves, nests or deletes.

use std::collections::HashMap;

use godpdf_core::OutlineItem;
use gpui::component::button::{Button, ButtonVariants};
use gpui::component::input::{Escape, Input, InputEvent, InputState};
use gpui::component::menu::{ContextMenuExt, DropdownMenu, PopupMenu};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, Subscription, WeakEntity, Window, div, px,
};

use super::{AddBookmark, label, menu_entry};
use crate::doc::{DocTab, guarded};
use crate::theme::{self, Palette};
use crate::{fonts, icons, toasts, widgets};

/// Where a bookmark is in the tree: its index at each level.
type Path = Vec<usize>;

/// Indent of each tree level.
const INDENT: f32 = 18.;

/// Bookmark state of one document.
#[derive(Default)]
pub struct Marks {
    /// The outline as read at a document generation.
    outline: Option<(u64, Result<Vec<OutlineItem>, String>)>,
    renaming: Option<Renaming>,
    /// Branches opened or closed by a click on their arrow; the others start open only when
    /// they hold the bookmark of the page being read.
    toggled: HashMap<Path, bool>,
}

/// Is the branch at `path` open: as last clicked, else when it holds the `active` bookmark.
fn is_open(toggled: &HashMap<Path, bool>, active: Option<&[usize]>, path: &[usize]) -> bool {
    toggled
        .get(path)
        .copied()
        .unwrap_or_else(|| active.is_some_and(|active| active.starts_with(path)))
}

/// A bookmark being renamed; its field is created on the next render (it needs a window).
struct Renaming {
    path: Path,
    title: String,
    field: Option<(Entity<InputState>, Subscription)>,
}

impl Renaming {
    fn of(path: &[usize], title: &str) -> Self {
        Self {
            path: path.to_vec(),
            title: title.to_owned(),
            field: None,
        }
    }
}

/// One change made from a row.
#[derive(Clone, Debug)]
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

/// The bookmarks of `tab`, read once per document generation.
fn outline(tab: &mut DocTab) -> Result<Vec<OutlineItem>, String> {
    let generation = tab.generation();
    let marks = &mut tab.panels.bookmarks;
    if marks
        .outline
        .as_ref()
        .is_none_or(|(read, _)| *read != generation)
    {
        let read =
            guarded(|| godpdf_core::outline(tab.session.bytes())).map_err(|err| err.to_string());
        tab.panels.bookmarks.outline = Some((generation, read));
    }
    match &tab.panels.bookmarks.outline {
        Some((_, read)) => read.clone(),
        None => Ok(Vec::new()),
    }
}

/// Write `items` as the bookmarks of `tab`; errors become toasts.
fn write(tab: &mut DocTab, label: &str, items: &[OutlineItem], cx: &mut Context<DocTab>) -> bool {
    match tab.apply(label, |pdf| godpdf_core::write_outline(pdf, items)) {
        Ok(()) => true,
        Err(err) => {
            toasts::error(cx, err);
            false
        }
    }
}

/// Ctrl+D: one more bookmark at the end, for the page being read, named after its first
/// line; its name is ready to be typed over.
pub fn add_for_current_page(tab: &mut DocTab, cx: &mut Context<DocTab>) {
    let mut items = match outline(tab) {
        Ok(items) => items,
        Err(err) => {
            toasts::error(cx, format!("Cannot read bookmarks: {err}"));
            return;
        }
    };
    let page = tab.view.current_page;
    let title = first_line(tab.session.bytes(), page);
    let path = vec![items.len()];
    items.push(OutlineItem {
        title: title.clone(),
        page: Some(page),
        children: Vec::new(),
    });
    if write(tab, "Add bookmark", &items, cx) {
        tab.panels.bookmarks.renaming = Some(Renaming::of(&path, &title));
    }
}

fn run_change(tab: &WeakEntity<DocTab>, change: Change, cx: &mut App) {
    let _ = tab.update(cx, |tab, cx| {
        let Ok(mut items) = outline(tab) else { return };
        let current = tab.view.current_page;
        let bytes = tab.session.bytes();
        let page_title = || first_line(bytes, current);
        if let Some(label) = apply(&mut items, change, current, &page_title) {
            write(tab, label, &items, cx);
        }
        cx.notify();
    });
}

/// Replace all bookmarks (menu entries of the panel header).
fn replace_all(tab: &WeakEntity<DocTab>, label: &'static str, every_page: bool, cx: &mut App) {
    let _ = tab.update(cx, |tab, cx| {
        let items: Vec<OutlineItem> = if every_page {
            (0..tab.page_count())
                .map(|page| OutlineItem {
                    title: format!("Page {}", page + 1),
                    page: Some(page),
                    children: Vec::new(),
                })
                .collect()
        } else {
            Vec::new()
        };
        write(tab, label, &items, cx);
        cx.notify();
    });
}

fn contents_page(tab: &WeakEntity<DocTab>, cx: &mut App) {
    let _ = tab.update(cx, |tab, cx| {
        match tab.apply("Contents page", |pdf| {
            godpdf_core::contents_page(pdf, "Contents")
        }) {
            Ok(()) => {
                toasts::success(cx, "Contents page added: each line leads to its page");
                tab.scroll_to_page(0);
            }
            Err(err) => toasts::error(cx, err),
        }
        cx.notify();
    });
}

pub fn render(tab: &Entity<DocTab>, window: &mut Window, cx: &mut App) -> AnyElement {
    let palette = Palette::of(cx);
    let weak = tab.downgrade();
    let (read, current) = tab.update(cx, |tab, _| (outline(tab), tab.view.current_page));
    let field = rename_field(tab, window, cx);
    let more_of = weak.clone();
    let has_items = read.as_ref().is_ok_and(|items| !items.is_empty());
    // A framed "Add" button (BOOKMARK_SIMPLE), a framed menu button
    // (DOTS_THREE_VERTICAL) on the right, then 6 px.
    let (surface, border, hover) = (palette.surface, palette.border, palette.hover);
    let framed = |button: gpui::Stateful<gpui::Div>| {
        button
            .h(px(theme::INTERACT_HEIGHT))
            .px(px(theme::BUTTON_PADDING.0))
            .flex()
            .items_center()
            .rounded(px(theme::RADIUS))
            .bg(surface)
            .border_1()
            .border_color(border)
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
    };
    let add = widgets::tooltip(
        framed(div().id("bookmark-add"))
            .child(widgets::with_icon(
                icons::BOOKMARK_SIMPLE,
                "Add",
                fonts::BODY,
                palette.text,
            ))
            .on_click(|_, window, cx| window.dispatch_action(Box::new(AddBookmark), cx)),
        format!("Bookmark page {}", current + 1),
    );
    let menu_palette = palette;
    let more = Button::new("bookmark-more")
        .ghost()
        .h(px(theme::INTERACT_HEIGHT))
        .px(px(theme::BUTTON_PADDING.0))
        .rounded(px(theme::RADIUS))
        .bg(surface)
        .border_1()
        .border_color(border)
        .child(widgets::icon(
            icons::DOTS_THREE_VERTICAL,
            fonts::BODY,
            palette.text,
        ))
        .dropdown_menu(move |menu, _, _| {
            let item = |glyph: &'static str,
                        label: &'static str,
                        enabled: bool,
                        run: fn(&WeakEntity<DocTab>, &mut App)| {
                let tab = more_of.clone();
                menu_entry(glyph, label, enabled, &menu_palette)
                    .on_click(move |_, _, cx| run(&tab, cx))
            };
            menu.item(item(
                icons::LIST_BULLETS,
                "Contents page",
                true,
                contents_page,
            ))
            .item(item(
                icons::FILES,
                "One bookmark per page",
                true,
                |tab, cx| replace_all(tab, "Bookmark every page", true, cx),
            ))
            .item(item(
                icons::TRASH,
                "Remove all bookmarks",
                has_items,
                |tab, cx| replace_all(tab, "Remove all bookmarks", false, cx),
            ))
        });
    let header = div()
        .flex()
        .items_center()
        .justify_between()
        .mb(px(6.))
        .child(add)
        .child(more);
    let items = match read {
        Ok(items) => items,
        Err(err) => {
            return div()
                .child(header)
                .child(label(
                    format!("Cannot read bookmarks: {err}"),
                    palette.error,
                ))
                .into_any_element();
        }
    };
    if items.is_empty() {
        return div()
            .child(header)
            .child(label(
                "No bookmarks yet. “Add” marks the page you are reading.",
                palette.text_dim,
            ))
            .into_any_element();
    }
    let active = active_path(&items, current);
    let toggled = tab.read(cx).panels.bookmarks.toggled.clone();
    let mut rows = Vec::new();
    let open = |path: &[usize]| is_open(&toggled, active.as_deref(), path);
    shown(
        &items,
        &mut Vec::new(),
        &open,
        &mut |path, item, siblings| {
            let renaming = field
                .as_ref()
                .filter(|(at, _)| at == path)
                .map(|(_, field)| field.clone());
            let arrow = (!item.children.is_empty()).then(|| open(path));
            rows.push(row(
                &weak,
                path,
                item,
                siblings,
                current,
                active.as_deref() == Some(path),
                arrow,
                renaming,
                &palette,
            ));
        },
    );
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(header)
        .child(
            div()
                .id("bookmarks")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(px(6.))
                .children(rows),
        )
        .into_any_element()
}

/// Every bookmark in reading order with its path and the size of its sibling list.
fn flatten(
    items: &[OutlineItem],
    path: &mut Path,
    visit: &mut impl FnMut(&[usize], &OutlineItem, usize),
) {
    for (index, item) in items.iter().enumerate() {
        path.push(index);
        visit(path, item, items.len());
        flatten(&item.children, path, visit);
        path.pop();
    }
}

/// Like [`flatten`], but the children of closed branches are skipped.
fn shown(
    items: &[OutlineItem],
    path: &mut Path,
    open: &dyn Fn(&[usize]) -> bool,
    visit: &mut impl FnMut(&[usize], &OutlineItem, usize),
) {
    for (index, item) in items.iter().enumerate() {
        path.push(index);
        visit(path, item, items.len());
        if open(path) {
            shown(&item.children, path, open, visit);
        }
        path.pop();
    }
}

/// The field of the bookmark being renamed, created and focused the first time it shows.
fn rename_field(
    tab: &Entity<DocTab>,
    window: &mut Window,
    cx: &mut App,
) -> Option<(Path, Entity<InputState>)> {
    let weak = tab.downgrade();
    let (path, title, existing) = {
        let renaming = tab.read(cx).panels.bookmarks.renaming.as_ref()?;
        let existing = renaming.field.as_ref().map(|(field, _)| field.clone());
        (renaming.path.clone(), renaming.title.clone(), existing)
    };
    if let Some(field) = existing {
        return Some((path, field));
    }
    let field = cx.new(|cx| InputState::new(window, cx).default_value(title));
    field.update(cx, |field, cx| field.focus(window, cx));
    let subscription = cx.subscribe(&field, move |field, event: &InputEvent, cx| {
        if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
            let text = field.read(cx).value().trim().to_owned();
            finish_rename(&weak, text, cx);
        }
    });
    tab.update(cx, |tab, _| {
        if let Some(renaming) = tab.panels.bookmarks.renaming.as_mut() {
            renaming.field = Some((field.clone(), subscription));
        }
    });
    Some((path, field))
}

/// Enter or leaving the field: rename when the text changed.
fn finish_rename(tab: &WeakEntity<DocTab>, text: String, cx: &mut App) {
    let Some(renaming) = tab
        .update(cx, |tab, _| tab.panels.bookmarks.renaming.take())
        .ok()
        .flatten()
    else {
        return;
    };
    if !text.is_empty() && text != renaming.title {
        run_change(tab, Change::Rename(renaming.path, text), cx);
    } else {
        let _ = tab.update(cx, |_, cx| cx.notify());
    }
}

#[allow(clippy::too_many_arguments)]
fn row(
    tab: &WeakEntity<DocTab>,
    path: &[usize],
    item: &OutlineItem,
    siblings: usize,
    current: usize,
    active: bool,
    arrow: Option<bool>,
    renaming: Option<Entity<InputState>>,
    palette: &Palette,
) -> AnyElement {
    // An `indent` wide slot (a branch's open / close arrow, blank for a leaf), the body
    // of a branch one `indent` further in.
    let indent = px(INDENT * (path.len() - 1) as f32);
    let slot = div()
        .flex_none()
        .w(px(INDENT))
        .h(px(24.))
        .flex()
        .items_center()
        .justify_center();
    if let Some(field) = renaming {
        // Esc gives the old name back: the rename is dropped before the field loses focus.
        let cancel_tab = tab.clone();
        return div()
            .flex()
            .pl(indent)
            .child(slot)
            .child(
                div()
                    .flex_1()
                    .on_action(move |_: &Escape, _, cx| {
                        let _ = cancel_tab.update(cx, |tab, cx| {
                            tab.panels.bookmarks.renaming = None;
                            cx.notify();
                        });
                    })
                    .child(Input::new(&field).h(px(24.))),
            )
            .into_any_element();
    }
    let slot = match arrow {
        Some(open) => {
            let (arrow_tab, arrow_path) = (tab.clone(), path.to_vec());
            slot.id(gpui::ElementId::Name(
                format!("bookmark-arrow-{path:?}").into(),
            ))
            .cursor_pointer()
            .child(widgets::icon(
                if open {
                    icons::CARET_DOWN
                } else {
                    icons::CARET_RIGHT
                },
                11.0,
                palette.text,
            ))
            .on_click(move |_, _, cx| {
                let _ = arrow_tab.update(cx, |tab, cx| {
                    tab.panels
                        .bookmarks
                        .toggled
                        .insert(arrow_path.clone(), !open);
                    cx.notify();
                });
            })
            .into_any_element()
        }
        None => slot.into_any_element(),
    };
    let hover = palette.hover;
    let number = item
        .page
        .map_or_else(|| "–".to_owned(), |page| (page + 1).to_string());
    let (click_tab, path_owned) = (tab.clone(), path.to_vec());
    let (title, page) = (item.title.clone(), item.page);
    let (menu_tab, menu_path, menu_item) = (tab.clone(), path.to_vec(), item.clone());
    let menu_palette = *palette;
    let hint = match item.page {
        Some(page) => format!(
            "{}
Page {} · double-click to rename",
            item.title,
            page + 1
        ),
        None => format!(
            "{}
Goes nowhere",
            item.title
        ),
    };
    let row = div()
        .id(gpui::ElementId::Name(format!("bookmark-{path:?}").into()))
        .flex_1()
        .min_w_0()
        .h(px(24.))
        .px(px(6.))
        .flex()
        .items_center()
        .gap(px(8.))
        .rounded(px(4.))
        .text_size(px(14.))
        .cursor_pointer()
        .when(active, |row| row.bg(palette.selected))
        .when(!active, |row| row.hover(move |style| style.bg(hover)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(item.title.clone()),
        )
        .child(
            div()
                .flex_none()
                .text_size(px(12.5))
                .text_color(palette.text_dim)
                .child(number),
        );
    let row = widgets::tooltip(row, hint)
        .on_click(move |event: &ClickEvent, _, cx| {
            let _ = click_tab.update(cx, |tab, cx| {
                if event.click_count() >= 2 {
                    tab.panels.bookmarks.renaming = Some(Renaming::of(&path_owned, &title));
                } else if let Some(page) = page {
                    tab.scroll_to_page(page);
                }
                cx.notify();
            });
        })
        .context_menu(move |menu, _, _| {
            bookmark_menu(
                menu,
                &menu_tab,
                &menu_path,
                &menu_item,
                siblings,
                current,
                &menu_palette,
            )
        });
    div()
        .flex_none()
        .flex()
        .pl(indent)
        .child(slot)
        .child(row)
        .into_any_element()
}

fn bookmark_menu(
    menu: PopupMenu,
    tab: &WeakEntity<DocTab>,
    path: &[usize],
    item: &OutlineItem,
    siblings: usize,
    current: usize,
    palette: &Palette,
) -> PopupMenu {
    let index = path[path.len() - 1];
    let entry = |label: String, glyph: &'static str, enabled: bool, change: Change| {
        let tab = tab.clone();
        menu_entry(glyph, label, enabled, palette)
            .on_click(move |_, _, cx| run_change(&tab, change.clone(), cx))
    };
    let rename_tab = tab.clone();
    let (rename_path, rename_title) = (path.to_vec(), item.title.clone());
    let path = path.to_vec();
    menu.item(
        menu_entry(icons::PENCIL_SIMPLE, "Rename", true, palette).on_click(move |_, _, cx| {
            let _ = rename_tab.update(cx, |tab, cx| {
                tab.panels.bookmarks.renaming = Some(Renaming::of(&rename_path, &rename_title));
                cx.notify();
            });
        }),
    )
    .separator()
    .item(entry(
        format!("Point to page {}", current + 1),
        icons::CROSSHAIR,
        item.page != Some(current),
        Change::SetPage(path.clone()),
    ))
    .item(entry(
        format!("Add inside, for page {}", current + 1),
        icons::PLUS,
        true,
        Change::AddInside(path.clone()),
    ))
    .separator()
    .item(entry(
        "Move up".into(),
        icons::ARROW_UP,
        index > 0,
        Change::MoveUp(path.clone()),
    ))
    .item(entry(
        "Move down".into(),
        icons::ARROW_DOWN,
        index + 1 < siblings,
        Change::MoveDown(path.clone()),
    ))
    .item(entry(
        "Put inside the one above".into(),
        icons::ARROW_RIGHT,
        index > 0,
        Change::Indent(path.clone()),
    ))
    .item(entry(
        "Take out one level".into(),
        icons::ARROW_LEFT,
        path.len() > 1,
        Change::Outdent(path.clone()),
    ))
    .separator()
    .item(entry(
        "Delete".into(),
        icons::TRASH,
        true,
        Change::Delete(path),
    ))
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
            if index >= uncles[parent_index].children.len() {
                return None;
            }
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
    let mut best: Option<(usize, Path)> = None;
    flatten(items, &mut Vec::new(), &mut |path, item, _| {
        if let Some(target) = item.page
            && target <= page
            && best.as_ref().is_none_or(|(found, _)| target >= *found)
        {
            best = Some((target, path.to_vec()));
        }
    });
    best.map(|(_, path)| path)
}

/// The first line of text of `page`, to name a new bookmark; "Page N" when it has none.
fn first_line(pdf: &[u8], page: usize) -> String {
    let line = guarded(|| godpdf_core::extract_text(pdf, page))
        .ok()
        .and_then(|text| {
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
        assert!(changed(Change::Delete(vec![7])).is_none());
    }

    #[test]
    fn branches_open_on_the_page_read_or_as_clicked() {
        let items = tree();
        let active = active_path(&items, 4);
        let mut toggled = HashMap::new();
        let count = |toggled: &HashMap<Path, bool>| {
            let mut rows = 0;
            let open = |path: &[usize]| is_open(toggled, active.as_deref(), path);
            shown(&items, &mut Vec::new(), &open, &mut |_, _, _| rows += 1);
            rows
        };
        assert_eq!(count(&toggled), 5);
        toggled.insert(vec![1], false);
        assert_eq!(count(&toggled), 3);
        assert!(!is_open(&toggled, None, &[2]));
    }

    #[test]
    fn the_part_being_read_is_found() {
        let items = tree();
        assert_eq!(active_path(&items, 4), Some(vec![1, 0]));
        assert_eq!(active_path(&items, 9), Some(vec![2]));
        assert_eq!(active_path(&items, 2), Some(vec![1]));
    }
}
