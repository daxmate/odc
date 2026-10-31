//! Patch wire format and transfer (spec section 11).
//!
//! A patch carries the block set difference between two commits plus the
//! index delta, so an offline mirror can advance from the base commit to the
//! target commit without shipping unchanged blocks. Applying to a descendant
//! HEAD installs the target and merges it three-way (spec section 11.1).

use crate::block;
use crate::cbor;
use crate::cdc::{self, RecordShape};
use crate::entry::Entry;
use crate::error::{Error, Result};
use crate::hash;
use crate::merge::{self, Strategy};
use crate::repo::{self, BlockReader, CommitRecord, Repo, DEFAULT_AUTHOR};
use crate::snapshot;
use crate::store::codec;
use crate::varint;
use rusqlite::params;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Patch magic, four ASCII bytes.
pub const MAGIC: [u8; 4] = *b"ODP1";

/// Patch layout version, independent from the container minor version.
pub const PATCH_VERSION: u8 = 1;

/// Bit 0 of the flags byte: the patch carries the full index delta.
pub const FLAG_INDEX_DELTA: u8 = 0x01;

/// Domain separation prefix for the patch checksum.
const CHECKSUM_DOMAIN: &[u8] = b"odc-patch-v1";

/// Logical description of a patch, mirroring the byte layout in the spec.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PatchPlan {
    /// Base commit hash the patch applies to.
    pub base: [u8; 32],
    /// Target commit hash produced by applying the patch.
    pub target: [u8; 32],
    /// Flags byte from the wire format (bit 0: index delta present).
    pub flags: u8,
    /// Block hashes the receiver is expected to already have.
    pub base_blocks: Vec<[u8; 32]>,
    /// Target blocks carried inline, as raw (uncompressed) block bytes.
    pub new_blocks: Vec<Vec<u8>>,
    /// Index delta rows: normalised word and its new content hash.
    pub index_rows: Vec<(String, [u8; 32])>,
    /// BLAKE3 checksum of every preceding patch byte.
    pub checksum: [u8; 32],
}

/// Result of applying a patch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatchOutcome {
    /// Commit the repository now points at.
    pub target: [u8; 32],
    /// Blocks already present locally and reused.
    pub base_blocks: usize,
    /// Blocks inserted from the patch.
    pub new_blocks: usize,
    /// Index rows in the delta.
    pub index_changes: usize,
    /// Patch size in bytes.
    pub bytes: usize,
}

/// Result of verifying a patch against a local repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatchVerify {
    /// Base commit hash.
    pub base: [u8; 32],
    /// Target commit hash.
    pub target: [u8; 32],
    /// Blocks expected from the base commit.
    pub base_blocks: usize,
    /// Blocks carried inline.
    pub new_blocks: usize,
    /// Index rows in the delta.
    pub index_changes: usize,
    /// Whether the target commit already exists locally.
    pub target_present: bool,
}

/// One entry reconstructed from a target block.
struct EntryRow {
    word: String,
    display: String,
    block_hash: [u8; 32],
    offset: i64,
    length: i64,
    content_hash: [u8; 32],
    field_hashes: Vec<u8>,
}

/// Target state rebuilt from the union of base and patched blocks.
struct TargetState {
    rows: Vec<EntryRow>,
    entry_root: [u8; 32],
    block_root: [u8; 32],
    entry_count: u64,
}

/// Payload lengths and block signatures for every block in the repository.
struct Universe {
    payload_len: HashMap<[u8; 32], usize>,
    block_by_signature: HashMap<[u8; 32], [u8; 32]>,
}

/// Cursor over patch bytes with canonical-varint helpers.
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }

    fn take(&mut self, len: usize, what: &'static str) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(len).ok_or(Error::Truncated(what))?;
        if end > self.bytes.len() {
            return Err(Error::Truncated(what));
        }
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn byte(&mut self, what: &'static str) -> Result<u8> {
        Ok(self.take(1, what)?[0])
    }

    fn varint(&mut self, what: &'static str) -> Result<u64> {
        let (value, used) = varint::decode(&self.bytes[self.pos..])?;
        if used == 0 {
            return Err(Error::Truncated(what));
        }
        self.pos += used;
        Ok(value)
    }

    fn count(&mut self, what: &'static str) -> Result<usize> {
        let value = self.varint(what)?;
        let value = usize::try_from(value).map_err(|_| Error::PatchFormat(what.to_string()))?;
        if value > self.remaining() {
            return Err(Error::Truncated(what));
        }
        Ok(value)
    }
}

