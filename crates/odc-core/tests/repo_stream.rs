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

#[test]
fn substring_search_matches_inside_words_and_escapes_like_wildcards() {
    let dir = temp_dir("substring");
    let mut repo = Repo::init(&dir).expect("init");
    let mut labelled = Entry::new("colour");
    labelled.display = "Colour Guide".to_string();
    let entries = vec![
        Entry::new("apple"),
        Entry::new("application"),
        Entry::new("banana"),
        Entry::new("where's the beef"),
        Entry::new("Beef up"),
        Entry::new("日本語"),
        Entry::new("100% pure"),
        Entry::new("snake_case"),
        labelled,
    ];
    repo.import(&entries, false).expect("import");
    repo.commit("words", "test", 1_700_000_002).expect("commit");

    // Matches inside the key, shortest key first.
    let hits = repo.search_substring("ppl", 10).expect("substring");
    assert_eq!(
        hits,
        vec![
            ("apple".to_string(), "apple".to_string()),
            ("application".to_string(), "application".to_string()),
        ]
    );

    // ASCII case folding applies to the normalised key and the display form.
    let hits = repo.search_substring("BEEF", 10).expect("substring");
    assert_eq!(
        hits.iter().map(|(key, _)| key.as_str()).collect::<Vec<_>>(),
        vec!["beef up", "where's the beef"]
    );
    let hits = repo.search_substring("GUIDE", 10).expect("display match");
    assert_eq!(
        hits,
        vec![("colour".to_string(), "Colour Guide".to_string())]
    );

    // CJK words match without any case folding.
    let hits = repo.search_substring("本", 10).expect("cjk");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, "日本語");

    // `%` and `_` are literal characters, not LIKE wildcards.
    let hits = repo.search_substring("100%", 10).expect("percent");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, "100% pure");
    assert_eq!(
        repo.search_substring("%", 10)
            .expect("literal percent")
            .len(),
        1
    );
    let hits = repo.search_substring("e_c", 10).expect("underscore");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, "snake_case");
    assert_eq!(
        repo.search_substring("_", 10)
            .expect("literal underscore")
            .len(),
        1
    );

    // limit, empty needle and misses.
    let hits = repo.search_substring("a", 1).expect("limit");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, "apple");
    assert!(repo.search_substring("", 10).expect("empty").is_empty());
    assert!(repo
        .search_substring("a", 0)
        .expect("zero limit")
        .is_empty());
    assert!(repo.search_substring("zzz", 10).expect("miss").is_empty());

    std::fs::remove_dir_all(&dir).ok();
}
