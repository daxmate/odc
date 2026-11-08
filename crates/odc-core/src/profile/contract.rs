//! Source ↔ L1 semantic contract (design decision D1, 2026-11-06).
//!
//! The golden panes freeze what the LDOCE6 adapter produced. A second
//! dictionary has no adapter, so the only ground truth is the source text
//! itself: this module is the ruler that compares an L1 fragment with the raw
//! source fragment it was built from. It answers four independent questions
//! and never repairs anything.
//!
//! 1. `bag` — the two trees yield the same text-token multiset. A token is a
//!    run of alphanumeric characters cut out of the *rendered* text: inline
//!    boundaries (`span`, `b`, `i`, ...) are transparent, so
//!    `sandwich<span>es</span>` and `sandwiches` both yield `sandwiches`, while
//!    block boundaries still separate. CJK text contributes one token per
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
//!    * a sense block is never a sibling *after* a position block inside one
//!      entry: senses belong inside a position container. A sense that leads the
//!      entry *before* the first position block is allowed (some dictionaries
//!      print a run-in sense ahead of the part-of-speech container);
//!    * every child of `nav.odc-refbar` is a reference block (`odc-ref*`).
//! 4. `clean` — no source class name and no source attribute survives the
//!    translation. Every class token starts with `odc-`; every attribute is
//!    `class`, one of `NEUTRAL_ATTRS`, or `data-odc-*` — which includes
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
/// Class of the raw fallback entry (spec section 6).
const RAW_ENTRY: &str = "odc-entry--raw";
/// Flag that marks a fragment the profile could not adapt.
const RAW_FLAG: &str = "data-odc-unadapted";

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
    /// The clean check was skipped for a raw fallback entry (spec section 6):
    /// that entry keeps the source markup on purpose, so reporting its source
    /// classes and attributes as leakage would be noise. Counted, never
    /// silent.
    pub clean_exempt_raw: bool,
    /// The L1 fragment is a raw fallback: the profile did not adapt this
    /// record at all (spec section 6). Such a record passes the four checks by
    /// construction — source compared with itself — so it must stay out of the
    /// pass rates and be reported through the coverage line instead.
    pub unadapted: bool,
    /// One entry per failed check, capped at `MAX_PER_CODE` per code.
    pub reasons: Vec<Reason>,
}

impl Verdict {
    /// Whether all four checks passed.
    ///
    /// This says nothing about whether the record was adapted: a raw fallback
    /// passes by construction. Use [`Verdict::adapted_ok`] when the question
    /// is "did the profile both adapt and keep this record intact?".
    #[must_use]
    pub fn ok(&self) -> bool {
        self.bag_ok && self.order_ok && self.structure_ok && self.clean_ok
    }

    /// Whether the record was adapted **and** passed all four checks.
    #[must_use]
    pub fn adapted_ok(&self) -> bool {
        !self.unadapted && self.ok()
    }

    /// The first failure in check order, for a compact report line.
    #[must_use]
    pub fn first_diff(&self) -> Option<String> {
        self.reasons.first().map(Reason::render)
    }
}

/// Aggregate of one comparison run: how much was really adapted, and how the
/// adapted slice scored.
///
/// The four per-check counts only cover **adapted** records. An unadapted
/// record is the source markup wrapped in `section.odc-entry--raw`, so
/// comparing it with its source is comparing a tree with itself: it passes
/// every check by construction. Counting those passes as wins is what made a
/// 99.99% unadapted run read as a full pass. The summary keeps the two
/// questions apart — [`ContractSummary::coverage`] says how much was adapted,
/// the four counts say how well the adapted part held up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContractSummary {
    /// Records handed to the ruler (aliases are not records).
    pub total: usize,
    /// Records the profile adapted.
    pub adapted: usize,
    /// Records the profile could not adapt (raw fallback, spec section 6).
    pub unadapted: usize,
    /// Adapted records whose text bag matched.
    pub bag_pass: usize,
    /// Adapted records whose block order matched.
    pub order_pass: usize,
    /// Adapted records whose skeleton invariants held.
    pub structure_pass: usize,
    /// Adapted records with no source class or attribute left.
    pub clean_pass: usize,
    /// Adapted records that passed all four checks.
    pub adapted_all_pass: usize,
}

