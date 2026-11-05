//! Experimental dictionary adapters for the ODC entry HTML spec v0.
//!
//! The spec lives in `docs/odc-html-spec.md`. Everything here is marked
//! experimental: the interface may change until the spec leaves v0.
//!
//! Layout:
//!
//! * [`mod@ldoce6`] — LDOCE6 / LDOCE6++ (English-Chinese) adapter,
//! * [`raw_fallback`] — spec section 6 fallback for fragments no adapter claims,
//! * [`count_slots`] — slot presence statistics used by the acceptance harness.

pub mod ldoce6;
pub(crate) mod tokens;

use crate::block_fmt::format_blocks;
use crate::error::Result;
use crate::html::repair_sanitize;

/// Context handed to [`Adapter::parse`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParseContext {
    /// Normalised entry key as stored by the container.
    pub key: String,
    /// Declared language of the headword.
    pub lang: String,
    /// Dictionary slug (`ldoce6encn`).
    pub dict: String,
}

impl ParseContext {
    /// Build a context; `lang` defaults to `en` when empty.
    #[must_use]
    pub fn new(key: impl Into<String>, dict: impl Into<String>, lang: impl Into<String>) -> Self {
        let lang = lang.into();
        Self {
            key: key.into(),
            dict: dict.into(),
            lang: if lang.is_empty() {
                String::from("en")
            } else {
                lang
            },
        }
    }
}

/// One class or tag counted while probing a fragment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signal {
    /// Signal name (a class or a tag name).
    pub name: String,
    /// How often it occurred in the fragment.
    pub count: usize,
}

/// Result of [`Adapter::probe`] (spec section 5.1).
#[derive(Clone, Debug, PartialEq)]
pub struct Probe {
    /// Dictionary slug the adapter claims.
    pub dict: &'static str,
    /// Confidence in `[0, 1]`.
    pub confidence: f64,
    /// Matched signals with counts, strongest first.
    pub signals: Vec<Signal>,
}

/// Why a fragment was not adapted (spec section 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// No adapter claimed the fragment.
    NoAdapter,
    /// An adapter matched but below its confidence threshold.
    ProbeLowConfidence,
    /// `parse` failed or produced text that drifted.
    ParseFailed,
    /// Structural coverage stayed below the adapter's threshold.
    CoverageLow,
}

impl Reason {
    /// Wire value written to `data-odc-reason`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoAdapter => "no-adapter",
            Self::ProbeLowConfidence => "probe-low-confidence",
            Self::ParseFailed => "parse-failed",
            Self::CoverageLow => "coverage-low",
        }
    }
}

/// Adapter contract (spec section 5.1).
pub trait Adapter {
    /// Dictionary slug this adapter writes into `data-odc-dict`.
    fn dict(&self) -> &'static str;
    /// Identify a fragment; `None` means "not mine".
    fn probe(&self, fragment: &str) -> Option<Probe>;
    /// Map a sanitised fragment to L1 markup.
    ///
    /// # Errors
    ///
    /// Returns an error only for unusable input; malformed fragments are
    /// expected to come back through [`raw_fallback`] instead.
    fn parse(&self, fragment: &str, ctx: &ParseContext) -> Result<String>;
    /// Map a resource reference (`sound://`, `img/...`) to a resource key.
    fn resolve(&self, reference: &str) -> Option<String>;
    /// Scoped CSS, or `None` when the base stylesheet is enough.
    fn styles(&self) -> Option<&'static str>;
}

/// Wrap a fragment in the raw fallback entry of spec section 6.
///
/// The payload is the repaired, sanitised fragment: dangerous markup never
/// comes back. Text stays searchable because it is the same body.
#[must_use]
pub fn raw_fallback(fragment: &str, ctx: &ParseContext, reason: Reason) -> String {
    let body = repair_sanitize(fragment).unwrap_or_else(|| String::from(fragment));
    let html = format!(
        "<section class=\"odc-entry odc-entry--raw\" data-odc-dict=\"{}\" data-odc-key=\"{}\" \
         data-odc-unadapted=\"1\" data-odc-reason=\"{}\">{}</section>",
        escape_attr(&ctx.dict),
        escape_attr(&ctx.key),
        reason.as_str(),
        body
    );
    format_blocks(&html).unwrap_or(html)
}

