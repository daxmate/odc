//! High level repository operations: init, import, commit, log, show, diff,
//! status and verify.

use crate::block::{self, Record};
use crate::cbor::{self, Value};
use crate::cdc;
use crate::entry::{normalize_key, Entry};
use crate::error::{Error, Result};
use crate::hash::{self, ZERO_HASH};
use crate::snapshot;
use crate::store::{self, codec, DB_FILE};
use rusqlite::{params, Connection, OptionalExtension};
use std::io::{Read, Write};
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
        let snapshot = {
            let writer = snapshot::SnapshotWriter::new(Vec::new(), 0)?;
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
                    entry.content_hash().as_slice()
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

    /// Visit every committed entry in key order.
    ///
    /// Blocks are decoded lazily. The index is stored in key order and blocks
    /// were cut from the same order, so consecutive entries nearly always
    /// share a block; a one-block cache keeps memory flat.
    pub fn for_each_entry<F>(&self, mut visit: F) -> Result<usize>
    where
        F: FnMut(Entry) -> Result<()>,
    {
        let mut statement = self
            .connection
            .prepare("SELECT word, block_hash, offset, length FROM entries ORDER BY word ASC")?;
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
            entries: self.entry_count()?,
            blocks: self.block_count()?,
            head: self.head()?.map(|hash| hash::to_hex(&hash)),
        })
    }

    /// Commit the working set.
    /// Record the working set as a new commit.
    ///
    /// Nothing is bulk-loaded: the staging table is read once in word order,
    /// blocks are cut and written as soon as they are complete, the entry
    /// root is folded incrementally and the commit index is compressed
    /// straight into the database file. Peak memory is one block plus that
    /// block's entry metadata, never the whole dictionary.
    pub fn commit(&mut self, message: &str, author: &str, ts: i64) -> Result<CommitOutcome> {
        let status = self.status()?;
        if status.is_clean() {
            return Err(Error::NoChanges);
        }
        let parent = self.head()?.unwrap_or(ZERO_HASH);
        let temp = TempPath::new(&self.dir, "commit-index");

        let transaction = self.connection.transaction()?;
        transaction.execute("DELETE FROM entries", [])?;

        let mut new_blocks = 0usize;
        let mut reused_blocks = 0usize;
        let mut entry_count = 0u64;
        let mut entries_root = hash::MerkleAccumulator::new();
        let mut records: Vec<Record> = Vec::new();
        let mut staged: Vec<Staged> = Vec::new();
        let mut chunker = cdc::StreamChunker::new();

        {
            let mut select = transaction.prepare(
                "SELECT word, display, payload, content_hash FROM staging ORDER BY word ASC",
            )?;
            let mut insert_entry = transaction.prepare(
                "INSERT INTO entries(word, display, block_hash, offset, length,
                                     content_hash, field_hashes)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            let mut block_exists =
                transaction.prepare("SELECT 1 FROM blocks WHERE block_hash = ?1")?;
            let mut insert_block = transaction.prepare(
                "INSERT INTO blocks(block_hash, codec, raw_size, data) VALUES (?1, ?2, ?3, ?4)",
            )?;

            let mut rows = select.query([])?;
            while let Some(row) = rows.next()? {
                let word: String = row.get(0)?;
                let display: String = row.get(1)?;
                let payload: Vec<u8> = row.get(2)?;
                let content_hash = hash_from_blob(&row.get::<_, Vec<u8>>(3)?)?;
                let field_hashes = cbor::encode(&field_hashes_of(&payload)?);
                entries_root.push(hash::leaf_hash(word.as_bytes(), &content_hash));
                entry_count += 1;

                let record = Record::new(word.as_bytes().to_vec(), payload);
                let mut cut = chunker.push(&record);
                if cut == cdc::Cut::Before {
                    write_block(
                        &mut insert_entry,
                        &mut block_exists,
                        &mut insert_block,
                        &records,
                        &staged,
                        &mut new_blocks,
                        &mut reused_blocks,
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
                        &mut insert_entry,
                        &mut block_exists,
                        &mut insert_block,
                        &records,
                        &staged,
                        &mut new_blocks,
                        &mut reused_blocks,
                    )?;
                    records.clear();
                    staged.clear();
                }
            }
            if !records.is_empty() {
                write_block(
                    &mut insert_entry,
                    &mut block_exists,
                    &mut insert_block,
                    &records,
                    &staged,
                    &mut new_blocks,
                    &mut reused_blocks,
                )?;
                records.clear();
                staged.clear();
            }
        }

        let block_root = {
            let mut accumulator = hash::MerkleAccumulator::new();
            let mut select = transaction
                .prepare("SELECT DISTINCT block_hash FROM entries ORDER BY block_hash ASC")?;
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

        let (raw_size, compressed_size) = {
            let file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(temp.path())?;
            let encoder = zstd::stream::write::Encoder::new(std::io::BufWriter::new(file), 3)
                .map_err(|error| Error::Io(error.to_string()))?;
            let mut writer = snapshot::SnapshotWriter::new(encoder, entry_count)?;
            {
                let mut select = transaction
                    .prepare("SELECT word, content_hash FROM entries ORDER BY word ASC")?;
                let mut rows = select.query([])?;
                while let Some(row) = rows.next()? {
                    let word: String = row.get(0)?;
                    let digest = hash_from_blob(&row.get::<_, Vec<u8>>(1)?)?;
                    writer.push(&word, &digest)?;
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

        Ok(CommitOutcome {
            commit,
            new_blocks,
            reused_blocks,
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

    /// Open a streaming reader over the commit index stored for `hash`.
    ///
    /// The compressed blob is decoded on the fly and only one entry is held in
    /// memory at a time.
    fn snapshot_reader(&self, hash: &[u8; 32]) -> Result<snapshot::SnapshotReader<impl Read + '_>> {
        let rowid: Option<i64> = self
            .connection
            .query_row(
                "SELECT rowid FROM commit_index WHERE commit_hash = ?1",
                params![hash.as_slice()],
                |row| row.get(0),
            )
            .optional()?;
        let rowid = rowid.ok_or_else(|| {
            Error::Inconsistent(format!("missing commit_index for {}", hash::to_hex(hash)))
        })?;
        let blob = self
            .connection
            .blob_open("main", "commit_index", "data", rowid, true)?;
        let decoder =
            zstd::stream::read::Decoder::new(blob).map_err(|error| Error::Io(error.to_string()))?;
        snapshot::SnapshotReader::new(decoder)
    }

    /// Merkle root and entry count of the snapshot stored for `hash`, folded
    /// while streaming.
    fn snapshot_summary(&self, hash: &[u8; 32]) -> Result<([u8; 32], u64)> {
        let mut reader = self.snapshot_reader(hash)?;
        let mut accumulator = hash::MerkleAccumulator::new();
        let mut count = 0u64;
        while let Some((word, digest)) = reader.next_pair()? {
            accumulator.push(hash::leaf_hash(word.as_bytes(), &digest));
            count += 1;
        }
        Ok((accumulator.finish(), count))
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
            let mut statement = self.connection.prepare(
                "SELECT word, display, block_hash, offset, length, content_hash, field_hashes
                 FROM entries ORDER BY block_hash",
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
            let mut accumulator = hash::MerkleAccumulator::new();
            for row in rows {
                let (word, content_hash) = row?;
                if let Ok(digest) = hash_from_blob(&content_hash) {
                    accumulator.push(hash::leaf_hash(word.as_bytes(), &digest));
                }
            }
            accumulator.finish()
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
                match self.snapshot_summary(&recomputed) {
                    Ok((snapshot_root, snapshot_count)) => {
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

/// Encode one cut block, store it if new and index its entries.
fn write_block(
    entries: &mut rusqlite::Statement<'_>,
    exists: &mut rusqlite::Statement<'_>,
    insert: &mut rusqlite::Statement<'_>,
    records: &[Record],
    staged: &[Staged],
    new_blocks: &mut usize,
    reused_blocks: &mut usize,
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
    if present.is_some() {
        *reused_blocks += 1;
    } else {
        let data = compress(&raw)?;
        insert.execute(params![
            block_hash.as_slice(),
            codec::ZSTD,
            raw.len() as i64,
            data,
        ])?;
        *new_blocks += 1;
    }
    for (index, offset) in decoded.offsets.iter().enumerate() {
        let item = &staged[index];
        entries.execute(params![
            item.word,
            item.display,
            block_hash.as_slice(),
            *offset as i64,
            records[index].encoded_len() as i64,
            item.content_hash.as_slice(),
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
