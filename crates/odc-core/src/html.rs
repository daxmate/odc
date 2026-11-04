//! HTML repair, sanitisation and formatting for MDX entry bodies.
//!
//! Dictionaries ship their entries as HTML fragments of wildly varying
//! quality: unclosed tags, inline event handlers, stray `<script>` blocks and
//! CSS that only a browser's error recovery can read. Importing that text
//! verbatim stores the damage, so every body can optionally go through this
//! module before it becomes an [`Entry`](crate::entry::Entry).
//!
//! The pipeline is deliberately narrow:
//!
//! 1. `html5ever` parses the fragment and repairs it following the HTML5
//!    tree-construction rules; tree-builder and tokenizer errors are collected.
//! 2. Dangerous elements (`script`, `iframe`, `object`, `embed`), event
//!    handler attributes and `javascript:` URLs are removed.
//! 3. CSS in `<style>` blocks and `style` attributes is parsed and
//!    re-serialised with `lightningcss`.
//! 4. The repaired tree is serialised and laid out by [`crate::block_fmt`]:
//!    line breaks and indentation appear only at block-level element
//!    boundaries. Tags are never split and text nodes are copied byte for
//!    byte, so the rendered text of dictionary entries is preserved exactly.
//!    A body that cannot be tokenised keeps its unformatted (repaired) markup
//!    and reports `format.failed`.
//!
//! `HtmlMode::Check` runs the same analysis but never touches the output, and
//! `HtmlMode::Off` returns the input before any parsing happens.
//!
//! Formatting failures do not abort anything: the outcome points at the
//! repaired serialisation and carries a `format.failed` diagnostic. The import
//! layer decides what to store (see `mdx_import`); this module always reports
//! what it would have written.

use std::cell::RefCell;
use std::rc::Rc;

use html5ever::driver::{parse_fragment_for_element, ParseOpts};
use html5ever::serialize::{SerializeOpts, TraversalScope};
use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::{local_name, ns, Attribute, QualName};
use lightningcss::printer::PrinterOptions;
use lightningcss::stylesheet::{ParserOptions, StyleAttribute, StyleSheet};
use markup5ever_rcdom::{Handle, Node, NodeData, RcDom, SerializableHandle};

use crate::block_fmt::format_blocks;

/// Diagnostic code for an HTML5 parse / tree-construction error.
pub const PARSE_ERROR: &str = "html.parse_error";
/// Diagnostic code for a removed element (see `DANGEROUS_ELEMENTS`).
pub const REMOVED_ELEMENT: &str = "html.removed_element";
/// Diagnostic code for a removed attribute (event handlers).
pub const REMOVED_ATTRIBUTE: &str = "html.removed_attribute";
/// Diagnostic code for a removed `javascript:` URL.
pub const UNSAFE_URL: &str = "html.unsafe_url";
/// Diagnostic code for CSS that `lightningcss` could not parse.
pub const CSS_PARSE_ERROR: &str = "css.parse_error";
/// Diagnostic code for a body the block formatter could not tokenise.
pub const FORMAT_FAILED: &str = "format.failed";

/// Elements removed from imported bodies.
///
/// The list is intentionally short: anything else is dictionary content and
/// must survive the round trip.
const DANGEROUS_ELEMENTS: [&str; 4] = ["script", "iframe", "object", "embed"];

/// Attributes whose value is loaded as a URL when `javascript:` is checked.
const URL_ATTRIBUTES: [&str; 9] = [
    "action",
    "background",
    "cite",
    "data",
    "formaction",
    "href",
    "longdesc",
    "poster",
    "src",
];

/// How an entry body is processed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HtmlMode {
    /// Return the body untouched, without parsing it.
    Off,
    /// Report problems without changing a byte of the output.
    Check,
    /// Repair, sanitise and format the body.
    #[default]
    Format,
}

impl HtmlMode {
    /// Parse a mode name as accepted by the CLI (`off`, `check`, `format`).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "off" => Some(Self::Off),
            "check" => Some(Self::Check),
            "format" => Some(Self::Format),
            _ => None,
        }
    }

    /// Name used by the CLI and reports.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Check => "check",
            Self::Format => "format",
        }
    }
}

