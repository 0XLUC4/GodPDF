//! Opt-in sharing of one PDF with one other GodPDF instance.

mod transport;

use std::sync::Arc;

use gpui::component::button::{Button, ButtonVariants};
use gpui::component::input::{Input, InputState};
use gpui::prelude::*;
use gpui::{
    AnyElement, ClipboardItem, Context, Entity, MouseButton, WeakEntity, Window, div, hsla, px,
};
use tokio::sync::watch;

use crate::app::GodPdfApp;
use crate::dialogs::{self, DialogCancel, DialogConfirm};
use crate::doc::{DocTab, guarded};
use crate::theme::Palette;
use crate::widgets::{accent_button, framed_button};
use crate::{fonts, text, toasts, tools};
pub(crate) use transport::Cursor;
use transport::{Event, INITIAL_TURN, MAX_PDF, Revision, Start, Turn};

pub(crate) const SHARED_PDF_LIMIT: usize = MAX_PDF;

/// An editing turn belongs to one peer. A handoff carries the complete latest PDF.
pub(crate) struct SharedDocument {
    pub(crate) editing: bool,
    turn: Turn,
    sender: watch::Sender<Option<Revision>>,
    pub(crate) cursor_sender: watch::Sender<Option<Cursor>>,
    pub(crate) peer_cursor: Option<Cursor>,
    pub(crate) peer_name: Option<String>,
    pub(crate) hosted: bool,
}

impl SharedDocument {
    pub(crate) fn publish(&self, bytes: Arc<Vec<u8>>) {
        if self.editing {
            self.sender.send_replace(Some(Revision {
                bytes,
                turn: self.turn,
                grant: None,
            }));
        }
    }

    pub(crate) fn pass(&mut self, bytes: Arc<Vec<u8>>) -> Result<(), String> {
        if !self.editing || self.sender.is_closed() {
            return Err("The editing turn is unavailable".into());
        }
        if bytes.len() > MAX_PDF {
            return Err(format!(
                "The shared PDF exceeds {} MiB",
                MAX_PDF / (1024 * 1024)
            ));
        }
        // A frame queued before the handoff cannot carry this new capability.
        let turn = self.turn;
        let mut next = INITIAL_TURN;
        getrandom::fill(&mut next).map_err(|_| "Could not create the next editing turn")?;
        self.editing = false;
        self.turn = next;
        self.sender.send_replace(Some(Revision {
            bytes,
            turn,
            grant: Some(next),
        }));
        Ok(())
    }

    fn accepts(&self, revision: &Revision) -> bool {
        !self.editing && revision.turn == self.turn && revision.grant != Some(self.turn)
    }
}

#[derive(Default)]
pub(crate) struct Collaboration {
    open: bool,
    code: Option<Entity<InputState>>,
    sequence: u64,
    session: Option<Session>,
}

struct Session {
    tab: Option<WeakEntity<DocTab>>,
    sender: watch::Sender<Option<Revision>>,
    cursor_sender: watch::Sender<Option<Cursor>>,
    invitation: Option<String>,
    peer: Option<String>,
}

/// Label of the header button: "Share" in a PDF, "Join" elsewhere, "Shared" during a session.
pub(crate) fn button_label(app: &GodPdfApp) -> &'static str {
    if app.collaboration.session.is_some() {
        "Shared"
    } else if app.doc_view() {
        "Share"
    } else {
        "Join"
    }
}

/// Header button: shares the open PDF at once (the popup shows its code), opens the join
/// popup on the home screen, or shows the running session.
pub(crate) fn toggle(app: &mut GodPdfApp, window: &mut Window, cx: &mut Context<GodPdfApp>) {
    if let Some(session) = &app.collaboration.session {
        if let Some(index) = app
            .tabs
            .iter()
            .position(|tab| session.tab.as_ref() == Some(&tab.downgrade()))
        {
            app.active = index;
            app.shell.home = false;
        }
        app.collaboration.open = !app.collaboration.open;
    } else if app.doc_view() {
        host(app, cx);
        app.collaboration.open = app.collaboration.session.is_some();
    } else {
        app.collaboration.open = !app.collaboration.open;
        if app.collaboration.open {
            let input = app
                .collaboration
                .code
                .get_or_insert_with(|| {
                    cx.new(|cx| InputState::new(window, cx).placeholder("123 456"))
                })
                .clone();
            input.update(cx, |input, cx| {
                input.set_value("", window, cx);
                input.focus(window, cx);
            });
        }
    }
    cx.notify();
}

