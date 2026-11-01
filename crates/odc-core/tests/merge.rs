//! Three-way merge: fast-forward, field-level combination, conflicts and strategies.

use odc_core::entry::{Def, Entry};
use odc_core::error::Error;
use odc_core::hash;
use odc_core::merge::{self, Choice, MergeKind, Strategy};
use odc_core::patch;
use odc_core::repo::Repo;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let dir =
        std::env::temp_dir().join(format!("odc-merge-{label}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn cleanup(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

fn entry(word: &str, sense: &str) -> Entry {
    let mut item = Entry::new(word);
    item.defs.push(Def::new(sense));
    item
}

fn phonetic(word: &str, value: &str) -> Entry {
    let mut item = entry(word, "meaning");
    item.phonetic = vec![value.to_string()];
    item
}

fn seed(dir: &Path, entries: &[Entry], message: &str, ts: i64) -> Repo {
    let mut repo = Repo::init(dir).expect("init");
    repo.import(entries, true).expect("import");
    repo.commit(message, "tester", ts).expect("commit");
    repo
}

fn head(repo: &Repo) -> [u8; 32] {
    repo.head().expect("head").expect("a commit")
}

fn checkpoint(dir: &Path) {
    let connection = Connection::open(dir.join("odc.db")).expect("db");
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .expect("checkpoint");
}

/// Copy a repository at its current HEAD (the merge base) into a second directory.
fn fork(source: &Path, target: &Path) {
    checkpoint(source);
    std::fs::create_dir_all(target).expect("fork dir");
    std::fs::copy(source.join("odc.db"), target.join("odc.db")).expect("copy db");
}

/// Copy commits, indexes and blocks from the source repository into the target.
fn import_commits(target: &Path, source: &Path) {
    checkpoint(source);
    let connection = Connection::open(target.join("odc.db")).expect("target db");
    connection
        .execute(
            "ATTACH DATABASE ?1 AS fork",
            params![source.join("odc.db").to_string_lossy().to_string()],
        )
        .expect("attach");
    for table in ["commits", "commit_index"] {
        connection
            .execute_batch(&format!(
                "INSERT OR IGNORE INTO main.{table} SELECT * FROM fork.{table};"
            ))
            .expect("copy objects");
    }
    // Block ids are local to a database: copy the content-addressed rows and
    // let the destination assign its own ids.
    connection
        .execute_batch(
            "INSERT OR IGNORE INTO main.blocks(block_hash, codec, raw_size, data)
             SELECT block_hash, codec, raw_size, data FROM fork.blocks;",
        )
        .expect("copy blocks");
    connection
        .execute_batch("DETACH DATABASE fork;")
        .expect("detach");
}

fn commit_entries(dir: &Path, entries: &[Entry], message: &str, ts: i64) -> [u8; 32] {
    let mut repo = Repo::open(dir).expect("open");
    repo.import(entries, true).expect("import");
    repo.commit(message, "tester", ts).expect("commit");
    head(&repo)
}

#[test]
fn merge_base_follows_chains_and_forks() {
    let work = temp_dir("base");
    let mut repo = seed(&work.join("repo"), &vector_base(), "base", 1_700_000_000);
    let base = head(&repo);
    repo.import(&[entry("alpha", "changed")], true)
        .expect("import");
    repo.commit("one", "tester", 1_700_000_010).expect("commit");
    let one = head(&repo);
    repo.import(&[entry("beta", "changed")], true)
        .expect("import");
    repo.commit("two", "tester", 1_700_000_020).expect("commit");
    let two = head(&repo);

    assert_eq!(
        merge::merge_base(&repo, &two, &one).expect("base"),
        Some(one)
    );
    assert_eq!(
        merge::merge_base(&repo, &two, &base).expect("base"),
        Some(base)
    );
    assert!(merge::is_ancestor(&repo, &base, &two).expect("ancestor"));
    assert!(!merge::is_ancestor(&repo, &two, &base).expect("ancestor"));
    cleanup(&work);
}

#[test]
fn fast_forward_moves_head_without_writing_a_merge_commit() {
    let work = temp_dir("ff");
    let base = vector_base();
    let repo_dir = work.join("repo");
    let repo = seed(&repo_dir, &base, "base", 1_700_000_000);
    let base_head = head(&repo);
    let fork_dir = work.join("fork");
    fork(&repo_dir, &fork_dir);
    drop(repo);
    let mut ahead = base.clone();
    ahead.push(entry("delta", "fourth"));
    let theirs_hash = commit_entries(&fork_dir, &ahead, "ahead", 1_700_000_020);
    import_commits(&repo_dir, &fork_dir);

    let mut repo = Repo::open(&repo_dir).expect("open");
    assert_eq!(head(&repo), base_head);
    let outcome = merge::merge(
        &mut repo,
        "HEAD",
        &hash::to_hex(&theirs_hash),
        Strategy::Auto,
    )
    .expect("fast-forward");
    assert_eq!(outcome.kind, MergeKind::FastForward);
    assert_eq!(head(&repo), theirs_hash);
    assert_eq!(repo.entry_count().expect("entries"), 4);
    assert!(repo.status().expect("status").is_clean());
    assert!(repo.verify().expect("verify").ok);
    cleanup(&work);
}

#[test]
fn disjoint_word_changes_merge_automatically() {
    let mut ours = vector_base();
    ours[0] = entry("alpha", "first ours");
    let mut theirs = vector_base();
    theirs.push(entry("delta", "fourth"));
    let (work, repo_dir, theirs_hash) = diverged("disjoint", ours, theirs);
    let mut repo = Repo::open(&repo_dir).expect("open");
    let ours_hash = head(&repo);

    let outcome = merge::merge(
        &mut repo,
        "HEAD",
        &hash::to_hex(&theirs_hash),
        Strategy::Auto,
    )
    .expect("merge");
    assert_eq!(outcome.kind, MergeKind::Merged);
    assert!(outcome.conflicts.is_empty());
    let commit = outcome.commit.as_ref().expect("commit");
    assert_eq!(commit.parent, ours_hash);
    assert_eq!(commit.parent2, theirs_hash);
    assert!(repo.status().expect("status").is_clean());
    assert!(repo.verify().expect("verify").ok);
    assert_eq!(
        repo.show("delta").expect("show").expect("entry").defs[0].sense,
        "fourth"
    );
    assert_eq!(
        repo.show("alpha").expect("show").expect("entry").defs[0].sense,
        "first ours"
    );

    // Merging the same commit again is a no-op.
    let again = merge::merge(
        &mut repo,
        "HEAD",
        &hash::to_hex(&theirs_hash),
        Strategy::Auto,
    )
    .expect("second merge");
    assert_eq!(again.kind, MergeKind::UpToDate);
    assert_eq!(head(&repo), commit.hash);
    cleanup(&work);
}

#[test]
fn field_level_merge_combines_disjoint_fields() {
    let mut base = vector_base();
    base[0] = phonetic("alpha", "al-fa");
    let mut ours = base.clone();
    ours[0].phonetic = vec!["al-fa-ours".to_string()];
    let mut theirs = base.clone();
    theirs[0] = entry("alpha", "theirs meaning");
    theirs[0].phonetic = vec!["al-fa".to_string()];

    let (work, repo_dir, theirs_hash) = diverged_from_base("fields", &base, ours, theirs);
    let mut repo = Repo::open(&repo_dir).expect("open");
    let ours_hash = head(&repo);
    let outcome = merge::merge(
        &mut repo,
        "HEAD",
        &hash::to_hex(&theirs_hash),
        Strategy::Auto,
    )
    .expect("merge");
    assert_eq!(outcome.kind, MergeKind::Merged);
    assert_eq!(outcome.auto_merged, 1);
    assert_eq!(outcome.commit.as_ref().expect("commit").parent, ours_hash);
    let merged = repo.show("alpha").expect("show").expect("entry");
    assert_eq!(merged.phonetic, vec!["al-fa-ours".to_string()]);
    assert_eq!(merged.defs[0].sense, "theirs meaning");
    assert!(repo.verify().expect("verify").ok);
    cleanup(&work);
}

#[test]
fn conflicts_block_the_commit_until_they_are_resolved() {
    let mut ours = vector_base();
    ours[0] = entry("alpha", "ours meaning");
    let mut theirs = vector_base();
    theirs[0] = entry("alpha", "theirs meaning");
    let (work, repo_dir, theirs_hash) = diverged("conflict", ours, theirs);
    let mut repo = Repo::open(&repo_dir).expect("open");
    let before = head(&repo);
    let entries_before = repo.entry_count().expect("entries");

    let outcome = merge::merge(
        &mut repo,
        "HEAD",
        &hash::to_hex(&theirs_hash),
        Strategy::Auto,
    )
    .expect("merge");
    assert_eq!(outcome.kind, MergeKind::Conflicts);
    assert!(outcome.commit.is_none());
    assert_eq!(head(&repo), before, "no commit while conflicts remain");
    assert_eq!(repo.entry_count().expect("entries"), entries_before);
    let conflicts = merge::conflicts(&repo).expect("conflicts");
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].word, "alpha");
    assert_eq!(conflicts[0].fields, vec!["defs".to_string()]);

    let error = merge::continue_merge(&mut repo, "tester", 1_700_000_100).expect_err("unresolved");
    assert_eq!(error.code(), "ODC-E029");

    // A second merge cannot start while one is unfinished.
    let error = merge::merge(
        &mut repo,
        "HEAD",
        &hash::to_hex(&theirs_hash),
        Strategy::Auto,
    )
    .expect_err("merge in progress");
    assert_eq!(error.code(), "ODC-E028");

    let resolved = merge::resolve(&mut repo, "alpha", Choice::Ours).expect("resolve");
    assert_eq!(resolved.fields, vec!["defs".to_string()]);
    let outcome = merge::continue_merge(&mut repo, "tester", 1_700_000_100).expect("continue");
    assert_eq!(outcome.kind, MergeKind::Merged);
    assert_eq!(
        outcome.commit.as_ref().expect("commit").parent2,
        theirs_hash
    );
    assert_eq!(
        repo.show("alpha").expect("show").expect("entry").defs[0].sense,
        "ours meaning"
    );
    assert!(merge::conflicts(&repo).expect("conflicts").is_empty());
    assert!(
        repo.status().expect("status").is_clean(),
        "merge --continue must leave the working set equal to HEAD"
    );
    assert!(repo.verify().expect("verify").ok);
    cleanup(&work);
}