/// Options for [`process_html`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HtmlOptions {
    /// What to do with a body; defaults to `Format`.
    pub mode: HtmlMode,
    /// Retained for API compatibility; the block formatter never wraps at a
    /// width. The value is ignored (default 80).
    pub line_width: usize,
    /// Samples kept per diagnostic code when callers aggregate a summary
    /// (default 20). [`process_html`] itself never truncates diagnostics.
    pub max_samples: usize,
}

impl Default for HtmlOptions {
    fn default() -> Self {
        Self {
            mode: HtmlMode::Format,
            line_width: 80,
            max_samples: 20,
        }
    }
}

/// One problem found while processing a body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HtmlDiagnostic {
    /// Stable code, safe to group by ([`PARSE_ERROR`], [`CSS_PARSE_ERROR`], ...).
    pub code: String,
    /// Short human-readable description.
    pub message: String,
    /// Optional extra context (parser position, CSS error detail, ...).
    pub detail: Option<String>,
}

/// Result of [`process_html`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HtmlOutcome {
    /// Body to store (`Check` and `Off` return the input unchanged).
    pub html: String,
    /// Whether `html` differs from the input.
    pub changed: bool,
    /// Every diagnostic for this body, in discovery order.
    pub diagnostics: Vec<HtmlDiagnostic>,
    /// Number of `<style>` blocks and `style` attributes seen.
    pub css_blocks: usize,
    /// Whether CSS rewriting changed anything.
    pub css_changed: bool,
    /// Whether the block formatter failed; `html` then holds the repaired
    /// body and the caller should keep the original text instead.
    pub failed: bool,
}

impl HtmlOutcome {
    fn unchanged(input: &str) -> Self {
        Self {
            html: input.to_string(),
            changed: false,
            diagnostics: Vec::new(),
            css_blocks: 0,
            css_changed: false,
            failed: false,
        }
    }
}

/// Parse error details are kept verbatim: the tree builder reports positions
/// only when `exact_errors` is on.
fn parse_options() -> ParseOpts {
    let mut options = ParseOpts::default();
    options.tree_builder.exact_errors = true;
    options.tokenizer.exact_errors = true;
    options
}

/// Repair, sanitise and (depending on the mode) format one entry body.
///
/// The function never fails: malformed input, empty strings and bodies without
/// markup all come back as an [`HtmlOutcome`]. Diagnostics are complete for
/// the body; sampling is the caller's job.
#[must_use]
pub fn process_html(input: &str, options: &HtmlOptions) -> HtmlOutcome {
    if options.mode == HtmlMode::Off || !input.contains('<') {
        return HtmlOutcome::unchanged(input);
    }

    let (dom, context) = parse_and_clean(input, options.mode);

    let mut diagnostics = context.diagnostics;
    let css_blocks = context.css_blocks;
    let css_changed = context.css_changed;
    if options.mode == HtmlMode::Check {
        return HtmlOutcome {
            html: input.to_string(),
            changed: false,
            diagnostics,
            css_blocks,
            css_changed: false,
            failed: false,
        };
    }

    let repaired = serialize_fragment(&dom);
    let (html, failed) = match format_blocks(&repaired) {
        Ok(formatted) => (formatted, false),
        Err(error) => {
            diagnostics.push(HtmlDiagnostic {
                code: FORMAT_FAILED.to_string(),
                message: "block formatter could not lay out the body".to_string(),
                detail: Some(error.to_string()),
            });
            (repaired, true)
        }
    };
    let changed = html != input;
    HtmlOutcome {
        html,
        changed,
        diagnostics,
        css_blocks,
        css_changed,
        failed,
    }
}

/// Repair and sanitise one body without formatting it.
///
/// Returns `None` for bodies without markup. Used by the formatter sampler
/// to compare the formatter's output with the markup it actually receives;
/// the import path calls [`process_html`] instead.
#[must_use]
pub fn repair_sanitize(input: &str) -> Option<String> {
    if !input.contains('<') {
        return None;
    }
    let (dom, _context) = parse_and_clean(input, HtmlMode::Format);
    Some(serialize_fragment(&dom))
}