fn close_popup(app: &mut GodPdfApp, cx: &mut Context<GodPdfApp>) {
    app.collaboration.open = false;
    cx.notify();
}

fn join(app: &mut GodPdfApp, cx: &mut Context<GodPdfApp>) {
    let Some(input) = app.collaboration.code.clone() else {
        return;
    };
    let code = input.read(cx).value().to_string();
    if transport::normalize_code(&code).is_none() {
        toasts::note(cx, "Enter the 6-digit code");
        return;
    }
    start(app, Start::Join { code }, None, cx);
}

pub(crate) fn tab_closing(app: &mut GodPdfApp, tab: &Entity<DocTab>, cx: &mut Context<GodPdfApp>) {
    if app
        .collaboration
        .session
        .as_ref()
        .and_then(|s| s.tab.as_ref())
        == Some(&tab.downgrade())
    {
        leave(app, cx);
    }
}

fn leave(app: &mut GodPdfApp, cx: &mut Context<GodPdfApp>) {
    app.collaboration.open = false;
    if let Some(session) = app.collaboration.session.take()
        && let Some(tab) = session.tab.and_then(|tab| tab.upgrade())
    {
        tab.update(cx, |tab, cx| {
            tab.shared = None;
            cx.notify();
        });
    }
    cx.notify();
}

fn computer_name() -> String {
    ["COMPUTERNAME", "HOSTNAME"]
        .iter()
        .find_map(|name| {
            std::env::var(name)
                .ok()
                .filter(|name| !name.trim().is_empty())
        })
        .or_else(|| std::fs::read_to_string("/proc/sys/kernel/hostname").ok())
        .unwrap_or_else(|| "GodPDF".into())
        .trim()
        .chars()
        .filter(|ch| !ch.is_control())
        .take(48)
        .collect()
}

fn start(
    app: &mut GodPdfApp,
    mode: Start,
    tab: Option<Entity<DocTab>>,
    cx: &mut Context<GodPdfApp>,
) {
    if app.collaboration.session.is_some() {
        return;
    }
    let (sender, mut events, cursor_sender) = match transport::start(mode, computer_name()) {
        Ok(transport) => transport,
        Err(error) => {
            toasts::error(cx, error);
            return;
        }
    };
    if let Some(tab) = &tab {
        tab.update(cx, |tab, cx| {
            tab.shared = Some(SharedDocument {
                editing: true,
                turn: INITIAL_TURN,
                sender: sender.clone(),
                cursor_sender: cursor_sender.clone(),
                peer_cursor: None,
                peer_name: None,
                hosted: true,
            });
            cx.notify();
        });
    }
    app.collaboration.sequence += 1;
    let sequence = app.collaboration.sequence;
    app.collaboration.session = Some(Session {
        tab: tab.map(|tab| tab.downgrade()),
        sender,
        cursor_sender,
        invitation: None,
        peer: None,
    });
    cx.notify();

    cx.spawn(async move |this, cx| {
        while let Some(event) = events.recv().await {
            let keep_running = this.update(cx, |app, cx| {
                if app.collaboration.sequence != sequence || app.collaboration.session.is_none() {
                    return false;
                }
                if let Err(error) = receive(app, event, cx) {
                    leave(app, cx);
                    toasts::error(cx, error);
                    return false;
                }
                cx.notify();
                true
            });
            if !matches!(keep_running, Ok(true)) {
                return;
            }
        }
        let _ = this.update(cx, |app, cx| {
            if app.collaboration.sequence == sequence && app.collaboration.session.is_some() {
                leave(app, cx);
                toasts::note(
                    cx,
                    "Shared session ended. The last received version stays open.",
                );
            }
        });
    })
    .detach();
}

