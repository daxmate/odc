//! BLAKE3 hashing helpers and the ODC Merkle rules (spec sections 6 and 7).

use crate::varint;
use blake3::Hasher;

/// An all-zero hash, used for absent parents and empty trees.
pub const ZERO_HASH: [u8; 32] = [0u8; 32];

/// Domain separation prefix for commit hashes.
pub const COMMIT_DOMAIN: &[u8] = b"odc-commit-v1";

/// BLAKE3-256 of `data`.
#[must_use]
pub fn blake3(data: &[u8]) -> [u8; 32] {
    *blake3::hash(data).as_bytes()
}

/// Hash a Merkle leaf: `0x00 || varint(len(word)) || word || content_hash`.
#[must_use]
pub fn leaf_hash(word: &[u8], content_hash: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Hasher::new();
    hasher.update(&[0x00]);
    hasher.update(&varint::encode_to_vec(word.len() as u64));
    hasher.update(word);
    hasher.update(content_hash);
    *hasher.finalize().as_bytes()
}

/// Hash an inner Merkle node: `0x01 || left || right`.
#[must_use]
pub fn node_hash(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Hasher::new();
    hasher.update(&[0x01]);
    hasher.update(left);
    hasher.update(right);
    *hasher.finalize().as_bytes()
}

/// Merkle root over `leaves` in the order given.
///
/// An odd node at the end of a level is promoted unchanged to the next level
/// (no duplication). An empty leaf set has the all-zero root.
#[must_use]
pub fn merkle_root(leaves: &[[u8; 32]]) -> [u8; 32] {
    if leaves.is_empty() {
        return ZERO_HASH;
    }
    let mut level: Vec<[u8; 32]> = leaves.to_vec();
    while level.len() > 1 {
        let mut next = Vec::with_capacity(level.len().div_ceil(2));
        for pair in level.chunks(2) {
            match pair {
                [left, right] => next.push(node_hash(left, right)),
                [single] => next.push(*single),
                _ => unreachable!("chunks(2) yields 1 or 2 items"),
            }
        }
        level = next;
    }
    level[0]
}

/// Compute `commit_hash` from its preimage components.
#[must_use]
pub fn commit_hash(
    parent: &[u8; 32],
    parent2: &[u8; 32],
    entry_root: &[u8; 32],
    block_root: &[u8; 32],
    entry_count: u64,
) -> [u8; 32] {
    let mut hasher = Hasher::new();
    hasher.update(COMMIT_DOMAIN);
    hasher.update(parent);
    hasher.update(parent2);
    hasher.update(entry_root);
    hasher.update(block_root);
    hasher.update(&entry_count.to_le_bytes());
    *hasher.finalize().as_bytes()
}

/// Lower-case hex encoding of a 32-byte hash.
#[must_use]
pub fn to_hex(hash: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in hash {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Parse a 64-character hex hash.
pub fn from_hex(text: &str) -> crate::error::Result<[u8; 32]> {
    if text.len() != 64 {
        return Err(crate::error::Error::InvalidArgument(format!(
            "hash must be 64 hex characters, got {}",
            text.len()
        )));
    }
    let mut out = [0u8; 32];
    for (index, chunk) in text.as_bytes().chunks(2).enumerate() {
        let chunk = std::str::from_utf8(chunk)
            .map_err(|_| crate::error::Error::InvalidArgument("hash is not ASCII".to_string()))?;
        out[index] = u8::from_str_radix(chunk, 16).map_err(|_| {
            crate::error::Error::InvalidArgument(format!("invalid hex byte {chunk}"))
        })?;
    }
    Ok(out)
}
