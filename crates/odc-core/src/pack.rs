//! Export a repository into a single portable `.odc` file.
//!
//! The exported file is a fresh SQLite database with the reference schema. Only
//! objects reachable from HEAD are written: the commit chain (or just HEAD for
//! `--shallow`), the matching `commit_index` snapshots, the current entry
//! index, and the blocks it points at. Uncommitted state (`staging`), merge
//! conflicts and stale blocks are never exported.

use crate::error::{Error, Result};
use crate::hash;
use crate::store;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Options for [`super::repo::Repo::pack`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PackOptions {
    /// Export only HEAD's commit instead of its reachable history.
    pub shallow: bool,
    /// Replace an existing output file.
    pub force: bool,
}

/// Result of a pack run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackReport {
    /// File that was written.
    pub path: PathBuf,
    /// Whether missing parents are expected in the exported file.
    pub shallow: bool,
    /// Commit rows written.
    pub commits: u64,
    /// Commit index snapshots written.
    pub commit_indexes: u64,
    /// Blocks written.
    pub blocks: u64,
    /// Entry index rows written.
    pub entries: u64,
    /// Size of the output file in bytes.
    pub bytes: u64,
}

/// Meta key that marks a file whose history starts at HEAD.
pub const SHALLOW_META_KEY: &str = "shallow";

/// Pack `connection` (backed by `db_path`) into `out`.
pub(crate) fn pack(
    connection: &Connection,
    db_path: &Path,
    head: [u8; 32],
    out: &Path,
    options: PackOptions,
    head_index: Option<(&[u8; 32], &[u8], u64)>,
) -> Result<PackReport> {
    if out.exists() {
        if same_file(out, db_path) {
            return Err(Error::InvalidArgument(format!(
                "refusing to overwrite the repository database {}",
                out.display()
            )));
        }
        if !options.force {
            return Err(Error::InvalidArgument(format!(
                "output already exists: {} (use --force to replace it)",
                out.display()
            )));
        }
        if !out.is_file() {
            return Err(Error::InvalidArgument(format!(
                "output is not a regular file: {}",
                out.display()
            )));
        }
        std::fs::remove_file(out)?;
    }

    let source_shallow = is_shallow(connection)?;
    let shallow = options.shallow || source_shallow;
    let commits = reachable_commits(connection, head, options.shallow)?;

    let result = write_pack(db_path, out, &commits, shallow, head_index);
    if result.is_err() {
        // Do not leave a half-written export behind.
        let _ = std::fs::remove_file(out);
    }
    let mut report = result?;
    report.bytes = std::fs::metadata(out)?.len();
    Ok(report)
}

/// SQLite layout version of the attached source database.
fn source_schema_version(connection: &Connection) -> Result<i64> {
    let raw: Option<Vec<u8>> = connection
        .query_row(
            "SELECT value FROM src.meta WHERE key = ?1",
            params![store::META_SCHEMA_VERSION],
            |row| row.get(0),
        )
        .optional()?;
    Ok(raw
        .and_then(|bytes| String::from_utf8_lossy(&bytes).trim().parse::<i64>().ok())
        .unwrap_or(1))
}