/// Parse a fragment and run the repair / sanitisation walk.
fn parse_and_clean(input: &str, mode: HtmlMode) -> (RcDom, Context) {
    // Parse as a fragment in a `<div>` context: entries are body fragments,
    // and document parsing would wrap them in `<html><head><body>` and report
    // a spurious "unexpected start tag in initial mode" for every body.
    let dom: RcDom = parse_fragment_for_element(
        RcDom::default(),
        parse_options(),
        fragment_context(),
        true,
        None,
    )
    .one(input.to_string());
    let mut context = Context::new(mode);
    // `<template>` keeps its children in a separate slot that the serialiser
    // does not visit; move them onto the element so nothing is dropped.
    flatten_templates(&dom.document);
    for error in dom.errors.borrow().iter() {
        context.push(PARSE_ERROR, error.to_string(), None);
    }
    clean_children(&dom.document, &mut context);
    (dom, context)
}

/// State shared by one tree walk.
struct Context {
    apply: bool,
    diagnostics: Vec<HtmlDiagnostic>,
    css_blocks: usize,
    css_changed: bool,
}

impl Context {
    fn new(mode: HtmlMode) -> Self {
        Self {
            apply: mode == HtmlMode::Format,
            diagnostics: Vec::new(),
            css_blocks: 0,
            css_changed: false,
        }
    }

    fn push(&mut self, code: &str, message: impl Into<String>, detail: Option<String>) {
        self.diagnostics.push(HtmlDiagnostic {
            code: code.to_string(),
            message: message.into(),
            detail,
        });
    }
}

fn flatten_templates(handle: &Handle) {
    if let NodeData::Element {
        template_contents, ..
    } = &handle.data
    {
        if let Some(contents) = template_contents.borrow_mut().take() {
            // `mem::take` matters: `Node::drop` clears the subtree of every
            // child a dropped node still owns, so cloning the handles and then
            // releasing the template contents would wipe the moved children.
            let inner: Vec<Handle> = std::mem::take(&mut *contents.children.borrow_mut());
            handle.children.borrow_mut().extend(inner);
        }
    }
    for child in handle.children.borrow().iter() {
        flatten_templates(child);
    }
}

/// Remove (or report, in `Check` mode) dangerous children, then walk the rest.
fn clean_children(parent: &Handle, context: &mut Context) {
    let children = parent.children.borrow().clone();
    let mut kept = Vec::with_capacity(children.len());
    let mut removed = false;
    for child in children {
        if let Some(tag) = dangerous_element(&child) {
            context.push(REMOVED_ELEMENT, format!("element <{tag}>"), None);
            if context.apply {
                removed = true;
                continue;
            }
        }
        clean_node(&child, context);
        kept.push(child);
    }
    if removed {
        *parent.children.borrow_mut() = kept;
    }
}

/// Inspect one node: attributes and CSS first, then its children.
fn clean_node(handle: &Handle, context: &mut Context) {
    if let NodeData::Element { name, attrs, .. } = &handle.data {
        clean_attributes(attrs, context);
        if name.local.as_ref().eq_ignore_ascii_case("style") {
            clean_style_element(&handle.children, context);
        }
    }
    clean_children(handle, context);
}

fn dangerous_element(handle: &Handle) -> Option<String> {
    let NodeData::Element { name, .. } = &handle.data else {
        return None;
    };
    let local = name.local.as_ref().to_ascii_lowercase();
    DANGEROUS_ELEMENTS
        .contains(&local.as_str())
        .then_some(local)
}

fn clean_attributes(attrs: &RefCell<Vec<Attribute>>, context: &mut Context) {
    let mut slots = attrs.borrow_mut();
    let mut kept = Vec::with_capacity(slots.len());
    for mut attr in slots.drain(..) {
        let local = attr.name.local.as_ref().to_ascii_lowercase();
        if local.len() > 2 && local.starts_with("on") {
            context.push(REMOVED_ATTRIBUTE, format!("attribute `{local}`"), None);
            if context.apply {
                continue;
            }
        }
        if URL_ATTRIBUTES.contains(&local.as_str()) && is_javascript_url(&attr.value) {
            context.push(UNSAFE_URL, format!("`javascript:` URL in `{local}`"), None);
            if context.apply {
                continue;
            }
        }
        if local == "style" && !attr.value.trim().is_empty() {
            context.css_blocks += 1;
            match serialize_style_attribute(&attr.value) {
                Ok(serialized) => {
                    if serialized != *attr.value {
                        context.css_changed = true;
                        if context.apply {
                            attr.value = StrTendril::from(serialized);
                        }
                    }
                }
                Err(error) => context.push(CSS_PARSE_ERROR, "style attribute", Some(error)),
            }
        }
        kept.push(attr);
    }
    *slots = kept;
}

