use odc_core::block::{self, Record};
use odc_core::cbor::{self, Value};
use odc_core::entry::{normalize_key, Def, Entry};
use odc_core::error::Error;
use odc_core::hash;
use odc_core::varint;

fn sample_entry(display: &str) -> Entry {
    let mut entry = Entry::new(display);
    entry.phonetic = vec!["ˈæp.əl".to_string()];
    entry.defs = vec![
        Def {
            pos: "noun".to_string(),
            sense: "a round fruit".to_string(),
            examples: vec!["an apple a day".to_string()],
            labels: vec!["common".to_string()],
        },
        Def::new("the tree"),
    ];
    entry.raw_html = Some("<b>apple</b>".to_string());
    entry
}

#[test]
fn varint_roundtrip_for_boundaries() {
    for value in [0u64, 1, 127, 128, 300, 65_535, 65_536, u64::MAX] {
        let encoded = varint::encode_to_vec(value);
        let (decoded, used) = varint::decode(&encoded).expect("decode");
        assert_eq!(decoded, value);
        assert_eq!(used, encoded.len());
    }
}

#[test]
fn cbor_encodes_maps_in_canonical_order() {
    let value = Value::Map(vec![
        (Value::Text("zz".to_string()), Value::Int(1)),
        (Value::Text("a".to_string()), Value::Int(2)),
    ]);
    let encoded = cbor::encode(&value);
    let decoded = cbor::decode(&encoded).expect("decode");
    assert_eq!(cbor::encode(&decoded), encoded);
    let names: Vec<&str> = decoded
        .entries()
        .filter_map(|(key, _)| key.as_text())
        .collect();
    assert_eq!(names, vec!["a", "zz"]);
    // 0xa2 map(2), 0x61 'a', 0x02, 0x62 "zz", 0x01
    assert_eq!(
        encoded,
        vec![0xa2, 0x61, b'a', 0x02, 0x62, b'z', b'z', 0x01]
    );
}

#[test]
fn cbor_rejects_non_canonical_map_order() {
    // map(2) with "zz" before "a" is not canonical.
    let bytes = vec![0xa2, 0x62, b'z', b'z', 0x01, 0x61, b'a', 0x02];
    assert!(matches!(cbor::decode(&bytes), Err(Error::Cbor(_))));
}

#[test]
fn block_roundtrip_preserves_records_and_offsets() {
    let records: Vec<Record> = ["alpha", "beta", "gamma"]
        .iter()
        .map(|word| Record::new(word.as_bytes().to_vec(), word.as_bytes().to_vec()))
        .collect();
    let raw = block::encode(&records).expect("encode");
    assert_eq!(&raw[..4], b"ODB1");
    assert_eq!(raw[4], block::FLAG_OFFSET_INDEX);
    let decoded = block::decode(&raw).expect("decode");
    assert_eq!(decoded.records, records);
    assert!(decoded.has_index);
    assert_eq!(decoded.offsets.len(), 3);
    for (record, offset) in decoded.records.iter().zip(&decoded.offsets) {
        let offset = usize::try_from(*offset).expect("offset");
        let length = record.encoded_len();
        let parsed = block::decode_record(&raw[offset..offset + length]).expect("record");
        assert_eq!(&parsed, record);
    }
}

#[test]
fn block_rejects_unsorted_records() {
    let records = vec![
        Record::new(b"b".to_vec(), b"1".to_vec()),
        Record::new(b"a".to_vec(), b"2".to_vec()),
    ];
    assert_eq!(block::encode(&records), Err(Error::UnsortedRecords));
}

#[test]
fn entry_payload_roundtrip_is_canonical() {
    let entry = sample_entry("Apple");
    let bytes = entry.encode_payload();
    assert_eq!(bytes, sample_entry("Apple").encode_payload());
    let decoded = Entry::decode_payload(&bytes).expect("decode");
    assert_eq!(decoded.display, entry.display);
    assert_eq!(decoded.defs, entry.defs);
    assert_eq!(decoded.raw_html, entry.raw_html);
    assert_eq!(decoded.content_hash(), entry.content_hash());
    assert_eq!(cbor::encode(&decoded.payload_value()), bytes);
}

#[test]
fn unknown_payload_fields_are_preserved_in_extra() {
    let mut entry = sample_entry("apple");
    entry
        .extra
        .push(("future_field".to_string(), Value::Int(7)));
    let decoded = Entry::decode_payload(&entry.encode_payload()).expect("decode");
    assert_eq!(
        decoded.extra,
        vec![("future_field".to_string(), Value::Int(7))]
    );
    assert_eq!(decoded.content_hash(), entry.content_hash());
}

#[test]
fn normalization_matches_the_spec() {
    assert_eq!(normalize_key("Apple"), "apple");
    assert_eq!(normalize_key("  Apple  "), "  apple  ");
    assert_eq!(normalize_key("ﬁle"), "file");
    assert_eq!(normalize_key("Straße"), "strasse");
    assert_eq!(normalize_key("ΣΟΦΟΣ"), "σοφοσ");
}

#[test]
fn merkle_rules_match_the_spec() {
    let zero = hash::ZERO_HASH;
    assert_eq!(hash::merkle_root(&[]), zero);
    let leaf_a = hash::leaf_hash(b"a", &[1u8; 32]);
    assert_eq!(hash::merkle_root(&[leaf_a]), leaf_a);
    let leaf_b = hash::leaf_hash(b"b", &[2u8; 32]);
    let leaf_c = hash::leaf_hash(b"c", &[3u8; 32]);
    let left = hash::node_hash(&leaf_a, &leaf_b);
    let root3 = hash::merkle_root(&[leaf_a, leaf_b, leaf_c]);
    assert_eq!(root3, hash::node_hash(&left, &leaf_c));
    let root2 = hash::merkle_root(&[leaf_a, leaf_b]);
    assert_eq!(root2, left);
}

#[test]
fn commit_hash_ignores_metadata() {
    let parent = [1u8; 32];
    let parent2 = [2u8; 32];
    let entry_root = [3u8; 32];
    let block_root = [4u8; 32];
    let a = hash::commit_hash(&parent, &parent2, &entry_root, &block_root, 10);
    let b = hash::commit_hash(&parent, &parent2, &entry_root, &block_root, 10);
    assert_eq!(a, b);
    let c = hash::commit_hash(&parent, &parent2, &entry_root, &block_root, 11);
    assert_ne!(a, c);
}

#[test]
fn duplicate_extra_keys_are_deduplicated() {
    let mut entry = sample_entry("dup");
    entry.extra.push(("dup".to_string(), Value::Int(1)));
    entry.extra.push(("dup".to_string(), Value::Int(2)));
    let payload = entry.payload_value();
    let extra = payload.get("extra").expect("extra field");
    assert_eq!(extra.entries().count(), 1);
    let encoded = entry.encode_payload();
    let decoded = cbor::decode(&encoded).expect("canonical");
    assert_eq!(cbor::encode(&decoded), encoded);
    let decoded_extra = decoded.get("extra").expect("extra field");
    assert_eq!(decoded_extra.entries().count(), 1);
}
