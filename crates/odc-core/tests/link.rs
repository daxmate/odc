//! `@@@LINK=` redirects: parsing, chained lookups, cycles and misses.

use odc_core::entry::Entry;
use odc_core::link::{redirect_target, LinkResolution, MAX_REDIRECT_HOPS};
use odc_core::Repo;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn scratch(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("odc-link-{name}-{stamp}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn cleanup(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

fn redirect(word: &str, target: &str) -> Entry {
    let mut entry = Entry::new(word);
    entry.raw_html = Some(format!("@@@LINK={target}"));
    entry
}

fn entry(word: &str, html: &str) -> Entry {
    let mut entry = Entry::new(word);
    entry.raw_html = Some(html.to_string());
    entry
}

fn seed(dir: &Path, entries: &[Entry]) -> Repo {
    let mut repo = Repo::init(dir).expect("init");
    repo.import(entries, true).expect("import");
    repo.commit("seed", "tester", 1_700_000_000)
        .expect("commit");
    repo
}

#[test]
fn target_needs_to_be_the_whole_body() {
    assert_eq!(redirect_target("@@@LINK=Apple"), Some("Apple"));
    assert_eq!(redirect_target("  @@@LINK=  Apple  "), Some("Apple"));
    assert_eq!(redirect_target("see @@@LINK=Apple"), None);
    assert_eq!(redirect_target("@@@LINK="), None);
    assert_eq!(redirect_target("<p>apple</p>"), None);
}

#[test]
fn direct_entry_reports_no_hops() {
    let dir = scratch("direct");
    let repo = seed(&dir, &[entry("apple", "<p>fruit</p>")]);

    let resolution = repo.resolve_link("Apple").expect("resolve");
    match resolution {
        LinkResolution::Found {
            key,
            hops,
            requested,
            ..
        } => {
            assert_eq!(requested, "apple");
            assert_eq!(key, "apple");
            assert_eq!(hops, 0);
        }
        other => panic!("unexpected resolution: {other:?}"),
    }
    cleanup(&dir);
}

#[test]
fn chain_reaches_the_final_entry() {
    let dir = scratch("chain");
    let repo = seed(
        &dir,
        &[
            redirect("apples", "apple"),
            redirect("apple", "malus"),
            entry("malus", "<p>fruit</p>"),
        ],
    );

    let resolution = repo.resolve_link("apples").expect("resolve");
    match resolution {
        LinkResolution::Found {
            key, hops, entry, ..
        } => {
            assert_eq!(key, "malus");
            assert_eq!(hops, 2);
            assert_eq!(entry.raw_html.as_deref(), Some("<p>fruit</p>"));
        }
        other => panic!("unexpected resolution: {other:?}"),
    }
    cleanup(&dir);
}

#[test]
fn missing_target_is_reported() {
    let dir = scratch("missing");
    let repo = seed(&dir, &[redirect("apples", "apple")]);

    let resolution = repo.resolve_link("apples").expect("resolve");
    match resolution {
        LinkResolution::Missing { target, hops, .. } => {
            assert_eq!(target, "apple");
            assert_eq!(hops, 1);
        }
        other => panic!("unexpected resolution: {other:?}"),
    }
    cleanup(&dir);
}

#[test]
fn cycle_stops_instead_of_looping() {
    let dir = scratch("cycle");
    let repo = seed(&dir, &[redirect("a", "b"), redirect("b", "a")]);

    let resolution = repo.resolve_link("a").expect("resolve");
    assert!(matches!(resolution, LinkResolution::Cycle { .. }));
    cleanup(&dir);
}

#[test]
fn chains_longer_than_the_limit_stop() {
    let dir = scratch("long");
    let mut entries: Vec<Entry> = (0..=MAX_REDIRECT_HOPS + 1)
        .map(|index| redirect(&format!("word{index}"), &format!("word{}", index + 1)))
        .collect();
    entries.push(entry(
        &format!("word{}", MAX_REDIRECT_HOPS + 2),
        "<p>end</p>",
    ));
    let repo = seed(&dir, &entries);

    let resolution = repo.resolve_link("word0").expect("resolve");
    assert!(matches!(resolution, LinkResolution::TooLong { .. }));
    cleanup(&dir);
}

#[test]
fn absent_word_is_not_found() {
    let dir = scratch("absent");
    let repo = seed(&dir, &[entry("apple", "<p>fruit</p>")]);

    let resolution = repo.resolve_link("nothing").expect("resolve");
    assert!(matches!(resolution, LinkResolution::NotFound { .. }));
    cleanup(&dir);
}
