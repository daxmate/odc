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
use crate::varint;
use std::ops::Range;
use std::sync::OnceLock;

/// Minimum block size in bytes.
pub const MIN_CHUNK: usize = 16 * 1024;
/// Maximum block size in bytes.
///
/// 1 MiB (was 256 KiB). Larger blocks trade dedupe granularity for fewer,
/// longer compression windows: every cut restarts the zstd frame, so a
/// quarter of the cuts gives roughly a quarter of the frame overhead on a
/// dictionary import whose content boundary hits are rare.
pub const MAX_CHUNK: usize = 1024 * 1024;
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

/// Record identity that is enough to reproduce chunk boundaries without the
/// payload bytes: key, payload digest and payload length.
#[derive(Clone, Copy, Debug)]
pub struct RecordShape<'a> {
    /// Record key bytes.
    pub key: &'a [u8],
    /// BLAKE3 digest of the payload.
    pub content_hash: &'a [u8; 32],
    /// Payload length in bytes.
    pub payload_len: usize,
}

impl RecordShape<'_> {
    /// Encoded record size, matching the size used by the chunker.
    #[must_use]
    pub fn encoded_len(&self) -> usize {
        varint::encoded_len(self.key.len() as u64)
            + self.key.len()
            + varint::encoded_len(self.payload_len as u64)
            + self.payload_len
    }
}

/// Split records described by `RecordShape` into block ranges of record
/// indices. Produces exactly the ranges `boundaries` yields for records with
/// the same keys, payload lengths and payload digests.
#[must_use]
pub fn boundaries_shaped(records: &[RecordShape<'_>]) -> Vec<Range<usize>> {
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
        let mut stream = Vec::with_capacity(record.key.len() + 1 + 32);
        stream.extend_from_slice(record.key);
        stream.push(0x00);
        stream.extend_from_slice(record.content_hash);
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

/// Where a block boundary falls relative to the record just pushed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cut {
    /// The record fits in the current block; keep going.
    None,
    /// A block must be closed *before* the pushed record is added.
    Before,
    /// The pushed record completes a block.
    After,
}

/// Incremental form of [`boundaries`].
///
/// Feeding records one at a time and closing a block whenever [`Cut::After`]
/// (or [`Cut::Before`]) is reported produces exactly the ranges that
/// [`boundaries`] returns for the same record sequence.
#[derive(Debug, Default)]
pub struct StreamChunker {
    size: usize,
    state: u64,
}

impl StreamChunker {
    /// Create a chunker positioned at the start of a block.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of encoded bytes accumulated in the current block.
    #[must_use]
    pub fn size(&self) -> usize {
        self.size
    }

    /// Offer the next record (already sorted by key).
    ///
    /// On [`Cut::Before`] the record is *not* consumed: close the block and
    /// offer the same record again, which then reports [`Cut::None`].
    pub fn push(&mut self, record: &Record) -> Cut {
        let record_len = record.encoded_len();
        if self.size > 0 && self.size + record_len > MAX_CHUNK {
            self.size = 0;
            self.state = 0;
            return Cut::Before;
        }
        self.size += record_len;
        let content_hash = hash::blake3(&record.payload);
        let mut stream = Vec::with_capacity(record.key.len() + 1 + 32);
        stream.extend_from_slice(&record.key);
        stream.push(0x00);
        stream.extend_from_slice(&content_hash);
        let (next_state, hit) = roll(self.state, &stream);
        self.state = next_state;
        let boundary = self.size >= MIN_CHUNK && hit;
        let oversized = self.size >= MAX_CHUNK;
        if boundary || oversized {
            self.size = 0;
            self.state = 0;
            return Cut::After;
        }
        Cut::None
    }
}