#[test]
fn strategies_pick_a_side_and_union_keeps_both() {
    let mut ours = vector_base();
    ours[0] = entry("alpha", "ours meaning");
    let mut theirs = vector_base();
    theirs[0] = entry("alpha", "theirs meaning");

    let (work, repo_dir, theirs_hash) = diverged("strategy-theirs", ours.clone(), theirs.clone());
    let mut repo = Repo::open(&repo_dir).expect("open");
    let outcome = merge::merge(
        &mut repo,
        "HEAD",
        &hash::to_hex(&theirs_hash),
        Strategy::Theirs,
    )
    .expect("merge");
    assert_eq!(outcome.kind, MergeKind::Merged);
    assert_eq!(
        repo.show("alpha").expect("show").expect("entry").defs[0].sense,
        "theirs meaning"
    );
    cleanup(&work);

    let (work, repo_dir, theirs_hash) = diverged("strategy-union", ours, theirs);
    let mut repo = Repo::open(&repo_dir).expect("open");
    let outcome = merge::merge(
        &mut repo,
        "HEAD",
        &hash::to_hex(&theirs_hash),
        Strategy::Union,
    )
    .expect("merge");
    assert_eq!(outcome.kind, MergeKind::Merged);
    assert_eq!(
        repo.show("alpha").expect("show").expect("entry").defs[0].sense,
        "ours meaning"
    );
    assert_eq!(
        repo.show("alpha~2").expect("show").expect("entry").defs[0].sense,
        "theirs meaning"
    );
    assert!(repo.verify().expect("verify").ok);
    cleanup(&work);
}