fn receive(app: &mut GodPdfApp, event: Event, cx: &mut Context<GodPdfApp>) -> Result<(), String> {
    let session = app
        .collaboration
        .session
        .as_mut()
        .ok_or("The session has ended")?;
    match event {
        Event::Invitation(code) => session.invitation = Some(code),
        Event::Connected(peer) => {
            let peer: String = peer
                .chars()
                .filter(|ch| !ch.is_control())
                .take(48)
                .collect();
            toasts::note(cx, format!("Connected with {peer}"));
            if let Some(tab) = session.tab.as_ref().and_then(WeakEntity::upgrade) {
                tab.update(cx, |tab, cx| {
                    if let Some(shared) = &mut tab.shared {
                        shared.peer_name = Some(peer.clone());
                    }
                    cx.notify();
                });
            }
            session.peer = Some(peer);
            session.invitation = None;
            app.collaboration.open = false;
        }
        Event::Document(bytes) => {
            if session.tab.is_some() {
                return Err("The peer tried to open a second document".into());
            }
            validate_pdf(&bytes)?;
            let sender = session.sender.clone();
            let cursor_sender = session.cursor_sender.clone();
            app.open_bytes(
                "Shared document.pdf".into(),
                None,
                bytes.as_ref().clone(),
                cx,
            );
            let tab = app.active_tab().cloned().ok_or("Cannot open shared PDF")?;
            tab.update(cx, |tab, cx| {
                tab.shared = Some(SharedDocument {
                    editing: false,
                    turn: INITIAL_TURN,
                    sender,
                    cursor_sender,
                    peer_cursor: None,
                    peer_name: None,
                    hosted: false,
                });
                cx.notify();
            });
            if let Some(session) = &mut app.collaboration.session {
                session.tab = Some(tab.downgrade());
            }
        }
        Event::Cursor(position) => {
            if let Some(tab) = session.tab.as_ref().and_then(WeakEntity::upgrade) {
                tab.update(cx, |tab, cx| {
                    if let Some(shared) = &mut tab.shared {
                        shared.peer_cursor = position;
                        cx.notify();
                    }
                });
            }
        }
        Event::Update(revision) => {
            let tab = session
                .tab
                .as_ref()
                .and_then(WeakEntity::upgrade)
                .ok_or("The shared document is closed")?;
            tab.update(cx, |tab, cx| -> Result<(), String> {
                if !tab
                    .shared
                    .as_ref()
                    .is_some_and(|shared| shared.accepts(&revision))
                {
                    return Err("The peer sent an update outside its editing turn".into());
                }
                if tab.session.bytes() != revision.bytes.as_slice() {
                    validate_pdf(&revision.bytes)?;
                    tab.receive_shared(revision.bytes.as_ref().clone())?;
                }
                if let Some(turn) = revision.grant {
                    tab.canvas.reset_shared_revision();
                    if let Some(shared) = &mut tab.shared {
                        shared.editing = true;
                        shared.turn = turn;
                    }
                }
                cx.notify();
                Ok(())
            })?;
        }
        Event::Failed(error) if error == "Collaboration stream closed" => {
            leave(app, cx);
            toasts::note(cx, "Peer disconnected. The current PDF stays open.");
        }
        Event::Failed(error) => return Err(error),
    }
    Ok(())
}

fn validate_pdf(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() <= MAX_PDF
        && guarded(|| godpdf_core::page_count(bytes)).is_ok_and(|pages| pages > 0)
        && !godpdf_core::is_encrypted(bytes)
    {
        Ok(())
    } else {
        Err("The shared PDF is invalid, encrypted, or too large".into())
    }
}

fn host(app: &mut GodPdfApp, cx: &mut Context<GodPdfApp>) {
    let Some(tab) = app.active_tab().cloned().filter(|_| app.doc_view()) else {
        return;
    };
    let bytes = tab.update(cx, |tab, cx| {
        text::finish_editing(tab, cx);
        tools::finish_shared_edit(tab, cx);
        tab.shared_bytes()
    });
    if let Err(error) = validate_pdf(&bytes) {
        toasts::error(cx, error);
        return;
    }
    start(app, Start::Host { document: bytes }, Some(tab), cx);
}

fn pass(app: &mut GodPdfApp, cx: &mut Context<GodPdfApp>) {
    let Some(tab) = app
        .collaboration
        .session
        .as_ref()
        .filter(|s| s.peer.is_some())
        .and_then(|s| s.tab.as_ref())
        .and_then(WeakEntity::upgrade)
    else {
        return;
    };
    let result = tab.update(cx, |tab, cx| {
        text::finish_editing(tab, cx);
        tools::finish_shared_edit(tab, cx);
        let bytes = tab.shared_bytes();
        let result = tab
            .shared
            .as_mut()
            .ok_or("The shared session ended".to_owned())?
            .pass(bytes);
        cx.notify();
        result
    });
    if let Err(error) = result {
        toasts::error(cx, error);
    }
    cx.notify();
}