impl ContractSummary {
    /// Fold one record's verdict into the run.
    pub fn record(&mut self, verdict: &Verdict) {
        self.total += 1;
        if verdict.unadapted {
            self.unadapted += 1;
            return;
        }
        self.adapted += 1;
        self.bag_pass += usize::from(verdict.bag_ok);
        self.order_pass += usize::from(verdict.order_ok);
        self.structure_pass += usize::from(verdict.structure_ok);
        self.clean_pass += usize::from(verdict.clean_ok);
        self.adapted_all_pass += usize::from(verdict.adapted_ok());
    }

    /// Share of records the profile really adapted, inside `[0, 1]`.
    ///
    /// An empty run has no coverage: this returns `0.0`, so a strict gate
    /// fails instead of passing on nothing.
    #[must_use]
    pub fn coverage(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        self.adapted as f64 / self.total as f64
    }

    /// One machine-readable line, shared by the example and the tests.
    ///
    /// `coverage <adapted>/<total> (<pct>%) | mapped <pass>/<adapted>
    /// four-dim | unadapted <unadapted>`
    #[must_use]
    pub fn render_line(&self) -> String {
        format!(
            "coverage {}/{} ({:.3}%) | mapped {}/{} four-dim | unadapted {}",
            self.adapted,
            self.total,
            self.coverage() * 100.0,
            self.adapted_all_pass,
            self.adapted,
            self.unadapted
        )
    }

    /// Why the strict gate rejects this run, or `None` when it passes.
    ///
    /// Strict means: every record adapted, and every adapted record passed
    /// the four checks. It is the only mode in which the four pass rates are
    /// allowed to stand in for "the profile works on this dictionary".
    #[must_use]
    pub fn strict_violation(&self) -> Option<String> {
        if self.total == 0 {
            return Some(String::from("no records compared"));
        }
        if self.unadapted > 0 {
            return Some(format!("{} unadapted record(s)", self.unadapted));
        }
        if self.adapted < self.total {
            return Some(format!(
                "coverage {}/{} below 100%",
                self.adapted, self.total
            ));
        }
        if self.adapted_all_pass < self.adapted {
            return Some(format!(
                "{} adapted record(s) failed the four-dim check",
                self.adapted - self.adapted_all_pass
            ));
        }
        None
    }
}

/// Compare one L1 fragment with the source fragment it was built from.
///
/// The four checks run under two readings of the same text. Source and L1 are
/// different trees: an element the source writes as an inline `span` can be a
/// block in the L1 (`p.odc-ref-panel__title`), and an element the engine
/// unwraps disappears from the L1 entirely. Segmentation is therefore not
/// comparable across the two trees, so a record passes when *either* reading
/// keeps the text intact:
///
/// * `Reading::Run` — a word may continue across inline boundaries
///   (`sandwich<span>es</span>` reads as `sandwiches`); block boundaries cut;
/// * `Reading::Node` — every text node is its own unit, which is how a
///   fragment looks before serialisation joins two mapped text nodes into one.
///
/// Both readings still cut at block boundaries and at whitespace, so a word
/// dropped, invented or glued across a paragraph fails under both. A record
/// is only reported under the strict reading.
#[must_use]
pub fn check(source: &str, l1: &str) -> Verdict {
    let source_dom = parse_fragment(source);
    let l1_dom = parse_fragment(l1);
    let strict = judge(&source_dom.document, &l1_dom.document, Reading::Run);
    if strict.ok() {
        return strict;
    }
    let relaxed = judge(&source_dom.document, &l1_dom.document, Reading::Node);
    if relaxed.ok() {
        return relaxed;
    }
    strict
}

/// How a tree's text is cut into comparison tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reading {
    /// A word runs across inline boundaries, block boundaries cut.
    Run,
    /// Every text node is its own unit.
    Node,
}

impl Reading {
    fn tokens(self, root: &Handle) -> Vec<String> {
        match self {
            Reading::Run => tokens_of(root),
            Reading::Node => tokens_by_node(root),
        }
    }
}

