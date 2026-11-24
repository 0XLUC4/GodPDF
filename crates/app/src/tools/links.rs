//! Links on the pages: a click with the Select or Hand tool follows one (to a page of the
//! document, or a web or mail address in the browser). The Link tool draws new ones.

use std::collections::HashMap;
use std::rc::Rc;

use godpdf_core::LinkTarget;
use gpui::App;

use super::canvas::page_frame;
use super::geom::DRect;
use crate::doc::{DocTab, guarded};
use crate::toasts;

/// The links of each page, where they show, read once per document revision.
#[derive(Default)]
pub struct Links {
    generation: u64,
    pages: HashMap<usize, Rc<Vec<(DRect, LinkTarget)>>>,
}

/// The links of `page` in displayed points. A page whose links cannot be read has none: its
/// comments, read from the same place, say why.
pub(super) fn of_page(tab: &mut DocTab, page: usize) -> Rc<Vec<(DRect, LinkTarget)>> {
    let generation = tab.generation();
    if tab.canvas.links.generation != generation {
        tab.canvas.links = Links {
            generation,
            pages: HashMap::new(),
        };
    }
    if let Some(found) = tab.canvas.links.pages.get(&page) {
        return Rc::clone(found);
    }
    let shown: Vec<_> = match page_frame(tab, page) {
        Some((geom, displayed)) => guarded(|| godpdf_core::links(tab.session.bytes(), page))
            .unwrap_or_default()
            .into_iter()
            .map(|link| (DRect::from_pdf(&geom, displayed, &link.rect), link.target))
            .collect(),
        None => Vec::new(),
    };
    let shown = Rc::new(shown);
    tab.canvas.links.pages.insert(page, Rc::clone(&shown));
    shown
}

/// Where the topmost link under `point` (displayed points of `page`) leads.
pub(super) fn at(tab: &mut DocTab, page: usize, point: (f32, f32)) -> Option<LinkTarget> {
    of_page(tab, page)
        .iter()
        .rev()
        .find(|(rect, _)| rect.contains(point, 0.0))
        .map(|(_, target)| target.clone())
}

/// Go where `target` leads: a page of the document, or a web or mail address outside.
pub(super) fn follow(tab: &mut DocTab, target: LinkTarget, cx: &mut App) {
    match target {
        LinkTarget::Page(page) if page < tab.page_count() => tab.scroll_to_page(page),
        LinkTarget::Page(page) => toasts::error(
            cx,
            format!("This link leads to page {}, which is not there", page + 1),
        ),
        // Core reads no other kind; checked again before anything leaves the app.
        LinkTarget::Uri(uri) if godpdf_core::is_web_uri(&uri) => cx.open_url(&uri),
        LinkTarget::Uri(_) => {}
    }
}