fn clean_style_element(children: &RefCell<Vec<Handle>>, context: &mut Context) {
    let original = collect_text(children);
    if original.trim().is_empty() {
        return;
    }
    context.css_blocks += 1;
    match serialize_stylesheet(&original) {
        Ok(serialized) => {
            if serialized != original {
                context.css_changed = true;
                if context.apply {
                    set_text_content(children, &serialized);
                }
            }
        }
        Err(error) => context.push(CSS_PARSE_ERROR, "style block", Some(error)),
    }
}

fn collect_text(children: &RefCell<Vec<Handle>>) -> String {
    let mut out = String::new();
    for child in children.borrow().iter() {
        if let NodeData::Text { contents } = &child.data {
            out.push_str(&contents.borrow());
        }
    }
    out
}

fn set_text_content(children: &RefCell<Vec<Handle>>, text: &str) {
    let mut slots = children.borrow_mut();
    let mut first: Option<Handle> = None;
    for child in slots.iter() {
        if matches!(child.data, NodeData::Text { .. }) {
            first = Some(child.clone());
            break;
        }
    }
    match first {
        Some(node) => {
            if let NodeData::Text { contents } = &node.data {
                *contents.borrow_mut() = StrTendril::from(text);
            }
            slots.retain(|child| Rc::ptr_eq(child, &node));
        }
        None => slots.push(Node::new(NodeData::Text {
            contents: RefCell::new(StrTendril::from(text)),
        })),
    }
}

/// True when the value starts with `javascript:` after HTML whitespace and
/// control characters are ignored (the HTML URL parser does the same).
fn is_javascript_url(value: &str) -> bool {
    let mut cleaned = String::with_capacity(value.len());
    for character in value.chars() {
        if !character.is_ascii_whitespace() && !character.is_ascii_control() {
            cleaned.push(character);
        }
    }
    cleaned
        .get(..11)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("javascript:"))
}

fn serialize_stylesheet(css: &str) -> Result<String, String> {
    let sheet =
        StyleSheet::parse(css, ParserOptions::default()).map_err(|error| error.to_string())?;
    let printed = sheet
        .to_css(PrinterOptions::default())
        .map_err(|error| error.to_string())?;
    Ok(printed.code)
}

fn serialize_style_attribute(css: &str) -> Result<String, String> {
    let attribute =
        StyleAttribute::parse(css, ParserOptions::default()).map_err(|error| error.to_string())?;
    let printed = attribute
        .to_css(PrinterOptions::default())
        .map_err(|error| error.to_string())?;
    Ok(printed.code)
}

/// Serialise the children of the fragment root the tree builder created.
///
/// `html5ever` puts fragment nodes under a fresh `<html>` root appended to the
/// document; unlike a full parse it never synthesises `<head>` or `<body>`,
/// so the root's children are exactly the fragment.
fn serialize_fragment(dom: &RcDom) -> String {
    let mut out = String::new();
    for child in dom.document.children.borrow().iter() {
        if is_element_named(child, "html") {
            out.push_str(&serialize_with(child, TraversalScope::ChildrenOnly(None)));
        } else {
            out.push_str(&serialize_with(child, TraversalScope::IncludeNode));
        }
    }
    out
}

fn is_element_named(handle: &Handle, name: &str) -> bool {
    match &handle.data {
        NodeData::Element { name: qual, .. } => qual.local.as_ref().eq_ignore_ascii_case(name),
        _ => false,
    }
}

/// Context element for fragment parsing.
///
/// A `<div>` context accepts every element a dictionary body may contain
/// while keeping the fragment out of the `<html><head><body>` wrapper.
fn fragment_context() -> Handle {
    Node::new(NodeData::Element {
        name: QualName::new(None, ns!(html), local_name!("div")),
        attrs: RefCell::new(Vec::new()),
        template_contents: RefCell::new(None),
        mathml_annotation_xml_integration_point: false,
    })
}

fn serialize_with(node: &Handle, traversal_scope: TraversalScope) -> String {
    let handle = SerializableHandle::from(node.clone());
    let options = SerializeOpts {
        traversal_scope,
        ..SerializeOpts::default()
    };
    let mut buffer = Vec::new();
    if html5ever::serialize(&mut buffer, &handle, options).is_err() {
        return String::new();
    }
    String::from_utf8_lossy(&buffer).into_owned()
}