/// BLAKE3 checksum over the domain prefix and the patch body.
#[must_use]
pub fn checksum(body: &[u8]) -> [u8; 32] {
    let mut buffer = Vec::with_capacity(CHECKSUM_DOMAIN.len() + body.len());
    buffer.extend_from_slice(CHECKSUM_DOMAIN);
    buffer.extend_from_slice(body);
    hash::blake3(&buffer)
}

fn encode_body(plan: &PatchPlan) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    out.push(PATCH_VERSION);
    out.push(plan.flags);
    out.extend_from_slice(&plan.base);
    out.extend_from_slice(&plan.target);
    varint::encode(plan.base_blocks.len() as u64, &mut out);
    for block_hash in &plan.base_blocks {
        out.extend_from_slice(block_hash);
    }
    varint::encode(plan.new_blocks.len() as u64, &mut out);
    for raw in &plan.new_blocks {
        let data = zstd::bulk::compress(raw, 3).map_err(|error| Error::Io(error.to_string()))?;
        varint::encode(raw.len() as u64, &mut out);
        varint::encode(codec::ZSTD as u64, &mut out);
        varint::encode(data.len() as u64, &mut out);
        out.extend_from_slice(&data);
    }
    varint::encode(plan.index_rows.len() as u64, &mut out);
    for (word, digest) in &plan.index_rows {
        varint::encode(word.len() as u64, &mut out);
        out.extend_from_slice(word.as_bytes());
        out.extend_from_slice(digest);
    }
    Ok(out)
}

/// Encode a plan into the frozen ODP1 byte layout.
pub fn encode(plan: &PatchPlan) -> Result<Vec<u8>> {
    let mut out = encode_body(plan)?;
    let digest = checksum(&out);
    out.extend_from_slice(&digest);
    Ok(out)
}

fn hash_from_blob(bytes: &[u8]) -> Result<[u8; 32]> {
    bytes
        .try_into()
        .map_err(|_| Error::PatchFormat("expected a 32-byte hash".to_string()))
}

/** Largest block the decoder will inflate; the chunker caps blocks at 256 KiB. */
const MAX_BLOCK_BYTES: usize = 64 * 1024 * 1024;

/// Walk the patch framing without inflating blocks, so truncation is
/// reported as `ODC-E002` before the checksum is consulted.
fn scan_framing(body: &[u8]) -> Result<()> {
    let mut cursor = Cursor::new(body);
    if cursor.take(4, "patch magic")? != MAGIC {
        return Err(Error::BadMagic("patch magic is not ODP1".to_string()));
    }
    cursor.byte("patch version")?;
    cursor.byte("patch flags")?;
    cursor.take(32, "patch base")?;
    cursor.take(32, "patch target")?;
    let base_count = cursor.varint("base block count")?;
    let base_bytes = usize::try_from(base_count)
        .map_err(|_| Error::PatchFormat("base block count is too large".to_string()))?;
    if base_bytes > cursor.remaining() / 32 {
        return Err(Error::Truncated("base block hash"));
    }
    cursor.take(base_bytes * 32, "base block hash")?;
    let block_count = cursor.varint("block count")?;
    for _ in 0..block_count {
        cursor.varint("block raw size")?;
        cursor.varint("block codec")?;
        let len = cursor.varint("block data length")?;
        let len = usize::try_from(len)
            .map_err(|_| Error::PatchFormat("block is too large".to_string()))?;
        cursor.take(len, "block data")?;
    }
    let index_count = cursor.varint("index count")?;
    for _ in 0..index_count {
        let len = cursor.varint("index word length")?;
        let len = usize::try_from(len)
            .map_err(|_| Error::PatchFormat("index word is too large".to_string()))?;
        cursor.take(len, "index word")?;
        cursor.take(32, "index content hash")?;
    }
    if cursor.remaining() != 0 {
        return Err(Error::TrailingData("patch body"));
    }
    Ok(())
}

