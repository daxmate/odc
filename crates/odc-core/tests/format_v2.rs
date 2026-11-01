//! Layout version 2: block ids, digest prefixes and version 1 read
//! compatibility.

use odc_core::entry::{Def, Entry};
use odc_core::repo::Repo;
use odc_core::store::{self, DB_FILE};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let dir = std::env::temp_dir().join(format!("odc-v2-{label}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn cleanup(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

fn entry(word: &str, sense: &str) -> Entry {
    let mut entry = Entry::new(word);
    entry.phonetic = vec![format!("/{word}/")];
    entry.defs = vec![Def {
        pos: "noun".to_string(),
        sense: sense.to_string(),
        examples: vec![format!("an example with {word}")],
        labels: Vec::new(),
    }];
    entry
}

fn seed(dir: &Path, words: &[(&str, &str)]) -> Vec<Entry> {
    let mut repo = Repo::init(dir).expect("init");
    let entries: Vec<Entry> = words
        .iter()
        .map(|(word, sense)| entry(word, sense))
        .collect();
    repo.import(&entries, false).expect("import");
    repo.commit("seed", "tester", 1_700_000_000)
        .expect("commit");
    entries
}

fn column_len(dir: &Path, table: &str, column: &str) -> i64 {
    let connection = Connection::open(dir.join(DB_FILE)).expect("open sqlite");
    connection
        .query_row(
            &format!("SELECT MAX(LENGTH({column})) FROM {table}"),
            [],
            |row| row.get::<_, Option<i64>>(0),
        )
        .expect("length")
        .unwrap_or(0)
}

#[test]
fn new_repositories_declare_layout_two() {
    let dir = temp_dir("layout");
    seed(&dir, &[("alpha", "first"), ("beta", "second")]);
    let connection = Connection::open(dir.join(DB_FILE)).expect("open sqlite");
    let version: Vec<u8> = connection
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            params![store::META_SCHEMA_VERSION],
            |row| row.get(0),
        )
        .expect("schema version");
    assert_eq!(String::from_utf8_lossy(&version), "2");

    let block_columns: Vec<String> = connection
        .prepare("SELECT name FROM pragma_table_info('entries')")
        .expect("table info")
        .query_map([], |row| row.get(0))
        .expect("columns")
        .collect::<Result<_, _>>()
        .expect("collect");
    assert!(
        block_columns.contains(&"block_id".to_string()),
        "{block_columns:?}"
    );
    assert!(
        !block_columns.contains(&"block_hash".to_string()),
        "{block_columns:?}"
    );

    let indexes: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'entries_block'",
            [],
            |row| row.get(0),
        )
        .expect("index count");
    assert_eq!(indexes, 0, "version 2 drops the entries_block index");
    drop(connection);
    cleanup(&dir);
}

#[test]
fn index_keeps_digest_prefixes() {
    let dir = temp_dir("prefix");
    seed(&dir, &[("alpha", "first"), ("beta", "second")]);
    assert_eq!(
        column_len(&dir, "entries", "content_hash"),
        store::DIGEST_PREFIX as i64
    );

    let connection = Connection::open(dir.join(DB_FILE)).expect("open sqlite");
    let field_hashes: Vec<u8> = connection
        .query_row("SELECT field_hashes FROM entries LIMIT 1", [], |row| {
            row.get(0)
        })
        .expect("field hashes");
    let value = odc_core::cbor::decode(&field_hashes).expect("cbor");
    for (_, digest) in value.entries() {
        assert_eq!(
            digest.as_bytes().expect("bytes").len(),
            store::DIGEST_PREFIX,
            "field digests are truncated to the prefix length"
        );
    }
    drop(connection);

    let repo = Repo::open(&dir).expect("open");
    assert!(repo.verify().expect("verify").ok);
    cleanup(&dir);
}

