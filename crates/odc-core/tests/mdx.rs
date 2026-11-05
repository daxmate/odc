//! MDX adapter tests: synthetic fixtures, error codes and key allocation.

use odc_core::error::Error;
#[allow(deprecated)]
use odc_core::mdx::MAX_RECORD_BYTES;
use odc_core::mdx::{
    entry_from_record, entry_from_record_with_limit, validate_max_record_bytes, MdxKeys, MdxRecord,
    MdxSource, DEFAULT_MAX_RECORD_BYTES, HARD_MAX_RECORD_BYTES,
};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const ENTRIES: [(&str, &str); 4] = [
    ("alpha", "<p>alpha body</p>"),
    ("Bravo", "<p>bravo body</p>"),
    ("bravo", "<p>second bravo body</p>"),
    ("charlie", "<p>charlie body</p>"),
];

fn temp_path(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    std::env::temp_dir().join(format!(
        "odc-mdx-{}-{nanos}-{label}.mdx",
        std::process::id()
    ))
}

fn read_records(path: &Path) -> Vec<(String, String)> {
    let source = MdxSource::open(path).expect("open fixture");
    source
        .records()
        .collect::<Result<Vec<_>, _>>()
        .expect("read records")
        .into_iter()
        .map(|record| (record.key, record.html))
        .collect()
}

#[test]
fn reads_a_version_1_fixture() {
    let path = temp_path("v1");
    odc_testkit::write_v1_mdx(&path, &ENTRIES).expect("write fixture");
    let source = MdxSource::open(&path).expect("open fixture");
    let header = source.header();
    assert_eq!(header.engine_version, odc_testkit::ENGINE_VERSION_V1);
    assert_eq!(header.records, 4);
    let records = read_records(&path);
    let keys: Vec<&str> = records.iter().map(|(key, _)| key.as_str()).collect();
    // Version 1 stores keys sorted by their UTF-16LE bytes.
    assert_eq!(keys, ["Bravo", "alpha", "bravo", "charlie"]);
    assert_eq!(records[2].1, "<p>second bravo body</p>");
    std::fs::remove_file(&path).ok();
}

#[test]
fn reads_a_version_2_fixture() {
    let path = temp_path("v2");
    odc_testkit::write_v2_mdx(&path, &ENTRIES).expect("write fixture");
    let source = MdxSource::open(&path).expect("open fixture");
    let header = source.header();
    assert_eq!(header.engine_version, odc_testkit::ENGINE_VERSION_V2);
    assert_eq!(header.records, 4);
    let mut records = read_records(&path);
    records.sort();
    let mut expected: Vec<(String, String)> = ENTRIES
        .iter()
        .map(|(key, body)| ((*key).to_string(), (*body).to_string()))
        .collect();
    expected.sort();
    assert_eq!(records, expected);
    std::fs::remove_file(&path).ok();
}

#[test]
fn records_become_payload_only_entries() {
    let record = MdxRecord {
        key: "Bravo".to_string(),
        html: "<p>bravo body</p>".to_string(),
    };
    let entry = entry_from_record(&record).expect("entry");
    assert_eq!(entry.word, "bravo");
    assert_eq!(entry.display, "Bravo");
    assert_eq!(entry.raw_html.as_deref(), Some("<p>bravo body</p>"));
    assert!(entry.defs.is_empty());
    assert!(entry.phonetic.is_empty());
    assert!(entry.extra.is_empty());
}

#[test]
fn duplicate_keys_get_a_suffix_in_file_order() {
    let mut keys = MdxKeys::new();
    assert_eq!(keys.allocate("bravo"), ("bravo".to_string(), false));
    assert_eq!(keys.allocate("bravo"), ("bravo\u{1f}2".to_string(), true));
    assert_eq!(keys.allocate("bravo"), ("bravo\u{1f}3".to_string(), true));
    assert_eq!(keys.allocate("alpha"), ("alpha".to_string(), false));
    // A key that literally carries a suffix keeps its own slot.
    assert_eq!(
        keys.allocate("bravo\u{1f}2"),
        ("bravo\u{1f}2\u{1f}2".to_string(), true)
    );
}

#[test]
fn oversized_records_are_rejected_with_a_stable_code() {
    let record = MdxRecord {
        key: "huge".to_string(),
        html: "x".repeat(DEFAULT_MAX_RECORD_BYTES + 1),
    };
    let error = entry_from_record(&record).expect_err("oversized record");
    assert_eq!(error.code(), "ODC-E022");
    assert!(matches!(
        error,
        Error::EntryTooLarge { bytes, limit, .. }
            if bytes == DEFAULT_MAX_RECORD_BYTES + 1 && limit == DEFAULT_MAX_RECORD_BYTES
    ));
}

