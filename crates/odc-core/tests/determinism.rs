//! Determinism tests: the same logical content must always produce the same
//! block hashes and the same commit hash, regardless of import order.

use odc_core::entry::{Def, Entry};
use odc_core::hash;
use odc_core::repo::Repo;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let dir = std::env::temp_dir().join(format!(
        "odc-determinism-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn cleanup(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

fn entry(index: usize) -> Entry {
    let mut entry = Entry::new(format!("word-{index:05}"));
    entry.phonetic = vec![format!("/wɜːd {index}/")];
    entry.defs = vec![
        Def {
            pos: "noun".to_string(),
            sense: format!("sense number {index}"),
            examples: vec![format!("an example for {index}")],
            labels: vec!["test".to_string()],
        },
        Def {
            pos: "verb".to_string(),
            sense: "to test".to_string(),
            examples: Vec::new(),
            labels: Vec::new(),
        },
    ];
    entry
}

fn block_hashes(dir: &Path) -> Vec<Vec<u8>> {
    let connection = rusqlite::Connection::open(dir.join("odc.db")).expect("open sqlite database");
    let mut statement = connection
        .prepare("SELECT block_hash FROM blocks ORDER BY block_hash")
        .expect("prepare");
    let rows = statement
        .query_map([], |row| row.get::<_, Vec<u8>>(0))
        .expect("query");
    rows.map(|row| row.expect("row")).collect()
}

#[test]
fn import_order_does_not_change_block_hashes() {
    let entries: Vec<Entry> = (0..120).map(entry).collect();

    let forward_order = entries.clone();
    let mut reversed_order = entries.clone();
    reversed_order.reverse();
    let mut shuffled_order = entries.clone();
    shuffled_order.rotate_left(37);

    let dir_a = temp_dir("a");
    let dir_b = temp_dir("b");
    let dir_c = temp_dir("c");

    let mut repo_a = Repo::init(&dir_a).expect("init a");
    repo_a.import(&forward_order, false).expect("import a");
    repo_a
        .commit("batch", "tester", 1_700_000_000)
        .expect("commit a");

    let mut repo_b = Repo::init(&dir_b).expect("init b");
    repo_b.import(&reversed_order, false).expect("import b");
    repo_b
        .commit("batch", "tester", 1_700_000_000)
        .expect("commit b");

    let mut repo_c = Repo::init(&dir_c).expect("init c");
    repo_c.import(&shuffled_order, false).expect("import c");
    repo_c
        .commit("batch", "tester", 1_700_000_000)
        .expect("commit c");

    let hashes_a = block_hashes(&dir_a);
    let hashes_b = block_hashes(&dir_b);
    let hashes_c = block_hashes(&dir_c);
    assert!(!hashes_a.is_empty());
    assert_eq!(hashes_a, hashes_b);
    assert_eq!(hashes_a, hashes_c);

    let head_a = repo_a.head().expect("head a").expect("present");
    let head_b = repo_b.head().expect("head b").expect("present");
    let head_c = repo_c.head().expect("head c").expect("present");
    assert_eq!(hash::to_hex(&head_a), hash::to_hex(&head_b));
    assert_eq!(hash::to_hex(&head_a), hash::to_hex(&head_c));

    cleanup(&dir_a);
    cleanup(&dir_b);
    cleanup(&dir_c);
}

#[test]
fn reimporting_in_a_different_order_is_clean() {
    let dir = temp_dir("reimport");
    let mut repo = Repo::init(&dir).expect("init");
    let entries: Vec<Entry> = (0..40).map(entry).collect();
    repo.import(&entries, false).expect("import");
    repo.commit("initial", "tester", 1).expect("commit");

    let mut reversed = entries.clone();
    reversed.reverse();
    let report = repo.import(&reversed, false).expect("reimport");
    assert_eq!(report.total, entries.len());
    let status = repo.status().expect("status");
    assert!(status.is_clean(), "{status:?}");
    assert!(matches!(
        repo.commit("noop", "tester", 2),
        Err(odc_core::Error::NoChanges)
    ));

    cleanup(&dir);
}

#[test]
fn commit_hash_covers_content_not_time_or_message() {
    let dir_a = temp_dir("hash-a");
    let dir_b = temp_dir("hash-b");

    let entries: Vec<Entry> = (0..10).map(entry).collect();
    let mut repo_a = Repo::init(&dir_a).expect("init a");
    repo_a.import(&entries, false).expect("import a");
    let outcome_a = repo_a
        .commit("first message", "alice", 1_600_000_000)
        .expect("commit a");

    let mut repo_b = Repo::init(&dir_b).expect("init b");
    repo_b.import(&entries, false).expect("import b");
    let outcome_b = repo_b
        .commit("a different message", "bob", 1_900_000_000)
        .expect("commit b");

    assert_eq!(
        hash::to_hex(&outcome_a.commit.hash),
        hash::to_hex(&outcome_b.commit.hash)
    );
    assert_ne!(outcome_a.commit.author, outcome_b.commit.author);

    cleanup(&dir_a);
    cleanup(&dir_b);
}
