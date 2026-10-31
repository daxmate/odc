//! Generate the machine readable test vectors under `spec/testvectors/`.
//!
//! Run with `cargo run -p odc-core --example gen_testvectors`. The vectors are
//! derived from the algorithms in `docs/odc-spec.md`; an independent
//! implementation must reproduce every value.

use odc_core::block::{self, Record};
use odc_core::cdc;
use odc_core::entry::{Def, Entry};
use odc_core::hash;
use serde_json::json;
use std::path::PathBuf;

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn vector_dir() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|path| path.parent())
        .map(|root| root.join("spec/testvectors"))
        .expect("workspace root")
}

fn sample_entries() -> Vec<Entry> {
    let mut apple = Entry::new("apple");
    apple.phonetic = vec!["ˈæp.əl".to_string()];
    apple.defs = vec![
        Def {
            pos: "noun".to_string(),
            sense: "a round fruit".to_string(),
            examples: vec!["an apple a day".to_string()],
            labels: vec![],
        },
        Def {
            pos: "noun".to_string(),
            sense: "the tree of the genus Malus".to_string(),
            examples: vec![],
            labels: vec!["botany".to_string()],
        },
    ];
    let mut zebra = Entry::new("Zebra");
    zebra.defs = vec![Def::new("an African equine")];
    zebra.raw_html = Some("<i>zebra</i>".to_string());
    let mut empty = Entry::new("empty");
    empty.raw_html = None;
    vec![apple, zebra, empty]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = vector_dir();
    std::fs::create_dir_all(&dir)?;

    let entries = sample_entries();
    let payload_cases: Vec<serde_json::Value> = entries
        .iter()
        .map(|entry| {
            json!({
                "word": entry.word,
                "payload_hex": hex(&entry.encode_payload()),
                "content_hash": hash::to_hex(&entry.content_hash()),
                "field_hashes": entry
                    .field_hashes()
                    .into_iter()
                    .map(|(name, digest)| json!([name, hash::to_hex(&digest)]))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    std::fs::write(
        dir.join("payload_encoding.json"),
        serde_json::to_string_pretty(&json!({
            "format": "odc-testvector-v1",
            "kind": "payload_encoding",
            "cases": payload_cases,
        }))?,
    )?;

    let mut records: Vec<Record> = entries
        .iter()
        .map(|entry| Record::new(entry.word.as_bytes().to_vec(), entry.encode_payload()))
        .collect();
    records.sort_by(|left, right| left.key.cmp(&right.key));
    let raw = block::encode(&records)?;
    let decoded = block::decode(&raw)?;
    std::fs::write(
        dir.join("block_encoding.json"),
        serde_json::to_string_pretty(&json!({
            "format": "odc-testvector-v1",
            "kind": "block_encoding",
            "records": records
                .iter()
                .map(|record| json!({
                    "key": String::from_utf8_lossy(&record.key),
                    "payload_hex": hex(&record.payload),
                }))
                .collect::<Vec<_>>(),
            "raw_block_hex": hex(&raw),
            "offsets": decoded.offsets,
            "block_hash": hash::to_hex(&hash::blake3(&raw)),
        }))?,
    )?;

    let mut cdc_records = Vec::new();
    let mut sizes = Vec::new();
    for index in 0..400usize {
        let size = 20 + (index * 37) % 380;
        let key = format!("word{index:04}");
        let payload = format!("sense-{index:04}-{}", "x".repeat(size));
        sizes.push(size);
        cdc_records.push(Record::new(key.into_bytes(), payload.into_bytes()));
    }
    let ranges = cdc::boundaries(&cdc_records);
    std::fs::write(
        dir.join("cdc_boundaries.json"),
        serde_json::to_string_pretty(&json!({
            "format": "odc-testvector-v1",
            "kind": "cdc_boundaries",
            "record_count": cdc_records.len(),
            "size_pattern": "size = 20 + (index * 37) % 380",
            "sizes": sizes,
            "gear_prefix": (0..8).map(|index| cdc::gear_table()[index]).collect::<Vec<_>>(),
            "chunks": ranges
                .iter()
                .map(|range| json!({
                    "start": range.start,
                    "end": range.end,
                    "bytes": cdc_records[range.clone()]
                        .iter()
                        .map(Record::encoded_len)
                        .sum::<usize>(),
                }))
                .collect::<Vec<_>>(),
        }))?,
    )?;

    let parent = [0x11u8; 32];
    let parent2 = [0x22u8; 32];
    let entry_root = [0x33u8; 32];
    let block_root = [0x44u8; 32];
    let cases = vec![
        json!({
            "name": "genesis",
            "parent": hash::to_hex(&hash::ZERO_HASH),
            "parent2": hash::to_hex(&hash::ZERO_HASH),
            "entry_root": hash::to_hex(&hash::ZERO_HASH),
            "block_root": hash::to_hex(&hash::ZERO_HASH),
            "entry_count": 0,
            "commit_hash": hash::to_hex(&hash::commit_hash(
                &hash::ZERO_HASH,
                &hash::ZERO_HASH,
                &hash::ZERO_HASH,
                &hash::ZERO_HASH,
                0
            )),
        }),
        json!({
            "name": "simple",
            "parent": hash::to_hex(&parent),
            "parent2": hash::to_hex(&hash::ZERO_HASH),
            "entry_root": hash::to_hex(&entry_root),
            "block_root": hash::to_hex(&block_root),
            "entry_count": 42,
            "commit_hash": hash::to_hex(&hash::commit_hash(&parent, &hash::ZERO_HASH, &entry_root, &block_root, 42)),
        }),
        json!({
            "name": "merge-shape",
            "parent": hash::to_hex(&parent),
            "parent2": hash::to_hex(&parent2),
            "entry_root": hash::to_hex(&entry_root),
            "block_root": hash::to_hex(&block_root),
            "entry_count": 1000,
            "commit_hash": hash::to_hex(&hash::commit_hash(&parent, &parent2, &entry_root, &block_root, 1000)),
        }),
    ];
    std::fs::write(
        dir.join("commit_hash.json"),
        serde_json::to_string_pretty(&json!({
            "format": "odc-testvector-v1",
            "kind": "commit_hash",
            "cases": cases,
        }))?,
    )?;

    std::fs::write(
        dir.join("genesis.json"),
        serde_json::to_string_pretty(&json!({
            "format": "odc-testvector-v1",
            "kind": "genesis",
            "format_version": "0.1",
            "commit_hash": hash::to_hex(&hash::commit_hash(
                &hash::ZERO_HASH,
                &hash::ZERO_HASH,
                &hash::ZERO_HASH,
                &hash::ZERO_HASH,
                0
            )),
        }))?,
    )?;

    println!("wrote test vectors to {}", dir.display());
    Ok(())
}
