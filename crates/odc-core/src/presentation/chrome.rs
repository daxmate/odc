//! P2/P3/P4: header, reference bar and entry menu.
//!
//! The bar is assembled from the blocks the profile marked as
//! `div.odc-ref-block` and classified through `span.odc-ref-title`. Grouping,
//! order and the `details`/summary chrome are engine decisions: no rule pack
//! can change them. The label text itself is never written by the engine: it
//! is the caption the profile kept from the source (`span.odc-ref-caption`).

use std::rc::Rc;

use markup5ever_rcdom::{Handle, NodeData};

use crate::adapters::ParseContext;
use crate::doc;
use crate::presentation::{attr, elements, has_class, tag, Spot};

/// Class of the source caption that names a reference block (B).
const CAPTION: &str = "odc-ref-caption";

/// Toolbar order for the kinds the vocabulary promotes to buttons.
const REF_ORDER: [&str; 8] = [
    "toc",
    "word-origin",
    "verb-table",
    "examples-other",
    "thesaurus",
    "collocation",
    "phrase-index",
    "word-family",
];

/// Build the reference bar of every entry.
pub fn assemble(root: &Handle, _ctx: &ParseContext) {
    for spot in elements(root) {
        if tag(&spot.node) == "section" && has_class(&spot.node, "odc-entry") {
            build_bar(&spot.node);
        }
    }
}

/// One `details` panel of the bar.
struct Group {
    kind: String,
    caption: String,
    blocks: Vec<(Handle, String)>,
}

fn build_bar(entry: &Handle) {
    // A source caption (`span.odc-ref-caption`) names the block that follows
    // it. Pair the two before detaching anything, so the caption text can
    // become the summary label instead of a loose run of text.
    let mut pending: Option<Spot> = None;
    let mut pairs: Vec<(Spot, Option<Spot>)> = Vec::new();
    for spot in elements(entry) {
        if has_class(&spot.node, CAPTION) {
            pending = Some(spot);
        } else if has_class(&spot.node, "odc-ref-block") {
            let caption = match pending.take() {
                Some(caption) if adjacent_caption(&caption, &spot) => Some(caption),
                other => {
                    pending = other;
                    None
                }
            };
            pairs.push((spot, caption));
        }
    }
    if pairs.is_empty() {
        return;
    }
    // Detach from the end: removing a node only shifts the indices of the
    // siblings that follow it.
    let mut blocks: Vec<(Handle, String)> = Vec::new();
    for (block, caption) in pairs.into_iter().rev() {
        let text = caption.as_ref().map_or_else(String::new, |spot| {
            crate::profile::selector::text_content(&spot.node)
                .trim()
                .to_string()
        });
        // The block sits after its caption in the same parent: take it first
        // so the caption index is still valid.
        blocks.push((block.take(), text));
        if let Some(spot) = caption {
            spot.take();
        }
    }
    blocks.reverse();
    let mut groups: Vec<Group> = Vec::new();
    for (block, caption) in blocks {
        let kind = describe_block(&block).0;
        match groups.iter_mut().find(|group| group.kind == kind) {
            Some(group) => {
                let leftover = if group.caption.is_empty() && !caption.is_empty() {
                    group.caption = caption;
                    String::new()
                } else {
                    caption
                };
                group.blocks.push((block, leftover));
            }
            None => groups.push(Group {
                kind,
                caption,
                blocks: vec![(block, String::new())],
            }),
        }
    }
    groups.sort_by_key(|group| ref_rank(&group.kind));
    let bar = doc::element("nav", vec![doc::make_attr("class", "odc-refbar")]);
    for group in groups {
        doc::append(&bar, panel(&group));
    }
    // The bar sits right after the header.
    let header = entry
        .children
        .borrow()
        .iter()
        .position(|child| tag(child) == "header" && has_class(child, "odc-head"));
    let at = header.map_or(0, |index| index + 1);
    doc::insert_at(entry, at, bar);
}

