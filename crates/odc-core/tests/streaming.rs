//! Streaming paths: incremental Merkle folding, block cutting, the snapshot
//! codec, diff and a fixed-vector commit that has to stay byte-compatible with
//! the pre-M2 implementation.

use odc_core::block::Record;
use odc_core::cbor::{self, Value};
use odc_core::cdc::{self, Cut, StreamChunker};
use odc_core::entry::{Def, Entry};
use odc_core::hash::{self, MerkleAccumulator};
use odc_core::repo::Repo;
use odc_core::snapshot::{SnapshotReader, SnapshotWriter};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let dir = std::env::temp_dir().join(format!(
        "odc-streaming-{}-{nanos}-{label}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn leaf(index: u64) -> [u8; 32] {
    hash::blake3(&index.to_le_bytes())
}

#[test]
fn merkle_accumulator_matches_merkle_root() {
    for count in 0..=257usize {
        let leaves: Vec<[u8; 32]> = (0..count as u64).map(leaf).collect();
        let expected = hash::merkle_root(&leaves);
        let mut accumulator = MerkleAccumulator::new();
        for value in &leaves {
            accumulator.push(*value);
        }
        assert_eq!(accumulator.len(), count as u64, "leaf count {count}");
        assert_eq!(accumulator.is_empty(), count == 0);
        assert_eq!(accumulator.finish(), expected, "root for {count} leaves");
    }
}

#[test]
fn merkle_accumulator_matches_for_random_sized_leaves() {
    let mut leaves = Vec::new();
    let mut accumulator = MerkleAccumulator::new();
    let mut state = 12345u64;
    for step in 0..4096 {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let value = hash::blake3(&state.to_le_bytes());
        leaves.push(value);
        accumulator.push(value);
        if step % 257 == 0 {
            assert_eq!(accumulator.clone().finish(), hash::merkle_root(&leaves));
        }
    }
    assert_eq!(accumulator.finish(), hash::merkle_root(&leaves));
}

#[test]
fn stream_chunker_matches_boundaries() {
    let records: Vec<Record> = (0..600)
        .map(|index| {
            let payload_len = 1024 + (index * 37) % 4096;
            Record::new(
                format!("w{index:05}"),
                vec![u8::try_from(index % 251).unwrap_or(0); payload_len],
            )
        })
        .collect();
    let expected = cdc::boundaries(&records);

    let mut actual = Vec::new();
    let mut chunker = StreamChunker::new();
    let mut start = 0usize;
    for (index, record) in records.iter().enumerate() {
        let mut cut = chunker.push(record);
        if cut == Cut::Before {
            actual.push(start..index);
            start = index;
            cut = chunker.push(record);
        }
        if cut == Cut::After {
            actual.push(start..index + 1);
            start = index + 1;
        }
    }
    if start < records.len() {
        actual.push(start..records.len());
    }
    assert_eq!(actual, expected);
    assert!(expected.len() > 2, "the fixture must cut more than twice");
}

#[test]
fn snapshot_stream_matches_canonical_cbor() {
    let pairs: Vec<(String, [u8; 32])> = (0..500u64)
        .map(|index| (format!("w{index:04}"), leaf(index)))
        .collect();
    let mut writer = SnapshotWriter::new(Vec::new(), pairs.len() as u64).expect("writer");
    for (word, digest) in &pairs {
        writer.push(word, digest).expect("push");
    }
    let (bytes, raw_size) = writer.finish().expect("finish");

    let expected = cbor::encode(&Value::Array(
        pairs
            .iter()
            .map(|(word, digest)| {
                Value::Array(vec![
                    Value::Text(word.clone()),
                    Value::Bytes(digest.to_vec()),
                ])
            })
            .collect(),
    ));
    assert_eq!(bytes, expected, "streaming CBOR must equal cbor::encode");
    assert_eq!(raw_size, bytes.len() as u64);

    let mut reader = SnapshotReader::new(&bytes[..]).expect("reader");
    let mut decoded = Vec::new();
    while let Some(pair) = reader.next_pair().expect("pair") {
        decoded.push(pair);
    }
    assert_eq!(decoded, pairs);
    assert_eq!(reader.remaining(), 0);
}

#[test]
fn empty_snapshot_is_a_single_array_header() {
    let writer = SnapshotWriter::new(Vec::new(), 0).expect("writer");
    let (bytes, raw_size) = writer.finish().expect("finish");
    assert_eq!(bytes, vec![0x80]);
    assert_eq!(raw_size, 1);
    let mut reader = SnapshotReader::new(&bytes[..]).expect("reader");
    assert!(reader.next_pair().expect("none").is_none());
}

/// Entries used by the compatibility vector below.
fn vector_entry(index: usize) -> Entry {
    let mut entry = Entry::new(format!("W{index:04}"));
    entry.phonetic = vec![format!("phonetic {index}")];
    let mut def = Def::new(format!("sense {index}"));
    def.pos = "noun".to_string();
    def.examples = vec![format!("example {index}")];
    entry.defs = vec![def];
    entry
}

#[test]
fn streaming_commit_reproduces_the_m1_vector() {
    // Values produced by the M1 (pre-streaming) implementation for this exact
    // fixture: 40 entries, author "vector", timestamp 1_700_000_000.
    const ENTRY_ROOT: &str = "31aee88c85bf671520aa8d8f12f8651354c040a84ecacaa42439ce3f624bb48a";
    const BLOCK_ROOT: &str = "8ce8cc6bc7651f286d59d7ca17130574ce242a2819db3516c8f7901fcd4c7263";
    const COMMIT_HASH: &str = "3b0084d72abd36b4338fc02c5b30c7663f644a266a70f8523f455ad263282da8";

    let dir = temp_dir("vector");
    let mut repo = Repo::init(&dir).expect("init");
    let entries: Vec<Entry> = (0..40).map(vector_entry).collect();
    repo.import(&entries, true).expect("import");
    let outcome = repo
        .commit("golden", "vector", 1_700_000_000)
        .expect("commit");

    assert_eq!(hash::to_hex(&outcome.commit.entry_root), ENTRY_ROOT);
    assert_eq!(hash::to_hex(&outcome.commit.block_root), BLOCK_ROOT);
    assert_eq!(hash::to_hex(&outcome.commit.hash), COMMIT_HASH);
    assert_eq!(outcome.commit.entry_count, 40);
    assert_eq!(outcome.new_blocks, 1);
    assert_eq!(outcome.reused_blocks, 0);
    assert!(repo.verify().expect("verify").ok);
}

#[test]
fn diff_covers_add_remove_modify_and_empty() {
    let dir = temp_dir("diff");
    let mut repo = Repo::init(&dir).expect("init");

    let first: Vec<Entry> = ["alpha", "beta"]
        .iter()
        .map(|word| Entry::new((*word).to_string()))
        .collect();
    repo.import(&first, true).expect("import");
    repo.commit("first", "tester", 1_700_000_000)
        .expect("commit");

    let same = repo.diff("HEAD", "HEAD").expect("diff");
    assert!(same.added.is_empty());
    assert!(same.removed.is_empty());
    assert!(same.modified.is_empty());

    let mut alpha = Entry::new("alpha");
    alpha.phonetic = vec!["changed".to_string()];
    let second: Vec<Entry> = vec![alpha, Entry::new("cherry")];
    repo.import(&second, true).expect("import");
    repo.commit("second", "tester", 1_700_000_010)
        .expect("commit");

    let diff = repo.diff("HEAD~1", "HEAD").expect("diff");
    assert_eq!(diff.added, vec!["cherry".to_string()]);
    assert_eq!(diff.removed, vec!["beta".to_string()]);
    assert_eq!(diff.modified, vec!["alpha".to_string()]);
    assert!(repo.verify().expect("verify").ok);
}

#[test]
fn corrupted_commit_index_is_reported() {
    let dir = temp_dir("corrupt");
    let mut repo = Repo::init(&dir).expect("init");
    repo.import(&[Entry::new("alpha")], true).expect("import");
    repo.commit("first", "tester", 1_700_000_000)
        .expect("commit");
    drop(repo);

    let connection = rusqlite::Connection::open(dir.join("odc.db")).expect("open db");
    connection
        .execute("UPDATE commit_index SET data = X'00ff'", [])
        .expect("corrupt");
    drop(connection);

    let repo = Repo::open(&dir).expect("reopen");
    let report = repo.verify().expect("verify runs");
    assert!(!report.ok, "verify must flag the broken index");
    assert!(!report.errors.is_empty());
    assert!(repo
        .diff("HEAD", "HEAD")
        .expect("same revision")
        .added
        .is_empty());
}
