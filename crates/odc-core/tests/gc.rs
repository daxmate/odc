//! Garbage collection: reachability, dry runs, shared blocks and vacuum.

use odc_core::entry::Entry;
use odc_core::repo::Repo;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let dir = std::env::temp_dir().join(format!("odc-gc-{}-{nanos}-{label}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// Entry with a payload big enough to fill several blocks.
fn bulky(word: &str, filler: usize) -> Entry {
    let mut entry = Entry::new(word.to_string());
    entry.phonetic = vec!["x".repeat(filler)];
    entry
}

#[test]
fn gc_requires_a_positive_keep_last() {
    let dir = temp_dir("zero");
    let mut repo = Repo::init(&dir).expect("init");
    let error = repo.gc(0, true, false).expect_err("zero is rejected");
    assert!(error.to_string().contains("keep-last"));
}

#[test]
fn gc_dry_run_writes_nothing() {
    let dir = temp_dir("dry");
    let mut repo = Repo::init(&dir).expect("init");
    repo.import(&[bulky("alpha", 5000), bulky("beta", 5000)], true)
        .expect("import");
    repo.commit("first", "tester", 1_700_000_000)
        .expect("commit");
    repo.import(&[bulky("gamma", 5000)], true).expect("import");
    repo.commit("second", "tester", 1_700_000_020)
        .expect("commit");

    let before_blocks = repo.block_count().expect("blocks");
    let before_log = repo.log(None).expect("log").len();
    let report = repo.gc(1, true, false).expect("dry run");

    assert!(report.dry_run);
    assert!(!report.vacuumed);
    assert_eq!(report.kept_commits, 1);
    assert_eq!(report.removed_commits, 2);
    assert!(report.removed_indexes >= 2);
    assert!(report.freed_bytes() > 0);
    assert_eq!(repo.block_count().expect("blocks"), before_blocks);
    assert_eq!(repo.log(None).expect("log").len(), before_log);
    assert!(repo.verify().expect("verify").ok);
}

#[test]
fn gc_removes_unreachable_commits_and_blocks() {
    let dir = temp_dir("reach");
    let mut repo = Repo::init(&dir).expect("init");
    repo.import(&[bulky("alpha", 5000), bulky("beta", 5000)], true)
        .expect("import");
    repo.commit("first", "tester", 1_700_000_000)
        .expect("commit");
    repo.import(&[bulky("gamma", 5000)], true).expect("import");
    repo.commit("second", "tester", 1_700_000_020)
        .expect("commit");

    let report = repo.gc(1, false, false).expect("gc");
    assert_eq!(report.kept_commits, 1);
    assert_eq!(report.removed_commits, 2);
    assert!(report.removed_indexes >= 2);
    assert!(report.removed_blocks > 0, "orphan blocks must be dropped");
    assert!(report.freed_bytes() > 0);

    assert!(repo.verify().expect("verify").ok);
    assert_eq!(repo.log(None).expect("log").len(), 1);
    assert!(repo.show("alpha").expect("show").is_none());
    let gamma = repo.show("gamma").expect("show").expect("kept entry");
    assert_eq!(gamma.word, "gamma");
    assert!(repo.resolve("HEAD~1").is_err(), "old history is gone");

    // A second run has nothing left to collect.
    let again = repo.gc(1, false, false).expect("gc again");
    assert_eq!(again.removed_commits, 0);
    assert_eq!(again.removed_blocks, 0);
    assert_eq!(again.freed_bytes(), 0);
}

#[test]
fn gc_keeps_blocks_shared_by_retained_commits() {
    let dir = temp_dir("shared");
    let mut repo = Repo::init(&dir).expect("init");
    let mut entries: Vec<Entry> = (0..400)
        .map(|index| bulky(&format!("w{index:03}"), 4000))
        .collect();
    repo.import(&entries, true).expect("import");
    repo.commit("first", "tester", 1_700_000_000)
        .expect("commit");

    entries[399].phonetic = vec!["changed".to_string()];
    repo.import(&entries, true).expect("import");
    let second = repo
        .commit("second", "tester", 1_700_000_020)
        .expect("commit");
    assert!(second.reused_blocks > 0, "expected shared blocks");

    // genesis + first + second are the whole history.
    let report = repo.gc(3, false, false).expect("gc");
    assert_eq!(report.kept_commits, 3);
    assert_eq!(report.removed_commits, 0);
    assert_eq!(report.removed_indexes, 0);
    // Blocks referenced by HEAD survive even when the first commit created
    // them; only the stale tail block of the old revision may be collected.
    assert!(repo.verify().expect("verify").ok);
    assert_eq!(repo.log(None).expect("log").len(), 3);
    for word in ["w000", "w200", "w399"] {
        assert!(
            repo.show(word).expect("show").is_some(),
            "{word} unreadable"
        );
    }
}

#[test]
fn gc_vacuum_keeps_the_repository_valid() {
    let dir = temp_dir("vacuum");
    let mut repo = Repo::init(&dir).expect("init");
    repo.import(&[bulky("alpha", 8000)], true).expect("import");
    repo.commit("first", "tester", 1_700_000_000)
        .expect("commit");
    repo.import(&[bulky("beta", 8000)], true).expect("import");
    repo.commit("second", "tester", 1_700_000_020)
        .expect("commit");

    let report = repo.gc(1, false, true).expect("gc");
    assert!(report.vacuumed);
    assert!(repo.verify().expect("verify").ok);
    assert_eq!(repo.log(None).expect("log").len(), 1);
}
