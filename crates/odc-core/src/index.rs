//! Commit index entries and their parent chain.
//!
//! A commit stores either a full snapshot of its index or the difference to
//! its first parent (see [`crate::snapshot`]). Reading a delta means walking
//! back to the nearest full entry; [`IndexChain`] does that lazily, so a
//! million-entry index is never materialised as a single buffer.

use crate::error::{Error, Result};
use crate::hash::{self, ZERO_HASH};
use crate::repo::hash_from_blob;
use crate::snapshot;
use rusqlite::{params, Connection, OptionalExtension};
use std::io::Read;

/// One commit index rebuilt from its entry and, for a delta, its parent chain.
pub(crate) struct IndexChain<'a> {
    source: Box<dyn snapshot::PairSource + 'a>,
    /// Sum of the stored raw_size columns this chain decoded.
    raw_size: u64,
}

impl IndexChain<'_> {
    /// Next pair of the rebuilt index.
    pub(crate) fn next_pair(&mut self) -> Result<Option<(String, [u8; 32])>> {
        self.source.next_pair()
    }

    /// Uncompressed CBOR bytes decoded across the whole chain.
    pub(crate) fn bytes_read(&self) -> u64 {
        self.source.bytes_read()
    }

    /// Stored uncompressed size summed over the chain.
    pub(crate) fn raw_size(&self) -> u64 {
        self.raw_size
    }
}

impl snapshot::PairSource for IndexChain<'_> {
    fn next_pair(&mut self) -> Result<Option<(String, [u8; 32])>> {
        self.source.next_pair()
    }

    fn bytes_read(&self) -> u64 {
        self.source.bytes_read()
    }
}

/// Open the index stored for a commit and, for a delta, replay it over its
/// parents until a full entry is reached.
pub(crate) fn open_index<'a>(
    connection: &'a Connection,
    hash: &[u8; 32],
    budget: u32,
) -> Result<IndexChain<'a>> {
    let row: Option<(i64, i64)> = connection
        .query_row(
            "SELECT rowid, raw_size FROM commit_index WHERE commit_hash = ?1",
            params![hash.as_slice()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((rowid, raw_size)) = row else {
        return Err(Error::Inconsistent(format!(
            "missing commit_index for {}",
            hash::to_hex(hash)
        )));
    };
    let raw_size = u64::try_from(raw_size).unwrap_or(u64::MAX);
    let blob = connection.blob_open("main", "commit_index", "data", rowid, true)?;
    let decoder =
        zstd::stream::read::Decoder::new(blob).map_err(|error| Error::Io(error.to_string()))?;
    let source: Box<dyn Read + 'a> = Box::new(decoder);
    let reader = snapshot::SnapshotReader::new(source)?;
    match reader.header().kind {
        snapshot::EntryKind::Full => Ok(IndexChain {
            source: Box::new(reader),
            raw_size,
        }),
        snapshot::EntryKind::Delta => {
            let parent = commit_parent(connection, hash)?.ok_or_else(|| {
                Error::Inconsistent(format!(
                    "commit {} has a delta index but no parent index to rebuild it",
                    hash::to_hex(hash)
                ))
            })?;
            if budget == 0 {
                return Err(Error::Inconsistent(format!(
                    "commit index chain at {} is longer than {} deltas",
                    hash::to_hex(hash),
                    snapshot::MAX_CHAIN_DEPTH
                )));
            }
            let parent = open_index(connection, &parent, budget - 1)?;
            let raw_size = raw_size.saturating_add(parent.raw_size());
            let merged = snapshot::DeltaSource::new(reader, Box::new(parent))?;
            Ok(IndexChain {
                source: Box::new(merged),
                raw_size,
            })
        }
    }
}

/// First-parent hash of a commit, or None at the genesis commit.
pub(crate) fn commit_parent(connection: &Connection, hash: &[u8; 32]) -> Result<Option<[u8; 32]>> {
    let parent: Option<Vec<u8>> = connection
        .query_row(
            "SELECT parent FROM commits WHERE commit_hash = ?1",
            params![hash.as_slice()],
            |row| row.get(0),
        )
        .optional()?;
    match parent {
        Some(bytes) => {
            let parent = hash_from_blob(&bytes)?;
            Ok((parent != ZERO_HASH).then_some(parent))
        }
        None => Ok(None),
    }
}

/// Frame of the index entry stored for a commit, without decoding its body.
pub(crate) fn index_header(
    connection: &Connection,
    hash: &[u8; 32],
) -> Result<Option<snapshot::EntryHeader>> {
    let rowid: Option<i64> = connection
        .query_row(
            "SELECT rowid FROM commit_index WHERE commit_hash = ?1",
            params![hash.as_slice()],
            |row| row.get(0),
        )
        .optional()?;
    let Some(rowid) = rowid else {
        return Ok(None);
    };
    let blob = connection.blob_open("main", "commit_index", "data", rowid, true)?;
    let decoder =
        zstd::stream::read::Decoder::new(blob).map_err(|error| Error::Io(error.to_string()))?;
    let reader = snapshot::SnapshotReader::new(decoder)?;
    Ok(Some(reader.header()))
}
