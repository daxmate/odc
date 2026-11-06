//! P1: the document skeleton and the order of an entry's blocks.

use markup5ever_rcdom::Handle;

use crate::doc;
use crate::presentation::{attr, elements, has_class, tag};

/// Rebuild the skeleton: entries at the top level, blocks in spec order.
pub fn arrange(root: &Handle) {
    lift_sections(root);
    number_homographs(root);
    for spot in elements(root) {
        if tag(&spot.node) == "section" && has_class(&spot.node, "odc-entry") {
            order_entry(&spot.node);
        }
    }
    lift_sections(root);
}

/// Move `section.odc-entry` out of any wrapper, keeping its leading siblings.
fn lift_sections(root: &Handle) {
    loop {
        let mut moved = false;
        let children = root.children.borrow().clone();
        for (index, child) in children.iter().enumerate() {
            if tag(child) != "div" || has_class(child, "odc-entry") {
                continue;
            }
            let inner = child.children.borrow().clone();
            if !inner
                .iter()
                .any(|kid| tag(kid) == "section" && has_class(kid, "odc-entry"))
            {
                continue;
            }
            let mut out: Vec<Handle> = Vec::new();
            let mut pending: Vec<Handle> = Vec::new();
            for kid in doc::take_children(child) {
                if tag(&kid) == "section" && has_class(&kid, "odc-entry") {
                    // Leading siblings move inside the section they introduce.
                    let mut kids = std::mem::take(&mut pending);
                    kids.extend(doc::take_children(&kid));
                    *kid.children.borrow_mut() = kids;
                    out.push(kid);
                } else {
                    pending.push(kid);
                }
            }
            out.extend(pending);
            let mut kids = root.children.borrow_mut();
            kids.remove(index);
            for (offset, kid) in out.into_iter().enumerate() {
                kids.insert(index + offset, kid);
            }
            drop(kids);
            moved = true;
            break;
        }
        if !moved {
            return;
        }
    }
}

/// Number the entries of one key when a record carries several homographs.
fn number_homographs(root: &Handle) {
    let entries: Vec<Handle> = root
        .children
        .borrow()
        .iter()
        .filter(|child| tag(child) == "section" && has_class(child, "odc-entry"))
        .cloned()
        .collect();
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for entry in &entries {
        let key = attr(entry, "data-odc-key").unwrap_or_default();
        *counts.entry(key).or_insert(0) += 1;
    }
    let mut seen: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for entry in entries {
        let key = attr(&entry, "data-odc-key").unwrap_or_default();
        let total = counts.get(&key).copied().unwrap_or(1);
        if total < 2 {
            continue;
        }
        let ordinal = seen.entry(key).or_insert(0);
        *ordinal += 1;
        doc::set_attr(&entry, "data-odc-homograph", &ordinal.to_string());
    }
}

/// Put lead, header, reference bar and body blocks in that order.
fn order_entry(entry: &Handle) {
    let children = entry.children.borrow().clone();
    let mut lead: Vec<Handle> = Vec::new();
    let mut header: Option<Handle> = None;
    let mut bar: Option<Handle> = None;
    let mut body: Vec<Handle> = Vec::new();
    for child in children {
        if !crate::profile::selector::is_element(&child) {
            continue;
        }
        let name = tag(&child);
        if name == "header" && has_class(&child, "odc-head") {
            header = Some(child);
            continue;
        }
        if name == "nav" && has_class(&child, "odc-refbar") {
            bar = Some(child);
            continue;
        }
        if is_lead(&child) {
            lead.push(child);
            continue;
        }
        body.push(child);
    }
    let mut ordered = lead;
    if let Some(header) = header {
        ordered.push(header);
    }
    if let Some(bar) = bar {
        ordered.push(bar);
    }
    ordered.extend(body);
    *entry.children.borrow_mut() = ordered;
}

/// Whether the node opens the entry (the homograph bar).
fn is_lead(node: &Handle) -> bool {
    crate::presentation::classes(node)
        .iter()
        .any(|class| class == "chwd")
        || attr(node, "data-odc-orig-class").as_deref() == Some("chwd")
}
