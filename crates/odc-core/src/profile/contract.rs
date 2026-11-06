//! Source ↔ L1 semantic contract (design decision D1, 2026-11-06).
//!
//! The golden panes freeze what the LDOCE6 adapter produced. A second
//! dictionary has no adapter, so the only ground truth is the source text
//! itself: this module is the ruler that compares an L1 fragment with the raw
//! source fragment it was built from. It answers four independent questions
//! and never repairs anything.
//!
//! 1. `bag` — the two trees yield the same text-token multiset. A token is a
//!    run of alphanumeric characters; CJK text contributes one token per
//!    character, because it is not written with spaces. Punctuation and
//!    whitespace only separate tokens, they are not compared. Case is kept, so
//!    a rule that re-cases a headword is still reported.
//! 2. `order` — inside every semantic block (`odc-sense`, `odc-subsense`,
//!    `odc-examples`, `odc-ref-block`, `odc-pos-block`, `odc-head`) the token
//!    sequence is a subsequence of the source sequence. D1 constrains order
//!    *within* a block only; how blocks are arranged across an entry is P1's
//!    business and is judged by the structure rules, not here.
//! 3. `structure` — the P1 skeleton invariants:
//!    * the fragment has a top-level `section.odc-entry`;
//!    * an entry that carries a position marker (`odc-pos`) outside its
//!      reference bar also carries `section.odc-pos-block[data-odc-pos]` as a
//!      direct child (reference panels quote other entries, so their markers do
//!      not count);
//!    * a sense block is never a sibling of a position block inside one entry:
//!      senses belong inside a position container;
//!    * every child of `nav.odc-refbar` is a reference block (`odc-ref*`).
//! 4. `clean` — no source class name and no source attribute survives the
//!    translation. Every class token starts with `odc-`; every attribute is
//!    `class`, one of [`NEUTRAL_ATTRS`], or `data-odc-*` — which includes
//!    `data-odc-orig-class`, the one place where the engine records a source
//!    class on purpose. Tag names are not judged: a neutral tag such as `h2`
//!    or `li` is allowed as long as its classes are engine-owned, and there is
//!    no closed list of "source" tags to ban.
//!
//! Chrome is not source text: elements carrying `data-odc-ui` (D2) and
//! `script`/`style` elements are left out of every comparison.

use std::collections::BTreeMap;

use markup5ever_rcdom::{Handle, NodeData};

use crate::profile::exec::parse_fragment;
use crate::profile::selector::{attr, class_list, element_name, is_element};

/// Class that marks an entry section.
const ENTRY: &str = "odc-entry";
/// Class that marks a position marker.
const POS: &str = "odc-pos";
/// Class that marks a position container.
const POS_BLOCK: &str = "odc-pos-block";
/// Class that marks the reference bar.
const REFBAR: &str = "odc-refbar";
/// Prefix every reference-bar child carries.
const REF_PREFIX: &str = "odc-ref";
/// Prefix of every engine-owned class.
const ODC_CLASS: &str = "odc-";
/// Prefix of every engine-owned attribute.
const ODC_ATTR: &str = "data-odc-";
/// Marker of chrome text a client injects (D2).
const UI: &str = "data-odc-ui";

/// Blocks whose internal text order D1 constrains.
const SEMANTIC_BLOCKS: [&str; 6] = [
    "odc-sense",
    "odc-subsense",
    "odc-examples",
    "odc-ref-block",
    "odc-pos-block",
    "odc-head",
];

/// Attributes a neutral element may keep from the source.
const NEUTRAL_ATTRS: [&str; 13] = [
    "class", "id", "href", "src", "srcset", "alt", "title", "lang", "dir", "colspan", "rowspan",
    "width", "height",
];

/// How many reasons one record may report per code.
const MAX_PER_CODE: usize = 3;

/// One failed check, with the detail that makes it reproducible.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reason {
    /// Stable machine code (`bag.missing`, `order.block`, ...).
    pub code: &'static str,
    /// Human-readable detail: the token, the block, the attribute.
    pub detail: String,
}

