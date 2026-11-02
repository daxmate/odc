//! High level repository operations: init, import, commit, log, show, diff,
//! status and verify.

use crate::block::{self, Record};
use crate::cbor::{self, Value};
use crate::cdc;
use crate::entry::{normalize_key, Entry};
use crate::error::{Error, Result};
use crate::hash::{self, ZERO_HASH};
use crate::index::{self, IndexChain};
use crate::pack::{self, PackOptions, PackReport};
use crate::snapshot;
use crate::store::{self, codec, DB_FILE};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Default author recorded when none is supplied.
pub const DEFAULT_AUTHOR: &str = "odc";

/// Cached payload bytes a commit must free before it rewrites the database to
/// give the space back to the file system.
const VACUUM_THRESHOLD: i64 = 64 * 1024 * 1024;

/// A checked-out repository backed by `odc.db`.
pub struct Repo {
    pub(crate) connection: Connection,
    dir: PathBuf,
    db_path: PathBuf,
    read_only: bool,
}

/// Result of `import`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// Rows inserted or replaced in the working set.
    pub written: usize,
    /// Rows removed by `--prune`.
    pub removed: usize,
    /// Working set size after the import.
    pub total: usize,
}

/// Working set relative to HEAD.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// Entries present only in the working set.
    pub added: usize,
    /// Entries whose content hash differs from HEAD.
    pub modified: usize,
    /// Entries present in HEAD but deleted from the working set.
    pub removed: usize,
    /// Working set size.
    pub total: usize,
    /// Entries materialised in the committed index.
    pub entries: usize,
    /// Blocks stored by the repository.
    pub blocks: usize,
    /// Current HEAD hash, if any.
    pub head: Option<String>,
}

impl Status {
    /// True when the working set equals HEAD.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.added == 0 && self.modified == 0 && self.removed == 0
    }
}

/// Outcome of a single working-set write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryWriteOutcome {
    /// The key did not exist in HEAD; a new working-set row was written.
    Added,
    /// The key already existed in HEAD; its working-set row was replaced.
    Modified,
    /// The key was removed from the working set.
    Deleted,
    /// The key was not in the working set, so there was nothing to delete.
    NotFound,
}

/// How one key differs between HEAD and the working set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    /// Present in the working set only.
    Added,
    /// Present on both sides with a different content hash.
    Modified,
    /// Present in HEAD only.
    Deleted,
}

/// One row of [`Repo::working_changes`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeRow {
    /// Normalised key.
    pub key: String,
    /// What happened to the key.
    pub kind: ChangeKind,
}

/// Kind of change reported by [`Repo::entry_diff`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffKind {
    /// Present in the working set only.
    Added,
    /// Present on both sides, with at least one changed field.
    Modified,
    /// Present in HEAD only.
    Deleted,
    /// Present in neither HEAD nor the working set.
    Absent,
}

/// One field of an [`EntryDiff`].
///
/// `before` and `after` are `None` when the field does not exist on that
/// side, so an added definition shows `before: None` for every attribute.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldDiff {
    /// Field name, e.g. `display`, `defs[0].sense` or `extra.source`.
    pub field: String,
    /// Value in HEAD, if the field exists there.
    pub before: Option<Value>,
    /// Value in the working set, if the field exists there.
    pub after: Option<Value>,
    /// Whether `before` and `after` differ.
    pub changed: bool,
}

/// Field-level diff of one entry between HEAD and the working set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryDiff {
    /// Normalised key.
    pub key: String,
    /// What happened to the entry as a whole.
    pub kind: DiffKind,
    /// Every field present on either side, in a stable order.
    pub fields: Vec<FieldDiff>,
}

/// A stored commit row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitRecord {
    /// Commit hash.
    pub hash: [u8; 32],
    /// First parent (zero for genesis).
    pub parent: [u8; 32],
    /// Second parent (zero unless the commit is a merge).
    pub parent2: [u8; 32],
    /// Merkle root over entries.
    pub entry_root: [u8; 32],
    /// Merkle root over blocks.
    pub block_root: [u8; 32],
    /// Number of entries.
    pub entry_count: u64,
    /// Unix timestamp in seconds.
    pub ts: i64,
    /// Free-form author string.
    pub author: String,
    /// Commit message.
    pub message: String,
}

/// Outcome of `commit`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitOutcome {
    /// The new commit row.
    pub commit: CommitRecord,
    /// Blocks newly written by this commit.
    pub new_blocks: usize,
    /// Blocks that already existed and were reused.
    pub reused_blocks: usize,
    /// Entries in the snapshot.
    pub entries: usize,
}

/// Word-level diff between two commits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diff {
    /// Left revision (resolved hash, hex).
    pub from: String,
    /// Right revision (resolved hash, hex).
    pub to: String,
    /// Words present only in `to`.
    pub added: Vec<String>,
    /// Words present only in `from`.
    pub removed: Vec<String>,
    /// Words present in both with a different content hash.
    pub modified: Vec<String>,
}

/// Storage shape of the commit index table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexStorage {
    /// Entries stored as full snapshots.
    pub full: usize,
    /// Entries stored as deltas.
    pub delta: usize,
    /// Uncompressed bytes over every entry.
    pub raw_bytes: u64,
    /// Compressed bytes over every entry.
    pub compressed_bytes: u64,
}

/// Result of `verify`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyReport {
    /// True when `errors` is empty.
    pub ok: bool,
    /// Blocks inspected.
    pub blocks: usize,
    /// Entries inspected.
    pub entries: usize,
    /// Commits inspected.
    pub commits: usize,
    /// Human readable problems; empty on success.
    pub errors: Vec<String>,
}

/// zstd level used for every block written by the reference implementation.
///
/// Level 3 was the original default; level 9 buys roughly 8% smaller blocks
/// on dictionary-sized payloads for a few seconds of extra import time, and
/// costs nothing on the read path (the frame header carries the level).
pub(crate) const ZSTD_LEVEL: i32 = 9;

fn compress(data: &[u8]) -> Result<Vec<u8>> {
    zstd::bulk::compress(data, ZSTD_LEVEL).map_err(|error| Error::Io(error.to_string()))
}

fn decompress(data: &[u8], raw_size: usize) -> Result<Vec<u8>> {
    zstd::bulk::decompress(data, raw_size).map_err(|error| Error::Io(error.to_string()))
}

/// Word and digest of one commit_digests row.
fn digest_row(row: &rusqlite::Row<'_>) -> Result<(String, [u8; 32])> {
    let word: String = row.get(0)?;
    let digest = hash_from_blob(&row.get::<_, Vec<u8>>(1)?)?;
    Ok((word, digest))
}

/// Fill the commit_delta temp table with the operations that turn the parent
/// index into the index held in commit_digests.
///
/// Both sides arrive in word order, so the difference is a single merge over
/// two streams; neither index is materialised as a map.
fn build_delta(transaction: &rusqlite::Transaction<'_>, parent: &[u8; 32]) -> Result<u64> {
    enum Step {
        AddNew,
        RemoveParent,
        Equal,
    }

    let mut parent_index = index::open_index(transaction, parent, snapshot::MAX_CHAIN_DEPTH)?;
    let mut next_parent = parent_index.next_pair()?;
    let mut operations = 0u64;
    {
        let mut insert = transaction
            .prepare("INSERT INTO commit_delta(word, kind, digest) VALUES (?1, ?2, ?3)")?;
        let mut select =
            transaction.prepare("SELECT word, digest FROM commit_digests ORDER BY word ASC")?;
        let mut rows = select.query([])?;
        let mut next_new = match rows.next()? {
            Some(row) => Some(digest_row(row)?),
            None => None,
        };
        loop {
            let step = match (&next_parent, &next_new) {
                (None, None) => break,
                (None, Some(_)) => Step::AddNew,
                (Some(_), None) => Step::RemoveParent,
                (Some((parent_word, _)), Some((word, _))) => match word.cmp(parent_word) {
                    std::cmp::Ordering::Less => Step::AddNew,
                    std::cmp::Ordering::Greater => Step::RemoveParent,
                    std::cmp::Ordering::Equal => Step::Equal,
                },
            };
            match step {
                Step::AddNew => {
                    let (word, digest) = next_new.take().expect("new pair was present");
                    insert.execute(params![word, 0_i64, digest.as_slice()])?;
                    operations += 1;
                    next_new = match rows.next()? {
                        Some(row) => Some(digest_row(row)?),
                        None => None,
                    };
                }
                Step::RemoveParent => {
                    let (word, _) = next_parent.take().expect("parent pair was present");
                    insert.execute(params![word, 2_i64, Option::<Vec<u8>>::None])?;
                    operations += 1;
                    next_parent = parent_index.next_pair()?;
                }
                Step::Equal => {
                    let (word, digest) = next_new.take().expect("new pair was present");
                    let (_, parent_digest) = next_parent.take().expect("parent pair was present");
                    next_parent = parent_index.next_pair()?;
                    next_new = match rows.next()? {
                        Some(row) => Some(digest_row(row)?),
                        None => None,
                    };
                    if digest != parent_digest {
                        insert.execute(params![word, 1_i64, digest.as_slice()])?;
                        operations += 1;
                    }
                }
            }
        }
    }
    Ok(operations)
}

/// Finish a zstd entry into temp path and report (raw bytes, compressed bytes).
fn finish_entry<W: Write>(
    writer: snapshot::SnapshotWriter<zstd::stream::write::Encoder<'static, W>>,
    path: &Path,
) -> Result<(u64, u64)> {
    let (encoder, raw_size) = writer.finish()?;
    let mut buffered = encoder
        .finish()
        .map_err(|error| Error::Io(error.to_string()))?;
    buffered.flush()?;
    let compressed_size = std::fs::metadata(path)?.len();
    Ok((raw_size, compressed_size))
}

/// True when `staging` was dropped after a commit and therefore mirrors HEAD.
fn workset_is_clean(connection: &Connection) -> Result<bool> {
    let value = store::get_meta(connection, store::META_WORKSET_CLEAN)?;
    Ok(matches!(value.as_deref(), Some(b"1")))
}

/// Drop the "working set equals HEAD" marker after anything wrote to staging.
pub(crate) fn mark_workset_dirty(connection: &Connection) -> Result<()> {
    connection.execute(
        "DELETE FROM meta WHERE key = ?1",
        [store::META_WORKSET_CLEAN],
    )?;
    Ok(())
}

/// Text value for a field that is always present.
fn text_value(text: &str) -> Value {
    Value::Text(text.to_string())
}

/// Text value for a field that may be explicitly null (raw_html).
fn nullable_text_value(text: Option<&str>) -> Value {
    text.map_or(Value::Null, text_value)
}

/// Canonical CBOR array for a list of strings.
fn text_array(values: &[String]) -> Value {
    Value::Array(values.iter().cloned().map(Value::Text).collect())
}

/// Append one field to a diff, computing changed from the two sides.
fn push_field(
    fields: &mut Vec<FieldDiff>,
    field: String,
    before: Option<Value>,
    after: Option<Value>,
) {
    let changed = before != after;
    fields.push(FieldDiff {
        field,
        before,
        after,
        changed,
    });
}

/// Flatten two entries into comparable fields, in a stable order.
///
/// defs is indexed by position so an inserted definition shows every
/// attribute of the shifted ones as changed; a definition that exists on one
/// side only yields None for every attribute on the other.
fn diff_fields(before: Option<&Entry>, after: Option<&Entry>) -> Vec<FieldDiff> {
    let mut fields = Vec::new();
    push_field(
        &mut fields,
        "display".to_string(),
        before.map(|entry| text_value(&entry.display)),
        after.map(|entry| text_value(&entry.display)),
    );
    push_field(
        &mut fields,
        "phonetic".to_string(),
        before.map(|entry| text_array(&entry.phonetic)),
        after.map(|entry| text_array(&entry.phonetic)),
    );
    let defs_before = before.map_or(0, |entry| entry.defs.len());
    let defs_after = after.map_or(0, |entry| entry.defs.len());
    for index in 0..defs_before.max(defs_after) {
        let old = before.and_then(|entry| entry.defs.get(index));
        let new = after.and_then(|entry| entry.defs.get(index));
        push_field(
            &mut fields,
            format!("defs[{index}].pos"),
            old.map(|def| text_value(&def.pos)),
            new.map(|def| text_value(&def.pos)),
        );
        push_field(
            &mut fields,
            format!("defs[{index}].sense"),
            old.map(|def| text_value(&def.sense)),
            new.map(|def| text_value(&def.sense)),
        );
        push_field(
            &mut fields,
            format!("defs[{index}].examples"),
            old.map(|def| text_array(&def.examples)),
            new.map(|def| text_array(&def.examples)),
        );
        push_field(
            &mut fields,
            format!("defs[{index}].labels"),
            old.map(|def| text_array(&def.labels)),
            new.map(|def| text_array(&def.labels)),
        );
    }
    push_field(
        &mut fields,
        "raw_html".to_string(),
        before.map(|entry| nullable_text_value(entry.raw_html.as_deref())),
        after.map(|entry| nullable_text_value(entry.raw_html.as_deref())),
    );
    let mut names: Vec<&str> = Vec::new();
    for entry in [before, after].into_iter().flatten() {
        for (name, _) in &entry.extra {
            if !names.contains(&name.as_str()) {
                names.push(name.as_str());
            }
        }
    }
    names.sort_unstable();
    for name in names {
        let old = before
            .and_then(|entry| entry.extra.iter().find(|(key, _)| key == name))
            .map(|(_, value)| value.clone());
        let new = after
            .and_then(|entry| entry.extra.iter().find(|(key, _)| key == name))
            .map(|(_, value)| value.clone());
        push_field(&mut fields, format!("extra.{name}"), old, new);
    }
    fields
}

