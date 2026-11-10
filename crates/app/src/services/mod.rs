//! App services: settings kept between sessions (`settings.rs`),
//! updates from GitHub (`updates.rs`), the "Working…" spinner of background jobs
//! (`jobs.rs`), files changed by another program (`watch.rs`), and their dialogs: About,
//! Updates, Preferences and the password asked for an encrypted PDF.
//!
//! Dialogs work as in `dialogs/`: an action records the dialog as pending in the
//! [`Services`] global; [`overlay`] (it has the window) builds and focuses it.

pub mod jobs;
pub mod settings;
pub mod updates;
pub mod watch;

use std::path::PathBuf;

use gpui::component::input::{Input, InputState};
use gpui::component::spinner::Spinner;
use gpui::component::{Sizable, Size};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Div, Entity, FocusHandle, Global, KeyBinding, MouseButton,
    SharedString, WeakEntity, Window, actions, div, hsla, img, px,
};

use crate::app::GodPdfApp;
use crate::dialogs::{self, DialogCancel, DialogConfirm};
use crate::theme::{self, Palette};
use crate::widgets::{self, accent_button, framed_button};
use crate::{fonts, icons, toasts};

actions!(godpdf, [About, Updates, Preferences]);

/// Key context of the dialogs without a text field: Enter and Esc close them.
const CONTEXT: &str = "GodPdfInfo";

/// An encrypted file waiting for its password.
struct Locked {
    app: WeakEntity<GodPdfApp>,
    name: String,
    path: Option<PathBuf>,
    bytes: Vec<u8>,
}

enum Pending {
    About,
    Updates,
    Preferences,
    Password(Locked),
}

enum Dialog {
    About(FocusHandle),
    Updates(FocusHandle),
    Preferences(FocusHandle),
    Password {
        locked: Locked,
        input: Entity<InputState>,
        error: Option<String>,
    },
}

#[derive(Default)]
struct Services {
    pending: Option<Pending>,
    open: Option<Dialog>,
    return_focus: Option<FocusHandle>,
}

impl Global for Services {}

fn state(cx: &mut App) -> &mut Services {
    cx.default_global::<Services>()
}

fn open_later(pending: Pending, cx: &mut App) {
    state(cx).pending = Some(pending);
    cx.refresh_windows();
}

pub fn register(cx: &mut App) {
    cx.set_global(Services::default());
    cx.bind_keys([
        KeyBinding::new("enter", DialogConfirm, Some(CONTEXT)),
        KeyBinding::new("escape", DialogCancel, Some(CONTEXT)),
    ]);
    cx.on_action(|_: &About, cx| open_later(Pending::About, cx));
    cx.on_action(|_: &Preferences, cx| open_later(Pending::Preferences, cx));
    cx.on_action(|_: &Updates, cx| {
        // Look again unless a list is already in.
        if !matches!(
            updates::get(cx).map(|updates| &updates.state),
            Some(updates::State::Ready(_))
        ) {
            updates::check(cx);
        }
        open_later(Pending::Updates, cx);
    });
}

/// Startup work of the root view: settings applied, old program removed, updates looked
/// for, files watched; settings written (and the new version started) on quit.
pub fn start(app: &mut GodPdfApp, cx: &mut Context<GodPdfApp>) {
    let remembered = settings::load();
    app.shell.show_tools = remembered.tools_panel;
    app.shell.recent = remembered.recent;
    if !remembered.sidebar {
        crate::panels::close(cx);
    }
    updates::remove_old_program();
    updates::check(cx);
    watch::start(cx);
    cx.on_app_quit(|_, cx| {
        settings::save();
        updates::restart_if_installed(cx);
        async {}
    })
    .detach();
}

/// Note what is shown, to be written on quit. Called by the root view every frame.
pub fn track(app: &GodPdfApp, window: &Window, cx: &App) {
    let (bounds, maximized) = match window.window_bounds() {
        gpui::WindowBounds::Windowed(bounds) => (bounds, false),
        gpui::WindowBounds::Maximized(bounds) => (bounds, true),
        gpui::WindowBounds::Fullscreen(bounds) => (bounds, false),
    };
    settings::track(settings::Settings {
        dark: Palette::is_dark(cx),
        sidebar: crate::panels::open_panel(cx).is_some(),
        tools_panel: app.shell.show_tools,
        recent: app.shell.recent.clone(),
        size: Some((bounds.size.width.into(), bounds.size.height.into())),
        maximized,
    });
}