/// Escape a value for use inside a double-quoted attribute.
pub(crate) fn escape_attr(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

/// P0 slots from spec section 4.2, in spec order.
pub const P0_SLOTS: [&str; 9] = [
    "odc-entry",
    "odc-entry-head",
    "odc-pos-block",
    "odc-sense",
    "odc-sense-number",
    "odc-def",
    "odc-def-trans",
    "odc-example",
    "odc-phrasal-verb",
];

/// Per-slot node counts for one L1 fragment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SlotCounts {
    /// `(slot, nodes)` pairs for every P0 slot.
    pub p0: Vec<(&'static str, usize)>,
}

impl SlotCounts {
    /// Number of P0 slots present at least once.
    #[must_use]
    pub fn p0_present(&self) -> usize {
        self.p0.iter().filter(|(_, n)| *n > 0).count()
    }

    /// Count for one slot.
    #[must_use]
    pub fn get(&self, slot: &str) -> usize {
        self.p0
            .iter()
            .find(|(name, _)| *name == slot)
            .map_or(0, |(_, n)| *n)
    }
}

/// Count P0 slot nodes in an L1 fragment.
///
/// `odc-entry-head` is matched through the `odc-head` class used by the
/// skeleton in spec section 3.1; both names are accepted.
#[must_use]
pub fn count_slots(html: &str) -> SlotCounts {
    let mut counts = [0usize; 9];
    for tok in tokens::tokenize(html) {
        let tokens::Tok::Start { attrs, .. } = tok else {
            continue;
        };
        for (idx, slot) in P0_SLOTS.iter().enumerate() {
            let hit = match *slot {
                "odc-entry-head" => {
                    tokens::has_class(&attrs, "odc-head")
                        || tokens::has_class(&attrs, "odc-entry-head")
                }
                "odc-phrasal-verb" => {
                    tokens::has_class(&attrs, "odc-pv")
                        || tokens::has_class(&attrs, "odc-phrasal-verb")
                }
                other => tokens::has_class(&attrs, other),
            };
            if hit {
                counts[idx] += 1;
            }
        }
    }
    SlotCounts {
        p0: P0_SLOTS.iter().copied().zip(counts).collect(),
    }
}

/// P0 slots the source fragment shows evidence for (spec 8.2 "expected" side).
///
/// This is the denominator of the adapter coverage metric: a slot counts as
/// expected when the source markup carries its LDOCE class.
#[must_use]
pub fn source_slots(html: &str) -> Vec<&'static str> {
    let mut found = [false; 9];
    // Popups and the entry menu duplicate body text; they never carry slots.
    let mut skip_depth = 0usize;
    let mut stack: Vec<(String, bool)> = Vec::new();
    for tok in tokens::tokenize(html) {
        match tok {
            tokens::Tok::End { name } => {
                if let Some(position) = stack.iter().rposition(|(open, _)| *open == name) {
                    let (_, skipped) = stack.remove(position);
                    if skipped {
                        skip_depth -= 1;
                    }
                }
            }
            tokens::Tok::Start { name, attrs, .. } => {
                let skip =
                    tokens::has_class(&attrs, "at-link") || tokens::has_class(&attrs, "entrymenu");
                if skip {
                    skip_depth += 1;
                    stack.push((name, true));
                    continue;
                }
                let is_tran = name == "tran";
                stack.push((name.clone(), false));
                if skip_depth > 0 {
                    continue;
                }
                let has = |class: &str| tokens::has_class(&attrs, class);
                found[0] = true; // odc-entry: every fragment is an entry
                found[1] |= has("hwd");
                found[2] |= has("pos");
                found[3] |= has("sense");
                found[4] |= has("sensenum");
                found[5] |= has("def");
                found[6] |= is_tran;
                found[7] |= has("example");
                found[8] |= has("phrvbentry");
            }
            _ => {}
        }
    }
    P0_SLOTS
        .iter()
        .copied()
        .zip(found)
        .filter_map(|(slot, hit)| hit.then_some(slot))
        .collect()
}

/// Number of lines that split a tag across a newline (spec 8.3).
///
/// A line is a violation when its last `<` has no `>` after it.
#[must_use]
pub fn split_line_violations(html: &str) -> usize {
    html.lines()
        .filter(|line| line.rfind('<').is_some_and(|lt| !line[lt..].contains('>')))
        .count()
}