pub(crate) fn hash_from_blob(bytes: &[u8]) -> Result<[u8; 32]> {
    bytes
        .try_into()
        .map_err(|_| Error::Inconsistent("hash column is not 32 bytes".to_string()))
}

impl Repo {
    /// Create a new repository in `dir` and write the genesis commit.
    pub fn init(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        let db_path = dir.join(DB_FILE);
        if db_path.exists() {
            return Err(Error::AlreadyInitialized(dir.display().to_string()));
        }
        std::fs::create_dir_all(&dir)?;
        let connection = store::open(&db_path)?;
        store::create_schema(&connection)?;
        store::set_meta(
            &connection,
            "format_version",
            store::FORMAT_VERSION.as_bytes(),
        )?;
        store::ensure_resources_table(&connection)?;
        let created = chrono_free_now();
        store::set_meta(&connection, "created_at", created.to_string().as_bytes())?;
        let repo = Self {
            connection,
            dir,
            db_path,
            read_only: false,
        };
        repo.write_genesis()?;
        Ok(repo)
    }

    /// Open an existing repository.
    ///
    /// `path` may be a repository directory (containing [`DB_FILE`]) or a
    /// single-file export produced by [`Repo::pack`].
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let dir = path.as_ref().to_path_buf();
        let db_path = if dir.is_file() {
            dir.clone()
        } else {
            dir.join(DB_FILE)
        };
        if !db_path.exists() {
            return Err(Error::NotARepository(dir.display().to_string()));
        }
        let connection = store::open(&db_path)?;
        store::check_version(&connection)?;
        store::migrate(&connection)?;
        let version = store::schema_version(&connection)?;
        store::install_entry_view(&connection, version)?;
        store::ensure_resources_table(&connection)?;
        // A pack is a single distribution file: writes would have nowhere to
        // stage a new commit and must fail with an explicit error instead of
        // an obscure I/O failure.
        let read_only = dir.is_file();
        Ok(Self {
            connection,
            dir,
            db_path,
            read_only,
        })
    }

    /// True when the repository was opened from a packed distribution file.
    #[must_use]
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Repository directory (or the exported file when opened from a pack).
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Whether this repository starts at HEAD and expects missing parents.
    pub fn is_shallow(&self) -> Result<bool> {
        pack::is_shallow(&self.connection)
    }

    /// Export reachable objects into a single portable file.
    pub fn pack(&self, out: impl AsRef<Path>, options: PackOptions) -> Result<PackReport> {
        let head = self.head()?.ok_or_else(|| {
            Error::InvalidArgument("pack requires at least one commit".to_string())
        })?;
        let head_index = if options.shallow || pack::is_shallow(&self.connection)? {
            self.materialized_head_index(&head)?
        } else {
            None
        };
        pack::pack(
            &self.connection,
            &self.db_path,
            head,
            out.as_ref(),
            options,
            head_index
                .as_ref()
                .map(|(raw_size, bytes)| (&head, bytes.as_slice(), *raw_size)),
        )
    }

    fn write_genesis(&self) -> Result<()> {
        let commit = CommitRecord {
            hash: hash::commit_hash(&ZERO_HASH, &ZERO_HASH, &ZERO_HASH, &ZERO_HASH, 0),
            parent: ZERO_HASH,
            parent2: ZERO_HASH,
            entry_root: ZERO_HASH,
            block_root: ZERO_HASH,
            entry_count: 0,
            ts: chrono_free_now(),
            author: DEFAULT_AUTHOR.to_string(),
            message: "genesis".to_string(),
        };
        self.insert_commit(&commit)?;
        store::set_meta(&self.connection, store::META_WORKSET_CLEAN, b"1")?;
        let snapshot = {
            let writer = snapshot::SnapshotWriter::full(Vec::new(), 0)?;
            let (buffer, _) = writer.finish()?;
            buffer
        };
        let compressed = compress(&snapshot)?;
        self.connection.execute(
            "INSERT INTO commit_index(commit_hash, codec, raw_size, data) VALUES (?1, ?2, ?3, ?4)",
            params![
                commit.hash.as_slice(),
                codec::ZSTD,
                snapshot.len() as i64,
                compressed
            ],
        )?;
        self.connection.execute(
            "INSERT INTO refs(name, commit_hash) VALUES ('HEAD', ?1)",
            params![commit.hash.as_slice()],
        )?;
        Ok(())
    }

    fn insert_commit(&self, commit: &CommitRecord) -> Result<()> {
        self.connection.execute(
            "INSERT INTO commits(commit_hash, parent, parent2, entry_root, block_root,
                                 entry_count, ts, author, message)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                commit.hash.as_slice(),
                commit.parent.as_slice(),
                commit.parent2.as_slice(),
                commit.entry_root.as_slice(),
                commit.block_root.as_slice(),
                commit.entry_count as i64,
                commit.ts,
                commit.author,
                commit.message,
            ],
        )?;
        Ok(())
    }

    /// Current HEAD hash.
    pub fn head(&self) -> Result<Option<[u8; 32]>> {
        let value: Option<Vec<u8>> = self
            .connection
            .query_row(
                "SELECT commit_hash FROM refs WHERE name = 'HEAD'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        value.map(|bytes| hash_from_blob(&bytes)).transpose()
    }

    /// Rewrite a version 1 database into the current layout before writing.
    ///
    /// Reading a v1 repository never touches the file; the first write is the
    /// point where the layout is upgraded and `meta.schema_version` becomes 2.
    pub(crate) fn ensure_writable(&mut self) -> Result<()> {
        if store::upgrade_to_v2(&self.connection)? {
            // The rewrite leaves the old table pages on the free list; give the
            // space back once instead of carrying it for the life of the file.
            self.connection.execute_batch("VACUUM")?;
        }
        Ok(())
    }

    /// [`Repo::ensure_writable`] plus the pack guard for editing.
    ///
    /// A packed distribution file can be maintained in place (`gc`) but not
    /// edited: a single file has nowhere to stage a commit next to it.
    fn ensure_editable(&mut self) -> Result<()> {
        if self.read_only {
            return Err(Error::ReadOnly(self.dir.display().to_string()));
        }
        self.ensure_writable()
    }

    /// Rebuild the working set from HEAD when the previous commit dropped it.
    ///
    /// Only committed rows are copied back, and only their index metadata: the
    /// payload of a committed entry is read from its block on demand.
    fn ensure_workset(&mut self) -> Result<()> {
        if !workset_is_clean(&self.connection)? {
            return Ok(());
        }
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT OR REPLACE INTO staging(word, display, content_hash)
             SELECT word, display, content_hash FROM entries",
            [],
        )?;
        mark_workset_dirty(&transaction)?;
        transaction.commit()?;
        Ok(())
    }

    /// Insert or replace entries in the working set.
    pub fn import(&mut self, entries: &[Entry], prune: bool) -> Result<ImportReport> {
        self.ensure_editable()?;
        self.ensure_workset()?;
        let transaction = self.connection.transaction()?;
        {
            let mut insert = transaction.prepare(
                "INSERT INTO staging(word, display, payload, content_hash) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(word) DO UPDATE SET display = excluded.display,
                                                 payload = excluded.payload,
                                                 content_hash = excluded.content_hash",
            )?;
            for entry in entries {
                insert.execute(params![
                    entry.word,
                    entry.display,
                    entry.encode_payload(),
                    entry.content_hash()[..store::DIGEST_PREFIX].to_vec()
                ])?;
            }
        }
        let mut removed = 0usize;
        if prune {
            transaction.execute_batch(
                "CREATE TEMP TABLE imported(word TEXT PRIMARY KEY);
                 DELETE FROM imported;",
            )?;
            {
                let mut insert =
                    transaction.prepare("INSERT OR IGNORE INTO imported(word) VALUES (?1)")?;
                for entry in entries {
                    insert.execute(params![entry.word])?;
                }
            }
            removed = transaction.execute(
                "DELETE FROM staging WHERE word NOT IN (SELECT word FROM imported)",
                [],
            )?;
            transaction.execute_batch("DROP TABLE imported;")?;
        }
        let total: i64 =
            transaction.query_row("SELECT COUNT(*) FROM staging", [], |row| row.get(0))?;
        transaction.commit()?;
        Ok(ImportReport {
            written: entries.len(),
            removed,
            total: usize::try_from(total).unwrap_or(0),
        })
    }

    /// Entries staged per transaction by [`Repo::import_stream`].
    pub const STREAM_BATCH: usize = 1024;

    /// Write one batch of entries to the working set in its own transaction.
    ///
    /// When `track_prune` is set, the batch is also recorded in the temporary
    /// `imported` table used by [`Repo::import_stream`].
    fn stage_entries(&mut self, entries: &[Entry], track_prune: bool) -> Result<()> {
        let transaction = self.connection.transaction()?;
        {
            let mut insert = transaction.prepare(
                "INSERT INTO staging(word, display, payload, content_hash) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(word) DO UPDATE SET display = excluded.display,
                                                 payload = excluded.payload,
                                                 content_hash = excluded.content_hash",
            )?;
            for entry in entries {
                insert.execute(params![
                    entry.word,
                    entry.display,
                    entry.encode_payload(),
                    entry.content_hash()[..store::DIGEST_PREFIX].to_vec()
                ])?;
            }
        }
        if track_prune {
            let mut insert =
                transaction.prepare("INSERT OR IGNORE INTO imported(word) VALUES (?1)")?;
            for entry in entries {
                insert.execute(params![entry.word])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// Import entries from a streaming source.
    ///
    /// Entries are staged in batches of [`Repo::STREAM_BATCH`], so memory use
    /// stays flat no matter how large the source is. `next` returns
    /// `Ok(None)` at the end of input; `progress` is called after each batch
    /// with the running number of entries written.
    pub fn import_stream<F, P>(
        &mut self,
        mut next: F,
        prune: bool,
        mut progress: P,
    ) -> Result<ImportReport>
    where
        F: FnMut() -> Result<Option<Entry>>,
        P: FnMut(usize),
    {
        self.ensure_editable()?;
        self.ensure_workset()?;
        if prune {
            self.connection.execute_batch(
                "CREATE TEMP TABLE IF NOT EXISTS imported(word TEXT PRIMARY KEY);
                 DELETE FROM imported;",
            )?;
        }
        let mut written = 0usize;
        let mut batch: Vec<Entry> = Vec::with_capacity(Self::STREAM_BATCH);
        loop {
            while batch.len() < Self::STREAM_BATCH {
                match next()? {
                    Some(entry) => batch.push(entry),
                    None => break,
                }
            }
            if batch.is_empty() {
                break;
            }
            self.stage_entries(&batch, prune)?;
            written += batch.len();
            batch.clear();
            progress(written);
        }
        let removed = if prune {
            self.connection.execute(
                "DELETE FROM staging WHERE word NOT IN (SELECT word FROM imported)",
                [],
            )?
        } else {
            0
        };
        let total: i64 = self
            .connection
            .query_row("SELECT COUNT(*) FROM staging", [], |row| row.get(0))?;
        Ok(ImportReport {
            written,
            removed,
            total: usize::try_from(total).unwrap_or(0),
        })
    }

    /// Number of entries in the committed index.
    pub fn entry_count(&self) -> Result<usize> {
        let count: i64 = self
            .connection
            .query_row("SELECT COUNT(*) FROM entries", [], |row| row.get(0))?;
        Ok(usize::try_from(count).unwrap_or(0))
    }

    /// Number of blocks stored in the repository.
    pub fn block_count(&self) -> Result<usize> {
        let count: i64 = self
            .connection
            .query_row("SELECT COUNT(*) FROM blocks", [], |row| row.get(0))?;
        Ok(usize::try_from(count).unwrap_or(0))
    }

    /// Number of MDD resources stored in the repository.
    pub fn resource_count(&self) -> Result<usize> {
        let count: i64 =
            self.connection
                .query_row("SELECT COUNT(*) FROM resources", [], |row| row.get(0))?;
        Ok(usize::try_from(count).unwrap_or(0))
    }

    /// Combined payload size of every stored resource, in bytes.
    pub fn resource_bytes(&self) -> Result<u64> {
        let total: i64 = self.connection.query_row(
            "SELECT COALESCE(SUM(size), 0) FROM resources",
            [],
            |row| row.get(0),
        )?;
        Ok(u64::try_from(total).unwrap_or(0))
    }

    /// Payload stored under the normalised resource `key`.
    ///
    /// The lookup falls back to a case-insensitive comparison because MDD
    /// packs are written on machines where `pic/Apple.png` and
    /// `pic/apple.png` are the same file.
    pub fn resource(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let key = crate::mdd_import::normalize_resource_key(key);
        self.connection
            .query_row(
                "SELECT b.codec, b.raw_size, b.data
                 FROM resources r JOIN blocks b ON b.id = r.block_id
                 WHERE r.key = ?1 COLLATE NOCASE
                 LIMIT 1",
                params![key],
                |row| {
                    let codec_id: i64 = row.get(0)?;
                    let raw_size: i64 = row.get(1)?;
                    let data: Vec<u8> = row.get(2)?;
                    Ok((codec_id, raw_size, data))
                },
            )
            .optional()?
            .map(|(codec_id, raw_size, data)| {
                decode_block_data(codec_id, usize::try_from(raw_size).unwrap_or(0), &data)
            })
            .transpose()
    }

    /// Store one resource under `key`, replacing a previous payload.
    pub fn put_resource(&mut self, key: &str, bytes: &[u8]) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        put_resource_row(&transaction, key, bytes)?;
        transaction.commit()?;
        Ok(())
    }

    /// Store a batch of `(key, payload)` pairs in one transaction.
    ///
    /// Returns the number of rows written. Payloads are content addressed, so
    /// a repeated resource costs one index row and no block.
    pub fn put_resources(&mut self, resources: &[(String, Vec<u8>)]) -> Result<usize> {
        let transaction = self.connection.unchecked_transaction()?;
        for (key, bytes) in resources {
            put_resource_row(&transaction, key, bytes)?;
        }
        transaction.commit()?;
        Ok(resources.len())
    }

    /// Visit every committed entry in key order.
    ///
    /// Blocks are decoded lazily. The index is stored in key order and blocks
    /// were cut from the same order, so consecutive entries nearly always
    /// share a block; a one-block cache keeps memory flat.
    pub fn for_each_entry<F>(&self, mut visit: F) -> Result<usize>
    where
        F: FnMut(Entry) -> Result<()>,
    {
        let mut statement = self.connection.prepare(
            "SELECT word, block_hash, offset, length FROM entry_index ORDER BY word ASC",
        )?;
        let mut rows = statement.query([])?;
        let mut cached: Option<(Vec<u8>, Vec<u8>)> = None;
        let mut count = 0usize;
        while let Some(row) = rows.next()? {
            let word: String = row.get(0)?;
            let block_hash: Vec<u8> = row.get(1)?;
            let offset = usize::try_from(row.get::<_, i64>(2)?)
                .map_err(|_| Error::Inconsistent("negative offset".to_string()))?;
            let length = usize::try_from(row.get::<_, i64>(3)?)
                .map_err(|_| Error::Inconsistent("negative length".to_string()))?;
            let stale = cached
                .as_ref()
                .is_none_or(|(hash, _)| hash.as_slice() != block_hash.as_slice());
            if stale {
                let raw = self.load_block_raw(&block_hash)?;
                cached = Some((block_hash.clone(), raw));
            }
            let Some((_, raw)) = cached.as_ref() else {
                return Err(Error::Inconsistent("block cache is empty".to_string()));
            };
            let slice = raw
                .get(offset..offset + length)
                .ok_or_else(|| Error::Inconsistent("entry slice outside block".to_string()))?;
            let record = block::decode_record(slice)?;
            let mut entry = Entry::decode_payload(&record.payload)?;
            entry.word = word;
            visit(entry)?;
            count += 1;
        }
        Ok(count)
    }

    /// Entries whose normalised key starts with `prefix`, in key order.
    pub fn search_prefix(&self, prefix: &str, limit: usize) -> Result<Vec<(String, String)>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let key = normalize_key(prefix);
        let upper = format!("{key}\u{10FFFF}");
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut statement = self.connection.prepare(
            "SELECT word, display FROM entries
             WHERE word >= ?1 AND word < ?2
             ORDER BY word ASC
             LIMIT ?3",
        )?;
        let rows = statement.query_map(params![key, upper, limit], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (word, display) = row?;
            if !word.starts_with(&key) {
                continue;
            }
            out.push((word, display));
        }
        Ok(out)
    }

    /// Entries whose normalised key or displayed spelling contains `needle`,
    /// shortest key first, then in key order.
    ///
    /// `word` holds normalised keys, so the needle is normalised the same way;
    /// `display` keeps the source spelling and SQLite's `LIKE` folds ASCII
    /// case. Both sides escape `%`, `_` and the escape character itself, so a
    /// query of `100%` only matches entries containing those four characters.
    pub fn search_substring(&self, needle: &str, limit: usize) -> Result<Vec<(String, String)>> {
        if needle.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let escape = |raw: &str| {
            let mut out = String::with_capacity(raw.len());
            for character in raw.chars() {
                if matches!(character, '\\' | '%' | '_') {
                    out.push('\\');
                }
                out.push(character);
            }
            out
        };
        let key_pattern = format!("%{}%", escape(&normalize_key(needle)));
        let display_pattern = format!("%{}%", escape(needle));
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut statement = self.connection.prepare(
            "SELECT word, display FROM entries
             WHERE word LIKE ?1 ESCAPE '\\'
                OR display LIKE ?2 ESCAPE '\\'
             ORDER BY length(word) ASC, word ASC
             LIMIT ?3",
        )?;
        let rows = statement.query_map(params![key_pattern, display_pattern, limit], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Compare the working set with HEAD.
    pub fn status(&self) -> Result<Status> {
        // A packed file has no staging table state at all: its working set is
        // HEAD by definition.
        if self.read_only || workset_is_clean(&self.connection)? {
            let entries = self.entry_count()?;
            return Ok(Status {
                total: entries,
                entries,
                blocks: self.block_count()?,
                head: self.head()?.map(|hash| hash::to_hex(&hash)),
                ..Status::default()
            });
        }
        let added: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM staging s
             LEFT JOIN entries e ON e.word = s.word
             WHERE e.word IS NULL",
            [],
            |row| row.get(0),
        )?;
        let modified: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM staging s
             JOIN entries e ON e.word = s.word
             WHERE s.content_hash <> e.content_hash",
            [],
            |row| row.get(0),
        )?;
        let removed: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM entries e
             LEFT JOIN staging s ON s.word = e.word
             WHERE s.word IS NULL",
            [],
            |row| row.get(0),
        )?;
        let total: i64 = self
            .connection
            .query_row("SELECT COUNT(*) FROM staging", [], |row| row.get(0))?;
        Ok(Status {
            added: usize::try_from(added).unwrap_or(0),
            modified: usize::try_from(modified).unwrap_or(0),
            removed: usize::try_from(removed).unwrap_or(0),
            total: usize::try_from(total).unwrap_or(0),
            entries: self.entry_count()?,
            blocks: self.block_count()?,
            head: self.head()?.map(|hash| hash::to_hex(&hash)),
        })
    }

    /// Working-set status against HEAD, for editing sessions.
    ///
    /// This is [`Repo::status`]: the editor and the CLI must never report
    /// different counts for the same repository, so there is one
    /// implementation, not two.
    pub fn working_status(&self) -> Result<Status> {
        self.status()
    }

    /// Record the working set as a new commit.
    ///
    /// Nothing is bulk-loaded: the staging table is read once in word order,
    /// blocks are cut and written as soon as they are complete, the entry
    /// root is folded incrementally and the commit index is compressed
    /// straight into the database file. Peak memory is one block plus that
    /// block's entry metadata, never the whole dictionary.
    pub fn commit(&mut self, message: &str, author: &str, ts: i64) -> Result<CommitOutcome> {
        self.ensure_editable()?;
        let status = self.status()?;
        if status.is_clean() {
            return Err(Error::NoChanges);
        }
        let parent = self.head()?.unwrap_or(ZERO_HASH);
        let temp = TempPath::new(&self.dir, "commit-index");

        let transaction = self.connection.transaction()?;
        // Committed entries keep only a pointer in `staging`. Snapshot the
        // slices before the index is rebuilt so their payloads can be
        // materialised while the new blocks are chunked.
        transaction.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS staging_location(
                 word TEXT PRIMARY KEY,
                 block_hash BLOB NOT NULL,
                 offset INTEGER NOT NULL,
                 length INTEGER NOT NULL
             );
             DELETE FROM staging_location;
             INSERT INTO staging_location(word, block_hash, offset, length)
                 SELECT s.word, e.block_hash, e.offset, e.length
                 FROM staging s JOIN entry_index e ON e.word = s.word
                 WHERE s.payload IS NULL;",
        )?;
        transaction.execute("DELETE FROM entries", [])?;

        let mut tally = BlockTally::default();
        let mut entry_count = 0u64;
        let mut entries_root = hash::MerkleAccumulator::new();
        let mut records: Vec<Record> = Vec::new();
        let mut staged: Vec<Staged> = Vec::new();
        let mut chunker = cdc::StreamChunker::new();

        // The index only keeps digest prefixes, but the commit index snapshot
        // and the Merkle tree are defined on the full 32-byte digest. Collect
        // them per entry while the payload is in hand; a TEMP table keeps the
        // peak memory flat on dictionaries with hundreds of thousands of rows.
        transaction.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS commit_digests(word TEXT PRIMARY KEY, digest BLOB NOT NULL);
             DELETE FROM commit_digests;
             CREATE TEMP TABLE IF NOT EXISTS commit_delta(word TEXT PRIMARY KEY,
                                                         kind INTEGER NOT NULL,
                                                         digest BLOB);
             DELETE FROM commit_delta;",
        )?;

        {
            let mut insert_digest =
                transaction.prepare("INSERT INTO commit_digests(word, digest) VALUES (?1, ?2)")?;
            let mut select = transaction.prepare(
                "SELECT word, display, payload, content_hash FROM staging ORDER BY word ASC",
            )?;
            let mut insert_entry = transaction.prepare(
                "INSERT INTO entries(word, display, block_id, offset, length,
                                     content_hash, field_hashes)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            let mut block_exists =
                transaction.prepare("SELECT id FROM blocks WHERE block_hash = ?1")?;
            let mut insert_block = transaction.prepare(
                "INSERT INTO blocks(block_hash, codec, raw_size, data) VALUES (?1, ?2, ?3, ?4)",
            )?;
            let mut select_location = transaction.prepare(
                "SELECT block_hash, offset, length FROM staging_location WHERE word = ?1",
            )?;
            let mut reader = BlockReader::new(&transaction);

            let mut rows = select.query([])?;
            while let Some(row) = rows.next()? {
                let word: String = row.get(0)?;
                let display: String = row.get(1)?;
                let stored_digest: Vec<u8> = row.get(3)?;
                let payload: Vec<u8> = match row.get::<_, Option<Vec<u8>>>(2)? {
                    Some(payload) => payload,
                    None => {
                        let location = select_location
                            .query_row(params![word], |row| {
                                Ok((
                                    row.get::<_, Vec<u8>>(0)?,
                                    row.get::<_, i64>(1)?,
                                    row.get::<_, i64>(2)?,
                                ))
                            })
                            .optional()?
                            .ok_or_else(|| {
                                Error::Inconsistent(format!(
                                    "working entry {word} has no payload and no committed slice"
                                ))
                            })?;
                        let block_hash = hash_from_blob(&location.0)?;
                        hydrate_payload(
                            &mut reader,
                            &(block_hash, location.1, location.2),
                            &word,
                            &stored_digest,
                        )?
                    }
                };
                // The index keeps a digest prefix; the tree and the commit
                // index snapshot are defined on the full payload digest, which
                // is recomputed here from the bytes that are about to be cut
                // into a block.
                let content_hash = hash::blake3(&payload);
                if !digest_matches(&stored_digest, &content_hash) {
                    return Err(Error::Inconsistent(format!(
                        "working entry {word} does not match its content hash"
                    )));
                }
                let field_hashes = cbor::encode(&field_hashes_of(&payload)?);
                insert_digest.execute(params![&word, content_hash.as_slice()])?;
                entries_root.push(hash::leaf_hash(word.as_bytes(), &content_hash));
                entry_count += 1;

                let record = Record::new(word.as_bytes().to_vec(), payload);
                let mut cut = chunker.push(&record);
                if cut == cdc::Cut::Before {
                    write_block(
                        &transaction,
                        &mut insert_entry,
                        &mut block_exists,
                        &mut insert_block,
                        &records,
                        &staged,
                        &mut tally,
                    )?;
                    records.clear();
                    staged.clear();
                    cut = chunker.push(&record);
                }
                records.push(record);
                staged.push(Staged {
                    word,
                    display,
                    content_hash,
                    field_hashes,
                });
                if cut == cdc::Cut::After {
                    write_block(
                        &transaction,
                        &mut insert_entry,
                        &mut block_exists,
                        &mut insert_block,
                        &records,
                        &staged,
                        &mut tally,
                    )?;
                    records.clear();
                    staged.clear();
                }
            }
            if !records.is_empty() {
                write_block(
                    &transaction,
                    &mut insert_entry,
                    &mut block_exists,
                    &mut insert_block,
                    &records,
                    &staged,
                    &mut tally,
                )?;
                records.clear();
                staged.clear();
            }
        }

        // The working set now equals HEAD exactly. Dropping it outright is
        // cheaper than keeping a word/display/content_hash mirror of the index
        // (10+ MiB on a mid-size dictionary) and it is rebuilt from `entries`
        // on the next write; `status` reads the flag instead of the table.
        let cached_payloads: i64 = transaction.query_row(
            "SELECT COALESCE(SUM(length(payload)), 0) FROM staging",
            [],
            |row| row.get(0),
        )?;
        transaction.execute("DELETE FROM staging", [])?;
        store::set_meta(&transaction, store::META_WORKSET_CLEAN, b"1")?;

        let block_root = {
            let mut accumulator = hash::MerkleAccumulator::new();
            let mut select = transaction
                .prepare("SELECT DISTINCT block_hash FROM entry_index ORDER BY block_hash ASC")?;
            let mut rows = select.query([])?;
            while let Some(row) = rows.next()? {
                let digest = hash_from_blob(&row.get::<_, Vec<u8>>(0)?)?;
                accumulator.push(digest);
            }
            accumulator.finish()
        };
        let entry_root = entries_root.finish();
        let commit_hash =
            hash::commit_hash(&parent, &ZERO_HASH, &entry_root, &block_root, entry_count);

        // The entry is a full snapshot at the genesis commit, every
        // ANCHOR_INTERVAL commits, and whenever the parent has no index to
        // diff against; otherwise it is the difference to the parent.
        let parent_header = if parent == ZERO_HASH {
            None
        } else {
            index::index_header(&transaction, &parent)?
        };
        let parent_depth = parent_header.map_or(0, |header| header.depth);
        let anchor = parent == ZERO_HASH
            || parent_header.is_none()
            || parent_depth + 1 >= snapshot::ANCHOR_INTERVAL;
        let (kind, depth, operation_count) = if anchor {
            (snapshot::EntryKind::Full, 0, entry_count)
        } else {
            let depth = parent_depth + 1;
            let operations = build_delta(&transaction, &parent)?;
            // A delta larger than half the index costs more to read than the
            // snapshot it replaces (it still has to decode the parent), so a
            // bulk rewrite resets the chain to a full anchor instead.
            if operations.saturating_mul(2) > entry_count {
                (snapshot::EntryKind::Full, 0, entry_count)
            } else {
                (snapshot::EntryKind::Delta, depth, operations)
            }
        };

        let (raw_size, compressed_size) = {
            let file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(temp.path())?;
            let encoder =
                zstd::stream::write::Encoder::new(std::io::BufWriter::new(file), ZSTD_LEVEL)
                    .map_err(|error| Error::Io(error.to_string()))?;
            match kind {
                snapshot::EntryKind::Full => {
                    let mut writer = snapshot::SnapshotWriter::full(encoder, entry_count)?;
                    let mut select = transaction
                        .prepare("SELECT word, digest FROM commit_digests ORDER BY word ASC")?;
                    let mut rows = select.query([])?;
                    while let Some(row) = rows.next()? {
                        let (word, digest) = digest_row(row)?;
                        writer.push(&word, &digest)?;
                    }
                    finish_entry(writer, temp.path())?
                }
                snapshot::EntryKind::Delta => {
                    let mut writer = snapshot::SnapshotWriter::delta(
                        encoder,
                        depth,
                        entry_count,
                        operation_count,
                    )?;
                    let mut select = transaction
                        .prepare("SELECT word, kind, digest FROM commit_delta ORDER BY word ASC")?;
                    let mut rows = select.query([])?;
                    while let Some(row) = rows.next()? {
                        let word: String = row.get(0)?;
                        let code: i64 = row.get(1)?;
                        let op = match code {
                            0 => snapshot::Op::Add {
                                word,
                                digest: hash_from_blob(&row.get::<_, Vec<u8>>(2)?)?,
                            },
                            1 => snapshot::Op::Change {
                                word,
                                digest: hash_from_blob(&row.get::<_, Vec<u8>>(2)?)?,
                            },
                            _ => snapshot::Op::Remove { word },
                        };
                        writer.push_op(&op)?;
                    }
                    finish_entry(writer, temp.path())?
                }
            }
        };

        let commit = CommitRecord {
            hash: commit_hash,
            parent,
            parent2: ZERO_HASH,
            entry_root,
            block_root,
            entry_count,
            ts,
            author: author.to_string(),
            message: message.to_string(),
        };

        transaction.execute(
            "INSERT INTO commits(commit_hash, parent, parent2, entry_root, block_root,
             entry_count, ts, author, message) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                commit.hash.as_slice(),
                commit.parent.as_slice(),
                commit.parent2.as_slice(),
                commit.entry_root.as_slice(),
                commit.block_root.as_slice(),
                commit.entry_count as i64,
                commit.ts,
                commit.author,
                commit.message,
            ],
        )?;
        transaction.execute(
            "INSERT INTO commit_index(commit_hash, codec, raw_size, data)
             VALUES (?1, ?2, ?3, zeroblob(?4))",
            params![
                commit.hash.as_slice(),
                codec::ZSTD,
                raw_size as i64,
                compressed_size as i64,
            ],
        )?;
        {
            // Copy the compressed snapshot into the database in fixed-size
            // chunks so the whole index never has to sit in memory.
            let rowid = transaction.last_insert_rowid();
            let mut blob = transaction.blob_open("main", "commit_index", "data", rowid, false)?;
            let mut source = std::fs::File::open(temp.path())?;
            let mut buffer = vec![0u8; 64 * 1024];
            loop {
                let read = source.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                blob.write_all(&buffer[..read])?;
            }
        }
        transaction.execute(
            "INSERT INTO refs(name, commit_hash) VALUES ('HEAD', ?1)
             ON CONFLICT(name) DO UPDATE SET commit_hash = excluded.commit_hash",
            params![commit.hash.as_slice()],
        )?;
        transaction.commit()?;

        // Dropping the payloads empties pages inside staging but leaves them in
        // the file. Large working sets are worth compacting once the commit is
        // durable; small edits should not pay for a full rewrite.
        if cached_payloads >= VACUUM_THRESHOLD {
            self.connection.execute_batch("VACUUM")?;
        }

        Ok(CommitOutcome {
            commit,
            new_blocks: tally.new_blocks,
            reused_blocks: tally.reused_blocks,
            entries: usize::try_from(entry_count).unwrap_or(usize::MAX),
        })
    }

    /// Load one commit row.
    pub fn commit_record(&self, hash: &[u8; 32]) -> Result<Option<CommitRecord>> {
        let row = self
            .connection
            .query_row(
                "SELECT parent, parent2, entry_root, block_root, entry_count, ts, author, message
                 FROM commits WHERE commit_hash = ?1",
                params![hash.as_slice()],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                    ))
                },
            )
            .optional()?;
        row.map(|row| {
            Ok(CommitRecord {
                hash: *hash,
                parent: hash_from_blob(&row.0)?,
                parent2: hash_from_blob(&row.1)?,
                entry_root: hash_from_blob(&row.2)?,
                block_root: hash_from_blob(&row.3)?,
                entry_count: u64::try_from(row.4).unwrap_or(0),
                ts: row.5,
                author: row.6,
                message: row.7,
            })
        })
        .transpose()
    }

    /// Resolve `HEAD`, `HEAD~N` or a commit hash prefix.
    pub fn resolve(&self, revision: &str) -> Result<[u8; 32]> {
        if revision == "HEAD" {
            return self
                .head()?
                .ok_or_else(|| Error::UnknownCommit("HEAD".to_string()));
        }
        if let Some(rest) = revision.strip_prefix("HEAD~") {
            let steps: usize = rest
                .parse()
                .map_err(|_| Error::InvalidArgument(format!("bad revision {revision}")))?;
            let mut current = self
                .head()?
                .ok_or_else(|| Error::UnknownCommit("HEAD".to_string()))?;
            for _ in 0..steps {
                let record = self
                    .commit_record(&current)?
                    .ok_or_else(|| Error::UnknownCommit(hash::to_hex(&current)))?;
                if record.parent == ZERO_HASH {
                    return Err(Error::UnknownCommit(format!(
                        "revision {revision} goes past the root commit"
                    )));
                }
                current = record.parent;
            }
            if self.commit_record(&current)?.is_none() {
                // The commit exists only as a parent link: `gc` removed it.
                return Err(Error::UnknownCommit(hash::to_hex(&current)));
            }
            return Ok(current);
        }
        let needle = revision.to_lowercase();
        if needle.len() < 4
            || !needle
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        {
            return Err(Error::InvalidArgument(format!("bad revision {revision}")));
        }
        let mut statement = self
            .connection
            .prepare("SELECT commit_hash, lower(hex(commit_hash)) FROM commits")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut matches = Vec::new();
        for row in rows {
            let (bytes, hex) = row?;
            if hex.starts_with(&needle) {
                matches.push(hash_from_blob(&bytes)?);
            }
        }
        match matches.len() {
            0 => Err(Error::UnknownCommit(revision.to_string())),
            1 => Ok(matches[0]),
            _ => Err(Error::InvalidArgument(format!(
                "ambiguous revision {revision} ({} matches)",
                matches.len()
            ))),
        }
    }

    /// Walk history from HEAD (newest first).
    pub fn log(&self, limit: Option<usize>) -> Result<Vec<CommitRecord>> {
        let mut out = Vec::new();
        let mut current = self.head()?;
        while let Some(hash) = current {
            if limit.is_some_and(|limit| out.len() >= limit) {
                break;
            }
            let Some(record) = self.commit_record(&hash)? else {
                // History was truncated by `gc`: the parent row is gone.
                break;
            };
            current = if record.parent == ZERO_HASH {
                None
            } else {
                Some(record.parent)
            };
            out.push(record);
        }
        Ok(out)
    }

    pub(crate) fn load_block_raw(&self, block_hash: &[u8]) -> Result<Vec<u8>> {
        let row = self
            .connection
            .query_row(
                "SELECT codec, raw_size, data FROM blocks WHERE block_hash = ?1",
                params![block_hash],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((codec_id, raw_size, data)) = row else {
            let name = hash_from_blob(block_hash)
                .map(|hash| hash::to_hex(&hash))
                .unwrap_or_else(|_| format!("{block_hash:02x?}"));
            return Err(Error::UnknownBlock(name));
        };
        let raw_size = usize::try_from(raw_size)
            .map_err(|_| Error::Inconsistent("negative raw_size".to_string()))?;
        decode_block_data(codec_id, raw_size, &data)
    }

    /// Read one entry by its normalised key.
    pub fn show(&self, word: &str) -> Result<Option<Entry>> {
        let key = normalize_key(word);
        let row = self
            .connection
            .query_row(
                "SELECT block_hash, offset, length FROM entry_index WHERE word = ?1",
                params![key],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((block_hash, offset, length)) = row else {
            return Ok(None);
        };
        let raw = self.load_block_raw(&block_hash)?;
        let offset = usize::try_from(offset)
            .map_err(|_| Error::Inconsistent("negative offset".to_string()))?;
        let length = usize::try_from(length)
            .map_err(|_| Error::Inconsistent("negative length".to_string()))?;
        let slice = raw
            .get(offset..offset + length)
            .ok_or_else(|| Error::Inconsistent("entry slice outside block".to_string()))?;
        let record = block::decode_record(slice)?;
        let mut entry = Entry::decode_payload(&record.payload)?;
        entry.word = key;
        Ok(Some(entry))
    }

    /// Follows `@@@LINK=` redirects starting at `word`.
    ///
    /// MDict cross-reference entries carry a body of exactly
    /// `@@@LINK=<target>` instead of markup. Callers that render entries
    /// must resolve them, otherwise the line shows up as body text.
    pub fn resolve_link(&self, word: &str) -> Result<crate::link::LinkResolution> {
        crate::link::resolve(word, |key| self.show(key))
    }

    /// Read the working-set payload for `word`.
    ///
    /// Committed entries keep only a pointer in `staging`; their payload is
    /// materialised from the committed block and verified against the stored
    /// content hash. Uncommitted entries carry their payload inline.
    pub fn working_payload(&self, word: &str) -> Result<Option<Vec<u8>>> {
        let key = normalize_key(word);
        let row = self
            .connection
            .query_row(
                "SELECT payload, content_hash FROM staging WHERE word = ?1",
                params![key],
                |row| Ok((row.get::<_, Option<Vec<u8>>>(0)?, row.get::<_, Vec<u8>>(1)?)),
            )
            .optional()?;
        let Some((payload, content_hash)) = row else {
            // A packed file has no staging rows and no clean marker, but its
            // working set is HEAD, so look through to the committed index.
            if !self.read_only && !workset_is_clean(&self.connection)? {
                return Ok(None);
            }
            // The working set was dropped after the last commit: a lookup for a
            // committed word is served straight from its block.
            let digest = self
                .connection
                .query_row(
                    "SELECT content_hash FROM entries WHERE word = ?1",
                    params![key],
                    |row| row.get::<_, Vec<u8>>(0),
                )
                .optional()?;
            let Some(digest) = digest else {
                return Ok(None);
            };
            return Ok(Some(self.payload_from_index(&key, &digest)?));
        };
        match payload {
            Some(payload) => Ok(Some(payload)),
            None => Ok(Some(self.payload_from_index(&key, &content_hash)?)),
        }
    }

    /// Read the working-set entry for `word`, materialising its payload when
    /// the working set only holds a pointer into the committed index.
    pub fn working_entry(&self, word: &str) -> Result<Option<Entry>> {
        let key = normalize_key(word);
        let Some(payload) = self.working_payload(&key)? else {
            return Ok(None);
        };
        let mut entry = Entry::decode_payload(&payload)?;
        entry.word = key;
        Ok(Some(entry))
    }

    /// One page of working-set entries in ascending key order.
    ///
    /// Keys are normalised and sorted with SQLite's binary collation, the
    /// same order [`Repo::commit`] writes blocks in. `after_key` is the last
    /// key of the previous page (exclusive), so the caller walks the whole
    /// dictionary by passing the previous page's last `Entry::word`.
    pub fn entries_page(&self, after_key: Option<&str>, limit: usize) -> Result<Vec<Entry>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let after = after_key.map(normalize_key);
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        // The table name is chosen here, never from the caller; the page is
        // read from the working set when it holds edits, from HEAD otherwise.
        let source = if self.read_only || workset_is_clean(&self.connection)? {
            "entries"
        } else {
            "staging"
        };
        let mut statement = self.connection.prepare(&format!(
            "SELECT word FROM {source}
             WHERE (?1 IS NULL OR word > ?1)
             ORDER BY word ASC
             LIMIT ?2"
        ))?;
        let mut rows = statement.query(params![after, limit])?;
        let mut keys: Vec<String> = Vec::new();
        while let Some(row) = rows.next()? {
            keys.push(row.get(0)?);
        }
        drop(rows);
        drop(statement);
        let mut entries = Vec::with_capacity(keys.len());
        for key in keys {
            if let Some(entry) = self.working_entry(&key)? {
                entries.push(entry);
            }
        }
        Ok(entries)
    }

    /// Insert or replace one entry in the working set.
    ///
    /// The key is normalised with [`normalize_key`]. HEAD is not touched: the
    /// change becomes part of the next [`Repo::commit`] and is visible to
    /// [`Repo::status`], [`Repo::working_changes`] and
    /// [`Repo::entry_diff`] immediately.
    pub fn upsert_entry(&mut self, entry: &Entry) -> Result<EntryWriteOutcome> {
        self.ensure_editable()?;
        self.ensure_workset()?;
        let key = normalize_key(&entry.word);
        let existed = self
            .connection
            .query_row(
                "SELECT 1 FROM entries WHERE word = ?1",
                params![key],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO staging(word, display, payload, content_hash) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(word) DO UPDATE SET display = excluded.display,
                                             payload = excluded.payload,
                                             content_hash = excluded.content_hash",
            params![
                key,
                entry.display,
                entry.encode_payload(),
                entry.content_hash()[..store::DIGEST_PREFIX].to_vec()
            ],
        )?;
        mark_workset_dirty(&transaction)?;
        transaction.commit()?;
        Ok(if existed {
            EntryWriteOutcome::Modified
        } else {
            EntryWriteOutcome::Added
        })
    }

    /// Remove one entry from the working set.
    ///
    /// Returns [`EntryWriteOutcome::NotFound`] when the key is not staged; a
    /// key that only exists in HEAD is staged first, so deleting it counts as
    /// a deletion against HEAD.
    pub fn delete_entry(&mut self, key: &str) -> Result<EntryWriteOutcome> {
        self.ensure_editable()?;
        self.ensure_workset()?;
        let key = normalize_key(key);
        let transaction = self.connection.transaction()?;
        let removed = transaction.execute("DELETE FROM staging WHERE word = ?1", params![key])?;
        mark_workset_dirty(&transaction)?;
        transaction.commit()?;
        Ok(if removed == 0 {
            EntryWriteOutcome::NotFound
        } else {
            EntryWriteOutcome::Deleted
        })
    }

    /// Changed keys between HEAD and the working set, ascending.
    ///
    /// `after_key` is exclusive, like [`Repo::entries_page`]. A clean working
    /// set returns an empty page without touching the staging table.
    pub fn working_changes(&self, after_key: Option<&str>, limit: usize) -> Result<Vec<ChangeRow>> {
        if limit == 0 || self.read_only || workset_is_clean(&self.connection)? {
            return Ok(Vec::new());
        }
        let after = after_key.map(normalize_key);
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut statement = self.connection.prepare(
            "SELECT key, kind FROM (
                 SELECT s.word AS key,
                        CASE WHEN e.word IS NULL THEN 'added' ELSE 'modified' END AS kind
                   FROM staging s LEFT JOIN entries e ON e.word = s.word
                  WHERE e.word IS NULL OR e.content_hash <> s.content_hash
                 UNION ALL
                 SELECT e.word AS key, 'deleted' AS kind
                   FROM entries e LEFT JOIN staging s ON s.word = e.word
                  WHERE s.word IS NULL
             )
             WHERE (?1 IS NULL OR key > ?1)
             ORDER BY key ASC
             LIMIT ?2",
        )?;
        let mut rows = statement.query(params![after, limit])?;
        let mut changes = Vec::new();
        while let Some(row) = rows.next()? {
            let key: String = row.get(0)?;
            let kind: String = row.get(1)?;
            let kind = match kind.as_str() {
                "added" => ChangeKind::Added,
                "modified" => ChangeKind::Modified,
                "deleted" => ChangeKind::Deleted,
                other => {
                    return Err(Error::Inconsistent(format!(
                        "unexpected change kind {other}"
                    )))
                }
            };
            changes.push(ChangeRow { key, kind });
        }
        Ok(changes)
    }

    /// Field-level diff of one key between HEAD and the working set.
    ///
    /// A key that only exists in the working set is reported as
    /// [`DiffKind::Added`] with `before: None` on every field; HEAD-only keys
    /// are [`DiffKind::Deleted`] with `after: None`.
    pub fn entry_diff(&self, key: &str) -> Result<EntryDiff> {
        let key = normalize_key(key);
        let before = self.show(&key)?;
        let after = self.working_entry(&key)?;
        let kind = match (&before, &after) {
            (None, None) => DiffKind::Absent,
            (None, Some(_)) => DiffKind::Added,
            (Some(_), None) => DiffKind::Deleted,
            (Some(_), Some(_)) => DiffKind::Modified,
        };
        Ok(EntryDiff {
            key,
            kind,
            fields: diff_fields(before.as_ref(), after.as_ref()),
        })
    }

    /// Throw away every working-set change and restore HEAD.
    ///
    /// History is untouched; only the staging table and its clean marker
    /// change. Calling this on a clean working set is a no-op.
    pub fn discard_working(&mut self) -> Result<()> {
        self.ensure_editable()?;
        if workset_is_clean(&self.connection)? {
            return Ok(());
        }
        let transaction = self.connection.transaction()?;
        transaction.execute("DELETE FROM staging", [])?;
        store::set_meta(&transaction, store::META_WORKSET_CLEAN, b"1")?;
        transaction.commit()?;
        Ok(())
    }

    fn payload_from_index(&self, word: &str, content_hash: &[u8]) -> Result<Vec<u8>> {
        let row = self
            .connection
            .query_row(
                "SELECT block_hash, offset, length FROM entry_index WHERE word = ?1",
                params![word],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((block_hash, offset, length)) = row else {
            return Err(Error::Inconsistent(format!(
                "working entry {word} has no payload and no committed slice"
            )));
        };
        let block_hash = hash_from_blob(&block_hash)?;
        let mut reader = BlockReader::new(&self.connection);
        hydrate_payload(
            &mut reader,
            &(block_hash, offset, length),
            word,
            content_hash,
        )
    }

    /// Frame of the index entry stored for a commit.
    ///
    /// `None` means the commit has no index row. Diagnostics use it to tell a
    /// full snapshot from a delta without decoding the body.
    pub fn commit_index_header(&self, hash: &[u8; 32]) -> Result<Option<snapshot::EntryHeader>> {
        index::index_header(&self.connection, hash)
    }

    /// Count and size of the stored commit indexes, split by encoding.
    pub fn index_storage(&self) -> Result<IndexStorage> {
        let mut entries = Vec::new();
        {
            let mut statement = self
                .connection
                .prepare("SELECT commit_hash, raw_size, LENGTH(data) FROM commit_index")?;
            let mut rows = statement.query([])?;
            while let Some(row) = rows.next()? {
                entries.push((
                    hash_from_blob(&row.get::<_, Vec<u8>>(0)?)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ));
            }
        }
        let mut storage = IndexStorage::default();
        for (hash, raw_size, compressed) in entries {
            storage.raw_bytes += u64::try_from(raw_size).unwrap_or(0);
            storage.compressed_bytes += u64::try_from(compressed).unwrap_or(0);
            match index::index_header(&self.connection, &hash)? {
                Some(header) if header.kind == snapshot::EntryKind::Delta => storage.delta += 1,
                _ => storage.full += 1,
            }
        }
        Ok(storage)
    }

    /// Open a streaming reader over the commit index stored for `hash`.
    ///
    /// The compressed blob is decoded on the fly and only one entry is held in
    /// memory at a time. A delta entry is replayed over its parent chain, which
    /// holds one pair from the parent and one operation per delta level.
    pub(crate) fn snapshot_reader(&self, hash: &[u8; 32]) -> Result<IndexChain<'_>> {
        index::open_index(&self.connection, hash, snapshot::MAX_CHAIN_DEPTH)
    }

    /// Compressed full snapshot of HEAD's index, for a shallow pack.
    ///
    /// A shallow pack keeps one commit, so an index that points at a parent
    /// would be unreadable in the destination. Returns the rebuilt bytes, or
    /// `None` when the stored entry is already a full snapshot.
    fn materialized_head_index(&self, hash: &[u8; 32]) -> Result<Option<(u64, Vec<u8>)>> {
        if index::index_header(&self.connection, hash)?
            .is_some_and(|header| header.kind == snapshot::EntryKind::Full)
        {
            return Ok(None);
        }
        let entry_count: i64 = self.connection.query_row(
            "SELECT entry_count FROM commits WHERE commit_hash = ?1",
            params![hash.as_slice()],
            |row| row.get(0),
        )?;
        let mut writer = snapshot::SnapshotWriter::full(
            Vec::new(),
            u64::try_from(entry_count).unwrap_or(u64::MAX),
        )?;
        {
            let mut reader = self.snapshot_reader(hash)?;
            while let Some((word, digest)) = reader.next_pair()? {
                writer.push(&word, &digest)?;
            }
        }
        let (raw, raw_size) = writer.finish()?;
        Ok(Some((raw_size, compress(&raw)?)))
    }

    /// Merkle root and entry count of the snapshot stored for `hash`, folded
    /// while streaming.
    fn snapshot_summary(&self, hash: &[u8; 32]) -> Result<([u8; 32], u64, u64, u64)> {
        let mut reader = self.snapshot_reader(hash)?;
        let mut accumulator = hash::MerkleAccumulator::new();
        let mut count = 0u64;
        while let Some((word, digest)) = reader.next_pair()? {
            accumulator.push(hash::leaf_hash(word.as_bytes(), &digest));
            count += 1;
        }
        Ok((
            accumulator.finish(),
            count,
            reader.raw_size(),
            reader.bytes_read(),
        ))
    }

    /// Materialise a commit index and check it against the commit row.
    ///
    /// Returns `(word, content_hash)` pairs in index order and fails with
    /// `ODC-E018` when the snapshot does not reproduce `entry_root` or
    /// `entry_count`.
    pub(crate) fn snapshot_pairs(
        &self,
        hash: &[u8; 32],
        commit: &CommitRecord,
    ) -> Result<Vec<(String, [u8; 32])>> {
        let mut reader = self.snapshot_reader(hash)?;
        let mut pairs = Vec::new();
        let mut accumulator = hash::MerkleAccumulator::new();
        while let Some((word, digest)) = reader.next_pair()? {
            accumulator.push(hash::leaf_hash(word.as_bytes(), &digest));
            pairs.push((word, digest));
        }
        if accumulator.finish() != commit.entry_root || pairs.len() as u64 != commit.entry_count {
            return Err(Error::Inconsistent(format!(
                "commit {} index does not match its roots",
                hash::to_hex(hash)
            )));
        }
        Ok(pairs)
    }

    /// Word-level diff between two revisions.
    ///
    /// Both commit indexes are decompressed and merged as streams in word
    /// order and neither side is ever materialised as a map.
    pub fn diff(&self, from: &str, to: &str) -> Result<Diff> {
        let from_hash = self.resolve(from)?;
        let to_hash = self.resolve(to)?;
        let mut added = Vec::new();
        let mut removed = Vec::new();
        let mut modified = Vec::new();
        if from_hash != to_hash {
            let mut left = self.snapshot_reader(&from_hash)?;
            let mut right = self.snapshot_reader(&to_hash)?;
            let mut left_item = left.next_pair()?;
            let mut right_item = right.next_pair()?;
            loop {
                match (left_item.take(), right_item.take()) {
                    (None, None) => break,
                    (None, Some((word, _))) => {
                        added.push(word);
                        right_item = right.next_pair()?;
                    }
                    (Some((word, _)), None) => {
                        removed.push(word);
                        left_item = left.next_pair()?;
                    }
                    (Some((left_word, left_digest)), Some((right_word, right_digest))) => {
                        match left_word.cmp(&right_word) {
                            std::cmp::Ordering::Less => {
                                removed.push(left_word);
                                left_item = left.next_pair()?;
                                right_item = Some((right_word, right_digest));
                            }
                            std::cmp::Ordering::Greater => {
                                added.push(right_word);
                                right_item = right.next_pair()?;
                                left_item = Some((left_word, left_digest));
                            }
                            std::cmp::Ordering::Equal => {
                                if left_digest != right_digest {
                                    modified.push(right_word);
                                }
                                left_item = left.next_pair()?;
                                right_item = right.next_pair()?;
                            }
                        }
                    }
                }
            }
        }
        Ok(Diff {
            from: hash::to_hex(&from_hash),
            to: hash::to_hex(&to_hash),
            added,
            removed,
            modified,
        })
    }

    /// Verify every stored object and index invariant that can be checked.
    pub fn verify(&self) -> Result<VerifyReport> {
        let mut errors = Vec::new();
        let mut blocks = 0usize;
        let mut entries = 0usize;
        let mut commits = 0usize;
        let mut entry_root = hash::MerkleAccumulator::new();

        {
            let mut statement = self
                .connection
                .prepare("SELECT block_hash, codec, raw_size, data FROM blocks")?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                ))
            })?;
            for row in rows {
                let (block_hash, codec_id, raw_size, data) = row?;
                blocks += 1;
                let raw_size = match usize::try_from(raw_size) {
                    Ok(size) => size,
                    Err(_) => {
                        errors.push("negative raw_size".to_string());
                        continue;
                    }
                };
                let raw = match decode_block_data(codec_id, raw_size, &data) {
                    Ok(raw) => raw,
                    Err(error) => {
                        errors.push(format!(
                            "block {}: {error}",
                            hash::to_hex(&hash_from_blob(&block_hash).unwrap_or(ZERO_HASH))
                        ));
                        continue;
                    }
                };
                let computed = hash::blake3(&raw);
                match hash_from_blob(&block_hash) {
                    Ok(stored) if stored == computed => {}
                    Ok(stored) => errors.push(format!(
                        "block {}: stored hash does not match content ({})",
                        hash::to_hex(&stored),
                        hash::to_hex(&computed)
                    )),
                    Err(error) => errors.push(error.to_string()),
                }
                if let Err(error) = block::decode(&raw) {
                    errors.push(format!("block decode: {error}"));
                }
            }
        }

        {
            // Word order keeps consecutive entries in one block, so the tree
            // can be folded here: version 2 only stores digest prefixes, and
            // the full digest is recomputed from the payload that is decoded
            // for verification anyway.
            let mut statement = self.connection.prepare(
                "SELECT word, display, block_hash, offset, length, content_hash, field_hashes
                 FROM entry_index ORDER BY word",
            )?;
            let mut loaded: Option<(Vec<u8>, Option<Vec<u8>>)> = None;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                    row.get::<_, Vec<u8>>(6)?,
                ))
            })?;
            for row in rows {
                let (word, display, block_hash, offset, length, content_hash, field_hashes) = row?;
                entries += 1;
                if loaded
                    .as_ref()
                    .map(|(hash, _)| hash != &block_hash)
                    .unwrap_or(true)
                {
                    let raw = self.load_block_raw(&block_hash).ok();
                    loaded = Some((block_hash.clone(), raw));
                }
                let Some(raw) = loaded.as_ref().and_then(|(_, raw)| raw.as_ref()) else {
                    errors.push(format!("entry {word}: block missing"));
                    continue;
                };
                let (Ok(offset), Ok(length)) = (usize::try_from(offset), usize::try_from(length))
                else {
                    errors.push(format!("entry {word}: negative offset/length"));
                    continue;
                };
                let Some(slice) = raw.get(offset..offset + length) else {
                    errors.push(format!("entry {word}: slice outside block"));
                    continue;
                };
                let record = match block::decode_record(slice) {
                    Ok(record) => record,
                    Err(error) => {
                        errors.push(format!("entry {word}: {error}"));
                        continue;
                    }
                };
                if record.key != word.as_bytes() {
                    errors.push(format!("entry {word}: block key does not match"));
                }
                let digest = hash::blake3(&record.payload);
                entry_root.push(hash::leaf_hash(word.as_bytes(), &digest));
                if !digest_matches(&content_hash, &digest) {
                    errors.push(format!("entry {word}: content hash mismatch"));
                }
                match Entry::decode_payload(&record.payload) {
                    Ok(entry) => {
                        if entry.display != display {
                            errors.push(format!("entry {word}: display column mismatch"));
                        }
                    }
                    Err(error) => errors.push(format!("entry {word}: {error}")),
                }
                match field_hashes_match(&field_hashes, &record.payload) {
                    Ok(true) => {}
                    Ok(false) => errors.push(format!("entry {word}: field hashes mismatch")),
                    Err(error) => errors.push(format!("entry {word}: {error}")),
                }
            }
        }

        let head_entry_root = entry_root.finish();

        {
            let mut statement = self.connection.prepare(
                "SELECT commit_hash, parent, parent2, entry_root, block_root, entry_count, ts,
                        author, message FROM commits",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                ))
            })?;
            for row in rows {
                let (
                    hash_bytes,
                    parent,
                    parent2,
                    entry_root,
                    block_root,
                    count,
                    _ts,
                    _author,
                    _message,
                ) = row?;
                commits += 1;
                let (Ok(parent), Ok(parent2), Ok(entry_root), Ok(block_root)) = (
                    hash_from_blob(&parent),
                    hash_from_blob(&parent2),
                    hash_from_blob(&entry_root),
                    hash_from_blob(&block_root),
                ) else {
                    errors.push("commit column is not a 32-byte hash".to_string());
                    continue;
                };
                let Ok(count) = u64::try_from(count) else {
                    errors.push("negative entry count".to_string());
                    continue;
                };
                let recomputed =
                    hash::commit_hash(&parent, &parent2, &entry_root, &block_root, count);
                match hash_from_blob(&hash_bytes) {
                    Ok(stored) if stored == recomputed => {}
                    Ok(stored) => errors.push(format!(
                        "commit {}: hash does not match fields",
                        hash::to_hex(&stored)
                    )),
                    Err(_) => errors.push("commit hash column is not 32 bytes".to_string()),
                }
                match self.snapshot_summary(&recomputed) {
                    Ok((snapshot_root, snapshot_count, snapshot_size, decoded_size)) => {
                        if snapshot_size != decoded_size {
                            errors.push(format!(
                                "commit {}: commit_index raw_size does not match its payload",
                                hash::to_hex(&recomputed)
                            ));
                        }
                        if snapshot_root != entry_root {
                            errors.push(format!(
                                "commit {}: entry_root does not match snapshot",
                                hash::to_hex(&recomputed)
                            ));
                        }
                        if snapshot_count != count {
                            errors.push(format!(
                                "commit {}: entry_count does not match snapshot",
                                hash::to_hex(&recomputed)
                            ));
                        }
                    }
                    Err(error) => errors.push(error.to_string()),
                }
            }
        }

        if let Some(head) = self.head()? {
            match self.commit_record(&head)? {
                Some(record) => {
                    if record.entry_root != head_entry_root {
                        errors.push("HEAD: entry_root does not match index".to_string());
                    }
                }
                None => errors.push("HEAD: commit row missing".to_string()),
            }
        }

        Ok(VerifyReport {
            ok: errors.is_empty(),
            blocks,
            entries,
            commits,
            errors,
        })
    }

    /// Delete blocks and commit indexes that no reachable commit needs.
    ///
    /// The retained set is every ref target plus the most recent keep_last
    /// commits of its first-parent chain. Blocks survive when the current
    /// entry index references them, so blocks used only by older retained
    /// revisions are collected: their commit indexes stay, which keeps
    /// `log` and `diff` working. `dry_run` reports what would be freed
    /// without writing; `vacuum` rebuilds the database file afterwards.
    pub fn gc(&mut self, keep_last: usize, dry_run: bool, vacuum: bool) -> Result<GcReport> {
        self.ensure_writable()?;
        if keep_last == 0 {
            return Err(Error::InvalidArgument(
                "gc requires --keep-last of at least 1".to_string(),
            ));
        }
        let mut retained: HashSet<[u8; 32]> = HashSet::new();
        {
            let mut tips = Vec::new();
            {
                let mut statement = self.connection.prepare("SELECT commit_hash FROM refs")?;
                let mut rows = statement.query([])?;
                while let Some(row) = rows.next()? {
                    tips.push(hash_from_blob(&row.get::<_, Vec<u8>>(0)?)?);
                }
            }
            for tip in tips {
                let mut current = Some(tip);
                let mut steps = 0usize;
                while let Some(hash) = current {
                    if steps >= keep_last || !retained.insert(hash) {
                        break;
                    }
                    steps += 1;
                    current = match self.commit_record(&hash)? {
                        Some(record) if record.parent != ZERO_HASH => Some(record.parent),
                        _ => None,
                    };
                }
            }
        }

        let mut removed_commits = Vec::new();
        {
            let mut statement = self.connection.prepare("SELECT commit_hash FROM commits")?;
            let mut rows = statement.query([])?;
            while let Some(row) = rows.next()? {
                let hash = hash_from_blob(&row.get::<_, Vec<u8>>(0)?)?;
                if !retained.contains(&hash) {
                    removed_commits.push(hash);
                }
            }
        }

        let mut removed_indexes = Vec::new();
        let mut freed_index_bytes = 0u64;
        {
            let mut statement = self
                .connection
                .prepare("SELECT commit_hash, LENGTH(data) FROM commit_index")?;
            let mut rows = statement.query([])?;
            while let Some(row) = rows.next()? {
                let hash = hash_from_blob(&row.get::<_, Vec<u8>>(0)?)?;
                if !retained.contains(&hash) {
                    freed_index_bytes += u64::try_from(row.get::<_, i64>(1)?).unwrap_or(0);
                    removed_indexes.push(hash);
                }
            }
        }

        let mut removed_blocks = Vec::new();
        let mut freed_block_bytes = 0u64;
        {
            let mut statement = self.connection.prepare(
                // Resources are not part of any commit, so they are kept by
                // their own index instead of the entries that reference them.
                "SELECT block_hash, LENGTH(data) FROM blocks
                 WHERE NOT EXISTS (
                     SELECT 1 FROM entry_index WHERE entry_index.block_hash = blocks.block_hash
                 )
                 AND NOT EXISTS (
                     SELECT 1 FROM resources r WHERE r.block_id = blocks.id
                 )",
            )?;
            let mut rows = statement.query([])?;
            while let Some(row) = rows.next()? {
                let hash = hash_from_blob(&row.get::<_, Vec<u8>>(0)?)?;
                freed_block_bytes += u64::try_from(row.get::<_, i64>(1)?).unwrap_or(0);
                removed_blocks.push(hash);
            }
        }

        if !dry_run {
            // A delta points at its parent, and the parent index is about to
            // disappear for the oldest retained commit: pin it with a full
            // snapshot first.
            self.anchor_boundary_indexes(&retained)?;
            let transaction = self.connection.transaction()?;
            {
                let mut delete =
                    transaction.prepare("DELETE FROM commit_index WHERE commit_hash = ?1")?;
                for hash in &removed_indexes {
                    delete.execute(params![hash.as_slice()])?;
                }
            }
            {
                let mut delete =
                    transaction.prepare("DELETE FROM commits WHERE commit_hash = ?1")?;
                for hash in &removed_commits {
                    delete.execute(params![hash.as_slice()])?;
                }
            }
            {
                let mut delete = transaction.prepare("DELETE FROM blocks WHERE block_hash = ?1")?;
                for hash in &removed_blocks {
                    delete.execute(params![hash.as_slice()])?;
                }
            }
            transaction.commit()?;
            if vacuum {
                self.connection.execute_batch("VACUUM")?;
            }
        }

        Ok(GcReport {
            dry_run,
            kept_commits: retained.len(),
            removed_commits: removed_commits.len(),
            removed_indexes: removed_indexes.len(),
            removed_blocks: removed_blocks.len(),
            freed_block_bytes,
            freed_index_bytes,
            vacuumed: vacuum && !dry_run,
        })
    }

    /// Rewrite a retained delta entry as a full snapshot when the parent it
    /// diffs against is about to be deleted.
    ///
    /// Without this the oldest retained commit would keep a delta whose parent
    /// index no longer exists, and reading it would fail as a broken chain.
    fn anchor_boundary_indexes(&self, retained: &HashSet<[u8; 32]>) -> Result<usize> {
        let mut rewritten = 0;
        let hashes: Vec<[u8; 32]> = retained.iter().copied().collect();
        for hash in hashes {
            let Some(header) = index::index_header(&self.connection, &hash)? else {
                continue;
            };
            if header.kind != snapshot::EntryKind::Delta {
                continue;
            }
            let parent_kept = index::commit_parent(&self.connection, &hash)?
                .is_some_and(|parent| retained.contains(&parent));
            if parent_kept {
                continue;
            }
            self.rewrite_index_as_full(&hash)?;
            rewritten += 1;
        }
        Ok(rewritten)
    }

    /// Replace the entry stored for a commit with a full snapshot of the index
    /// it rebuilds today.
    fn rewrite_index_as_full(&self, hash: &[u8; 32]) -> Result<()> {
        let entry_count: i64 = self.connection.query_row(
            "SELECT entry_count FROM commits WHERE commit_hash = ?1",
            params![hash.as_slice()],
            |row| row.get(0),
        )?;
        let temp = TempPath::new(&self.dir, "commit-index");
        let (raw_size, compressed_size) = {
            let file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(temp.path())?;
            let encoder =
                zstd::stream::write::Encoder::new(std::io::BufWriter::new(file), ZSTD_LEVEL)
                    .map_err(|error| Error::Io(error.to_string()))?;
            let mut writer = snapshot::SnapshotWriter::full(
                encoder,
                u64::try_from(entry_count).unwrap_or(u64::MAX),
            )?;
            {
                let mut reader = self.snapshot_reader(hash)?;
                while let Some((word, digest)) = reader.next_pair()? {
                    writer.push(&word, &digest)?;
                }
            }
            finish_entry(writer, temp.path())?
        };
        self.connection.execute(
            "UPDATE commit_index SET codec = ?2, raw_size = ?3, data = zeroblob(?4)
             WHERE commit_hash = ?1",
            params![
                hash.as_slice(),
                codec::ZSTD,
                raw_size as i64,
                compressed_size as i64,
            ],
        )?;
        let rowid: i64 = self.connection.query_row(
            "SELECT rowid FROM commit_index WHERE commit_hash = ?1",
            params![hash.as_slice()],
            |row| row.get(0),
        )?;
        {
            let mut blob =
                self.connection
                    .blob_open("main", "commit_index", "data", rowid, false)?;
            let mut file = std::fs::File::open(temp.path())?;
            let mut buffer = vec![0u8; 64 * 1024];
            loop {
                let read = file.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                blob.write_all(&buffer[..read])?;
            }
        }
        Ok(())
    }
}
pub(crate) fn decode_block_data(codec_id: i64, raw_size: usize, data: &[u8]) -> Result<Vec<u8>> {
    match codec_id {
        codec::RAW => Ok(data.to_vec()),
        codec::ZSTD => decompress(data, raw_size),
        other => Err(Error::UnknownCodec(other)),
    }
}