/// Decode and validate the ODP1 byte layout.
pub fn decode(bytes: &[u8]) -> Result<PatchPlan> {
    if bytes.len() < 32 {
        return Err(Error::Truncated("patch"));
    }
    let (body, stored) = bytes.split_at(bytes.len() - 32);
    scan_framing(body)?;
    if checksum(body) != hash_from_blob(stored)? {
        return Err(Error::PatchChecksum);
    }
    let mut cursor = Cursor::new(body);
    if cursor.take(4, "patch magic")? != MAGIC {
        return Err(Error::BadMagic("patch magic is not ODP1".to_string()));
    }
    let version = cursor.byte("patch version")?;
    if version != PATCH_VERSION {
        return Err(Error::UnsupportedVersion {
            found: format!("patch {version}"),
            supported: "patch 1",
        });
    }
    let flags = cursor.byte("patch flags")?;
    if flags & !FLAG_INDEX_DELTA != 0 {
        return Err(Error::PatchFormat(format!(
            "unknown patch flags {flags:#04x}"
        )));
    }
    let base = hash_from_blob(cursor.take(32, "patch base")?)?;
    let target = hash_from_blob(cursor.take(32, "patch target")?)?;
    let base_count = cursor.count("base block count")?;
    let mut base_blocks = Vec::with_capacity(base_count);
    for _ in 0..base_count {
        base_blocks.push(hash_from_blob(cursor.take(32, "base block hash")?)?);
    }
    let block_count = cursor.count("block count")?;
    let mut new_blocks = Vec::with_capacity(block_count);
    for _ in 0..block_count {
        let raw_size = cursor.varint("block raw size")?;
        let raw_size = usize::try_from(raw_size)
            .map_err(|_| Error::PatchFormat("block is too large".to_string()))?;
        if raw_size > MAX_BLOCK_BYTES {
            return Err(Error::PatchFormat("block is too large".to_string()));
        }
        let codec_id = cursor.varint("block codec")?;
        let data_len = cursor.count("block data length")?;
        let data = cursor.take(data_len, "block data")?;
        let raw = match i64::try_from(codec_id).unwrap_or(i64::MAX) {
            codec::ZSTD => repo::decode_block_data(codec::ZSTD, raw_size, data)?,
            codec::RAW => data.to_vec(),
            other => return Err(Error::UnknownCodec(other)),
        };
        if raw.len() != raw_size {
            return Err(Error::PatchFormat(
                "block raw size does not match its payload".to_string(),
            ));
        }
        block::decode(&raw)?;
        new_blocks.push(raw);
    }
    let index_count = cursor.count("index count")?;
    let mut index_rows = Vec::with_capacity(index_count);
    for _ in 0..index_count {
        let len = cursor.count("index word length")?;
        let word = std::str::from_utf8(cursor.take(len, "index word")?)
            .map_err(|_| Error::PatchFormat("index word is not UTF-8".to_string()))?
            .to_string();
        let digest = hash_from_blob(cursor.take(32, "index content hash")?)?;
        index_rows.push((word, digest));
    }
    if cursor.remaining() != 0 {
        return Err(Error::TrailingData("patch body"));
    }
    Ok(PatchPlan {
        base,
        target,
        flags,
        base_blocks,
        new_blocks,
        index_rows,
        checksum: hash_from_blob(stored)?,
    })
}

fn require_commit(repo: &Repo, commit_hash: &[u8; 32]) -> Result<CommitRecord> {
    repo.commit_record(commit_hash)?
        .ok_or_else(|| Error::UnknownCommit(hash::to_hex(commit_hash)))
}

