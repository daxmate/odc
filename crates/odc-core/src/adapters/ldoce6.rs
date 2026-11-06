//! LDOCE6 / LDOCE6++ (English-Chinese) adapter — prototype.
//!
//! Scope: the `LDOCE6++ En-Cn V2-19` family, which stores one MDX record per
//! headword and keeps the entry body inside a closed-over `span.entryhead`
//! (the source markup leaks the wrapper across the whole entry). The adapter
//! therefore rebuilds the block skeleton instead of trusting source nesting.
//!
//! Mapping follows `docs/odc-html-spec.md` section 4; the fallback path is
//! section 6 and the acceptance numbers are section 8.

use std::collections::HashMap;

use super::tokens::{self, Tok};
use super::{escape_attr, raw_fallback, Adapter, ParseContext, Probe, Reason, Signal};
use crate::block_fmt::format_blocks;
use crate::error::Result;
use crate::html::repair_sanitize;

/// Dictionary slug written to `data-odc-dict`.
pub const SLUG: &str = "ldoce6encn";

/// Class fingerprints used by `probe`.
const PROBE_CLASSES: [&str; 9] = [
    "entry",
    "entryhead",
    "hwd",
    "sensenum",
    "def",
    "phrvbentry",
    "signpost",
    "exat",
    "deft",
];

/// The LDOCE6 adapter.
#[derive(Debug, Default, Clone, Copy)]
pub struct Ldoce6;

impl Adapter for Ldoce6 {
    fn dict(&self) -> &'static str {
        SLUG
    }

    fn probe(&self, fragment: &str) -> Option<Probe> {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for tok in tokens::tokenize(fragment) {
            if let Tok::Start { attrs, .. } = tok {
                for class in PROBE_CLASSES {
                    if tokens::has_class(&attrs, class) {
                        *counts.entry(class).or_default() += 1;
                    }
                }
            }
        }
        let entry = counts.get("entry").copied().unwrap_or(0);
        let entryhead = counts.get("entryhead").copied().unwrap_or(0);
        let sensenum = counts.get("sensenum").copied().unwrap_or(0);
        let def = counts.get("def").copied().unwrap_or(0);
        let exat = counts.get("exat").copied().unwrap_or(0);
        let confidence = if entry >= 1 && entryhead >= 1 {
            0.99
        } else if sensenum >= 3 && def >= 1 && exat >= 1 {
            0.62
        } else {
            return None;
        };
        let mut signals: Vec<Signal> = counts
            .iter()
            .map(|(name, count)| Signal {
                name: String::from(*name),
                count: *count,
            })
            .collect();
        signals.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
        Some(Probe {
            dict: SLUG,
            confidence,
            signals,
        })
    }

    fn parse(&self, fragment: &str, ctx: &ParseContext) -> Result<String> {
        if let Some(target) = redirect_target(fragment) {
            return Ok(redirect_entry(&target, ctx));
        }
        if fragment.trim().is_empty() {
            return Ok(raw_fallback(fragment, ctx, Reason::ParseFailed));
        }
        // Idempotence (spec 8.4): L1 input is already in the target shape, so it
        // is returned untouched. Re-formatting would not be a no-op: the block
        // formatter re-indents nested lists differently on a second pass.
        if is_l1(fragment) {
            return Ok(String::from(fragment));
        }
        let Some(clean) = repair_sanitize(fragment) else {
            return Ok(raw_fallback(fragment, ctx, Reason::ParseFailed));
        };
        let mut builder = Builder::new(ctx);
        builder.build(&clean);
        if !builder.adapted || builder.out.is_empty() {
            return Ok(raw_fallback(fragment, ctx, Reason::ParseFailed));
        }
        let mut html = builder.finish();
        html = strip_popup_buttons(&html);
        html = format_blocks(&html).unwrap_or(html);
        // Toolbar button labels are new chrome; everything else must stay verbatim.
        let clean_for_compare = strip_popup_buttons(&clean);
        if !same_words(&clean_for_compare, &html) {
            return Ok(raw_fallback(fragment, ctx, Reason::ParseFailed));
        }
        Ok(html)
    }

    fn resolve(&self, reference: &str) -> Option<String> {
        resource_key(reference)
    }

    fn styles(&self) -> Option<&'static str> {
        None
    }
}

/// True when the fragment already carries an L1 root.
#[must_use]
pub fn is_l1(fragment: &str) -> bool {
    tokens::tokenize(fragment).into_iter().any(
        |tok| matches!(tok, Tok::Start { ref attrs, .. } if tokens::has_class(attrs, "odc-entry")),
    )
}

/// Compare the extracted text of two documents (spec 8.1).
///
/// The spec asks for a byte comparison *after* removing the whitespace the
/// formatter inserts at block boundaries. `content_text` does exactly that:
/// every whitespace run collapses to one space, so indentation and line breaks
/// between blocks do not count while every word does.
#[must_use]
pub fn same_text(left: &str, right: &str) -> bool {
    content_text(left) == content_text(right)
}

/// UI triggers such as "Word Origin" are not entry text; the toolbar owns them.
fn strip_popup_buttons(html: &str) -> String {
    const OPEN: &str = "<span data-odc-orig-class=\"popup-button\">";
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        let after = &rest[start + OPEN.len()..];
        match after.find("</span>") {
            Some(end) if !after[..end].contains('<') => {
                rest = &after[end + "</span>".len()..];
            }
            _ => {
                out.push_str(OPEN);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Fidelity guard for the reordered entry: same words, same counts, any order.
///
/// Toolbar button labels are UI chrome and are ignored on both sides.
#[must_use]
pub fn same_words(left: &str, right: &str) -> bool {
    let mut a: Vec<String> = content_text(&strip_chrome(left))
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let mut b: Vec<String> = content_text(&strip_chrome(right))
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    a.sort_unstable();
    b.sort_unstable();
    a == b
}

/// Whitespace-normalised text of a document.
#[must_use]
pub fn content_text(html: &str) -> String {
    let mut out = String::new();
    for tok in tokens::tokenize(html) {
        let Tok::Text(text) = tok else {
            continue;
        };
        for word in text.split_whitespace() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(word);
        }
    }
    out
}

/// Concatenated character data of a document, in document order.
#[must_use]
pub fn text_concat(html: &str) -> String {
    let mut out = String::new();
    for tok in tokens::tokenize(html) {
        if let Tok::Text(text) = tok {
            out.push_str(&text);
        }
    }
    out
}

/// Text-node runs of a document, whitespace-only runs dropped.
#[must_use]
pub fn text_runs(html: &str) -> Vec<String> {
    tokens::tokenize(html)
        .into_iter()
        .filter_map(|tok| match tok {
            Tok::Text(text) if !text.trim().is_empty() => Some(text),
            _ => None,
        })
        .collect()
}

/// Entry slug from spec 3.4: lower-case, non `[a-z0-9-]` folded to `-`.
#[must_use]
pub fn entry_slug(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for ch in key.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_lowercase() || lower.is_ascii_digit() {
            out.push(lower);
        } else {
            out.push('-');
        }
    }
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        let mut fallback = String::from("u");
        for (index, ch) in key.chars().enumerate() {
            if index > 0 {
                fallback.push('-');
            }
            fallback.push_str(&format!("{:X}", u32::from(ch)));
        }
        fallback
    } else {
        trimmed
    }
}

