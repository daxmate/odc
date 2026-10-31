//! Extra spec vectors that are computed by hand from the normative rules in
//! `docs/odc-spec.md` (sections 2 and 3), independently of the Rust encoder.
//! If the implementation and these vectors disagree, the implementation is
//! wrong by construction.

use odc_core::block::{self, Record};
use odc_core::varint;
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
    assert!(text.len().is_multiple_of(2), "odd length hex: {text}");
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).expect("hex digit"))
        .collect()
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn varint_vectors_match_hand_computed_encodings() {
    let document = load("varint_encoding.json");
    let cases = document["cases"].as_array().expect("cases array");
    assert!(cases.len() >= 10);
    for case in cases {
        let value: u64 = case["value_dec"]
            .as_str()
            .expect("value_dec string")
            .parse()
            .expect("u64");
        let expected = hex_to_bytes(case["hex"].as_str().expect("hex"));
        let length = case["length"].as_u64().expect("length") as usize;
        assert_eq!(varint::encode_to_vec(value), expected, "value {value}");
        assert_eq!(varint::encoded_len(value), length, "value {value}");
        assert_eq!(
            varint::decode(&expected),
            Ok((value, expected.len())),
            "value {value}"
        );
    }
}

#[test]
fn block_vector_matches_hand_computed_encoding() {
    let document = load("block_encoding_edges.json");
    let records: Vec<Record> = document["records"]
        .as_array()
        .expect("records")
        .iter()
        .map(|record| {
            Record::new(
                record["key"].as_str().expect("key").as_bytes().to_vec(),
                hex_to_bytes(record["payload_hex"].as_str().expect("payload_hex")),
            )
        })
        .collect();
    let raw = block::encode(&records).expect("encode");
    assert_eq!(
        bytes_to_hex(&raw),
        document["raw_block_hex"].as_str().expect("raw_block_hex")
    );
    let decoded = block::decode(&raw).expect("decode");
    assert_eq!(decoded.records, records);
    let offsets: Vec<u64> = document["offsets"]
        .as_array()
        .expect("offsets")
        .iter()
        .map(|offset| offset.as_u64().expect("offset"))
        .collect();
    assert_eq!(decoded.offsets, offsets);
    assert!(decoded.has_index);
}
