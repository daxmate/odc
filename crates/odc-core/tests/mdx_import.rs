//! MDX import pipeline tests: corrupt-block skipping and report bookkeeping.

use odc_core::mdx_scan::{scan, RecordSection, ScanOutcome};
use odc_core::{import_mdx, MdxImportOptions, Repo};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn section_of(path: &Path) -> RecordSection {
    match scan(path).expect("scan") {
        ScanOutcome::Scanned(section) => *section,
        other => panic!("expected a scanned record section, got {other:?}"),
    }
}

struct Fixture {
    root: PathBuf,
    mdx: PathBuf,
    repo: PathBuf,
    entries: Vec<(String, String)>,
}

impl Fixture {
    fn new(label: &str, count: usize) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("odc-mdx-import-{label}-{nanos}"));
        std::fs::create_dir_all(&root).expect("temp dir");
        let mdx = root.join("fixture.mdx");
        let repo = root.join("repo");
        let entries: Vec<(String, String)> = (0..count)
            .map(|index| {
                let key = format!("word{index:04}");
                let body = format!("<p>{index}</p>{}", "x".repeat(8 * 1024));
                (key, body)
            })
            .collect();
        let rows: Vec<(&str, &str)> = entries
            .iter()
            .map(|(key, body)| (key.as_str(), body.as_str()))
            .collect();
        odc_testkit::write_v2_mdx(&mdx, &rows).expect("write fixture");
        Self {
            root,
            mdx,
            repo,
            entries,
        }
    }

    fn open_repo(&self) -> Repo {
        Repo::init(&self.repo).expect("init repo")
    }

    fn corrupt_block(&self, ordinal: usize) {
        let section = section_of(&self.mdx);
        let block = &section.blocks[ordinal];
        let mut bytes = std::fs::read(&self.mdx).expect("read fixture");
        let start = usize::try_from(block.file_offset).expect("offset fits usize");
        let size = usize::try_from(block.compressed_size).expect("size fits usize");
        bytes[start + 8 + (size - 8) / 2] ^= 0xff;
        std::fs::write(&self.mdx, bytes).expect("write corrupt fixture");
    }

    fn body_of(&self, repo: &Repo, index: usize) -> Option<String> {
        let (key, expected) = &self.entries[index];
        repo.working_entry(key).expect("lookup").map(|entry| {
            let html = entry.raw_html.clone().expect("raw html");
            assert_eq!(&html, expected, "payload of {key} survives intact");
            html
        })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn options() -> MdxImportOptions {
    MdxImportOptions::default()
}

fn corrupt_fixture(label: &str, count: usize) -> Fixture {
    let fixture = Fixture::new(label, count);
    let blocks = section_of(&fixture.mdx).blocks.len();
    assert!(blocks > 2, "fixture must span several record blocks");
    fixture.corrupt_block(0);
    fixture
}

#[test]
fn imports_a_clean_fixture_without_repair() {
    let fixture = Fixture::new("clean", 40);
    let mut repo = fixture.open_repo();
    let report = import_mdx(&fixture.mdx, &mut repo, options()).expect("import");
    assert_eq!(report.skipped_blocks, 0);
    assert_eq!(report.skipped_entries, 0);
    assert!(report.corrupt_blocks.is_empty());
    assert_eq!(report.imported_entries, 40);
    assert_eq!(report.written, 40);
    assert_eq!(report.working_set, 40);
    assert_eq!(report.header.engine_version, "2.0");
    for index in [0, 7, 39] {
        assert!(fixture.body_of(&repo, index).is_some());
    }
}

#[test]
fn unscannable_record_sections_fall_back_to_the_original_file() {
    let fixture = Fixture::new("trailing", 40);
    // Extra bytes after the record section break the layout self-check, so the
    // scanner refuses to repair; the import must still read the file as-is.
    let mut bytes = std::fs::read(&fixture.mdx).expect("read fixture");
    bytes.extend_from_slice(&[0xab; 64]);
    std::fs::write(&fixture.mdx, bytes).expect("append trailing bytes");
    assert!(matches!(
        scan(&fixture.mdx).expect("scan"),
        ScanOutcome::Unscannable(_)
    ));

    let mut repo = fixture.open_repo();
    let report = import_mdx(&fixture.mdx, &mut repo, options()).expect("import must succeed");
    assert!(
        report.scan_warning.is_some(),
        "the fallback must be announced"
    );
    assert_eq!(report.skipped_blocks, 0);
    assert_eq!(report.skipped_entries, 0);
    assert!(report.corrupt_blocks.is_empty());
    assert_eq!(report.imported_entries, fixture.entries.len());
    assert_eq!(report.working_set, fixture.entries.len());
    assert!(fixture.body_of(&repo, 0).is_some());
}

#[test]
fn imports_a_corrupt_fixture_by_skipping_the_damaged_block() {
    let fixture = corrupt_fixture("skip", 40);
    let mut repo = fixture.open_repo();
    let report = import_mdx(&fixture.mdx, &mut repo, options()).expect("import");
    assert_eq!(report.skipped_blocks, 1);
    assert!(report.skipped_entries > 0, "the damaged block held entries");
    assert_eq!(report.corrupt_blocks.len(), 1);
    assert_eq!(report.corrupt_blocks[0].ordinal, 0);
    assert_eq!(
        report.imported_entries + report.skipped_entries,
        fixture.entries.len(),
        "every physical record is either imported or skipped"
    );
    assert_eq!(report.working_set, report.imported_entries);
    // The first block is damaged, so its entries are the ones that vanish.
    assert!(fixture.body_of(&repo, 0).is_none());
    let survivors = fixture.entries.len() - report.skipped_entries;
    assert!(survivors > 0);
    for index in (fixture.entries.len() - survivors)..fixture.entries.len() {
        assert!(fixture.body_of(&repo, index).is_some());
    }
}

#[test]
fn strict_mode_still_reports_the_mdx_error() {
    let fixture = corrupt_fixture("strict", 40);
    let mut repo = fixture.open_repo();
    let error = import_mdx(
        &fixture.mdx,
        &mut repo,
        MdxImportOptions {
            skip_corrupt_blocks: false,
            ..MdxImportOptions::default()
        },
    )
    .expect_err("strict import must fail");
    assert_eq!(error.code(), "ODC-E021");
}

#[test]
fn version_1_imports_ignore_the_repair_path() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("odc-mdx-import-v1-{nanos}"));
    std::fs::create_dir_all(&root).expect("temp dir");
    let mdx = root.join("v1.mdx");
    odc_testkit::write_v1_mdx(
        &mdx,
        &[("alpha", "<p>alpha</p>"), ("bravo", "<p>bravo</p>")],
    )
    .expect("write fixture");
    let mut repo = Repo::init(root.join("repo")).expect("init repo");
    let report = import_mdx(&mdx, &mut repo, options()).expect("import");
    assert_eq!(report.skipped_blocks, 0);
    assert_eq!(report.imported_entries, 2);
    assert_eq!(report.header.engine_version, "1.2");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn progress_reports_written_entries() {
    let fixture = Fixture::new("progress", 300);
    let mut repo = fixture.open_repo();
    let marks = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = std::sync::Arc::clone(&marks);
    let report = import_mdx(
        &fixture.mdx,
        &mut repo,
        MdxImportOptions {
            progress: Some(Box::new(move |written| {
                sink.lock().expect("progress log").push(written);
            })),
            ..MdxImportOptions::default()
        },
    )
    .expect("import");
    let marks = marks.lock().expect("progress log");
    assert!(!marks.is_empty(), "progress callback fired");
    assert_eq!(marks.last().copied(), Some(report.written));
    assert_eq!(report.imported_entries, 300);
}