impl Reason {
    /// Build a reason.
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }

    /// `code: detail`, for a one-line report.
    #[must_use]
    pub fn render(&self) -> String {
        format!("{}: {}", self.code, self.detail)
    }
}

/// The four independent verdicts for one record.
#[derive(Clone, Debug, Default)]
pub struct Verdict {
    /// Both trees carry the same text-token multiset.
    pub bag_ok: bool,
    /// Every semantic block keeps the source token order.
    pub order_ok: bool,
    /// The P1 skeleton invariants hold.
    pub structure_ok: bool,
    /// No source class or source attribute survived.
    pub clean_ok: bool,
    /// One entry per failed check, capped at [`MAX_PER_CODE`] per code.
    pub reasons: Vec<Reason>,
}

impl Verdict {
    /// Whether all four checks passed.
    #[must_use]
    pub fn ok(&self) -> bool {
        self.bag_ok && self.order_ok && self.structure_ok && self.clean_ok
    }

    /// The first failure in check order, for a compact report line.
    #[must_use]
    pub fn first_diff(&self) -> Option<String> {
        self.reasons.first().map(Reason::render)
    }
}

/// Compare one L1 fragment with the source fragment it was built from.
#[must_use]
pub fn check(source: &str, l1: &str) -> Verdict {
    let source_dom = parse_fragment(source);
    let l1_dom = parse_fragment(l1);
    let source_tokens = tokens_of(&source_dom.document);
    let l1_tokens = tokens_of(&l1_dom.document);
    let mut verdict = Verdict::default();

    let expected = multiset(&source_tokens);
    let actual = multiset(&l1_tokens);
    verdict.bag_ok = expected == actual;
    if !verdict.bag_ok {
        bag_reasons(&expected, &actual, &mut verdict.reasons);
    }

    let order = order_reasons(&l1_dom.document, &source_tokens);
    verdict.order_ok = order.is_empty();
    verdict.reasons.extend(order);

    let structure = structure_reasons(&l1_dom.document);
    verdict.structure_ok = structure.is_empty();
    verdict.reasons.extend(structure);

    let clean = clean_reasons(&l1_dom.document);
    verdict.clean_ok = clean.is_empty();
    verdict.reasons.extend(clean);

    verdict
}

/// Text tokens of one tree, in document order.
#[must_use]
pub fn tokens_of(root: &Handle) -> Vec<String> {
    let mut out = Vec::new();
    collect_tokens(root, &mut out);
    out
}

fn collect_tokens(node: &Handle, out: &mut Vec<String>) {
    if let NodeData::Text { contents } = &node.data {
        push_tokens(&contents.borrow(), out);
        return;
    }
    if is_element(node) && skipped(node) {
        return;
    }
    let children = node.children.borrow().clone();
    for child in children {
        collect_tokens(&child, out);
    }
}

/// Whether an element's text is chrome rather than source text.
fn skipped(node: &Handle) -> bool {
    if attr(node, UI).is_some() {
        return true;
    }
    matches!(element_name(node).as_deref(), Some("script" | "style"))
}

/// Split one text run into comparison tokens.
///
/// Latin runs stay whole (`don't` is `don` + `t`); every CJK character is its
/// own token, because those scripts are not space-separated.
fn push_tokens(text: &str, out: &mut Vec<String>) {
    let mut word = String::new();
    for ch in text.chars() {
        if is_cjk(ch) {
            flush(&mut word, out);
            out.push(ch.to_string());
        } else if ch.is_alphanumeric() {
            word.push(ch);
        } else {
            flush(&mut word, out);
        }
    }
    flush(&mut word, out);
}

fn flush(word: &mut String, out: &mut Vec<String>) {
    if !word.is_empty() {
        out.push(std::mem::take(word));
    }
}

