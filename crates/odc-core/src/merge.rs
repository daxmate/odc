//! Three-way merge between commits (spec section 11.1).
//!
//! A merge compares the entry trees of two commits against their merge base.
//! Changes that touch disjoint words or disjoint payload fields are combined
//! automatically; everything else becomes a conflict that is recorded in the
//! `conflicts` table and must be resolved with [`resolve`] before
//! [`continue_merge`] can write the merge commit.

use crate::block;
use crate::cbor::{self, Value};
use crate::entry::Entry;
use crate::error::{Error, Result};
use crate::hash::{self, ZERO_HASH};
use crate::repo::{
    decode_block_data, hash_from_blob, BlockReader, CommitRecord, MergeRow, Repo, DEFAULT_AUTHOR,
};
use crate::store;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

/// `meta` row that records an unfinished merge.
pub const MERGE_META_KEY: &str = "merge";

/// How divergent changes are combined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strategy {
    /// Field-level merge; conflicting entries stop the merge.
    Auto,
    /// Keep our side for every divergent entry.
    Ours,
    /// Keep their side for every divergent entry.
    Theirs,
    /// Keep both sides; their copy moves to a `~N` suffix.
    Union,
}

impl Strategy {
    /// Stable lower-case spelling used on the command line and in JSON.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Ours => "ours",
            Self::Theirs => "theirs",
            Self::Union => "union",
        }
    }

    /// Parse the `--strategy` value.
    pub fn parse(text: &str) -> Result<Self> {
        match text {
            "auto" => Ok(Self::Auto),
            "ours" => Ok(Self::Ours),
            "theirs" => Ok(Self::Theirs),
            "union" => Ok(Self::Union),
            other => Err(Error::InvalidArgument(format!(
                "unknown merge strategy {other}"
            ))),
        }
    }
}

/// How one recorded conflict was resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// Take our payload.
    Ours,
    /// Take their payload.
    Theirs,
    /// Keep both payloads, the second under a suffixed key.
    Union,
    /// Take a payload supplied from a file.
    File,
}

impl Resolution {
    /// Stable lower-case spelling stored in the conflict detail.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ours => "ours",
            Self::Theirs => "theirs",
            Self::Union => "union",
            Self::File => "file",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "ours" => Some(Self::Ours),
            "theirs" => Some(Self::Theirs),
            "union" => Some(Self::Union),
            "file" => Some(Self::File),
            _ => None,
        }
    }
}

/// Caller-selected resolution for one conflicting word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    /// Take our payload.
    Ours,
    /// Take their payload.
    Theirs,
    /// Keep both payloads.
    Union,
    /// Take the supplied canonical CBOR payload.
    File(Vec<u8>),
}

/// A conflict as reported to the caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// Normalised word.
    pub word: String,
    /// Payload fields that disagree; empty for whole-entry conflicts.
    pub fields: Vec<String>,
    /// Base content hash (`None` when the word did not exist in the base).
    pub base: Option<[u8; 32]>,
    /// Our content hash.
    pub ours: Option<[u8; 32]>,
    /// Their content hash.
    pub theirs: Option<[u8; 32]>,
    /// How the conflict was resolved, if it has been.
    pub resolution: Option<Resolution>,
}

/// State of an unfinished merge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeState {
    /// Merge base, `None` when the histories are unrelated.
    pub base: Option<[u8; 32]>,
    /// Our commit (HEAD when the merge started).
    pub ours: [u8; 32],
    /// Their commit.
    pub theirs: [u8; 32],
    /// Requested strategy.
    pub strategy: Strategy,
}

/// How a merge ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeKind {
    /// Their commit is already contained in ours.
    UpToDate,
    /// HEAD moved to their commit without a merge commit.
    FastForward,
    /// A merge commit was written.
    Merged,
    /// A merge commit was not written because conflicts remain.
    Conflicts,
}

/// Result of [`merge`] and [`continue_merge`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeOutcome {
    /// What happened.
    pub kind: MergeKind,
    /// Merge base, when the histories are related.
    pub base: Option<[u8; 32]>,
    /// Our commit.
    pub ours: [u8; 32],
    /// Their commit.
    pub theirs: [u8; 32],
    /// Strategy that was requested.
    pub strategy: Strategy,
    /// The merge commit (fast-forward reports their commit here).
    pub commit: Option<CommitRecord>,
    /// Conflicts that remain after the operation.
    pub conflicts: Vec<Conflict>,
    /// Entries combined by a field-level merge.
    pub auto_merged: usize,
    /// Blocks written by the merge commit.
    pub new_blocks: usize,
    /// Blocks reused by the merge commit.
    pub reused_blocks: usize,
}

/// One resolved or unresolved conflict as stored in the `conflicts` table.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ConflictData {
    word: String,
    base: Option<[u8; 32]>,
    ours: Option<[u8; 32]>,
    theirs: Option<[u8; 32]>,
    fields: Vec<String>,
    base_fields: BTreeMap<String, [u8; 32]>,
    ours_fields: BTreeMap<String, [u8; 32]>,
    theirs_fields: BTreeMap<String, [u8; 32]>,
    resolution: Option<Resolution>,
    payload: Option<Vec<u8>>,
    union_key: Option<String>,
    created: i64,
}