#[test]
fn patch_apply_into_a_descendant_head_merges_three_way() {
    let work = temp_dir("patch-merge");
    let base = vector_base();
    let source = work.join("source");
    let repo = seed(&source, &base, "base", 1_700_000_000);
    let base_hash = head(&repo);
    drop(repo);
    let mirror = work.join("mirror");
    fork(&source, &mirror);

    let mut theirs = base.clone();
    theirs[1] = entry("beta", "beta theirs");
    let mut source_repo = Repo::open(&source).expect("open");
    source_repo.import(&theirs, true).expect("import");
    source_repo
        .commit("theirs", "tester", 1_700_000_020)
        .expect("commit");
    let target = head(&source_repo);
    let plan = patch::create(
        &source_repo,
        &hash::to_hex(&base_hash),
        &hash::to_hex(&target),
    )
    .expect("create");
    let bytes = patch::encode(&plan).expect("encode");

    let mut mirror_repo = Repo::open(&mirror).expect("open");
    let mut ours = base.clone();
    ours[0] = entry("alpha", "alpha ours");
    mirror_repo.import(&ours, true).expect("import");
    mirror_repo
        .commit("ours", "tester", 1_700_000_010)
        .expect("commit");
    let ours_head = head(&mirror_repo);

    match patch::apply(&mut mirror_repo, &bytes).expect("apply") {
        patch::ApplyOutcome::Merged { merge: outcome, .. } => {
            assert_eq!(outcome.kind, MergeKind::Merged);
            assert_eq!(outcome.commit.as_ref().expect("commit").parent, ours_head);
            assert_eq!(outcome.commit.as_ref().expect("commit").parent2, target);
        }
        other => panic!("expected a merge, got {other:?}"),
    }
    assert!(
        mirror_repo.status().expect("status").is_clean(),
        "merged patch apply must leave the working set equal to HEAD"
    );
    assert_eq!(
        mirror_repo
            .show("alpha")
            .expect("show")
            .expect("entry")
            .defs[0]
            .sense,
        "alpha ours"
    );
    assert_eq!(
        mirror_repo.show("beta").expect("show").expect("entry").defs[0].sense,
        "beta theirs"
    );
    assert!(mirror_repo.verify().expect("verify").ok);
    cleanup(&work);
}

