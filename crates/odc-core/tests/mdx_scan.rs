//! Record-section scanning and repair tests.

use odc_core::mdx::MdxSource;
use odc_core::mdx_scan::{is_sentinel_payload, scan, write_repaired_copy};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_path(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("odc-mdx-scan-{label}-{nanos}"));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join("fixture.mdx")
}

/// Build a version 2 file with `count` entries of roughly 8 KiB each.
fn fixture(label: &str, count: usize) -> (PathBuf, Vec<(String, String)>) {
    let path = temp_path(label);
    let entries: Vec<(String, String)> = (0..count)
        .map(|index| {
            let key = format!("word{index:04}");
            let body = format!("<p>{index}</p>{}", "x".repeat(8 * 1024));
            (key, body)
        })
        .collect();
    let rows: Vec<(&str, &str)> = entries
        .iter()
        .map(|(key, body)| (key.as_str(), body.as_str()))
        .collect();
    odc_testkit::write_v2_mdx(&path, &rows).expect("write fixture");
    (path, entries)
}

fn records_of(path: &Path) -> Vec<(String, String)> {
    let source = MdxSource::open(path).expect("open fixture");
    source
        .records()
        .collect::<Result<Vec<_>, _>>()
        .expect("read records")
        .into_iter()
        .map(|record| (record.key, record.html))
        .collect()
}

/// Flip one byte in the middle of a block's compressed payload.
fn corrupt_block(path: &Path, ordinal: usize) {
    let section = scan(path).expect("scan").expect("version 2 file");
    let block = &section.blocks[ordinal];
    let mut bytes = std::fs::read(path).expect("read fixture");
    let start = usize::try_from(block.file_offset).expect("offset fits usize");
    let size = usize::try_from(block.compressed_size).expect("size fits usize");
    let index = start + 8 + (size - 8) / 2;
    bytes[index] ^= 0xff;
    std::fs::write(path, bytes).expect("write corrupt fixture");
}

#[test]
fn scan_matches_the_documented_version_2_layout() {
    let (path, _) = fixture("layout", 40);
    let section = scan(&path).expect("scan").expect("version 2 file");
    let file_size = std::fs::metadata(&path).expect("metadata").len();
    assert!(section.num_blocks > 1, "fixture needs several blocks");
    assert_eq!(section.num_blocks, section.blocks.len() as u64);
    assert_eq!(section.index_len, section.num_blocks * 16);
    assert_eq!(
        section
            .blocks
            .iter()
            .map(|b| b.compressed_size)
            .sum::<u64>(),
        section.blocks_len
    );
    assert_eq!(
        section.offset + 32 + section.index_len + section.blocks_len,
        file_size
    );
    assert_eq!(section.num_entries, 40);
    assert!(
        section.corrupt.is_empty(),
        "a fresh fixture has no corrupt blocks"
    );
    let first = &section.blocks[0];
    assert_eq!(first.decompressed_offset, 0);
    let last = section.blocks.last().expect("blocks");
    assert!(last.decompressed_offset > 0);
    std::fs::remove_dir_all(path.parent().expect("dir")).ok();
}

#[test]
fn detects_a_corrupt_middle_block() {
    let (path, _) = fixture("middle", 40);
    let intact = scan(&path).expect("scan").expect("version 2 file");
    let ordinal = intact.blocks.len() / 2;
    corrupt_block(&path, ordinal);
    let section = scan(&path).expect("scan").expect("version 2 file");
    assert_eq!(section.corrupt.len(), 1, "exactly one block is damaged");
    let block = &section.corrupt[0];
    assert_eq!(block.ordinal, ordinal as u64);
    assert_eq!(
        block.declared_size,
        intact.blocks[ordinal].decompressed_size
    );
    assert!(
        block.detail.contains("zlib")
            || block.detail.contains("adler32")
            || block.detail.contains("decompressed")
    );
    let (start, end) = block.decompressed_range();
    assert_eq!(start, intact.blocks[ordinal].decompressed_offset);
    assert_eq!(end - start, block.declared_size);
    std::fs::remove_dir_all(path.parent().expect("dir")).ok();
}

