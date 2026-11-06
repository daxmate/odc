//! P5/P7: derive `data-odc-orig-*` attributes and resource URLs.
//!
//! The source fragment is stamped before any rule runs, so a rule that renames,
//! unwraps or moves a node cannot lose the provenance. Stamping is engine work:
//! a rule pack never declares these attributes.

use markup5ever_rcdom::Handle;

use crate::doc;
use crate::presentation::{attr, elements, has_class, tag, Spot, LOCK};

/// Attribute that carries a source anchor until the sense pass claims it.
const ANCHOR: &str = "data-odc-anchor";

/// Stamp the source fragment: anchors, audio resources and cross references.
pub fn capture(root: &Handle) {
    let spots = elements(root);
    let mut pending: Vec<(usize, String)> = Vec::new();
    for (index, spot) in spots.iter().enumerate() {
        if tag(&spot.node) != "a" {
            continue;
        }
        let href = attr(&spot.node, "href");
        let anchor = attr(&spot.node, "id").or_else(|| attr(&spot.node, "name"));
        match href.as_deref() {
            Some(href) if href.starts_with("sound://") => audio(spot, href),
            Some(href) if href.starts_with("entry://") => crossref(spot, href),
            Some(_) => {}
            None => {
                if let Some(anchor) = anchor {
                    pending.push((index, anchor));
                }
            }
        }
    }
    // An anchor belongs to the block that follows it.
    for (index, anchor) in pending {
        if let Some(next) = spots.get(index + 1) {
            doc::set_attr(&next.node, ANCHOR, &anchor);
        }
    }
}

/// Turn a `sound://` link into an audio button (P7).
fn audio(spot: &Spot, href: &str) {
    let variant = if href.contains("/bre/") {
        "uk"
    } else if href.contains("/ame/") {
        "us"
    } else {
        ""
    };
    let mut attrs = Vec::new();
    if let Some(key) = crate::adapters::ldoce6::resource_key(href) {
        attrs.push(doc::make_attr("data-odc-src", &key));
    }
    attrs.push(doc::make_attr("data-odc-orig-src", href));
    if !variant.is_empty() {
        attrs.push(doc::make_attr("data-odc-variant", variant));
    }
    attrs.push(doc::make_attr("class", "odc-audio"));
    attrs.push(doc::make_attr(LOCK, ""));
    let button = doc::element("button", attrs);
    doc::replace_at(&spot.parent, spot.index, button);
}

/// Turn an entry link into a cross reference.
fn crossref(spot: &Spot, href: &str) {
    let target = href.trim_start_matches("entry://").trim_start_matches('#');
    doc::set_attr(&spot.node, "href", &format!("#{target}"));
    doc::set_attr(&spot.node, "class", "odc-crossref");
    doc::set_attr(&spot.node, "data-odc-target", target);
    doc::set_attr(&spot.node, "data-odc-orig-target", href);
    doc::set_attr(&spot.node, LOCK, "");
}

/// Post-pass: publish the derived attributes and hide source classes.
pub fn finish(root: &Handle) {
    for spot in elements(root) {
        let anchor = attr(&spot.node, ANCHOR);
        doc::remove_attr(&spot.node, ANCHOR);
        if let Some(anchor) = anchor {
            if has_class(&spot.node, "odc-sense") || has_class(&spot.node, "odc-subsense") {
                doc::set_attr(&spot.node, "data-odc-orig-anchor", &anchor);
            }
        }
        demote_class(&spot.node);
        // The lock is an executor marker, never part of L1.
        doc::remove_attr(&spot.node, LOCK);
    }
}

/// Move a source `class` into `data-odc-orig-class` on non-slot elements.
fn demote_class(node: &Handle) {
    let Some(value) = attr(node, "class") else {
        return;
    };
    let classes = crate::presentation::classes(node);
    if classes.is_empty() || classes.iter().any(|class| class.starts_with("odc-")) {
        return;
    }
    doc::remove_attr(node, "class");
    doc::set_attr(node, "data-odc-orig-class", &value);
}
