//! CLI integration tests: exercise the real binary through `assert_cmd` and a
//! throw-away repository directory.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn odc(dir: &Path) -> Command {
    let mut command = Command::cargo_bin("odc").expect("odc binary");
    command.arg("--repo").arg(dir);
    command
}

fn write(dir: &Path, name: &str, contents: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    fs::write(&path, contents).expect("write fixture");
    path
}

fn jsonl_fixture() -> &'static str {
    r#"{"word":"apple","display":"apple","phonetic":["/ˈæp.əl/"],"defs":[{"pos":"noun","sense":"a round fruit","examples":["an apple a day"],"labels":["food"]}]}
{"word":"book","display":"book","phonetic":[],"defs":[{"pos":"noun","sense":"bound pages","examples":[],"labels":[]}]}
"#
}

fn jsonl_fixture_v2() -> &'static str {
    r#"{"word":"apple","display":"apple","phonetic":["/ˈæp.əl/"],"defs":[{"pos":"noun","sense":"a round fruit (revised)","examples":[],"labels":[]}]}
{"word":"cherry","display":"cherry","phonetic":[],"defs":[{"pos":"noun","sense":"a small fruit","examples":[],"labels":[]}]}
"#
}

#[test]
fn init_reports_genesis_and_rejects_a_second_init() {
    let temp = TempDir::new().expect("tempdir");
    odc(temp.path())
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("initialized ODC repository"))
        .stdout(predicate::str::contains("genesis"));

    odc(temp.path())
        .arg("init")
        .assert()
        .failure()
        .stderr(predicate::str::contains("ODC-E013"));
}

#[test]
fn import_jsonl_and_csv() {
    let temp = TempDir::new().expect("tempdir");
    odc(temp.path()).arg("init").assert().success();

    let jsonl = write(temp.path(), "words.jsonl", jsonl_fixture());
    odc(temp.path())
        .arg("import")
        .arg(&jsonl)
        .assert()
        .success()
        .stdout(predicate::str::contains("imported 2 entries"));

    let csv = write(
        temp.path(),
        "more.csv",
        "word,pos,sense\ncandle,noun,a wax stick\n",
    );
    odc(temp.path())
        .args(["import", "--format", "csv"])
        .arg(&csv)
        .assert()
        .success()
        .stdout(predicate::str::contains("imported 1 entries"));

    // Without --prune the import merges into the working set.
    odc(temp.path())
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "changes: 3 added, 0 modified, 0 removed",
        ))
        .stdout(predicate::str::contains("working set: 3 entries"));
}

#[test]
fn commit_without_changes_is_reported() {
    let temp = TempDir::new().expect("tempdir");
    odc(temp.path()).arg("init").assert().success();
    let jsonl = write(temp.path(), "words.jsonl", jsonl_fixture());
    odc(temp.path())
        .arg("import")
        .arg(&jsonl)
        .assert()
        .success();
    odc(temp.path())
        .args(["commit", "-m", "initial"])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 entries"));

    odc(temp.path())
        .args(["commit", "-m", "again"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("ODC-E012"));

    odc(temp.path())
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("working set clean (2 entries)"));
}

#[test]
fn diff_json_has_the_documented_shape() {
    let temp = TempDir::new().expect("tempdir");
    odc(temp.path()).arg("init").assert().success();

    let first = write(temp.path(), "v1.jsonl", jsonl_fixture());
    odc(temp.path())
        .arg("import")
        .arg(&first)
        .assert()
        .success();
    odc(temp.path())
        .args(["commit", "-m", "v1"])
        .assert()
        .success();

    let second = write(temp.path(), "v2.jsonl", jsonl_fixture_v2());
    odc(temp.path())
        .args(["import", "--prune"])
        .arg(&second)
        .assert()
        .success();
    odc(temp.path())
        .args(["commit", "-m", "v2"])
        .assert()
        .success();

    let output = odc(temp.path())
        .args(["diff", "HEAD~1", "HEAD", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).expect("diff json");
    let object = value.as_object().expect("diff object");
    for key in ["from", "to", "added", "removed", "modified"] {
        assert!(object.contains_key(key), "missing key {key}");
    }
    let added: Vec<&str> = object["added"]
        .as_array()
        .expect("added array")
        .iter()
        .map(|value| value.as_str().expect("word"))
        .collect();
    let removed: Vec<&str> = object["removed"]
        .as_array()
        .expect("removed array")
        .iter()
        .map(|value| value.as_str().expect("word"))
        .collect();
    let modified: Vec<&str> = object["modified"]
        .as_array()
        .expect("modified array")
        .iter()
        .map(|value| value.as_str().expect("word"))
        .collect();
    assert_eq!(added, vec!["cherry"]);
    assert_eq!(removed, vec!["book"]);
    assert_eq!(modified, vec!["apple"]);
}

#[test]
fn show_json_returns_the_entry_and_missing_words_fail() {
    let temp = TempDir::new().expect("tempdir");
    odc(temp.path()).arg("init").assert().success();
    let jsonl = write(temp.path(), "words.jsonl", jsonl_fixture());
    odc(temp.path())
        .arg("import")
        .arg(&jsonl)
        .assert()
        .success();
    odc(temp.path())
        .args(["commit", "-m", "v1"])
        .assert()
        .success();

    let output = odc(temp.path())
        .args(["show", "apple", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).expect("show json");
    assert_eq!(value["word"], "apple");
    assert_eq!(value["defs"][0]["sense"], "a round fruit");

    // Unknown words must exit non-zero and mention the word on stderr.
    odc(temp.path())
        .args(["show", "durian"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("durian"));
}

#[test]
fn verify_and_log_report_repository_state() {
    let temp = TempDir::new().expect("tempdir");
    odc(temp.path()).arg("init").assert().success();
    let jsonl = write(temp.path(), "words.jsonl", jsonl_fixture());
    odc(temp.path())
        .arg("import")
        .arg(&jsonl)
        .assert()
        .success();
    odc(temp.path())
        .args(["commit", "-m", "initial"])
        .assert()
        .success();

    odc(temp.path())
        .arg("verify")
        .assert()
        .success()
        .stdout(predicate::str::contains("verify: OK"));

    let output = odc(temp.path())
        .args(["log", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).expect("log json");
    let entries = value.as_array().expect("log array");
    assert!(entries.len() >= 2, "genesis plus one commit");
    assert!(entries.iter().any(|entry| entry["message"] == "initial"));
    assert_eq!(
        entries.last().expect("genesis commit")["entry_count"],
        0,
        "genesis has no entries"
    );
}

#[test]
fn invalid_arguments_are_rejected_by_the_parser() {
    let temp = TempDir::new().expect("tempdir");
    odc(temp.path()).arg("init").assert().success();
    let fixture = write(temp.path(), "words.jsonl", jsonl_fixture());

    odc(temp.path())
        .args(["import", "--format", "yaml"])
        .arg(&fixture)
        .assert()
        .failure()
        .code(2);

    odc(temp.path())
        .args(["import", "/definitely/missing/file.jsonl"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("ODC-E"));
}