#[test]
fn limit_stops_after_the_requested_entries() {
    let fixture = Fixture::new("limit", 20);
    let mut repo = fixture.open_repo();
    let report = import_mdx(
        &fixture.mdx,
        &mut repo,
        MdxImportOptions {
            limit: Some(5),
            ..MdxImportOptions::default()
        },
    )
    .expect("import");
    assert_eq!(report.imported_entries, 5);
    assert_eq!(report.working_set, 5);
}

#[test]
fn prune_removes_entries_missing_from_the_file() {
    let fixture = Fixture::new("prune", 10);
    let mut repo = fixture.open_repo();
    let first = import_mdx(&fixture.mdx, &mut repo, options()).expect("import");
    assert_eq!(first.working_set, 10);
    let partial = fixture.mdx.with_file_name("partial.mdx");
    let rows: Vec<(&str, &str)> = fixture.entries[..5]
        .iter()
        .map(|(key, body)| (key.as_str(), body.as_str()))
        .collect();
    odc_testkit::write_v2_mdx(&partial, &rows).expect("write partial");
    let report = import_mdx(
        &partial,
        &mut repo,
        MdxImportOptions {
            prune: true,
            ..MdxImportOptions::default()
        },
    )
    .expect("import");
    assert_eq!(report.removed, 5);
    assert_eq!(report.working_set, 5);
}
