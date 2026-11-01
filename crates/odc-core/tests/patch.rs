//! Patch transfer: roundtrip, size budget, block-set diff and rejections.

use odc_core::entry::{Def, Entry};
use odc_core::error::Error;
use odc_core::hash;
use odc_core::patch;
use odc_core::repo::Repo;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let dir =
        std::env::temp_dir().join(format!("odc-patch-{}-{nanos}-{label}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn entry(index: usize, sense: &str) -> Entry {
    let mut item = Entry::new(format!("word-{index:06}"));
    item.defs.push(Def::new(sense));
    item
}

fn dictionary(count: usize) -> Vec<Entry> {
    (0..count)
        .map(|index| entry(index, &format!("definition {index:06}")))
        .collect()
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

fn block_set(dir: &Path) -> BTreeSet<[u8; 32]> {
    let connection = rusqlite::Connection::open(dir.join("odc.db")).expect("db");
    let mut statement = connection
        .prepare("SELECT DISTINCT b.block_hash FROM entries e JOIN blocks b ON b.id = e.block_id")
        .expect("statement");
    let rows = statement
        .query_map([], |row| row.get::<_, Vec<u8>>(0))
        .expect("query");
    let mut set = BTreeSet::new();
    for row in rows {
        let blob = row.expect("row");
        set.insert(blob.try_into().expect("32-byte hash"));
    }
    set
}

fn state(repo: &Repo) -> ([u8; 32], usize, usize) {
    (
        head(repo),
        repo.entry_count().expect("entries"),
        repo.block_count().expect("blocks"),
    )
}

fn code(error: &Error) -> &'static str {
    error.code()
}

#[test]
fn roundtrip_reproduces_the_target_commit() {
    let work = temp_dir("roundtrip");
    let source = work.join("source");
    let initial = dictionary(400);
    let mut repo = seed(&source, &initial, "A", 1_700_000_000);
    let base_hash = head(&repo);
    let base_blocks = block_set(&source);

    let mut edited = initial.clone();
    edited[7].defs = vec![Def::new("changed seven")];
    edited.push(entry(9999, "brand new"));
    repo.import(&edited, true).expect("import edit");
    repo.commit("B", "tester", 1_700_000_010).expect("commit B");
    let target_hash = head(&repo);
    let target_blocks = block_set(&source);

    let plan = patch::create(&repo, "HEAD~1", "HEAD").expect("create");
    let bytes = patch::encode(&plan).expect("encode");
    assert_eq!(patch::decode(&bytes).expect("decode"), plan);

    let carried: BTreeSet<[u8; 32]> = plan
        .new_blocks
        .iter()
        .map(|raw| hash::blake3(raw))
        .collect();
    let expected_new: BTreeSet<[u8; 32]> =
        target_blocks.difference(&base_blocks).copied().collect();
    let expected_base: BTreeSet<[u8; 32]> =
        target_blocks.intersection(&base_blocks).copied().collect();
    assert_eq!(
        carried, expected_new,
        "only blocks new to the target travel"
    );
    assert_eq!(
        plan.base_blocks.iter().copied().collect::<BTreeSet<_>>(),
        expected_base
    );
    assert_eq!(
        plan.index_rows.len(),
        2,
        "one modified entry and one addition"
    );

    let mirror = work.join("mirror");
    let mut mirror_repo = seed(&mirror, &initial, "A", 1_700_000_100);
    assert_eq!(
        head(&mirror_repo),
        base_hash,
        "commit hash ignores ts/author"
    );

    let report = patch::verify(&mirror_repo, &bytes).expect("verify");
    assert_eq!(report.target, target_hash);
    assert!(!report.target_present);

    let outcome = patch::apply(&mut mirror_repo, &bytes).expect("apply");
    assert_eq!(outcome_applied(&outcome).target, target_hash);
    assert_eq!(head(&mirror_repo), target_hash);
    assert_eq!(mirror_repo.entry_count().expect("entries"), edited.len());
    let changed = mirror_repo
        .show("word-000007")
        .expect("show")
        .expect("entry");
    assert_eq!(changed.defs[0].sense, "changed seven");
    let added = mirror_repo
        .show("word-009999")
        .expect("show")
        .expect("entry");
    assert_eq!(added.defs[0].sense, "brand new");
    assert!(mirror_repo.verify().expect("verify repo").ok);
}

#[test]
fn hundred_edits_in_a_hundred_thousand_words_stay_under_fifty_kilobytes() {
    let work = temp_dir("size");
    let source = work.join("source");
    let initial = dictionary(100_000);
    let mut repo = seed(&source, &initial, "A", 1_700_000_000);

    let mut edited = initial.clone();
    for (index, item) in edited.iter_mut().enumerate().skip(50_000).take(100) {
        item.defs = vec![Def::new(format!("definition {index:06} revised"))];
    }
    repo.import(&edited, true).expect("import edit");
    repo.commit("B", "tester", 1_700_000_010).expect("commit B");

    let plan = patch::create(&repo, "HEAD~1", "HEAD").expect("create");
    let bytes = patch::encode(&plan).expect("encode");
    println!(
        "100k entries, 100 edits: {} bytes, {} new blocks",
        bytes.len(),
        plan.new_blocks.len()
    );
    assert!(
        bytes.len() < 50 * 1024,
        "patch is {} bytes, over the 50 KiB budget",
        bytes.len()
    );
    assert_eq!(plan.index_rows.len(), 100);
    assert!(
        plan.new_blocks.len() < 10,
        "edits are local to a few blocks"
    );
}

#[test]
fn rejects_wrong_base_merges_replays_and_corruption() {
    let work = temp_dir("reject");
    let source = work.join("source");
    let initial = dictionary(200);
    let mut repo = seed(&source, &initial, "A", 1_700_000_000);

    let mut edited = initial.clone();
    edited[3].defs = vec![Def::new("changed three")];
    repo.import(&edited, true).expect("import edit");
    repo.commit("B", "tester", 1_700_000_010).expect("commit B");
    let target_hash = head(&repo);
    let plan = patch::create(&repo, "HEAD~1", "HEAD").expect("create");
    let bytes = patch::encode(&plan).expect("encode");

    // Unrelated repository: unrelated history must be refused without writes.
    let stranger = work.join("stranger");
    let mut stranger_repo = seed(&stranger, &dictionary(50), "other", 1_700_000_020);
    let before = state(&stranger_repo);
    let error = patch::apply(&mut stranger_repo, &bytes).expect_err("wrong base");
    assert_eq!(code(&error), "ODC-E024");
    assert_eq!(state(&stranger_repo), before);

    // A descendant HEAD merges the target three-way instead of rejecting it.
    let merge_dir = work.join("merge");
    let mut merge_repo = seed(&merge_dir, &initial, "A", 1_700_000_030);
    let mut local = initial.clone();
    local[10].defs = vec![Def::new("local edit")];
    merge_repo.import(&local, true).expect("import");
    merge_repo
        .commit("local", "tester", 1_700_000_040)
        .expect("commit local");
    let local_head = head(&merge_repo);
    let outcome = patch::apply(&mut merge_repo, &bytes).expect("merged apply");
    match &outcome {
        patch::ApplyOutcome::Merged { merge, .. } => {
            assert_eq!(merge.kind, odc_core::merge::MergeKind::Merged);
            let commit = merge.commit.as_ref().expect("merge commit");
            assert_eq!(commit.parent, local_head);
            assert_eq!(commit.parent2, target_hash);
        }
        other => panic!("expected a merge, got {other:?}"),
    }
    assert_ne!(head(&merge_repo), local_head);
    assert_eq!(
        merge_repo
            .show("word-000010")
            .expect("show")
            .expect("entry")
            .defs[0]
            .sense,
        "local edit"
    );
    assert!(merge_repo.verify().expect("verify").ok);

    // Exact base: applies, then repeated application is refused.
    let mirror = work.join("mirror");
    let mut mirror_repo = seed(&mirror, &initial, "A", 1_700_000_050);
    patch::apply(&mut mirror_repo, &bytes).expect("apply");
    let before = state(&mirror_repo);
    let error = patch::apply(&mut mirror_repo, &bytes).expect_err("replay");
    assert_eq!(code(&error), "ODC-E026");
    assert_eq!(state(&mirror_repo), before);

    // Corrupted checksum.
    let mut corrupted = bytes.clone();
    let last = corrupted.len() - 1;
    corrupted[last] ^= 0x01;
    let error = patch::decode(&corrupted).expect_err("checksum");
    assert_eq!(code(&error), "ODC-E023");

    // Truncated file.
    let truncated = &bytes[..bytes.len() - 40];
    let error = patch::decode(truncated).expect_err("truncated");
    assert_eq!(code(&error), "ODC-E002");

    // Applying either damaged file to a pristine base leaves it untouched.
    let pristine = work.join("pristine");
    let mut pristine_repo = seed(&pristine, &initial, "A", 1_700_000_060);
    let before = state(&pristine_repo);
    let error = patch::apply(&mut pristine_repo, &corrupted).expect_err("corrupted apply");
    assert_eq!(code(&error), "ODC-E023");
    assert_eq!(state(&pristine_repo), before);
    let error = patch::apply(&mut pristine_repo, truncated).expect_err("truncated apply");
    assert_eq!(code(&error), "ODC-E002");
    assert_eq!(state(&pristine_repo), before);

    assert_eq!(head(&mirror_repo), target_hash);
}

fn outcome_applied(outcome: &patch::ApplyOutcome) -> &patch::PatchOutcome {
    match outcome {
        patch::ApplyOutcome::Applied(outcome) => outcome,
        patch::ApplyOutcome::Merged { patch, .. } => patch,
    }
}