impl ConflictData {
    fn to_conflict(&self) -> Conflict {
        Conflict {
            word: self.word.clone(),
            fields: self.fields.clone(),
            base: self.base,
            ours: self.ours,
            theirs: self.theirs,
            resolution: self.resolution,
        }
    }
}

/// Read every parent of `commit` that is present in the database.
fn parents(repo: &Repo, commit: &[u8; 32]) -> Result<Vec<[u8; 32]>> {
    let row: Option<(Vec<u8>, Vec<u8>)> = repo
        .connection
        .query_row(
            "SELECT parent, parent2 FROM commits WHERE commit_hash = ?1",
            params![commit.as_slice()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((parent, parent2)) = row else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for blob in [parent, parent2] {
        let digest = hash_from_blob(&blob)?;
        if digest != ZERO_HASH {
            out.push(digest);
        }
    }
    Ok(out)
}

/// Distance from `start` to every reachable commit, including itself.
fn distances(repo: &Repo, start: &[u8; 32]) -> Result<HashMap<[u8; 32], usize>> {
    let mut out = HashMap::new();
    let mut queue = VecDeque::new();
    out.insert(*start, 0);
    queue.push_back(*start);
    while let Some(current) = queue.pop_front() {
        let depth = out.get(&current).copied().unwrap_or_default();
        for parent in parents(repo, &current)? {
            if let std::collections::hash_map::Entry::Vacant(entry) = out.entry(parent) {
                entry.insert(depth + 1);
                queue.push_back(parent);
            }
        }
    }
    Ok(out)
}

/// True when `ancestor` is reachable from `descendant` through either parent.
pub fn is_ancestor(repo: &Repo, ancestor: &[u8; 32], descendant: &[u8; 32]) -> Result<bool> {
    if ancestor == descendant {
        return Ok(true);
    }
    let reachable = distances(repo, descendant)?;
    Ok(reachable.contains_key(ancestor))
}

/// Nearest common ancestor over the `parent`/`parent2` DAG.
///
/// Candidates are ranked by increasing `max(distance_ours, distance_theirs)`
/// (a layered BFS over both histories); ties are broken by the smaller distance
/// sum and then by the lexicographically smallest `commit_hash`, which keeps
/// the result deterministic for every caller.
pub fn merge_base(repo: &Repo, ours: &[u8; 32], theirs: &[u8; 32]) -> Result<Option<[u8; 32]>> {
    let ours_distances = distances(repo, ours)?;
    let theirs_distances = distances(repo, theirs)?;
    let mut best: Option<(usize, usize, [u8; 32])> = None;
    for (candidate, ours_depth) in &ours_distances {
        let Some(theirs_depth) = theirs_distances.get(candidate).copied() else {
            continue;
        };
        let score = (
            (*ours_depth).max(theirs_depth),
            ours_depth.saturating_add(theirs_depth),
            *candidate,
        );
        if best.is_none_or(|current| score < current) {
            best = Some(score);
        }
    }
    Ok(best.map(|(_, _, candidate)| candidate))
}

fn load_tree(repo: &Repo, commit: &[u8; 32]) -> Result<BTreeMap<String, [u8; 32]>> {
    let record = repo
        .commit_record(commit)?
        .ok_or_else(|| Error::UnknownCommit(hash::to_hex(commit)))?;
    let pairs = repo.snapshot_pairs(commit, &record)?;
    Ok(pairs.into_iter().collect())
}

/// Payload locations for a set of content hashes.
struct Payloads<'a> {
    slices: HashMap<[u8; 32], ([u8; 32], i64, i64)>,
    reader: BlockReader<'a>,
}

impl<'a> Payloads<'a> {
    fn build(connection: &'a Connection, needed: &HashSet<[u8; 32]>) -> Result<Self> {
        let mut slices = HashMap::new();
        if !needed.is_empty() {
            let mut statement =
                connection.prepare("SELECT block_hash, codec, raw_size, data FROM blocks")?;
            let mut rows = statement.query([])?;
            while let Some(row) = rows.next()? {
                let block_hash = hash_from_blob(&row.get::<_, Vec<u8>>(0)?)?;
                let codec_id: i64 = row.get(1)?;
                let raw_size: i64 = row.get(2)?;
                let data: Vec<u8> = row.get(3)?;
                let raw_size = usize::try_from(raw_size)
                    .map_err(|_| Error::Inconsistent("negative raw_size".to_string()))?;
                let raw = decode_block_data(codec_id, raw_size, &data)?;
                let decoded = block::decode(&raw)?;
                for (index, record) in decoded.records.iter().enumerate() {
                    let digest = hash::blake3(&record.payload);
                    if needed.contains(&digest) {
                        if let std::collections::hash_map::Entry::Vacant(entry) =
                            slices.entry(digest)
                        {
                            let offset = i64::try_from(decoded.offsets[index]).map_err(|_| {
                                Error::Inconsistent("block offset overflow".to_string())
                            })?;
                            let length = i64::try_from(record.encoded_len()).map_err(|_| {
                                Error::Inconsistent("record length overflow".to_string())
                            })?;
                            entry.insert((block_hash, offset, length));
                        }
                    }
                }
            }
        }
        Ok(Self {
            slices,
            reader: BlockReader::new(connection),
        })
    }

    fn slice(&self, digest: &[u8; 32]) -> Result<([u8; 32], i64, i64)> {
        self.slices.get(digest).copied().ok_or_else(|| {
            Error::UnknownBlock(format!(
                "payload {} is no longer stored; gc after the commit removed its block",
                hash::to_hex(digest)
            ))
        })
    }

    fn load(&mut self, digest: &[u8; 32]) -> Result<Vec<u8>> {
        let slice = self.slice(digest)?;
        self.load_slice(&slice)
    }

    fn load_slice(&mut self, slice: &([u8; 32], i64, i64)) -> Result<Vec<u8>> {
        Ok(self.reader.record(slice)?.payload)
    }

    fn map(&mut self, digest: Option<&[u8; 32]>) -> Result<BTreeMap<String, Value>> {
        match digest {
            Some(digest) => {
                let payload = self.load(digest)?;
                payload_map(&payload)
            }
            None => Ok(BTreeMap::new()),
        }
    }
}

fn payload_map(payload: &[u8]) -> Result<BTreeMap<String, Value>> {
    let value = cbor::decode(payload)?;
    let Value::Map(pairs) = value else {
        return Err(Error::Cbor("entry payload must be a map"));
    };
    let mut out = BTreeMap::new();
    for (key, value) in pairs {
        let Some(name) = key.as_text() else {
            return Err(Error::Cbor("entry payload keys must be text"));
        };
        out.insert(name.to_string(), value);
    }
    Ok(out)
}

fn field_hashes(fields: &BTreeMap<String, Value>) -> BTreeMap<String, [u8; 32]> {
    fields
        .iter()
        .map(|(name, value)| (name.clone(), hash::blake3(&cbor::encode(value))))
        .collect()
}

enum FieldMerge {
    Merged(Vec<u8>),
    Conflict(Vec<String>),
}

fn field_merge(
    ours_slice: &([u8; 32], i64, i64),
    base: Option<&[u8; 32]>,
    theirs: &[u8; 32],
    payloads: &mut Payloads<'_>,
) -> Result<FieldMerge> {
    let ours_payload = payloads.load_slice(ours_slice)?;
    let ours_fields = payload_map(&ours_payload)?;
    let base_fields = payloads.map(base)?;
    let theirs_fields = payloads.map(Some(theirs))?;

    let mut names: BTreeSet<&str> = BTreeSet::new();
    for map in [&ours_fields, &theirs_fields, &base_fields] {
        names.extend(map.keys().map(String::as_str));
    }

    let mut merged: Vec<(String, Value)> = Vec::new();
    let mut conflicts: Vec<String> = Vec::new();
    for name in names {
        let base_value = base_fields.get(name);
        let ours_value = ours_fields.get(name);
        let theirs_value = theirs_fields.get(name);
        if ours_value == theirs_value {
            if let Some(value) = ours_value {
                merged.push((name.to_string(), value.clone()));
            }
        } else if ours_value == base_value {
            if let Some(value) = theirs_value {
                merged.push((name.to_string(), value.clone()));
            }
        } else if theirs_value == base_value {
            if let Some(value) = ours_value {
                merged.push((name.to_string(), value.clone()));
            }
        } else {
            conflicts.push(name.to_string());
        }
    }

    if !conflicts.is_empty() {
        return Ok(FieldMerge::Conflict(conflicts));
    }
    let pairs: Vec<(Value, Value)> = merged
        .into_iter()
        .map(|(name, value)| (Value::Text(name), value))
        .collect();
    Ok(FieldMerge::Merged(cbor::encode(&Value::Map(pairs))))
}

/// One entry in our index, pointing at the block record that stores it.
struct OursEntry {
    display: String,
    slice: ([u8; 32], i64, i64),
}

fn ours_index(connection: &Connection) -> Result<HashMap<String, OursEntry>> {
    let mut statement =
        connection.prepare("SELECT word, display, block_hash, offset, length FROM entry_index")?;
    let mut rows = statement.query([])?;
    let mut out = HashMap::new();
    while let Some(row) = rows.next()? {
        let word: String = row.get(0)?;
        let display: String = row.get(1)?;
        let block = hash_from_blob(&row.get::<_, Vec<u8>>(2)?)?;
        let offset: i64 = row.get(3)?;
        let length: i64 = row.get(4)?;
        out.insert(
            word,
            OursEntry {
                display,
                slice: (block, offset, length),
            },
        );
    }
    Ok(out)
}

struct Plan {
    rows: Vec<MergeRow>,
    conflicts: Vec<ConflictData>,
    auto_merged: usize,
}

struct Planner<'a> {
    ours: &'a BTreeMap<String, [u8; 32]>,
    theirs: &'a BTreeMap<String, [u8; 32]>,
    base: &'a BTreeMap<String, [u8; 32]>,
    index: &'a HashMap<String, OursEntry>,
    payloads: Payloads<'a>,
    output: BTreeMap<String, MergeRow>,
    conflicts: Vec<ConflictData>,
    all_words: BTreeSet<String>,
    resolutions: HashMap<String, ConflictData>,
    strategy: Strategy,
    auto_merged: usize,
    now: i64,
}

