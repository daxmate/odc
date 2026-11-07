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
