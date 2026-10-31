//! Minimal deterministic CBOR model (RFC 8949 core deterministic encoding).
//!
//! ODC only needs a strict subset of CBOR for entry payloads and hash maps:
//! integers, byte strings, text strings, arrays, maps, booleans and null.
//! Floats, tags, indefinite lengths and simple values other than the ones
//! listed above are rejected, which keeps payload canonicalisation trivial and
//! byte-for-byte reproducible across languages.

use crate::error::{Error, Result};
use crate::varint;

/// A CBOR value restricted to the ODC core model.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Value {
    /// `null` (0xf6).
    Null,
    /// `true` / `false`.
    Bool(bool),
    /// A signed integer encoded in the smallest major-type form.
    Int(i64),
    /// A byte string.
    Bytes(Vec<u8>),
    /// A UTF-8 text string.
    Text(String),
    /// An array of values.
    Array(Vec<Value>),
    /// A map. Keys are sorted canonically when encoding.
    Map(Vec<(Value, Value)>),
}

impl Value {
    /// Build a text-keyed map from pairs, keeping insertion order (the encoder
    /// sorts on write, so callers never depend on order for hashing).
    #[must_use]
    pub fn map<K, V>(pairs: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: Into<String>,
        V: Into<Value>,
    {
        Self::Map(
            pairs
                .into_iter()
                .map(|(key, value)| (Value::Text(key.into()), value.into()))
                .collect(),
        )
    }

    /// Look up a text key inside a map value.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Self::Map(pairs) => pairs.iter().find_map(|(k, v)| match k {
                Value::Text(text) if text == key => Some(v),
                _ => None,
            }),
            _ => None,
        }
    }

    /// Iterate over the entries of a map value.
    pub fn entries(&self) -> impl Iterator<Item = (&Value, &Value)> {
        let pairs: &[(Value, Value)] = match self {
            Self::Map(pairs) => pairs.as_slice(),
            _ => &[],
        };
        pairs.iter().map(|(k, v)| (k, v))
    }

    /// Borrow the value as text, if it is one.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            _ => None,
        }
    }

    /// Borrow the value as a byte string, if it is one.
    #[must_use]
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Bytes(bytes) => Some(bytes),
            _ => None,
        }
    }

    /// Borrow the value as an array, if it is one.
    #[must_use]
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }

    /// Borrow the value as an integer, if it is one.
    #[must_use]
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(value) => Some(*value),
            _ => None,
        }
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::Text(value.to_string())
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<Vec<Value>> for Value {
    fn from(value: Vec<Value>) -> Self {
        Self::Array(value)
    }
}

impl From<Vec<u8>> for Value {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(value)
    }
}

impl<const N: usize> From<[u8; N]> for Value {
    fn from(value: [u8; N]) -> Self {
        Self::Bytes(value.to_vec())
    }
}

impl From<&[u8]> for Value {
    fn from(value: &[u8]) -> Self {
        Self::Bytes(value.to_vec())
    }
}

impl FromIterator<Value> for Value {
    fn from_iter<T: IntoIterator<Item = Value>>(iter: T) -> Self {
        Self::Array(iter.into_iter().collect())
    }
}

fn push_head(major: u8, argument: u64, out: &mut Vec<u8>) {
    let head = major << 5;
    if argument < 24 {
        out.push(head | u8::try_from(argument).expect("argument < 24"));
    } else if let Ok(small) = u8::try_from(argument) {
        out.push(head | 24);
        out.push(small);
    } else if let Ok(medium) = u16::try_from(argument) {
        out.push(head | 25);
        out.extend_from_slice(&medium.to_be_bytes());
    } else if let Ok(large) = u32::try_from(argument) {
        out.push(head | 26);
        out.extend_from_slice(&large.to_be_bytes());
    } else {
        out.push(head | 27);
        out.extend_from_slice(&argument.to_be_bytes());
    }
}

/// Append the deterministic encoding of `value` to `out`.
pub fn encode_into(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Null => out.push(0xf6),
        Value::Bool(true) => out.push(0xf5),
        Value::Bool(false) => out.push(0xf4),
        Value::Int(number) => {
            if *number >= 0 {
                push_head(0, *number as u64, out);
            } else {
                // major type 1 stores -1 - n
                let magnitude = (-1 - *number) as u64;
                push_head(1, magnitude, out);
            }
        }
        Value::Bytes(bytes) => {
            push_head(2, bytes.len() as u64, out);
            out.extend_from_slice(bytes);
        }
        Value::Text(text) => {
            push_head(3, text.len() as u64, out);
            out.extend_from_slice(text.as_bytes());
        }
        Value::Array(items) => {
            push_head(4, items.len() as u64, out);
            for item in items {
                encode_into(item, out);
            }
        }
        Value::Map(pairs) => {
            let mut encoded: Vec<(Vec<u8>, &Value)> = pairs
                .iter()
                .map(|(key, item)| {
                    let mut key_bytes = Vec::new();
                    encode_into(key, &mut key_bytes);
                    (key_bytes, item)
                })
                .collect();
            encoded.sort_by(|left, right| left.0.cmp(&right.0));
            push_head(5, encoded.len() as u64, out);
            for (key_bytes, item) in encoded {
                out.extend_from_slice(&key_bytes);
                encode_into(item, out);
            }
        }
    }
}

/// Deterministic encoding of `value`.
#[must_use]
pub fn encode(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    encode_into(value, &mut out);
    out
}

struct Decoder<'a> {
    input: &'a [u8],
    position: usize,
}