impl Planner<'_> {
    fn push_ours(&mut self, word: &str, digest: &[u8; 32]) -> Result<()> {
        let entry = self.index.get(word).ok_or_else(|| {
            Error::Inconsistent(format!("word {word} is missing from the HEAD index"))
        })?;
        let row = MergeRow {
            word: word.to_string(),
            display: Some(entry.display.clone()),
            content_hash: *digest,
            slice: Some(entry.slice),
            payload: None,
        };
        self.output.insert(word.to_string(), row);
        Ok(())
    }

    fn push_theirs(&mut self, word: &str, digest: &[u8; 32]) -> Result<()> {
        let slice = self.payloads.slice(digest)?;
        let row = MergeRow {
            word: word.to_string(),
            display: None,
            content_hash: *digest,
            slice: Some(slice),
            payload: None,
        };
        self.output.insert(word.to_string(), row);
        Ok(())
    }

    fn push_theirs_at(&mut self, key: String, digest: &[u8; 32]) -> Result<()> {
        let slice = self.payloads.slice(digest)?;
        let row = MergeRow {
            word: key.clone(),
            display: None,
            content_hash: *digest,
            slice: Some(slice),
            payload: None,
        };
        self.output.insert(key, row);
        Ok(())
    }

    fn push_payload(&mut self, word: &str, payload: Vec<u8>) {
        let digest = hash::blake3(&payload);
        let row = MergeRow {
            word: word.to_string(),
            display: None,
            content_hash: digest,
            slice: None,
            payload: Some(payload),
        };
        self.output.insert(word.to_string(), row);
    }

    fn union_key(&self, word: &str) -> String {
        let mut counter = 2usize;
        loop {
            let candidate = format!("{word}~{counter}");
            if !self.all_words.contains(&candidate) && !self.output.contains_key(&candidate) {
                return candidate;
            }
            counter += 1;
        }
    }

    fn union_resolution_key(&self, word: &str, stored: Option<&str>) -> String {
        if let Some(key) = stored {
            return key.to_string();
        }
        self.union_key(word)
    }

    fn push_both(&mut self, word: &str, ours: &[u8; 32], theirs: &[u8; 32]) -> Result<()> {
        self.push_ours(word, ours)?;
        let renamed = self.union_key(word);
        self.push_theirs_at(renamed, theirs)
    }

    fn push_drop(&mut self, word: &str) {
        self.output.remove(word);
    }

    fn record_conflict(
        &mut self,
        word: &str,
        base: Option<&[u8; 32]>,
        ours: Option<&[u8; 32]>,
        theirs: Option<&[u8; 32]>,
        fields: Vec<String>,
    ) -> Result<()> {
        let base_fields = field_hashes(&self.payloads.map(base)?);
        let ours_fields = field_hashes(&self.payloads.map(ours)?);
        let theirs_fields = field_hashes(&self.payloads.map(theirs)?);
        self.conflicts.push(ConflictData {
            word: word.to_string(),
            base: base.copied(),
            ours: ours.copied(),
            theirs: theirs.copied(),
            fields,
            base_fields,
            ours_fields,
            theirs_fields,
            resolution: None,
            payload: None,
            union_key: None,
            created: self.now,
        });
        Ok(())
    }

    fn apply_resolution(
        &mut self,
        word: &str,
        kind: Resolution,
        conflict: &ConflictData,
        ours: Option<&[u8; 32]>,
        theirs: Option<&[u8; 32]>,
    ) -> Result<()> {
        match kind {
            Resolution::Ours => match ours {
                Some(digest) => self.push_ours(word, digest)?,
                None => self.push_drop(word),
            },
            Resolution::Theirs => match theirs {
                Some(digest) => self.push_theirs(word, digest)?,
                None => self.push_drop(word),
            },
            Resolution::Union => match (ours, theirs) {
                (Some(ours_hash), Some(theirs_hash)) => {
                    self.push_ours(word, ours_hash)?;
                    let renamed = self.union_resolution_key(word, conflict.union_key.as_deref());
                    self.push_theirs_at(renamed, theirs_hash)?;
                }
                (Some(ours_hash), None) => self.push_ours(word, ours_hash)?,
                (None, Some(theirs_hash)) => self.push_theirs(word, theirs_hash)?,
                (None, None) => self.push_drop(word),
            },
            Resolution::File => {
                let payload = conflict.payload.clone().ok_or_else(|| {
                    Error::InvalidArgument(format!("conflict {word} has no resolved payload"))
                })?;
                self.push_payload(word, payload);
            }
        }
        Ok(())
    }

    fn run(mut self) -> Result<Plan> {
        let words: Vec<String> = self.all_words.iter().cloned().collect();
        for word in words {
            let base = self.base.get(&word).copied();
            let ours = self.ours.get(&word).copied();
            let theirs = self.theirs.get(&word).copied();

            if ours == theirs {
                if let Some(digest) = ours {
                    self.push_ours(&word, &digest)?;
                }
                continue;
            }
            if ours == base {
                if let Some(digest) = theirs {
                    self.push_theirs(&word, &digest)?;
                }
                continue;
            }
            if theirs == base {
                if let Some(digest) = ours {
                    self.push_ours(&word, &digest)?;
                }
                continue;
            }

            let resolution = self
                .resolutions
                .get(&word)
                .and_then(|conflict| conflict.resolution.map(|kind| (kind, conflict.clone())));

            match (ours, theirs) {
                (Some(ours_hash), Some(theirs_hash)) => {
                    if let Some((kind, conflict)) = &resolution {
                        let (kind, conflict) = (*kind, conflict.clone());
                        self.apply_resolution(
                            &word,
                            kind,
                            &conflict,
                            Some(&ours_hash),
                            Some(&theirs_hash),
                        )?;
                        continue;
                    }
                    match self.strategy {
                        Strategy::Ours => self.push_ours(&word, &ours_hash)?,
                        Strategy::Theirs => self.push_theirs(&word, &theirs_hash)?,
                        Strategy::Union => self.push_both(&word, &ours_hash, &theirs_hash)?,
                        Strategy::Auto => {
                            let entry = self.index.get(&word).ok_or_else(|| {
                                Error::Inconsistent(format!(
                                    "word {word} is missing from the HEAD index"
                                ))
                            })?;
                            let slice = entry.slice;
                            match field_merge(
                                &slice,
                                base.as_ref(),
                                &theirs_hash,
                                &mut self.payloads,
                            )? {
                                FieldMerge::Merged(payload) => {
                                    self.auto_merged += 1;
                                    self.push_payload(&word, payload);
                                }
                                FieldMerge::Conflict(fields) => self.record_conflict(
                                    &word,
                                    base.as_ref(),
                                    Some(&ours_hash),
                                    Some(&theirs_hash),
                                    fields,
                                )?,
                            }
                        }
                    }
                }
                (Some(ours_hash), None) => {
                    if let Some((kind, conflict)) = &resolution {
                        let (kind, conflict) = (*kind, conflict.clone());
                        self.apply_resolution(&word, kind, &conflict, Some(&ours_hash), None)?;
                        continue;
                    }
                    match self.strategy {
                        Strategy::Theirs => self.push_drop(&word),
                        Strategy::Ours | Strategy::Union => {
                            self.push_ours(&word, &ours_hash)?;
                        }
                        Strategy::Auto => self.record_conflict(
                            &word,
                            base.as_ref(),
                            Some(&ours_hash),
                            None,
                            Vec::new(),
                        )?,
                    }
                }
                (None, Some(theirs_hash)) => {
                    if let Some((kind, conflict)) = &resolution {
                        let (kind, conflict) = (*kind, conflict.clone());
                        self.apply_resolution(&word, kind, &conflict, None, Some(&theirs_hash))?;
                        continue;
                    }
                    match self.strategy {
                        Strategy::Ours => self.push_drop(&word),
                        Strategy::Theirs | Strategy::Union => {
                            self.push_theirs(&word, &theirs_hash)?;
                        }
                        Strategy::Auto => self.record_conflict(
                            &word,
                            base.as_ref(),
                            None,
                            Some(&theirs_hash),
                            Vec::new(),
                        )?,
                    }
                }
                (None, None) => {}
            }
        }

        let mut conflicts = self.conflicts;
        conflicts.sort_by(|left, right| left.word.cmp(&right.word));
        Ok(Plan {
            rows: self.output.into_values().collect(),
            conflicts,
            auto_merged: self.auto_merged,
        })
    }
}

