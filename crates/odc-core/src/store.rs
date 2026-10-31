//! SQLite storage layer (spec section 9).
//!
//! The on-disk SQLite layout is an implementation detail: the normative format
//! is the block/commit model. Still, this module fixes the schema used by the
//! reference implementation so that databases can be inspected with any
//! SQLite tool.

use crate::error::{Error, Result};
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;

/// Format version string written into `meta`.
pub const FORMAT_VERSION: &str = "0.1";

/// Default database file name inside a repository directory.
pub const DB_FILE: &str = "odc.db";

/// Compression codec identifiers stored in `blocks.codec` / `commit_index.codec`.
#[allow(dead_code)]
pub mod codec {
    /// Block stored verbatim.
    pub const RAW: i64 = 0;
    /// Block stored with zstd.
    pub const ZSTD: i64 = 1;
}

const SCHEMA: &str = r"
CREATE TABLE meta(key TEXT PRIMARY KEY, value BLOB NOT NULL);
CREATE TABLE blocks(
    block_hash BLOB PRIMARY KEY,
    codec INTEGER NOT NULL,
    raw_size INTEGER NOT NULL,
    data BLOB NOT NULL
);
CREATE TABLE entries(
    word TEXT PRIMARY KEY,
    display TEXT NOT NULL,
    block_hash BLOB NOT NULL,
    offset INTEGER NOT NULL,
    length INTEGER NOT NULL,
    content_hash BLOB NOT NULL,
    field_hashes BLOB NOT NULL
);
CREATE INDEX entries_block ON entries(block_hash);
CREATE TABLE commits(
    commit_hash BLOB PRIMARY KEY,
    parent BLOB NOT NULL,
    parent2 BLOB NOT NULL,
    entry_root BLOB NOT NULL,
    block_root BLOB NOT NULL,
    entry_count INTEGER NOT NULL,
    ts INTEGER NOT NULL,
    author TEXT NOT NULL,
    message TEXT NOT NULL
);
CREATE TABLE commit_index(
    commit_hash BLOB PRIMARY KEY,
    codec INTEGER NOT NULL,
    raw_size INTEGER NOT NULL,
    data BLOB NOT NULL
);
CREATE TABLE refs(name TEXT PRIMARY KEY, commit_hash BLOB NOT NULL);
CREATE TABLE conflicts(id INTEGER PRIMARY KEY, word TEXT NOT NULL, detail TEXT NOT NULL);
CREATE TABLE staging(
    word TEXT PRIMARY KEY,
    display TEXT NOT NULL,
    payload BLOB,
    content_hash BLOB NOT NULL
);
";

/// Open an existing database and apply connection pragmas.
pub fn open(path: &Path) -> Result<Connection> {
    let connection = Connection::open(path)?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(connection)
}

/// Create every table and index. Fails if the schema already exists.
pub fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(SCHEMA)?;
    Ok(())
}

/// Read the stored format version string.
pub fn format_version(connection: &Connection) -> Result<Option<String>> {
    let value: Option<Vec<u8>> = connection
        .query_row(
            "SELECT value FROM meta WHERE key = 'format_version'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(value.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
}

/// Write a meta row.
pub fn set_meta(connection: &Connection, key: &str, value: &[u8]) -> Result<()> {
    connection.execute(
        "INSERT INTO meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

/// Read a meta row.
pub fn get_meta(connection: &Connection, key: &str) -> Result<Option<Vec<u8>>> {
    let value: Option<Vec<u8>> = connection
        .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()?;
    Ok(value)
}

/// Bring an existing database up to the current working-set schema.
///
/// Databases written before the working set stopped caching committed
/// payloads declare `staging.payload` as `NOT NULL`. SQLite cannot drop a
/// column constraint in place, so the table is rebuilt and its rows are
/// copied verbatim. Uncommitted payloads survive the migration unchanged.
pub fn migrate(connection: &Connection) -> Result<()> {
    let not_null: Option<i64> = connection
        .query_row(
            "SELECT \"notnull\" FROM pragma_table_info('staging') WHERE name = 'payload'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if not_null != Some(1) {
        return Ok(());
    }
    let transaction = connection.unchecked_transaction()?;
    transaction.execute_batch(
        "ALTER TABLE staging RENAME TO staging_not_null;
         CREATE TABLE staging(
             word TEXT PRIMARY KEY,
             display TEXT NOT NULL,
             payload BLOB,
             content_hash BLOB NOT NULL
         );
         INSERT INTO staging(word, display, payload, content_hash)
             SELECT word, display, payload, content_hash FROM staging_not_null;
         DROP TABLE staging_not_null;",
    )?;
    transaction.commit()?;
    Ok(())
}

/// Reject a database whose major version is not supported.
pub fn check_version(connection: &Connection) -> Result<()> {
    match format_version(connection)? {
        None => Err(Error::NotARepository(
            "meta.format_version missing".to_string(),
        )),
        Some(found) => {
            let major = found.split('.').next().unwrap_or_default();
            if major == "0" {
                Ok(())
            } else {
                Err(Error::UnsupportedVersion {
                    found,
                    supported: FORMAT_VERSION,
                })
            }
        }
    }
}
