//! P2/P3/P4: header, reference bar and entry menu.
//!
//! The bar is assembled from the blocks the profile marked as
//! `div.odc-ref-block` and classified through `span.odc-ref-title`. Grouping,
//! order, the `details`/summary chrome and the UI captions are engine
//! decisions: no rule pack can change them.

use markup5ever_rcdom::Handle;

use crate::adapters::ParseContext;
use crate::doc;
use crate::presentation::{attr, elements, has_class, tag};

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
    blocks: Vec<Handle>,
}

fn build_bar(entry: &Handle) {
    // Detach from the end: removing a node only shifts the indices of the
    // siblings that follow it.
    let mut spots: Vec<_> = elements(entry)
        .into_iter()
        .filter(|spot| has_class(&spot.node, "odc-ref-block"))
        .collect();
    spots.reverse();
    let mut blocks: Vec<Handle> = spots
        .into_iter()
        .map(crate::presentation::Spot::take)
        .collect();
    blocks.reverse();
    if blocks.is_empty() {
        return;
    }
    let mut groups: Vec<Group> = Vec::new();
    for block in blocks {
        let kind = describe_block(&block).0;
        match groups.iter_mut().find(|group| group.kind == kind) {
            Some(group) => group.blocks.push(block),
            None => groups.push(Group {
                kind,
                blocks: vec![block],
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
    // P4/D2: the caption is chrome, not entry text. L1 carries the kind the
    // client localises; the engine never writes an English label.
    let summary = doc::element(
        "summary",
        vec![
            doc::make_attr("class", "odc-refbar__label"),
            doc::make_attr("data-odc-ui", ui_kind(&group.kind)),
        ],
    );
    doc::append(&details, summary);
    let panel = doc::element("div", vec![doc::make_attr("class", "odc-ref-panel")]);
    for block in &group.blocks {
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