fn signature_of(shapes: &[RecordShape<'_>]) -> [u8; 32] {
    let mut buffer = Vec::new();
    for shape in shapes {
        varint::encode(shape.key.len() as u64, &mut buffer);
        buffer.extend_from_slice(shape.key);
        buffer.extend_from_slice(shape.content_hash);
    }
    hash::blake3(&buffer)
}

impl Universe {
    fn load(repo: &Repo) -> Result<Self> {
        let mut universe = Self {
            payload_len: HashMap::new(),
            block_by_signature: HashMap::new(),
        };
        let mut statement = repo
            .connection
            .prepare("SELECT block_hash, codec, raw_size, data FROM blocks")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let block_hash = repo::hash_from_blob(&row.get::<_, Vec<u8>>(0)?)?;
            let codec_id: i64 = row.get(1)?;
            let raw_size: i64 = row.get(2)?;
            let raw_size = usize::try_from(raw_size)
                .map_err(|_| Error::Inconsistent("negative raw_size".to_string()))?;
            let data: Vec<u8> = row.get(3)?;
            let raw = repo::decode_block_data(codec_id, raw_size, &data)?;
            let decoded = block::decode(&raw)?;
            let mut signature = Vec::new();
            for record in &decoded.records {
                let content_hash = hash::blake3(&record.payload);
                universe
                    .payload_len
                    .entry(content_hash)
                    .or_insert(record.payload.len());
                varint::encode(record.key.len() as u64, &mut signature);
                signature.extend_from_slice(&record.key);
                signature.extend_from_slice(&content_hash);
            }
            universe
                .block_by_signature
                .insert(hash::blake3(&signature), block_hash);
        }
        Ok(universe)
    }
}

fn head_block_hashes(repo: &Repo) -> Result<Vec<[u8; 32]>> {
    let mut statement = repo
        .connection
        .prepare("SELECT DISTINCT block_hash FROM entries")?;
    let rows = statement.query_map([], |row| row.get::<_, Vec<u8>>(0))?;
    let mut hashes = Vec::new();
    for row in rows {
        hashes.push(repo::hash_from_blob(&row?)?);
    }
    Ok(hashes)
}

fn commit_block_set(
    repo: &Repo,
    commit: &CommitRecord,
    pairs: &[(String, [u8; 32])],
    universe: &Universe,
    is_head: bool,
) -> Result<Vec<[u8; 32]>> {
    let mut hashes = if is_head {
        head_block_hashes(repo)?
    } else {
        let mut shapes = Vec::with_capacity(pairs.len());
        for (word, digest) in pairs {
            let payload_len = *universe.payload_len.get(digest).ok_or_else(|| {
                Error::UnknownBlock(format!(
                    "payload of {word} ({}) is not stored in any block",
                    hash::to_hex(digest)
                ))
            })?;
            shapes.push(RecordShape {
                key: word.as_bytes(),
                content_hash: digest,
                payload_len,
            });
        }
        let mut out = Vec::new();
        for range in cdc::boundaries_shaped(&shapes) {
            let signature = signature_of(&shapes[range]);
            let block_hash = *universe.block_by_signature.get(&signature).ok_or_else(|| {
                Error::UnknownBlock(format!(
                    "a block of commit {} is missing",
                    hash::to_hex(&commit.hash)
                ))
            })?;
            out.push(block_hash);
        }
        out
    };
    hashes.sort();
    hashes.dedup();
    if hash::merkle_root(&hashes) != commit.block_root {
        return Err(Error::Inconsistent(format!(
            "commit {} block set does not reproduce block_root",
            hash::to_hex(&commit.hash)
        )));
    }
    Ok(hashes)
}