#[test]
fn patch_apply_conflicts_leave_the_repository_untouched() {
    let work = temp_dir("patch-conflict");
    let base = vector_base();
    let source = work.join("source");
    let repo = seed(&source, &base, "base", 1_700_000_000);
    let base_hash = head(&repo);
    drop(repo);
    let mirror = work.join("mirror");
    fork(&source, &mirror);

    let mut theirs = base.clone();
    theirs[0] = entry("alpha", "alpha theirs");
    let mut source_repo = Repo::open(&source).expect("open");
    source_repo.import(&theirs, true).expect("import");
    source_repo
        .commit("theirs", "tester", 1_700_000_020)
        .expect("commit");
    let target = head(&source_repo);
    let plan = patch::create(
        &source_repo,
        &hash::to_hex(&base_hash),
        &hash::to_hex(&target),
    )
    .expect("create");
    let bytes = patch::encode(&plan).expect("encode");

    let mut mirror_repo = Repo::open(&mirror).expect("open");
    let mut ours = base.clone();
    ours[0] = entry("alpha", "alpha ours");
    mirror_repo.import(&ours, true).expect("import");
    mirror_repo
        .commit("ours", "tester", 1_700_000_010)
        .expect("commit");
    let head_before = head(&mirror_repo);
    let entries_before = mirror_repo.entry_count().expect("entries");

    match patch::apply(&mut mirror_repo, &bytes).expect("apply") {
        patch::ApplyOutcome::Merged { merge: outcome, .. } => {
            assert_eq!(outcome.kind, MergeKind::Conflicts);
            assert!(outcome.commit.is_none());
        }
        other => panic!("expected a merge, got {other:?}"),
    }
    assert_eq!(head(&mirror_repo), head_before, "HEAD stays put");
    assert_eq!(mirror_repo.entry_count().expect("entries"), entries_before);
    assert_eq!(merge::conflicts(&mirror_repo).expect("conflicts").len(), 1);
    merge::resolve(&mut mirror_repo, "alpha", Choice::Theirs).expect("resolve");
    let outcome =
        merge::continue_merge(&mut mirror_repo, "tester", 1_700_000_100).expect("continue");
    assert_eq!(outcome.kind, MergeKind::Merged);
    assert!(mirror_repo.verify().expect("verify").ok);
    cleanup(&work);
}