/// "123456" shown as "123 456".
fn spaced(code: &str) -> String {
    let (left, right) = code.split_at(code.len() / 2);
    format!("{left} {right}")
}

fn status(session: &Session, editing: bool) -> String {
    match &session.peer {
        Some(peer) if editing => format!("{peer} · You can edit"),
        Some(peer) => format!("{peer} · Viewing your peer’s edits"),
        None if session.invitation.is_some() => "Waiting for the other person…".into(),
        None => "Connecting…".into(),
    }
}

fn can_edit(session: &Session, cx: &Context<GodPdfApp>) -> bool {
    session
        .tab
        .as_ref()
        .and_then(WeakEntity::upgrade)
        .is_some_and(|tab| tab.read(cx).can_edit())
}

fn copy_code(code: &str, cx: &mut gpui::App) {
    cx.write_to_clipboard(ClipboardItem::new_string(code.to_owned()));
    toasts::note(cx, "Code copied");
}

/// Slim bar under the header while a session runs and its popup is closed.
pub(crate) fn panel(
    app: &mut GodPdfApp,
    palette: &Palette,
    _: &mut Window,
    cx: &mut Context<GodPdfApp>,
) -> Option<AnyElement> {
    let session = app.collaboration.session.as_ref()?;
    let active = app.active_tab();
    if app.collaboration.open
        || !app.doc_view()
        || active.map(Entity::downgrade).as_ref() != session.tab.as_ref()
        || active.is_some_and(|tab| tab.read(cx).grid.on)
    {
        session
            .cursor_sender
            .send_if_modified(|cursor| cursor.take().is_some());
    }
    if app.collaboration.open {
        return None;
    }
    let editing = can_edit(session, cx);
    let mut row = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(12.))
        .py(px(4.))
        .bg(palette.surface)
        .border_b_1()
        .border_color(palette.border)
        .text_color(palette.text_dim)
        .child(status(session, editing));
    if let Some(code) = &session.invitation {
        let code = code.clone();
        row = row.child(
            Button::new("copy-share-code")
                .ghost()
                .child(format!("Copy code {}", spaced(&code)))
                .on_click(move |_, _, cx| copy_code(&code, cx)),
        );
    }
    if editing && session.peer.is_some() {
        row = row.child(
            Button::new("pass-editing-turn")
                .ghost()
                .child("Pass editing control")
                .on_click(cx.listener(|app, _, _, cx| pass(app, cx))),
        );
    }
    row = row.child(
        Button::new("leave-shared-pdf")
            .ghost()
            .child("End session")
            .on_click(cx.listener(|app, _, _, cx| leave(app, cx))),
    );
    Some(row.into_any_element())
}