/// Rewrite a fresh repository into the version 1 layout.
///
/// The content hashes come back from the commit index snapshot, which still
/// holds the full digest, so the downgraded file is a faithful v1 database for
/// the columns that matter to readers: `entries.block_hash` and 32-byte content
/// hashes.
fn downgrade_to_version_one(dir: &Path) {
    let digests: Vec<(String, [u8; 32])> = {
        let repo = Repo::open(dir).expect("open");
        let mut pairs = Vec::new();
        repo.for_each_entry(|entry| {
            pairs.push((entry.word.clone(), entry.content_hash()));
            Ok(())
        })
        .expect("entries");
        pairs
    };
    let connection = Connection::open(dir.join(DB_FILE)).expect("open sqlite");
    connection
        .execute_batch(
            "CREATE TABLE blocks_v1(
                 block_hash BLOB PRIMARY KEY,
                 codec INTEGER NOT NULL,
                 raw_size INTEGER NOT NULL,
                 data BLOB NOT NULL
             );
             INSERT INTO blocks_v1(block_hash, codec, raw_size, data)
                 SELECT block_hash, codec, raw_size, data FROM blocks;
             CREATE TABLE entries_v1(
                 word TEXT PRIMARY KEY,
                 display TEXT NOT NULL,
                 block_hash BLOB NOT NULL,
                 offset INTEGER NOT NULL,
                 length INTEGER NOT NULL,
                 content_hash BLOB NOT NULL,
                 field_hashes BLOB NOT NULL
             );
             INSERT INTO entries_v1(word, display, block_hash, offset, length, content_hash, field_hashes)
                 SELECT e.word, e.display, b.block_hash, e.offset, e.length, e.content_hash, e.field_hashes
                 FROM entries e JOIN blocks b ON b.id = e.block_id;
             DROP TABLE entries;
             ALTER TABLE entries_v1 RENAME TO entries;
             DROP TABLE blocks;
             ALTER TABLE blocks_v1 RENAME TO blocks;
             CREATE INDEX entries_block ON entries(block_hash);
             DELETE FROM meta WHERE key = 'schema_version';",
        )
        .expect("downgrade");
    let mut update = connection
        .prepare("UPDATE entries SET content_hash = ?2 WHERE word = ?1")
        .expect("prepare");
    for (word, digest) in digests {
        update
            .execute(params![word, digest.as_slice()])
            .expect("update");
    }
    drop(update);
    drop(connection);
}

#[test]
fn version_one_databases_stay_readable_and_upgrade_on_commit() {
    let dir = temp_dir("v1");
    let entries = seed(&dir, &[("alpha", "first"), ("beta", "second")]);
    downgrade_to_version_one(&dir);

    {
        let connection = Connection::open(dir.join(DB_FILE)).expect("open sqlite");
        let version: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM meta WHERE key = ?1",
                params![store::META_SCHEMA_VERSION],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(version, 0, "the fixture is a version 1 database");
    }

    // Reading a v1 repository must not touch the file.
    let before = std::fs::read(dir.join(DB_FILE)).expect("read db");
    let repo = Repo::open(&dir).expect("open");
    assert_eq!(repo.entry_count().expect("count"), 2);
    let shown = repo.show("alpha").expect("show").expect("entry");
    assert_eq!(shown.defs[0].sense, "first");
    assert_eq!(repo.search_prefix("al", 10).expect("search").len(), 1);
    let exported: Vec<Entry> = {
        let mut out = Vec::new();
        repo.for_each_entry(|entry| {
            out.push(entry);
            Ok(())
        })
        .expect("export");
        out
    };
    assert_eq!(exported.len(), 2);
    assert!(repo.verify().expect("verify").ok, "{:?}", repo.verify());
    drop(repo);
    assert_eq!(
        std::fs::read(dir.join(DB_FILE)).expect("read db"),
        before,
        "opening a v1 repository for reading leaves the file untouched"
    );

    // The first write upgrades the layout and records the new version.
    let mut repo = Repo::open(&dir).expect("open");
    let mut edited = entries[0].clone();
    edited.defs[0].sense = "first edited".to_string();
    repo.import(&[edited], false).expect("import");
    repo.commit("edit", "tester", 1_700_000_100)
        .expect("commit");
    assert!(repo.status().expect("status").is_clean());
    assert!(repo.verify().expect("verify").ok);
    drop(repo);

    let connection = Connection::open(dir.join(DB_FILE)).expect("open sqlite");
    let version: Vec<u8> = connection
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            params![store::META_SCHEMA_VERSION],
            |row| row.get(0),
        )
        .expect("schema version");
    assert_eq!(String::from_utf8_lossy(&version), "2");
    drop(connection);

    let repo = Repo::open(&dir).expect("open");
    assert_eq!(
        repo.show("alpha").expect("show").expect("entry").defs[0].sense,
        "first edited"
    );
    cleanup(&dir);
}