fn plan(
    connection: &Connection,
    base: &BTreeMap<String, [u8; 32]>,
    ours: &BTreeMap<String, [u8; 32]>,
    theirs: &BTreeMap<String, [u8; 32]>,
    strategy: Strategy,
    resolutions: HashMap<String, ConflictData>,
    now: i64,
) -> Result<Plan> {
    let index = ours_index(connection)?;
    let mut needed: HashSet<[u8; 32]> = HashSet::new();
    needed.extend(base.values().copied());
    needed.extend(ours.values().copied());
    needed.extend(theirs.values().copied());
    let mut all_words: BTreeSet<String> = BTreeSet::new();
    all_words.extend(base.keys().cloned());
    all_words.extend(ours.keys().cloned());
    all_words.extend(theirs.keys().cloned());
    let planner = Planner {
        ours,
        theirs,
        base,
        index: &index,
        payloads: Payloads::build(connection, &needed)?,
        output: BTreeMap::new(),
        conflicts: Vec::new(),
        all_words,
        resolutions,
        strategy,
        auto_merged: 0,
        now,
    };
    planner.run()
}

fn state_to_value(state: &MergeState) -> Value {
    let base = state
        .base
        .map_or(Value::Null, |digest| Value::Bytes(digest.to_vec()));
    Value::map([
        ("base", base),
        ("ours", Value::Bytes(state.ours.to_vec())),
        ("theirs", Value::Bytes(state.theirs.to_vec())),
        ("strategy", Value::Text(state.strategy.as_str().to_string())),
    ])
}