/// An encrypted file needs a password to open: ask for it.
pub fn ask_password(
    name: String,
    path: Option<PathBuf>,
    bytes: Vec<u8>,
    cx: &mut Context<GodPdfApp>,
) {
    let locked = Locked {
        app: cx.weak_entity(),
        name,
        path,
        bytes,
    };
    open_later(Pending::Password(locked), cx);
}

/// Close the open dialog without a window at hand (focus goes back on the next frame).
pub fn close_dialog(cx: &mut App) {
    state(cx).open = None;
    cx.refresh_windows();
}

fn close(window: &mut Window, cx: &mut App) {
    let services = state(cx);
    services.open = None;
    if let Some(focus) = services.return_focus.take() {
        window.focus(&focus, cx);
    }
    window.refresh();
}

/// Password dialog "Unlock": open the decrypted file, keeping the password for saving.
fn unlock(window: &mut Window, cx: &mut App) {
    let Some(Dialog::Password { locked, input, .. }) = state(cx).open.take() else {
        return close(window, cx);
    };
    let password = input.read(cx).value().to_string();
    match crate::doc::guarded(|| godpdf_core::decrypt(&locked.bytes, &password)) {
        Ok(bytes) => {
            close(window, cx);
            let Locked {
                app, name, path, ..
            } = locked;
            let _ = app.update(cx, |app, cx| {
                app.open_bytes(name, path, bytes, cx);
                if let Some(tab) = app.tabs.last() {
                    dialogs::keep_password(tab.entity_id(), password, cx);
                }
            });
            toasts::info(
                cx,
                "Unlocked. Saving keeps the password. Use “Remove password” to drop it.",
            );
        }
        Err(godpdf_core::Error::BadPassword) => {
            // The field is emptied and the dialog says why.
            input.update(cx, |input, cx| input.set_value("", window, cx));
            state(cx).open = Some(Dialog::Password {
                locked,
                input,
                error: Some("Wrong password".to_owned()),
            });
            window.refresh();
        }
        Err(err) => {
            close(window, cx);
            toasts::error(cx, format!("Cannot decrypt “{}”: {err}", locked.name));
        }
    }
}

fn build(pending: Pending, window: &mut Window, cx: &mut App) {
    let (open, input) = match pending {
        Pending::About => (Dialog::About(cx.focus_handle()), None),
        Pending::Updates => (Dialog::Updates(cx.focus_handle()), None),
        Pending::Preferences => (Dialog::Preferences(cx.focus_handle()), None),
        Pending::Password(locked) => {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Password")
                    .masked(true)
            });
            let dialog = Dialog::Password {
                locked,
                input: input.clone(),
                error: None,
            };
            (dialog, Some(input))
        }
    };
    let focus = match &open {
        Dialog::About(focus) | Dialog::Updates(focus) | Dialog::Preferences(focus) => {
            Some(focus.clone())
        }
        Dialog::Password { .. } => None,
    };
    let focused = window.focused(cx);
    let services = state(cx);
    if services.open.is_none() {
        services.return_focus = focused;
    }
    services.open = Some(open);
    match (input, focus) {
        (Some(input), _) => input.update(cx, |input, cx| input.focus(window, cx)),
        (None, Some(focus)) => window.focus(&focus, cx),
        (None, None) => {}
    }
}

/// Drawn over the window while one of these dialogs is open.
pub fn overlay(window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    if let Some(pending) = state(cx).pending.take() {
        build(pending, window, cx);
    }
    let palette = Palette::of(cx);
    let card = match cx.try_global::<Services>()?.open.as_ref()? {
        Dialog::About(focus) => info_card(focus, &palette).children(about(&palette)),
        Dialog::Updates(focus) => info_card(focus, &palette).children(updates_body(&palette, cx)),
        Dialog::Preferences(focus) => {
            info_card(focus, &palette).children(preferences(&palette, cx))
        }
        Dialog::Password {
            locked,
            input,
            error,
        } => password(locked, input, error.as_deref(), &palette),
    };
    Some(
        div()
            .id("service-backdrop")
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .justify_center()
            .items_center()
            // Backdrop: black at alpha 100.
            .bg(hsla(0., 0., 0., 100. / 255.))
            .on_mouse_down(MouseButton::Left, |_, window, cx| close(window, cx))
            .child(card)
            .into_any_element(),
    )
}

