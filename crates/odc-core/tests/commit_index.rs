//! Commit index encoding: full snapshots, deltas and chain rebuilds.

use odc_core::entry::{Def, Entry};
use odc_core::error::Error;
use odc_core::repo::Repo;
use odc_core::snapshot::EntryKind;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let dir = std::env::temp_dir().join(format!(
        "odc-commit-index-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn cleanup(dir: &std::path::Path) {
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

fn hex(hash: &[u8; 32]) -> String {
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn import_and_commit(repo: &mut Repo, entries: &[Entry], message: &str, ts: i64) -> String {
    repo.import(entries, true).expect("import");
    hex(&repo
        .commit(message, "tester", ts)
        .expect("commit")
        .commit
        .hash)
}

#[test]
fn first_commit_is_full_and_edits_are_deltas() {
    let dir = temp_dir("first");
    let mut repo = Repo::init(&dir).expect("init");
    let genesis = repo.head().expect("head").expect("genesis");
    let header = repo
        .commit_index_header(&genesis)
        .expect("header")
        .expect("entry");
    assert_eq!(header.kind, EntryKind::Full);
    assert_eq!(header.entry_count, 0);

    // Everything is new, so the first commit is a full snapshot as well.
    let mut entries: Vec<Entry> = (0..20).map(entry_for).collect();
    let first = import_and_commit(&mut repo, &entries, "first", 1_700_100_000);
    let first_hash = repo.resolve(&first).expect("resolve");
    let header = repo
        .commit_index_header(&first_hash)
        .expect("header")
        .expect("entry");
    assert_eq!(header.kind, EntryKind::Full);
    assert_eq!(header.depth, 0);
    assert_eq!(header.entry_count, 20);

    // Two changed entries out of twenty cost a delta.
    entries[3].defs[0].sense = "revised three".to_string();
    entries[7].defs[0].sense = "revised seven".to_string();
    let second = import_and_commit(&mut repo, &entries, "second", 1_700_100_001);
    let second_hash = repo.resolve(&second).expect("resolve");
    let header = repo
        .commit_index_header(&second_hash)
        .expect("header")
        .expect("entry");
    assert_eq!(header.kind, EntryKind::Delta);
    assert_eq!(header.depth, 1);
    assert_eq!(header.entry_count, 20);
    assert_eq!(header.body_count, 2);

    let storage = repo.index_storage().expect("storage");
    assert_eq!(storage.full, 2);
    assert_eq!(storage.delta, 1);
    assert!(repo.verify().expect("verify").ok);
    cleanup(&dir);
}

#[test]
fn delta_chain_rebuilds_every_commit() {
    let dir = temp_dir("chain");
    let mut repo = Repo::init(&dir).expect("init");
    let mut entries: Vec<Entry> = (0..12).map(entry_for).collect();
    let base = import_and_commit(&mut repo, &entries, "base", 1_700_200_000);
    let mut revisions = vec![base];

    for step in 0..5 {
        let index = step % entries.len();
        entries[index].defs[0].sense = format!("sense at step {step}");
        revisions.push(import_and_commit(
            &mut repo,
            &entries,
            &format!("step {step}"),
            1_700_200_001 + step as i64,
        ));
    }

    for window in revisions.windows(2) {
        let diff = repo.diff(&window[0], &window[1]).expect("diff");
        assert_eq!(diff.added.len(), 0);
        assert_eq!(diff.removed.len(), 0);
        assert_eq!(diff.modified.len(), 1);
    }
    assert!(repo.verify().expect("verify").ok);
    cleanup(&dir);
}

#[test]
fn a_full_anchor_appears_every_thirty_two_deltas() {
    let dir = temp_dir("anchor");
    let mut repo = Repo::init(&dir).expect("init");
    let mut entries: Vec<Entry> = (0..6).map(entry_for).collect();
    import_and_commit(&mut repo, &entries, "base", 1_700_300_000);
    let mut depth = 0u32;
    let mut anchors = 1usize;
    let mut deepest = 0u32;

    for step in 0..40 {
        entries[0].defs[0].sense = format!("sense at step {step}");
        let revision = import_and_commit(
            &mut repo,
            &entries,
            &format!("step {step}"),
            1_700_300_001 + step,
        );
        let hash = repo.resolve(&revision).expect("resolve");
        let header = repo
            .commit_index_header(&hash)
            .expect("header")
            .expect("entry");
        match header.kind {
            EntryKind::Full => {
                assert_eq!(header.depth, 0);
                depth = 0;
                anchors += 1;
            }
            EntryKind::Delta => {
                depth += 1;
                assert_eq!(header.depth, depth, "delta depth must follow the parent");
                deepest = deepest.max(depth);
            }
        }
        assert!(depth <= 31, "a chain may not hold more than 31 deltas");
    }
    assert!(anchors > 1, "the history must cross an anchor");
    assert_eq!(deepest, 31);
    assert!(repo.verify().expect("verify").ok);
    cleanup(&dir);
}

#[test]
fn removed_words_are_replayed_from_the_parent() {
    let dir = temp_dir("removed");
    let mut repo = Repo::init(&dir).expect("init");
    let entries: Vec<Entry> = (0..10).map(entry_for).collect();
    let base = import_and_commit(&mut repo, &entries, "base", 1_700_400_000);
    let kept: Vec<Entry> = entries[3..].to_vec();
    let pruned = import_and_commit(&mut repo, &kept, "prune", 1_700_400_001);

    let diff = repo.diff(&base, &pruned).expect("diff");
    assert_eq!(diff.removed.len(), 3);
    assert_eq!(diff.added.len(), 0);
    assert!(repo.show("word-00000").expect("show").is_none());
    assert!(repo.show("word-00009").expect("show").is_some());

    let mut entries = kept;
    entries[0].defs[0].sense = "kept and edited".to_string();
    let edited = import_and_commit(&mut repo, &entries, "edit", 1_700_400_002);
    let diff = repo.diff(&pruned, &edited).expect("diff");
    assert_eq!(diff.modified.len(), 1);
    assert!(repo.verify().expect("verify").ok);
    cleanup(&dir);
}

#[test]
fn a_missing_parent_index_is_reported_not_panicked() {
    let dir = temp_dir("broken");
    let mut repo = Repo::init(&dir).expect("init");
    let genesis = repo.head().expect("head").expect("genesis");
    let mut entries: Vec<Entry> = (0..8).map(entry_for).collect();
    let first = import_and_commit(&mut repo, &entries, "first", 1_700_500_000);
    entries[0].defs[0].sense = "after the break".to_string();
    let second = import_and_commit(&mut repo, &entries, "second", 1_700_500_001);

    let second_hash = repo.resolve(&second).expect("resolve");
    let header = repo
        .commit_index_header(&second_hash)
        .expect("header")
        .expect("entry");
    assert_eq!(header.kind, EntryKind::Delta);

    // Drop the parent index the delta points at.
    let connection = rusqlite::Connection::open(dir.join("odc.db")).expect("open");
    let parent = repo.resolve(&first).expect("resolve");
    connection
        .execute(
            "DELETE FROM commit_index WHERE commit_hash = ?1",
            rusqlite::params![parent.as_slice()],
        )
        .expect("delete");
    drop(connection);

    let error = repo
        .diff(&hex(&genesis), &second)
        .expect_err("the chain is broken");
    assert!(matches!(error, Error::Inconsistent(_)), "{error:?}");

    let report = repo.verify().expect("verify");
    assert!(!report.ok);
    assert!(
        report.errors.iter().any(|error| error.contains("ODC-E018")),
        "{:?}",
        report.errors
    );
    cleanup(&dir);
}

#[test]
fn shallow_pack_materialises_a_delta_head() {
    let dir = temp_dir("shallow-delta");
    let mut repo = Repo::init(&dir).expect("init");
    let mut entries: Vec<Entry> = (0..10).map(entry_for).collect();
    import_and_commit(&mut repo, &entries, "base", 1_700_800_000);
    entries[0].defs[0].sense = "edited after the base".to_string();
    let head = import_and_commit(&mut repo, &entries, "edit", 1_700_800_001);
    let head_hash = repo.resolve(&head).expect("resolve");
    assert_eq!(
        repo.commit_index_header(&head_hash)
            .expect("header")
            .expect("entry")
            .kind,
        EntryKind::Delta
    );

    let out = dir.join("head.odc");
    repo.pack(
        &out,
        odc_core::pack::PackOptions {
            shallow: true,
            force: false,
        },
    )
    .expect("pack");
    let packed = Repo::open(&out).expect("open");
    let packed_head = packed.head().expect("head").expect("HEAD");
    assert_eq!(
        packed
            .commit_index_header(&packed_head)
            .expect("header")
            .expect("entry")
            .kind,
        EntryKind::Full,
        "a single-commit export has no parent to replay"
    );
    assert!(packed.verify().expect("verify").ok);
    assert!(packed.show("word-00000").expect("show").is_some());
    cleanup(&dir);
}

#[test]
fn gc_pins_the_oldest_retained_entry() {
    let dir = temp_dir("gc-pin");
    let mut repo = Repo::init(&dir).expect("init");
    let mut entries: Vec<Entry> = (0..20).map(entry_for).collect();
    import_and_commit(&mut repo, &entries, "base", 1_700_900_000);
    for step in 0..3 {
        entries[step].defs[0].sense = format!("edited at step {step}");
        import_and_commit(
            &mut repo,
            &entries,
            &format!("step {step}"),
            1_700_900_001 + step as i64,
        );
    }
    let oldest_kept = repo.resolve("HEAD~1").expect("resolve");
    assert_eq!(
        repo.commit_index_header(&oldest_kept)
            .expect("header")
            .expect("entry")
            .kind,
        EntryKind::Delta
    );

    repo.gc(2, false, false).expect("gc");
    assert_eq!(
        repo.commit_index_header(&oldest_kept)
            .expect("header")
            .expect("entry")
            .kind,
        EntryKind::Full,
        "the boundary entry is pinned before its parent goes away"
    );
    assert!(repo.verify().expect("verify").ok);
    let diff = repo.diff("HEAD~1", "HEAD").expect("diff");
    assert_eq!(diff.modified.len(), 1);
    cleanup(&dir);
}