fn state_from_value(value: &Value) -> Result<MergeState> {
    let ours = value
        .get("ours")
        .and_then(Value::as_bytes)
        .ok_or_else(|| Error::Inconsistent("merge state has no ours".to_string()))?;
    let theirs = value
        .get("theirs")
        .and_then(Value::as_bytes)
        .ok_or_else(|| Error::Inconsistent("merge state has no theirs".to_string()))?;
    let base = match value.get("base") {
        Some(Value::Bytes(bytes)) => Some(hash_from_blob(bytes)?),
        _ => None,
    };
    let strategy = match value.get("strategy").and_then(Value::as_text) {
        Some(text) => Strategy::parse(text)?,
        None => Strategy::Auto,
    };
    Ok(MergeState {
        base,
        ours: hash_from_blob(ours)?,
        theirs: hash_from_blob(theirs)?,
        strategy,
    })
}

fn hash_map_value(map: &BTreeMap<String, [u8; 32]>) -> Value {
    Value::map(
        map.iter()
            .map(|(name, digest)| (name.as_str(), Value::Bytes(digest.to_vec()))),
    )
}

fn hash_map_from(value: Option<&Value>) -> Result<BTreeMap<String, [u8; 32]>> {
    let Some(Value::Map(pairs)) = value else {
        return Ok(BTreeMap::new());
    };
    let mut out = BTreeMap::new();
    for (key, item) in pairs {
        let name = key
            .as_text()
            .ok_or_else(|| Error::Inconsistent("field hash key is not text".to_string()))?;
        let bytes = item
            .as_bytes()
            .ok_or_else(|| Error::Inconsistent("field hash is not bytes".to_string()))?;
        out.insert(name.to_string(), hash_from_blob(bytes)?);
    }
    Ok(out)
}

