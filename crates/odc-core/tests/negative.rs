//! Negative and robustness tests: malformed input must produce explicit error
//! codes instead of panics, and repository verification must detect corruption
//! in blocks, entry rows and the commit index.

use odc_core::block::{self, Record};
use odc_core::cbor;
use odc_core::entry::{Def, Entry};
use odc_core::error::Error;
use odc_core::repo::Repo;
use odc_core::varint;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let dir = std::env::temp_dir().join(format!(
        "odc-negative-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn cleanup(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

fn entry(index: usize) -> Entry {
    let mut entry = Entry::new(format!("word-{index:05}"));
    entry.defs = vec![Def::new(format!("definition {index}"))];
    entry
}

fn record_bytes(key: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    varint::encode(key.len() as u64, &mut out);
    out.extend_from_slice(key);
    varint::encode(payload.len() as u64, &mut out);
    out.extend_from_slice(payload);
    out
}

fn raw_block(records: &[(&str, &str)], offsets: &[u64]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&block::MAGIC);
    out.push(if offsets.is_empty() {
        0
    } else {
        block::FLAG_OFFSET_INDEX
    });
    varint::encode(records.len() as u64, &mut out);
    for (key, payload) in records {
        out.extend_from_slice(&record_bytes(key.as_bytes(), payload.as_bytes()));
    }
    for offset in offsets {
        varint::encode(*offset, &mut out);
    }
    out
}

#[test]
fn truncated_blocks_never_panic() {
    let records = vec![
        Record::new(b"alpha".to_vec(), b"first payload".to_vec()),
        Record::new(b"beta".to_vec(), b"second payload".to_vec()),
    ];
    let raw = block::encode(&records).expect("encode");
    for cut in 0..raw.len() {
        let result = block::decode(&raw[..cut]);
        assert!(
            result.is_err(),
            "prefix of length {cut} decoded successfully"
        );
    }
    assert_eq!(
        block::decode(&[]).unwrap_err(),
        Error::Truncated("block header")
    );
}

#[test]
fn bad_magic_is_reported() {
    let raw = block::encode(&[Record::new(b"alpha".to_vec(), b"p".to_vec())]).expect("encode");
    let mut corrupted = raw.clone();
    corrupted[0] = b'X';
    assert!(matches!(block::decode(&corrupted), Err(Error::BadMagic(_))));

    let mut reserved = raw;
    reserved[4] |= 0x80;
    assert!(matches!(block::decode(&reserved), Err(Error::BadMagic(_))));
}

#[test]
fn offset_index_mismatch_is_reported() {
    let raw = raw_block(&[("alpha", "one"), ("beta", "two")], &[5, 5_000]);
    match block::decode(&raw) {
        Err(Error::Inconsistent(message)) => {
            assert!(
                message.contains("offset index"),
                "unexpected message: {message}"
            );
        }
        other => panic!("expected Inconsistent, got {other:?}"),
    }

    // An offset far beyond the buffer must also be rejected, not panic.
    let raw = raw_block(&[("alpha", "one")], &[u64::MAX]);
    assert!(matches!(block::decode(&raw), Err(Error::Inconsistent(_))));
}

#[test]
fn unsorted_and_duplicate_keys_are_rejected() {
    let unsorted = vec![
        Record::new(b"beta".to_vec(), b"".to_vec()),
        Record::new(b"alpha".to_vec(), b"".to_vec()),
    ];
    assert_eq!(block::encode(&unsorted), Err(Error::UnsortedRecords));

    let duplicate = vec![
        Record::new(b"alpha".to_vec(), b"one".to_vec()),
        Record::new(b"alpha".to_vec(), b"two".to_vec()),
    ];
    assert_eq!(
        block::encode(&duplicate),
        Err(Error::DuplicateKey("alpha".to_string()))
    );

    let empty_key = vec![Record::new(Vec::new(), b"".to_vec())];
    assert!(matches!(
        block::encode(&empty_key),
        Err(Error::InvalidArgument(_))
    ));

    let raw = raw_block(&[("alpha", "one"), ("alpha", "two")], &[]);
    assert!(matches!(block::decode(&raw), Err(Error::DuplicateKey(_))));
}

#[test]
fn overlong_varints_are_rejected() {
    // A stream that never terminates is truncated, not silently accepted.
    assert_eq!(varint::decode(&[0xff; 10]), Err(Error::MalformedVarint));
    assert!(matches!(
        varint::decode(&[0x81; 10]),
        Err(Error::Truncated("varint") | Error::MalformedVarint)
    ));
    // More than ten bytes can never be a minimal u64 encoding.
    assert_eq!(varint::decode(&[0x80; 11]), Err(Error::MalformedVarint));
    // Non-minimal encodings are rejected.
    assert_eq!(
        varint::decode(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x00]),
        Err(Error::MalformedVarint)
    );
    assert_eq!(varint::decode(&[0x80, 0x00]), Err(Error::MalformedVarint));
}