/// Whether a character belongs to a script written without spaces.
fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x3040..=0x30FF      // hiragana, katakana
        | 0x3400..=0x4DBF    // CJK extension A
        | 0x4E00..=0x9FFF    // CJK unified ideographs
        | 0xAC00..=0xD7AF    // hangul syllables
        | 0xF900..=0xFAFF    // CJK compatibility ideographs
    )
}

fn multiset(tokens: &[String]) -> BTreeMap<&str, usize> {
    let mut out = BTreeMap::new();
    for token in tokens {
        *out.entry(token.as_str()).or_insert(0) += 1;
    }
    out
}

/// Word-level differences, missing words first.
fn bag_reasons(
    expected: &BTreeMap<&str, usize>,
    actual: &BTreeMap<&str, usize>,
    out: &mut Vec<Reason>,
) {
    let mut missing = 0usize;
    for (token, count) in expected {
        let got = actual.get(token).copied().unwrap_or(0);
        if got >= *count {
            continue;
        }
        missing += 1;
        if missing <= MAX_PER_CODE {
            out.push(Reason::new(
                "bag.missing",
                format!("{token:?} x{} absent from L1", count - got),
            ));
        }
    }
    let mut extra = 0usize;
    for (token, count) in actual {
        let want = expected.get(token).copied().unwrap_or(0);
        if want >= *count {
            continue;
        }
        extra += 1;
        if extra <= MAX_PER_CODE {
            out.push(Reason::new(
                "bag.extra",
                format!("{token:?} x{} not in source", count - want),
            ));
        }
    }
}

/// Check the internal order of every semantic block.
fn order_reasons(root: &Handle, source: &[String]) -> Vec<Reason> {
    let mut out = Vec::new();
    let mut blocks: Vec<Handle> = Vec::new();
    collect_blocks(root, &mut blocks);
    for block in blocks {
        let tokens = tokens_of(&block);
        if tokens.is_empty() {
            continue;
        }
        if let Some((index, token)) = gap(&tokens, source) {
            let class = class_list(&block).join(".");
            out.push(Reason::new(
                "order.block",
                format!(".{class}: token {token:?} (#{index}) is out of source order"),
            ));
            if out.len() >= MAX_PER_CODE {
                break;
            }
        }
    }
    out
}

fn collect_blocks(node: &Handle, out: &mut Vec<Handle>) {
    if is_element(node)
        && class_list(node)
            .iter()
            .any(|class| SEMANTIC_BLOCKS.contains(&class.as_str()))
    {
        out.push(node.clone());
    }
    let children = node.children.borrow().clone();
    for child in children {
        collect_blocks(&child, out);
    }
}

/// First token of `block` that cannot be matched in `full` in order.
fn gap(block: &[String], full: &[String]) -> Option<(usize, String)> {
    let mut at = 0usize;
    for (index, token) in block.iter().enumerate() {
        let found = full
            .get(at..)
            .and_then(|rest| rest.iter().position(|item| item == token));
        match found {
            Some(offset) => at += offset + 1,
            None => return Some((index, token.clone())),
        }
    }
    None
}

