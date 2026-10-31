//! High level repository operations: init, import, commit, log, show, diff,
//! status and verify.

use crate::block::{self, Record};
use crate::cbor::{self, Value};
use crate::cdc;
use crate::entry::{normalize_key, Entry};
use crate::error::{Error, Result};
use crate::hash::{self, ZERO_HASH};
use crate::store::{self, codec, DB_FILE};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// Default author recorded when none is supplied.
pub const DEFAULT_AUTHOR: &str = "odc";

/// A checked-out repository backed by `odc.db`.
pub struct Repo {
    connection: Connection,
    dir: PathBuf,
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

/// A stored commit row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitRecord {
    /// Commit hash.
    pub hash: [u8; 32],
    /// First parent (zero for genesis).
    pub parent: [u8; 32],
    /// Second parent (zero in M0: merges are not produced yet).
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

fn compress(data: &[u8]) -> Result<Vec<u8>> {
    zstd::bulk::compress(data, 3).map_err(|error| Error::Io(error.to_string()))
}

fn decompress(data: &[u8], raw_size: usize) -> Result<Vec<u8>> {
    zstd::bulk::decompress(data, raw_size).map_err(|error| Error::Io(error.to_string()))
}

fn hash_from_blob(bytes: &[u8]) -> Result<[u8; 32]> {
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
        let created = chrono_free_now();
        store::set_meta(&connection, "created_at", created.to_string().as_bytes())?;
        let repo = Self { connection, dir };
        repo.write_genesis()?;
        Ok(repo)
    }

