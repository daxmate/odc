//! Text fidelity, idempotency and slot statistics for profile runs.
//!
//! Text extraction reuses the tokeniser the adapters already use, so the
//! profile pipeline and the LDOCE6++ harness measure the same thing. Fidelity
//! is the design draft definition: whitespace-collapsed text, compared
//! character by character in document order.

use std::collections::BTreeMap;

use crate::adapters::tokens::{tokenize, Tok};
use crate::adapters::{count_slots, ParseContext, SlotCounts};
use crate::profile::schema::Profile;
use crate::profile::{exec, Outcome};

/// Whitespace-collapsed text of a fragment, in document order.
#[must_use]
pub fn text_of(html: &str) -> String {
    let mut out = String::new();
    for token in tokenize(html) {
        if let Tok::Text(text) = token {
            out.push_str(&text);
        }
    }
    crate::profile::selector::collapse_ws(&out)
}

/// Individual text runs, whitespace-only runs dropped.
#[must_use]
pub fn text_runs(html: &str) -> Vec<String> {
    let mut runs: Vec<String> = Vec::new();
    for token in tokenize(html) {
        if let Tok::Text(text) = token {
            let text = crate::profile::selector::collapse_ws(&text);
            if !text.is_empty() {
                runs.push(text);
            }
        }
    }
    runs
}

/// Word multiset of a fragment, for order-insensitive comparison.
#[must_use]
pub fn words(html: &str) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for word in text_of(html).split_whitespace() {
        *out.entry(String::from(word)).or_insert(0) += 1;
    }
    out
}

/// Whether two fragments carry the same text in the same order.
#[must_use]
pub fn text_fidelity(left: &str, right: &str) -> bool {
    text_of(left) == text_of(right)
}

/// Whether two fragments carry the same words in any order.
#[must_use]
pub fn word_fidelity(left: &str, right: &str) -> bool {
    words(left) == words(right)
}

/// P0/P1 slot counts of an L1 fragment.
#[must_use]
pub fn slot_counts(html: &str) -> SlotCounts {
    count_slots(html)
}

/// Apply the profile to an already-adapted fragment and check the result is
/// byte-identical, i.e. the profile is a no-op on its own output.
#[must_use]
pub fn idempotent(profile: &Profile, l1: &str, ctx: &ParseContext) -> bool {
    match exec::apply(profile, l1, ctx) {
        Outcome::Adapted { html, .. } => html == l1,
        _ => false,
    }
}

/// Accumulates the structural fingerprint of a source corpus.
///
/// The digest covers tag and class histograms only, so it survives text
/// edits but changes as soon as the markup dialect changes.
#[derive(Clone, Debug, Default)]
pub struct FingerprintBuilder {
    tags: BTreeMap<String, u64>,
    classes: BTreeMap<String, u64>,
    samples: u64,
}

/// Algorithm identifier written into profiles.
pub const FINGERPRINT_ALGORITHM: &str = "odc-fingerprint/1";

impl FingerprintBuilder {
    /// Add one source fragment to the histogram.
    pub fn add(&mut self, html: &str) {
        for token in tokenize(html) {
            let Tok::Start { name, attrs, .. } = token else {
                continue;
            };
            *self.tags.entry(name).or_insert(0) += 1;
            for (key, value) in attrs {
                if key.eq_ignore_ascii_case("class") {
                    for class in value.split_whitespace() {
                        *self.classes.entry(String::from(class)).or_insert(0) += 1;
                    }
                }
            }
        }
        self.samples += 1;
    }

    /// Number of fragments added.
    #[must_use]
    pub fn samples(&self) -> u64 {
        self.samples
    }

    /// Tag histogram.
    #[must_use]
    pub fn tags(&self) -> &BTreeMap<String, u64> {
        &self.tags
    }

    /// Class histogram.
    #[must_use]
    pub fn classes(&self) -> &BTreeMap<String, u64> {
        &self.classes
    }

    /// SHA-256 digest of the sorted histograms.
    #[must_use]
    pub fn digest(&self) -> String {
        use sha2::{Digest, Sha256};

        let mut text = String::from(FINGERPRINT_ALGORITHM);
        for (tag, count) in &self.tags {
            text.push('\n');
            text.push_str(&format!("{count} {tag}"));
        }
        for (class, count) in &self.classes {
            text.push('\n');
            text.push_str(&format!("{count} {class}"));
        }
        let digest = Sha256::digest(text.as_bytes());
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }
}