/// The share / join popup shows the short code.
pub(crate) fn overlay(
    app: &mut GodPdfApp,
    window: &mut Window,
    cx: &mut Context<GodPdfApp>,
) -> Option<AnyElement> {
    if !app.collaboration.open {
        return None;
    }
    let palette = Palette::of(cx);
    let card = dialogs::card(&palette, 340.)
        .key_context("GodPdfDialog")
        .on_action(cx.listener(|app, _: &DialogCancel, _, cx| close_popup(app, cx)));
    let close = framed_button("share-close", "Close", true, &palette)
        .on_click(cx.listener(|app, _, _, cx| close_popup(app, cx)));
    let card = match &app.collaboration.session {
        None => {
            let input =
                app.collaboration.code.clone().unwrap_or_else(|| {
                    cx.new(|cx| InputState::new(window, cx).placeholder("123 456"))
                });
            app.collaboration.code = Some(input.clone());
            card.on_action(cx.listener(|app, _: &DialogConfirm, _, cx| join(app, cx)))
                .child(dialogs::title("Join a shared PDF"))
                .child(
                    div()
                        .text_color(palette.text_dim)
                        .child("Enter the 6-digit code from the person sharing the PDF."),
                )
                .child(Input::new(&input))
                .child(
                    dialogs::button_row()
                        .child(
                            accent_button("share-join", "Join", true, &palette)
                                .on_click(cx.listener(|app, _, _, cx| join(app, cx))),
                        )
                        .child(close),
                )
        }
        Some(session) => {
            let editing = can_edit(session, cx);
            let mut card = card
                .on_action(cx.listener(|app, _: &DialogConfirm, _, cx| close_popup(app, cx)))
                .child(dialogs::title(
                    if session.tab.is_some() && session.invitation.is_some() {
                        "Share this PDF"
                    } else {
                        "Shared PDF"
                    },
                ));
            let mut buttons = dialogs::button_row();
            if let Some(code) = &session.invitation {
                let copied = code.clone();
                card = card
                        .child(div().text_color(palette.text_dim).child(
                            "Send this code to the other person. In GodPDF, they click Join and enter the code.",
                        ))
                        .child(
                            div()
                                .py(px(8.))
                                .flex()
                                .justify_center()
                                .font_family(fonts::MONO)
                                .text_size(px(34.))
                                .child(spaced(code)),
                        );
                buttons = buttons.child(
                    accent_button("share-copy", "Copy code", true, &palette)
                        .on_click(move |_, _, cx| copy_code(&copied, cx)),
                );
            }
            card = card.child(
                div()
                    .text_color(palette.text_dim)
                    .child(status(session, editing)),
            );
            if editing && session.peer.is_some() {
                buttons = buttons.child(
                    accent_button("share-pass", "Pass editing control", true, &palette)
                        .on_click(cx.listener(|app, _, _, cx| pass(app, cx))),
                );
            }
            let end = if session.peer.is_some() {
                "End session"
            } else {
                "Cancel"
            };
            card.child(
                buttons
                    .child(
                        framed_button("share-end", end, true, &palette)
                            .on_click(cx.listener(|app, _, _, cx| leave(app, cx))),
                    )
                    .child(close),
            )
        }
    };
    Some(
        div()
            .id("share-backdrop")
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .justify_center()
            .items_center()
            .bg(hsla(0., 0., 0., 100. / 255.))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|app, _, _, cx| close_popup(app, cx)),
            )
            .child(card)
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_editing_turn_blocks_mutations_until_restored() {
        let initial = godpdf_core::blank_document(72.0, 72.0).unwrap();
        let mut tab = DocTab::new(godpdf_core::Session::new("test.pdf", initial));
        let (sender, mut updates) = watch::channel(None);
        tab.shared = Some(SharedDocument {
            editing: true,
            turn: INITIAL_TURN,
            sender,
            cursor_sender: watch::channel(None).0,
            peer_cursor: None,
            peer_name: None,
            hosted: true,
        });
        tab.apply("Add second page", |bytes| {
            godpdf_core::insert_blank_page(bytes, 1, 72.0, 72.0)
        })
        .unwrap();
        assert!(
            updates
                .borrow_and_update()
                .as_ref()
                .is_some_and(|update| update.grant.is_none())
        );

        let snapshot = tab.shared_bytes();
        assert!(
            tab.apply("Oversized edit", |_| Ok(vec![0; MAX_PDF + 1]))
                .is_err()
        );
        assert_eq!(tab.shared_bytes(), snapshot);
        tab.shared.as_mut().unwrap().pass(snapshot.clone()).unwrap();
        let sent = updates.borrow_and_update().as_ref().cloned().unwrap();
        let turn = sent.grant.unwrap();
        assert_eq!(sent.bytes, snapshot);
        assert_ne!(turn, INITIAL_TURN);
        let shared = tab.shared.as_ref().unwrap();
        assert!(!shared.accepts(&sent));
        assert!(shared.accepts(&Revision {
            bytes: snapshot.clone(),
            turn,
            grant: None
        }));
        assert!(
            tab.apply("Unexpected edit", |_| panic!("read-only edit ran"))
                .is_err()
        );
        assert!(
            tab.amend("Unexpected edit", |_| panic!("read-only amend ran"))
                .is_err()
        );
        assert!(tab.undo().is_none());
        assert!(tab.redo().is_none());

        let remote = godpdf_core::insert_blank_page(&snapshot, 2, 72.0, 72.0).unwrap();
        tab.receive_shared(remote).unwrap();
        assert_eq!(tab.page_count(), 3);
        assert!(!updates.has_changed().unwrap());

        tab.shared.as_mut().unwrap().editing = true;
        assert!(tab.undo().is_some());
        assert_eq!(tab.page_count(), 2);
        assert!(updates.has_changed().unwrap());
    }
}