fn opt_bytes(digest: Option<&[u8; 32]>) -> Value {
    digest.map_or(Value::Null, |hash| Value::Bytes(hash.to_vec()))
}

fn conflict_to_value(conflict: &ConflictData) -> Value {
    let fields = Value::Array(
        conflict
            .fields
            .iter()
            .map(|name| Value::Text(name.clone()))
            .collect(),
    );
    let resolution = conflict
        .resolution
        .map_or(Value::Null, |kind| Value::Text(kind.as_str().to_string()));
    let payload = conflict
        .payload
        .as_ref()
        .map_or(Value::Null, |bytes| Value::Bytes(bytes.clone()));
    let union_key = conflict
        .union_key
        .as_ref()
        .map_or(Value::Null, |key| Value::Text(key.clone()));
    Value::map([
        ("base", opt_bytes(conflict.base.as_ref())),
        ("base_fields", hash_map_value(&conflict.base_fields)),
        ("fields", fields),
        ("ours", opt_bytes(conflict.ours.as_ref())),
        ("ours_fields", hash_map_value(&conflict.ours_fields)),
        ("payload", payload),
        ("resolution", resolution),
        ("theirs", opt_bytes(conflict.theirs.as_ref())),
        ("theirs_fields", hash_map_value(&conflict.theirs_fields)),
        ("ts", Value::Int(conflict.created)),
        ("union_key", union_key),
    ])
}

fn digest_value(value: Option<&Value>) -> Result<Option<[u8; 32]>> {
    match value {
        Some(Value::Bytes(bytes)) => Ok(Some(hash_from_blob(bytes)?)),
        _ => Ok(None),
    }
}

fn conflict_from_value(word: &str, value: &Value) -> Result<ConflictData> {
    let fields = match value.get("fields") {
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_text()
                    .map(str::to_string)
                    .ok_or_else(|| Error::Inconsistent("conflict field is not text".to_string()))
            })
            .collect::<Result<Vec<String>>>()?,
        _ => Vec::new(),
    };
    let resolution = value
        .get("resolution")
        .and_then(Value::as_text)
        .and_then(Resolution::parse);
    let payload = match value.get("payload") {
        Some(Value::Bytes(bytes)) => Some(bytes.clone()),
        _ => None,
    };
    let union_key = value
        .get("union_key")
        .and_then(Value::as_text)
        .map(str::to_string);
    let created = value.get("ts").and_then(Value::as_int).unwrap_or(0);
    Ok(ConflictData {
        word: word.to_string(),
        base: digest_value(value.get("base"))?,
        ours: digest_value(value.get("ours"))?,
        theirs: digest_value(value.get("theirs"))?,
        fields,
        base_fields: hash_map_from(value.get("base_fields"))?,
        ours_fields: hash_map_from(value.get("ours_fields"))?,
        theirs_fields: hash_map_from(value.get("theirs_fields"))?,
        resolution,
        payload,
        union_key,
        created,
    })
}

fn write_state(connection: &Connection, state: &MergeState) -> Result<()> {
    let encoded = cbor::encode(&state_to_value(state));
    store::set_meta(connection, MERGE_META_KEY, &encoded)
}

fn clear_state(connection: &Connection) -> Result<()> {
    connection.execute("DELETE FROM meta WHERE key = ?1", params![MERGE_META_KEY])?;
    connection.execute("DELETE FROM conflicts", [])?;
    Ok(())
}

/// The unfinished merge, if any.
pub fn pending_state(repo: &Repo) -> Result<Option<MergeState>> {
    let Some(bytes) = store::get_meta(&repo.connection, MERGE_META_KEY)? else {
        return Ok(None);
    };
    let value = cbor::decode(&bytes)?;
    Ok(Some(state_from_value(&value)?))
}

