//! A file changed by another program is read again, unless it was changed here too:
//! the files of the open documents are looked at every 2 s, off the UI thread (a file on a
//! sleeping network drive can take seconds to answer).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use godpdf_core::Session;
use gpui::{App, Context, EntityId, Global};

use crate::app::GodPdfApp;
use crate::{files, toasts};

/// How often the files are looked at.
const EVERY: Duration = Duration::from_secs(2);

/// When the file of each document was last seen written.
#[derive(Default)]
struct Watch(HashMap<EntityId, SystemTime>);

impl Global for Watch {}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

/// Look at the files every [`EVERY`] for as long as the app lives.
pub fn start(cx: &mut Context<GodPdfApp>) {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(EVERY).await;
            let Ok(files) = this.update(cx, |app, cx| {
                app.tabs
                    .iter()
                    .filter_map(|tab| Some((tab.entity_id(), tab.read(cx).session.path.clone()?)))
                    .collect::<Vec<_>>()
            }) else {
                break;
            };
            let seen = cx
                .background_executor()
                .spawn(async move {
                    files
                        .into_iter()
                        .map(|(id, path)| {
                            let time = modified(&path);
                            (id, path, time)
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            if this.update(cx, |app, cx| check(app, seen, cx)).is_err() {
                break;
            }
        }
    })
    .detach();
}

/// The document `tab` was just written to `path` here: not a change by another program.
pub fn saved(tab: EntityId, path: &Path, cx: &mut App) {
    if let Some(time) = modified(path) {
        cx.default_global::<Watch>().0.insert(tab, time);
    }
}

/// What was `seen` of each document's file: its tab, path and modified time.
fn check(
    app: &mut GodPdfApp,
    seen: Vec<(EntityId, PathBuf, Option<SystemTime>)>,
    cx: &mut Context<GodPdfApp>,
) {
    let open: Vec<_> = app.tabs.iter().map(|tab| tab.entity_id()).collect();
    cx.default_global::<Watch>()
        .0
        .retain(|id, _| open.contains(id));
    for (id, path, time) in seen {
        let Some(tab) = app.tabs.iter().find(|tab| tab.entity_id() == id).cloned() else {
            continue;
        };
        // Saved elsewhere meanwhile: the next look sees the new file.
        if tab.read(cx).session.path.as_ref() != Some(&path) {
            continue;
        }
        let Some(time) = time else {
            continue;
        };
        // Text being typed is not in the document yet: read the file again once it is.
        let typing = {
            let doc = tab.read(cx);
            doc.text.editing() || doc.canvas.typing()
        };
        if typing {
            continue;
        }
        // First look: just remember it.
        let known = cx.default_global::<Watch>().0.insert(id, time);
        if known.is_none_or(|known| known == time) {
            continue;
        }
        let (name, dirty) = {
            let doc = tab.read(cx);
            (doc.title().to_owned(), doc.is_dirty())
        };
        if dirty {
            toasts::error(
                cx,
                format!("“{name}” was changed by another program; what you changed here is kept"),
            );
            continue;
        }
        match files::read_file(&path) {
            Ok(file) if !godpdf_core::is_encrypted(&file.bytes) => {
                tab.update(cx, |tab, cx| {
                    let mut session = Session::new(file.name, file.bytes);
                    session.path = Some(file.path);
                    tab.session = session;
                    tab.canvas.forget_selection();
                    tab.reload();
                    cx.notify();
                });
                toasts::info(
                    cx,
                    format!("“{name}” was changed by another program: read again"),
                );
            }
            _ => toasts::error(
                cx,
                format!("“{name}” was changed by another program; open it again to see it"),
            ),
        }
    }
}