fn judge(source_root: &Handle, l1_root: &Handle, reading: Reading) -> Verdict {
    let source_tokens = reading.tokens(source_root);
    let l1_tokens = reading.tokens(l1_root);
    let mut verdict = Verdict::default();

    let expected = multiset(&source_tokens);
    let actual = multiset(&l1_tokens);
    verdict.bag_ok = expected == actual;
    if !verdict.bag_ok {
        bag_reasons(&expected, &actual, &mut verdict.reasons);
    }

    let order = order_reasons(l1_root, &source_tokens, reading);
    verdict.order_ok = order.is_empty();
    verdict.reasons.extend(order);

    let structure = structure_reasons(l1_root);
    verdict.structure_ok = structure.is_empty();
    verdict.reasons.extend(structure);

    let mut clean = Vec::new();
    verdict.clean_exempt_raw = clean_reasons(l1_root, &mut clean);
    verdict.clean_ok = clean.is_empty();
    verdict.reasons.extend(clean);

    verdict.unadapted = unadapted_entry(l1_root);
    verdict
}

/// Text tokens of one tree, in document order.
///
/// Tokens are cut out of a *run of rendered text*, not out of one text node:
/// pieces that share a rendering position (`sandwich` followed by
/// `<span>es</span>`, or a text node the engine had to split while mapping)
/// are joined before they are split into words. Only a block boundary ends a
/// run, so two paragraphs are never glued into one word.
#[must_use]
pub fn tokens_of(root: &Handle) -> Vec<String> {
    let mut out = Vec::new();
    let mut run = String::new();
    collect_tokens(root, &mut run, &mut out);
    flush(&mut run, &mut out);
    out
}

fn collect_tokens(node: &Handle, run: &mut String, out: &mut Vec<String>) {
    if let NodeData::Text { contents } = &node.data {
        push_text(&contents.borrow(), run, out);
        return;
    }
    if is_element(node) && skipped(node) {
        // Chrome is not source text, and it is not a place a word may run
        // through either: an injected label ends the visible run.
        flush(run, out);
        return;
    }
    let block = is_element(node) && is_block(node);
    if block {
        flush(run, out);
    }
    let children = node.children.borrow().clone();
    for child in children {
        collect_tokens(&child, run, out);
    }
    if block {
        flush(run, out);
    }
}

/// Text tokens of one tree under the per-node reading: every text node is
/// cut into words on its own, so a word split by an element stays split.
fn tokens_by_node(root: &Handle) -> Vec<String> {
    let mut out = Vec::new();
    collect_node_tokens(root, &mut out);
    out
}

fn collect_node_tokens(node: &Handle, out: &mut Vec<String>) {
    if let NodeData::Text { contents } = &node.data {
        let mut run = String::new();
        push_text(&contents.borrow(), &mut run, out);
        flush(&mut run, out);
        return;
    }
    if is_element(node) && skipped(node) {
        return;
    }
    for child in node.children.borrow().clone() {
        collect_node_tokens(&child, out);
    }
}

/// Whether an element's text is chrome rather than source text.
fn skipped(node: &Handle) -> bool {
    if attr(node, UI).is_some() {
        return true;
    }
    matches!(element_name(node).as_deref(), Some("script" | "style"))
}

/// Add one text node to the current run, cutting words out of it.
///
/// Latin runs stay whole (`don't` is `don` + `t`); every CJK character is its
/// own token, because those scripts are not space-separated. The word in
/// flight is *not* flushed at the end: the run may continue in the next text
/// node.
fn push_text(text: &str, run: &mut String, out: &mut Vec<String>) {
    for ch in text.chars() {
        if is_cjk(ch) {
            flush(run, out);
            out.push(ch.to_string());
        } else if ch.is_alphanumeric() {
            run.push(ch);
        } else {
            flush(run, out);
        }
    }
}

/// Elements that end a text run: their content sits on its own line.
///
/// Inline elements (`span`, `b`, `i`, `em`, `a`, `u`, `strong`, ...) are
/// transparent — text on both sides of them shares one rendering position, so
/// a word may continue across the boundary.
const BLOCKS: [&str; 43] = [
    "address",
    "article",
    "aside",
    "blockquote",
    "body",
    "br",
    "caption",
    "dd",
    "details",
    "dialog",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "html",
    "li",
    "main",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "ul",
];