/// Card of a dialog with no field: focused, Enter / Esc close it. Its body, then "Close" /
/// "Done" are added by the caller.
fn info_card(focus: &FocusHandle, palette: &Palette) -> Div {
    dialogs::card(palette, 420.)
        .track_focus(focus)
        .key_context(CONTEXT)
        .on_action(|_: &DialogConfirm, window, cx| close(window, cx))
        .on_action(|_: &DialogCancel, window, cx| close(window, cx))
}

/// One filled button that closes the dialog.
fn close_row(label: &'static str, palette: &Palette) -> AnyElement {
    dialogs::button_row()
        .child(
            accent_button("service-close", label, true, palette)
                .on_click(|_, window, cx| close(window, cx)),
        )
        .into_any_element()
}

fn about(palette: &Palette) -> Vec<AnyElement> {
    let dim = palette.text_dim;
    let line = |text: SharedString, size: f32, color| {
        div()
            .text_size(px(size))
            .text_color(color)
            .child(text)
            .into_any_element()
    };
    let column = div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(theme::ITEM_SPACING.1))
        .child(
            img(crate::shell::welcome::LOGO.clone())
                .size(px(120.))
                .rounded(px(26.)),
        )
        .child(line(crate::APP_NAME.into(), 26., palette.text))
        .child(line(
            format!("Version {}", env!("CARGO_PKG_VERSION")).into(),
            fonts::BODY,
            dim,
        ))
        .child(div().h(px(6.)))
        .child(line(
            "Every PDF tool. Local. Pure Rust.".into(),
            fonts::BODY,
            palette.text,
        ))
        .child(line(
            "No account, no server, no telemetry.".into(),
            fonts::BODY,
            dim,
        ))
        // Small text.
        .child(line("Built with GPUI, lopdf and hayro".into(), 9., dim));
    vec![column.into_any_element(), close_row("Close", palette)]
}

fn preferences(palette: &Palette, cx: &App) -> Vec<AnyElement> {
    let dark = Palette::is_dark(cx);
    let choice = |id: &'static str, glyph, label: &'static str, on: bool| {
        let selected = palette.selected;
        widgets::button(id, "", palette)
            .child(widgets::with_icon(glyph, label, fonts::BODY, palette.text))
            .when(on, |chip| chip.bg(selected))
            .on_click(move |_, window, cx| {
                if Palette::is_dark(cx) != (id == "pref-dark") {
                    theme::toggle(cx);
                }
                window.refresh();
            })
    };
    vec![
        dialogs::title(widgets::with_icon(
            icons::GEAR,
            "Preferences",
            19.,
            palette.text,
        ))
        .into_any_element(),
        div()
            .flex()
            .items_center()
            .gap(px(theme::ITEM_SPACING.0))
            .child("Theme")
            .child(choice("pref-dark", icons::MOON, "Dark", dark))
            .child(choice("pref-light", icons::SUN, "Light", !dark))
            .into_any_element(),
        close_row("Done", palette),
    ]
}

fn password(
    locked: &Locked,
    input: &Entity<InputState>,
    error: Option<&str>,
    palette: &Palette,
) -> Div {
    dialogs::card(palette, 420.)
        .key_context("GodPdfDialog")
        .on_action(|_: &DialogConfirm, window, cx| unlock(window, cx))
        .on_action(|_: &DialogCancel, window, cx| close(window, cx))
        .child(dialogs::title(widgets::with_icon(
            icons::LOCK,
            "Password required",
            19.,
            palette.text,
        )))
        .child(format!("“{}” is encrypted.", locked.name))
        .child(Input::new(input))
        .children(error.map(|error| dialogs::error_line(error.to_owned(), palette)))
        .child(
            dialogs::button_row()
                .child(
                    accent_button("unlock", "Unlock", true, palette)
                        .on_click(|_, window, cx| unlock(window, cx)),
                )
                .child(
                    framed_button("unlock-cancel", "Cancel", true, palette)
                        .on_click(|_, window, cx| close(window, cx)),
                ),
        )
}