#[test]
fn a_record_exactly_at_the_default_limit_is_accepted() {
    let record = MdxRecord {
        key: "exact".to_string(),
        html: "x".repeat(DEFAULT_MAX_RECORD_BYTES),
    };
    let entry = entry_from_record(&record).expect("4 MiB fits exactly");
    assert_eq!(entry.raw_html.as_deref(), Some(record.html.as_str()));
}

#[test]
fn one_byte_over_the_limit_is_rejected() {
    let record = MdxRecord {
        key: "over".to_string(),
        html: "x".repeat(DEFAULT_MAX_RECORD_BYTES + 1),
    };
    let error = entry_from_record_with_limit(&record, DEFAULT_MAX_RECORD_BYTES)
        .expect_err("one byte over the limit");
    assert_eq!(error.code(), "ODC-E022");
}

#[test]
fn a_raised_limit_accepts_a_five_mib_record() {
    let record = MdxRecord {
        key: "big".to_string(),
        html: "x".repeat(5 * 1024 * 1024),
    };
    let error = entry_from_record(&record).expect_err("above the default limit");
    assert_eq!(error.code(), "ODC-E022");
    let entry =
        entry_from_record_with_limit(&record, 8 * 1024 * 1024).expect("an 8 MiB limit fits");
    assert_eq!(
        entry.raw_html.as_deref().map(str::len),
        Some(5 * 1024 * 1024)
    );
}

#[test]
fn record_limits_outside_the_supported_range_are_rejected() {
    assert!(validate_max_record_bytes(1).is_ok());
    assert!(validate_max_record_bytes(HARD_MAX_RECORD_BYTES).is_ok());
    let zero = validate_max_record_bytes(0).expect_err("zero is invalid");
    assert_eq!(zero.code(), "ODC-E016");
    let too_big = validate_max_record_bytes(HARD_MAX_RECORD_BYTES + 1).expect_err("over 16 MiB");
    assert_eq!(too_big.code(), "ODC-E016");
    assert!(
        too_big.to_string().contains("max_record_bytes"),
        "{too_big}"
    );
}

#[test]
#[allow(deprecated)]
fn the_legacy_constant_tracks_the_default() {
    assert_eq!(MAX_RECORD_BYTES, DEFAULT_MAX_RECORD_BYTES);
}

#[test]
fn empty_keys_are_rejected() {
    let record = MdxRecord {
        key: "   ".to_string(),
        html: "<p>no key</p>".to_string(),
    };
    let error = entry_from_record(&record).expect_err("empty key");
    assert_eq!(error.code(), "ODC-E016");
}

#[test]
fn truncated_files_report_an_mdx_error() {
    let path = temp_path("truncated");
    odc_testkit::write_v1_mdx(&path, &ENTRIES).expect("write fixture");
    let bytes = std::fs::read(&path).expect("read fixture");
    std::fs::write(&path, &bytes[..40]).expect("truncate");
    let error = MdxSource::open(&path).expect_err("truncated file must fail");
    assert_eq!(error.code(), "ODC-E021");
    std::fs::remove_file(&path).ok();
}

#[test]
fn corrupt_block_payloads_are_detected() {
    let path = temp_path("corrupt");
    odc_testkit::write_v1_mdx(&path, &ENTRIES).expect("write fixture");
    let mut bytes = std::fs::read(&path).expect("read fixture");
    let last = bytes.len() - 6;
    bytes[last] ^= 0xff;
    std::fs::write(&path, &bytes).expect("write corrupt fixture");
    let source = MdxSource::open(&path).expect("open still succeeds");
    let result: Result<Vec<_>, _> = source.records().collect();
    let error = result.expect_err("corrupt record block must fail");
    assert_eq!(error.code(), "ODC-E021");
    std::fs::remove_file(&path).ok();
}

#[test]
fn unsupported_engine_versions_are_rejected() {
    let path = temp_path("version");
    odc_testkit::write_v1_mdx(&path, &ENTRIES).expect("write fixture");
    let mut bytes = std::fs::read(&path).expect("read fixture");
    // The header is UTF-16LE; replace "1.2" with "3.0".
    let needle = [b'1', 0, b'.', 0, b'2', 0];
    let position = bytes
        .windows(needle.len())
        .position(|window| window == needle)
        .expect("version string in header");
    bytes[position] = b'3';
    std::fs::write(&path, &bytes).expect("rewrite fixture");
    let error = MdxSource::open(&path).expect_err("version 3 must fail");
    assert_eq!(error.code(), "ODC-E021");
    std::fs::remove_file(&path).ok();
}
