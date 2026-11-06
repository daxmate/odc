//! Presentation layer: turns a slot-marked document into L1.
//!
//! The profile answers *what* a piece of markup is; this module answers *how it
//! is laid out*. It owns the document skeleton, the chrome (header, reference
//! bar, entry menu), the derived attributes (`data-odc-orig-*`), the anchor ids
//! and the self-check. Nothing here can be overridden by a rule pack.

mod anchors;
mod check;
mod chrome;
mod derive;
mod nest;
mod order;
mod resource;
mod state;

use std::rc::Rc;

use markup5ever_rcdom::Handle;

use crate::adapters::ParseContext;

pub use check::{CheckItem, CheckReport};

/// Stamp the source fragment before any rule runs.
///
/// The returned notes describe references the engine kept verbatim; they are
/// fed back into the self-check once the presentation passes have run.
pub fn capture(root: &Handle) -> Vec<String> {
    derive::capture(root)
}

/// Attribute that marks a node as engine-owned; rule packs skip those nodes.
pub const LOCK: &str = "data-odc-locked";

/// Run every presentation pass over a rule-processed document.
pub fn apply(root: &Handle, ctx: &ParseContext, mut notes: Vec<String>) -> CheckReport {
    // The profile rebuilds the skeleton from the source nesting, so every
    // quoted span.entry arrives as a second section.odc-entry. Flatten them
    // before anything reads the entry structure (P1).
    nest::demote(root);
    order::arrange(root);
    chrome::assemble(root, ctx);
    anchors::assign(root, ctx);
    // Source anchors must be mapped before the provenance pass drops the
    // bookkeeping attribute that carries the owner.
    notes.extend(anchors::alias(root));
    // Source runtime state goes before provenance: a box that was open is not
    // part of the entry's identity, so `data-odc-orig-class` must not record it.
    state::strip(root);
    derive::finish(root);
    check::run(root, ctx, notes)
}

/// One element together with its position in the tree.
pub(crate) struct Spot {
    pub parent: Handle,
    pub index: usize,
    pub node: Handle,
}

impl Spot {
    /// Remove the element from its parent, returning it.
    pub fn take(self) -> Handle {
        if let Some(node) = crate::doc::remove_at(&self.parent, self.index) {
            return node;
        }
        self.node
    }
}

/// Every element under 'root', in document order.
pub(crate) fn elements(root: &Handle) -> Vec<Spot> {
    let mut out = Vec::new();
    collect(root, &mut out);
    out
}

fn collect(parent: &Handle, out: &mut Vec<Spot>) {
    let children = parent.children.borrow().clone();
    for (index, child) in children.iter().enumerate() {
        if crate::profile::selector::is_element(child) {
            out.push(Spot {
                parent: Rc::clone(parent),
                index,
                node: Rc::clone(child),
            });
            collect(child, out);
        }
    }
}

/// Class list of an element.
pub(crate) fn classes(node: &Handle) -> Vec<String> {
    crate::profile::selector::class_list(node)
}

/// Whether 'node' carries 'class'.
pub(crate) fn has_class(node: &Handle, class: &str) -> bool {
    classes(node).iter().any(|item| item == class)
}

/// Tag name of an element.
pub(crate) fn tag(node: &Handle) -> String {
    crate::profile::selector::element_name(node).unwrap_or_default()
}

/// Attribute value of an element.
pub(crate) fn attr(node: &Handle, name: &str) -> Option<String> {
    crate::profile::selector::attr(node, name)
}

/// Whether 'node' is owned by the presentation layer.
pub(crate) fn locked(node: &Handle) -> bool {
    attr(node, LOCK).is_some()
}
