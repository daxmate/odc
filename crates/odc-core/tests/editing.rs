//! Editing-session API: paging, writes, status, changes, diff and discard.

use odc_core::entry::{Def, Entry};
use odc_core::error::Error;
use odc_core::pack::PackOptions;
use odc_core::repo::{ChangeKind, DiffKind, EntryWriteOutcome, Repo, WorkingResolution};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Parallel tests can observe the same clock value; a counter keeps every
/// scratch directory unique within one test binary.
static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "odc-edit-{label}-{}-{nanos}-{sequence}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn cleanup(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

fn entry(key: &str, sense: &str) -> Entry {
    let mut entry = Entry::new(key);
    entry.word = key.to_string();
    entry.defs.push(Def::new(sense));
    entry
}

fn seeded() -> (PathBuf, Repo) {
    let dir = temp_dir("editing");
    let mut repo = Repo::init(&dir).expect("init");
    let entries = [
        entry("alpha", "one"),
        entry("beta", "two"),
        entry("gamma", "three"),
    ];
    repo.import(&entries, false).expect("import");
    repo.commit("seed", "tester", 1).expect("commit");
    (dir, repo)
}

fn keys(entries: &[Entry]) -> Vec<&str> {
    entries.iter().map(|entry| entry.word.as_str()).collect()
}

#[test]
fn entries_page_walks_keys_in_ascending_order() {
    let (dir, repo) = seeded();
    let first = repo.entries_page(None, 2).expect("page");
    assert_eq!(keys(&first), ["alpha", "beta"]);
    let cursor = first.last().map(|entry| entry.word.clone());
    let second = repo.entries_page(cursor.as_deref(), 2).expect("page");
    assert_eq!(keys(&second), ["gamma"]);
    assert_eq!(second[0].defs[0].sense, "three");
    assert!(repo
        .entries_page(Some("gamma"), 2)
        .expect("page")
        .is_empty());
    assert!(repo.entries_page(None, 0).expect("empty page").is_empty());
    cleanup(&dir);
}

#[test]
fn empty_workspace_stays_usable() {
    let dir = temp_dir("editing-empty");
    let mut repo = Repo::init(&dir).expect("init");
    assert!(repo.entries_page(None, 10).expect("page").is_empty());
    assert!(repo.working_changes(None, 10).expect("changes").is_empty());
    assert!(repo.status().expect("status").is_clean());
    assert_eq!(
        repo.delete_entry("missing").expect("delete"),
        EntryWriteOutcome::NotFound
    );
    assert_eq!(
        repo.upsert_entry(&entry("alpha", "one")).expect("upsert"),
        EntryWriteOutcome::Added
    );
    assert_eq!(repo.status().expect("status").added, 1);
    repo.discard_working().expect("discard");
    assert!(repo.status().expect("status").is_clean());
    cleanup(&dir);
}

#[test]
fn upsert_and_delete_track_status_and_changes() {
    let (dir, mut repo) = seeded();
    assert_eq!(
        repo.upsert_entry(&entry("Delta", "four")).expect("upsert"),
        EntryWriteOutcome::Added
    );
    assert_eq!(
        repo.upsert_entry(&entry("alpha", "one v2"))
            .expect("upsert"),
        EntryWriteOutcome::Modified
    );
    assert_eq!(
        repo.delete_entry("BETA").expect("delete"),
        EntryWriteOutcome::Deleted
    );
    assert_eq!(
        repo.delete_entry("nope").expect("delete"),
        EntryWriteOutcome::NotFound
    );

    let status = repo.status().expect("status");
    assert_eq!((status.added, status.modified, status.removed), (1, 1, 1));
    assert_eq!(status.total, 3);

    let changes = repo.working_changes(None, 10).expect("changes");
    assert_eq!(
        changes
            .iter()
            .map(|row| (row.key.as_str(), row.kind))
            .collect::<Vec<_>>(),
        [
            ("alpha", ChangeKind::Modified),
            ("beta", ChangeKind::Deleted),
            ("delta", ChangeKind::Added)
        ]
    );
    let page = repo.working_changes(None, 1).expect("page");
    assert_eq!(page.len(), 1);
    let rest = repo.working_changes(Some("alpha"), 10).expect("page");
    assert_eq!(rest.len(), 2);
    cleanup(&dir);
}

#[test]
fn entry_diff_reports_field_level_changes() {
    let (dir, mut repo) = seeded();
    let mut edited = repo.show("alpha").expect("show").expect("entry");
    edited.display = "Alpha".to_string();
    edited.defs[0].sense = "first".to_string();
    edited.defs[0].examples.push("An example.".to_string());
    edited.raw_html = Some("<p>Alpha</p>".to_string());
    repo.upsert_entry(&edited).expect("upsert");

    let diff = repo.entry_diff("alpha").expect("diff");
    assert_eq!(diff.kind, DiffKind::Modified);
    let sense = diff
        .fields
        .iter()
        .find(|field| field.field == "defs[0].sense")
        .expect("sense field");
    assert!(sense.changed);
    assert_eq!(
        sense.before,
        Some(odc_core::cbor::Value::Text("one".to_string()))
    );
    assert_eq!(
        sense.after,
        Some(odc_core::cbor::Value::Text("first".to_string()))
    );
    let unchanged = diff
        .fields
        .iter()
        .find(|field| field.field == "defs[0].pos")
        .expect("pos field");
    assert!(!unchanged.changed);

    repo.upsert_entry(&entry("delta", "four")).expect("upsert");
    let added = repo.entry_diff("delta").expect("diff");
    assert_eq!(added.kind, DiffKind::Added);
    assert!(added.fields.iter().all(|field| field.before.is_none()));
    assert!(added
        .fields
        .iter()
        .any(|field| field.field == "defs[0].sense" && field.changed));

    repo.delete_entry("gamma").expect("delete");
    let deleted = repo.entry_diff("gamma").expect("diff");
    assert_eq!(deleted.kind, DiffKind::Deleted);
    assert!(deleted.fields.iter().all(|field| field.after.is_none()));

    let absent = repo.entry_diff("missing").expect("diff");
    assert_eq!(absent.kind, DiffKind::Absent);
    assert!(absent
        .fields
        .iter()
        .all(|field| field.before.is_none() && field.after.is_none()));
    cleanup(&dir);
}