/// Build a patch from `base` to `target`.
///
/// Only blocks that exist in the target and are absent from the base are
/// carried inline; the base side lists the blocks the receiver already has.
pub fn create(repo: &Repo, base_rev: &str, target_rev: &str) -> Result<PatchPlan> {
    let base_hash = repo.resolve(base_rev)?;
    let target_hash = repo.resolve(target_rev)?;
    if base_hash == target_hash {
        return Err(Error::NoChanges);
    }
    let base = require_commit(repo, &base_hash)?;
    let target = require_commit(repo, &target_hash)?;
    if !is_ancestor(repo, &base_hash, &target_hash)? {
        return Err(Error::InvalidArgument(
            "patch target must descend from the base commit".to_string(),
        ));
    }
    let base_pairs = repo.snapshot_pairs(&base_hash, &base)?;
    let target_pairs = repo.snapshot_pairs(&target_hash, &target)?;
    let universe = Universe::load(repo)?;
    let head = repo.head()?;
    let base_set = commit_block_set(
        repo,
        &base,
        &base_pairs,
        &universe,
        head.as_ref() == Some(&base_hash),
    )?;
    let target_set = commit_block_set(
        repo,
        &target,
        &target_pairs,
        &universe,
        head.as_ref() == Some(&target_hash),
    )?;
    let base_lookup: BTreeSet<[u8; 32]> = base_set.into_iter().collect();
    let target_lookup: BTreeSet<[u8; 32]> = target_set.into_iter().collect();
    let mut base_blocks = Vec::new();
    let mut new_blocks = Vec::new();
    for block_hash in &target_lookup {
        if base_lookup.contains(block_hash) {
            base_blocks.push(*block_hash);
        } else {
            new_blocks.push(repo.load_block_raw(block_hash)?);
        }
    }
    let mut base_index = BTreeMap::new();
    for (word, digest) in &base_pairs {
        base_index.insert(word.as_str(), *digest);
    }
    let mut index_rows = Vec::new();
    for (word, digest) in &target_pairs {
        if base_index.get(word.as_str()) != Some(digest) {
            index_rows.push((word.clone(), *digest));
        }
    }
    let mut plan = PatchPlan {
        base: base_hash,
        target: target_hash,
        flags: FLAG_INDEX_DELTA,
        base_blocks,
        new_blocks,
        index_rows,
        checksum: [0u8; 32],
    };
    plan.checksum = checksum(&encode_body(&plan)?);
    Ok(plan)
}

fn is_ancestor(repo: &Repo, ancestor: &[u8; 32], from: &[u8; 32]) -> Result<bool> {
    merge::is_ancestor(repo, ancestor, from)
}

/// Where HEAD sits relative to the patch base.
enum BasePosition {
    /// HEAD is exactly the base commit.
    Exact,
    /// HEAD descends from the base, so the patch needs a three-way merge.
    Descendant,
}

fn check_base(repo: &Repo, plan: &PatchPlan) -> Result<BasePosition> {
    if repo.commit_record(&plan.base)?.is_none() {
        return Err(Error::PatchBaseMismatch(format!(
            "base commit {} is not present",
            hash::to_hex(&plan.base)
        )));
    }
    let head = repo
        .head()?
        .ok_or_else(|| Error::PatchBaseMismatch("repository has no commits".to_string()))?;
    if head == plan.base {
        return Ok(BasePosition::Exact);
    }
    if is_ancestor(repo, &plan.base, &head)? {
        return Ok(BasePosition::Descendant);
    }
    Err(Error::PatchBaseMismatch(format!(
        "HEAD {} is not base {}",
        hash::to_hex(&head),
        hash::to_hex(&plan.base)
    )))
}

fn target_state(repo: &Repo, plan: &PatchPlan) -> Result<TargetState> {
    let mut seen = BTreeSet::new();
    let mut rows = Vec::new();
    for block_hash in &plan.base_blocks {
        if !seen.insert(*block_hash) {
            return Err(Error::PatchFormat(
                "base block list contains a duplicate".to_string(),
            ));
        }
        let raw = repo.load_block_raw(block_hash)?;
        collect_entries(*block_hash, &raw, &mut rows)?;
    }
    for raw in &plan.new_blocks {
        let block_hash = hash::blake3(raw);
        if !seen.insert(block_hash) {
            return Err(Error::PatchFormat(
                "block appears twice in the patch".to_string(),
            ));
        }
        collect_entries(block_hash, raw, &mut rows)?;
    }
    rows.sort_by(|left, right| left.word.cmp(&right.word));
    for window in rows.windows(2) {
        if window[0].word == window[1].word {
            return Err(Error::DuplicateKey(window[0].word.clone()));
        }
    }
    let mut entry_root = hash::MerkleAccumulator::new();
    for row in &rows {
        entry_root.push(hash::leaf_hash(row.word.as_bytes(), &row.content_hash));
    }
    let entry_root = entry_root.finish();
    let block_root = hash::merkle_root(&seen.iter().copied().collect::<Vec<_>>());
    let entry_count = rows.len() as u64;
    let computed = hash::commit_hash(
        &plan.base,
        &hash::ZERO_HASH,
        &entry_root,
        &block_root,
        entry_count,
    );
    if computed != plan.target {
        return Err(Error::HashMismatch(format!(
            "patch target {} but its content hashes to {}",
            hash::to_hex(&plan.target),
            hash::to_hex(&computed)
        )));
    }
    Ok(TargetState {
        rows,
        entry_root,
        block_root,
        entry_count,
    })
}