fn pos_slug(pos: &str) -> String {
    let mut out = String::with_capacity(pos.len());
    for ch in pos.trim().chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_lowercase() || lower.is_ascii_digit() {
            out.push(lower);
        } else if lower == ' ' || lower == '_' || lower == '/' {
            out.push('-');
        }
    }
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        String::from("x")
    } else {
        trimmed
    }
}

/// Text of the first `Text` token that follows a start tag carrying `class`.
fn text_after_class(tokens: &[Tok], from: usize, class: &str) -> Option<String> {
    for index in from..tokens.len() {
        if let Tok::Start { attrs, .. } = &tokens[index] {
            if tokens::has_class(attrs, class) {
                for tok in tokens.iter().skip(index + 1) {
                    match tok {
                        Tok::Text(text) => {
                            let trimmed = text.trim();
                            if !trimmed.is_empty() {
                                return Some(trimmed.to_string());
                            }
                        }
                        Tok::Start { .. } => {}
                        Tok::End { .. } => return None,
                    }
                }
                return None;
            }
        }
    }
    None
}

/// Text that immediately follows the start tag at `at`.
fn inline_text(tokens: &[Tok], at: usize) -> String {
    match tokens.get(at + 1) {
        Some(Tok::Text(text)) => text.trim().to_string(),
        _ => String::new(),
    }
}

/// One open block frame.
struct Frame {
    level: u8,
    tag: &'static str,
    close: String,
    inline_base: usize,
}

/// One open inline element: source end tag name -> emitted end tag name.
struct Inline {
    src: String,
    out: String,
    role: &'static str,
}

/// Rebuilds a sanitised LDOCE6 fragment into L1 markup.
pub(crate) struct Builder<'a> {
    ctx: &'a ParseContext,
    out: String,
    frames: Vec<Frame>,
    inline: Vec<Inline>,
    /// Span depths of popup triggers whose label text must be dropped.
    mute_spans: Vec<usize>,
    head_open: bool,
    head_done: bool,
    /// 1-based index of the homograph entry currently being built.
    homograph: usize,
    /// True when the record holds more than one entry (a homograph group).
    homograph_group: bool,
    /// Pos-block ids already emitted, so homographs never reuse one.
    pos_ids: Vec<String>,
    pos_slug: String,
    scope_id: String,
    sense_seq: u32,
    sense_n: String,
    pv_seq: usize,
    phon_index: usize,
    pending_anchor: Option<String>,
    anchors: Vec<(String, String)>,
    toc_open: bool,
    toc_capture: bool,
    toc: String,
    refs: String,
    /// Byte offset just past the entry head, where the reference toolbar goes.
    refbar_pos: Option<usize>,
    /// Popup blocks collected so far; popups can arrive in several bursts.
    ref_blocks: Vec<RefBlock>,
    /// Buffered markup that carried no popup header.
    loose: String,
    /// Span of the toolbar already written into `out`.
    refbar_at: Option<(usize, usize)>,
    capturing: bool,
    ref_summary: bool,
    span_depth: usize,
    ref_box_depth: Option<usize>,
    mute_images: usize,
    def_depth: usize,
    adapted: bool,
}

impl<'a> Builder<'a> {
    pub(crate) fn new(ctx: &'a ParseContext) -> Self {
        Self {
            ctx,
            out: String::with_capacity(4096),
            frames: Vec::new(),
            inline: Vec::new(),
            mute_spans: Vec::new(),
            head_open: false,
            head_done: false,
            homograph: 0,
            homograph_group: false,
            pos_ids: Vec::new(),
            pos_slug: String::new(),
            scope_id: String::new(),
            sense_seq: 0,
            sense_n: String::new(),
            pv_seq: 0,
            phon_index: 0,
            pending_anchor: None,
            anchors: Vec::new(),
            toc_open: false,
            toc_capture: false,
            toc: String::new(),
            refs: String::new(),
            refbar_pos: None,
            ref_blocks: Vec::new(),
            loose: String::new(),
            refbar_at: None,
            capturing: false,
            span_depth: 0,
            ref_box_depth: None,
            mute_images: 0,
            def_depth: 0,
            ref_summary: false,
            adapted: false,
        }
    }

    /// Run the whole fragment through the rewriter.
    pub(crate) fn build(&mut self, html: &str) {
        // Every root entry opens with its own `div.chwd` bar, so more than one
        // bar means this record holds a homograph group.
        self.homograph_group = html.matches("class=\"chwd\"").count() > 1;
        let tokens = tokens::tokenize(html);
        self.open_entry();
        for (index, tok) in tokens.iter().enumerate() {
            match tok {
                Tok::Text(text) => self.emit(text),
                Tok::Start { name, attrs, void } => {
                    self.start(name, attrs, *void, &tokens, index);
                }
                Tok::End { name } => self.end(name),
            }
        }
        self.end_capture();
        self.close_toc();
        self.flush_deferred();
        self.close_from(0);
    }

    /// Move reference blocks and the entry menu into one toolbar under the
    /// headword, so the main senses are not interleaved with popup content.
    fn flush_deferred(&mut self) {
        let refs = std::mem::take(&mut self.refs);
        let toc = std::mem::take(&mut self.toc);
        collect_blocks(&mut self.ref_blocks, &mut self.loose, &toc, Some("toc"));
        collect_blocks(&mut self.ref_blocks, &mut self.loose, &refs, None);
        self.split_verb_tables();
        if self.ref_blocks.is_empty() && self.loose.trim().is_empty() {
            return;
        }
        let items = build_refbar_items(&self.ref_blocks, &self.loose);
        let bar = format!("<nav class=\"odc-refbar\">{items}</nav>");

        if let Some((start, end)) = self.refbar_at {
            self.out.replace_range(start..end, &bar);
            self.refbar_at = Some((start, start + bar.len()));
        } else {
            let pos = self
                .refbar_pos
                .unwrap_or(self.out.len())
                .min(self.out.len());
            self.out.insert_str(pos, &bar);
            self.refbar_at = Some((pos, pos + bar.len()));
        }
    }

