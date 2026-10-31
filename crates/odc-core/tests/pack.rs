//! Single-file pack export: reachability, shallow metadata and overwrite rules.

use odc_core::entry::{Def, Entry};
use odc_core::error::Error;
use odc_core::pack::PackOptions;
use odc_core::repo::Repo;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let dir = std::env::temp_dir().join(format!("odc-pack-{}-{nanos}-{label}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn entry(index: usize, sense: &str) -> Entry {
    let mut item = Entry::new(format!("word-{index:06}"));
    item.defs.push(Def::new(sense));
    item
}

fn seed(dir: &Path) -> Repo {
    let mut repo = Repo::init(dir).expect("init");
    let first: Vec<Entry> = (0..3).map(|index| entry(index, "first")).collect();
    repo.import(&first, false).expect("import");
    repo.commit("first", "tester", 1_700_000_000)
        .expect("commit");
    let second: Vec<Entry> = (0..5).map(|index| entry(index, "second")).collect();
    repo.import(&second, false).expect("import");
    repo.commit("second", "tester", 1_700_000_100)
        .expect("commit");
    repo
}

fn bulky_entry(index: usize, generation: &str) -> Entry {
    let mut item = Entry::new(format!("bulk-{index:06}"));
    item.defs.push(Def::new(format!(
        "{generation}-{index}-{}",
        "padding".repeat(40)
    )));
    item
}

fn full_options() -> PackOptions {
    PackOptions {
        shallow: false,
        force: false,
    }
}

#[test]
fn shallow_pack_contains_only_head_and_marks_the_file() {
    let work = temp_dir("shallow");
    let repo = seed(&work.join("repo"));
    let out = work.join("head.odc");
    let report = repo
        .pack(
            &out,
            PackOptions {
                shallow: true,
                force: false,
            },
        )
        .expect("pack");
    assert!(report.shallow);
    assert_eq!(report.commits, 1);
    assert_eq!(report.entries, 5);
    assert!(out.is_file());
    assert!(!out.with_extension("odc-wal").exists());

    let packed = Repo::open(&out).expect("open packed file");
    assert!(packed.is_shallow().expect("shallow flag"));
    let history = packed.log(None).expect("log");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].message, "second");
    assert_eq!(packed.entry_count().expect("entries"), 5);
    assert_eq!(packed.verify().expect("verify").errors.len(), 0);
}

#[test]
fn full_pack_keeps_reachable_history() {
    let work = temp_dir("full");
    let repo = seed(&work.join("repo"));
    let out = work.join("full.odc");
    let report = repo.pack(&out, full_options()).expect("pack");
    assert!(!report.shallow);
    assert_eq!(report.commits, 3, "genesis + two commits");

    let packed = Repo::open(&out).expect("open packed file");
    assert!(!packed.is_shallow().expect("shallow flag"));
    let history = packed.log(None).expect("log");
    assert_eq!(history.len(), 3);
    assert_eq!(history[0].message, "second");
    assert_eq!(history[1].message, "first");
    assert_eq!(history[2].message, "genesis");
    assert_eq!(packed.verify().expect("verify").errors.len(), 0);
}

#[test]
fn full_pack_of_a_shallow_file_stays_shallow() {
    let work = temp_dir("nested");
    let repo = seed(&work.join("repo"));
    let shallow = work.join("head.odc");
    repo.pack(
        &shallow,
        PackOptions {
            shallow: true,
            force: false,
        },
    )
    .expect("shallow pack");

    let packed = Repo::open(&shallow).expect("open shallow");
    let again = work.join("again.odc");
    let report = packed.pack(&again, full_options()).expect("full pack");
    assert!(report.shallow, "missing ancestry cannot be restored");
    assert_eq!(report.commits, 1);

    let reopened = Repo::open(&again).expect("open again");
    assert_eq!(reopened.log(None).expect("log").len(), 1);
    assert_eq!(reopened.verify().expect("verify").errors.len(), 0);
}

#[test]
fn full_pack_keeps_old_blocks_shallow_pack_drops_them() {
    let work = temp_dir("blocks");
    let mut repo = Repo::init(work.join("repo")).expect("init");
    let first: Vec<Entry> = (0..400).map(|index| bulky_entry(index, "first")).collect();
    repo.import(&first, false).expect("import");
    repo.commit("first", "tester", 1_700_000_000)
        .expect("commit");
    let second: Vec<Entry> = (0..400).map(|index| bulky_entry(index, "second")).collect();
    repo.import(&second, false).expect("import");
    repo.commit("second", "tester", 1_700_000_100)
        .expect("commit");

    let full = work.join("full.odc");
    let shallow = work.join("shallow.odc");
    repo.pack(&full, full_options()).expect("full pack");
    repo.pack(
        &shallow,
        PackOptions {
            shallow: true,
            force: false,
        },
    )
    .expect("shallow pack");

    let full_repo = Repo::open(&full).expect("open full");
    let shallow_repo = Repo::open(&shallow).expect("open shallow");
    assert!(full_repo.block_count().expect("full blocks") > 1);
    assert!(
        full_repo.block_count().expect("full blocks")
            > shallow_repo.block_count().expect("shallow blocks"),
        "full keeps blocks from older commits"
    );
    assert_eq!(full_repo.verify().expect("verify full").errors.len(), 0);
    assert_eq!(
        shallow_repo.verify().expect("verify shallow").errors.len(),
        0
    );
}

#[test]
fn pack_refuses_an_existing_output_unless_forced() {
    let work = temp_dir("force");
    let repo = seed(&work.join("repo"));
    let out = work.join("exists.odc");
    std::fs::write(&out, b"keep me").expect("placeholder");

    let error = repo.pack(&out, full_options()).expect_err("must refuse");
    assert!(matches!(error, Error::InvalidArgument(_)));
    assert_eq!(std::fs::read(&out).expect("read"), b"keep me");

    let report = repo
        .pack(
            &out,
            PackOptions {
                shallow: false,
                force: true,
            },
        )
        .expect("forced pack");
    assert_eq!(report.commits, 3);
    assert!(std::fs::metadata(&out).expect("stat").len() > 8);
}