fn collect_entries(block_hash: [u8; 32], raw: &[u8], rows: &mut Vec<EntryRow>) -> Result<()> {
    let decoded = block::decode(raw)?;
    for (index, record) in decoded.records.iter().enumerate() {
        let word = std::str::from_utf8(&record.key)
            .map_err(|_| Error::PatchFormat("block key is not UTF-8".to_string()))?
            .to_string();
        let entry = Entry::decode_payload(&record.payload)?;
        let offset = decoded.offsets.get(index).copied().unwrap_or(0);
        rows.push(EntryRow {
            word,
            display: entry.display,
            block_hash,
            offset: i64::try_from(offset).unwrap_or(i64::MAX),
            length: i64::try_from(record.encoded_len()).unwrap_or(i64::MAX),
            content_hash: hash::blake3(&record.payload),
            field_hashes: cbor::encode(&repo::field_hashes_of(&record.payload)?),
        });
    }
    Ok(())
}

fn check_index_delta(repo: &Repo, plan: &PatchPlan, state: &TargetState) -> Result<()> {
    if plan.flags & FLAG_INDEX_DELTA == 0 {
        return Ok(());
    }
    let base = require_commit(repo, &plan.base)?;
    let base_pairs = repo.snapshot_pairs(&plan.base, &base)?;
    let base_index: HashMap<&str, [u8; 32]> = base_pairs
        .iter()
        .map(|(word, digest)| (word.as_str(), *digest))
        .collect();
    let expected: Vec<(String, [u8; 32])> = state
        .rows
        .iter()
        .filter(|row| base_index.get(row.word.as_str()) != Some(&row.content_hash))
        .map(|row| (row.word.clone(), row.content_hash))
        .collect();
    let mut given = plan.index_rows.clone();
    given.sort_by(|left, right| left.0.cmp(&right.0));
    if given != expected {
        return Err(Error::Inconsistent(format!(
            "patch index delta has {} rows but the target index changes {} entries",
            given.len(),
            expected.len()
        )));
    }
    Ok(())
}