fn read_conflicts(connection: &Connection) -> Result<Vec<ConflictData>> {
    let mut statement =
        connection.prepare("SELECT word, detail FROM conflicts ORDER BY word ASC")?;
    let mut rows = statement.query([])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let word: String = row.get(0)?;
        let detail: Vec<u8> = row.get(1)?;
        let value = cbor::decode(&detail)?;
        out.push(conflict_from_value(&word, &value)?);
    }
    Ok(out)
}

fn write_conflicts(connection: &Connection, conflicts: &[ConflictData]) -> Result<()> {
    connection.execute("DELETE FROM conflicts", [])?;
    let mut statement =
        connection.prepare("INSERT INTO conflicts(word, detail) VALUES (?1, ?2)")?;
    for conflict in conflicts {
        let detail = cbor::encode(&conflict_to_value(conflict));
        statement.execute(params![&conflict.word, detail])?;
    }
    Ok(())
}

/// Conflicts recorded by the unfinished merge.
pub fn conflicts(repo: &Repo) -> Result<Vec<Conflict>> {
    Ok(read_conflicts(&repo.connection)?
        .iter()
        .map(ConflictData::to_conflict)
        .collect())
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}

fn fast_forward(repo: &mut Repo, ours: [u8; 32], theirs: [u8; 32]) -> Result<MergeOutcome> {
    let tree = load_tree(repo, &theirs)?;
    let needed: HashSet<[u8; 32]> = tree.values().copied().collect();
    let mut rows = Vec::with_capacity(tree.len());
    {
        let payloads = Payloads::build(&repo.connection, &needed)?;
        for (word, digest) in &tree {
            rows.push(MergeRow {
                word: word.clone(),
                display: None,
                content_hash: *digest,
                slice: Some(payloads.slice(digest)?),
                payload: None,
            });
        }
    }
    repo.fast_forward_tree(&mut rows, &theirs)?;
    Ok(MergeOutcome {
        kind: MergeKind::FastForward,
        base: Some(ours),
        ours,
        theirs,
        strategy: Strategy::Auto,
        commit: repo.commit_record(&theirs)?,
        conflicts: Vec::new(),
        auto_merged: 0,
        new_blocks: 0,
        reused_blocks: 0,
    })
}

/// Merge `theirs` into `ours` (which must be HEAD).
///
/// A fast-forward moves HEAD without a merge commit. A clean merge writes a
/// commit whose `parent2` is `theirs`. Divergent entries either merge at field
/// level or become conflicts: no commit is written while any conflict remains.
pub fn merge(
    repo: &mut Repo,
    ours: &str,
    theirs: &str,
    strategy: Strategy,
) -> Result<MergeOutcome> {
    if let Some(state) = pending_state(repo)? {
        return Err(Error::MergeInProgress(format!(
            "merge {} into {} is unfinished",
            hash::to_hex(&state.theirs),
            hash::to_hex(&state.ours)
        )));
    }
    let ours_hash = repo.resolve(ours)?;
    let theirs_hash = repo.resolve(theirs)?;
    let head = repo
        .head()?
        .ok_or_else(|| Error::InvalidArgument("repository has no commits".to_string()))?;
    if ours_hash != head {
        return Err(Error::InvalidArgument(format!(
            "merge target must be HEAD, got {}",
            &hash::to_hex(&ours_hash)[..12]
        )));
    }
    if !repo.status()?.is_clean() {
        return Err(Error::InvalidArgument(
            "working set has uncommitted changes".to_string(),
        ));
    }
    merge_commits(repo, ours_hash, theirs_hash, strategy)
}

/// Merge two commits that are already stored in the repository.
pub fn merge_commits(
    repo: &mut Repo,
    ours: [u8; 32],
    theirs: [u8; 32],
    strategy: Strategy,
) -> Result<MergeOutcome> {
    if is_ancestor(repo, &theirs, &ours)? {
        let base = merge_base(repo, &ours, &theirs)?;
        return Ok(MergeOutcome {
            kind: MergeKind::UpToDate,
            base,
            ours,
            theirs,
            strategy,
            commit: repo.commit_record(&ours)?,
            conflicts: Vec::new(),
            auto_merged: 0,
            new_blocks: 0,
            reused_blocks: 0,
        });
    }
    if is_ancestor(repo, &ours, &theirs)? {
        let mut outcome = fast_forward(repo, ours, theirs)?;
        outcome.strategy = strategy;
        return Ok(outcome);
    }
    let base = merge_base(repo, &ours, &theirs)?;
    let base_tree = match base {
        Some(hash) => load_tree(repo, &hash)?,
        None => BTreeMap::new(),
    };
    let ours_tree = load_tree(repo, &ours)?;
    let theirs_tree = load_tree(repo, &theirs)?;
    let timestamp = now();
    let plan = plan(
        &repo.connection,
        &base_tree,
        &ours_tree,
        &theirs_tree,
        strategy,
        HashMap::new(),
        timestamp,
    )?;

    if !plan.conflicts.is_empty() {
        write_state(
            &repo.connection,
            &MergeState {
                base,
                ours,
                theirs,
                strategy,
            },
        )?;
        write_conflicts(&repo.connection, &plan.conflicts)?;
        let conflicts = plan
            .conflicts
            .iter()
            .map(ConflictData::to_conflict)
            .collect();
        return Ok(MergeOutcome {
            kind: MergeKind::Conflicts,
            base,
            ours,
            theirs,
            strategy,
            commit: None,
            conflicts,
            auto_merged: plan.auto_merged,
            new_blocks: 0,
            reused_blocks: 0,
        });
    }

    let message = format!("merge {}", &hash::to_hex(&theirs)[..12]);
    let mut rows = plan.rows;
    let outcome = repo.write_merge_commit(
        &ours,
        &theirs,
        &mut rows,
        &message,
        DEFAULT_AUTHOR,
        timestamp,
    )?;
    clear_state(&repo.connection)?;
    Ok(MergeOutcome {
        kind: MergeKind::Merged,
        base,
        ours,
        theirs,
        strategy,
        commit: Some(outcome.commit),
        conflicts: Vec::new(),
        auto_merged: plan.auto_merged,
        new_blocks: outcome.new_blocks,
        reused_blocks: outcome.reused_blocks,
    })
}

