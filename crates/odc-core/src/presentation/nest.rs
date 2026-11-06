//! P1: one entry root per fragment.
//!
//! A reference panel quotes another document: LDOCE6 nests a `span.entry` in
//! every popup, and the profile promotes all of them to `section.odc-entry`.
//! The presentation layer keeps exactly one entry root, so a
//! `section.odc-entry` that sits inside another one becomes
//! `div.odc-entry-inner`: same attributes, same children, same text — only the
//! entry role goes away.
//!
//! The pass runs after the profile rebuilt the skeleton and before anything
//! reads the entry structure, and it is idempotent: a second run finds nothing
//! to demote.

use html5ever::tendril::StrTendril;
use markup5ever_rcdom::{Handle, NodeData};

use crate::doc;
use crate::presentation::{has_class, tag};

/// Class that marks the demoted copy of a nested entry.
pub(crate) const INNER: &str = "odc-entry-inner";

/// Demote every `section.odc-entry` that sits inside another one.
pub fn demote(root: &Handle) {
    // Two levels of quoting are possible, so repeat until the tree is flat.
    while demote_pass(root) > 0 {}
}

/// Demote one layer and report how many nodes changed.
fn demote_pass(root: &Handle) -> usize {
    let mut targets: Vec<(Handle, usize)> = Vec::new();
    collect(root, false, &mut targets);
    for (parent, index) in &targets {
        demote_one(parent, *index);
    }
    targets.len()
}

/// Gather nested entry sections together with their position in the parent.
fn collect(node: &Handle, inside_entry: bool, out: &mut Vec<(Handle, usize)>) {
    for (index, child) in node.children.borrow().iter().enumerate() {
        if !crate::profile::selector::is_element(child) {
            continue;
        }
        let is_entry = tag(child) == "section" && has_class(child, "odc-entry");
        if is_entry && inside_entry {
            out.push((node.clone(), index));
            continue;
        }
        collect(child, inside_entry || is_entry, out);
    }
}

/// Replace the nested section with a `div.odc-entry-inner`, keeping the
/// remaining classes, every other attribute and all children.
fn demote_one(parent: &Handle, index: usize) {
    let child = match parent.children.borrow().get(index).cloned() {
        Some(child) => child,
        None => return,
    };
    let attrs = match &child.data {
        NodeData::Element { attrs, .. } => attrs.borrow().clone(),
        _ => return,
    };
    let attrs = attrs
        .into_iter()
        .map(|mut attr| {
            if attr.name.local.as_ref() == "class" {
                attr.value = StrTendril::from(inner_classes(&attr.value));
            }
            attr
        })
        .collect();
    let inner = doc::element("div", attrs);
    for kid in doc::take_children(&child) {
        doc::append(&inner, kid);
    }
    doc::replace_at(parent, index, inner);
}

/// Class list with the entry role renamed; every other token stays put.
fn inner_classes(classes: &str) -> String {
    classes
        .split_whitespace()
        .map(|class| if class == "odc-entry" { INNER } else { class })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presentation::elements;

    fn parse(html: &str) -> Handle {
        crate::profile::exec::parse_fragment(html).document
    }

    fn render(html: &str) -> Handle {
        let document = parse(html);
        demote(&document);
        document
    }

    fn entries(root: &Handle) -> usize {
        elements(root)
            .iter()
            .filter(|spot| tag(&spot.node) == "section" && has_class(&spot.node, "odc-entry"))
            .count()
    }

    fn inners(root: &Handle) -> usize {
        elements(root)
            .iter()
            .filter(|spot| tag(&spot.node) == "div" && has_class(&spot.node, INNER))
            .count()
    }

    #[test]
    fn a_single_top_level_entry_is_left_alone() {
        let root = render(r#"<section class="odc-entry" data-odc-key="x"><p>keep</p></section>"#);
        assert_eq!(entries(&root), 1);
        assert_eq!(inners(&root), 0);
    }

    #[test]
    fn one_nested_entry_becomes_a_plain_inner_block() {
        let root = render(
            r#"<section class="odc-entry"><p>outer</p><section class="odc-entry odc-entry--pop"><p>inner</p></section></section>"#,
        );
        assert_eq!(entries(&root), 1);
        assert_eq!(inners(&root), 1);
        let html = crate::profile::exec::serialize(&root);
        assert!(
            html.contains(r#"<div class="odc-entry-inner odc-entry--pop">"#),
            "{html}"
        );
        assert!(html.contains("<p>inner</p>"), "{html}");
    }

    #[test]
    fn two_levels_of_quoting_are_flattened() {
        let root = render(
            r#"<section class="odc-entry"><p>a</p><section class="odc-entry"><p>b</p><section class="odc-entry"><p>c</p></section></section></section>"#,
        );
        assert_eq!(entries(&root), 1);
        assert_eq!(inners(&root), 2);
    }

    #[test]
    fn demotion_keeps_the_text_and_the_attributes() {
        let source = r#"<section class="odc-entry" data-odc-dict="d" data-odc-key="k" data-odc-lang="en"><p>outer</p><section class="odc-entry" data-odc-key="other"><span>quoted text</span></section></section>"#;
        let root = render(source);
        let html = crate::profile::exec::serialize(&root);
        assert!(html.contains(r#"data-odc-key="other""#), "{html}");
        assert!(html.contains("<span>quoted text</span>"), "{html}");
        assert!(html.contains("<p>outer</p>"), "{html}");
    }

    #[test]
    fn the_pass_is_idempotent() {
        let root = render(
            r#"<section class="odc-entry"><p>a</p><section class="odc-entry"><p>b</p></section></section>"#,
        );
        let once = crate::profile::exec::serialize(&root);
        demote(&root);
        assert_eq!(crate::profile::exec::serialize(&root), once);
    }
}