/// Whether an element ends a text run.
fn is_block(node: &Handle) -> bool {
    element_name(node).is_some_and(|name| BLOCKS.contains(&name.as_str()))
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
fn order_reasons(root: &Handle, source: &[String], reading: Reading) -> Vec<Reason> {
    let mut out = Vec::new();
    let mut blocks: Vec<Handle> = Vec::new();
    collect_blocks(root, &mut blocks);
    for block in blocks {
        let tokens = reading.tokens(&block);
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
        let has_pos_block = children.iter().any(is_pos_block);
        if has_pos_marker(&entry) && !has_pos_block {
            out.push(Reason::new(
                "structure.pos_block",
                "position marker without section.odc-pos-block[data-odc-pos]",
            ));
        }
        if has_pos_block {
            // Senses that lead the entry (before the first position block) are
            // allowed; once a position block has been seen, a following sense
            // sibling is still a structural error.
            let mut seen_pos_block = false;
            let after_pos_block = children.iter().find(|child| {
                if is_pos_block(child) {
                    seen_pos_block = true;
                    return false;
                }
                seen_pos_block
                    && (has_class(child, "odc-sense") || has_class(child, "odc-subsense"))
            });
            if let Some(sense) = after_pos_block {
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

/// Whether `node` is a position container
/// (`section.odc-pos-block[data-odc-pos]`).
fn is_pos_block(node: &Handle) -> bool {
    tag_is(node, "section")
        && has_class(node, POS_BLOCK)
        && attr(node, "data-odc-pos").is_some_and(|value| !value.trim().is_empty())
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
///
/// A raw fallback entry (spec section 6) is exempt: the profile could not
/// adapt the fragment, so the sanitiser kept the source markup on purpose.
/// Its subtree is skipped and the caller is told that the exemption happened,
/// so the report can count it instead of hiding it.
fn clean_reasons(root: &Handle, out: &mut Vec<Reason>) -> bool {
    let mut counters = CleanCounters::default();
    check_clean(root, false, &mut counters, out);
    counters.exempted
}

/// Running counters of one clean pass.
#[derive(Default)]
struct CleanCounters {
    /// Source classes reported so far, across every non-exempt element.
    classes: usize,
    /// Source attributes reported so far.
    attrs: usize,
    /// Whether a raw fallback entry was met.
    exempted: bool,
}

fn check_clean(
    node: &Handle,
    inherited: bool,
    counters: &mut CleanCounters,
    out: &mut Vec<Reason>,
) {
    let exempt = inherited || (is_element(node) && is_raw_fallback(node));
    if exempt {
        counters.exempted = true;
    } else if is_element(node) {
        report_leaks(node, counters, out);
    }
    for child in node.children.borrow().clone() {
        check_clean(&child, exempt, counters, out);
    }
}

/// Report the source classes and attributes of one non-exempt element.
fn report_leaks(element: &Handle, counters: &mut CleanCounters, out: &mut Vec<Reason>) {
    let tag = element_name(element).unwrap_or_default();
    for class in class_list(element) {
        if class.starts_with(ODC_CLASS) {
            continue;
        }
        counters.classes += 1;
        if counters.classes <= MAX_PER_CODE {
            out.push(Reason::new(
                "clean.class",
                format!("source class {class:?} on <{tag}>"),
            ));
        }
    }
    let NodeData::Element { attrs: names, .. } = &element.data else {
        return;
    };
    for name in names.borrow().iter() {
        let name = name.name.local.to_string();
        if name.starts_with(ODC_ATTR) || NEUTRAL_ATTRS.contains(&name.as_str()) {
            continue;
        }
        counters.attrs += 1;
        if counters.attrs <= MAX_PER_CODE {
            out.push(Reason::new(
                "clean.attr",
                format!("source attribute {name:?} on <{tag}>"),
            ));
        }
    }
}

/// Whether an element is a raw fallback entry (spec section 6).
fn is_raw_fallback(node: &Handle) -> bool {
    has_class(node, RAW_ENTRY) || attr(node, RAW_FLAG).as_deref() == Some("1")
}

/// Whether the L1 fragment is a raw fallback, i.e. not adapted at all.
///
/// The engine marks the whole record: `section.odc-entry--raw` plus
/// `data-odc-unadapted="1"`. A fragment can hold several top-level entries, so
/// either the root or any top-level entry marks the record as unadapted.
fn unadapted_entry(root: &Handle) -> bool {
    if is_element(root) && is_raw_fallback(root) {
        return true;
    }
    // Fragment parsing wraps the input in `html`/`body`; `top_level` peels
    // those off and hands back the record entries.
    top_level(root).iter().any(is_raw_fallback)
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

    fn tokens(html: &str) -> Vec<String> {
        tokens_of(&parse_fragment(html).document)
    }

    #[test]
    fn an_inline_boundary_does_not_split_a_word() {
        // Mapping `span.ndv` to its children leaves two text nodes behind:
        // `sandwich` + `es`. Cutting tokens per node used to give
        // ["sandwich", "es"], which the source ("sandwiches") never matches.
        assert_eq!(tokens("<p>sandwich<span>es</span></p>"), ["sandwiches"]);
        assert_eq!(tokens("<p>sandwiches</p>"), ["sandwiches"]);

        let out = verdict(
            "<p>sandwiches</p>",
            r#"<section class="odc-entry"><div class="odc-sense">sandwich<span>es</span></div></section>"#,
        );
        assert!(out.bag_ok, "{:?}", out.reasons);
        assert!(out.order_ok, "{:?}", out.reasons);
    }

    #[test]
    fn a_block_boundary_still_separates_words() {
        assert_eq!(tokens("<p>sand</p><p>wiches</p>"), ["sand", "wiches"]);

        let out = verdict(
            "<p>sand</p><p>wiches</p>",
            r#"<section class="odc-entry"><p>sandwiches</p></section>"#,
        );
        assert!(!out.bag_ok, "{:?}", out.reasons);
    }

    #[test]
    fn a_word_lost_inside_a_run_is_still_reported() {
        let source = "<p>sandwich<span>es</span> and tea</p>";
        let l1 = r#"<section class="odc-entry"><div class="odc-sense">sandwich<span>es</span></div></section>"#;
        let out = verdict(source, l1);
        assert!(!out.bag_ok);
        assert!(out
            .reasons
            .iter()
            .any(|item| item.code == "bag.missing" && item.detail.contains("and")));
    }

    #[test]
    fn a_reordered_run_is_still_reported() {
        let source = "<p>one <b>two</b> three</p>";
        let l1 = r#"<section class="odc-entry"><div class="odc-sense">three one <b>two</b></div></section>"#;
        let out = verdict(source, l1);
        assert!(!out.order_ok);
        assert!(out.reasons.iter().any(|item| item.code == "order.block"));
    }

    #[test]
    fn a_raw_fallback_entry_is_exempt_from_the_clean_check() {
        let source = r#"<div class="dict-merge"><p>a b</p></div>"#;
        let l1 = r#"<section class="odc-entry odc-entry--raw" data-odc-key="k" data-odc-unadapted="1"><div class="dict-merge"><p>a b</p></div></section>"#;
        let out = verdict(source, l1);
        assert!(out.clean_exempt_raw);
        assert!(out.clean_ok, "{:?}", out.reasons);
        assert!(out.reasons.is_empty(), "{:?}", out.reasons);
    }

    #[test]
    fn a_raw_fallback_record_is_unadapted_yet_still_passes_the_four_checks() {
        let source = r#"<div class="dict-merge"><p>a b</p></div>"#;
        let l1 = r#"<section class="odc-entry odc-entry--raw" data-odc-key="k" data-odc-unadapted="1"><div class="dict-merge"><p>a b</p></div></section>"#;
        let out = verdict(source, l1);
        assert!(out.unadapted);
        // `ok()` keeps its old meaning: the four checks passed.
        assert!(out.ok(), "{:?}", out.reasons);
        // The new question: was this record adapted at all? No.
        assert!(!out.adapted_ok());
        let mut summary = ContractSummary::default();
        summary.record(&out);
        assert_eq!(summary.total, 1);
        assert_eq!(summary.adapted, 0);
        assert_eq!(summary.unadapted, 1);
        assert_eq!(summary.coverage(), 0.0);
        assert_eq!(summary.adapted_all_pass, 0);
    }

    #[test]
    fn summary_keeps_unadapted_records_out_of_the_pass_rates() {
        let mapped_pass = verdict(
            "<p>a b</p>",
            r#"<section class="odc-entry"><p>a b</p></section>"#,
        );
        let mapped_fail = verdict(
            "<p>a b</p>",
            r#"<section class="odc-entry"><p>a c</p></section>"#,
        );
        let raw_source = r#"<div class="dict-merge"><p>a b</p></div>"#;
        let raw_l1 = r#"<section class="odc-entry odc-entry--raw" data-odc-key="k" data-odc-unadapted="1"><div class="dict-merge"><p>a b</p></div></section>"#;
        assert!(mapped_pass.adapted_ok(), "{:?}", mapped_pass.reasons);
        assert!(!mapped_fail.adapted_ok());
        let mut summary = ContractSummary::default();
        for item in [
            &mapped_pass,
            &mapped_pass,
            &mapped_fail,
            &verdict(raw_source, raw_l1),
            &verdict(raw_source, raw_l1),
        ] {
            summary.record(item);
        }
        assert_eq!(summary.total, 5);
        assert_eq!(summary.adapted, 3);
        assert_eq!(summary.unadapted, 2);
        assert_eq!(summary.bag_pass, 2);
        assert_eq!(summary.adapted_all_pass, 2);
        // The four rates never use the five-record total as their base.
        assert!(summary.order_pass <= summary.adapted);
        assert!(summary.structure_pass <= summary.adapted);
        assert!(summary.clean_pass <= summary.adapted);
        assert_eq!(summary.coverage(), 0.6);
        assert_eq!(
            summary.render_line(),
            "coverage 3/5 (60.000%) | mapped 2/3 four-dim | unadapted 2"
        );
        assert!(summary.strict_violation().is_some());
    }

    #[test]
    fn a_fully_adapted_run_has_full_coverage_and_no_strict_violation() {
        let out = verdict(
            "<p>a b</p>",
            r#"<section class="odc-entry"><p>a b</p></section>"#,
        );
        let mut summary = ContractSummary::default();
        summary.record(&out);
        assert_eq!(summary.coverage(), 1.0);
        assert_eq!(
            summary.render_line(),
            "coverage 1/1 (100.000%) | mapped 1/1 four-dim | unadapted 0"
        );
        assert!(summary.strict_violation().is_none());
    }

    #[test]
    fn an_ordinary_entry_is_still_checked_for_source_classes() {
        let source = r#"<div class="dict-merge"><p>a b</p></div>"#;
        let l1 = r#"<section class="odc-entry"><div class="dict-merge"><p>a b</p></div></section>"#;
        let out = verdict(source, l1);
        assert!(!out.clean_ok);
        assert!(!out.clean_exempt_raw);
        assert!(out
            .reasons
            .iter()
            .any(|item| item.code == "clean.class" && item.detail.contains("dict-merge")));
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
    fn a_leading_sense_may_sit_before_a_position_block() {
        // Some dictionaries print a run-in sense ahead of the part-of-speech
        // container; that sense has no position block to live in yet.
        let source = "<p>lead verb move</p>";
        let l1 = r#"<section class="odc-entry">
            <div class="odc-sense">lead</div>
            <section class="odc-pos-block" data-odc-pos="verb"><p>verb move</p></section>
        </section>"#;
        let out = verdict(source, l1);
        assert!(out.structure_ok, "{:?}", out.reasons);
    }

    #[test]
    fn a_sense_between_position_blocks_is_still_a_structure_failure() {
        let source = "<p>verb move noun walk</p>";
        let l1 = r#"<section class="odc-entry">
            <section class="odc-pos-block" data-odc-pos="verb"><p>verb</p></section>
            <div class="odc-sense">move</div>
            <section class="odc-pos-block" data-odc-pos="noun"><p>noun walk</p></section>
        </section>"#;
        let out = verdict(source, l1);
        assert!(!out.structure_ok);
        assert!(out
            .reasons
            .iter()
            .any(|item| item.code == "structure.sense_sibling"));
    }

    #[test]
    fn a_nested_entry_keeps_its_own_boundary() {
        // The nested entry starts with a sense and has no position block of its
        // own; the outer entry's position block is not its business.
        let source = "<p>verb lead</p>";
        let l1 = r#"<section class="odc-entry">
            <section class="odc-pos-block" data-odc-pos="verb"><p>verb</p>
                <section class="odc-entry"><div class="odc-sense">lead</div></section>
            </section>
        </section>"#;
        let out = verdict(source, l1);
        assert!(out.structure_ok, "{:?}", out.reasons);
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