    /// Give every captured `div.verbtable` its own toolbar entry.
    ///
    /// The source nests the conjugation table after the word-origin popup; it
    /// belongs to the original "Verb Table" button, not to the etymology panel.
    fn split_verb_tables(&mut self) {
        let mut blocks = Vec::with_capacity(self.ref_blocks.len() + 1);
        for mut block in std::mem::take(&mut self.ref_blocks) {
            let (tables, rest) = take_verb_tables(&block.html);
            block.html = rest;
            if !block.html.trim().is_empty() || !block.title.is_empty() {
                blocks.push(block);
            }
            for table in tables {
                blocks.push(RefBlock {
                    kind: String::from("verb-table"),
                    source: String::from("entry"),
                    title: String::new(),
                    html: table,
                });
            }
        }
        let (tables, rest) = take_verb_tables(&self.loose);
        self.loose = rest;
        for table in tables {
            blocks.push(RefBlock {
                kind: String::from("verb-table"),
                source: String::from("entry"),
                title: String::new(),
                html: table,
            });
        }
        self.ref_blocks = blocks;
    }

    /// Final markup with cross-reference anchors resolved.
    pub(crate) fn finish(mut self) -> String {
        for (orig, id) in std::mem::take(&mut self.anchors) {
            let from = format!("href=\"#{}\"", escape_attr(&orig));
            let to = format!("href=\"#{}\"", escape_attr(&id));
            self.out = self.out.replace(&from, &to);
            let from = format!("data-odc-target=\"{}\"", escape_attr(&orig));
            let to = format!("data-odc-target=\"{}\"", escape_attr(&id));
            self.out = self.out.replace(&from, &to);
        }
        self.out
    }

    fn open_entry(&mut self) {
        self.homograph += 1;
        let homograph = if self.homograph_group {
            format!(" data-odc-homograph=\"{}\"", self.homograph)
        } else {
            String::new()
        };
        let start = format!(
            "<section class=\"odc-entry\" data-odc-dict=\"{}\" data-odc-key=\"{}\"{homograph} data-odc-lang=\"{}\">",
            escape_attr(&self.ctx.dict),
            escape_attr(&self.ctx.key),
            escape_attr(&self.ctx.lang)
        );
        self.open(0, start, "</section>");
    }

    /// Close the entry being built and start the next homograph as its
    /// sibling, so every entry keeps its own head, toolbar and senses.
    fn next_entry(&mut self) {
        self.end_capture();
        self.close_toc();
        self.flush_deferred();
        self.close_from(0);
        self.ref_blocks.clear();
        self.loose.clear();
        self.reset_entry();
        self.open_entry();
    }

    /// Per-entry state that must not leak into the next homograph.
    fn reset_entry(&mut self) {
        self.head_open = false;
        self.head_done = false;
        self.phon_index = 0;
        self.pos_slug.clear();
        self.scope_id.clear();
        self.sense_seq = 0;
        self.sense_n.clear();
        self.pending_anchor = None;
        self.toc_open = false;
        self.toc_capture = false;
        self.toc.clear();
        self.refs.clear();
        self.refbar_pos = None;
        self.refbar_at = None;
        self.capturing = false;
        self.span_depth = 0;
        self.ref_box_depth = None;
        self.def_depth = 0;
        self.ref_summary = false;
        self.mute_spans.clear();
    }

    fn emit(&mut self, text: &str) {
        if !self.mute_spans.is_empty() {
            return;
        }
        if self.capturing {
            self.refs.push_str(text);
        } else if self.toc_capture {
            self.toc.push_str(text);
        } else {
            self.out.push_str(text);
        }
    }

    /// Leave the reference buffer.
    ///
    /// Reference blocks are held back so the block skeleton stays flat
    /// (entry > refs > ref) even though the source nests them inside the head.
    /// They are spliced back at the position of the first popup, so the text
    /// order of the fragment never changes.
    fn end_capture(&mut self) {
        self.close_tag("atlink");
        if !self.capturing {
            return;
        }
        while self
            .frames
            .last()
            .is_some_and(|frame| matches!(frame.tag, "ref" | "refs" | "atlink"))
        {
            self.close_top();
        }
        self.capturing = false;
    }

    fn open(&mut self, level: u8, start: String, close: &str) {
        self.emit(&start);
        self.frames.push(Frame {
            level,
            tag: "block",
            close: String::from(close),
            inline_base: self.inline.len(),
        });
    }

    fn open_tagged(&mut self, level: u8, tag: &'static str, start: String, close: &str) {
        self.emit(&start);
        self.frames.push(Frame {
            level,
            tag,
            close: String::from(close),
            inline_base: self.inline.len(),
        });
    }

    fn close_top(&mut self) {
        let Some(frame) = self.frames.pop() else {
            return;
        };
        while self.inline.len() > frame.inline_base {
            let inline = self.inline.pop().expect("inline frame");
            self.emit("</");
            self.emit(&inline.out);
            self.emit(">");
        }
        self.emit(&frame.close);
    }

    fn close_from(&mut self, level: u8) {
        if level <= 3 {
            self.ref_box_depth = None;
        }
        while self.frames.last().is_some_and(|frame| frame.level >= level) {
            self.close_top();
        }
    }

    fn close_tag(&mut self, tag: &str) {
        while let Some(frame) = self.frames.last() {
            if frame.tag == tag {
                self.close_top();
                return;
            }
            if frame.level == 0 {
                return;
            }
            self.close_top();
        }
    }

    fn has_open(&self, tag: &str) -> bool {
        self.frames.iter().any(|frame| frame.tag == tag)
    }

    fn in_ref(&self) -> bool {
        self.has_open("ref") || self.has_open("toc") || self.has_open("atlink")
    }

    fn in_pv(&self) -> bool {
        self.has_open("pv")
    }

    fn ensure_head(&mut self) {
        if self.head_done || self.in_pv() {
            return;
        }
        if !self.head_open {
            self.close_from(1);
            self.open_tagged(
                1,
                "head",
                String::from("<header class=\"odc-head\">"),
                "</header>",
            );
            self.head_open = true;
        }
    }

    fn close_head(&mut self) {
        if self.head_open {
            self.close_tag("head");
            self.head_open = false;
            self.head_done = true;
            if self.refbar_pos.is_none() {
                self.refbar_pos = Some(self.out.len());
            }
        }
    }

    fn inline_open(&mut self, start: &str, src: &str, out: &str) {
        self.inline_open_role(start, src, out, "");
    }

    fn inline_open_role(&mut self, start: &str, src: &str, out: &str, role: &'static str) {
        self.emit(start);
        self.inline.push(Inline {
            src: String::from(src),
            out: String::from(out),
            role,
        });
    }

    fn passthrough(&mut self, name: &str, attrs: &[(String, String)], void: bool) {
        let replaced = match name {
            "div" | "ul" | "li" | "table" | "tbody" | "tr" | "td" | "th" | "p" | "i" | "b"
            | "em" | "strong" | "sup" | "sub" | "br" | "span" => name,
            _ => "span",
        };
        let orig = tokens::attr(attrs, "class").unwrap_or("");
        let mut start = format!("<{replaced}");
        if !orig.is_empty() {
            start.push_str(&format!(" data-odc-orig-class=\"{}\"", escape_attr(orig)));
        }
        start.push('>');
        if void || replaced == "br" {
            self.emit(&start);
        } else {
            self.inline_open(&start, name, replaced);
        }
    }