/// One example of a diagnostic, kept by [`HtmlImportSummary`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HtmlSample {
    /// Headword (or other caller label) the diagnostic belongs to.
    pub headword: String,
    /// Diagnostic code (see the constants at the top of this module).
    pub code: String,
    /// Human-readable description.
    pub message: String,
}

/// Aggregated HTML work for an import, a `check-html` run or `normalize`.
///
/// [`process_html`] reports every diagnostic of every body; this summary is
/// where they are counted and sampled, so callers never walk a body twice.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HtmlImportSummary {
    /// Bodies that went through `process_html`.
    pub entries_checked: usize,
    /// Bodies the caller stored with a different `raw_html`.
    pub entries_changed: usize,
    /// Bodies whose formatting failed; they keep their original text.
    pub entries_failed: usize,
    /// Diagnostic count per code.
    pub diagnostics: std::collections::BTreeMap<String, u64>,
    /// Up to `max_samples` examples per code, in discovery order.
    pub samples: Vec<HtmlSample>,
    /// Bodies that contained at least one `<style>` block or `style` attribute.
    pub css_entries: usize,
    /// Bodies whose CSS was rewritten.
    pub css_changed: usize,
}

impl HtmlImportSummary {
    /// Fold one [`HtmlOutcome`] into the summary.
    ///
    /// `changed` is what the caller actually stored: an entry whose formatting
    /// failed is kept verbatim, so it is not counted as changed even though
    /// `outcome.html` holds a best-effort repair.
    pub fn record(
        &mut self,
        headword: &str,
        outcome: &HtmlOutcome,
        changed: bool,
        max_samples: usize,
    ) {
        self.entries_checked += 1;
        if changed {
            self.entries_changed += 1;
        }
        if outcome.failed {
            self.entries_failed += 1;
        }
        if outcome.css_blocks > 0 {
            self.css_entries += 1;
        }
        if outcome.css_changed {
            self.css_changed += 1;
        }
        for diagnostic in &outcome.diagnostics {
            *self.diagnostics.entry(diagnostic.code.clone()).or_insert(0) += 1;
            let already = self
                .samples
                .iter()
                .filter(|sample| sample.code == diagnostic.code)
                .count();
            if already < max_samples {
                self.samples.push(HtmlSample {
                    headword: headword.to_string(),
                    code: diagnostic.code.clone(),
                    message: diagnostic.message.clone(),
                });
            }
        }
    }

