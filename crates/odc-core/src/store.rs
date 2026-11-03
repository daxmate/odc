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

/// Meta key that marks `staging` as a clean mirror of HEAD.
///
/// After a successful commit the working set is byte-for-byte identical to the
/// committed index, so the whole table is dropped and this flag records that.
/// The rows are rebuilt from `entries` the first time a writer needs them (see
/// `Repo::ensure_workset`). Values: `"1"` when staging is empty and equals
/// HEAD, absent otherwise.
pub const META_WORKSET_CLEAN: &str = "workset_clean";

/// Default database file name inside a repository directory.
pub const DB_FILE: &str = "odc.db";

/// SQLite layout version written into `meta.schema_version`.
///
/// Version 1 is the format shipped before 0.2: 32-byte digests in
/// `entries`, block references by hash, one full index snapshot per commit.
/// Version 2 stores block ids, 8-byte digest prefixes and delta snapshots.
/// Version 3 adds the `publishers` trust store (signature sidecars v1).
/// The `format_version` string does not change: every layout describes the
/// same block/commit model, and older repositories stay readable.
pub const SCHEMA_VERSION: i64 = 3;

/// Meta key holding the SQLite layout version.
pub const META_SCHEMA_VERSION: &str = "schema_version";

/// Number of leading digest bytes kept in the index.
///
/// `content_hash` and every `field_hashes` entry keep the first
/// `DIGEST_PREFIX` bytes of BLAKE3. 64 bits make an accidental collision
/// negligible (2^-64) while cutting the index metadata by 75%; the Merkle tree
/// still uses the full 32-byte digest, so commit hashes do not depend on it.
pub const DIGEST_PREFIX: usize = 8;

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
    id INTEGER PRIMARY KEY,
    block_hash BLOB NOT NULL UNIQUE,
    codec INTEGER NOT NULL,
    raw_size INTEGER NOT NULL,
    data BLOB NOT NULL
);
CREATE TABLE entries(
    word TEXT PRIMARY KEY,
    display TEXT NOT NULL,
    block_id INTEGER NOT NULL,
    offset INTEGER NOT NULL,
    length INTEGER NOT NULL,
    content_hash BLOB NOT NULL,
    field_hashes BLOB NOT NULL
);
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
CREATE TABLE publishers(
    public_key TEXT PRIMARY KEY,
    name TEXT NOT NULL DEFAULT '',
    added_at INTEGER NOT NULL,
    source TEXT NOT NULL
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
    set_meta(
        connection,
        META_SCHEMA_VERSION,
        SCHEMA_VERSION.to_string().as_bytes(),
    )?;
    install_entry_view(connection, SCHEMA_VERSION)?;
    Ok(())
}

/// Layout version of an open database; a missing marker means version 1.
pub fn schema_version(connection: &Connection) -> Result<i64> {
    let Some(raw) = get_meta(connection, META_SCHEMA_VERSION)? else {
        return Ok(1);
    };
    String::from_utf8_lossy(&raw)
        .parse::<i64>()
        .map_err(|_| Error::Inconsistent("meta.schema_version is not an integer".to_string()))
}

/// Install the connection-local `entry_index` view every read path uses.
///
/// Version 2 stores block references as `blocks.id`; the view joins the hash
/// back in so readers keep addressing blocks by content hash, exactly as they
/// did on version 1. Version 1 databases expose the columns they already have.
/// A TEMP view lives in the connection temporary schema, so opening a v1
/// repository for reading never writes to the file.
pub fn install_entry_view(connection: &Connection, version: i64) -> Result<()> {
    connection.execute("DROP VIEW IF EXISTS entry_index", [])?;
    let sql = if version >= 2 {
        "CREATE TEMP VIEW entry_index AS
         SELECT e.word AS word, e.display AS display, b.block_hash AS block_hash,
                e.block_id AS block_id, e.offset AS offset, e.length AS length,
                e.content_hash AS content_hash, e.field_hashes AS field_hashes
         FROM entries e JOIN blocks b ON b.id = e.block_id"
    } else {
        "CREATE TEMP VIEW entry_index AS
         SELECT word, display, block_hash, NULL AS block_id, offset, length,
                content_hash, field_hashes
         FROM entries"
    };
    connection.execute(sql, [])?;
    Ok(())
}

/// Statement that adds the MDD resource index.
///
/// Resource payloads stay in `blocks` with [`codec::RAW`]: an MDD entry is
/// already a compressed image or audio file, so recompressing it would only
/// cost time. The table maps the normalised resource path to its block and
/// keeps the payload size for reporting.
///
/// The table is created on every open, which keeps databases written by older
/// builds readable without a schema-version bump.
pub const RESOURCES_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS resources(
    key TEXT PRIMARY KEY,
    block_id INTEGER NOT NULL,
    size INTEGER NOT NULL
);";

/// Adds the `resources` table to a database of any layout version.
pub fn ensure_resources_table(connection: &Connection) -> Result<()> {
    connection.execute_batch(RESOURCES_SCHEMA)?;
    Ok(())
}

