//! End-to-end CLI coverage for the dialect survey.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const ENTRIES: [(&str, &str); 3] = [
    (
        "alpha",
        r#"<div class="entry"><span class="hwd">alpha</span><span class="def">first</span></div>"#,
    ),
    (
        "bravo",
        r#"<div class="entry"><span class="hwd">bravo</span><span class="def">second</span></div>"#,
    ),
    (
        "charlie",
        r#"<div class="entry"><span class="hwd">charlie</span></div>"#,
    ),
];

fn odc() -> Command {
    Command::cargo_bin("odc").expect("odc binary")
}

fn fixture(dir: &Path, entries: &[(&str, &str)]) -> PathBuf {
    let path = dir.join("survey.mdx");
    odc_testkit::write_v2_mdx(&path, entries).expect("fixture");
    path
}

#[test]
fn survey_writes_report_and_samples() {
    let temp = TempDir::new().expect("temp dir");
    let mdx = fixture(temp.path(), &ENTRIES);
    let out = temp.path().join("report");

    let output = odc()
        .arg("survey")
        .arg(&mdx)
        .arg("--out")
        .arg(&out)
        .arg("--json")
        .output()
        .expect("run odc survey");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let report: Value = serde_json::from_slice(&output.stdout).expect("stdout is a JSON report");
    assert_eq!(report["entries_scanned"], 3);
    assert_eq!(report["verdict"]["kinds"], 2);
    assert_eq!(report["verdict"]["clusters"], 2);
    assert_eq!(report["verdict"]["grade"], "easy");
    assert_eq!(report["estimate"]["rules"], 2);
    assert_eq!(report["estimate"]["ops"], 0);
    assert_eq!(report["estimate"]["verdict"], "easy");
    assert!(report["estimate"]["basis"].as_array().is_some());
    assert!(report["quirks"].as_array().is_some());

    assert!(out.join("survey.md").is_file());
    assert!(out.join("survey.json").is_file());
    let samples = out.join("samples");
    let clusters: Vec<_> = fs::read_dir(&samples)
        .expect("samples dir")
        .filter_map(Result::ok)
        .collect();
    assert_eq!(clusters.len(), 2);
    assert!(clusters.iter().all(|entry| fs::read_dir(entry.path())
        .expect("cluster dir")
        .next()
        .is_some()));
}

#[test]
fn survey_exits_two_without_a_container() {
    let temp = TempDir::new().expect("temp dir");
    let mdx = fixture(temp.path(), &[("alpha", "plain text, no markup")]);
    let out = temp.path().join("report");

    odc()
        .arg("survey")
        .arg(&mdx)
        .arg("--out")
        .arg(&out)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("no usable entry container"));
}

#[test]
fn survey_rejects_unsupported_selectors() {
    let temp = TempDir::new().expect("temp dir");
    let mdx = fixture(temp.path(), &ENTRIES);
    let out = temp.path().join("report");

    odc()
        .arg("survey")
        .arg(&mdx)
        .arg("--out")
        .arg(&out)
        .arg("--entry-selector")
        .arg("div > p")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("unsupported selector syntax"));
}
#[test]
fn survey_dumps_a_reproducible_stratified_sample() {
    let temp = TempDir::new().expect("temp dir");
    let mut rows: Vec<(String, String)> = Vec::new();
    for (letter, name) in [
        ("a", "alpha"),
        ("b", "bravo"),
        ("c", "charlie"),
        ("d", "delta"),
    ] {
        for index in 0..10 {
            rows.push((
                format!("{name}{index}"),
                format!(
                    r#"<div class="entry"><span class="hwd">{name}{index}</span><span class="def">{letter}</span></div>"#
                ),
            ));
        }
    }
    rows.push(("alpha-alias".to_string(), "@@@LINK=alpha0".to_string()));
    let entries: Vec<(&str, &str)> = rows
        .iter()
        .map(|(key, html)| (key.as_str(), html.as_str()))
        .collect();
    let mdx = fixture(temp.path(), &entries);
    let out = temp.path().join("report");
    let first = temp.path().join("first.jsonl");
    let second = temp.path().join("second.jsonl");

    for path in [&first, &second] {
        odc()
            .arg("survey")
            .arg(&mdx)
            .arg("--out")
            .arg(&out)
            .arg("--dump-jsonl")
            .arg(path)
            .arg("--dump-limit")
            .arg("20")
            .assert()
            .success();
    }

    let first_text = fs::read_to_string(&first).expect("jsonl");
    let second_text = fs::read_to_string(&second).expect("jsonl");
    assert_eq!(first_text, second_text, "the sample must be reproducible");

    let lines: Vec<&str> = first_text.lines().collect();
    assert_eq!(lines.len(), 20);
    let mut buckets = std::collections::BTreeMap::new();
    for line in &lines {
        let record: Value = serde_json::from_str(line).expect("jsonl record");
        assert!(record["key"].is_string());
        assert!(record["text"].is_string());
        assert!(!record["text"]
            .as_str()
            .unwrap_or("")
            .starts_with("@@@LINK="));
        let key = record["key"].as_str().expect("key").to_string();
        *buckets.entry(key[..1].to_uppercase()).or_insert(0) += 1;
    }
    assert_eq!(buckets.len(), 4);
    for count in buckets.values() {
        assert!(*count >= 5, "every bucket keeps its floor: {buckets:?}");
    }

    let keys = fs::read_to_string(first.with_extension("keys")).expect("keys manifest");
    assert_eq!(keys.lines().count(), lines.len());
    assert!(!keys.contains("@@@LINK"));
}

#[test]
fn a_zero_dump_limit_writes_every_record() {
    let temp = TempDir::new().expect("temp dir");
    let mut rows: Vec<(String, String)> = Vec::new();
    for (letter, name) in [("a", "alpha"), ("b", "bravo")] {
        for index in 0..6 {
            rows.push((
                format!("{name}{index}"),
                format!(
                    r#"<div class="entry"><span class="hwd">{name}{index}</span><span class="def">{letter}</span></div>"#
                ),
            ));
        }
    }
    rows.push(("alpha-alias".to_string(), "@@@LINK=alpha0".to_string()));
    let entries: Vec<(&str, &str)> = rows
        .iter()
        .map(|(key, html)| (key.as_str(), html.as_str()))
        .collect();
    let mdx = fixture(temp.path(), &entries);
    let out = temp.path().join("report");
    let jsonl = temp.path().join("all.jsonl");

    odc()
        .arg("survey")
        .arg(&mdx)
        .arg("--out")
        .arg(&out)
        .arg("--dump-jsonl")
        .arg(&jsonl)
        .arg("--dump-limit")
        .arg("0")
        .assert()
        .success();

    let text = fs::read_to_string(&jsonl).expect("jsonl");
    let lines = text.lines().filter(|line| !line.trim().is_empty()).count();
    // 12 body records, alias skipped: zero means "everything", not "nothing".
    assert_eq!(lines, 12);
}