/// Whether the attached `src` database has a table called `name`.
fn source_has_table(transaction: &rusqlite::Transaction<'_>, name: &str) -> Result<bool> {
    let count: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM src.sqlite_master WHERE type = 'table' AND name = ?1",
        params![name],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

/// Whether the repository marks itself as shallow.
pub(crate) fn is_shallow(connection: &Connection) -> Result<bool> {
    Ok(
        store::get_meta(connection, SHALLOW_META_KEY)?
            .is_some_and(|value| value.as_slice() == b"1"),
    )
}

fn write_pack(
    db_path: &Path,
    out: &Path,
    commits: &[[u8; 32]],
    shallow: bool,
    head_index: Option<(&[u8; 32], &[u8], u64)>,
) -> Result<PackReport> {
    let mut target = store::open(out)?;
    store::create_schema(&target)?;
    store::ensure_resources_table(&target)?;
    target.execute(
        "ATTACH DATABASE ?1 AS src",
        params![db_path.to_string_lossy().as_ref()],
    )?;

    let transaction = target.transaction()?;
    transaction.execute(
        "INSERT INTO main.meta(key, value)
         SELECT key, value FROM src.meta
         WHERE key NOT IN ('shallow', 'merge', 'schema_version', 'workset_clean')",
        [],
    )?;
    transaction.execute(
        "INSERT INTO main.refs(name, commit_hash)
         SELECT name, commit_hash FROM src.refs",
        [],
    )?;
    {
        let mut insert_commit = transaction.prepare(
            "INSERT INTO main.commits(commit_hash, parent, parent2, entry_root, block_root,
                                      entry_count, ts, author, message)
             SELECT commit_hash, parent, parent2, entry_root, block_root,
                    entry_count, ts, author, message
             FROM src.commits WHERE commit_hash = ?1",
        )?;
        let mut insert_index = transaction.prepare(
            "INSERT INTO main.commit_index(commit_hash, codec, raw_size, data)
             SELECT commit_hash, codec, raw_size, data
             FROM src.commit_index WHERE commit_hash = ?1",
        )?;
        for hash in commits {
            insert_commit.execute([hash.as_slice()])?;
            insert_index.execute([hash.as_slice()])?;
        }
        if let Some((hash, data, raw_size)) = head_index {
            // A shallow pack carries a single commit, so its index cannot
            // point at a parent: store the rebuilt full snapshot instead.
            transaction.execute(
                "UPDATE main.commit_index SET codec = ?2, raw_size = ?3, data = ?4
                 WHERE commit_hash = ?1",
                params![
                    hash.as_slice(),
                    store::codec::ZSTD,
                    i64::try_from(raw_size).unwrap_or(i64::MAX),
                    data,
                ],
            )?;
        }
    }
    // Block ids are copied verbatim: the source rowid becomes the destination
    // id, so entry rows keep pointing at the same block. A version 1 source
    // addresses blocks by hash, so the id is resolved while copying.
    let source_version = source_schema_version(&transaction)?;
    // Resources are optional: a database written before MDD support has no
    // table, and attaching it must not fail.
    let has_resources = source_has_table(&transaction, "resources")?;
    if shallow {
        // Entries and resources both point at blocks; a shallow pack keeps
        // exactly the payload its index can reach.
        let entries = if source_version >= 2 {
            "SELECT b.block_hash FROM src.entries e JOIN src.blocks b ON b.id = e.block_id"
        } else {
            "SELECT block_hash FROM src.entries"
        };
        let referenced = if has_resources {
            format!(
                "{entries} UNION
                 SELECT b.block_hash FROM src.resources r JOIN src.blocks b ON b.id = r.block_id"
            )
        } else {
            entries.to_string()
        };
        transaction.execute(
            &format!(
                "INSERT INTO main.blocks(id, block_hash, codec, raw_size, data)
                 SELECT rowid, block_hash, codec, raw_size, data FROM src.blocks
                 WHERE block_hash IN ({referenced})"
            ),
            [],
        )?;
    } else {
        // Every block left in the store was produced by a reachable commit;
        // `gc` is what removes blocks no retained revision needs.
        transaction.execute(
            "INSERT INTO main.blocks(id, block_hash, codec, raw_size, data)
             SELECT rowid, block_hash, codec, raw_size, data FROM src.blocks",
            [],
        )?;
    }
    if source_version >= 2 {
        transaction.execute(
            "INSERT INTO main.entries(word, display, block_id, offset, length,
                                      content_hash, field_hashes)
             SELECT word, display, block_id, offset, length, content_hash, field_hashes
             FROM src.entries",
            [],
        )?;
    } else {
        transaction.execute(
            "INSERT INTO main.entries(word, display, block_id, offset, length,
                                      content_hash, field_hashes)
             SELECT e.word, e.display, b.rowid, e.offset, e.length,
                    substr(e.content_hash, 1, ?1), e.field_hashes
             FROM src.entries e JOIN src.blocks b ON b.block_hash = e.block_hash",
            params![store::DIGEST_PREFIX as i64],
        )?;
        store::truncate_field_hashes(&transaction)?;
    }
    if has_resources {
        transaction.execute(
            "INSERT INTO main.resources(key, block_id, size)
             SELECT key, block_id, size FROM src.resources",
            [],
        )?;
    }
    if shallow {
        transaction.execute(
            "INSERT INTO main.meta(key, value) VALUES (?1, ?2)",
            params![SHALLOW_META_KEY, b"1".as_slice()],
        )?;
    }
    let counts = (
        count(&transaction, "commits")?,
        count(&transaction, "commit_index")?,
        count(&transaction, "blocks")?,
        count(&transaction, "entries")?,
    );
    transaction.commit()?;

    // Collapse the WAL so the export is one physical file.
    target.execute("DETACH DATABASE src", [])?;
    target.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode = DELETE;")?;
    drop(target);

    Ok(PackReport {
        path: out.to_path_buf(),
        shallow,
        commits: counts.0,
        commit_indexes: counts.1,
        blocks: counts.2,
        entries: counts.3,
        bytes: 0,
    })
}

fn count(connection: &Connection, table: &str) -> Result<u64> {
    let sql = format!("SELECT COUNT(*) FROM main.{table}");
    let value: i64 = connection.query_row(&sql, [], |row| row.get(0))?;
    Ok(u64::try_from(value).unwrap_or(0))
}

/// HEAD plus its reachable parents, or just HEAD when `shallow` is set.
///
/// Missing parents are a normal boundary: `gc` can truncate history and a
/// shallow file never contains them.
fn reachable_commits(
    connection: &Connection,
    head: [u8; 32],
    shallow: bool,
) -> Result<Vec<[u8; 32]>> {
    if head == hash::ZERO_HASH {
        return Err(Error::InvalidArgument(
            "pack requires at least one commit".to_string(),
        ));
    }
    let mut seen: HashSet<[u8; 32]> = HashSet::new();
    let mut queue = vec![head];
    let mut out = Vec::new();
    while let Some(current) = queue.pop() {
        if !seen.insert(current) {
            continue;
        }
        let parents: Option<(Vec<u8>, Vec<u8>)> = connection
            .query_row(
                "SELECT parent, parent2 FROM commits WHERE commit_hash = ?1",
                params![current.as_slice()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((parent, parent2)) = parents else {
            if current == head {
                return Err(Error::Inconsistent(format!(
                    "HEAD commit {} is missing from the commit table",
                    hash::to_hex(&current)
                )));
            }
            continue;
        };
        out.push(current);
        if shallow {
            continue;
        }
        for bytes in [parent, parent2] {
            let Ok(parent) = <[u8; 32]>::try_from(bytes.as_slice()) else {
                return Err(Error::Inconsistent(
                    "commit parent column is not 32 bytes".to_string(),
                ));
            };
            if parent != hash::ZERO_HASH {
                queue.push(parent);
            }
        }
    }
    Ok(out)
}

fn same_file(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}