/// Record how one conflicted word should be resolved.
pub fn resolve(repo: &mut Repo, word: &str, choice: Choice) -> Result<Conflict> {
    let state = pending_state(repo)?.ok_or(Error::NoMergeInProgress)?;
    let key = crate::entry::normalize_key(word);
    let mut conflicts = read_conflicts(&repo.connection)?;
    let position = conflicts
        .iter()
        .position(|conflict| conflict.word == key)
        .ok_or_else(|| {
            Error::InvalidArgument(format!("no conflict for word {key} in the merge"))
        })?;
    match &choice {
        Choice::Ours => conflicts[position].resolution = Some(Resolution::Ours),
        Choice::Theirs => conflicts[position].resolution = Some(Resolution::Theirs),
        Choice::Union => {
            let mut used: BTreeSet<String> = BTreeSet::new();
            used.extend(conflicts.iter().map(|item| item.word.clone()));
            for tree in [
                load_tree(repo, &state.ours)?,
                load_tree(repo, &state.theirs)?,
            ] {
                used.extend(tree.into_keys());
            }
            if let Some(base) = state.base {
                used.extend(load_tree(repo, &base)?.into_keys());
            }
            let mut counter = 2usize;
            let union_key = loop {
                let candidate = format!("{key}~{counter}");
                if !used.contains(&candidate) {
                    break candidate;
                }
                counter += 1;
            };
            conflicts[position].resolution = Some(Resolution::Union);
            conflicts[position].union_key = Some(union_key);
        }
        Choice::File(bytes) => {
            let value = cbor::decode(bytes)?;
            let Value::Map(_) = value else {
                return Err(Error::InvalidArgument(
                    "resolved payload must be a CBOR map".to_string(),
                ));
            };
            Entry::decode_payload(bytes)?;
            conflicts[position].resolution = Some(Resolution::File);
            conflicts[position].payload = Some(cbor::encode(&value));
        }
    }
    let resolved = conflicts[position].to_conflict();
    write_conflicts(&repo.connection, &conflicts)?;
    Ok(resolved)
}

/// Write the merge commit after every conflict has been resolved.
pub fn continue_merge(repo: &mut Repo, author: &str, ts: i64) -> Result<MergeOutcome> {
    let state = pending_state(repo)?.ok_or(Error::NoMergeInProgress)?;
    let recorded = read_conflicts(&repo.connection)?;
    let unresolved = recorded
        .iter()
        .filter(|conflict| conflict.resolution.is_none())
        .count();
    if unresolved > 0 {
        return Err(Error::UnresolvedConflicts(unresolved));
    }
    let resolutions: HashMap<String, ConflictData> = recorded
        .into_iter()
        .map(|conflict| (conflict.word.clone(), conflict))
        .collect();
    let base_tree = match state.base {
        Some(hash) => load_tree(repo, &hash)?,
        None => BTreeMap::new(),
    };
    let ours_tree = load_tree(repo, &state.ours)?;
    let theirs_tree = load_tree(repo, &state.theirs)?;
    let plan = plan(
        &repo.connection,
        &base_tree,
        &ours_tree,
        &theirs_tree,
        state.strategy,
        resolutions,
        ts,
    )?;
    if !plan.conflicts.is_empty() {
        write_conflicts(&repo.connection, &plan.conflicts)?;
        return Err(Error::UnresolvedConflicts(plan.conflicts.len()));
    }
    let message = format!("merge {}", &hash::to_hex(&state.theirs)[..12]);
    let mut rows = plan.rows;
    let outcome =
        repo.write_merge_commit(&state.ours, &state.theirs, &mut rows, &message, author, ts)?;
    clear_state(&repo.connection)?;
    Ok(MergeOutcome {
        kind: MergeKind::Merged,
        base: state.base,
        ours: state.ours,
        theirs: state.theirs,
        strategy: state.strategy,
        commit: Some(outcome.commit),
        conflicts: Vec::new(),
        auto_merged: plan.auto_merged,
        new_blocks: outcome.new_blocks,
        reused_blocks: outcome.reused_blocks,
    })
}
