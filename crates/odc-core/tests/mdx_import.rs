//! MDX import pipeline tests: corrupt-block skipping and report bookkeeping.

use odc_core::mdx_scan::{scan, RecordSection, ScanOutcome};
use odc_core::profile::Profile;
use odc_core::{
    import_mdx, ImportProgress, ImportStage, MdxImportOptions, OversizedRecord, Repo,
    DEFAULT_MAX_RECORD_BYTES, HARD_MAX_RECORD_BYTES,
};
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
        let entries: Vec<(String, String)> = (0..count)
            .map(|index| {
                let key = format!("word{index:04}");
                let body = format!("<p>{index}</p>{}", "x".repeat(8 * 1024));
                (key, body)
            })
            .collect();
        Self::from_entries(label, entries)
    }

    fn from_entries(label: &str, entries: Vec<(String, String)>) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("odc-mdx-import-{label}-{nanos}"));
        std::fs::create_dir_all(&root).expect("temp dir");
        let mdx = root.join("fixture.mdx");
        let repo = root.join("repo");
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
        let html = self.stored_body(repo, index)?;
        assert_eq!(&html, expected, "payload of {key} survives intact");
        Some(html)
    }

    /// The body as stored, whatever the profile made of it.
    fn stored_body(&self, repo: &Repo, index: usize) -> Option<String> {
        let (key, _) = &self.entries[index];
        repo.working_entry(key)
            .expect("lookup")
            .map(|entry| entry.raw_html.clone().expect("raw html"))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn options() -> MdxImportOptions {
    // These tests cover corrupt-block recovery and record plumbing, not the
    // HTML formatter, and their fixtures are unbreakable 8 KiB text runs.
    MdxImportOptions {
        html: odc_core::html::HtmlOptions {
            mode: odc_core::html::HtmlMode::Off,
            ..odc_core::html::HtmlOptions::default()
        },
        ..MdxImportOptions::default()
    }
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
fn progress_reports_stages_and_totals() {
    let fixture = Fixture::new("progress", 300);
    let mut repo = fixture.open_repo();
    let marks = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = std::sync::Arc::clone(&marks);
    let report = import_mdx(
        &fixture.mdx,
        &mut repo,
        MdxImportOptions {
            progress: Some(Box::new(move |sample: ImportProgress| {
                sink.lock().expect("progress log").push(sample);
            })),
            ..MdxImportOptions::default()
        },
    )
    .expect("import");
    let samples = marks.lock().expect("progress log").clone();

    assert!(!samples.is_empty(), "progress callback fired");
    assert_eq!(
        samples.first().map(|sample| sample.stage),
        Some(ImportStage::Preparing),
        "the first sample announces the job before any record is read"
    );
    assert_eq!(samples.first().map(|sample| sample.done), Some(0));
    assert_eq!(
        samples.last().map(|sample| sample.stage),
        Some(ImportStage::Committing),
        "the last sample closes the import"
    );

    // done is stage-scoped, so monotonicity is checked per stage.
    let mut entries = 0_u64;
    let mut resources = 0_u64;
    let mut entries_total = None;
    for sample in &samples {
        match sample.stage {
            ImportStage::Entries => {
                assert!(sample.done >= entries, "entry progress moved backwards");
                assert!(sample.done <= sample.total, "done stayed within total");
                entries = sample.done;
                entries_total = Some(sample.total);
            }
            ImportStage::Resources => {
                assert!(
                    sample.done >= resources,
                    "resource progress moved backwards"
                );
                resources = sample.done;
            }
            ImportStage::Preparing | ImportStage::Committing => {}
        }
    }

    let last = samples.last().expect("final sample");
    assert_eq!(last.done, last.total, "the final sample is complete");
    assert_eq!(
        entries_total,
        Some(300),
        "entries total is the record count"
    );
    assert_eq!(entries, 300, "every record is accounted for");
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

fn write_limit_fixture(label: &str, rows: &[(&str, &str)]) -> (PathBuf, PathBuf) {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("odc-mdx-limit-{label}-{nanos}"));
    std::fs::create_dir_all(&root).expect("temp dir");
    let mdx = root.join("fixture.mdx");
    let repo = root.join("repo");
    odc_testkit::write_v2_mdx(&mdx, rows).expect("write fixture");
    (mdx, repo)
}

fn cleanup(mdx: &Path) {
    if let Some(root) = mdx.parent() {
        let _ = std::fs::remove_dir_all(root);
    }
}

#[test]
fn the_default_limit_accepts_four_mib_and_lists_the_record_above_it() {
    let exact = "e".repeat(DEFAULT_MAX_RECORD_BYTES);
    let over = "o".repeat(DEFAULT_MAX_RECORD_BYTES + 1);
    let rows = [
        ("alpha_exact", exact.as_str()),
        ("bravo_over", over.as_str()),
        ("charlie_small", "<p>ok</p>"),
    ];
    let (mdx, repo_dir) = write_limit_fixture("default", &rows);
    let mut repo = Repo::init(&repo_dir).expect("init repo");
    let report = import_mdx(&mdx, &mut repo, options()).expect("import");
    assert_eq!(report.imported_entries, 2, "the exact 4 MiB record imports");
    assert_eq!(report.skipped_oversized, 1);
    assert_eq!(
        report.skipped_oversized_samples,
        vec![OversizedRecord {
            key: "bravo_over".to_string(),
            bytes: DEFAULT_MAX_RECORD_BYTES + 1,
        }]
    );
    let entry = repo.working_entry("alpha_exact").expect("lookup");
    assert!(entry.is_some(), "the exact-limit record is stored");
    cleanup(&mdx);
}

#[test]
fn a_custom_limit_imports_a_five_mib_record() {
    let big = "b".repeat(5 * 1024 * 1024);
    let rows = [("big", big.as_str())];
    let (mdx, repo_dir) = write_limit_fixture("custom", &rows);
    let mut repo = Repo::init(&repo_dir).expect("init repo");
    let default_run = import_mdx(&mdx, &mut repo, options()).expect("import at the default");
    assert_eq!(default_run.imported_entries, 0);
    assert_eq!(default_run.skipped_oversized, 1);
    let raised = import_mdx(
        &mdx,
        &mut repo,
        MdxImportOptions {
            max_record_bytes: 8 * 1024 * 1024,
            ..options()
        },
    )
    .expect("import with a raised limit");
    assert_eq!(raised.imported_entries, 1);
    assert_eq!(raised.skipped_oversized, 0);
    assert!(raised.skipped_oversized_samples.is_empty());
    assert!(repo.working_entry("big").expect("lookup").is_some());
    cleanup(&mdx);
}

#[test]
fn oversized_details_are_capped_at_one_hundred_entries() {
    let bodies: Vec<(String, String)> = (0..105)
        .map(|index| (format!("word{index:04}"), "x".repeat(2 * 1024)))
        .collect();
    let rows: Vec<(&str, &str)> = bodies
        .iter()
        .map(|(key, body)| (key.as_str(), body.as_str()))
        .collect();
    let (mdx, repo_dir) = write_limit_fixture("cap", &rows);
    let mut repo = Repo::init(&repo_dir).expect("init repo");
    let report = import_mdx(
        &mdx,
        &mut repo,
        MdxImportOptions {
            max_record_bytes: 1024,
            ..options()
        },
    )
    .expect("import");
    assert_eq!(report.skipped_oversized, 105, "the count stays complete");
    assert_eq!(report.skipped_oversized_samples.len(), 100);
    assert_eq!(report.skipped_oversized_samples[0].key, "word0000");
    assert_eq!(report.skipped_oversized_samples[99].key, "word0099");
    cleanup(&mdx);
}

#[test]
fn out_of_range_record_limits_are_rejected() {
    let (mdx, repo_dir) = write_limit_fixture("range", &[("alpha", "<p>a</p>")]);
    let mut repo = Repo::init(&repo_dir).expect("init repo");
    let zero = import_mdx(
        &mdx,
        &mut repo,
        MdxImportOptions {
            max_record_bytes: 0,
            ..options()
        },
    )
    .expect_err("zero must be refused");
    assert_eq!(zero.code(), "ODC-E016");
    let too_big = import_mdx(
        &mdx,
        &mut repo,
        MdxImportOptions {
            max_record_bytes: HARD_MAX_RECORD_BYTES + 1,
            ..options()
        },
    )
    .expect_err("over 16 MiB must be refused");
    assert_eq!(too_big.code(), "ODC-E016");
    assert!(
        too_big.to_string().contains("max_record_bytes"),
        "{too_big}"
    );
    cleanup(&mdx);
}

// --- Profile-driven imports -------------------------------------------------

/// A record the LDOCE6 profile claims: `div > span.entry` is its P0 rule.
fn ldoce6_body(word: &str) -> String {
    format!(
        "<div><span class=\"entry\">{word}</span>\
         <span class=\"def\">{word} means something</span></div>"
    )
}

fn ldoce6_profile() -> Profile {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../profiles/ldoce6.profile.json");
    let text = std::fs::read_to_string(&path).expect("read profile");
    Profile::from_json(&text).expect("parse profile")
}

#[test]
fn a_profile_import_reports_the_contract() {
    let fixture = Fixture::from_entries(
        "profile",
        vec![
            ("alpha".to_string(), ldoce6_body("alpha")),
            ("bravo".to_string(), ldoce6_body("bravo")),
        ],
    );
    let mut repo = fixture.open_repo();
    let report = import_mdx(
        &fixture.mdx,
        &mut repo,
        MdxImportOptions {
            profile: Some(ldoce6_profile()),
            ..options()
        },
    )
    .expect("import");

    let contract = report.contract.expect("contract summary");
    assert!(contract.total > 0, "records measured: {contract:?}");
    assert!(contract.coverage() > 0.0, "coverage: {contract:?}");
    assert_eq!(report.p0_failed, 0, "P0 failures: {contract:?}");
    let body = fixture.stored_body(&repo, 0).expect("entry body");
    assert!(body.contains("odc-entry"), "L1 slots missing: {body}");
    assert!(body.contains("odc-def"), "L1 slots missing: {body}");
}

#[test]
fn an_unclaimed_record_falls_back_and_counts_as_unadapted() {
    let fixture = Fixture::from_entries(
        "profile-fallback",
        vec![
            ("alpha".to_string(), ldoce6_body("alpha")),
            (
                "plain".to_string(),
                "<p>plain text without the entry wrapper</p>".to_string(),
            ),
        ],
    );
    let mut repo = fixture.open_repo();
    let report = import_mdx(
        &fixture.mdx,
        &mut repo,
        MdxImportOptions {
            profile: Some(ldoce6_profile()),
            ..options()
        },
    )
    .expect("import");

    let contract = report.contract.expect("contract summary");
    assert_eq!(contract.total, 2, "{contract:?}");
    assert_eq!(contract.unadapted, 1, "{contract:?}");
    assert_eq!(report.p0_failed, 1);
    assert_eq!(report.p0_failed_samples, vec![String::from("plain")]);
    let body = fixture.stored_body(&repo, 1).expect("fallback body");
    assert!(
        body.contains("odc-entry--raw") || body.contains("data-odc-unadapted"),
        "raw fallback marker missing: {body}"
    );
}

#[test]
fn without_a_profile_the_import_is_unchanged() {
    let expected = ldoce6_body("alpha");
    let fixture =
        Fixture::from_entries("profile-off", vec![("alpha".to_string(), expected.clone())]);
    let mut repo = fixture.open_repo();
    let report = import_mdx(&fixture.mdx, &mut repo, options()).expect("import");

    assert!(report.contract.is_none(), "no profile, no contract");
    assert_eq!(report.p0_failed, 0);
    assert_eq!(
        fixture.body_of(&repo, 0).as_deref(),
        Some(expected.as_str()),
        "records are stored verbatim without a profile"
    );
}