#[test]
fn detects_corrupt_first_and_last_blocks() {
    let (path, _) = fixture("edges", 40);
    let intact = scan(&path).expect("scan").expect("version 2 file");
    let last = intact.blocks.len() - 1;
    corrupt_block(&path, 0);
    corrupt_block(&path, last);
    let section = scan(&path).expect("scan").expect("version 2 file");
    let ordinals: Vec<u64> = section.corrupt.iter().map(|block| block.ordinal).collect();
    assert_eq!(ordinals, vec![0, last as u64]);
    std::fs::remove_dir_all(path.parent().expect("dir")).ok();
}

#[test]
fn version_1_files_are_left_to_the_strict_path() {
    let path = temp_path("v1");
    odc_testkit::write_v1_mdx(&path, &[("alpha", "<p>a</p>"), ("bravo", "<p>b</p>")])
        .expect("write fixture");
    assert!(scan(&path).expect("scan").is_none());
    std::fs::remove_dir_all(path.parent().expect("dir")).ok();
}

#[test]
fn layout_self_check_rejects_truncated_files() {
    let (path, _) = fixture("truncated", 8);
    let mut bytes = std::fs::read(&path).expect("read fixture");
    bytes.pop();
    std::fs::write(&path, &bytes).expect("truncate fixture");
    let error = scan(&path).expect_err("self-check must reject the file");
    assert_eq!(error.code(), "ODC-E021");
    std::fs::remove_dir_all(path.parent().expect("dir")).ok();
}

#[test]
fn repair_keeps_every_record_offset() {
    let (path, entries) = fixture("repair", 40);
    let original = records_of(&path);
    let intact = scan(&path).expect("scan").expect("version 2 file");
    corrupt_block(&path, 1);
    let section = scan(&path).expect("scan").expect("version 2 file");
    assert_eq!(section.corrupt.len(), 1);

    let expected: HashMap<&str, &str> = entries
        .iter()
        .map(|(key, body)| (key.as_str(), body.as_str()))
        .collect();
    let repaired_path;
    {
        let copy = write_repaired_copy(&path, &section).expect("repair");
        repaired_path = copy.path().to_path_buf();
        assert!(repaired_path.exists(), "temporary copy exists while alive");
        let repaired = records_of(&repaired_path);
        assert_eq!(repaired.len(), original.len(), "no record gained or lost");
        let keys: Vec<&str> = repaired.iter().map(|(key, _)| key.as_str()).collect();
        let original_keys: Vec<&str> = original.iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(keys, original_keys, "key section is untouched");
        let skipped: Vec<&(String, String)> = repaired
            .iter()
            .filter(|(_, html)| is_sentinel_payload(html))
            .collect();
        assert!(!skipped.is_empty(), "damaged block became sentinel records");
        assert!(skipped.len() < repaired.len(), "other blocks survived");
        for (key, html) in &repaired {
            if is_sentinel_payload(html) {
                continue;
            }
            assert_eq!(expected[key.as_str()], html);
        }
        assert_eq!(
            skipped.len()
                + repaired
                    .iter()
                    .filter(|(_, h)| !is_sentinel_payload(h))
                    .count(),
            repaired.len()
        );
    }
    assert!(!repaired_path.exists(), "temporary copy is removed on drop");
    assert_eq!(intact.corrupt.len(), 0);
    std::fs::remove_dir_all(path.parent().expect("dir")).ok();
}

#[test]
fn repair_requires_corrupt_blocks() {
    let (path, _) = fixture("clean", 8);
    let section = scan(&path).expect("scan").expect("version 2 file");
    let error = write_repaired_copy(&path, &section).expect_err("clean files are not repaired");
    assert_eq!(error.code(), "ODC-E016");
    std::fs::remove_dir_all(path.parent().expect("dir")).ok();
}
