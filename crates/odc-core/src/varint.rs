//! Canonical unsigned LEB128 varints (spec section 3.1).

use crate::error::{Error, Result};

/// Maximum number of bytes a canonical u64 varint can occupy.
pub const MAX_LEN: usize = 10;

/// Number of bytes `value` occupies when encoded canonically.
#[must_use]
pub fn encoded_len(mut value: u64) -> usize {
    let mut len = 1;
    while value >= 0x80 {
        value >>= 7;
        len += 1;
    }
    len
}

/// Append the canonical LEB128 encoding of `value` to `out`.
pub fn encode(value: u64, out: &mut Vec<u8>) {
    let mut value = value;
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Convenience wrapper returning a freshly allocated encoding.
#[must_use]
pub fn encode_to_vec(value: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(encoded_len(value));
    encode(value, &mut out);
    out
}

/// Decode one canonical varint from the front of `input`.
///
/// Returns the value and the number of bytes consumed. Non-minimal encodings
/// (trailing zero continuation groups) are rejected so that a block has a
/// single valid byte representation.
pub fn decode(input: &[u8]) -> Result<(u64, usize)> {
    let mut value: u64 = 0;
    let mut shift: u32 = 0;
    for (index, &byte) in input.iter().enumerate() {
        if index >= MAX_LEN {
            return Err(Error::MalformedVarint);
        }
        let low = u64::from(byte & 0x7f);
        if shift == 63 && low > 1 {
            return Err(Error::MalformedVarint);
        }
        value |= low.checked_shl(shift).ok_or(Error::MalformedVarint)?;
        if byte & 0x80 == 0 {
            // Reject overlong encodings such as [0x80, 0x00].
            if byte == 0 && index > 0 {
                return Err(Error::MalformedVarint);
            }
            return Ok((value, index + 1));
        }
        shift += 7;
    }
    Err(Error::Truncated("varint"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_boundaries() {
        for value in [0, 1, 127, 128, 16_383, 16_384, u32::MAX as u64, u64::MAX] {
            let encoded = encode_to_vec(value);
            assert_eq!(encoded.len(), encoded_len(value));
            let (decoded, used) = decode(&encoded).expect("decode");
            assert_eq!(decoded, value);
            assert_eq!(used, encoded.len());
        }
    }

    #[test]
    fn rejects_overlong() {
        assert_eq!(decode(&[0x80, 0x00]), Err(Error::MalformedVarint));
        assert_eq!(decode(&[0xff, 0x00]), Err(Error::MalformedVarint));
    }

    #[test]
    fn rejects_truncated() {
        assert_eq!(decode(&[0x80]), Err(Error::Truncated("varint")));
        assert_eq!(decode(&[]), Err(Error::Truncated("varint")));
    }
}
