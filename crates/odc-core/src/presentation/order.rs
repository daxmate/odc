//! P1: the document skeleton and the order of an entry's blocks.
//!
//! The rebuild is data-driven: a *content block* is an element the rules marked
//! with the `odc-*` vocabulary, a *position* is the `.odc-pos` slot or a
//! `data-odc-pos` mark. No source tag name decides anything, so one pass serves
//! both shapes: a source that already ships position containers (LDOCE6) and a
//! source whose senses are flat siblings parked inside generic wrappers
//! (OALD9's long `ol`).
//!
//! Two rules keep the result explainable:
//!
//! - only top-level entries are rebuilt; an `odc-entry` nested in another one
//!   is broken input and stays where it is, untouched;
//! - a run of consecutive siblings that share one position becomes one block.
//!   Runs are never merged across a different position, so the text order is
//!   exactly the source order (no reordering to collect "verb" together).
//!
//! Content without any position mark stays a plain block at entry level: P1
//! only lists position blocks that carry a `data-odc-pos`, and inventing one
//! would claim knowledge the rules did not provide.

use markup5ever_rcdom::{Handle, NodeData};

use crate::doc;
use crate::presentation::{anchors, attr, elements, has_class, tag};

const ENTRY: &str = "odc-entry";
const HEAD: &str = "odc-head";
const REFBAR: &str = "odc-refbar";
const POS: &str = "odc-pos";
const POS_BLOCK: &str = "odc-pos-block";

/// Rebuild the skeleton: entries at the top level, blocks in spec order.
pub fn arrange(root: &Handle) {
    lift_sections(root);
    number_homographs(root);
    let mut roots: Vec<Handle> = Vec::new();
    entry_roots(root, false, &mut roots);
    for entry in roots {
        rebuild(&entry);
    }
    lift_sections(root);
}

