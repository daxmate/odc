//! Deepening a shallow export from the repository it was packed from.

use odc_core::entry::{Def, Entry};
use odc_core::error::Error;
use odc_core::fetch::deepen;
use odc_core::pack::PackOptions;
use odc_core::repo::Repo;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let dir =
        std::env::temp_dir().join(format!("odc-fetch-{}-{nanos}-{label}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn entry(index: usize, sense: &str) -> Entry {
    let mut item = Entry::new(format!("word-{index:06}"));
    item.defs.push(Def::new(sense));
    item
}

/// Five commits in total: the genesis written by `init` plus four imports.
fn seed(dir: &Path) -> Repo {
    let mut repo = Repo::init(dir).expect("init");
    for step in 0..4 {
        repo.import(&[entry(step, "sense")], false).expect("import");
        repo.commit(
            &format!("step {step}"),
            "tester",
            1_700_000_000 + i64::try_from(step).expect("step"),
        )
        .expect("commit");
    }
    repo
}

fn short_pack(repo: &Repo, out: &Path) {
    let report = repo
        .pack(
            out,
            PackOptions {
                shallow: true,
                force: false,
            },
        )
        .expect("pack");
    assert!(report.shallow);
    assert_eq!(report.commits, 1);
}

#[test]
fn deepen_copies_the_chain_down_to_the_requested_depth() {
    let work = temp_dir("depth");
    let source = seed(&work.join("source"));
    let out = work.join("head.odc");
    short_pack(&source, &out);

    let mut repo = Repo::open(&out).expect("open");
    assert!(repo.is_shallow().expect("shallow flag"));
    assert_eq!(repo.log(None).expect("log").len(), 1);

    let report = deepen(&mut repo, source.dir(), 3).expect("deepen");

    assert_eq!(report.depth_requested, 3);
    assert_eq!(report.commits_added, 2);
    assert_eq!(report.commit_index_added, 2);
    assert!(!report.complete);
    assert!(repo.is_shallow().expect("shallow flag"));

    let log = repo.log(None).expect("log");
    assert_eq!(log.len(), 3);
    let expected = source.log(None).expect("source log");
    assert_eq!(log[0].hash, expected[0].hash);
    assert_eq!(log[1], expected[1]);
    assert_eq!(log[2], expected[2]);
    assert!(repo.verify().expect("verify").ok);

    // The oldest copied commit lost its parent, so its snapshot had to be
    // rebuilt as a full entry; reading it back proves the rebuild happened.
    let diff = repo
        .diff("HEAD~2", "HEAD~1")
        .expect("diff across the boundary");
    assert_eq!(diff.added.len(), 1);
}

#[test]
fn deepen_past_genesis_completes_the_history() {
    let work = temp_dir("complete");
    let source = seed(&work.join("source"));
    let out = work.join("head.odc");
    short_pack(&source, &out);

    let mut repo = Repo::open(&out).expect("open");
    let head = repo.head().expect("head");
    deepen(&mut repo, source.dir(), 3).expect("deepen to 3");

    let report = deepen(&mut repo, source.dir(), 99).expect("deepen to genesis");

    assert_eq!(report.commits_added, 2);
    assert!(report.complete);
    // Clearing `meta.shallow` is what makes the repository read as complete.
    assert!(!repo.is_shallow().expect("shallow flag"));
    assert_eq!(repo.head().expect("head"), head);

    let log = repo.log(None).expect("log");
    assert_eq!(log.len(), 5);
    assert_eq!(log, source.log(None).expect("source log"));
    let verify = repo.verify().expect("verify");
    assert!(verify.ok, "verify errors: {:?}", verify.errors);
    assert_eq!(verify.commits, 5);
}

#[test]
fn deepen_is_a_no_op_at_depth_one() {
    let work = temp_dir("noop");
    let source = seed(&work.join("source"));
    let out = work.join("head.odc");
    short_pack(&source, &out);

    let mut repo = Repo::open(&out).expect("open");
    let report = deepen(&mut repo, source.dir(), 1).expect("deepen");

    assert_eq!(report.commits_added, 0);
    assert_eq!(report.commit_index_added, 0);
    assert!(!report.complete);
    assert_eq!(repo.log(None).expect("log").len(), 1);
}

#[test]
fn deepen_rejects_a_complete_repository() {
    let work = temp_dir("full");
    let source = seed(&work.join("source"));
    let mut repo = Repo::open(source.dir()).expect("open");

    let error = deepen(&mut repo, source.dir(), 3).expect_err("must refuse");
    assert!(matches!(error, Error::InvalidArgument(_)), "{error:?}");
}

#[test]
fn deepen_rejects_depth_zero() {
    let work = temp_dir("zero");
    let source = seed(&work.join("source"));
    let out = work.join("head.odc");
    short_pack(&source, &out);

    let mut repo = Repo::open(&out).expect("open");
    let error = deepen(&mut repo, source.dir(), 0).expect_err("must refuse");
    assert!(matches!(error, Error::InvalidArgument(_)), "{error:?}");
}

#[test]
fn deepen_stops_when_the_source_has_no_more_history() {
    let work = temp_dir("truncated");
    let source = seed(&work.join("source"));
    let out = work.join("head.odc");
    short_pack(&source, &out);
    let shallow_source = work.join("one-commit.odc");
    short_pack(&source, &shallow_source);

    let mut repo = Repo::open(&out).expect("open");
    let error = deepen(&mut repo, &shallow_source, 5).expect_err("must refuse");
    assert!(matches!(error, Error::Inconsistent(_)), "{error:?}");
    // A failed fetch leaves the destination exactly as it was.
    assert!(repo.is_shallow().expect("shallow flag"));
    assert_eq!(repo.log(None).expect("log").len(), 1);
}

#[test]
fn deepen_rejects_a_source_that_is_not_a_repository() {
    let work = temp_dir("not-a-repo");
    let source = seed(&work.join("source"));
    let out = work.join("head.odc");
    short_pack(&source, &out);

    let mut repo = Repo::open(&out).expect("open");
    let error = deepen(&mut repo, &work.join("missing"), 3).expect_err("must refuse");
    assert!(matches!(error, Error::NotARepository(_)), "{error:?}");
}