#[test]
fn duplicate_cbor_keys_are_rejected() {
    // map(2) { "a": 1, "a": 2 }
    let bytes = [0xa2, 0x61, b'a', 0x01, 0x61, b'a', 0x02];
    assert!(
        cbor::decode(&bytes).is_err(),
        "duplicate map keys must be rejected"
    );
    assert!(cbor::validate_canonical(&bytes).is_err());
}

#[test]
fn verify_reports_entry_index_corruption() {
    let dir = temp_dir("entry-index");
    let mut repo = Repo::init(&dir).expect("init");
    repo.import(&[entry(1), entry(2), entry(3)], false)
        .expect("import");
    repo.commit("initial", "tester", 1).expect("commit");
    drop(repo);

    {
        let connection = rusqlite::Connection::open(dir.join("odc.db")).expect("open db");
        connection
            .execute(
                "UPDATE entries SET length = length + 100000 WHERE word = 'word-00001'",
                [],
            )
            .expect("corrupt entry row");
    }

    let repo = Repo::open(&dir).expect("open");
    let report = repo.verify().expect("verify must not fail hard");
    assert!(!report.ok);
    assert!(!report.errors.is_empty());
    cleanup(&dir);
}

#[test]
fn verify_reports_commit_index_corruption() {
    let dir = temp_dir("commit-index");
    let mut repo = Repo::init(&dir).expect("init");
    repo.import(&[entry(1)], false).expect("import");
    repo.commit("initial", "tester", 1).expect("commit");
    drop(repo);

    {
        let connection = rusqlite::Connection::open(dir.join("odc.db")).expect("open db");
        let updated = connection
            .execute(
                "UPDATE commit_index SET data = X'00' WHERE rowid = (SELECT MAX(rowid) FROM commit_index)",
                [],
            )
            .expect("corrupt commit index");
        assert_eq!(updated, 1);
    }

    let repo = Repo::open(&dir).expect("open");
    let report = repo.verify().expect("verify must not fail hard");
    assert!(!report.ok);
    assert!(!report.errors.is_empty());
    cleanup(&dir);
}

#[test]
fn verify_reports_truncated_block_payload() {
    let dir = temp_dir("block-payload");
    let mut repo = Repo::init(&dir).expect("init");
    repo.import(&[entry(1), entry(2)], false).expect("import");
    repo.commit("initial", "tester", 1).expect("commit");
    drop(repo);

    {
        let connection = rusqlite::Connection::open(dir.join("odc.db")).expect("open db");
        connection
            .execute(
                "UPDATE blocks SET data = substr(data, 1, 8) WHERE block_hash = (SELECT MIN(block_hash) FROM blocks)",
                [],
            )
            .expect("truncate block");
    }

    let repo = Repo::open(&dir).expect("open");
    let report = repo.verify().expect("verify must not fail hard");
    assert!(!report.ok);
    assert!(report.errors.iter().any(|error| error.contains("block")));
    cleanup(&dir);
}

#[test]
fn opening_a_missing_repository_fails_explicitly() {
    let dir = temp_dir("missing");
    let nested = dir.join("nope");
    assert!(matches!(Repo::open(&nested), Err(Error::NotARepository(_))));
    cleanup(&dir);
}
