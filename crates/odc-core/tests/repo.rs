use odc_core::entry::{Def, Entry};
use odc_core::error::Error;
use odc_core::hash;
use odc_core::repo::Repo;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let dir = std::env::temp_dir().join(format!("odc-test-{label}-{}-{nanos}", std::process::id()));
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

#[test]
fn init_creates_a_genesis_commit() {
    let dir = temp_dir("init");
    let repo = Repo::init(&dir).expect("init");
    let commits = repo.log(None).expect("log");
    assert_eq!(commits.len(), 1);
    let genesis = &commits[0];
    assert_eq!(genesis.entry_count, 0);
    assert_eq!(genesis.parent, hash::ZERO_HASH);
    assert_eq!(
        genesis.hash,
        hash::commit_hash(
            &hash::ZERO_HASH,
            &hash::ZERO_HASH,
            &hash::ZERO_HASH,
            &hash::ZERO_HASH,
            0
        )
    );
    assert!(repo.status().expect("status").is_clean());
    let report = repo.verify().expect("verify");
    assert!(report.ok, "{:?}", report.errors);
    assert_eq!(report.commits, 1);
    cleanup(&dir);
}

#[test]
fn double_init_is_rejected() {
    let dir = temp_dir("double-init");
    Repo::init(&dir).expect("init");
    assert!(matches!(
        Repo::init(&dir),
        Err(Error::AlreadyInitialized(_))
    ));
    cleanup(&dir);
}

#[test]
fn import_commit_show_diff_and_verify() {
    let dir = temp_dir("flow");
    let mut repo = Repo::init(&dir).expect("init");
    let entries: Vec<Entry> = (0..50).map(entry_for).collect();
    let report = repo.import(&entries, false).expect("import");
    assert_eq!(report.written, 50);
    assert_eq!(repo.status().expect("status").added, 50);
    let first = repo
        .commit("first", "tester", 1_700_000_000)
        .expect("commit");
    assert_eq!(first.entries, 50);
    assert!(repo.status().expect("status").is_clean());

    let shown = repo.show("WORD-00007").expect("show").expect("present");
    assert_eq!(shown.word, "word-00007");
    assert_eq!(shown.defs[0].sense, "definition number 7");

    // Modify one, add one, delete one.
    let mut changed = vec![entries[1].clone()];
    changed[0].defs[0].sense = "an updated definition".to_string();
    let mut added = entry_for(999);
    added.word = "zzz-new".to_string();
    let mut kept: Vec<Entry> = entries[2..].to_vec();
    kept.extend(changed);
    kept.push(added);
    let report = repo.import(&kept, true).expect("import prune");
    assert_eq!(report.removed, 1);
    let second = repo
        .commit("second", "tester", 1_700_000_100)
        .expect("commit");
    assert_eq!(second.entries, 50);

    let diff = repo.diff("HEAD~1", "HEAD").expect("diff");
    assert_eq!(diff.added, vec!["zzz-new".to_string()]);
    assert_eq!(diff.removed, vec!["word-00000".to_string()]);
    assert_eq!(diff.modified, vec!["word-00001".to_string()]);

    let history = repo.log(None).expect("log");
    assert_eq!(history.len(), 3);
    assert_eq!(history[0].message, "second");

    let report = repo.verify().expect("verify");
    assert!(report.ok, "{:?}", report.errors);
    assert_eq!(report.entries, 50);
    cleanup(&dir);
}

#[test]
fn commit_requires_changes() {
    let dir = temp_dir("nochanges");
    let mut repo = Repo::init(&dir).expect("init");
    assert!(matches!(
        repo.commit("empty", "tester", 0),
        Err(Error::NoChanges)
    ));
    repo.import(&[entry_for(1)], false).expect("import");
    repo.commit("one", "tester", 1).expect("commit");
    assert!(matches!(
        repo.commit("again", "tester", 2),
        Err(Error::NoChanges)
    ));
    cleanup(&dir);
}

#[test]
fn verify_detects_block_corruption() {
    let dir = temp_dir("tamper");
    let mut repo = Repo::init(&dir).expect("init");
    let entries: Vec<Entry> = (0..40).map(entry_for).collect();
    repo.import(&entries, false).expect("import");
    repo.commit("data", "tester", 10).expect("commit");
    drop(repo);

    let db = dir.join("odc.db");
    let connection = rusqlite::Connection::open(&db).expect("open");
    let raw_size: i64 = connection
        .query_row(
            "SELECT raw_size FROM blocks ORDER BY block_hash LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("raw_size");
    let replacement = zstd::bulk::compress(&vec![0u8; usize::try_from(raw_size).expect("size")], 3)
        .expect("compress");
    connection
        .execute(
            "UPDATE blocks SET data = ?1 WHERE block_hash = (SELECT block_hash FROM blocks ORDER BY block_hash LIMIT 1)",
            [replacement],
        )
        .expect("update");
    drop(connection);

    let repo = Repo::open(&dir).expect("reopen");
    let report = repo.verify().expect("verify");
    assert!(!report.ok);
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("does not match content")),
        "{:?}",
        report.errors
    );
    cleanup(&dir);
}

#[test]
fn ten_thousand_entry_end_to_end() {
    let dir = temp_dir("10k");
    let mut repo = Repo::init(&dir).expect("init");
    let entries: Vec<Entry> = (0..10_000).map(entry_for).collect();
    let report = repo.import(&entries, false).expect("import");
    assert_eq!(report.total, 10_000);
    let first = repo
        .commit("import 10k", "tester", 1_700_000_000)
        .expect("commit");
    assert_eq!(first.entries, 10_000);
    assert!(first.new_blocks >= 1);

    let chain = repo.log(None).expect("log");
    assert_eq!(chain.len(), 2);
    assert_eq!(chain[0].entry_count, 10_000);

    // modify one, add one, remove one
    let mut next: Vec<Entry> = entries[1..].to_vec();
    next[0].defs[0].sense = "changed".to_string();
    let mut extra = entry_for(12_345);
    extra.word = "zzzz-added".to_string();
    next.push(extra);
    let report = repo.import(&next, true).expect("import");
    assert_eq!(report.removed, 1);
    let second = repo
        .commit("edit", "tester", 1_700_000_100)
        .expect("commit");
    assert_eq!(second.entries, 10_000);

    let diff = repo.diff("HEAD~1", "HEAD").expect("diff");
    assert_eq!(diff.added.len(), 1);
    assert_eq!(diff.removed.len(), 1);
    assert_eq!(diff.modified.len(), 1);

    let report = repo.verify().expect("verify");
    assert!(report.ok, "{:?}", report.errors);
    assert_eq!(report.entries, 10_000);
    cleanup(&dir);
}