pub(crate) fn field_hashes_of(payload: &[u8]) -> Result<Value> {
    let value = cbor::decode(payload)?;
    let mut pairs = Vec::new();
    for (key, item) in value.entries() {
        if let Some(name) = key.as_text() {
            let digest = hash::blake3(&cbor::encode(item));
            pairs.push((
                Value::Text(name.to_string()),
                Value::Bytes(digest[..store::DIGEST_PREFIX].to_vec()),
            ));
        }
    }
    Ok(Value::Map(pairs))
}

/// Compare a stored digest against a freshly computed one.
///
/// Version 1 stored the full 32 bytes, version 2 stores the first
/// [`store::DIGEST_PREFIX`] bytes; the comparison uses the shorter of the two so
/// both layouts verify against the same payload.
pub(crate) fn digest_matches(stored: &[u8], computed: &[u8; 32]) -> bool {
    let end = stored.len().min(computed.len());
    end > 0 && stored[..end] == computed[..end]
}

/// Normalise a `field_hashes` CBOR map into sorted (name, digest) pairs.
fn digest_map(value: &Value) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = value
        .entries()
        .filter_map(|(key, item)| Some((key.as_text()?.to_string(), item.as_bytes()?.to_vec())))
        .collect();
    out.sort_by(|left, right| left.0.cmp(&right.0));
    out
}

