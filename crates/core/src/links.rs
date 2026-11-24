//! Links of a page: where each /Link annotation leads, and new links.

use std::collections::HashMap;

use lopdf::{Object, ObjectId, dictionary};

use crate::annotate::{annots_entries, normalized, read_rect, set_annots};
use crate::info::{dest_page, item_dest};
use crate::pdf::{check_page, deref, dict_entry, load, page_ids, save};
use crate::{Error, Rect, Result};

/// Where a link leads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkTarget {
    /// A page of the same document, 0-based.
    Page(usize),
    /// An address opened outside: `http`, `https` or `mailto` (see [`is_web_uri`]).
    Uri(String),
}

/// A link of a page: the area to click (PDF space) and where it leads.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub rect: Rect,
    pub target: LinkTarget,
}

/// True for the addresses a link may open: `http://`, `https://` and `mailto:` ones.
pub fn is_web_uri(uri: &str) -> bool {
    let lower = uri.trim().to_ascii_lowercase();
    ["http://", "https://", "mailto:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme) && lower.len() > scheme.len())
}

/// The links of `page`, in drawing order. Links going nowhere this app can follow (another
/// file, a script, an unknown named destination) are left out.
pub fn links(pdf: &[u8], page: usize) -> Result<Vec<Link>> {
    let doc = load(pdf)?;
    let ids = page_ids(&doc);
    check_page(page, ids.len())?;
    let pages: HashMap<ObjectId, usize> = ids.iter().enumerate().map(|(i, &id)| (id, i)).collect();
    let found = annots_entries(&doc, ids[page])?
        .iter()
        .filter_map(|entry| {
            let dict = deref(&doc, entry)?.as_dict().ok()?;
            if dict.get(b"Subtype").and_then(Object::as_name).ok()? != b"Link" {
                return None;
            }
            let target = match item_dest(&doc, dict).and_then(|dest| dest_page(&dest, &pages)) {
                Some(page) => LinkTarget::Page(page),
                None => LinkTarget::Uri(uri_of(&doc, dict)?),
            };
            Some(Link {
                rect: read_rect(&doc, dict)?,
                target,
            })
        })
        .collect();
    Ok(found)
}

/// The address of a /URI action, when it is one a link may open.
fn uri_of(doc: &lopdf::Document, link: &lopdf::Dictionary) -> Option<String> {
    let action = dict_entry(doc, link, b"A")?;
    if action.get(b"S").and_then(Object::as_name).ok()? != b"URI" {
        return None;
    }
    let bytes = deref(doc, action.get(b"URI").ok()?)?.as_str().ok()?;
    let uri = String::from_utf8_lossy(bytes).trim().to_owned();
    is_web_uri(&uri).then_some(uri)
}

/// Add an invisible link over `rect` (PDF space) of `page`, leading to `target`.
pub fn add_link(pdf: &[u8], page: usize, rect: Rect, target: &LinkTarget) -> Result<Vec<u8>> {
    let rect = normalized(rect)?;
    if rect.x1 - rect.x0 < 1.0 || rect.y1 - rect.y0 < 1.0 {
        return Err(Error::InvalidArgument("the link area is too small".into()));
    }
    let mut doc = load(pdf)?;
    let ids = page_ids(&doc);
    check_page(page, ids.len())?;
    let page_id = ids[page];
    let mut link = dictionary! {
        "Type" => "Annot",
        "Subtype" => "Link",
        "Rect" => [rect.x0, rect.y0, rect.x1, rect.y1].map(Object::Real).to_vec(),
        "Border" => vec![0.into(), 0.into(), 0.into()],
        "P" => page_id,
    };
    match target {
        LinkTarget::Page(to) => {
            check_page(*to, ids.len())?;
            link.set("Dest", vec![ids[*to].into(), "Fit".into()]);
        }
        LinkTarget::Uri(uri) if is_web_uri(uri) => link.set(
            "A",
            dictionary! { "S" => "URI", "URI" => Object::string_literal(uri.trim()) },
        ),
        LinkTarget::Uri(uri) => {
            return Err(Error::InvalidArgument(format!(
                "“{uri}” is not a web or mail address"
            )));
        }
    }
    let link_id = doc.add_object(link);
    let mut entries = annots_entries(&doc, page_id)?;
    entries.push(link_id.into());
    set_annots(&mut doc, page_id, entries)?;
    save(&mut doc)
}
