use odc_core::block::{self, Record};
use odc_core::cdc::{self, MAX_CHUNK, MIN_CHUNK};
use odc_core::hash;

fn make_records(count: usize, payload_size: usize) -> Vec<Record> {
    (0..count)
        .map(|index| {
            let key = format!("word{index:06}");
            let body = format!("{key}::{}", "x".repeat(payload_size));
            Record::new(key.into_bytes(), body.into_bytes())
        })
        .collect()
}

#[test]
fn gear_table_is_derived_from_blake3() {
    let table = cdc::gear_table();
    assert_eq!(table.len(), 256);
    let mut input = b"odc-gear-v1".to_vec();
    input.push(0);
    let digest = hash::blake3(&input);
    let mut expected = [0u8; 8];
    expected.copy_from_slice(&digest[..8]);
    assert_eq!(table[0], u64::from_le_bytes(expected));
    assert_eq!(cdc::gear_table(), table);
}

#[test]
fn boundaries_cover_every_record_in_order() {
    let records = make_records(3000, 120);
    let ranges = cdc::boundaries(&records);
    assert!(
        ranges.len() > 1,
        "expected several blocks, got {}",
        ranges.len()
    );
    let mut next = 0;
    for range in &ranges {
        assert_eq!(range.start, next);
        assert!(range.end > range.start);
        next = range.end;
    }
    assert_eq!(next, records.len());
}

#[test]
fn boundaries_are_deterministic() {
    let records = make_records(2000, 80);
    assert_eq!(cdc::boundaries(&records), cdc::boundaries(&records));
}

#[test]
fn blocks_respect_the_size_window() {
    let records = make_records(3000, 120);
    let sizes = cdc::block_sizes(&records);
    let mut chunked = 0;
    for (range, size) in cdc::boundaries(&records).iter().zip(&sizes) {
        let single = range.end - range.start == 1;
        let record_len = records[range.start].encoded_len();
        if single && record_len > MAX_CHUNK {
            assert!(*size > MAX_CHUNK);
            continue;
        }
        assert!(
            *size <= MAX_CHUNK + 64 || *size < MIN_CHUNK,
            "block too large: {size}"
        );
        if *size >= MIN_CHUNK {
            chunked += 1;
        }
    }
    assert!(chunked > 0);
}

#[test]
fn oversized_record_gets_its_own_block() {
    let mut records = vec![Record::new(b"aaa".to_vec(), b"small".to_vec())];
    records.push(Record::new(b"bbb".to_vec(), vec![7u8; MAX_CHUNK + 1024]));
    records.push(Record::new(b"ccc".to_vec(), b"small".to_vec()));
    let ranges = cdc::boundaries(&records);
    assert_eq!(ranges.len(), 3);
    assert_eq!(ranges[1], 1..2);
}

#[test]
fn chunking_roundtrips_through_blocks() {
    let records = make_records(1500, 200);
    let ranges = cdc::boundaries(&records);
    let mut recovered = Vec::new();
    for range in ranges {
        let raw = block::encode(&records[range.clone()]).expect("encode");
        let decoded = block::decode(&raw).expect("decode");
        recovered.extend(decoded.records);
    }
    assert_eq!(recovered, records);
}