/// Whether `caption` sits directly before `block`, with only blank text
/// between them.
fn adjacent_caption(caption: &Spot, block: &Spot) -> bool {
    if !Rc::ptr_eq(&caption.parent, &block.parent) || caption.index >= block.index {
        return false;
    }
    let children = caption.parent.children.borrow();
    children[caption.index + 1..block.index].iter().all(|child| {
        matches!(&child.data, NodeData::Text { contents } if contents.borrow().trim().is_empty())
    })
}

/// Read the kind, the source and the title of a reference block.
fn describe_block(block: &Handle) -> (String, String, String) {
    let mut kind = attr(block, "data-odc-kind").unwrap_or_default();
    let mut title = String::new();
    for spot in elements(block) {
        if !has_class(&spot.node, "odc-ref-title") {
            continue;
        }
        if kind.is_empty() {
            kind = attr(&spot.node, "data-odc-kind").unwrap_or_default();
        }
        title = crate::profile::selector::text_content(&spot.node);
        break;
    }
    if kind.is_empty() {
        kind = String::from("other");
    }
    let title = title.trim().to_string();
    (kind, source_of(&title), title)
}

/// Where the panel content comes from, read from its caption.
fn source_of(title: &str) -> String {
    if title.contains("CORPUS") {
        String::from("corpus")
    } else if title.contains("OTHER") {
        String::from("other-entries")
    } else {
        String::from("entry")
    }
}

/// Build one collapsed panel.
fn panel(group: &Group) -> Handle {
    let details = doc::element(
        "details",
        vec![
            doc::make_attr("class", "odc-refbar__item"),
            doc::make_attr("data-odc-kind", &group.kind),
        ],
    );
    // B (2026-11-07): the label repeats the caption the source spelled out,
    // so the entry keeps its own words. The empty `data-odc-ui` slot names the
    // kind a client localises; marking the label itself would declare source
    // text to be chrome and hide it from text-fidelity checks.
    let summary = doc::element(
        "summary",
        vec![doc::make_attr("class", "odc-refbar__label")],
    );
    let slot = doc::element(
        "span",
        vec![doc::make_attr("data-odc-ui", ui_kind(&group.kind))],
    );
    doc::append(&summary, slot);
    if !group.caption.is_empty() {
        doc::append(&summary, doc::text_node(&group.caption));
    }
    doc::append(&details, summary);
    let panel = doc::element("div", vec![doc::make_attr("class", "odc-ref-panel")]);
    for (block, leftover) in &group.blocks {
        let (_, source, title) = describe_block(block);
        for spot in elements(block) {
            if has_class(&spot.node, "odc-ref-title") {
                spot.take();
                break;
            }
        }
        let holder = doc::element(
            "div",
            vec![
                doc::make_attr("class", "odc-ref-block"),
                doc::make_attr("data-odc-source", &source),
            ],
        );
        if !title.is_empty() {
            let caption = doc::element("p", vec![doc::make_attr("class", "odc-ref-panel__title")]);
            doc::append(&caption, doc::text_node(&title));
            doc::append(&holder, caption);
        }
        // A second block of the same kind cannot share the one summary label;
        // its caption stays next to the block so no source word is lost.
        if !leftover.is_empty() {
            let caption = doc::element("p", vec![doc::make_attr("class", CAPTION)]);
            doc::append(&caption, doc::text_node(leftover));
            doc::append(&holder, caption);
        }
        for child in doc::take_children(block) {
            doc::append(&holder, child);
        }
        doc::append(&panel, holder);
    }
    doc::append(&details, panel);
    details
}

/// Caption key the client resolves to a localised label (D2).
///
/// The menu is the one caption the source never spells out, so it keeps the
/// shared name; every other panel is named after its kind.
fn ui_kind(kind: &str) -> &str {
    if kind == "toc" {
        "entry-menu"
    } else {
        kind
    }
}

/// Position of a kind in the toolbar.
fn ref_rank(kind: &str) -> usize {
    REF_ORDER
        .iter()
        .position(|known| *known == kind)
        .unwrap_or(REF_ORDER.len())
}
