//! Tab row: home tab, one tab per document,
//! "+ Open", then updates / about / theme on the right. Fill `surface`, margin 8 / 8 / 4 / 0,
//! no line under it: the active tab merges into the header below.

use std::path::{Path, PathBuf};

use gpui::component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClipboardItem, Context, Div, MouseButton, SharedString, Stateful, WeakEntity,
    Window, div, px,
};

use super::{ReopenTab, TAB_HEIGHT};
use crate::app::{GodPdfApp, Open, ToggleTheme};
use crate::theme::Palette;
use crate::widgets::{self, ICON_BUTTON, SMALL_ICON_BUTTON, dispatch, icon};
use crate::{fonts, icons, toasts};

/// Longest document name shown in a tab before "…".
const TAB_TEXT_MAX: f32 = 180.0;

impl GodPdfApp {
    /// Menu > File > Reopen closed tab, Ctrl+Shift+T.
    pub fn reopen_tab(&mut self, _: &ReopenTab, _: &mut Window, cx: &mut Context<Self>) {
        match self.shell.closed.pop() {
            Some(path) => self.open_path(&path, cx),
            None => toasts::info(cx, "No closed tab to open again"),
        }
    }

    /// Close every other document that has nothing to save.
    fn close_others(&mut self, keep: usize, cx: &mut Context<Self>) {
        let Some(kept) = self.tabs.get(keep).cloned() else {
            return;
        };
        let mut unsaved = 0;
        let mut index = 0;
        while index < self.tabs.len() {
            let tab = &self.tabs[index];
            if *tab == kept {
                index += 1;
            } else if tab.read(cx).is_dirty() {
                unsaved += 1;
                index += 1;
            } else {
                self.remove_tab(index, cx);
            }
        }
        self.active = self.tabs.iter().position(|tab| *tab == kept).unwrap_or(0);
        self.shell.home = false;
        if unsaved > 0 {
            toasts::info(
                cx,
                format!("{unsaved} document(s) with unsaved changes stay open"),
            );
        }
        cx.notify();
    }
}

pub fn render(
    app: &GodPdfApp,
    palette: &Palette,
    window: &Window,
    cx: &mut Context<GodPdfApp>,
) -> AnyElement {
    let doc_view = app.doc_view();
    let home = tab_shape("home-tab", !doc_view, palette)
        .w(px(40.))
        .child(
            content()
                .w_full()
                .justify_center()
                .child(icon(icons::HOUSE, 16.0, palette.text)),
        )
        .on_click(cx.listener(|app, _, _, cx| {
            if !app.tabs.is_empty() {
                app.shell.home = true;
                cx.notify();
            }
        }));
    let home = widgets::tooltip(home, "Home");

    // Share what is left between the tabs; names shrink before anything overflows (the
    // row width less margins 8 + 8, the 2 px lead and the home tab with its spacing).
    let available = f32::from(window.viewport_size().width) - 16.0 - 2.0 - 40.0 - 2.0;
    let reserved = 2.0 * ICON_BUTTON + 100.0;
    let chrome = 12.0 + 16.0 + 8.0 + 8.0 + SMALL_ICON_BUTTON + 22.0;
    let per_tab = (available - reserved) / app.tabs.len().max(1) as f32;
    let text_max = (per_tab - chrome).clamp(40.0, TAB_TEXT_MAX);

    let state = TabMenu {
        app: cx.weak_entity(),
        count: app.tabs.len(),
        reopen: !app.shell.closed.is_empty(),
    };
    let tabs: Vec<_> = app
        .tabs
        .iter()
        .enumerate()
        .map(|(index, tab)| {
            let tab = tab.read(cx);
            let active = doc_view && index == app.active;
            let title: SharedString = tab.title().to_owned().into();
            let path = tab.session.path.clone();
            (index, title, tab.is_dirty(), active, path)
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|(index, title, dirty, active, path)| {
            doc_tab(index, title, dirty, active, text_max, palette, cx)
                .context_menu({
                    let state = state.clone();
                    move |menu, _, cx| state.build(menu, index, path.clone(), cx)
                })
                .into_any_element()
        })
        .collect();

    let open = widgets::tooltip(
        widgets::button("open", "", palette)
            .child(widgets::with_icon(icons::PLUS, "Open", 14.0, palette.text))
            .on_click(dispatch(Open)),
        "Open PDF (Ctrl+O)",
    );

    let dark = Palette::is_dark(cx);
    let (glyph, tip) = if dark {
        (icons::SUN, "Light mode")
    } else {
        (icons::MOON, "Dark mode")
    };
    let right = div()
        .ml_auto()
        .flex()
        .items_center()
        .gap(px(2.))
        .child(crate::services::updates_button(palette, cx))
        .child(
            widgets::icon_button("about", icons::QUESTION, "About GodPDF", palette)
                .on_click(dispatch(crate::services::About)),
        )
        .child(widgets::icon_button("theme", glyph, tip, palette).on_click(dispatch(ToggleTheme)));

    div()
        .id("tab-row")
        .flex_none()
        .flex()
        .items_center()
        .gap(px(2.))
        .h(px(TAB_HEIGHT + 4.))
        .pt(px(4.))
        .pl(px(8. + 2.))
        .pr(px(8.))
        .bg(palette.surface)
        .text_size(px(fonts::BODY))
        .child(home)
        .children(tabs)
        .child(div().w(px(6. - 2.)))
        .child(open)
        .child(right)
        .into_any_element()
}

/// What the right-click menu of a tab needs.
#[derive(Clone)]
struct TabMenu {
    app: WeakEntity<GodPdfApp>,
    count: usize,
    reopen: bool,
}

impl TabMenu {
    fn build(
        &self,
        menu: PopupMenu,
        index: usize,
        path: Option<PathBuf>,
        cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        let palette = Palette::of(cx);
        let item = |glyph: &'static str, label: &'static str, enabled: bool| {
            let color = if enabled {
                palette.text
            } else {
                palette.disabled()
            };
            PopupMenuItem::element(move |_, _| widgets::with_icon(glyph, label, fonts::BODY, color))
                .disabled(!enabled)
        };
        let app = self.app.clone();
        let with_app =
            move |run: fn(&mut GodPdfApp, usize, &mut Window, &mut Context<GodPdfApp>)| {
                let app = app.clone();
                move |_: &gpui::ClickEvent, window: &mut Window, cx: &mut App| {
                    let _ = app.update(cx, |app, cx| run(app, index, window, cx));
                }
            };
        let menu = menu
            .min_w(px(160.))
            .item(
                item(icons::X, "Close", true).on_click(with_app(|app, index, window, cx| {
                    app.request_close(index, window, cx)
                })),
            )
            .item(
                item(icons::X_CIRCLE, "Close the others", self.count > 1)
                    .on_click(with_app(|app, index, _, cx| app.close_others(index, cx))),
            )
            .item(
                item(
                    icons::CLOCK_COUNTER_CLOCKWISE,
                    "Reopen closed tab",
                    self.reopen,
                )
                .on_click(|_, window, cx| window.dispatch_action(Box::new(ReopenTab), cx)),
            );
        let Some(path) = path else {
            return menu;
        };
        let shown = path.clone();
        menu.separator()
            .item(
                item(icons::FOLDER_OPEN, "Show in folder", true).on_click(move |_, _, cx| {
                    if let Err(err) = reveal(&shown) {
                        toasts::error(cx, err);
                    }
                }),
            )
            .item(
                item(icons::COPY, "Copy path", true).on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(path.display().to_string()));
                }),
            )
    }
}