impl<'a> Decoder<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(count)
            .ok_or(Error::Truncated("cbor"))?;
        let slice = self
            .input
            .get(self.position..end)
            .ok_or(Error::Truncated("cbor"))?;
        self.position = end;
        Ok(slice)
    }

    fn argument(&mut self, additional: u8) -> Result<u64> {
        match additional {
            0..=23 => Ok(u64::from(additional)),
            24 => Ok(u64::from(self.take(1)?[0])),
            25 => {
                let bytes = self.take(2)?;
                Ok(u64::from(u16::from_be_bytes([bytes[0], bytes[1]])))
            }
            26 => {
                let bytes = self.take(4)?;
                let array: [u8; 4] = bytes.try_into().expect("4 bytes");
                Ok(u64::from(u32::from_be_bytes(array)))
            }
            27 => {
                let bytes = self.take(8)?;
                let array: [u8; 8] = bytes.try_into().expect("8 bytes");
                Ok(u64::from_be_bytes(array))
            }
            _ => Err(Error::UnsupportedCbor(additional)),
        }
    }

    fn value(&mut self) -> Result<Value> {
        let initial = self.take(1)?[0];
        let major = initial >> 5;
        let additional = initial & 0x1f;
        match major {
            0 => {
                let argument = self.argument(additional)?;
                i64::try_from(argument)
                    .map(Value::Int)
                    .map_err(|_| Error::Cbor("integer out of i64 range"))
            }
            1 => {
                let argument = self.argument(additional)?;
                if argument > i64::MAX as u64 {
                    return Err(Error::Cbor("negative integer out of i64 range"));
                }
                Ok(Value::Int(-1 - argument as i64))
            }
            2 => {
                let len = self.argument(additional)?;
                let len = usize::try_from(len).map_err(|_| Error::Cbor("length out of range"))?;
                Ok(Value::Bytes(self.take(len)?.to_vec()))
            }
            3 => {
                let len = self.argument(additional)?;
                let len = usize::try_from(len).map_err(|_| Error::Cbor("length out of range"))?;
                let bytes = self.take(len)?;
                let text = std::str::from_utf8(bytes)
                    .map_err(|_| Error::Cbor("invalid UTF-8 in text string"))?;
                Ok(Value::Text(text.to_string()))
            }
            4 => {
                let len = self.argument(additional)?;
                let len = usize::try_from(len).map_err(|_| Error::Cbor("length out of range"))?;
                let mut items = Vec::with_capacity(len.min(1024));
                for _ in 0..len {
                    items.push(self.value()?);
                }
                Ok(Value::Array(items))
            }
            5 => {
                let len = self.argument(additional)?;
                let len = usize::try_from(len).map_err(|_| Error::Cbor("length out of range"))?;
                let mut pairs: Vec<(Value, Value)> = Vec::with_capacity(len.min(1024));
                let mut last_key: Option<Vec<u8>> = None;
                let mut keys: Vec<Vec<u8>> = Vec::with_capacity(len.min(1024));
                for _ in 0..len {
                    let key_start = self.position;
                    let key = self.value()?;
                    let key_bytes = self.input[key_start..self.position].to_vec();
                    if keys.contains(&key_bytes) {
                        return Err(Error::Cbor("duplicate map key"));
                    }
                    if let Some(previous) = &last_key {
                        if key_bytes <= *previous {
                            return Err(Error::Cbor("map keys not in canonical order"));
                        }
                    }
                    last_key = Some(key_bytes.clone());
                    keys.push(key_bytes);
                    let value = self.value()?;
                    pairs.push((key, value));
                }
                Ok(Value::Map(pairs))
            }
            7 => match additional {
                20 => Ok(Value::Bool(false)),
                21 => Ok(Value::Bool(true)),
                22 => Ok(Value::Null),
                _ => Err(Error::UnsupportedCbor(additional)),
            },
            other => Err(Error::UnsupportedCbor(other)),
        }
    }
}

/// Decode exactly one deterministic CBOR value; trailing bytes are an error.
pub fn decode(input: &[u8]) -> Result<Value> {
    let mut decoder = Decoder { input, position: 0 };
    let value = decoder.value()?;
    if decoder.position != input.len() {
        return Err(Error::TrailingData("cbor value"));
    }
    Ok(value)
}

/// Re-encode `value` and verify it is already canonical.
pub fn validate_canonical(bytes: &[u8]) -> Result<Value> {
    let value = decode(bytes)?;
    if encode(&value) == bytes {
        Ok(value)
    } else {
        Err(Error::Cbor("non-canonical encoding"))
    }
}

/// Encode a length-prefixed byte string helper used by several structures.
#[must_use]
pub fn text(value: &str) -> Value {
    Value::Text(value.to_string())
}

/// Build a varint-prefixed byte string value (used for indexes in tests).
#[must_use]
pub fn bytes_with_prefix(prefix: u8, value: &[u8]) -> Vec<u8> {
    let mut out = vec![prefix];
    varint::encode(value.len() as u64, &mut out);
    out.extend_from_slice(value);
    out
}

/// Write a definite-length array header for `len` items.
///
/// Streaming encoders use this together with [`write_text`] and [`write_bytes`]
/// when the item count is known up front but the items arrive one at a time.
pub fn write_array_header(len: u64, out: &mut Vec<u8>) {
    push_head(4, len, out);
}

/// Write one CBOR text string.
pub fn write_text(text: &str, out: &mut Vec<u8>) {
    push_head(3, text.len() as u64, out);
    out.extend_from_slice(text.as_bytes());
}

/// Write one CBOR byte string.
pub fn write_bytes(bytes: &[u8], out: &mut Vec<u8>) {
    push_head(2, bytes.len() as u64, out);
    out.extend_from_slice(bytes);
}