    /// Open an existing repository.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        let db_path = dir.join(DB_FILE);
        if !db_path.exists() {
            return Err(Error::NotARepository(dir.display().to_string()));
        }
        let connection = store::open(&db_path)?;
        store::check_version(&connection)?;
        Ok(Self { connection, dir })
    }

    /// Repository directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
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
        let snapshot = encode_snapshot(&[]);
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

    /// Insert or replace entries in the working set.
    pub fn import(&mut self, entries: &[Entry], prune: bool) -> Result<ImportReport> {
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
                    entry.content_hash().as_slice()
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

    /// Compare the working set with HEAD.
    pub fn status(&self) -> Result<Status> {
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
            head: self.head()?.map(|hash| hash::to_hex(&hash)),
        })
    }

    /// Commit the working set.
    pub fn commit(&mut self, message: &str, author: &str, ts: i64) -> Result<CommitOutcome> {
        let status = self.status()?;
        if status.is_clean() {
            return Err(Error::NoChanges);
        }
        let parent = self.head()?.unwrap_or(ZERO_HASH);

        struct Staged {
            word: String,
            display: String,
            payload: Vec<u8>,
            content_hash: [u8; 32],
        }

        let staged: Vec<Staged> = {
            let mut statement = self.connection.prepare(
                "SELECT word, display, payload, content_hash FROM staging ORDER BY word ASC",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                ))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (word, display, payload, content_hash) = row?;
                out.push(Staged {
                    word,
                    display,
                    payload,
                    content_hash: hash_from_blob(&content_hash)?,
                });
            }
            out
        };

        let records: Vec<Record> = staged
            .iter()
            .map(|item| Record::new(item.word.as_bytes().to_vec(), item.payload.clone()))
            .collect();
        let ranges = cdc::boundaries(&records);

        struct EncodedBlock {
            hash: [u8; 32],
            data: Vec<u8>,
            raw_size: usize,
            offsets: Vec<u64>,
            start: usize,
        }

        let mut encoded = Vec::with_capacity(ranges.len());
        for range in &ranges {
            let raw = block::encode(&records[range.clone()])?;
            let hash = hash::blake3(&raw);
            let data = compress(&raw)?;
            let decoded = block::decode(&raw)?;
            encoded.push(EncodedBlock {
                hash,
                raw_size: raw.len(),
                data,
                offsets: decoded.offsets,
                start: range.start,
            });
        }

        let leaves: Vec<[u8; 32]> = staged
            .iter()
            .map(|item| hash::leaf_hash(item.word.as_bytes(), &item.content_hash))
            .collect();
        let entry_root = hash::merkle_root(&leaves);
        let mut block_hashes: Vec<[u8; 32]> = encoded.iter().map(|item| item.hash).collect();
        block_hashes.sort_unstable();
        let block_root = hash::merkle_root(&block_hashes);
        let entry_count = u64::try_from(staged.len()).unwrap_or(u64::MAX);
        let commit_hash =
            hash::commit_hash(&parent, &ZERO_HASH, &entry_root, &block_root, entry_count);
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
        let snapshot_rows: Vec<(String, [u8; 32])> = staged
            .iter()
            .map(|item| (item.word.clone(), item.content_hash))
            .collect();
        let snapshot = encode_snapshot(&snapshot_rows);
        let snapshot_compressed = compress(&snapshot)?;
        let transaction = self.connection.transaction()?;
        transaction.execute("DELETE FROM entries", [])?;
        let mut new_blocks = 0usize;
        let mut reused_blocks = 0usize;
        for encoded_block in &encoded {
            let exists: Option<i64> = transaction
                .query_row(
                    "SELECT 1 FROM blocks WHERE block_hash = ?1",
                    params![encoded_block.hash.as_slice()],
                    |row| row.get(0),
                )
                .optional()?;
            if exists.is_some() {
                reused_blocks += 1;
            } else {
                transaction.execute(
                    "INSERT INTO blocks(block_hash, codec, raw_size, data) VALUES (?1, ?2, ?3, ?4)",
                    params![
                        encoded_block.hash.as_slice(),
                        codec::ZSTD,
                        encoded_block.raw_size as i64,
                        encoded_block.data
                    ],
                )?;
                new_blocks += 1;
            }
            for (index, offset) in encoded_block.offsets.iter().enumerate() {
                let record_index = encoded_block.start + index;
                let item = &staged[record_index];
                let length = records[record_index].encoded_len();
                transaction.execute(
                    "INSERT INTO entries(word, display, block_hash, offset, length,
                                         content_hash, field_hashes)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        item.word,
                        item.display,
                        encoded_block.hash.as_slice(),
                        *offset as i64,
                        length as i64,
                        item.content_hash.as_slice(),
                        cbor::encode(&field_hashes_of(&item.payload)?)
                    ],
                )?;
            }
        }
        transaction.execute(
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
        transaction.execute(
            "INSERT INTO commit_index(commit_hash, codec, raw_size, data) VALUES (?1, ?2, ?3, ?4)",
            params![
                commit.hash.as_slice(),
                codec::ZSTD,
                snapshot.len() as i64,
                snapshot_compressed
            ],
        )?;
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
            entries: staged.len(),
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
            let record = self
                .commit_record(&hash)?
                .ok_or_else(|| Error::UnknownCommit(hash::to_hex(&hash)))?;
            current = if record.parent == ZERO_HASH {
                None
            } else {
                Some(record.parent)
            };
            out.push(record);
        }
        Ok(out)
    }

    fn load_block_raw(&self, block_hash: &[u8]) -> Result<Vec<u8>> {
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
                "SELECT block_hash, offset, length FROM entries WHERE word = ?1",
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

    fn snapshot(&self, hash: &[u8; 32]) -> Result<BTreeMap<String, [u8; 32]>> {
        let row = self
            .connection
            .query_row(
                "SELECT codec, raw_size, data FROM commit_index WHERE commit_hash = ?1",
                params![hash.as_slice()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )
            .optional()?;
        let (codec_id, raw_size, data) = row.ok_or_else(|| {
            Error::Inconsistent(format!("missing commit_index for {}", hash::to_hex(hash)))
        })?;
        let raw_size = usize::try_from(raw_size)
            .map_err(|_| Error::Inconsistent("negative raw_size".to_string()))?;
        let raw = decode_block_data(codec_id, raw_size, &data)?;
        decode_snapshot(&raw)
    }

    /// Word-level diff between two revisions.
    pub fn diff(&self, from: &str, to: &str) -> Result<Diff> {
        let from_hash = self.resolve(from)?;
        let to_hash = self.resolve(to)?;
        let left = self.snapshot(&from_hash)?;
        let right = self.snapshot(&to_hash)?;
        let mut added = Vec::new();
        let mut removed = Vec::new();
        let mut modified = Vec::new();
        for (word, digest) in &right {
            match left.get(word) {
                None => added.push(word.clone()),
                Some(previous) if previous != digest => modified.push(word.clone()),
                Some(_) => {}
            }
        }
        for word in left.keys() {
            if !right.contains_key(word) {
                removed.push(word.clone());
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
        let mut block_cache: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();

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
                block_cache.insert(block_hash, raw);
            }
        }

        {
            let mut statement = self.connection.prepare(
                "SELECT word, display, block_hash, offset, length, content_hash, field_hashes
                 FROM entries",
            )?;
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
                let Some(raw) = block_cache.get(&block_hash) else {
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
                if hash::blake3(&record.payload)
                    != hash_from_blob(&content_hash).unwrap_or(ZERO_HASH)
                {
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
                match field_hashes_of(&record.payload) {
                    Ok(expected) => {
                        if cbor::encode(&expected) != field_hashes {
                            errors.push(format!("entry {word}: field hashes mismatch"));
                        }
                    }
                    Err(error) => errors.push(format!("entry {word}: {error}")),
                }
            }
        }

        let head_entry_root = {
            let mut statement = self
                .connection
                .prepare("SELECT word, content_hash FROM entries ORDER BY word ASC")?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })?;
            let mut leaves = Vec::new();
            for row in rows {
                let (word, content_hash) = row?;
                if let Ok(digest) = hash_from_blob(&content_hash) {
                    leaves.push(hash::leaf_hash(word.as_bytes(), &digest));
                }
            }
            hash::merkle_root(&leaves)
        };

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
                match self.snapshot(&recomputed) {
                    Ok(snapshot) => {
                        let leaves: Vec<[u8; 32]> = snapshot
                            .iter()
                            .map(|(word, digest)| hash::leaf_hash(word.as_bytes(), digest))
                            .collect();
                        if hash::merkle_root(&leaves) != entry_root {
                            errors.push(format!(
                                "commit {}: entry_root does not match snapshot",
                                hash::to_hex(&recomputed)
                            ));
                        }
                        if snapshot.len() as u64 != count {
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
}

fn decode_block_data(codec_id: i64, raw_size: usize, data: &[u8]) -> Result<Vec<u8>> {
    match codec_id {
        codec::RAW => Ok(data.to_vec()),
        codec::ZSTD => decompress(data, raw_size),
        other => Err(Error::UnknownCodec(other)),
    }
}

fn field_hashes_of(payload: &[u8]) -> Result<Value> {
    let value = cbor::decode(payload)?;
    let mut pairs = Vec::new();
    for (key, item) in value.entries() {
        if let Some(name) = key.as_text() {
            pairs.push((
                Value::Text(name.to_string()),
                Value::Bytes(hash::blake3(&cbor::encode(item)).to_vec()),
            ));
        }
    }
    Ok(Value::Map(pairs))
}

fn encode_snapshot(rows: &[(String, [u8; 32])]) -> Vec<u8> {
    let array = rows
        .iter()
        .map(|(word, digest)| {
            Value::Array(vec![
                Value::Text(word.clone()),
                Value::Bytes(digest.to_vec()),
            ])
        })
        .collect();
    cbor::encode(&Value::Array(array))
}

fn decode_snapshot(raw: &[u8]) -> Result<BTreeMap<String, [u8; 32]>> {
    let value = cbor::decode(raw)?;
    let items = value
        .as_array()
        .ok_or(Error::Inconsistent("snapshot is not an array".to_string()))?;
    let mut out = BTreeMap::new();
    for item in items {
        let pair = item.as_array().ok_or(Error::Inconsistent(
            "snapshot row is not an array".to_string(),
        ))?;
        let word = pair
            .first()
            .and_then(Value::as_text)
            .ok_or(Error::Inconsistent("snapshot row without word".to_string()))?;
        let digest = pair
            .get(1)
            .and_then(Value::as_bytes)
            .ok_or(Error::Inconsistent("snapshot row without hash".to_string()))?;
        out.insert(word.to_string(), hash_from_blob(digest)?);
    }
    Ok(out)
}

fn chrono_free_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}
