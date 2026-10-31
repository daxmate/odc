//! Streaming import, entry iteration and prefix search at the repository layer.

use odc_core::entry::Entry;
use odc_core::repo::Repo;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let dir =
        std::env::temp_dir().join(format!("odc-stream-{}-{nanos}-{label}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn word(index: usize) -> String {
    format!("word{index:05}")
}

#[test]
fn streaming_import_batches_and_prunes() {
    let dir = temp_dir("import");
    let mut repo = Repo::init(&dir).expect("init");

    let mut index = 0usize;
    let report = repo
        .import_stream(
            || {
                if index >= 2500 {
                    return Ok(None);
                }
                let mut entry = Entry::new(word(index));
                entry.raw_html = Some(format!("<p>body {index}</p>"));
                index += 1;
                Ok(Some(entry))
            },
            false,
            |_| {},
        )
        .expect("stream import");
    assert_eq!(report.written, 2500);
    assert_eq!(report.removed, 0);
    assert_eq!(report.total, 2500);

    let status = repo.status().expect("status");
    assert_eq!(status.added, 2500);
    assert_eq!(status.total, 2500);
    assert_eq!(status.entries, 0);
    assert_eq!(status.blocks, 0);

    repo.commit("stream", "test", 1_700_000_000)
        .expect("commit");
    let status = repo.status().expect("status");
    assert!(status.is_clean());
    assert_eq!(status.entries, 2500);
    assert!(status.blocks >= 1);

    // Re-importing the first 1000 entries with --prune drops the rest.
    let mut index = 0usize;
    let report = repo
        .import_stream(
            || {
                if index >= 1000 {
                    return Ok(None);
                }
                let entry = Entry::new(word(index));
                index += 1;
                Ok(Some(entry))
            },
            true,
            |_| {},
        )
        .expect("pruned import");
    assert_eq!(report.written, 1000);
    assert_eq!(report.total, 1000);

    let status = repo.status().expect("status");
    assert_eq!(status.total, 1000);
    assert_eq!(status.removed, 1500);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn entry_iteration_is_ordered_and_prefix_search_uses_the_index() {
    let dir = temp_dir("search");
    let mut repo = Repo::init(&dir).expect("init");
    let names = ["alpha", "Alpine", "beta", "Bravo", "bravo", "zulu"];
    let entries: Vec<Entry> = names.iter().map(|name| Entry::new(*name)).collect();
    repo.import(&entries, false).expect("import");
    repo.commit("words", "test", 1_700_000_001).expect("commit");

    let mut seen = Vec::new();
    let count = repo
        .for_each_entry(|entry| {
            seen.push(entry.word);
            Ok(())
        })
        .expect("iterate");
    assert_eq!(count, 5);
    let mut sorted = seen.clone();
    sorted.sort();
    assert_eq!(seen, sorted, "entries must arrive in key order");

    let hits = repo.search_prefix("Al", 100).expect("prefix");
    assert_eq!(
        hits.iter()
            .map(|(word, _)| word.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "alpine"]
    );

    let limited = repo.search_prefix("b", 1).expect("prefix");
    assert_eq!(limited.len(), 1);
    assert_eq!(limited[0].0, "beta");

    let none = repo.search_prefix("xyz", 10).expect("prefix");
    assert!(none.is_empty());

    std::fs::remove_dir_all(&dir).ok();
}