#[test]
fn patch_apply_leaves_a_clean_working_set_and_allows_merges() {
    let work = temp_dir("patch-clean");
    let base = vector_base();
    let source = work.join("source");
    let repo = seed(&source, &base, "base", 1_700_000_000);
    let base_hash = head(&repo);
    drop(repo);
    let base_copy = work.join("base-copy");
    fork(&source, &base_copy);
    let mirror = work.join("mirror");
    fork(&base_copy, &mirror);

    let mut theirs = base.clone();
    theirs[1] = entry("beta", "beta theirs");
    let mut source_repo = Repo::open(&source).expect("open");
    source_repo.import(&theirs, true).expect("import");
    source_repo
        .commit("theirs", "tester", 1_700_000_020)
        .expect("commit");
    let target = head(&source_repo);
    let plan = patch::create(
        &source_repo,
        &hash::to_hex(&base_hash),
        &hash::to_hex(&target),
    )
    .expect("create");
    let bytes = patch::encode(&plan).expect("encode");

    let mut mirror_repo = Repo::open(&mirror).expect("open");
    match patch::apply(&mut mirror_repo, &bytes).expect("apply") {
        patch::ApplyOutcome::Applied(outcome) => assert_eq!(outcome.target, target),
        other => panic!("expected an exact-base apply, got {other:?}"),
    }
    assert_eq!(head(&mirror_repo), target);
    assert!(
        mirror_repo.status().expect("status").is_clean(),
        "apply must reset the working set to the new HEAD"
    );

    // A divergent commit merges right away instead of failing on a dirty
    // working set (ODC-E016).
    let mut ours = base.clone();
    ours[0] = entry("alpha", "alpha copy");
    let other = commit_entries(&base_copy, &ours, "copy", 1_700_000_010);
    import_commits(&mirror, &base_copy);
    let outcome = merge::merge(
        &mut mirror_repo,
        "HEAD",
        &hash::to_hex(&other),
        Strategy::Auto,
    )
    .expect("merge after apply");
    assert_eq!(outcome.kind, MergeKind::Merged);
    assert!(mirror_repo.status().expect("status").is_clean());
    assert!(mirror_repo.verify().expect("verify").ok);
    cleanup(&work);
}

#[test]
fn errors_are_reported_with_merge_codes() {
    let work = temp_dir("errors");
    let mut repo = seed(&work.join("repo"), &vector_base(), "base", 1_700_000_000);
    let error = merge::continue_merge(&mut repo, "tester", 1).expect_err("nothing to continue");
    assert_eq!(error.code(), "ODC-E030");
    let error = merge::resolve(&mut repo, "alpha", Choice::Ours).expect_err("no conflict");
    assert_eq!(error.code(), "ODC-E030");
    let error = merge::merge(&mut repo, "HEAD", "xyz", Strategy::Auto).expect_err("bad revision");
    assert_eq!(error.code(), "ODC-E016");
    let _ = Error::NoMergeInProgress;
    cleanup(&work);
}

fn vector_base() -> Vec<Entry> {
    vec![
        entry("alpha", "first"),
        entry("beta", "second"),
        entry("gamma", "third"),
    ]
}

/// Base repository plus a fork that edits their side and a local commit on ours.
fn diverged(label: &str, ours: Vec<Entry>, theirs: Vec<Entry>) -> (PathBuf, PathBuf, [u8; 32]) {
    diverged_from_base(label, &vector_base(), ours, theirs)
}

fn diverged_from_base(
    label: &str,
    base: &[Entry],
    ours: Vec<Entry>,
    theirs: Vec<Entry>,
) -> (PathBuf, PathBuf, [u8; 32]) {
    let work = temp_dir(label);
    let repo_dir = work.join("repo");
    drop(seed(&repo_dir, base, "base", 1_700_000_000));
    let fork_dir = work.join("fork");
    fork(&repo_dir, &fork_dir);
    commit_entries(&repo_dir, &ours, "ours", 1_700_000_010);
    let theirs_hash = commit_entries(&fork_dir, &theirs, "theirs", 1_700_000_020);
    import_commits(&repo_dir, &fork_dir);
    (work, repo_dir, theirs_hash)
}