    fn end(&mut self, name: &str) {
        if name == "span" {
            self.span_depth = self.span_depth.saturating_sub(1);
            if self.mute_spans.last() == Some(&(self.span_depth + 1)) {
                self.mute_spans.pop();
            }
            if self.ref_box_depth == Some(self.span_depth + 1) {
                self.ref_box_depth = None;
                if self.has_open("ref") {
                    self.close_from(3);
                }
            }
        }
        if name == "a" && self.mute_images > 0 {
            self.mute_images -= 1;
        }
        if let Some(last) = self.inline.last() {
            if last.src == name {
                let inline = self.inline.pop().expect("inline frame");
                self.emit("</");
                self.emit(&inline.out);
                self.emit(">");
                if inline.role == "def" {
                    self.def_depth = self.def_depth.saturating_sub(1);
                }
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn start(
        &mut self,
        name: &str,
        attrs: &[(String, String)],
        void: bool,
        tokens: &[Tok],
        index: usize,
    ) {
        let has = |class: &str| tokens::has_class(attrs, class);

        if name == "span" {
            self.span_depth += 1;
        }
        if name == "span" && has("popup-button") {
            // Popup triggers are UI chrome, never entry text: drop the label.
            self.mute_spans.push(self.span_depth);
            return;
        }
        if matches!(
            name,
            "link" | "script" | "style" | "meta" | "head" | "html" | "body"
        ) {
            return;
        }
        if name == "a" {
            self.anchor(attrs, void);
            return;
        }
        if name == "img" {
            self.image(attrs);
            return;
        }
        if name != "span" && name != "div" {
            match name {
                "en" | "signen" | "sign" | "cnt" | "collt" => {
                    self.inline_open("<span>", name, "span");
                }
                "tran" => {
                    if self.def_depth > 0 {
                        self.inline_open(
                            "<span class=\"odc-def-trans\" data-odc-lang=\"zh\">",
                            name,
                            "span",
                        );
                    } else {
                        self.inline_open("<span>", name, "span");
                    }
                }
                "example" => {
                    self.inline_open(
                        "<span class=\"odc-ex-trans\" data-odc-lang=\"zh\">",
                        name,
                        "span",
                    );
                }
                "exaen" => {
                    self.inline_open("<span class=\"odc-ex-text\">", name, "span");
                }
                "deft" | "exat" | "l6" | "sigt" | "atl" => {}
                _ => self.passthrough(name, attrs, void),
            }
            return;
        }
        if name == "div" {
            if has("chwd") {
                // `div.chwd` opens every root entry. The first bar belongs to
                // the entry already open; later bars start a sibling homograph.
                if self.head_done {
                    self.next_entry();
                } else {
                    self.end_capture();
                    self.close_toc();
                    self.flush_deferred();
                    self.close_from(1);
                }
            }
            if has("at-link") {
                self.close_head();
                self.close_from(2);
                self.pending_anchor = None;
                self.open_tagged(2, "atlink", String::new(), "");
                return;
            }
            if tokens::attr(attrs, "class").is_none() {
                return;
            }
            self.passthrough(name, attrs, void);
            return;
        }

        if has("entry") {
            // `span.entry` also wraps popup payloads (word origin, verb tables,
            // the entry menu), so it never decides an entry boundary by itself:
            // homographs are separated by their `div.chwd` bars.
            if self.in_ref() || self.capturing {
                return;
            }
            self.adapted = true;
            return;
        }
        if has("entryhead") || has("buttons") {
            return;
        }
        if has("hwd") {
            self.ensure_head();
            self.inline_open("<h1 class=\"odc-hw\">", "span", "h1");
            self.adapted = true;
            return;
        }
        if has("phrvbhwd") {
            self.inline_open("<h3 class=\"odc-pv-hw\">", "span", "h3");
            self.adapted = true;
            return;
        }
        if has("hyphenation") {
            self.ensure_head();
            self.inline_open(
                "<span class=\"odc-hw-alt\" data-odc-kind=\"hyphenation\">",
                "span",
                "span",
            );
            return;
        }
        if has("homnum") {
            self.ensure_head();
            self.inline_open(
                "<span class=\"odc-hw-alt\" data-odc-kind=\"homograph\">",
                "span",
                "span",
            );
            return;
        }
        if has("proncodes") {
            self.ensure_head();
            self.phon_index += 1;
            let variant = if self.phon_index == 1 { "uk" } else { "us" };
            self.inline_open(
                &format!("<span class=\"odc-phon\" data-odc-variant=\"{variant}\">"),
                "span",
                "span",
            );
            return;
        }
        if has("pron") {
            self.inline_open("<span>", "span", "span");
            return;
        }
        if has("freq") || has("level") {
            self.ensure_head();
            let orig = tokens::attr(attrs, "class").unwrap_or("");
            self.inline_open(
                &format!("<span data-odc-orig-class=\"{}\">", escape_attr(orig)),
                "span",
                "span",
            );
            return;
        }
        if has("pos") {
            if self.capturing {
                self.passthrough(name, attrs, void);
                return;
            }
            if self.in_pv() {
                self.inline_open(
                    "<span class=\"odc-label\" data-odc-kind=\"pos\">",
                    "span",
                    "span",
                );
                return;
            }
            self.end_capture();
            self.close_toc();
            self.close_head();
            self.close_from(2);
            let label = inline_text(tokens, index);
            self.pos_slug = pos_slug(&label);
            let base = format!("{}-{}", entry_slug(&self.ctx.key), self.pos_slug);
            let id = if self.pos_ids.contains(&base) {
                format!("{base}-{}", self.homograph)
            } else {
                base
            };
            self.pos_ids.push(id.clone());
            self.scope_id = id.clone();
            self.sense_seq = 0;
            self.sense_n.clear();
            self.open(
                2,
                format!(
                    "<section class=\"odc-pos-block\" data-odc-pos=\"{}\" id=\"{}\">",
                    escape_attr(&label),
                    escape_attr(&id)
                ),
                "</section>",
            );
            self.inline_open("<h2 class=\"odc-pos\">", "span", "h2");
            self.adapted = true;
            return;
        }
        if has("inflections") {
            self.ensure_head();
            let kind = text_after_class(tokens, index, "infllab")
                .map_or("inflection", |text| inflection_kind(&text));
            self.inline_open(
                &format!("<span class=\"odc-inflection\" data-odc-kind=\"{kind}\">"),
                "span",
                "span",
            );
            return;
        }
        if has("popheader") {
            self.ref_header(attrs, tokens, index);
            return;
        }
        if has("entrymenu") {
            self.open_toc();
            return;
        }
        if has("tail") {
            return;
        }
        if has("thesbox") || has("collobox") || has("grambox") {
            let kind = if has("thesbox") {
                "thesaurus"
            } else if has("collobox") {
                "collocation"
            } else {
                "grammar"
            };
            self.open_inline_ref(kind);
            return;
        }
        if has("heading") && self.has_open("ref") && !self.ref_summary {
            self.ref_summary = true;
            self.inline_open("<summary class=\"odc-ref-title\">", "span", "summary");
            return;
        }
        if has("colloboxheading") {
            self.ref_summary = true;
            self.inline_open("<summary class=\"odc-ref-title\">", "span", "summary");
            return;
        }
        if has("subsense") {
            self.open_subsense(tokens, index);
            return;
        }
        if has("sensenum") {
            if self.in_ref() {
                self.passthrough(name, attrs, void);
                return;
            }
            self.inline_open("<span class=\"odc-sense-number\">", "span", "span");
            return;
        }
        if has("sense") {
            self.open_sense(tokens, index);
            return;
        }
        if has("signpost") {
            if self.in_ref() {
                self.passthrough(name, attrs, void);
                return;
            }
            self.inline_open("<span class=\"odc-signpost\">", "span", "span");
            return;
        }
        if has("gram") || has("propform") {
            self.inline_open("<span class=\"odc-grammar\">", "span", "span");
            return;
        }
        if has("gramexa") {
            return;
        }
        if has("def") {
            if self.capturing {
                self.passthrough(name, attrs, void);
                return;
            }
            self.def_depth += 1;
            self.inline_open_role("<span class=\"odc-def\">", "span", "span", "def");
            return;
        }
        if has("registerlab") {
            self.inline_open(
                "<span class=\"odc-label\" data-odc-kind=\"register\">",
                "span",
                "span",
            );
            return;
        }
        if has("colloinexa") || has("colloc") || has("colloexa") || has("collo") {
            self.inline_open("<span class=\"odc-collocation\">", "span", "span");
            return;
        }
        if has("phrvbentry") {
            self.open_pv(tokens, index);
            return;
        }
        if has("example") {
            self.close_from(8);
            if !self.has_open("examples") {
                self.open_tagged(
                    7,
                    "examples",
                    String::from("<ul class=\"odc-examples\">"),
                    "</ul>",
                );
            }
            self.open(8, String::from("<li class=\"odc-example\">"), "</li>");
            self.adapted = true;
            return;
        }
        if has("phrase") {
            self.inline_open("<span class=\"odc-phrase-index\">", "span", "span");
            return;
        }
        if has("phrasetext") || has("object") || has("lexunit") || has("gloss") {
            self.inline_open("<span>", "span", "span");
            return;
        }
        self.passthrough(name, attrs, void);
    }

    fn close_toc(&mut self) {
        if self.toc_open {
            self.close_tag("toc");
            self.toc_open = false;
            self.toc_capture = false;
        }
    }

    /// Inline reference box (`thesbox`, `collobox`, `grambox`) that the source
    /// nests inside the entry body rather than in a popup.
    fn open_inline_ref(&mut self, kind: &'static str) {
        self.capturing = true;
        if self.has_open("ref") {
            return;
        }
        self.ref_summary = false;
        self.ref_box_depth = Some(self.span_depth);
        if !self.has_open("refs") {
            self.open_tagged(
                2,
                "refs",
                String::from("<section class=\"odc-refs\">"),
                "</section>",
            );
        }
        self.open_tagged(
            3,
            "ref",
            format!(
                "<details class=\"odc-ref\" data-odc-kind=\"{kind}\" data-odc-source=\"entry\">"
            ),
            "</details>",
        );
    }

    fn open_toc(&mut self) {
        if self.toc_open {
            return;
        }
        self.close_tag("atlink");
        self.pending_anchor = None;
        self.close_head();
        self.close_from(2);
        self.toc_capture = true;
        self.open_tagged(
            2,
            "toc",
            String::from("<details class=\"odc-toc\">"),
            "</details>",
        );
        self.toc_open = true;
    }

    fn open_sense(&mut self, tokens: &[Tok], index: usize) {
        self.end_capture();
        self.close_toc();
        self.close_from(5);
        let label = text_after_class(tokens, index, "sensenum").unwrap_or_default();
        self.sense_seq += 1;
        let n = numeric_prefix(&label).unwrap_or_else(|| self.sense_seq.to_string());
        self.sense_n.clone_from(&n);
        let scope = if self.scope_id.is_empty() {
            entry_slug(&self.ctx.key)
        } else {
            self.scope_id.clone()
        };
        let id = format!("{scope}-s{n}");
        let anchor = self.pending_anchor.take();
        let mut start = format!(
            "<div class=\"odc-sense\" data-odc-n=\"{}\" data-odc-label=\"{}\" data-odc-scope=\"{}\" id=\"{}\"",
            escape_attr(&n),
            escape_attr(&label),
            escape_attr(&scope),
            escape_attr(&id)
        );
        if let Some(anchor) = anchor {
            start.push_str(&format!(
                " data-odc-orig-anchor=\"{}\"",
                escape_attr(&anchor)
            ));
            self.anchors.push((anchor, id.clone()));
        }
        start.push('>');
        self.open(5, start, "</div>");
        self.adapted = true;
    }

    fn open_subsense(&mut self, tokens: &[Tok], index: usize) {
        self.end_capture();
        self.close_from(6);
        let label = text_after_class(tokens, index, "sensenum").unwrap_or_default();
        let letters: String = label
            .chars()
            .filter(|ch| ch.is_ascii_alphabetic())
            .collect::<String>()
            .to_ascii_lowercase();
        let letter = if letters.is_empty() {
            String::from("a")
        } else {
            letters
        };
        let scope = if self.scope_id.is_empty() {
            entry_slug(&self.ctx.key)
        } else {
            self.scope_id.clone()
        };
        let sense_n = if self.sense_n.is_empty() {
            self.sense_seq.to_string()
        } else {
            self.sense_n.clone()
        };
        let id = format!("{scope}-s{sense_n}{letter}");
        let mut start = format!(
            "<div class=\"odc-subsense\" data-odc-n=\"{}\" data-odc-label=\"{}\" data-odc-scope=\"{}\" id=\"{}\"",
            escape_attr(&letter),
            escape_attr(&label),
            escape_attr(&scope),
            escape_attr(&id)
        );
        if let Some(anchor) = self.pending_anchor.take() {
            start.push_str(&format!(
                " data-odc-orig-anchor=\"{}\"",
                escape_attr(&anchor)
            ));
            self.anchors.push((anchor, id.clone()));
        }
        start.push('>');
        self.open(6, start, "</div>");
        self.adapted = true;
    }

    fn open_pv(&mut self, tokens: &[Tok], index: usize) {
        self.end_capture();
        self.close_toc();
        self.close_head();
        self.close_from(4);
        if !self.has_open("pvs") {
            self.open_tagged(
                3,
                "pvs",
                String::from("<section class=\"odc-pvs\">"),
                "</section>",
            );
        }
        self.pv_seq += 1;
        let pos = if self.pos_slug.is_empty() {
            String::from("x")
        } else {
            self.pos_slug.clone()
        };
        let id = format!("{}-{}-pv{}", entry_slug(&self.ctx.key), pos, self.pv_seq);
        let lemma = tokens
            .get(index)
            .map(|_| element_text(tokens, index, "phrvbhwd"))
            .unwrap_or_default();
        self.scope_id = id.clone();
        self.sense_n.clear();
        self.open_tagged(
            4,
            "pv",
            format!(
                "<article class=\"odc-pv\" data-odc-lemma=\"{}\" id=\"{}\">",
                escape_attr(&lemma),
                escape_attr(&id)
            ),
            "</article>",
        );
        self.adapted = true;
    }

    fn ref_header(&mut self, attrs: &[(String, String)], tokens: &[Tok], index: usize) {
        let pop = tokens::attr(attrs, "class")
            .unwrap_or("")
            .split_whitespace()
            .find(|token| token.starts_with("pop") && *token != "popheader")
            .unwrap_or("");
        let title = inline_text(tokens, index);
        let (kind, source) = ref_kind(pop, &title);
        self.close_tag("atlink");
        self.pending_anchor = None;
        if kind == "toc" {
            self.open_toc();
        } else {
            self.close_head();
            self.close_toc();
            self.close_from(3);
            self.capturing = true;
            if !self.has_open("refs") {
                self.open_tagged(
                    2,
                    "refs",
                    String::from("<section class=\"odc-refs\">"),
                    "</section>",
                );
            }
            self.open_tagged(
                3,
                "ref",
                format!(
                    "<details class=\"odc-ref\" data-odc-kind=\"{kind}\" data-odc-source=\"{source}\">"
                ),
                "</details>",
            );
        }
        self.ref_summary = true;
        self.inline_open("<summary class=\"odc-ref-title\">", "span", "summary");
    }

    fn anchor(&mut self, attrs: &[(String, String)], void: bool) {
        if let Some(href) = tokens::attr(attrs, "href") {
            if href.starts_with("sound://") {
                self.audio(href);
                return;
            }
            if href.starts_with("entry://") || href.starts_with('#') {
                let target = href
                    .strip_prefix("entry://")
                    .unwrap_or(href)
                    .trim_start_matches('#');
                self.inline_open(
                    &format!(
                        "<a class=\"odc-crossref\" href=\"#{}\" data-odc-target=\"{}\" data-odc-orig-target=\"{}\">",
                        escape_attr(target),
                        escape_attr(target),
                        escape_attr(href)
                    ),
                    "a",
                    "a",
                );
                return;
            }
        }
        if let Some(anchor) = tokens::attr(attrs, "name").or_else(|| tokens::attr(attrs, "id")) {
            self.pending_anchor = Some(String::from(anchor));
            return;
        }
        self.passthrough("a", attrs, void);
    }

    fn audio(&mut self, href: &str) {
        let variant = if href.contains("/bre/") {
            "uk"
        } else if href.contains("/ame/") {
            "us"
        } else {
            ""
        };
        let mut start = String::from("<button class=\"odc-audio\"");
        if let Some(key) = resource_key(href) {
            start.push_str(&format!(" data-odc-src=\"{}\"", escape_attr(&key)));
        }
        start.push_str(&format!(" data-odc-orig-src=\"{}\"", escape_attr(href)));
        if !variant.is_empty() {
            start.push_str(&format!(" data-odc-variant=\"{variant}\""));
        }
        start.push_str("></button>");
        self.emit(&start);
        self.mute_images += 1;
    }

    fn image(&mut self, attrs: &[(String, String)]) {
        if self.mute_images > 0 {
            return;
        }
        let Some(src) = tokens::attr(attrs, "src") else {
            return;
        };
        if src.contains("spkr") {
            return;
        }
        let mut start = String::from("<img class=\"odc-img\"");
        if let Some(key) = resource_key(src) {
            start.push_str(&format!(" src=\"{}\"", escape_attr(&key)));
        } else {
            start.push_str(" data-odc-missing=\"1\"");
        }
        start.push_str(&format!(
            " data-odc-orig-src=\"{}\" alt=\"\">",
            escape_attr(src)
        ));
        self.emit(&start);
    }
}

/// Map an LDOCE resource reference to an `odc-resource://` key (spec 3.5).
///
/// The mapping only normalises the string; it never touches the archive.
#[must_use]
pub fn resource_key(reference: &str) -> Option<String> {
    let trimmed = reference.trim();
    let key = if let Some(rest) = trimmed.strip_prefix("sound://") {
        rest.to_string()
    } else if trimmed.starts_with("img/") || trimmed.starts_with("./img/") {
        trimmed.trim_start_matches("./").to_string()
    } else {
        trimmed.to_string()
    };
    let key = key.replace('\\', "/");
    let key = key.trim_start_matches('/');
    let key = key.strip_prefix("./").unwrap_or(key);
    if key.is_empty() {
        None
    } else {
        Some(format!("odc-resource://{key}"))
    }
}

/// Leading integer of a sense label (`12` of `12a`); `None` for letters.
fn numeric_prefix(label: &str) -> Option<String> {
    let digits: String = label.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        None
    } else {
        Some(digits)
    }
}

/// Map an `infllab` label to the `data-odc-kind` value of an inflection.
fn inflection_kind(label: &str) -> &'static str {
    let lower = label.to_ascii_lowercase();
    if lower.contains("past tense and past participle") {
        "past"
    } else if lower.contains("past participle") {
        "past-participle"
    } else if lower.contains("past tense") {
        "past"
    } else if lower.contains("present participle") {
        "present-participle"
    } else if lower.contains("plural") {
        "plural"
    } else if lower.contains("comparative") {
        "comparative"
    } else if lower.contains("superlative") {
        "superlative"
    } else {
        "inflection"
    }
}

/// Reference block kind and source from a `popheader` class and title.
fn ref_kind(pop: &str, title: &str) -> (&'static str, &'static str) {
    let source = if title.contains("CORPUS") {
        "corpus"
    } else if title.contains("OTHER") {
        "other-entries"
    } else {
        "entry"
    };
    match pop {
        "pope_menu" => ("toc", "entry"),
        "popetym" => ("word-origin", "entry"),
        "popexa" => ("examples-other", "other-entries"),
        "popthes" => ("thesaurus", source),
        "popcollo" => ("collocation", source),
        "popphrase" => ("phrase-index", source),
        "popwf" => ("word-family", "entry"),
        _ => ("other", source),
    }
}

/// Drop UI chrome from markup before comparing words.
///
/// Toolbar labels and popup trigger labels are not entry text, and the popup
/// buttons themselves are removed from L1.
fn strip_chrome(html: &str) -> String {
    let without_l1 = strip_elements(html, "data-odc-orig-class=\"popup-button\"", "span");
    let without_source = strip_elements(&without_l1, "class=\"popup-button\"", "span");
    strip_refbar_labels(&without_source)
}

/// Remove elements whose start tag carries `marker`, closing tag `tag`.
fn strip_elements(html: &str, marker: &str, tag: &str) -> String {
    let open_mark = format!("<{tag}");
    let close_mark = format!("</{tag}>");
    let mut out = String::with_capacity(html.len());
    let mut cursor = 0;
    while let Some(at) = html[cursor..].find(marker) {
        let marker_pos = cursor + at;
        let Some(open) = html[..marker_pos].rfind(&open_mark) else {
            break;
        };
        if open < cursor {
            break;
        }
        if !html[open..].contains('>') {
            break;
        }
        let mut depth = 0usize;
        let mut i = open;
        let mut close = None;
        while i < html.len() {
            let next_open = html[i..].find(&open_mark).map(|end| i + end);
            let next_close = html[i..].find(&close_mark).map(|end| i + end);
            match (next_open, next_close) {
                (Some(start), Some(end)) if start < end => {
                    depth += 1;
                    i = start + open_mark.len();
                }
                (_, Some(end)) => {
                    depth -= 1;
                    i = end + close_mark.len();
                    if depth == 0 {
                        close = Some(i);
                        break;
                    }
                }
                _ => break,
            }
        }
        let Some(after) = close else {
            break;
        };
        out.push_str(&html[cursor..open]);
        cursor = after;
    }
    out.push_str(&html[cursor..]);
    out
}

/// Markup without the toolbar button labels.
///
/// The labels restate the popup names for the toolbar; they are UI chrome, not
/// entry text, so the text-drift guard compares everything but those labels.
fn strip_refbar_labels(html: &str) -> String {
    const OPEN: &str = "<summary class=\"odc-refbar__label\">";
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        match after.find("</summary>") {
            Some(close) => rest = &after[close + "</summary>".len()..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// One buffered popup block, parsed back out of the captured reference HTML.
struct RefBlock {
    kind: String,
    source: String,
    title: String,
    html: String,
}

/// A toolbar entry: every buffered block of one kind, merged.
struct RefGroup {
    kind: String,
    label: String,
    /// Indices into the block list, in source order.
    blocks: Vec<usize>,
    /// Whether this group carries the loose markup instead of blocks.
    loose: bool,
}

/// Toolbar order for the kinds the source dictionary promotes to buttons.
const REF_ORDER: [&str; 6] = [
    "toc",
    "word-origin",
    "verb-table",
    "examples-other",
    "collocation",
    "phrase-index",
];

/// Render the toolbar items: one collapsed button per popup kind.
///
/// The block text is kept verbatim: only the details wrappers and their
/// summaries are replaced, so the toolbar never shifts the words of the entry.
fn build_refbar_items(blocks: &[RefBlock], loose: &str) -> String {
    let mut groups: Vec<RefGroup> = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        let label = ref_label(&block.kind, &plain_text(&block.title));
        match groups.iter_mut().find(|group| group.kind == block.kind) {
            Some(group) => group.blocks.push(index),
            None => groups.push(RefGroup {
                kind: block.kind.clone(),
                label,
                blocks: vec![index],
                loose: false,
            }),
        }
    }
    if !loose.trim().is_empty() {
        // Buffered markup with no popup header still has to reach the page.
        groups.push(RefGroup {
            kind: String::from("other"),
            label: String::from("Other"),
            blocks: Vec::new(),
            loose: true,
        });
    }
    groups.sort_by_key(|group| ref_rank(&group.kind));

    let mut html = String::new();
    for group in groups {
        html.push_str("<details class=\"odc-refbar__item\" data-odc-kind=\"");
        html.push_str(&escape_attr(&group.kind));
        html.push_str("\"><summary class=\"odc-refbar__label\">");
        html.push_str(&escape_attr(&group.label));
        html.push_str("</summary><div class=\"odc-ref-panel\">");
        if group.loose {
            html.push_str(loose.trim());
        }
        for index in group.blocks {
            let block = &blocks[index];
            html.push_str("<div class=\"odc-ref-block\" data-odc-source=\"");
            html.push_str(&escape_attr(&block.source));
            html.push_str("\">");
            if !block.title.is_empty() {
                html.push_str("<p class=\"odc-ref-panel__title\">");
                html.push_str(&block.title);
                html.push_str("</p>");
            }
            html.push_str(block.html.trim());
            html.push_str("</div>");
        }
        html.push_str("</div></details>");
    }
    html
}

/// Collect details blocks, appending loose markup to the block it followed.
fn collect_blocks(
    blocks: &mut Vec<RefBlock>,
    loose: &mut String,
    html: &str,
    toc_kind: Option<&str>,
) {
    for part in details_parts(html) {
        match part {
            BlockPart::Block { tag, inner } => {
                let (title, body) = split_summary(&inner);
                let (kind, source) = match toc_kind {
                    Some(kind) => (String::from(kind), String::from("entry")),
                    None => (
                        String::from(tag_attr(&tag, "data-odc-kind").unwrap_or("other")),
                        String::from(tag_attr(&tag, "data-odc-source").unwrap_or("entry")),
                    ),
                };
                blocks.push(RefBlock {
                    kind,
                    source,
                    title,
                    html: String::from(body),
                });
            }
            BlockPart::Loose(markup) => {
                let rest = strip_refs_wrapper(&markup);
                if rest.trim().is_empty() {
                    continue;
                }
                match blocks.last_mut() {
                    Some(block) => block.html.push_str(rest.trim()),
                    None => loose.push_str(rest.trim()),
                }
            }
        }
    }
}

/// Class marker the adapter writes for LDOCE `div.verbtable`.
const VERB_TABLE_CLASS: &str = "data-odc-orig-class=\"verbtable\"";

/// Pull every `div.verbtable` out of captured markup: `(tables, rest)`.
fn take_verb_tables(html: &str) -> (Vec<String>, String) {
    let mut tables = Vec::new();
    let mut rest = String::with_capacity(html.len());
    let mut cursor = 0;
    while let Some(found) = html[cursor..].find(VERB_TABLE_CLASS) {
        let marker = cursor + found;
        let Some(open) = html[..marker].rfind("<div") else {
            break;
        };
        let Some(close) = closing_div(html, open) else {
            break;
        };
        rest.push_str(&html[cursor..open]);
        tables.push(String::from(html[open..close].trim()));
        cursor = close;
    }
    rest.push_str(&html[cursor..]);
    (tables, rest)
}

/// End offset (just past `</div>`) of the div that starts at `open`.
fn closing_div(html: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = open;
    while i < html.len() {
        let next_open = html[i..].find("<div").map(|at| i + at);
        let next_close = html[i..].find("</div>").map(|at| i + at);
        match (next_open, next_close) {
            (Some(start), Some(end)) if start < end => {
                depth += 1;
                i = start + "<div".len();
            }
            (_, Some(end)) => {
                depth -= 1;
                i = end + "</div>".len();
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => return None,
        }
    }
    None
}

/// Button label for a popup kind; unknown kinds fall back to the block title.
fn ref_label(kind: &str, title: &str) -> String {
    match kind {
        "toc" => String::from("Entry menu"),
        "word-origin" => String::from("Word Origin"),
        "verb-table" => String::from("Verb Table"),
        "examples-other" => String::from("Examples"),
        "collocation" => String::from("Collocations"),
        "phrase-index" => String::from("Phrases"),
        _ if !title.is_empty() => String::from(title),
        _ => String::from(kind),
    }
}

/// Position of a kind in the toolbar; unknown kinds keep their source order.
fn ref_rank(kind: &str) -> usize {
    REF_ORDER
        .iter()
        .position(|known| *known == kind)
        .unwrap_or(REF_ORDER.len())
}

/// Value of a data attribute in a raw start tag.
fn tag_attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('\"')? + start;
    Some(&tag[start..end])
}

/// One slice of a buffered fragment: a details block or loose markup.
enum BlockPart {
    Block { tag: String, inner: String },
    Loose(String),
}

/// Split the buffered HTML into details blocks and the markup between them.
fn details_parts(html: &str) -> Vec<BlockPart> {
    let mut parts = Vec::new();
    let mut cursor = 0;
    while let Some(rel) = html[cursor..].find("<details") {
        let start = cursor + rel;
        let Some(open_rel) = html[start..].find('>') else {
            break;
        };
        let open_end = start + open_rel;
        let mut depth = 1usize;
        let mut i = open_end + 1;
        let mut close = None;
        while depth > 0 {
            let next_open = html[i..].find("<details").map(|at| i + at);
            let next_close = html[i..].find("</details>").map(|at| i + at);
            match (next_open, next_close) {
                (Some(open), Some(end)) if open < end => {
                    depth += 1;
                    i = open + "<details".len();
                }
                (_, Some(end)) => {
                    depth -= 1;
                    i = end + "</details>".len();
                    if depth == 0 {
                        close = Some(end);
                    }
                }
                _ => break,
            }
        }
        let Some(close_start) = close else {
            break;
        };
        if start > cursor {
            parts.push(BlockPart::Loose(String::from(&html[cursor..start])));
        }
        parts.push(BlockPart::Block {
            tag: String::from(&html[start..=open_end]),
            inner: String::from(&html[open_end + 1..close_start]),
        });
        cursor = i;
    }
    if cursor < html.len() {
        parts.push(BlockPart::Loose(String::from(&html[cursor..])));
    }
    parts
}

/// Drop the `<section class="odc-refs">` wrappers the buffer carries.
fn strip_refs_wrapper(markup: &str) -> String {
    markup
        .replace("<section class=\"odc-refs\">", "")
        .replace("</section>", "")
}

/// Take the leading summary off a block body: (summary HTML, rest).
fn split_summary(inner: &str) -> (String, &str) {
    let lead = inner.len() - inner.trim_start().len();
    let rest = &inner[lead..];
    if !rest.starts_with("<summary") {
        return (String::new(), inner);
    }
    let (Some(open), Some(close)) = (rest.find('>'), rest.find("</summary>")) else {
        return (String::new(), inner);
    };
    if close < open {
        return (String::new(), inner);
    }
    (
        String::from(rest[open + 1..close].trim()),
        &rest[close + "</summary>".len()..],
    )
}

/// Visible text of an HTML fragment, with runs of whitespace collapsed.
fn plain_text(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            other if !in_tag => out.push(other),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Ldoce6 {
    /// Map a fragment without the text-drift guard.
    ///
    /// The acceptance harness uses this to measure the mapping itself; the
    /// public entry point is [`Adapter::parse`], which downgrades drift to the
    /// raw fallback.
    #[must_use]
    pub fn map_unguarded(&self, fragment: &str, ctx: &ParseContext) -> Option<String> {
        let clean = repair_sanitize(fragment)?;
        let mut builder = Builder::new(ctx);
        builder.build(&clean);
        if !builder.adapted {
            return None;
        }
        let mut html = builder.finish();
        html = format_blocks(&html).unwrap_or(html);
        Some(html)
    }
}

/// Full text of the first element carrying `class`, including nested elements.
fn element_text(tokens: &[Tok], from: usize, class: &str) -> String {
    let mut start = None;
    for index in from..tokens.len() {
        if let Tok::Start { attrs, .. } = &tokens[index] {
            if tokens::has_class(attrs, class) {
                start = Some(index);
                break;
            }
        }
    }
    let Some(start) = start else {
        return String::new();
    };
    let name = match &tokens[start] {
        Tok::Start { name, .. } => name.clone(),
        _ => return String::new(),
    };
    let mut depth = 1usize;
    let mut out = String::new();
    for tok in tokens.iter().skip(start + 1) {
        match tok {
            Tok::Start { name: inner, .. } if *inner == name => depth += 1,
            Tok::End { name: inner } if *inner == name => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Tok::Text(text) => {
                if !out.is_empty() && !out.ends_with(' ') && !text.starts_with(char::is_whitespace)
                {
                    out.push(' ');
                }
                out.push_str(text);
            }
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// MDict alias record: `@@@LINK=<target>`.
///
/// The spec has no slot for aliases; the prototype emits a marked entry so the
/// container keeps one record per key and a renderer can follow the target.
#[must_use]
pub fn redirect_target(fragment: &str) -> Option<String> {
    let rest = fragment.trim().strip_prefix("@@@LINK=")?;
    let target = rest.trim();
    if target.is_empty() {
        None
    } else {
        Some(target.to_string())
    }
}

/// L1 shell for an alias record.
#[must_use]
pub fn redirect_entry(target: &str, ctx: &ParseContext) -> String {
    let html = format!(
        "<section class=\"odc-entry odc-entry--redirect\" data-odc-dict=\"{}\" \
         data-odc-key=\"{}\" data-odc-lang=\"{}\" data-odc-redirect=\"{}\"></section>",
        escape_attr(&ctx.dict),
        escape_attr(&ctx.key),
        escape_attr(&ctx.lang),
        escape_attr(target)
    );
    format_blocks(&html).unwrap_or(html)
}
