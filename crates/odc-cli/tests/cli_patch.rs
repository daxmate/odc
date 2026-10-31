//! CLI integration tests for `odc patch create|apply|verify`.

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
    fs::write(&path, contents).expect("fixture");
    path
}

fn jsonl(from: usize, to: usize, marker: Option<usize>) -> String {
    let mut out = String::new();
    for index in from..to {
        let sense = if Some(index) == marker {
            format!("definition {index:05} changed")
        } else {
            format!("definition {index:05}")
        };
        out.push_str(&format!(
            "{{\"word\":\"word-{index:05}\",\"display\":\"Word {index:05}\",\"defs\":[{{\"pos\":\"noun\",\"sense\":\"{sense}\"}}]}}\n"
        ));
    }
    out
}

fn log_json(dir: &Path) -> Vec<Value> {
    let output = odc(dir)
        .args(["log", "--json"])
        .output()
        .expect("log output");
    assert!(output.status.success(), "log failed");
    serde_json::from_slice(&output.stdout).expect("log json")
}

fn hash_of(dir: &Path, message: &str) -> String {
    log_json(dir)
        .into_iter()
        .find(|commit| commit["message"] == message)
        .and_then(|commit| commit["hash"].as_str().map(str::to_string))
        .expect("commit hash")
}

fn head_hash(dir: &Path) -> String {
    log_json(dir)[0]["hash"].as_str().expect("head").to_string()
}

#[test]
fn create_verify_and_apply_roundtrip() {
    let temp = TempDir::new().expect("tempdir");
    let source = temp.path().join("source");
    let mirror = temp.path().join("mirror");
    let patch_path = temp.path().join("fix.odp");

    odc(&source).arg("init").assert().success();
    let v1 = write(temp.path(), "v1.jsonl", &jsonl(0, 500, None));
    let v2 = write(temp.path(), "v2.jsonl", &jsonl(0, 500, Some(42)));
    odc(&source).args(["import"]).arg(&v1).assert().success();
    odc(&source)
        .args(["commit", "-m", "A", "--timestamp", "1700000000"])
        .assert()
        .success();
    odc(&source).args(["import"]).arg(&v2).assert().success();
    odc(&source)
        .args(["commit", "-m", "B", "--timestamp", "1700000100"])
        .assert()
        .success();
    let base = hash_of(&source, "A");
    let target = hash_of(&source, "B");

    let output = odc(&source)
        .args(["patch", "create", &base, &target, "-o"])
        .arg(&patch_path)
        .arg("--json")
        .output()
        .expect("create");
    assert!(output.status.success(), "create failed: {output:?}");
    let report: Value = serde_json::from_slice(&output.stdout).expect("create json");
    assert_eq!(report["base"], base);
    assert_eq!(report["target"], target);
    assert_eq!(report["index_changes"], 1);
    assert!(report["new_blocks"].as_u64().expect("count") >= 1);
    assert_eq!(
        report["bytes"].as_u64().expect("bytes"),
        fs::metadata(&patch_path).expect("len").len()
    );

    odc(&mirror).arg("init").assert().success();
    odc(&mirror).args(["import"]).arg(&v1).assert().success();
    odc(&mirror)
        .args(["commit", "-m", "A", "--timestamp", "1700000000"])
        .assert()
        .success();
    assert_eq!(head_hash(&mirror), base);

    odc(&mirror)
        .args(["patch", "verify"])
        .arg(&patch_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("ok: base"));

    odc(&mirror)
        .args(["patch", "apply"])
        .arg(&patch_path)
        .arg("--json")
        .assert()
        .success();
    assert_eq!(head_hash(&mirror), target);

    odc(&mirror)
        .args(["show", "word-00042", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("changed"));
}

#[test]
fn failures_exit_four_and_leave_the_repository_alone() {
    let temp = TempDir::new().expect("tempdir");
    let source = temp.path().join("source");
    let patch_path = temp.path().join("fix.odp");

    odc(&source).arg("init").assert().success();
    let v1 = write(temp.path(), "v1.jsonl", &jsonl(0, 200, None));
    let v2 = write(temp.path(), "v2.jsonl", &jsonl(0, 200, Some(7)));
    odc(&source).args(["import"]).arg(&v1).assert().success();
    odc(&source)
        .args(["commit", "-m", "A", "--timestamp", "1700000000"])
        .assert()
        .success();
    odc(&source).args(["import"]).arg(&v2).assert().success();
    odc(&source)
        .args(["commit", "-m", "B", "--timestamp", "1700000100"])
        .assert()
        .success();
    let base = hash_of(&source, "A");
    let target = hash_of(&source, "B");
    odc(&source)
        .args(["patch", "create", &base, &target, "-o"])
        .arg(&patch_path)
        .assert()
        .success();

    // Unknown revision is rejected with the stable commit code.
    odc(&source)
        .args(["patch", "create", "deadbeef", &target, "-o"])
        .arg(temp.path().join("nope.odp"))
        .assert()
        .code(4)
        .stderr(predicate::str::contains("ODC-E011"));

    // A repository with unrelated history rejects the patch without writing.
    let stranger = temp.path().join("stranger");
    odc(&stranger).arg("init").assert().success();
    let other = write(temp.path(), "other.jsonl", &jsonl(1000, 1050, None));
    odc(&stranger)
        .args(["import"])
        .arg(&other)
        .assert()
        .success();
    odc(&stranger)
        .args(["commit", "-m", "other", "--timestamp", "1700000200"])
        .assert()
        .success();
    let stranger_head = head_hash(&stranger);
    odc(&stranger)
        .args(["patch", "apply"])
        .arg(&patch_path)
        .assert()
        .code(4)
        .stderr(predicate::str::contains("ODC-E024"));
    assert_eq!(head_hash(&stranger), stranger_head);

    // A correct mirror applies once and refuses a replay.
    let mirror = temp.path().join("mirror");
    odc(&mirror).arg("init").assert().success();
    odc(&mirror).args(["import"]).arg(&v1).assert().success();
    odc(&mirror)
        .args(["commit", "-m", "A", "--timestamp", "1700000000"])
        .assert()
        .success();
    odc(&mirror)
        .args(["patch", "apply"])
        .arg(&patch_path)
        .assert()
        .success();
    assert_eq!(head_hash(&mirror), target);
    odc(&mirror)
        .args(["patch", "apply"])
        .arg(&patch_path)
        .assert()
        .code(4)
        .stderr(predicate::str::contains("ODC-E026"));
    assert_eq!(head_hash(&mirror), target);

    // A corrupted checksum is refused.
    let mut bytes = fs::read(&patch_path).expect("read patch");
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    let corrupted = temp.path().join("corrupt.odp");
    fs::write(&corrupted, &bytes).expect("write corrupt");
    odc(&mirror)
        .args(["patch", "verify"])
        .arg(&corrupted)
        .assert()
        .code(4)
        .stderr(predicate::str::contains("ODC-E023"));
}
