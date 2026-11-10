//! A file changed by another program is read again, unless it was changed here too
//! (egui `GodApp::watch_files`): the files of the open documents are looked at every 2 s.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, SystemTime};

use godpdf_core::Session;
use gpui::{App, Context, EntityId, Global};

use crate::app::GodPdfApp;
use crate::{files, toasts};

/// egui `DISK_WATCH`.
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
            if this.update(cx, check).is_err() {
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

fn check(app: &mut GodPdfApp, cx: &mut Context<GodPdfApp>) {
    let open: Vec<_> = app.tabs.iter().map(|tab| tab.entity_id()).collect();
    cx.default_global::<Watch>()
        .0
        .retain(|id, _| open.contains(id));
    for tab in app.tabs.clone() {
        let id = tab.entity_id();
        let Some(path) = tab.read(cx).session.path.clone() else {
            continue;
        };
        let Some(time) = modified(&path) else {
            continue;
        };
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
            toasts::info(
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
                    tab.reload();
                    cx.notify();
                });
                toasts::info(
                    cx,
                    format!("“{name}” was changed by another program: read again"),
                );
            }
            _ => toasts::info(
                cx,
                format!("“{name}” was changed by another program; open it again to see it"),
            ),
        }
    }
}
