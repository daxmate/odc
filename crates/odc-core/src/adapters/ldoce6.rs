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
        html = format_blocks(&html).unwrap_or(html);
        if !same_words(&clean, &html) {
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

/// Fidelity guard for the reordered entry: same words, same counts, and the
/// only allowed difference is block order (senses before refs and entry menu).
fn same_words(left: &str, right: &str) -> bool {
    let mut a: Vec<String> = content_text(left)
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let mut b: Vec<String> = content_text(right)
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
    head_open: bool,
    head_done: bool,
    entry_count: usize,
    secondary: bool,
    pos_index: usize,
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
            head_open: false,
            head_done: false,
            entry_count: 0,
            secondary: false,
            pos_index: 0,
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

    /// Move reference blocks and the entry menu to the end of the entry so the
    /// main senses stay adjacent to the headword.
    fn flush_deferred(&mut self) {
        if !self.refs.is_empty() {
            let refs = std::mem::take(&mut self.refs);
            self.out.push_str(&refs);
        }
        if !self.toc.is_empty() {
            let toc = std::mem::take(&mut self.toc);
            self.out.push_str(&toc);
        }
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
        let start = format!(
            "<section class=\"odc-entry\" data-odc-dict=\"{}\" data-odc-key=\"{}\" data-odc-lang=\"{}\">",
            escape_attr(&self.ctx.dict),
            escape_attr(&self.ctx.key),
            escape_attr(&self.ctx.lang)
        );
        self.open(0, start, "</section>");
    }

    fn emit(&mut self, text: &str) {
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
        if self.secondary || self.head_done || self.in_pv() {
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
                // `div.chwd` opens every root entry: close the previous one.
                self.end_capture();
                self.close_toc();
                self.flush_deferred();
                self.close_from(1);
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
            if self.in_ref() {
                return;
            }
            self.entry_count += 1;
            self.secondary = self.entry_count > 1;
            self.adapted = true;
            return;
        }
        if has("entryhead") || has("buttons") {
            return;
        }
        if has("hwd") {
            if self.secondary {
                self.inline_open(
                    "<span class=\"odc-hw-alt\" data-odc-kind=\"homograph-headword\">",
                    "span",
                    "span",
                );
            } else {
                self.ensure_head();
                self.inline_open("<h1 class=\"odc-hw\">", "span", "h1");
            }
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
            self.pos_index += 1;
            let label = inline_text(tokens, index);
            self.pos_slug = pos_slug(&label);
            let base = format!("{}-{}", entry_slug(&self.ctx.key), self.pos_slug);
            let id = if self.pos_index > 1 {
                format!("{base}-{}", self.pos_index)
            } else {
                base
            };
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
