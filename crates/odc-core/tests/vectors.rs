use odc_core::block::{self, Record};
use odc_core::cbor;
use odc_core::cdc;
use odc_core::hash;
use serde_json::Value;
use std::path::PathBuf;

fn vector_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spec/testvectors")
}

fn load(name: &str) -> Value {
    let path = vector_dir().join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    serde_json::from_str(&text).expect("valid json")
}

fn hex_to_bytes(text: &str) -> Vec<u8> {
    assert!(text.len().is_multiple_of(2), "odd hex length");
    (0..text.len() / 2)
        .map(|index| u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).expect("hex"))
        .collect()
}

fn hex32(text: &str) -> [u8; 32] {
    hex_to_bytes(text).try_into().expect("32 bytes")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn payload_vectors_match_the_reference_hashes() {
    let vector = load("payload_encoding.json");
    let cases = vector["cases"].as_array().expect("cases");
    assert!(!cases.is_empty());
    for case in cases {
        let payload_hex = case["payload_hex"].as_str().expect("payload_hex");
        let payload = hex_to_bytes(payload_hex);
        assert_eq!(hex(&payload), payload_hex);
        let decoded = cbor::decode(&payload).expect("canonical payload");
        assert_eq!(cbor::encode(&decoded), payload, "payload is canonical");
        assert_eq!(
            hash::to_hex(&hash::blake3(&payload)),
            case["content_hash"].as_str().expect("content_hash"),
            "content hash for {}",
            case["word"]
        );
        for entry in case["field_hashes"].as_array().expect("field_hashes") {
            let name = entry[0].as_str().expect("name");
            let expected = entry[1].as_str().expect("digest");
            let value = decoded.get(name).expect("field present");
            assert_eq!(
                hash::to_hex(&hash::blake3(&cbor::encode(value))),
                expected,
                "field hash for {name}"
            );
        }
    }
}

#[test]
fn block_vectors_roundtrip() {
    let vector = load("block_encoding.json");
    let raw = hex_to_bytes(vector["raw_block_hex"].as_str().expect("raw_block_hex"));
    let records: Vec<Record> = vector["records"]
        .as_array()
        .expect("records")
        .iter()
        .map(|item| {
            Record::new(
                item["key"].as_str().expect("key").as_bytes().to_vec(),
                hex_to_bytes(item["payload_hex"].as_str().expect("payload_hex")),
            )
        })
        .collect();
    let decoded = block::decode(&raw).expect("decode vector block");
    assert_eq!(decoded.records, records);
    let offsets: Vec<u64> = vector["offsets"]
        .as_array()
        .expect("offsets")
        .iter()
        .map(|item| item.as_u64().expect("offset"))
        .collect();
    assert_eq!(decoded.offsets, offsets);
    assert_eq!(
        hash::to_hex(&hash::blake3(&raw)),
        vector["block_hash"].as_str().expect("block_hash")
    );
    assert_eq!(block::encode(&records).expect("re-encode"), raw);
}

#[test]
fn cdc_vectors_match_boundaries() {
    let vector = load("cdc_boundaries.json");
    let sizes: Vec<usize> = vector["sizes"]
        .as_array()
        .expect("sizes")
        .iter()
        .map(|item| usize::try_from(item.as_u64().expect("size")).expect("usize"))
        .collect();
    let records: Vec<Record> = sizes
        .iter()
        .enumerate()
        .map(|(index, size)| {
            Record::new(
                format!("word{index:04}").into_bytes(),
                format!("sense-{index:04}-{}", "x".repeat(*size)).into_bytes(),
            )
        })
        .collect();
    let ranges = cdc::boundaries(&records);
    let chunks = vector["chunks"].as_array().expect("chunks");
    assert_eq!(ranges.len(), chunks.len());
    for (range, chunk) in ranges.iter().zip(chunks) {
        assert_eq!(u64::try_from(range.start).expect("start"), chunk["start"]);
        assert_eq!(u64::try_from(range.end).expect("end"), chunk["end"]);
        let bytes: usize = records[range.clone()].iter().map(Record::encoded_len).sum();
        assert_eq!(
            u64::try_from(bytes).expect("bytes"),
            chunk["bytes"].as_u64().expect("bytes")
        );
    }
    let gear_prefix = vector["gear_prefix"].as_array().expect("gear_prefix");
    for (index, expected) in gear_prefix.iter().enumerate() {
        assert_eq!(
            cdc::gear_table()[index],
            expected.as_u64().expect("gear entry")
        );
    }
}

#[test]
fn commit_vectors_match() {
    let vector = load("commit_hash.json");
    let cases = vector["cases"].as_array().expect("cases");
    assert!(!cases.is_empty());
    for case in cases {
        let computed = hash::commit_hash(
            &hex32(case["parent"].as_str().expect("parent")),
            &hex32(case["parent2"].as_str().expect("parent2")),
            &hex32(case["entry_root"].as_str().expect("entry_root")),
            &hex32(case["block_root"].as_str().expect("block_root")),
            case["entry_count"].as_u64().expect("entry_count"),
        );
        assert_eq!(
            hash::to_hex(&computed),
            case["commit_hash"].as_str().expect("commit_hash"),
            "case {}",
            case["name"]
        );
    }
}

#[test]
fn genesis_vector_matches() {
    let vector = load("genesis.json");
    let computed = hash::commit_hash(
        &hash::ZERO_HASH,
        &hash::ZERO_HASH,
        &hash::ZERO_HASH,
        &hash::ZERO_HASH,
        0,
    );
    assert_eq!(
        hash::to_hex(&computed),
        vector["commit_hash"].as_str().expect("commit_hash")
    );
    assert_eq!(
        vector["format_version"].as_str().expect("version"),
        odc_core::FORMAT_VERSION
    );
}