/// Check the P1 skeleton invariants.
fn structure_reasons(root: &Handle) -> Vec<Reason> {
    let mut out = Vec::new();
    let mut entries: Vec<Handle> = Vec::new();
    for child in top_level(root) {
        if tag_is(&child, "section") && has_class(&child, ENTRY) {
            entries.push(child);
        }
    }
    if entries.is_empty() {
        out.push(Reason::new(
            "structure.entry",
            "no section.odc-entry at the top level",
        ));
        return out;
    }

    let mut nested: Vec<Handle> = Vec::new();
    for entry in &entries {
        collect_entries(entry, &mut nested);
    }
    entries.extend(nested);

    for entry in entries {
        let children = element_children(&entry);
        let has_pos_block = children.iter().any(|child| {
            tag_is(child, "section")
                && has_class(child, POS_BLOCK)
                && attr(child, "data-odc-pos").is_some_and(|value| !value.trim().is_empty())
        });
        if has_pos_marker(&entry) && !has_pos_block {
            out.push(Reason::new(
                "structure.pos_block",
                "position marker without section.odc-pos-block[data-odc-pos]",
            ));
        }
        if has_pos_block {
            if let Some(sense) = children
                .iter()
                .find(|child| has_class(child, "odc-sense") || has_class(child, "odc-subsense"))
            {
                let class = class_list(sense).join(".");
                out.push(Reason::new(
                    "structure.sense_sibling",
                    format!(".{class} is a sibling of a position block"),
                ));
            }
        }
    }

    for bar in refbars(root) {
        if let Some(child) = bar.children.borrow().iter().find(|child| {
            is_element(child)
                && !class_list(child)
                    .iter()
                    .any(|class| class.starts_with(REF_PREFIX))
        }) {
            out.push(Reason::new(
                "structure.refbar",
                format!(
                    "nav.odc-refbar holds a non-reference child <{} class=\"{}\">",
                    element_name(child).unwrap_or_default(),
                    class_list(child).join(" ")
                ),
            ));
        }
    }
    out
}

/// Children of a fragment root, with the `html`/`head`/`body` wrappers the
/// HTML parser adds peeled off. A fragment parser may park the markup in a
/// wrapper (or split it between a head and a body), so "top level" is the first
/// generation of nodes that is not one of those wrappers.
fn top_level(node: &Handle) -> Vec<Handle> {
    fn walk(node: &Handle, out: &mut Vec<Handle>) {
        for child in node.children.borrow().iter() {
            if !is_element(child) {
                continue;
            }
            match element_name(child).as_deref() {
                Some("html" | "head" | "body") => walk(child, out),
                _ => out.push(child.clone()),
            }
        }
    }
    let mut out = Vec::new();
    walk(node, &mut out);
    out
}

fn element_children(node: &Handle) -> Vec<Handle> {
    node.children
        .borrow()
        .iter()
        .filter(|child| is_element(child))
        .cloned()
        .collect()
}

fn collect_entries(node: &Handle, out: &mut Vec<Handle>) {
    for child in node.children.borrow().iter() {
        if !is_element(child) {
            continue;
        }
        if tag_is(child, "section") && has_class(child, ENTRY) {
            out.push(child.clone());
            continue;
        }
        collect_entries(child, out);
    }
}

/// Whether an entry carries a position marker outside its reference bar.
///
/// Reference panels quote other entries and a nested entry is a different
/// document, so neither contributes a marker to the entry under test.
fn has_pos_marker(node: &Handle) -> bool {
    for child in node.children.borrow().iter() {
        if !is_element(child) || skipped(child) {
            continue;
        }
        if has_class(child, REFBAR) || (tag_is(child, "section") && has_class(child, ENTRY)) {
            continue;
        }
        if has_class(child, POS) || has_pos_marker(child) {
            return true;
        }
    }
    false
}

/// Every `nav.odc-refbar` in the tree.
fn refbars(root: &Handle) -> Vec<Handle> {
    let mut all = Vec::new();
    collect_elements(root, &mut all);
    all.into_iter()
        .filter(|node| tag_is(node, "nav") && has_class(node, REFBAR))
        .collect()
}

fn collect_elements(node: &Handle, out: &mut Vec<Handle>) {
    for child in node.children.borrow().iter() {
        if !is_element(child) {
            continue;
        }
        out.push(child.clone());
        collect_elements(child, out);
    }
}

