//! Directory-shaped MDX import and pack conversion tests.

use odc_core::pack::PackOptions;
use odc_core::{copy_pack_to_repo, import_mdx_to_repo, Entry, Error, MdxImportOptions, Repo};
use odc_testkit::{write_mdd, write_v2_mdx};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const IMAGE: &[u8] = b"\x89PNG\r\n\x1a\nimage-bytes";

struct Fixture {
    root: PathBuf,
    mdx: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("odc-repo-{label}-{nanos}"));
        std::fs::create_dir_all(&root).expect("temp dir");
        let mdx = root.join("dictionary.mdx");
        write_v2_mdx(
            &mdx,
            &[
                ("apple", "<img src=\"pic/apple.png\">an apple"),
                ("banana", "a banana"),
            ],
        )
        .expect("mdx fixture");
        write_mdd(&root.join("dictionary.mdd"), &[("\\pic\\apple.png", IMAGE)])
            .expect("mdd fixture");
        Self { root, mdx }
    }

    fn repo_dir(&self) -> PathBuf {
        self.root.join("library.odc")
    }

    fn import(&self) -> odc_core::MdxRepoImportReport {
        import_mdx_to_repo(&self.mdx, &self.repo_dir(), MdxImportOptions::default())
            .expect("import into repo")
    }
}

fn assert_hex_commit(commit: &str) {
    assert_eq!(commit.len(), 64, "commit hash must be 32 bytes of hex");
    assert!(
        commit.chars().all(|c| c.is_ascii_hexdigit()),
        "commit hash must be hex"
    );
}

#[test]
fn import_creates_an_editable_repository() {
    let fixture = Fixture::new("editable");
    let report = fixture.import();
    assert_eq!(report.report.imported_entries, 2);
    assert_eq!(report.repo_path, fixture.repo_dir());
    assert_hex_commit(&report.commit);

    let repo = Repo::open(fixture.repo_dir()).expect("open imported repo");
    assert!(!repo.is_read_only(), "an imported library must be editable");
    assert!(repo.head().expect("head").is_some(), "HEAD must exist");
    assert!(repo.entry_count().expect("entry count") >= 2);
    assert_eq!(repo.resource_count().expect("resource count"), 1);
    let hits = repo.search_prefix("app", 10).expect("search");
    assert_eq!(hits.first().map(|(word, _)| word.as_str()), Some("apple"));
    drop(repo);

    // Editing must work on the copy: a new entry is stageable and committable.
    let mut repo = Repo::open(fixture.repo_dir()).expect("reopen");
    repo.upsert_entry(&Entry::new("zebra")).expect("upsert");
    let outcome = repo
        .commit("add zebra", odc_core::DEFAULT_AUTHOR, 0)
        .expect("commit edit");
    assert!(!outcome.commit.hash.is_empty());
}

#[test]
fn import_refuses_a_non_empty_directory() {
    let fixture = Fixture::new("non-empty");
    let repo_dir = fixture.repo_dir();
    std::fs::create_dir_all(&repo_dir).expect("dir");
    let marker = repo_dir.join("keep.txt");
    std::fs::write(&marker, b"do not touch").expect("marker");

    let error = import_mdx_to_repo(&fixture.mdx, &repo_dir, MdxImportOptions::default())
        .expect_err("non-empty directory must be refused");
    assert!(
        matches!(error, Error::AlreadyInitialized(_)),
        "got {error:?}"
    );
    assert_eq!(
        std::fs::read(&marker).expect("marker survives"),
        b"do not touch".to_vec()
    );
    assert!(!repo_dir.join("odc.db").exists());
}

#[test]
fn a_failed_import_leaves_no_repository_behind() {
    let fixture = Fixture::new("failed");
    let repo_dir = fixture.repo_dir();
    let missing = fixture.root.join("missing.mdx");

    import_mdx_to_repo(&missing, &repo_dir, MdxImportOptions::default())
        .expect_err("missing source must fail");
    assert!(
        !repo_dir.exists(),
        "the directory created for a failed import must be removed"
    );
}

#[test]
fn a_copied_pack_becomes_editable() {
    let fixture = Fixture::new("pack");
    fixture.import();
    let repo = Repo::open(fixture.repo_dir()).expect("open");
    let pack = fixture.root.join("export.odc");
    repo.pack(&pack, PackOptions::default()).expect("pack");
    let before = std::fs::read(&pack).expect("read pack");
    let entries = repo.entry_count().expect("count");
    drop(repo);

    let copy_dir = fixture.root.join("copy.odc");
    let converted = copy_pack_to_repo(&pack, &copy_dir).expect("convert pack");
    assert_eq!(converted.entries, entries);
    assert_hex_commit(&converted.commit);

    let mut copy = Repo::open(&copy_dir).expect("open copy");
    assert!(!copy.is_read_only(), "a copied pack must be editable");
    copy.upsert_entry(&Entry::new("zebra"))
        .expect("upsert on copy");
    copy.commit("add zebra", odc_core::DEFAULT_AUTHOR, 0)
        .expect("commit on copy");
    drop(copy);

    assert_eq!(
        std::fs::read(&pack).expect("re-read pack"),
        before,
        "the original pack must not change"
    );
    let original = Repo::open(&pack).expect("reopen pack");
    assert!(original.is_read_only(), "the pack stays read-only");
}

#[test]
fn copy_requires_a_pack_file() {
    let fixture = Fixture::new("not-a-pack");
    let error = copy_pack_to_repo(&fixture.root.join("absent.odc"), &fixture.root.join("out"))
        .expect_err("absent pack must fail");
    assert!(matches!(error, Error::NotARepository(_)), "got {error:?}");
}

fn _assert_types(_: &Path) {}