/// Statement that adds the publisher trust store (v0.10).
pub const PUBLISHERS_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS publishers(
    public_key TEXT PRIMARY KEY,
    name TEXT NOT NULL DEFAULT '',
    added_at INTEGER NOT NULL,
    source TEXT NOT NULL
);";

/// Add the `publishers` table to databases written before v0.10.
///
/// The table is created on every open, so version 1 and 2 repositories gain a
/// writable trust store without a destructive rewrite. The layout version is
/// bumped once the table exists, which keeps `schema_version` monotonic.
pub fn has_publishers_table(connection: &Connection) -> Result<bool> {
    let exists: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'publishers'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(exists.is_some())
}

pub fn ensure_publishers_table(connection: &Connection) -> Result<()> {
    connection.execute_batch(PUBLISHERS_SCHEMA)?;
    if schema_version(connection)? < SCHEMA_VERSION {
        set_meta(
            connection,
            META_SCHEMA_VERSION,
            SCHEMA_VERSION.to_string().as_bytes(),
        )?;
    }
    Ok(())
}

/// Rewrite a version 1 database into the current layout.
///
/// Makes the file writable as v2 and returns `true` when a rewrite happened.
/// The content-addressed payload (blocks, commits, refs) is untouched: only
/// the index columns move to block ids and digest prefixes.
pub fn upgrade_to_v2(connection: &Connection) -> Result<bool> {
    if schema_version(connection)? >= 2 {
        return Ok(false);
    }
    // The connection-local view references `entries`; drop it before the tables
    // are rebuilt and install the version 2 flavour once the rewrite commits.
    connection.execute("DROP VIEW IF EXISTS entry_index", [])?;
    let transaction = connection.unchecked_transaction()?;
    transaction.execute_batch(
        "CREATE TABLE blocks_v2(
             id INTEGER PRIMARY KEY,
             block_hash BLOB NOT NULL UNIQUE,
             codec INTEGER NOT NULL,
             raw_size INTEGER NOT NULL,
             data BLOB NOT NULL
         );
         INSERT INTO blocks_v2(block_hash, codec, raw_size, data)
             SELECT block_hash, codec, raw_size, data FROM blocks ORDER BY rowid;
         DROP TABLE blocks;
         ALTER TABLE blocks_v2 RENAME TO blocks;
         CREATE TABLE entries_v2(
             word TEXT PRIMARY KEY,
             display TEXT NOT NULL,
             block_id INTEGER NOT NULL,
             offset INTEGER NOT NULL,
             length INTEGER NOT NULL,
             content_hash BLOB NOT NULL,
             field_hashes BLOB NOT NULL
         );
         INSERT INTO entries_v2(word, display, block_id, offset, length, content_hash, field_hashes)
             SELECT e.word, e.display, b.id, e.offset, e.length,
                    substr(e.content_hash, 1, 8), e.field_hashes
             FROM entries e JOIN blocks b ON b.block_hash = e.block_hash;
         DROP TABLE entries;
         ALTER TABLE entries_v2 RENAME TO entries;
         CREATE TABLE staging_v2(
             word TEXT PRIMARY KEY,
             display TEXT NOT NULL,
             payload BLOB,
             content_hash BLOB NOT NULL
         );
         INSERT INTO staging_v2(word, display, payload, content_hash)
             SELECT word, display, payload, substr(content_hash, 1, 8) FROM staging;
         DROP TABLE staging;
         ALTER TABLE staging_v2 RENAME TO staging;",
    )?;
    truncate_field_hashes(&transaction)?;
    transaction.execute("DELETE FROM meta WHERE key = ?1", [META_WORKSET_CLEAN])?;
    transaction.commit()?;
    set_meta(
        connection,
        META_SCHEMA_VERSION,
        SCHEMA_VERSION.to_string().as_bytes(),
    )?;
    install_entry_view(connection, SCHEMA_VERSION)?;
    Ok(true)
}

/// Re-encode `field_hashes` maps at `DIGEST_PREFIX` bytes.
pub(crate) fn truncate_field_hashes(connection: &Connection) -> Result<()> {
    let mut updates: Vec<(String, Vec<u8>)> = Vec::new();
    {
        let mut statement = connection.prepare("SELECT word, field_hashes FROM entries")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let word: String = row.get(0)?;
            let encoded: Vec<u8> = row.get(1)?;
            let value = crate::cbor::decode(&encoded)?;
            let mut pairs = Vec::new();
            for (key, item) in value.entries() {
                let Some(name) = key.as_text() else {
                    continue;
                };
                let bytes = item.as_bytes().ok_or_else(|| {
                    Error::Inconsistent(format!("entry {word}: field hash is not a byte string"))
                })?;
                let end = bytes.len().min(DIGEST_PREFIX);
                pairs.push((
                    crate::cbor::Value::Text(name.to_string()),
                    crate::cbor::Value::Bytes(bytes[..end].to_vec()),
                ));
            }
            updates.push((word, crate::cbor::encode(&crate::cbor::Value::Map(pairs))));
        }
    }
    let mut update = connection.prepare("UPDATE entries SET field_hashes = ?2 WHERE word = ?1")?;
    for (word, encoded) in updates {
        update.execute(rusqlite::params![word, encoded])?;
    }
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
