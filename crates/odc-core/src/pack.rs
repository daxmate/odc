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

    let result = write_pack(db_path, out, &commits, shallow);
    if result.is_err() {
        // Do not leave a half-written export behind.
        let _ = std::fs::remove_file(out);
    }
    let mut report = result?;
    report.bytes = std::fs::metadata(out)?.len();
    Ok(report)
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
) -> Result<PackReport> {
    let mut target = store::open(out)?;
    store::create_schema(&target)?;
    target.execute(
        "ATTACH DATABASE ?1 AS src",
        params![db_path.to_string_lossy().as_ref()],
    )?;

    let transaction = target.transaction()?;
    transaction.execute(
        "INSERT INTO main.meta(key, value)
         SELECT key, value FROM src.meta WHERE key NOT IN ('shallow', 'merge')",
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
    }
    transaction.execute(
        "INSERT INTO main.entries(word, display, block_hash, offset, length,
                                  content_hash, field_hashes)
         SELECT word, display, block_hash, offset, length, content_hash, field_hashes
         FROM src.entries",
        [],
    )?;
    if shallow {
        transaction.execute(
            "INSERT INTO main.blocks(block_hash, codec, raw_size, data)
             SELECT block_hash, codec, raw_size, data FROM src.blocks
             WHERE block_hash IN (SELECT block_hash FROM src.entries)",
            [],
        )?;
    } else {
        // Every block left in the store was produced by a reachable commit;
        // `gc` is what removes blocks no retained revision needs.
        transaction.execute(
            "INSERT INTO main.blocks(block_hash, codec, raw_size, data)
             SELECT block_hash, codec, raw_size, data FROM src.blocks",
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