/// Check that no source class name and no source attribute survived.
fn clean_reasons(root: &Handle) -> Vec<Reason> {
    let mut out = Vec::new();
    let mut elements = Vec::new();
    collect_elements(root, &mut elements);
    let mut classes = 0usize;
    let mut attrs = 0usize;
    for element in elements {
        let tag = element_name(&element).unwrap_or_default();
        for class in class_list(&element) {
            if class.starts_with(ODC_CLASS) {
                continue;
            }
            classes += 1;
            if classes <= MAX_PER_CODE {
                out.push(Reason::new(
                    "clean.class",
                    format!("source class {class:?} on <{tag}>"),
                ));
            }
        }
        let NodeData::Element { attrs: names, .. } = &element.data else {
            continue;
        };
        for name in names.borrow().iter() {
            let name = name.name.local.to_string();
            if name.starts_with(ODC_ATTR) || NEUTRAL_ATTRS.contains(&name.as_str()) {
                continue;
            }
            attrs += 1;
            if attrs <= MAX_PER_CODE {
                out.push(Reason::new(
                    "clean.attr",
                    format!("source attribute {name:?} on <{tag}>"),
                ));
            }
        }
    }
    out
}

fn tag_is(node: &Handle, name: &str) -> bool {
    element_name(node).as_deref() == Some(name)
}

