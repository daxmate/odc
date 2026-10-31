//! Property-based tests for the ODC core codecs.
//!
//! Each property targets an invariant that the hand-written unit tests only
//! sample: varint minimality, block round-tripping, canonical CBOR
//! round-tripping, entry payload round-tripping and content-defined chunking
//! stability.

use odc_core::block::{self, Record};
use odc_core::cbor::{self, Value};
use odc_core::cdc;
use odc_core::entry::{Def, Entry};
use odc_core::varint;
use proptest::collection::btree_map;
use proptest::prelude::*;
use std::collections::BTreeMap;

fn arb_text() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9 _-]{1,24}"
}

fn arb_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::Int),
        proptest::collection::vec(any::<u8>(), 0..32).prop_map(Value::Bytes),
        arb_text().prop_map(Value::Text),
    ];
    leaf.prop_recursive(3, 32, 4, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
            proptest::collection::btree_map(arb_text(), inner, 0..6).prop_map(|map| {
                Value::Map(
                    map.into_iter()
                        .map(|(key, value)| (Value::Text(key), value))
                        .collect(),
                )
            }),
        ]
    })
}

fn records_from(map: BTreeMap<String, Vec<u8>>) -> Vec<Record> {
    map.into_iter()
        .map(|(key, payload)| Record::new(key.into_bytes(), payload))
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    #[test]
    fn varint_roundtrip_and_minimal_length(value in any::<u64>()) {
        let encoded = varint::encode_to_vec(value);
        prop_assert_eq!(encoded.len(), varint::encoded_len(value));
        prop_assert!(encoded.len() <= varint::MAX_LEN);
        prop_assert_eq!(varint::decode(&encoded), Ok((value, encoded.len())));
    }

    #[test]
    fn varint_never_panics_on_arbitrary_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..16)) {
        // Decoding arbitrary input must either succeed or return an error;
        // panic (including a slicing panic) is a bug.
        if let Ok((_, used)) = varint::decode(&bytes) {
            prop_assert!(used > 0 && used <= bytes.len());
        }
    }

    #[test]
    fn block_roundtrip_preserves_records(
        map in btree_map(arb_text(), proptest::collection::vec(any::<u8>(), 0..64), 0..8)
    ) {
        let records = records_from(map);
        let raw = block::encode(&records).expect("encode");
        let decoded = block::decode(&raw).expect("decode");
        prop_assert_eq!(&decoded.records, &records);
        prop_assert_eq!(decoded.offsets.len(), records.len());
        prop_assert!(decoded.has_index);
        for (record, offset) in records.iter().zip(&decoded.offsets) {
            let start = usize::try_from(*offset).expect("offset fits usize");
            let end = start + record.encoded_len();
            let parsed = block::decode_record(&raw[start..end]).expect("decode_record");
            prop_assert_eq!(&parsed, record);
        }
    }

    #[test]
    fn cbor_value_roundtrip(value in arb_value()) {
        let encoded = cbor::encode(&value);
        let decoded = cbor::decode(&encoded).expect("canonical bytes decode");
        // Map entry order inside `Value::Map` follows insertion order, while
        // the encoder sorts keys canonically; equality therefore holds on the
        // encoded form, not on the in-memory vector order.
        prop_assert_eq!(cbor::encode(&decoded), encoded.clone());
        prop_assert_eq!(cbor::validate_canonical(&encoded), Ok(decoded));
    }

    #[test]
    fn entry_payload_roundtrip(
        display in arb_text(),
        senses in proptest::collection::vec("[a-z ]{0,20}", 0..4),
        examples in proptest::collection::vec(arb_text(), 0..3),
    ) {
        let mut entry = Entry::new(display);
        entry.defs = senses
            .into_iter()
            .map(|sense| Def {
                pos: "noun".to_string(),
                sense,
                examples: examples.clone(),
                labels: vec!["domain".to_string()],
            })
            .collect();
        let payload = entry.encode_payload();
        let decoded = Entry::decode_payload(&payload).expect("payload decodes");
        prop_assert_eq!(&decoded.display, &entry.display);
        prop_assert_eq!(&decoded.defs, &entry.defs);
        prop_assert_eq!(decoded.encode_payload(), payload);
    }

    #[test]
    fn cdc_boundaries_are_local(
        map in btree_map(arb_text(), proptest::collection::vec(any::<u8>(), 0..200), 4..24),
        insert_at in 0usize..24,
        inserted in arb_text(),
    ) {
        // Insert one extra record at an arbitrary position. Every boundary that
        // lies strictly before the block containing the insertion point must be
        // unchanged: chunking is defined on the byte stream and the rolling
        // state is reset at each boundary.
        let base = records_from(map);
        let original = cdc::boundaries(&base);
        let insert_at = insert_at.min(base.len());
        let mut extended = base.clone();
        extended.insert(insert_at, Record::new(inserted.into_bytes(), vec![0xAB; 16]));

        prop_assert_eq!(cdc::boundaries(&base), original.clone());
        let updated = cdc::boundaries(&extended);
        prop_assert!(!updated.is_empty());
        prop_assert_eq!(updated.last().map(|range| range.end), Some(extended.len()));

        let containing = original.iter().position(|range| range.contains(&insert_at));
        if let Some(index) = containing {
            let prefix_end = original[index].start;
            if prefix_end > 0 {
                prop_assert_eq!(&original[..index], &updated[..index]);
            }
        }
    }
}