/// Show `path` selected in the file manager.
fn reveal(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    let started = {
        use std::os::windows::process::CommandExt as _;
        std::process::Command::new("explorer")
            .raw_arg(format!("/select,\"{}\"", path.display()))
            .spawn()
    };
    #[cfg(target_os = "macos")]
    let started = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn();
    #[cfg(not(any(windows, target_os = "macos")))]
    let started = std::process::Command::new("xdg-open")
        .arg(path.parent().unwrap_or(path))
        .spawn();
    started
        .map(|_| ())
        .map_err(|err| format!("Cannot show {}: {err}", path.display()))
}

/// Tab-shaped hit area: active = `bg` with rounded top corners from 4 px
/// below the row top down to the header; hovered = `hover`, radius 8, 5 px in from top and
/// bottom. Add the width and a [`content`] child.
fn tab_shape(id: impl Into<gpui::ElementId>, active: bool, palette: &Palette) -> Stateful<Div> {
    let id = id.into();
    let group: SharedString = format!("{id:?}").into();
    let hover = palette.hover;
    let fill = div().absolute().left_0().right_0();
    let fill = if active {
        fill.top(px(4.)).bottom_0().rounded_t(px(8.)).bg(palette.bg)
    } else {
        fill.top(px(5.))
            .bottom(px(5.))
            .rounded(px(8.))
            .group_hover(group.clone(), move |style| style.bg(hover))
    };
    div()
        .id(id)
        .group(group)
        .relative()
        .flex_none()
        .h(px(TAB_HEIGHT))
        .cursor_pointer()
        .child(fill)
}

/// Content of a tab, 2 px below the middle.
fn content() -> Div {
    div().relative().h_full().pt(px(4.)).flex().items_center()
}

/// One document tab: FILE_PDF 16 centered 20 px in, name (14) at 36 px cut
/// with "…" at `text_max`, `*` (15) in 14 px when unsaved, close cross (22 / 13) 8 px from
/// the right edge.
fn doc_tab(
    index: usize,
    title: SharedString,
    dirty: bool,
    active: bool,
    text_max: f32,
    palette: &Palette,
    cx: &mut Context<GodPdfApp>,
) -> Stateful<Div> {
    let color = if active {
        palette.text
    } else {
        palette.text_dim
    };
    let close = widgets::small_icon_button(("close-tab", index), icons::X, "Close", palette)
        .on_click(cx.listener(move |app, _, window, cx| {
            cx.stop_propagation();
            app.request_close(index, window, cx);
        }));
    let name = div()
        .max_w(px(text_max))
        .overflow_hidden()
        .text_ellipsis()
        .whitespace_nowrap()
        .text_size(px(14.))
        .text_color(color)
        .child(title.clone());
    let tip: SharedString = if dirty {
        format!("{title} (unsaved changes)").into()
    } else {
        title
    };
    let tab = tab_shape(("tab", index), active, palette)
        // Widths: 12 + 16 + 8 + text + (14) + 8 + close + 8.
        .child(
            content()
                .pl(px(12.))
                .pr(px(8.))
                .child(icon(icons::FILE_PDF, 16.0, color))
                .child(div().w(px(8.)))
                .child(name)
                .when(dirty, |tab| {
                    tab.child(
                        div()
                            .w(px(14.))
                            .pl(px(4.))
                            .text_size(px(15.))
                            .text_color(color)
                            .child("*"),
                    )
                })
                .child(div().w(px(8.)))
                .child(close),
        )
        .on_click(cx.listener(move |app, _, _, cx| {
            app.active = index;
            app.shell.home = false;
            cx.notify();
        }))
        .on_mouse_down(
            MouseButton::Middle,
            cx.listener(move |app, _, window, cx| app.request_close(index, window, cx)),
        );
    widgets::tooltip(tab, tip)
}
