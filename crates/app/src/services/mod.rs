//! App services: settings kept between sessions (`settings.rs`),
//! updates from GitHub (`updates.rs`), the "Working…" spinner of background jobs
//! (`jobs.rs`), files changed by another program (`watch.rs`), the Updates popover of the
//! tab row, and the dialogs: About, Preferences and the password asked for an encrypted PDF.
//!
//! Dialogs work as in `dialogs/`: an action records the dialog as pending in the
//! [`Services`] global; [`overlay`] (it has the window) builds and focuses it.

pub mod jobs;
pub mod settings;
pub mod updates;
pub mod watch;

use std::path::PathBuf;

use gpui::AnimationExt as _;
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
    Preferences,
    Password(Locked),
}

enum Dialog {
    About(FocusHandle),
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
        set_updates_open(true, cx);
        cx.refresh_windows();
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
        Dialog::About(focus) | Dialog::Preferences(focus) => Some(focus.clone()),
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
            18.,
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
            18.,
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

/// Open or close the Updates popover; opening looks for releases unless a list is already in.
fn set_updates_open(open: bool, cx: &mut App) {
    let state = cx.default_global::<updates::Updates>();
    state.open = open;
    if open && !matches!(state.state, updates::State::Ready(_)) {
        updates::check(cx);
    }
}

/// The Updates button of the tab row (blue dot and "GodPDF X.Y.Z is available" when a new
/// version is out) with its popover, right-aligned under it. Esc or a click outside closes it.
pub fn updates_button(palette: &Palette, cx: &App) -> AnyElement {
    let newest = updates::newest(cx);
    let open = updates::get(cx).is_some_and(|updates| updates.open);
    let tip = newest.as_ref().map_or_else(
        || "Updates".to_owned(),
        |release| format!("{} {} is available", crate::APP_NAME, release.version()),
    );
    let palette = *palette;
    gpui::base::Popover::new("updates-popover")
        .anchor(gpui::Anchor::TopRight)
        .offset(px(6.))
        .open(open)
        .on_open_change(|open, _, cx| set_updates_open(*open, cx))
        .trigger_with(move |_, _, _| {
            let dot = newest.map(|_| {
                // Radius 4 at the top-right corner, 6 in.
                div()
                    .absolute()
                    .top(px(6. - 4.))
                    .right(px(6. - 4.))
                    .size(px(8.))
                    .rounded(px(3.5))
                    .bg(theme::hex(theme::DOWNLOAD_BLUE))
            });
            widgets::icon_button("updates", icons::ARROW_CIRCLE_UP, tip, &palette)
                .relative()
                .children(dot)
                .into_any_element()
        })
        .content(move |_, _, cx| updates_card(&palette, cx))
        .into_any_element()
}

/// The popover card: the new version with its notes and "Update and restart", or the
/// status (up to date / checking / failed) with a button to check again.
fn updates_card(palette: &Palette, cx: &App) -> AnyElement {
    use updates::{Install, State};
    let current = env!("CARGO_PKG_VERSION");
    let (state, downloading) = updates::get(cx)
        .map(|updates| {
            let downloading = matches!(updates.install, Install::Downloading);
            (updates.state.clone(), downloading)
        })
        .unwrap_or_default();
    let card = div()
        .w(px(256.))
        .p(px(12.))
        .flex()
        .flex_col()
        .gap(px(4.))
        .bg(palette.panel)
        .border_1()
        .border_color(palette.border)
        .rounded(px(10.))
        .shadow(widgets::popup_shadow(palette))
        .text_size(px(12.))
        .text_color(palette.text);
    let title = |text: String| {
        div()
            .text_size(px(14.))
            .text_color(palette.text)
            .child(text)
    };
    let muted = |text: String| div().text_color(palette.text_dim).child(text);

    let newest = match &state {
        State::Ready(releases) => releases.iter().find(|release| release.is_newer()).cloned(),
        _ => None,
    };
    if let Some(release) = newest {
        let notes = updates::bullets(&release.notes);
        let install = release.asset.clone().filter(|_| updates::CAN_INSTALL);
        return card
            .child(title(format!(
                "{} {} is available",
                crate::APP_NAME,
                release.version()
            )))
            .child(muted(format!("You have {current}.")))
            .when(!notes.is_empty(), |card| {
                card.child(
                    div()
                        .id("updates-notes")
                        .mt(px(4.))
                        .max_h(px(200.))
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .children(notes.into_iter().map(|item| {
                            div()
                                .flex()
                                .items_start()
                                .gap(px(6.))
                                .child("•")
                                .child(div().flex_1().min_w_0().child(item))
                        })),
                )
            })
            .child(div().h(px(4.)))
            .child(update_button(install.clone(), release.url, downloading))
            .when(install.is_some(), |card| {
                card.child(
                    div()
                        .pt(px(2.))
                        .text_size(px(11.))
                        .text_color(palette.text_dim)
                        .child("Unsaved changes are asked about before the app restarts."),
                )
            })
            .into_any_element();
    }

    let (heading, line, checking) = match state {
        State::Idle | State::Loading => (
            "Checking…".to_owned(),
            format!("{} {current}", crate::APP_NAME),
            true,
        ),
        State::Ready(_) => (
            "You're up to date".to_owned(),
            format!("{} {current}", crate::APP_NAME),
            false,
        ),
        State::Failed(err) => ("Could not check for updates".to_owned(), err, false),
    };
    card.child(
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(title(heading))
                    .child(muted(line)),
            )
            .child(refresh_button(checking, palette)),
    )
    .into_any_element()
}