/// Top-level entries only: an entry nested in another one is broken input and
/// is rebuilt by nobody (there is no way to tell which order it belongs to).
fn entry_roots(node: &Handle, inside: bool, out: &mut Vec<Handle>) {
    for child in node.children.borrow().iter() {
        if !crate::profile::selector::is_element(child) {
            continue;
        }
        let entry = tag(child) == "section" && has_class(child, ENTRY);
        if entry && !inside {
            out.push(child.clone());
        }
        entry_roots(child, inside || entry, out);
    }
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

/// A rebuilt entry body: ready blocks plus children still to be wrapped.
enum Block {
    /// A block that already carries its own skeleton.
    Ready(Handle),
    /// A run of children that share one position.
    Group(String, Vec<Handle>),
    /// A source text node: kept so inline runs do not collide into one word.
    Text(Handle),
}

/// Rebuild one entry: chrome first, then one block per position run.
fn rebuild(entry: &Handle) {
    let mut lead: Vec<Handle> = Vec::new();
    let mut head: Option<Handle> = None;
    let mut bar: Option<Handle> = None;
    let mut blocks: Vec<Block> = Vec::new();
    let mut open: Option<(String, Vec<Handle>)> = None;
    let mut flat: Vec<Handle> = Vec::new();
    flatten(&doc::take_children(entry), &mut flat);
    for node in flat {
        if !crate::profile::selector::is_element(&node) {
            match open.as_mut() {
                Some((_, kids)) => kids.push(node),
                None => blocks.push(Block::Text(node)),
            }
            continue;
        }
        if tag(&node) == "header" && has_class(&node, HEAD) {
            if head.is_none() {
                head = Some(node);
            }
            continue;
        }
        if tag(&node) == "nav" && has_class(&node, REFBAR) {
            if bar.is_none() {
                bar = Some(node);
            }
            continue;
        }
        if has_class(&node, ENTRY) {
            // A nested entry is broken input: keep it out of the runs.
            close(&mut blocks, &mut open);
            blocks.push(Block::Ready(node));
            continue;
        }
        if is_lead(&node) {
            lead.push(node);
            continue;
        }
        if has_class(&node, POS_BLOCK) {
            close(&mut blocks, &mut open);
            stamp_pos(&node);
            blocks.push(Block::Ready(node));
            continue;
        }
        match block_pos(&node) {
            Some(pos) => {
                let same = matches!(&open, Some((current, _)) if *current == pos);
                if !same {
                    close(&mut blocks, &mut open);
                }
                if open.is_none() {
                    open = Some((pos, Vec::new()));
                }
                if let Some((_, kids)) = open.as_mut() {
                    kids.push(node);
                }
            }
            None => match open.as_mut() {
                // A marker opens its run: the blocks that follow belong to it
                // until another position or an already-finished block.
                Some((_, kids)) => kids.push(node),
                None => blocks.push(Block::Ready(node)),
            },
        }
    }
    close(&mut blocks, &mut open);

    let mut ordered = lead;
    if let Some(head) = head {
        ordered.push(head);
    }
    if let Some(bar) = bar {
        ordered.push(bar);
    }
    for block in blocks {
        ordered.push(match block {
            Block::Ready(node) => node,
            Block::Group(pos, kids) => wrap(&pos, kids),
            Block::Text(node) => node,
        });
    }
    *entry.children.borrow_mut() = ordered;
}

/// Close the open run, if any, into a finished block.
fn close(blocks: &mut Vec<Block>, open: &mut Option<(String, Vec<Handle>)>) {
    if let Some((pos, kids)) = open.take() {
        blocks.push(Block::Group(pos, kids));
    }
}

/// Wrap a run of children into its position block.
fn wrap(pos: &str, kids: Vec<Handle>) -> Handle {
    let section = doc::element(
        "section",
        vec![
            doc::make_attr("class", POS_BLOCK),
            doc::make_attr("data-odc-pos", pos),
        ],
    );
    *section.children.borrow_mut() = kids;
    section
}

/// Give an existing position block its `data-odc-pos` (P1 requires it, and the
/// label is a slot, so the engine can read it back without guessing).
fn stamp_pos(block: &Handle) {
    let pos = attr(block, "data-odc-pos")
        .and_then(|raw| pos_value(&raw))
        .or_else(|| inner_pos(block));
    if let Some(pos) = pos {
        doc::set_attr(block, "data-odc-pos", &pos);
    }
}

/// Position carried by a content block itself, never by its descendants.
fn block_pos(node: &Handle) -> Option<String> {
    attr(node, "data-odc-pos")
        .and_then(|raw| pos_value(&raw))
        .or_else(|| {
            if has_class(node, POS) {
                pos_value(&crate::profile::selector::text_content(node))
            } else {
                None
            }
        })
}

/// First `.odc-pos` label under a container.
fn inner_pos(node: &Handle) -> Option<String> {
    for spot in elements(node) {
        if has_class(&spot.node, POS) {
            if let Some(pos) = pos_value(&crate::profile::selector::text_content(&spot.node)) {
                return Some(pos);
            }
        }
    }
    None
}

/// `data-odc-pos` value: the URL-safe slug of the position label.
fn pos_value(raw: &str) -> Option<String> {
    let slug = anchors::entry_slug(raw.trim());
    if slug.is_empty() {
        None
    } else {
        Some(slug)
    }
}

/// Lift content blocks out of generic wrappers, keeping document order.
///
/// A wrapper is anything the rules did not mark: no `odc-*` class, no
/// `data-odc-*` attribute, no `id` and no text of its own. Text nodes are
/// carried along so two inline blocks never collide into one word.
fn flatten(children: &[Handle], out: &mut Vec<Handle>) {
    for child in children {
        if crate::profile::selector::is_element(child) && !is_block(child) {
            // Detach before descending: rcdom's Drop wipes the subtree a node
            // still owns, so a discarded wrapper would otherwise empty the
            // blocks another branch already lifted out of it.
            let inner = doc::take_children(child);
            flatten(&inner, out);
        } else {
            out.push(child.clone());
        }
    }
}

/// Whether the rules marked this element as a block, or it carries its own text.
fn is_block(node: &Handle) -> bool {
    if crate::presentation::classes(node)
        .iter()
        .any(|class| class.starts_with("odc-"))
    {
        return true;
    }
    if has_marker(node) {
        return true;
    }
    node.children
        .borrow()
        .iter()
        .any(|child| match &child.data {
            NodeData::Text { contents } => !contents.borrow().trim().is_empty(),
            _ => false,
        })
}

/// `id` keeps a source anchor addressable; `data-odc-*` is slot data.
fn has_marker(node: &Handle) -> bool {
    if attr(node, "id").is_some() {
        return true;
    }
    let NodeData::Element { attrs, .. } = &node.data else {
        return false;
    };
    attrs
        .borrow()
        .iter()
        .any(|item| item.name.local.as_ref().starts_with("data-odc-"))
}

/// Whether the node opens the entry (the homograph bar).
fn is_lead(node: &Handle) -> bool {
    crate::presentation::classes(node)
        .iter()
        .any(|class| class == "chwd")
        || attr(node, "data-odc-orig-class").as_deref() == Some("chwd")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::exec::{parse_fragment, serialize};

    fn rebuild_once(html: &str) -> String {
        let dom = parse_fragment(html);
        arrange(&dom.document);
        serialize(&dom.document)
    }

    /// A rule-processed document in the OALD9 shape: one entry, no position
    /// containers, all senses parked inside one long `ol`, the position as a
    /// sibling `odc-pos` slot.
    const FLAT: &str = r##"<section class="odc-entry" data-odc-key="go">
        <div class="top"><a href="#v">verb</a> | <a href="#n">noun</a></div>
        <header class="odc-head"><h1 class="odc-hw">go</h1></header>
        <ol class="h-g">
            <h2 class="odc-pos">verb</h2>
            <div class="odc-sense">move</div>
            <div class="odc-sense">leave</div>
            <h2 class="odc-pos">noun</h2>
            <div class="odc-sense">attempt</div>
        </ol>
    </section>"##;

    #[test]
    fn flat_senses_become_position_blocks_in_document_order() {
        let out = rebuild_once(FLAT);
        assert_eq!(out.matches("odc-pos-block").count(), 2, "{out}");
        let verb = out.find(r#"data-odc-pos="verb""#).expect("verb block");
        let noun = out.find(r#"data-odc-pos="noun""#).expect("noun block");
        assert!(verb < noun, "positions keep source order: {out}");
        // Both verb senses live in the verb block, not in the noun one.
        let verb_block = &out[verb..noun];
        assert_eq!(verb_block.matches("odc-sense").count(), 2, "{out}");
        let noun_block = &out[noun..];
        assert_eq!(noun_block.matches("odc-sense").count(), 1, "{out}");
        // The header is hoisted and the wrapper list is gone.
        let head = out.find("odc-head").expect("header");
        assert!(head < verb, "header comes before the blocks: {out}");
    }

    #[test]
    fn a_second_pass_changes_nothing() {
        let once = rebuild_once(FLAT);
        let twice = rebuild_once(&once);
        assert_eq!(once, twice);
    }

    /// LDOCE6 shape: the rules already cut position containers, so the pass
    /// only stamps the position it can read from the slot.
    #[test]
    fn existing_position_containers_are_kept_and_stamped() {
        let html = r#"<section class="odc-entry" data-odc-key="make">
            <header class="odc-head"><h1 class="odc-hw">make</h1></header>
            <section class="odc-pos-block"><h2 class="odc-pos">verb</h2>
                <div class="odc-sense">produce</div>
            </section>
            <section class="odc-pos-block"><h2 class="odc-pos">noun</h2>
                <div class="odc-sense">brand</div>
            </section>
        </section>"#;
        let out = rebuild_once(html);
        assert_eq!(out.matches("odc-pos-block").count(), 2, "{out}");
        assert!(out.contains(r#"data-odc-pos="verb""#), "{out}");
        assert!(out.contains(r#"data-odc-pos="noun""#), "{out}");
        assert_eq!(rebuild_once(&out), out);
    }

    /// Content with no position mark stays a plain block: the engine never
    /// invents a position the rules did not provide.
    #[test]
    fn content_without_a_position_is_not_wrapped() {
        let html = r#"<section class="odc-entry" data-odc-key="saks">
            <header class="odc-head"><h1 class="odc-hw">Saks</h1></header>
            <div class="odc-sense">a store</div>
        </section>"#;
        let out = rebuild_once(html);
        assert!(!out.contains("odc-pos-block"), "{out}");
        assert!(
            out.contains(r#"<div class="odc-sense">a store</div>"#),
            "{out}"
        );
    }

    /// Inline blocks separated only by source text keep their separator, so two
    /// words never collide into one token.
    #[test]
    fn inline_separators_survive_the_lift() {
        let html = r#"<section class="odc-entry" data-odc-key="x">
            <header class="odc-head"><h1 class="odc-hw">x</h1></header>
            <h2 class="odc-pos">verb</h2>
            <div><span class="odc-def">one</span> <span class="odc-def">two</span></div>
        </section>"#;
        let out = rebuild_once(html);
        assert!(out.contains("one</span> <span"), "{out}");
    }
}