/// Compare a stored `field_hashes` blob with the digests of a payload.
fn field_hashes_match(stored: &[u8], payload: &[u8]) -> Result<bool> {
    let expected = digest_map(&field_hashes_of(payload)?);
    let actual = digest_map(&cbor::decode(stored)?);
    if expected.len() != actual.len() {
        return Ok(false);
    }
    Ok(expected
        .iter()
        .zip(&actual)
        .all(|((name, left), (other, right))| {
            name == other && left.len().min(right.len()) > 0 && {
                let end = left.len().min(right.len());
                left[..end] == right[..end]
            }
        }))
}

fn chrono_free_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}

/// One entry awaiting insertion into the new commit index.
struct Staged {
    word: String,
    display: String,
    content_hash: [u8; 32],
    field_hashes: Vec<u8>,
}

/// Store one resource payload and point the index at its block.
///
/// The payload is written verbatim (`codec::RAW`): MDD entries are already
/// compressed media and the block table is content addressed, so identical
/// payloads are stored once no matter how many keys reference them.
fn put_resource_row(connection: &Connection, key: &str, bytes: &[u8]) -> Result<()> {
    let key = crate::mdd_import::normalize_resource_key(key);
    let block_hash = hash::blake3(bytes);
    let existing: Option<i64> = connection
        .query_row(
            "SELECT id FROM blocks WHERE block_hash = ?1",
            params![block_hash.as_slice()],
            |row| row.get(0),
        )
        .optional()?;
    let block_id = match existing {
        Some(id) => id,
        None => {
            connection.execute(
                "INSERT INTO blocks(block_hash, codec, raw_size, data) VALUES (?1, ?2, ?3, ?4)",
                params![block_hash.as_slice(), codec::RAW, bytes.len() as i64, bytes],
            )?;
            connection.last_insert_rowid()
        }
    };
    connection.execute(
        "INSERT INTO resources(key, block_id, size) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET block_id = excluded.block_id, size = excluded.size",
        params![key, block_id, bytes.len() as i64],
    )?;
    Ok(())
}