/// The Updates dialog: status, the releases with their notes, install / links.
fn updates_body(palette: &Palette, cx: &App) -> Vec<AnyElement> {
    use updates::{CAN_INSTALL, RELEASES_PAGE, State};
    let current = env!("CARGO_PKG_VERSION");
    let mut body = vec![
        dialogs::title(widgets::with_icon(
            icons::ARROW_CIRCLE_UP,
            "Updates",
            19.,
            palette.text,
        ))
        .into_any_element(),
    ];
    let (state, install) = updates::get(cx)
        .map(|updates| (updates.state.clone(), updates.install.clone()))
        .unwrap_or_default();
    match state {
        State::Idle | State::Loading => {
            body.push(spinner_line("Looking for new versions…", palette));
        }
        State::Failed(err) => {
            body.push(
                div()
                    .text_color(palette.error)
                    .child(err)
                    .into_any_element(),
            );
            body.push(
                div()
                    .flex()
                    .child(
                        framed_button(
                            "updates-retry",
                            widgets::with_icon(
                                icons::ARROW_CLOCKWISE,
                                "Try again",
                                fonts::BODY,
                                palette.text,
                            ),
                            true,
                            palette,
                        )
                        .on_click(|_, window, cx| {
                            updates::check(cx);
                            window.refresh();
                        }),
                    )
                    .into_any_element(),
            );
        }
        State::Ready(releases) => {
            let newest = releases.iter().find(|release| release.is_newer());
            let status = match newest {
                Some(release) => format!("{} is out. You have version {current}.", release.tag),
                None => format!("You have the latest version ({current})."),
            };
            body.push(div().child(status).into_any_element());
            if releases.is_empty() {
                body.push(
                    div()
                        .text_color(palette.text_dim)
                        .child("No release published yet.")
                        .into_any_element(),
                );
            }
            let newest_tag = newest.map(|release| release.tag.clone());
            let rows = releases.into_iter().enumerate().map(|(index, release)| {
                let latest = newest_tag.as_deref() == Some(release.tag.as_str());
                let action = match (&release.asset, latest && CAN_INSTALL) {
                    (Some(asset), true) => Some(install_row(asset.clone(), &install, palette)),
                    _ if !release.url.is_empty() => Some(link(
                        ("release-link", index),
                        Some(icons::ARROW_SQUARE_OUT),
                        "View on GitHub",
                        release.url.clone(),
                        palette,
                    )),
                    _ => None,
                };
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme::ITEM_SPACING.1))
                    .pt(px(6.))
                    .border_t_1()
                    .border_color(palette.border)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(theme::ITEM_SPACING.0))
                            .child(div().text_size(px(16.)).child(release.name.clone()))
                            .when(release.is_newer(), |row| {
                                row.child(div().text_color(palette.accent_text).child("New"))
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(9.))
                            .text_color(palette.text_dim)
                            .child(format!("{} · {}", release.tag, release.date)),
                    )
                    .children(markdown(&release.notes, palette))
                    .children(action)
            });
            body.push(
                div()
                    .id("releases")
                    .max_h(px(360.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(theme::ITEM_SPACING.1))
                    .children(rows)
                    .into_any_element(),
            );
        }
    }
    body.push(div().h(px(0.)).into_any_element());
    body.push(link(
        "all-releases",
        None,
        "All releases on GitHub",
        RELEASES_PAGE.to_owned(),
        palette,
    ));
    body.push(close_row("Close", palette));
    body
}