fn has_class(node: &Handle, class: &str) -> bool {
    class_list(node).iter().any(|item| item == class)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verdict(source: &str, l1: &str) -> Verdict {
        check(source, l1)
    }

    #[test]
    fn identical_text_passes_the_bag() {
        let source = "<p>a <b>small</b> test</p>";
        let l1 = r#"<section class="odc-entry"><p>a <b>small</b> test</p></section>"#;
        let out = verdict(source, l1);
        assert!(out.bag_ok, "{:?}", out.reasons);
    }

    #[test]
    fn a_reordered_bag_is_still_the_same_multiset() {
        let source = "<p>one two three</p>";
        let l1 =
            r#"<section class="odc-entry"><div class="odc-sense">three one two</div></section>"#;
        let out = verdict(source, l1);
        assert!(out.bag_ok, "{:?}", out.reasons);
        assert!(
            !out.order_ok,
            "order must catch the swap: {:?}",
            out.reasons
        );
        assert!(out.reasons.iter().any(|item| item.code == "order.block"));
    }

    #[test]
    fn a_missing_word_is_reported_against_the_source() {
        let source = "<p>alpha beta gamma</p>";
        let l1 = r#"<section class="odc-entry"><p>alpha beta</p></section>"#;
        let out = verdict(source, l1);
        assert!(!out.bag_ok);
        assert!(out
            .reasons
            .iter()
            .any(|item| item.code == "bag.missing" && item.detail.contains("gamma")));
    }

    #[test]
    fn an_invented_word_is_reported_against_the_source() {
        let source = "<p>alpha beta</p>";
        let l1 = r#"<section class="odc-entry"><p>alpha beta delta</p></section>"#;
        let out = verdict(source, l1);
        assert!(!out.bag_ok);
        assert!(out
            .reasons
            .iter()
            .any(|item| item.code == "bag.extra" && item.detail.contains("delta")));
    }

    #[test]
    fn chinese_text_is_compared_per_character() {
        let source = "<p>中文词条</p>";
        let same = r#"<section class="odc-entry"><p>中文词条</p></section>"#;
        assert!(verdict(source, same).bag_ok);
        let other = r#"<section class="odc-entry"><p>中文词</p></section>"#;
        let out = verdict(source, other);
        assert!(!out.bag_ok);
        assert!(out
            .reasons
            .iter()
            .any(|item| item.code == "bag.missing" && item.detail.contains("条")));
    }

    #[test]
    fn a_block_keeps_its_internal_order_when_moved() {
        // The reference block moved to the front of the entry: D1 allows that,
        // because only the order inside a block is judged.
        let source = r#"<div class="sense"><h2>verb</h2><p>move along</p></div>
            <div class="origin"><p>old english</p></div>"#;
        let l1 = r#"<section class="odc-entry">
            <nav class="odc-refbar"><div class="odc-refbar__item"><div class="odc-ref-block"><p>old english</p></div></div></nav>
            <section class="odc-pos-block" data-odc-pos="verb"><div class="odc-sense"><h2 class="odc-pos">verb</h2><p>move along</p></div></section>
        </section>"#;
        let out = verdict(source, l1);
        assert!(out.order_ok, "{:?}", out.reasons);
    }

    #[test]
    fn a_block_with_swapped_text_fails_the_order_check() {
        let source = "<p>first second</p>";
        let l1 = r#"<section class="odc-entry"><div class="odc-examples"><p>second first</p></div></section>"#;
        let out = verdict(source, l1);
        assert!(!out.order_ok);
        assert!(out.reasons.iter().any(|item| item.code == "order.block"));
    }

    #[test]
    fn a_missing_entry_is_a_structure_failure() {
        let source = "<p>a b</p>";
        let l1 = "<div><p>a b</p></div>";
        let out = verdict(source, l1);
        assert!(!out.structure_ok);
        assert!(out
            .reasons
            .iter()
            .any(|item| item.code == "structure.entry"));
    }

    #[test]
    fn a_position_marker_needs_a_position_block() {
        let source = "<p>verb</p>";
        let l1 = r#"<section class="odc-entry"><h2 class="odc-pos">verb</h2></section>"#;
        let out = verdict(source, l1);
        assert!(!out.structure_ok);
        assert!(out
            .reasons
            .iter()
            .any(|item| item.code == "structure.pos_block"));
    }

    #[test]
    fn a_position_marker_inside_the_reference_bar_is_not_an_entry_marker() {
        let source = "<p>verb see noun</p>";
        let l1 = r#"<section class="odc-entry">
            <nav class="odc-refbar"><div class="odc-refbar__item"><div class="odc-ref-block">
                <h2 class="odc-pos">verb</h2><p>see noun</p>
            </div></div></nav>
        </section>"#;
        let out = verdict(source, l1);
        assert!(out.structure_ok, "{:?}", out.reasons);
    }

    #[test]
    fn a_sense_may_not_sit_beside_a_position_block() {
        let source = "<p>verb move</p>";
        let l1 = r#"<section class="odc-entry">
            <section class="odc-pos-block" data-odc-pos="verb"><p>verb</p></section>
            <div class="odc-sense">move</div>
        </section>"#;
        let out = verdict(source, l1);
        assert!(!out.structure_ok);
        assert!(out
            .reasons
            .iter()
            .any(|item| item.code == "structure.sense_sibling"));
    }

    #[test]
    fn the_reference_bar_holds_reference_blocks_only() {
        let source = "<p>a b</p>";
        let l1 = r#"<section class="odc-entry">
            <nav class="odc-refbar"><div class="odc-sense">a b</div></nav>
        </section>"#;
        let out = verdict(source, l1);
        assert!(!out.structure_ok);
        assert!(out
            .reasons
            .iter()
            .any(|item| item.code == "structure.refbar"));
    }

    #[test]
    fn a_source_class_name_is_reported() {
        let source = "<p>a b</p>";
        let l1 = r#"<section class="odc-entry"><div class="h-g"><p>a b</p></div></section>"#;
        let out = verdict(source, l1);
        assert!(!out.clean_ok);
        assert!(out
            .reasons
            .iter()
            .any(|item| item.code == "clean.class" && item.detail.contains("h-g")));
    }

    #[test]
    fn a_source_attribute_is_reported_but_orig_class_is_allowed() {
        let source = "<p>a b</p>";
        let l1 = r#"<section class="odc-entry"><span geo="br" data-odc-orig-class="neutral">a b</span></section>"#;
        let out = verdict(source, l1);
        assert!(!out.clean_ok);
        assert!(out.reasons.iter().any(|item| item.code == "clean.attr"));
        assert!(!out
            .reasons
            .iter()
            .any(|item| item.detail.contains("orig-class")));
    }

    #[test]
    fn chrome_text_marked_as_ui_does_not_count() {
        let source = "<p>a b</p>";
        let l1 = r#"<section class="odc-entry"><p>a b</p><summary data-odc-ui="entry-menu">Entry menu</summary></section>"#;
        let out = verdict(source, l1);
        assert!(out.bag_ok, "{:?}", out.reasons);
    }
}