/// How many blocks a commit wrote and how many it found already stored.
#[derive(Default)]
struct BlockTally {
    new_blocks: usize,
    reused_blocks: usize,
}

/// Encode one cut block, store it if new and index its entries.
fn write_block(
    connection: &Connection,
    entries: &mut rusqlite::Statement<'_>,
    exists: &mut rusqlite::Statement<'_>,
    insert: &mut rusqlite::Statement<'_>,
    records: &[Record],
    staged: &[Staged],
    tally: &mut BlockTally,
) -> Result<()> {
    if records.is_empty() {
        return Ok(());
    }
    let raw = block::encode(records)?;
    let block_hash = hash::blake3(&raw);
    let decoded = block::decode(&raw)?;
    let present: Option<i64> = exists
        .query_row(params![block_hash.as_slice()], |row| row.get(0))
        .optional()?;
    let block_id = match present {
        Some(id) => {
            tally.reused_blocks += 1;
            id
        }
        None => {
            let data = compress(&raw)?;
            insert.execute(params![
                block_hash.as_slice(),
                codec::ZSTD,
                raw.len() as i64,
                data,
            ])?;
            tally.new_blocks += 1;
            connection.last_insert_rowid()
        }
    };
    for (index, offset) in decoded.offsets.iter().enumerate() {
        let item = &staged[index];
        entries.execute(params![
            item.word,
            item.display,
            block_id,
            *offset as i64,
            records[index].encoded_len() as i64,
            &item.content_hash[..store::DIGEST_PREFIX],
            item.field_hashes,
        ])?;
    }
    Ok(())
}

