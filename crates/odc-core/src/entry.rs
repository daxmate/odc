//! Entry payload model and canonical CBOR encoding (spec sections 4 and 5).

use crate::cbor::{self, Value};
use crate::error::{Error, Result};
use crate::hash;
use unicode_normalization::UnicodeNormalization;

/// A single sense / definition inside an entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Def {
    /// Part of speech tag, e.g. `noun`.
    pub pos: String,
    /// The definition text.
    pub sense: String,
    /// Example sentences.
    pub examples: Vec<String>,
    /// Usage labels (register, region, domain, ...).
    pub labels: Vec<String>,
}

impl Def {
    /// Build a minimal definition with only a sense.
    #[must_use]
    pub fn new(sense: impl Into<String>) -> Self {
        Self {
            sense: sense.into(),
            ..Self::default()
        }
    }

    fn to_value(&self) -> Value {
        Value::map([
            ("pos", Value::Text(self.pos.clone())),
            ("sense", Value::Text(self.sense.clone())),
            (
                "examples",
                Value::Array(self.examples.iter().cloned().map(Value::Text).collect()),
            ),
            (
                "labels",
                Value::Array(self.labels.iter().cloned().map(Value::Text).collect()),
            ),
        ])
    }

    fn from_value(value: &Value) -> Result<Self> {
        Ok(Self {
            pos: text_field(value, "pos")?.unwrap_or_default(),
            sense: text_field(value, "sense")?.unwrap_or_default(),
            examples: text_array(value, "examples")?,
            labels: text_array(value, "labels")?,
        })
    }
}

/// One dictionary entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    /// Normalised lookup key (NFKC + case folding); primary key of the entry.
    pub word: String,
    /// Display form shown to readers (defaults to the source spelling).
    pub display: String,
    /// Pronunciation strings, IPA or otherwise.
    pub phonetic: Vec<String>,
    /// Definitions.
    pub defs: Vec<Def>,
    /// Original HTML fallback for entries that cannot be structured yet.
    pub raw_html: Option<String>,
    /// Unknown payload fields preserved verbatim for forward compatibility.
    pub extra: Vec<(String, Value)>,
}

impl Entry {
    /// Create an entry, normalising the lookup key and defaulting `display`.
    #[must_use]
    pub fn new(display: impl Into<String>) -> Self {
        let display = display.into();
        let word = normalize_key(&display);
        Self {
            word,
            display,
            ..Self::default()
        }
    }

    /// The payload map including every known field (canonical, deterministic).
    #[must_use]
    pub fn payload_value(&self) -> Value {
        let mut pairs: Vec<(String, Value)> = vec![
            ("display".to_string(), Value::Text(self.display.clone())),
            (
                "phonetic".to_string(),
                Value::Array(self.phonetic.iter().cloned().map(Value::Text).collect()),
            ),
            (
                "defs".to_string(),
                Value::Array(self.defs.iter().map(Def::to_value).collect()),
            ),
            (
                "raw_html".to_string(),
                self.raw_html
                    .as_ref()
                    .map_or(Value::Null, |html| Value::Text(html.clone())),
            ),
            (
                "extra".to_string(),
                Value::Map(
                    self.extra
                        .iter()
                        .map(|(key, value)| (Value::Text(key.clone()), value.clone()))
                        .collect(),
                ),
            ),
        ];
        pairs.sort_by(|left, right| left.0.cmp(&right.0));
        Value::Map(
            pairs
                .into_iter()
                .map(|(key, value)| (Value::Text(key), value))
                .collect(),
        )
    }

    /// Canonical CBOR bytes of the payload.
    #[must_use]
    pub fn encode_payload(&self) -> Vec<u8> {
        cbor::encode(&self.payload_value())
    }

    /// BLAKE3 of the canonical payload bytes.
    #[must_use]
    pub fn content_hash(&self) -> [u8; 32] {
        hash::blake3(&self.encode_payload())
    }

    /// Per-field hashes: field name to BLAKE3 of that field's canonical CBOR.
    #[must_use]
    pub fn field_hashes(&self) -> Vec<(String, [u8; 32])> {
        let payload = self.payload_value();
        let mut out: Vec<(String, [u8; 32])> = payload
            .entries()
            .filter_map(|(key, value)| {
                key.as_text()
                    .map(|name| (name.to_string(), hash::blake3(&cbor::encode(value))))
            })
            .collect();
        out.sort_by(|left, right| left.0.cmp(&right.0));
        out
    }

    /// The `field_hashes` CBOR map stored in the index (name to 32 bytes).
    #[must_use]
    pub fn field_hashes_value(&self) -> Value {
        Value::Map(
            self.field_hashes()
                .into_iter()
                .map(|(name, digest)| (Value::Text(name), Value::Bytes(digest.to_vec())))
                .collect(),
        )
    }

    /// Decode a payload produced by the payload encoder.
    pub fn decode_payload(bytes: &[u8]) -> Result<Self> {
        let value = cbor::decode(bytes)?;
        let Value::Map(_) = value else {
            return Err(Error::Cbor("entry payload must be a map"));
        };
        let mut entry = Self {
            display: text_field(&value, "display")?.unwrap_or_default(),
            phonetic: text_array(&value, "phonetic")?,
            raw_html: text_field(&value, "raw_html")?,
            ..Self::default()
        };
        if let Some(defs) = value.get("defs").and_then(Value::as_array) {
            entry.defs = defs.iter().map(Def::from_value).collect::<Result<_>>()?;
        }
        if let Some(extra) = value.get("extra") {
            for (key, item) in extra.entries() {
                if let Some(name) = key.as_text() {
                    if !is_known_field(name) {
                        entry.extra.push((name.to_string(), item.clone()));
                    }
                }
            }
        }
        for (key, item) in value.entries() {
            if let Some(name) = key.as_text() {
                if !is_known_field(name) && !entry.extra.iter().any(|(known, _)| known == name) {
                    entry.extra.push((name.to_string(), item.clone()));
                }
            }
        }
        entry.extra.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(entry)
    }
}

/// The five payload fields defined by ODC 0.1.
#[must_use]
pub fn is_known_field(name: &str) -> bool {
    matches!(name, "display" | "phonetic" | "defs" | "raw_html" | "extra")
}

fn text_field(value: &Value, name: &str) -> Result<Option<String>> {
    match value.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(Error::Cbor("expected text field")),
    }
}

fn text_array(value: &Value, name: &str) -> Result<Vec<String>> {
    match value.get(name) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_text()
                    .map(str::to_string)
                    .ok_or(Error::Cbor("expected text array element"))
            })
            .collect(),
        Some(_) => Err(Error::Cbor("expected text array")),
    }
}

/// Normalise a query key: NFKC then case folding (spec section 4.2).
///
/// Full case folding is approximated by Unicode lowercase plus the special
/// cases listed in the spec ("known limitations"): `ß`/ẞ map to `ss` and the
/// final sigma `ς` maps to `σ`.
#[must_use]
pub fn normalize_key(raw: &str) -> String {
    let folded: String = raw.nfkc().collect::<String>().to_lowercase();
    let mut out = String::with_capacity(folded.len());
    for character in folded.chars() {
        match character {
            'ß' | 'ẞ' => out.push_str("ss"),
            'ς' => out.push('σ'),
            other => out.push(other),
        }
    }
    out
}