    /// True when nothing was checked yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries_checked == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(input: &str) -> HtmlOutcome {
        process_html(input, &HtmlOptions::default())
    }

    fn codes(outcome: &HtmlOutcome) -> Vec<&str> {
        outcome
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect()
    }

    fn text_of(html: &str) -> String {
        let dom: RcDom = parse_fragment_for_element(
            RcDom::default(),
            parse_options(),
            fragment_context(),
            true,
            None,
        )
        .one(html.to_string());
        let mut out = String::new();
        collect_all_text(&dom.document, &mut out);
        out
    }

    fn collect_all_text(handle: &Handle, out: &mut String) {
        if let NodeData::Text { contents } = &handle.data {
            out.push_str(&contents.borrow());
        }
        for child in handle.children.borrow().iter() {
            collect_all_text(child, out);
        }
        if let NodeData::Element {
            template_contents, ..
        } = &handle.data
        {
            if let Some(contents) = template_contents.borrow().as_ref() {
                collect_all_text(contents, out);
            }
        }
    }

    #[test]
    fn repairs_unclosed_tags() {
        let outcome = format("<p>a<b>b</p>");
        assert!(outcome.changed);
        assert!(outcome.html.starts_with("<p>a<b>b</b>"), "{}", outcome.html);
        assert!(outcome.html.ends_with("</p>"), "{}", outcome.html);
        assert!(codes(&outcome).contains(&PARSE_ERROR));
    }

    #[test]
    fn inline_neighbours_keep_text() {
        // Strict whitespace sensitivity: no space may appear between the
        // inline elements, otherwise the rendered text changes ("abc" -> "a b c").
        let input = "<p>a<b>b</b><i>c</i>d</p>";
        let outcome = format(input);
        assert_eq!(outcome.html, "<p>a<b>b</b><i>c</i>d</p>");
        assert_eq!(text_of(&outcome.html), text_of(input));
        assert_eq!(text_of(&outcome.html), "abcd");
    }

    #[test]
    fn removes_scripts_handlers_and_javascript_urls() {
        let input = concat!(
            "<p onclick=\"x()\">t<script>alert(1)</script>",
            "<iframe src=\"x\"></iframe>",
            "<object data=\"x\"></object><embed src=\"x\">",
            "<a href=\"javascript:void(0)\">l</a></p>"
        );
        let outcome = format(input);
        let html = &outcome.html;
        assert!(!html.contains("script"), "{html}");
        assert!(!html.contains("onclick"), "{html}");
        assert!(!html.contains("javascript"), "{html}");
        assert!(!html.contains("iframe"), "{html}");
        assert!(!html.contains("object"), "{html}");
        assert!(!html.contains("embed"), "{html}");
        let codes = codes(&outcome);
        assert!(codes.contains(&REMOVED_ELEMENT));
        assert!(codes.contains(&REMOVED_ATTRIBUTE));
        assert!(codes.contains(&UNSAFE_URL));
    }

    #[test]
    fn keeps_plain_text_and_safe_markup() {
        let input = "<p>a < b &amp; c</p>";
        let outcome = format(input);
        assert_eq!(text_of(&outcome.html), "a < b & c");
    }

    #[test]
    fn reports_css_errors_and_rewrites_valid_css() {
        let ok = format("<style>p{color:red}</style><p style=\"color : red\">x</p>");
        assert!(!codes(&ok).contains(&CSS_PARSE_ERROR));
        assert_eq!(ok.css_blocks, 2);
        assert!(ok.css_changed);
        assert!(ok.html.contains("color: red"), "{}", ok.html);
        assert!(ok.html.contains("<p style=\"color: red\">"), "{}", ok.html);

        let bad = format("<style>p { color red }</style><p style=\"color red\">y</p>");
        let css_errors = codes(&bad)
            .iter()
            .filter(|code| **code == CSS_PARSE_ERROR)
            .count();
        assert_eq!(css_errors, 2);
        assert!(bad.html.contains("color red"), "{}", bad.html);
    }

    #[test]
    fn formatting_is_idempotent() {
        let input = "<div><p>Hello<BR>world</p><ul><li>one<li>two</ul></div>";
        let once = format(input);
        let twice = format(&once.html);
        assert_eq!(once.html, twice.html);
        assert!(once.changed);
    }

    #[test]
    fn check_mode_is_byte_identical() {
        let input = "<p onclick=\"x()\">t<script>alert(1)</script>  <b>u</p>";
        let outcome = process_html(
            input,
            &HtmlOptions {
                mode: HtmlMode::Check,
                ..HtmlOptions::default()
            },
        );
        assert_eq!(outcome.html, input);
        assert!(!outcome.changed);
        assert!(!outcome.diagnostics.is_empty());
    }

    #[test]
    fn off_mode_returns_input_untouched() {
        let input = "<p>a<b>b</p>";
        let outcome = process_html(
            input,
            &HtmlOptions {
                mode: HtmlMode::Off,
                ..HtmlOptions::default()
            },
        );
        assert_eq!(outcome.html, input);
        assert!(outcome.diagnostics.is_empty());
    }

    #[test]
    fn empty_and_plain_bodies_are_safe() {
        for input in ["", "plain text", "&amp; &lt;"] {
            let outcome = format(input);
            assert_eq!(outcome.html, input);
            assert!(!outcome.changed);
            assert!(outcome.diagnostics.is_empty());
        }
        let spaced = format("a < b");
        assert_eq!(text_of(&spaced.html), "a < b");
    }

    #[test]
    fn preserves_template_contents() {
        // `<template>` children live in a separate slot; moving them out must
        // not let `Node::drop` clear the subtree.
        let outcome = format("<template><p>x</p></template>");
        // The block formatter breaks before block-level children, including
        // those that were moved out of the template slot.
        assert_eq!(outcome.html, "<template>\n<p>x</p></template>");
        assert_eq!(text_of(&outcome.html).trim(), "x");
    }

    #[test]
    fn preserves_preformatted_text() {
        let input = "<pre>  a\n   b</pre>";
        let outcome = format(input);
        assert!(
            outcome.html.contains("<pre>  a\n   b</pre>"),
            "{}",
            outcome.html
        );
    }
}
