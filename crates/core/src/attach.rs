//! Files carried inside a document (the /EmbeddedFiles name tree).

use std::collections::HashSet;

use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary, text_string};

use crate::info::text_entry;
use crate::pdf::{deref, dict_entry, load, save};
use crate::{Error, Result};

const MAX_DEPTH: usize = 32;

/// A file attached to the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// Every file attached to the document, in name order.
pub fn attachments(pdf: &[u8]) -> Result<Vec<Attachment>> {
    let doc = load(pdf)?;
    Ok(entries(&doc)
        .into_iter()
        .filter_map(|(key, spec)| {
            let name = spec_name(&doc, &spec).unwrap_or(key);
            let spec = deref(&doc, &spec)?.as_dict().ok()?;
            let files = dict_entry(&doc, spec, b"EF")?;
            let stream = files
                .get(b"UF")
                .or_else(|_| files.get(b"F"))
                .ok()
                .and_then(|file| deref(&doc, file))?
                .as_stream()
                .ok()?;
            let bytes = stream
                .decompressed_content()
                .unwrap_or_else(|_| stream.content.clone());
            Some(Attachment { name, bytes })
        })
        .collect())
}

/// Attach `bytes` under `name`; a file of the same name is replaced.
pub fn add_attachment(pdf: &[u8], name: &str, bytes: &[u8]) -> Result<Vec<u8>> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::InvalidArgument("the file has no name".into()));
    }
    let mut doc = load(pdf)?;
    let mut list = entries(&doc);
    list.retain(|(key, spec)| key != name && spec_name(&doc, spec).as_deref() != Some(name));
    let file = Stream::new(
        dictionary! {
            "Type" => "EmbeddedFile",
            "Params" => dictionary! { "Size" => bytes.len() as i64 },
        },
        bytes.to_vec(),
    );
    let file = doc.add_object(file);
    let spec = doc.add_object(dictionary! {
        "Type" => "Filespec",
        "F" => text_string(name),
        "UF" => text_string(name),
        "EF" => dictionary! { "F" => file, "UF" => file },
    });
    list.push((name.to_owned(), Object::Reference(spec)));
    write(&mut doc, list)?;
    save(&mut doc)
}

/// Take the file `name` out of the document.
pub fn remove_attachment(pdf: &[u8], name: &str) -> Result<Vec<u8>> {
    let mut doc = load(pdf)?;
    let mut list = entries(&doc);
    let before = list.len();
    list.retain(|(key, spec)| key != name && spec_name(&doc, spec).as_deref() != Some(name));
    if list.len() == before {
        return Err(Error::InvalidArgument(format!("no attached file “{name}”")));
    }
    write(&mut doc, list)?;
    save(&mut doc)
}

fn spec_name(doc: &Document, spec: &Object) -> Option<String> {
    let spec = deref(doc, spec)?.as_dict().ok()?;
    text_entry(doc, spec, b"UF").or_else(|| text_entry(doc, spec, b"F"))
}

/// (key, file specification) pairs of the name tree, flattened.
fn entries(doc: &Document) -> Vec<(String, Object)> {
    let mut out = Vec::new();
    let tree = doc
        .catalog()
        .ok()
        .and_then(|catalog| dict_entry(doc, catalog, b"Names"))
        .and_then(|names| dict_entry(doc, names, b"EmbeddedFiles"));
    if let Some(tree) = tree {
        collect(doc, tree, &mut out, &mut HashSet::new(), 0);
    }
    out
}

fn collect(
    doc: &Document,
    node: &Dictionary,
    out: &mut Vec<(String, Object)>,
    seen: &mut HashSet<ObjectId>,
    depth: usize,
) {
    if depth > MAX_DEPTH {
        return;
    }
    if let Some(pairs) = node
        .get(b"Names")
        .ok()
        .and_then(|n| deref(doc, n))
        .and_then(|n| n.as_array().ok())
    {
        for pair in pairs.chunks(2) {
            if let [key, value] = pair
                && let Some(key) = deref(doc, key).and_then(|k| lopdf::decode_text_string(k).ok())
            {
                out.push((key, value.clone()));
            }
        }
    }
    let kids = node
        .get(b"Kids")
        .ok()
        .and_then(|k| deref(doc, k))
        .and_then(|k| k.as_array().ok());
    for kid in kids.into_iter().flatten() {
        if let Object::Reference(id) = kid
            && !seen.insert(*id)
        {
            continue;
        }
        if let Some(kid) = deref(doc, kid).and_then(|k| k.as_dict().ok()) {
            collect(doc, kid, out, seen, depth + 1);
        }
    }
}

/// Write `list` as a single-node name tree, sorted by key; no tree when it is empty.
fn write(doc: &mut Document, mut list: Vec<(String, Object)>) -> Result<()> {
    list.sort_by(|a, b| a.0.cmp(&b.0));
    let tree = (!list.is_empty()).then(|| {
        let names: Vec<Object> = list
            .into_iter()
            .flat_map(|(key, spec)| [text_string(&key), spec])
            .collect();
        doc.add_object(dictionary! { "Names" => names })
    });
    let mut names = dict_entry(doc, doc.catalog()?, b"Names")
        .cloned()
        .unwrap_or_default();
    match tree {
        Some(tree) => names.set("EmbeddedFiles", tree),
        None => drop(names.remove(b"EmbeddedFiles")),
    }
    let catalog = doc.catalog_mut()?;
    if names.is_empty() {
        catalog.remove(b"Names");
    } else {
        catalog.set("Names", names);
    }
    Ok(())
}
