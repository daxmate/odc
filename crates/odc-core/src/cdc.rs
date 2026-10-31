//! Content-defined chunking over records (spec section 7).
//!
//! Boundaries are decided on the byte stream `key || 0x00 || content_hash` of
//! each record, rolled into a 64-bit gear hash byte by byte. When the low 16
//! bits of the rolling hash reach zero inside a record and the block has
//! reached the minimum size, the block is cut at the end of that record; the
//! maximum size forces a cut unconditionally. Records are atomic: a record is
//! never split across blocks.

use crate::block::Record;
use crate::hash;
use std::ops::Range;
use std::sync::OnceLock;

/// Minimum block size in bytes.
pub const MIN_CHUNK: usize = 16 * 1024;
/// Maximum block size in bytes.
pub const MAX_CHUNK: usize = 256 * 1024;
/// Boundary mask applied to the rolling hash.
pub const MASK: u64 = 0xffff;
/// Domain separation string for the gear table.
pub const GEAR_DOMAIN: &[u8] = b"odc-gear-v1";

/// The 256-entry gear table, derived from BLAKE3 so that every implementation
/// can reproduce it: `gear[i] = LE64(BLAKE3("odc-gear-v1" || [i])[0..8])`.
#[must_use]
pub fn gear_table() -> &'static [u64; 256] {
    static TABLE: OnceLock<[u64; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = [0u64; 256];
        for (index, slot) in table.iter_mut().enumerate() {
            let mut input = Vec::with_capacity(GEAR_DOMAIN.len() + 1);
            input.extend_from_slice(GEAR_DOMAIN);
            input.push(index as u8);
            let digest = hash::blake3(&input);
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&digest[..8]);
            *slot = u64::from_le_bytes(bytes);
        }
        table
    })
}

fn roll(mut state: u64, bytes: &[u8]) -> (u64, bool) {
    let table = gear_table();
    let mut hit = false;
    for byte in bytes {
        state = (state << 1).wrapping_add(table[usize::from(*byte)]);
        if state & MASK == 0 {
            hit = true;
        }
    }
    (state, hit)
}

/// Split `records` (already sorted by key) into block ranges of record
/// indices. The returned ranges are contiguous and cover every record.
#[must_use]
pub fn boundaries(records: &[Record]) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    if records.is_empty() {
        return ranges;
    }
    let mut start = 0usize;
    let mut size = 0usize;
    let mut state = 0u64;
    for (index, record) in records.iter().enumerate() {
        let record_len = record.encoded_len();
        if size > 0 && size + record_len > MAX_CHUNK {
            ranges.push(start..index);
            start = index;
            size = 0;
            state = 0;
        }
        size += record_len;
        let content_hash = hash::blake3(&record.payload);
        let mut stream = Vec::with_capacity(record.key.len() + 1 + 32);
        stream.extend_from_slice(&record.key);
        stream.push(0x00);
        stream.extend_from_slice(&content_hash);
        let (next_state, hit) = roll(state, &stream);
        state = next_state;
        let boundary = size >= MIN_CHUNK && hit;
        let oversized = size >= MAX_CHUNK;
        if boundary || oversized {
            ranges.push(start..index + 1);
            start = index + 1;
            size = 0;
            state = 0;
        }
    }
    if start < records.len() {
        ranges.push(start..records.len());
    }
    ranges
}

/// Convenience helper returning the encoded size of every block produced for
/// the given records (used by tests and documentation).
#[must_use]
pub fn block_sizes(records: &[Record]) -> Vec<usize> {
    boundaries(records)
        .into_iter()
        .map(|range| records[range].iter().map(Record::encoded_len).sum())
        .collect()
}