fn write_target(
    repo: &mut Repo,
    plan: &PatchPlan,
    state: &TargetState,
    move_head: bool,
) -> Result<()> {
    let transaction = repo.connection.transaction()?;
    {
        let mut insert_block = transaction.prepare(
            "INSERT OR IGNORE INTO blocks(block_hash, codec, raw_size, data)
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        for raw in &plan.new_blocks {
            let data =
                zstd::bulk::compress(raw, 3).map_err(|error| Error::Io(error.to_string()))?;
            insert_block.execute(params![
                hash::blake3(raw).as_slice(),
                codec::ZSTD,
                i64::try_from(raw.len()).unwrap_or(i64::MAX),
                data,
            ])?;
        }
    }
    if move_head {
        transaction.execute("DELETE FROM entries", [])?;
        {
            let mut insert_entry = transaction.prepare(
                "INSERT INTO entries(word, display, block_hash, offset, length,
                                 content_hash, field_hashes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for row in &state.rows {
                insert_entry.execute(params![
                    &row.word,
                    &row.display,
                    row.block_hash.as_slice(),
                    row.offset,
                    row.length,
                    row.content_hash.as_slice(),
                    &row.field_hashes,
                ])?;
            }
        }
        // Keep the working set identical to the freshly written index.
        transaction.execute("DELETE FROM staging", [])?;
        {
            let mut reader = BlockReader::new(&transaction);
            let mut insert_staging = transaction.prepare(
                "INSERT INTO staging(word, display, payload, content_hash) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for row in &state.rows {
                let slice = (row.block_hash, row.offset, row.length);
                let payload = reader.record(&slice)?.payload;
                insert_staging.execute(params![
                    &row.word,
                    &row.display,
                    payload,
                    row.content_hash.as_slice(),
                ])?;
            }
        }
    }
    let message = format!("apply patch {}", hash::to_hex(&plan.target));
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        });
    transaction.execute(
        "INSERT INTO commits(commit_hash, parent, parent2, entry_root, block_root,
         entry_count, ts, author, message) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            plan.target.as_slice(),
            plan.base.as_slice(),
            hash::ZERO_HASH.as_slice(),
            state.entry_root.as_slice(),
            state.block_root.as_slice(),
            i64::try_from(state.entry_count).unwrap_or(i64::MAX),
            ts,
            DEFAULT_AUTHOR,
            message,
        ],
    )?;
    let (raw_snapshot, raw_size) = {
        let mut writer = snapshot::SnapshotWriter::new(Vec::new(), state.entry_count)?;
        for row in &state.rows {
            writer.push(&row.word, &row.content_hash)?;
        }
        writer.finish()?
    };
    let data =
        zstd::bulk::compress(&raw_snapshot, 3).map_err(|error| Error::Io(error.to_string()))?;
    transaction.execute(
        "INSERT INTO commit_index(commit_hash, codec, raw_size, data)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            plan.target.as_slice(),
            codec::ZSTD,
            i64::try_from(raw_size).unwrap_or(i64::MAX),
            data,
        ],
    )?;
    if move_head {
        transaction.execute(
            "INSERT INTO refs(name, commit_hash) VALUES ('HEAD', ?1)
             ON CONFLICT(name) DO UPDATE SET commit_hash = excluded.commit_hash",
            params![plan.target.as_slice()],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

/// Verify that a patch is internally consistent and applies to this repository.
pub fn verify(repo: &Repo, bytes: &[u8]) -> Result<PatchVerify> {
    let plan = decode(bytes)?;
    if repo.commit_record(&plan.base)?.is_none() {
        return Err(Error::PatchBaseMismatch(format!(
            "base commit {} is not present",
            hash::to_hex(&plan.base)
        )));
    }
    let target_present = repo.commit_record(&plan.target)?.is_some();
    let state = target_state(repo, &plan)?;
    check_index_delta(repo, &plan, &state)?;
    Ok(PatchVerify {
        base: plan.base,
        target: plan.target,
        base_blocks: plan.base_blocks.len(),
        new_blocks: plan.new_blocks.len(),
        index_changes: plan.index_rows.len(),
        target_present,
    })
}

/// Apply a patch to a repository whose HEAD is exactly the patch base.
///
/// Every validation happens before the single write transaction, so a rejected
/// patch leaves the repository untouched.
/// Result of applying a patch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// HEAD advanced to the target commit.
    Applied(PatchOutcome),
    /// HEAD already descends from the base, so the patch was merged three-way.
    Merged {
        /// Patch statistics.
        patch: PatchOutcome,
        /// Merge result.
        merge: Box<merge::MergeOutcome>,
    },
}

/// Apply a patch to the repository.
///
/// An exact-base repository advances to the target commit. When HEAD descends
/// from the base, the patch target is installed and merged three-way; a merge
/// that cannot be completed leaves HEAD and the working set untouched and
/// records conflicts for [`crate::merge::resolve`].
pub fn apply(repo: &mut Repo, bytes: &[u8]) -> Result<ApplyOutcome> {
    let plan = decode(bytes)?;
    if repo.commit_record(&plan.target)?.is_some() {
        return Err(Error::PatchApplied(hash::to_hex(&plan.target)));
    }
    let position = check_base(repo, &plan)?;
    let state = target_state(repo, &plan)?;
    check_index_delta(repo, &plan, &state)?;
    let patch = PatchOutcome {
        target: plan.target,
        base_blocks: plan.base_blocks.len(),
        new_blocks: plan.new_blocks.len(),
        index_changes: plan.index_rows.len(),
        bytes: bytes.len(),
    };
    match position {
        BasePosition::Exact => {
            write_target(repo, &plan, &state, true)?;
            Ok(ApplyOutcome::Applied(patch))
        }
        BasePosition::Descendant => {
            write_target(repo, &plan, &state, false)?;
            let head = repo
                .head()?
                .ok_or_else(|| Error::PatchBaseMismatch("repository has no commits".to_string()))?;
            let merge = merge::merge_commits(repo, head, plan.target, Strategy::Auto)?;
            Ok(ApplyOutcome::Merged {
                patch,
                merge: Box::new(merge),
            })
        }
    }
}