/// Scratch file removed when it goes out of scope.
struct TempPath {
    path: PathBuf,
}

impl TempPath {
    fn new(dir: &Path, label: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let path = dir.join(format!(".odc-{label}-{}-{nanos}.tmp", std::process::id()));
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Result of a [`Repo::gc`] run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GcReport {
    /// True when nothing was written.
    pub dry_run: bool,
    /// Commits kept on the first-parent chains of the refs.
    pub kept_commits: usize,
    /// Commit rows removed.
    pub removed_commits: usize,
    /// Commit indexes removed.
    pub removed_indexes: usize,
    /// Stored blocks removed.
    pub removed_blocks: usize,
    /// Bytes of block data removed.
    pub freed_block_bytes: u64,
    /// Bytes of commit index data removed.
    pub freed_index_bytes: u64,
    /// True when the database was vacuumed.
    pub vacuumed: bool,
}

impl GcReport {
    /// Total payload bytes the run removed (or would remove).
    #[must_use]
    pub fn freed_bytes(&self) -> u64 {
        self.freed_block_bytes + self.freed_index_bytes
    }
}

/// One row of a merged tree that is about to become a commit.
///
/// Rows normally point at an existing block record (the slice); payloads that
/// only exist as a result of the merge (field-level combinations, union copies,
/// file resolutions) carry their bytes inline and get packed into new blocks.
pub(crate) struct MergeRow {
    /// Normalised key.
    pub word: String,
    /// Display form; decoded from the payload when absent.
    pub display: Option<String>,
    /// BLAKE3 of the canonical payload.
    pub content_hash: [u8; 32],
    /// Existing block record that stores this payload.
    pub slice: Option<([u8; 32], i64, i64)>,
    /// Payload bytes for rows that need a new block.
    pub payload: Option<Vec<u8>>,
}

/// Load and decompress one block.
pub(crate) fn load_block_raw_from(connection: &Connection, block_hash: &[u8]) -> Result<Vec<u8>> {
    let row = connection
        .query_row(
            "SELECT codec, raw_size, data FROM blocks WHERE block_hash = ?1",
            params![block_hash],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((codec_id, raw_size, data)) = row else {
        return Err(Error::UnknownBlock(hash::to_hex(&hash_from_blob(
            block_hash,
        )?)));
    };
    let raw_size = usize::try_from(raw_size)
        .map_err(|_| Error::Inconsistent("negative raw_size".to_string()))?;
    decode_block_data(codec_id, raw_size, &data)
}

/// Reader over block records with a single-block cache.
pub(crate) struct BlockReader<'a> {
    connection: &'a Connection,
    cache: Option<([u8; 32], Vec<u8>)>,
}

impl<'a> BlockReader<'a> {
    pub(crate) fn new(connection: &'a Connection) -> Self {
        Self {
            connection,
            cache: None,
        }
    }

    fn raw(&mut self, block: &[u8; 32]) -> Result<&[u8]> {
        let stale = self
            .cache
            .as_ref()
            .is_none_or(|(cached, _)| cached != block);
        if stale {
            let raw = load_block_raw_from(self.connection, block.as_slice())?;
            self.cache = Some((*block, raw));
        }
        Ok(&self
            .cache
            .as_ref()
            .expect("block cache is populated before use")
            .1)
    }

    /// Decode the record stored at the given slice.
    pub(crate) fn record(&mut self, slice: &([u8; 32], i64, i64)) -> Result<Record> {
        let (block_hash, offset, length) = *slice;
        let raw = self.raw(&block_hash)?;
        let start = usize::try_from(offset)
            .map_err(|_| Error::Inconsistent("negative record offset".to_string()))?;
        let length = usize::try_from(length)
            .map_err(|_| Error::Inconsistent("negative record length".to_string()))?;
        let end = start
            .checked_add(length)
            .ok_or_else(|| Error::Inconsistent("record slice overflow".to_string()))?;
        let bytes = raw
            .get(start..end)
            .ok_or_else(|| Error::Inconsistent("record slice outside block".to_string()))?;
        block::decode_record(bytes)
    }
}

/// Materialise one record from a committed block and verify it against the
/// working set's `content_hash`.
pub(crate) fn hydrate_payload(
    reader: &mut BlockReader<'_>,
    slice: &([u8; 32], i64, i64),
    word: &str,
    content_hash: &[u8],
) -> Result<Vec<u8>> {
    let record = reader.record(slice)?;
    if record.key != word.as_bytes() {
        return Err(Error::Inconsistent(format!(
            "block record does not match working entry {word}"
        )));
    }
    if !digest_matches(content_hash, &hash::blake3(&record.payload)) {
        return Err(Error::Inconsistent(format!(
            "block record for {word} does not match its content hash"
        )));
    }
    Ok(record.payload)
}

fn flush_merge_records(
    exists: &mut rusqlite::Statement<'_>,
    insert: &mut rusqlite::Statement<'_>,
    records: &mut Vec<Record>,
    locations: &mut HashMap<String, ([u8; 32], i64, i64)>,
    new_blocks: &mut usize,
    reused_blocks: &mut usize,
) -> Result<()> {
    if records.is_empty() {
        return Ok(());
    }
    let raw = block::encode(records)?;
    let block_hash = hash::blake3(&raw);
    let present: Option<i64> = exists
        .query_row(params![block_hash.as_slice()], |row| row.get(0))
        .optional()?;
    if present.is_some() {
        *reused_blocks += 1;
    } else {
        let data = compress(&raw)?;
        insert.execute(params![
            block_hash.as_slice(),
            codec::ZSTD,
            i64::try_from(raw.len())
                .map_err(|_| Error::Inconsistent("block too large".to_string()))?,
            data,
        ])?;
        *new_blocks += 1;
    }
    let decoded = block::decode(&raw)?;
    for (index, record) in records.iter().enumerate() {
        let word = String::from_utf8(record.key.clone())
            .map_err(|_| Error::Inconsistent("record key is not utf-8".to_string()))?;
        let offset = i64::try_from(decoded.offsets[index])
            .map_err(|_| Error::Inconsistent("block offset overflow".to_string()))?;
        let length = i64::try_from(record.encoded_len())
            .map_err(|_| Error::Inconsistent("record length overflow".to_string()))?;
        locations.insert(word, (block_hash, offset, length));
    }
    records.clear();
    Ok(())
}

/// Replace the entry index and the working set with the merged rows, packing
/// the payloads that have no block record of their own.
fn stage_merge_rows(connection: &Connection, rows: &mut [MergeRow]) -> Result<(usize, usize)> {
    {
        let mut reader = BlockReader::new(connection);
        for row in rows.iter_mut() {
            let Some(slice) = row.slice else {
                continue;
            };
            let record = reader.record(&slice)?;
            let key_matches = record.key == row.word.as_bytes();
            let hash_matches = hash::blake3(&record.payload) == row.content_hash;
            if !key_matches || !hash_matches {
                row.slice = None;
                row.payload = Some(record.payload);
            }
        }
    }
    for row in rows.iter() {
        if row.slice.is_none() && row.payload.is_none() {
            return Err(Error::Inconsistent(format!(
                "merge row {} has neither a slice nor a payload",
                row.word
            )));
        }
    }

    let mut new_blocks = 0usize;
    let mut reused_blocks = 0usize;
    let mut locations: HashMap<String, ([u8; 32], i64, i64)> = HashMap::new();
    {
        let mut exists = connection.prepare("SELECT 1 FROM blocks WHERE block_hash = ?1")?;
        let mut insert = connection.prepare(
            "INSERT INTO blocks(block_hash, codec, raw_size, data) VALUES (?1, ?2, ?3, ?4)",
        )?;
        let mut records: Vec<Record> = Vec::new();
        let mut chunker = cdc::StreamChunker::new();
        for row in rows.iter() {
            if row.slice.is_some() {
                continue;
            }
            let payload = row.payload.clone().ok_or_else(|| {
                Error::Inconsistent(format!("merge row {} has no payload", row.word))
            })?;
            let record = Record::new(row.word.clone(), payload);
            loop {
                match chunker.push(&record) {
                    cdc::Cut::Before => flush_merge_records(
                        &mut exists,
                        &mut insert,
                        &mut records,
                        &mut locations,
                        &mut new_blocks,
                        &mut reused_blocks,
                    )?,
                    cdc::Cut::None => {
                        records.push(record);
                        break;
                    }
                    cdc::Cut::After => {
                        records.push(record);
                        flush_merge_records(
                            &mut exists,
                            &mut insert,
                            &mut records,
                            &mut locations,
                            &mut new_blocks,
                            &mut reused_blocks,
                        )?;
                        break;
                    }
                }
            }
        }
        flush_merge_records(
            &mut exists,
            &mut insert,
            &mut records,
            &mut locations,
            &mut new_blocks,
            &mut reused_blocks,
        )?;
    }

    connection.execute("DELETE FROM entries", [])?;
    connection.execute("DELETE FROM staging", [])?;
    mark_workset_dirty(connection)?;
    {
        let mut insert_entry = connection.prepare(
            "INSERT INTO entries(word, display, block_id, offset, length, content_hash, field_hashes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        let mut block_id_of = connection.prepare("SELECT id FROM blocks WHERE block_hash = ?1")?;
        let mut insert_staging = connection
            .prepare("INSERT INTO staging(word, display, content_hash) VALUES (?1, ?2, ?3)")?;
        let mut reader = BlockReader::new(connection);
        for row in rows.iter() {
            let payload = match row.slice {
                Some(slice) => reader.record(&slice)?.payload,
                None => row.payload.clone().ok_or_else(|| {
                    Error::Inconsistent(format!("merge row {} has no payload", row.word))
                })?,
            };
            if hash::blake3(&payload) != row.content_hash {
                return Err(Error::Inconsistent(format!(
                    "merge row {} has a payload that does not match its hash",
                    row.word
                )));
            }
            let display = match &row.display {
                Some(display) => display.clone(),
                None => Entry::decode_payload(&payload)?.display,
            };
            let display = if display.is_empty() {
                row.word.clone()
            } else {
                display
            };
            let field_hashes = cbor::encode(&field_hashes_of(&payload)?);
            let (block_hash, offset, length) = row
                .slice
                .or_else(|| locations.get(&row.word).copied())
                .ok_or_else(|| {
                    Error::Inconsistent(format!("merge row {} has no block location", row.word))
                })?;
            let block_id: i64 =
                block_id_of.query_row(params![block_hash.as_slice()], |row| row.get(0))?;
            insert_entry.execute(params![
                &row.word,
                &display,
                block_id,
                offset,
                length,
                &row.content_hash[..store::DIGEST_PREFIX],
                field_hashes,
            ])?;
            insert_staging.execute(params![
                &row.word,
                &display,
                &row.content_hash[..store::DIGEST_PREFIX],
            ])?;
        }
    }
    Ok((new_blocks, reused_blocks))
}

impl Repo {
    /// Write a merge commit whose parents are the two given commits.
    pub(crate) fn write_merge_commit(
        &mut self,
        parent: &[u8; 32],
        parent2: &[u8; 32],
        rows: &mut [MergeRow],
        message: &str,
        author: &str,
        ts: i64,
    ) -> Result<CommitOutcome> {
        let entry_count = u64::try_from(rows.len())
            .map_err(|_| Error::Inconsistent("too many entries".to_string()))?;
        let temp = TempPath::new(&self.dir, "merge-index");
        let transaction = self.connection.transaction()?;
        let (new_blocks, reused_blocks) = stage_merge_rows(&transaction, rows)?;

        let (entry_root, observed) = {
            // The merged rows carry the full 32-byte digest of their payload; the
            // index only stores a prefix, so the tree is folded from the rows.
            let mut ordered: Vec<(&str, [u8; 32])> = rows
                .iter()
                .map(|row| (row.word.as_str(), row.content_hash))
                .collect();
            ordered.sort_by(|left, right| left.0.cmp(right.0));
            let mut accumulator = hash::MerkleAccumulator::new();
            let mut count = 0u64;
            for (word, digest) in &ordered {
                accumulator.push(hash::leaf_hash(word.as_bytes(), digest));
                count += 1;
            }
            (accumulator.finish(), count)
        };
        if observed != entry_count {
            return Err(Error::Inconsistent(
                "merged index does not match the planned entry count".to_string(),
            ));
        }
        let block_root = {
            let mut accumulator = hash::MerkleAccumulator::new();
            let mut select = transaction
                .prepare("SELECT DISTINCT block_hash FROM entry_index ORDER BY block_hash ASC")?;
            let mut rows = select.query([])?;
            while let Some(row) = rows.next()? {
                let digest = hash_from_blob(&row.get::<_, Vec<u8>>(0)?)?;
                accumulator.push(digest);
            }
            accumulator.finish()
        };

        let commit_hash = hash::commit_hash(parent, parent2, &entry_root, &block_root, entry_count);
        let commit = CommitRecord {
            hash: commit_hash,
            parent: *parent,
            parent2: *parent2,
            entry_root,
            block_root,
            entry_count,
            ts,
            author: author.to_string(),
            message: message.to_string(),
        };
        transaction.execute(
            "INSERT INTO commits(commit_hash, parent, parent2, entry_root, block_root,
             entry_count, ts, author, message) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                commit.hash.as_slice(),
                commit.parent.as_slice(),
                commit.parent2.as_slice(),
                commit.entry_root.as_slice(),
                commit.block_root.as_slice(),
                commit.entry_count as i64,
                commit.ts,
                commit.author,
                commit.message,
            ],
        )?;

        let (raw_size, compressed_size) = {
            let file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(temp.path())?;
            let encoder =
                zstd::stream::write::Encoder::new(std::io::BufWriter::new(file), ZSTD_LEVEL)
                    .map_err(|error| Error::Io(error.to_string()))?;
            let mut writer = snapshot::SnapshotWriter::full(encoder, entry_count)?;
            {
                // The merged rows carry the full digest; `staging` only keeps
                // the prefix that the working-set comparison needs.
                let mut ordered: Vec<(&str, [u8; 32])> = rows
                    .iter()
                    .map(|row| (row.word.as_str(), row.content_hash))
                    .collect();
                ordered.sort_by(|left, right| left.0.cmp(right.0));
                for (word, digest) in &ordered {
                    writer.push(word, digest)?;
                }
            }
            let (encoder, raw_size) = writer.finish()?;
            let mut buffered = encoder
                .finish()
                .map_err(|error| Error::Io(error.to_string()))?;
            buffered.flush()?;
            let compressed_size = std::fs::metadata(temp.path())?.len();
            (raw_size, compressed_size)
        };
        transaction.execute(
            "INSERT INTO commit_index(commit_hash, codec, raw_size, data)
             VALUES (?1, ?2, ?3, zeroblob(?4))",
            params![
                commit.hash.as_slice(),
                codec::ZSTD,
                raw_size as i64,
                compressed_size as i64,
            ],
        )?;
        {
            let rowid = transaction.last_insert_rowid();
            let mut blob = transaction.blob_open("main", "commit_index", "data", rowid, false)?;
            let mut source = std::fs::File::open(temp.path())?;
            let mut buffer = vec![0u8; 64 * 1024];
            let mut position = 0usize;
            loop {
                let read = source.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                blob.write_all(&buffer[..read])?;
                position += read;
            }
            if position != usize::try_from(compressed_size).unwrap_or(0) {
                return Err(Error::Inconsistent(
                    "merge snapshot was not copied completely".to_string(),
                ));
            }
        }
        transaction.execute(
            "INSERT INTO refs(name, commit_hash) VALUES ('HEAD', ?1)
             ON CONFLICT(name) DO UPDATE SET commit_hash = excluded.commit_hash",
            params![commit.hash.as_slice()],
        )?;
        transaction.commit()?;

        Ok(CommitOutcome {
            commit,
            new_blocks,
            reused_blocks,
            entries: usize::try_from(entry_count).unwrap_or(usize::MAX),
        })
    }

    /// Move HEAD to an existing commit and rebuild the index from its snapshot.
    pub(crate) fn fast_forward_tree(
        &mut self,
        rows: &mut [MergeRow],
        target: &[u8; 32],
    ) -> Result<()> {
        let transaction = self.connection.transaction()?;
        stage_merge_rows(&transaction, rows)?;
        transaction.execute(
            "INSERT INTO refs(name, commit_hash) VALUES ('HEAD', ?1)
             ON CONFLICT(name) DO UPDATE SET commit_hash = excluded.commit_hash",
            params![target.as_slice()],
        )?;
        transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod index_entry_tests {
    use super::*;
    use crate::entry::Def;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "odc-index-entry-{label}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn entry(index: usize) -> Entry {
        let mut entry = Entry::new(format!("word-{index:05}"));
        entry.phonetic = vec![format!("/wɜːd{index}/")];
        entry.defs = vec![Def {
            pos: "noun".to_string(),
            sense: format!("definition number {index}"),
            examples: vec![format!("an example with word-{index:05}")],
            labels: Vec::new(),
        }];
        entry
    }

    /// Bytes of the pre-delta encoding: a bare array of pairs.
    fn legacy_entry(pairs: &[(String, [u8; 32])]) -> (Vec<u8>, u64) {
        let value = Value::Array(
            pairs
                .iter()
                .map(|(word, digest)| {
                    Value::Array(vec![
                        Value::Text(word.clone()),
                        Value::Bytes(digest.to_vec()),
                    ])
                })
                .collect(),
        );
        let raw = cbor::encode(&value);
        (compress(&raw).expect("compress"), raw.len() as u64)
    }

    fn pairs_of(repo: &Repo, hash: &[u8; 32]) -> Vec<(String, [u8; 32])> {
        let mut reader = repo.snapshot_reader(hash).expect("reader");
        let mut pairs = Vec::new();
        while let Some(pair) = reader.next_pair().expect("pair") {
            pairs.push(pair);
        }
        pairs
    }

    fn rewrite_as_legacy(repo: &Repo) {
        let mut hashes = Vec::new();
        {
            let mut statement = repo
                .connection
                .prepare("SELECT commit_hash FROM commit_index")
                .expect("prepare");
            let mut rows = statement.query([]).expect("query");
            while let Some(row) = rows.next().expect("row") {
                hashes
                    .push(hash_from_blob(&row.get::<_, Vec<u8>>(0).expect("hash")).expect("blob"));
            }
        }
        for hash in hashes {
            let pairs = pairs_of(repo, &hash);
            let (data, raw_size) = legacy_entry(&pairs);
            repo.connection
                .execute(
                    "UPDATE commit_index SET raw_size = ?2, data = ?3 WHERE commit_hash = ?1",
                    params![hash.as_slice(), raw_size as i64, data],
                )
                .expect("rewrite");
        }
    }

    #[test]
    fn legacy_full_only_indexes_still_read() {
        let dir = temp_dir("legacy");
        let mut repo = Repo::init(&dir).expect("init");
        let mut entries: Vec<Entry> = (0..30).map(entry).collect();
        repo.import(&entries, true).expect("import");
        repo.commit("first", "tester", 1_700_600_000)
            .expect("commit");
        for step in 0..3 {
            entries[step].defs[0].sense = format!("revised {step}");
            repo.import(&entries, true).expect("import");
            repo.commit(
                &format!("step {step}"),
                "tester",
                1_700_600_001 + step as i64,
            )
            .expect("commit");
        }

        // Rewrite every entry the way the previous release did.
        rewrite_as_legacy(&repo);
        let storage = repo.index_storage().expect("storage");
        assert_eq!(storage.delta, 0);
        assert!(storage.full >= 5);

        let diff = repo.diff("HEAD~2", "HEAD").expect("diff");
        assert_eq!(diff.modified.len(), 2);
        assert!(repo.show("word-00029").expect("show").is_some());
        assert!(repo.verify().expect("verify").ok);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delta_encoding_shrinks_the_history() {
        let dir = temp_dir("size");
        let mut repo = Repo::init(&dir).expect("init");
        let mut entries: Vec<Entry> = (0..300).map(entry).collect();
        let mut new_bytes = 0u64;
        let mut old_bytes = 0u64;
        let mut new_raw = 0u64;
        let mut old_raw = 0u64;
        for step in 0..12 {
            entries[step].defs[0].sense = format!("revised at step {step}");
            repo.import(&entries, true).expect("import");
            let outcome = repo
                .commit(
                    &format!("step {step}"),
                    "tester",
                    1_700_700_000 + step as i64,
                )
                .expect("commit");
            let hash = outcome.commit.hash;
            let (raw_size, compressed): (i64, i64) = repo
                .connection
                .query_row(
                    "SELECT raw_size, LENGTH(data) FROM commit_index WHERE commit_hash = ?1",
                    params![hash.as_slice()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .expect("size");
            new_raw += u64::try_from(raw_size).unwrap_or(0);
            new_bytes += u64::try_from(compressed).unwrap_or(0);
            let (_, legacy_raw) = legacy_entry(&pairs_of(&repo, &hash));
            old_raw += legacy_raw;
            old_bytes += legacy_entry(&pairs_of(&repo, &hash)).0.len() as u64;
        }
        println!(
            "commit index bytes: old={old_bytes} new={new_bytes} (raw old={old_raw} new={new_raw})"
        );
        assert!(new_bytes * 4 < old_bytes, "deltas must be much smaller");
        assert!(new_raw < old_raw);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