/// The full-width blue button: "Update and restart" (a spinner while downloading), or "View
/// on GitHub" where the app cannot replace itself.
fn update_button(install: Option<String>, page: String, downloading: bool) -> AnyElement {
    let white = gpui::white();
    let button = div()
        .id("install-update")
        .w_full()
        .h(px(32.))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .rounded(px(theme::RADIUS))
        .bg(theme::hex(theme::DOWNLOAD_BLUE))
        .text_color(white);
    let button = match install {
        Some(_) if downloading => button
            .opacity(0.6)
            .child(Spinner::new().with_size(Size::Small).color(white))
            .child("Downloading…"),
        Some(asset) => button
            .cursor_pointer()
            .hover(|style| style.opacity(0.9))
            .child("Update and restart")
            .on_click(move |_, window, cx| {
                updates::install(asset.clone(), cx);
                window.refresh();
            }),
        None => {
            let page = if page.is_empty() {
                updates::RELEASES_PAGE.to_owned()
            } else {
                page
            };
            button
                .cursor_pointer()
                .hover(|style| style.opacity(0.9))
                .child("View on GitHub")
                .on_click(move |_, _, cx| cx.open_url(&page))
        }
    };
    button.into_any_element()
}

/// Check again; the arrow spins while checking.
fn refresh_button(checking: bool, palette: &Palette) -> AnyElement {
    // A Lucide SVG, not a Phosphor glyph: GPUI rotates SVGs and images only, not text.
    let arrow = gpui::svg()
        .path("icons/refresh-cw.svg")
        .size(px(16.))
        .text_color(if checking {
            palette.text_dim
        } else {
            palette.text
        });
    let arrow = if checking {
        arrow
            .with_animation(
                "updates-checking",
                gpui::Animation::new(std::time::Duration::from_secs(1)).repeat(),
                |arrow, delta| {
                    arrow.with_transformation(gpui::Transformation::rotate(gpui::percentage(delta)))
                },
            )
            .into_any_element()
    } else {
        arrow.into_any_element()
    };
    let hover = palette.hover;
    let button = div()
        .id("updates-refresh")
        .flex_none()
        .size(px(widgets::ICON_BUTTON))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme::RADIUS))
        .child(arrow)
        .when(!checking, |button| {
            button
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .on_click(|_, window, cx| {
                    updates::check(cx);
                    window.refresh();
                })
        });
    widgets::tooltip(button, "Check for updates").into_any_element()
}