/// The blue "Download and install" button, or the download under way.
fn install_row(asset: String, install: &updates::Install, palette: &Palette) -> AnyElement {
    use updates::Install;
    if matches!(install, Install::Downloading) {
        return spinner_line(
            "Downloading the update… GodPDF restarts when it is done.",
            palette,
        );
    }
    let white = gpui::white();
    let button = div()
        .id("install-update")
        .flex()
        .items_center()
        .h(px(30.))
        .px(px(theme::BUTTON_PADDING.0))
        .rounded(px(theme::RADIUS))
        .bg(theme::hex(theme::DOWNLOAD_BLUE))
        .cursor_pointer()
        .hover(|style| style.opacity(0.9))
        .child(widgets::with_icon(
            icons::DOWNLOAD_SIMPLE,
            "Download and install",
            fonts::BODY,
            white,
        ))
        .on_click(move |_, window, cx| {
            updates::install(asset.clone(), cx);
            window.refresh();
        });
    div()
        .flex()
        .flex_col()
        .items_start()
        .gap(px(theme::ITEM_SPACING.1))
        .pt(px(4.))
        .child(widgets::tooltip(
            button,
            "Replace GodPDF with this version and restart",
        ))
        .children(match install {
            Install::Failed(err) => Some(div().text_color(palette.error).child(err.clone())),
            _ => None,
        })
        .into_any_element()
}

fn spinner_line(text: &'static str, palette: &Palette) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(theme::ITEM_SPACING.0))
        .child(Spinner::new().with_size(Size::Small).color(palette.text))
        .child(text)
        .into_any_element()
}

/// A link: accent text, underlined on hover, opens `url` in the browser.
fn link(
    id: impl Into<gpui::ElementId>,
    glyph: Option<&'static str>,
    label: &'static str,
    url: String,
    palette: &Palette,
) -> AnyElement {
    let color = palette.accent_text;
    let content = match glyph {
        Some(glyph) => widgets::with_icon(glyph, label, fonts::BODY, color),
        None => div().text_color(color).child(label),
    };
    div()
        .flex()
        .child(
            div()
                .id(id)
                .cursor_pointer()
                .hover(|style| style.underline())
                .child(content)
                .on_click(move |_, _, cx| cx.open_url(&url)),
        )
        .into_any_element()
}

/// The release notes: headings, bullets, `**bold**` and `code` marks
/// dropped (Ubuntu Light has no bold).
fn markdown(text: &str, palette: &Palette) -> Vec<AnyElement> {
    let clean = |line: &str| line.replace("**", "").replace('`', "");
    text.lines()
        .map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return div().h(px(4.)).into_any_element();
            }
            if let Some(heading) = trimmed.strip_prefix('#') {
                let heading = clean(heading.trim_start_matches('#').trim());
                return div().text_size(px(15.)).child(heading).into_any_element();
            }
            let bullet = ["- ", "* ", "+ "]
                .iter()
                .find_map(|marker| trimmed.strip_prefix(marker));
            match bullet {
                Some(item) => div()
                    .flex()
                    .items_start()
                    .gap(px(theme::ITEM_SPACING.0))
                    .text_size(px(14.))
                    .child("•")
                    .child(div().flex_1().min_w_0().child(clean(item)))
                    .into_any_element(),
                None => div()
                    .text_size(px(14.))
                    .text_color(palette.text)
                    .child(clean(trimmed))
                    .into_any_element(),
            }
        })
        .collect()
}

/// The "Working…" spinner of the header, while a background job runs.
pub fn working(palette: &Palette, cx: &App) -> Option<AnyElement> {
    if !jobs::busy(cx) {
        return None;
    }
    let spinner = div().id("working").flex().items_center().child(
        Spinner::new()
            .with_size(Size::Small)
            .color(palette.text_dim),
    );
    Some(widgets::tooltip(spinner, "Working…").into_any_element())
}

/// The Updates button of the tab row: blue dot and "Update available: <tag>" when one is out.
pub fn updates_button(palette: &Palette, cx: &App) -> AnyElement {
    let newest = updates::newest(cx);
    let tip = newest.as_ref().map_or_else(
        || "Updates".to_owned(),
        |release| format!("Update available: {}", release.tag),
    );
    let button = widgets::icon_button("updates", icons::ARROW_CIRCLE_UP, tip, palette)
        .relative()
        .on_click(widgets::dispatch(Updates));
    let dot = newest.map(|_| {
        // Radius 4 at the top-right corner, 6 in.
        div()
            .absolute()
            .top(px(6. - 4.))
            .right(px(6. - 4.))
            .size(px(8.))
            .rounded_full()
            .bg(theme::hex(theme::DOWNLOAD_BLUE))
    });
    button.children(dot).into_any_element()
}
