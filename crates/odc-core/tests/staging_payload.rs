//! Working-set storage: committed entries keep only a pointer, uncommitted
//! entries keep their payload, and NULL payloads are materialised on demand.

use odc_core::entry::{Def, Entry};
use odc_core::error::Error;
use odc_core::repo::Repo;
use odc_core::store::DB_FILE;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let dir = std::env::temp_dir().join(format!(
        "odc-staging-test-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn cleanup(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

fn entry_for(index: usize) -> Entry {
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

fn staging_rows(dir: &Path) -> i64 {
    let connection = Connection::open(dir.join(DB_FILE)).expect("open sqlite");
    connection
        .query_row("SELECT COUNT(*) FROM staging", [], |row| row.get(0))
        .expect("count staging")
}

fn staging_payload_bytes(dir: &Path) -> i64 {
    let connection = Connection::open(dir.join(DB_FILE)).expect("open sqlite");
    connection
        .query_row(
            "SELECT COALESCE(SUM(length(payload)), 0) FROM staging",
            [],
            |row| row.get(0),
        )
        .expect("sum payload")
}

fn payload_not_null(dir: &Path) -> bool {
    let connection = Connection::open(dir.join(DB_FILE)).expect("open sqlite");
    let mut statement = connection
        .prepare("SELECT \"notnull\" FROM pragma_table_info('staging') WHERE name = 'payload'")
        .expect("prepare");
    statement
        .query_row([], |row| row.get::<_, i64>(0))
        .expect("notnull")
        == 1
}

fn seeded(label: &str, count: usize) -> (PathBuf, Vec<Entry>) {
    let dir = temp_dir(label);
    let mut repo = Repo::init(&dir).expect("init");
    let entries: Vec<Entry> = (0..count).map(entry_for).collect();
    repo.import(&entries, false).expect("import");
    repo.commit("seed", "tester", 1_700_000_000)
        .expect("commit");
    drop(repo);
    (dir, entries)
}

#[test]
fn commit_nulls_committed_payloads() {
    let dir = temp_dir("null");
    let mut repo = Repo::init(&dir).expect("init");
    let entries: Vec<Entry> = (0..50).map(entry_for).collect();
    repo.import(&entries, false).expect("import");
    assert!(staging_payload_bytes(&dir) > 0, "import caches payloads");
    repo.commit("first", "tester", 1_700_000_000)
        .expect("commit");
    assert_eq!(staging_payload_bytes(&dir), 0);
    let status = repo.status().expect("status");
    assert!(status.is_clean(), "{status:?}");
    assert_eq!(status.total, 50);
    let shown = repo.show("word-00007").expect("show").expect("entry");
    assert_eq!(shown.defs, entries[7].defs);
    drop(repo);
    cleanup(&dir);
}

#[test]
fn dirty_entries_keep_payload_until_commit() {
    let dir = temp_dir("dirty");
    let mut repo = Repo::init(&dir).expect("init");
    let entries: Vec<Entry> = (0..20).map(entry_for).collect();
    repo.import(&entries, false).expect("import");
    repo.commit("first", "tester", 1_700_000_000)
        .expect("commit");
    assert_eq!(staging_payload_bytes(&dir), 0);

    let mut changed = entries[3].clone();
    changed.defs[0].sense = "edited sense".to_string();
    repo.import(&[changed.clone()], false).expect("import");
    let status = repo.status().expect("status");
    assert_eq!(status.modified, 1, "{status:?}");
    assert_eq!(status.added, 0, "{status:?}");
    assert!(staging_payload_bytes(&dir) > 0);

    repo.commit("second", "tester", 1_700_000_100)
        .expect("commit");
    assert_eq!(staging_payload_bytes(&dir), 0);
    let shown = repo.show("word-00003").expect("show").expect("entry");
    assert_eq!(shown.defs[0].sense, "edited sense");
    drop(repo);
    cleanup(&dir);
}

#[test]
fn commit_drops_the_working_set() {
    let dir = temp_dir("drop");
    let mut repo = Repo::init(&dir).expect("init");
    assert!(repo.status().expect("status").is_clean());
    assert_eq!(staging_rows(&dir), 0);

    let entries: Vec<Entry> = (0..20).map(entry_for).collect();
    repo.import(&entries, false).expect("import");
    repo.commit("first", "tester", 1_700_000_000)
        .expect("commit");
    assert_eq!(staging_rows(&dir), 0, "commit must drop the working set");

    let status = repo.status().expect("status");
    assert!(status.is_clean(), "{status:?}");
    assert_eq!(status.total, 20, "{status:?}");

    // The next write rebuilds the working set from the committed index, so an
    // edit is still reported relative to HEAD.
    let mut changed = entries[7].clone();
    changed.defs[0].sense = "edited sense".to_string();
    repo.import(&[changed], false).expect("import");
    assert_eq!(staging_rows(&dir), 20);
    let status = repo.status().expect("status");
    assert_eq!(
        (status.added, status.modified, status.removed),
        (0, 1, 0),
        "{status:?}"
    );

    repo.commit("second", "tester", 1_700_000_100)
        .expect("commit");
    assert_eq!(staging_rows(&dir), 0);
    let shown = repo.show("word-00007").expect("show").expect("entry");
    assert_eq!(shown.defs[0].sense, "edited sense");
    drop(repo);
    cleanup(&dir);
}

#[test]
fn prune_after_commit_rebuilds_the_working_set() {
    let (dir, entries) = seeded("prune-rebuild", 10);
    let mut repo = Repo::open(&dir).expect("open");
    let kept: Vec<Entry> = entries[..4].to_vec();
    let report = repo.import(&kept, true).expect("import prune");
    assert_eq!(report.removed, 6, "{report:?}");
    let status = repo.status().expect("status");
    assert_eq!((status.added, status.modified, status.removed), (0, 0, 6));
    drop(repo);
    cleanup(&dir);
}

#[test]
fn working_entry_materialises_from_blocks() {
    let (dir, entries) = seeded("mat", 40);
    let repo = Repo::open(&dir).expect("open");
    assert_eq!(staging_payload_bytes(&dir), 0);

    let payload = repo
        .working_payload("word-00011")
        .expect("payload")
        .expect("present");
    assert_eq!(payload, entries[11].encode_payload());
    let entry = repo
        .working_entry("word-00011")
        .expect("entry")
        .expect("present");
    assert_eq!(entry.defs, entries[11].defs);
    assert!(repo.working_entry("missing").expect("missing").is_none());
    drop(repo);
    cleanup(&dir);
}

#[test]
fn forged_slice_is_rejected() {
    let (dir, _) = seeded("forged", 40);
    {
        let connection = Connection::open(dir.join(DB_FILE)).expect("open sqlite");
        connection
            .execute(
                "UPDATE entries SET offset = (SELECT offset FROM entries WHERE word = 'word-00001'),
                                     length = (SELECT length FROM entries WHERE word = 'word-00001'),
                                     block_id = (SELECT block_id FROM entries WHERE word = 'word-00001')
                 WHERE word = 'word-00000'",
                [],
            )
            .expect("forge slice");
    }
    let repo = Repo::open(&dir).expect("open");
    let error = repo.working_entry("word-00000").expect_err("must fail");
    assert!(matches!(error, Error::Inconsistent(_)), "{error:?}");
    drop(repo);
    cleanup(&dir);
}

#[test]
fn legacy_schema_is_migrated() {
    let dir = temp_dir("legacy");
    let mut repo = Repo::init(&dir).expect("init");
    let entries: Vec<Entry> = (0..10).map(entry_for).collect();
    repo.import(&entries, false).expect("import");
    drop(repo);

    {
        let connection = Connection::open(dir.join(DB_FILE)).expect("open sqlite");
        connection
            .execute_batch(
                "ALTER TABLE staging RENAME TO staging_legacy;
                 CREATE TABLE staging(
                     word TEXT PRIMARY KEY,
                     display TEXT NOT NULL,
                     payload BLOB NOT NULL,
                     content_hash BLOB NOT NULL
                 );
                 INSERT INTO staging(word, display, payload, content_hash)
                     SELECT word, display, payload, content_hash FROM staging_legacy;
                 DROP TABLE staging_legacy;",
            )
            .expect("rebuild legacy staging");
    }
    assert!(payload_not_null(&dir));

    let mut repo = Repo::open(&dir).expect("open migrates");
    assert!(!payload_not_null(&dir), "payload column accepts NULL");
    let status = repo.status().expect("status");
    assert_eq!(status.added, 10, "{status:?}");
    assert!(
        staging_payload_bytes(&dir) > 0,
        "uncommitted payloads survive"
    );
    assert_eq!(
        repo.working_payload("word-00004").expect("payload"),
        Some(entries[4].encode_payload())
    );

    repo.commit("migrated", "tester", 1_700_000_000)
        .expect("commit after migration");
    assert_eq!(staging_payload_bytes(&dir), 0);
    assert!(repo.status().expect("status").is_clean());
    let shown = repo.show("word-00004").expect("show").expect("entry");
    assert_eq!(shown.defs, entries[4].defs);
    drop(repo);
    cleanup(&dir);
}

#[test]
fn commit_keeps_staging_small() {
    let dir = temp_dir("volume");
    let mut repo = Repo::init(&dir).expect("init");
    let entries: Vec<Entry> = (0..100_000).map(entry_for).collect();
    let raw_bytes: usize = entries
        .iter()
        .map(|entry| entry.encode_payload().len())
        .sum();
    repo.import(&entries, false).expect("import");
    repo.commit("volume", "tester", 1_700_000_000)
        .expect("commit");
    let cached = staging_payload_bytes(&dir);
    assert_eq!(cached, 0);
    assert!(
        (cached as usize) * 20 < raw_bytes,
        "staging payloads {cached} should be under 5% of {raw_bytes} raw bytes"
    );
    drop(repo);
    cleanup(&dir);
}
