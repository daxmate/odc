//! P5/P7: derive `data-odc-orig-*` attributes and resource URLs.
//!
//! The source fragment is stamped before any rule runs, so a rule that renames,
//! unwraps or moves a node cannot lose the provenance. Stamping is engine work:
//! a rule pack never declares these attributes.

use markup5ever_rcdom::Handle;

use crate::doc;
use crate::presentation::{attr, elements, has_class, resource, tag, Spot, LOCK};

/// Attribute that carries a source anchor until the sense pass claims it.
pub(crate) const ANCHOR: &str = "data-odc-anchor";

/// Attribute that marks a bare source anchor element.
///
/// A rule that rewrites a block keeps its children, so the anchor element is
/// the most durable carrier: P6 maps it to the id of the block that contains
/// it, and a page link that used to point at a source uuid keeps pointing at
/// the same place.
pub(crate) const ANCHOR_SELF: &str = "data-odc-anchor-self";

/// Stamp the source fragment: anchors, media resources and cross references.
///
/// Returns the self-check notes for references the engine could not resolve;
/// they travel with the document instead of being swallowed here.
pub fn capture(root: &Handle) -> Vec<String> {
    let mut notes = Vec::new();
    let spots = elements(root);
    let mut pending: Vec<(usize, String)> = Vec::new();
    for (index, spot) in spots.iter().enumerate() {
        match tag(&spot.node).as_str() {
            "a" => {
                let href = attr(&spot.node, "href");
                let anchor = attr(&spot.node, "id").or_else(|| attr(&spot.node, "name"));
                match href.as_deref() {
                    Some(href) if href.starts_with("sound://") => audio(spot, href, &mut notes),
                    Some(href) if href.starts_with("entry://") => crossref(spot, href),
                    Some(_) => {}
                    None => {
                        if let Some(anchor) = anchor {
                            doc::set_attr(&spot.node, ANCHOR_SELF, &anchor);
                            pending.push((index, anchor));
                        }
                    }
                }
            }
            "img" | "source" => media(spot, &mut notes),
            _ => {}
        }
    }
    // An anchor belongs to the block that follows it.
    for (index, anchor) in pending {
        if let Some(next) = spots.get(index + 1) {
            doc::set_attr(&next.node, ANCHOR, &anchor);
        }
    }
    notes
}

/// Turn a `sound://` link into an audio button (P7).
fn audio(spot: &Spot, href: &str, notes: &mut Vec<String>) {
    let variant = if href.contains("/bre/") {
        "uk"
    } else if href.contains("/ame/") {
        "us"
    } else {
        ""
    };
    let mut attrs = Vec::new();
    let resolved = resource::resolve(href);
    if resolved.is_resolved() {
        attrs.push(doc::make_attr("data-odc-src", &resolved.url(href)));
    } else {
        note_unresolved("audio", href, &resolved, notes);
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

/// Publish a media reference as an `odc-resource://` key.
fn media(spot: &Spot, notes: &mut Vec<String>) {
    let Some(src) = attr(&spot.node, "src") else {
        return;
    };
    let resolved = resource::resolve(&src);
    doc::set_attr(&spot.node, "src", &resolved.url(&src));
    // The provenance is written even for a kept-verbatim reference: it is what
    // makes the element part of the entry instead of loose markup, and it keeps
    // the original URL recoverable after a later pass rewrites the key.
    doc::set_attr(&spot.node, "data-odc-orig-src", &src);
    if !resolved.is_resolved() {
        note_unresolved(&tag(&spot.node), &src, &resolved, notes);
    }
}

/// Record why a resource stayed verbatim (P8 note, never a failure).
fn note_unresolved(
    kind: &str,
    reference: &str,
    resolved: &resource::Resolved,
    notes: &mut Vec<String>,
) {
    if let Some(reason) = &resolved.reason {
        let note = format!("kept {kind} {reference:?}: {reason}");
        if !notes.contains(&note) {
            notes.push(note);
        }
    }
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
        doc::remove_attr(&spot.node, ANCHOR_SELF);
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