#[test]
fn discard_working_restores_head() {
    let (dir, mut repo) = seeded();
    repo.upsert_entry(&entry("delta", "four")).expect("upsert");
    repo.delete_entry("beta").expect("delete");
    repo.discard_working().expect("discard");
    assert!(repo.status().expect("status").is_clean());
    let page = repo.entries_page(None, 10).expect("page");
    assert_eq!(keys(&page), ["alpha", "beta", "gamma"]);
    repo.discard_working().expect("discard again");
    cleanup(&dir);
}

#[test]
fn commit_after_edits_then_no_changes() {
    let (dir, mut repo) = seeded();
    let mut edited = repo.show("alpha").expect("show").expect("entry");
    edited.defs[0].sense = "one v2".to_string();
    repo.upsert_entry(&edited).expect("upsert");
    let outcome = repo.commit("edit", "tester", 2).expect("commit");
    assert_eq!(outcome.entries, 3);
    let shown = repo.show("alpha").expect("show").expect("entry");
    assert_eq!(shown.defs[0].sense, "one v2");
    assert!(matches!(
        repo.commit("again", "tester", 3),
        Err(Error::NoChanges)
    ));
    cleanup(&dir);
}

#[test]
fn packed_file_is_read_only() {
    let work = temp_dir("editing-pack");
    let mut repo = Repo::init(work.join("repo")).expect("init");
    repo.import(&[entry("alpha", "one")], false)
        .expect("import");
    repo.commit("seed", "tester", 1).expect("commit");
    let pack = work.join("out.odc");
    repo.pack(
        &pack,
        PackOptions {
            shallow: false,
            force: false,
        },
    )
    .expect("pack");
    drop(repo);

    let mut packed = Repo::open(&pack).expect("open pack");
    assert!(packed.is_read_only());
    assert!(matches!(
        packed.upsert_entry(&entry("beta", "two")),
        Err(Error::ReadOnly(_))
    ));
    assert!(matches!(
        packed.delete_entry("alpha"),
        Err(Error::ReadOnly(_))
    ));
    assert!(matches!(packed.discard_working(), Err(Error::ReadOnly(_))));
    assert!(matches!(
        packed.commit("x", "tester", 2),
        Err(Error::ReadOnly(_))
    ));
    assert_eq!(packed.entries_page(None, 10).expect("page").len(), 1);
    cleanup(&work);
}

#[test]
fn resolve_prefer_working_follows_uncommitted_edits() {
    let (dir, mut repo) = seeded();

    // Unchanged: the committed entry is served.
    match repo.resolve_prefer_working("alpha").expect("resolve") {
        WorkingResolution::Found { entry, hops, .. } => {
            assert_eq!(hops, 0);
            assert_eq!(entry.defs[0].sense, "one");
        }
        other => panic!("unexpected: {other:?}"),
    }

    // Modified: the working copy wins over the committed body.
    repo.upsert_entry(&entry("alpha", "one v2"))
        .expect("upsert");
    match repo.resolve_prefer_working("alpha").expect("resolve") {
        WorkingResolution::Found { entry, hops, .. } => {
            assert_eq!(hops, 0);
            assert_eq!(entry.defs[0].sense, "one v2");
        }
        other => panic!("unexpected: {other:?}"),
    }

    // Added: a word only the working set carries.
    repo.upsert_entry(&entry("delta", "four")).expect("upsert");
    match repo.resolve_prefer_working("delta").expect("resolve") {
        WorkingResolution::Found { entry, hops, .. } => {
            assert_eq!(hops, 0);
            assert_eq!(entry.defs[0].sense, "four");
        }
        other => panic!("unexpected: {other:?}"),
    }

    // Deleted: HEAD still carries the word, so this is not NotFound.
    repo.delete_entry("beta").expect("delete");
    assert_eq!(
        repo.resolve_prefer_working("BETA").expect("resolve"),
        WorkingResolution::Deleted {
            requested: "beta".to_string()
        }
    );

    // Redirects resolve against the working set too.
    let mut redirect = entry("zulu", "");
    redirect.raw_html = Some("@@@LINK=alpha".to_string());
    repo.upsert_entry(&redirect).expect("upsert");
    match repo.resolve_prefer_working("zulu").expect("resolve") {
        WorkingResolution::Found {
            key, hops, entry, ..
        } => {
            assert_eq!(key, "alpha");
            assert_eq!(hops, 1);
            assert_eq!(entry.defs[0].sense, "one v2");
        }
        other => panic!("unexpected: {other:?}"),
    }

    cleanup(&dir);
}
