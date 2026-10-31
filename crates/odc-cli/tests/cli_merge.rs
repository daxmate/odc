//! CLI integration tests for merge, resolve, merge --continue and conflicts.

use assert_cmd::Command;
use predicates::prelude::*;
use rusqlite::{params, Connection};
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

fn jsonl(entries: &[(&str, &str)]) -> String {
    let mut out = String::new();
    for (word, sense) in entries {
        let value = serde_json::json!({
            "word": word,
            "display": word,
            "defs": [{ "pos": "noun", "sense": sense }],
        });
        out.push_str(&value.to_string());
        out.push(char::from(10));
    }
    out
}

fn checkpoint(dir: &Path) {
    let connection = Connection::open(dir.join("odc.db")).expect("db");
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .expect("checkpoint");
}

fn fork(source: &Path, target: &Path) {
    checkpoint(source);
    fs::create_dir_all(target).expect("fork dir");
    fs::copy(source.join("odc.db"), target.join("odc.db")).expect("copy db");
}

fn import_commits(target: &Path, source: &Path) {
    checkpoint(source);
    let connection = Connection::open(target.join("odc.db")).expect("target db");
    connection
        .execute(
            "ATTACH DATABASE ?1 AS fork",
            params![source.join("odc.db").to_string_lossy().to_string()],
        )
        .expect("attach");
    for table in ["commits", "commit_index", "blocks"] {
        connection
            .execute_batch(&format!(
                "INSERT OR IGNORE INTO main.{table} SELECT * FROM fork.{table};"
            ))
            .expect("copy objects");
    }
    connection
        .execute_batch("DETACH DATABASE fork;")
        .expect("detach");
}

fn head_of(dir: &Path) -> String {
    let output = odc(dir)
        .args(["log", "--json"])
        .output()
        .expect("log output");
    assert!(output.status.success(), "log failed: {output:?}");
    let commits: Value = serde_json::from_slice(&output.stdout).expect("log json");
    commits[0]["hash"].as_str().expect("hash").to_string()
}

fn parse_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("json output")
}

fn seed(temp: &TempDir, entries: &[(&str, &str)]) -> std::path::PathBuf {
    let repo = temp.path().join("repo");
    let file = write(temp.path(), "base.jsonl", &jsonl(entries));
    odc(&repo).arg("init").assert().success();
    odc(&repo).arg("import").arg(&file).assert().success();
    odc(&repo)
        .args(["commit", "-m", "base", "--timestamp", "1700000000"])
        .assert()
        .success();
    repo
}

#[test]
fn merges_disjoint_word_changes_and_reports_json() {
    let temp = TempDir::new().expect("tempdir");
    let repo_dir = seed(&temp, &[("alpha", "first"), ("beta", "second")]);
    let fork_dir = temp.path().join("fork");
    fork(&repo_dir, &fork_dir);

    let ours = write(
        temp.path(),
        "ours.jsonl",
        &jsonl(&[("alpha", "first ours"), ("beta", "second")]),
    );
    odc(&repo_dir).arg("import").arg(&ours).assert().success();
    odc(&repo_dir)
        .args(["commit", "-m", "ours", "--timestamp", "1700000010"])
        .assert()
        .success();

    let theirs = write(
        temp.path(),
        "theirs.jsonl",
        &jsonl(&[("alpha", "first"), ("beta", "second"), ("delta", "fourth")]),
    );
    odc(&fork_dir).arg("import").arg(&theirs).assert().success();
    odc(&fork_dir)
        .args(["commit", "-m", "theirs", "--timestamp", "1700000020"])
        .assert()
        .success();
    let theirs_hash = head_of(&fork_dir);
    import_commits(&repo_dir, &fork_dir);

    let ours_head = head_of(&repo_dir);
    let output = odc(&repo_dir)
        .args(["merge", "HEAD", &theirs_hash, "--json"])
        .output()
        .expect("merge output");
    assert!(output.status.success(), "merge failed: {output:?}");
    let report = parse_json(&output);
    assert_eq!(report["kind"], "merged");
    assert_eq!(report["theirs"], theirs_hash);
    assert_eq!(report["base"].as_str().map(str::len), Some(64));

    // parent2 records the merged commit.
    let log: Value = parse_json(
        &odc(&repo_dir)
            .args(["log", "--json"])
            .output()
            .expect("log"),
    );
    assert_eq!(log[0]["parent"].as_str(), Some(ours_head.as_str()));
    assert_eq!(log[0]["parent2"].as_str(), Some(theirs_hash.as_str()));
    odc(&repo_dir).arg("verify").assert().success();
    odc(&repo_dir)
        .arg("lookup")
        .arg("delta")
        .assert()
        .success()
        .stdout(predicate::str::contains("fourth"));
}

#[test]
fn conflicts_exit_with_five_and_can_be_resolved() {
    let temp = TempDir::new().expect("tempdir");
    let repo_dir = seed(&temp, &[("alpha", "first")]);
    let fork_dir = temp.path().join("fork");
    fork(&repo_dir, &fork_dir);

    let ours = write(
        temp.path(),
        "ours.jsonl",
        &jsonl(&[("alpha", "ours meaning")]),
    );
    odc(&repo_dir).arg("import").arg(&ours).assert().success();
    odc(&repo_dir)
        .args(["commit", "-m", "ours", "--timestamp", "1700000010"])
        .assert()
        .success();

    let theirs = write(
        temp.path(),
        "theirs.jsonl",
        &jsonl(&[("alpha", "theirs meaning")]),
    );
    odc(&fork_dir).arg("import").arg(&theirs).assert().success();
    odc(&fork_dir)
        .args(["commit", "-m", "theirs", "--timestamp", "1700000020"])
        .assert()
        .success();
    let theirs_hash = head_of(&fork_dir);
    import_commits(&repo_dir, &fork_dir);
    let before = head_of(&repo_dir);

    let output = odc(&repo_dir)
        .args(["merge", "HEAD", &theirs_hash, "--json"])
        .output()
        .expect("merge output");
    assert_eq!(output.status.code(), Some(5));
    let report = parse_json(&output);
    assert_eq!(report["kind"], "conflicts");
    assert_eq!(report["conflicts"][0]["word"], "alpha");
    assert_eq!(report["conflicts"][0]["fields"][0], "defs");
    assert_eq!(head_of(&repo_dir), before, "no commit while conflicted");

    let conflicts: Value = parse_json(
        &odc(&repo_dir)
            .args(["conflicts", "--json"])
            .output()
            .expect("conflicts"),
    );
    assert_eq!(conflicts.as_array().map(Vec::len), Some(1));

    odc(&repo_dir)
        .arg("resolve")
        .arg("alpha")
        .arg("--ours")
        .assert()
        .success();

    let output = odc(&repo_dir)
        .args(["merge", "--continue", "--json"])
        .output()
        .expect("continue");
    assert!(output.status.success(), "continue failed: {output:?}");
    let report = parse_json(&output);
    assert_eq!(report["kind"], "merged");
    odc(&repo_dir)
        .arg("lookup")
        .arg("alpha")
        .assert()
        .success()
        .stdout(predicate::str::contains("ours meaning"));
    odc(&repo_dir).arg("verify").assert().success();
}

#[test]
fn union_strategy_keeps_both_entries() {
    let temp = TempDir::new().expect("tempdir");
    let repo_dir = seed(&temp, &[("alpha", "first")]);
    let fork_dir = temp.path().join("fork");
    fork(&repo_dir, &fork_dir);

    let ours = write(
        temp.path(),
        "ours.jsonl",
        &jsonl(&[("alpha", "ours meaning")]),
    );
    odc(&repo_dir).arg("import").arg(&ours).assert().success();
    odc(&repo_dir)
        .args(["commit", "-m", "ours", "--timestamp", "1700000010"])
        .assert()
        .success();

    let theirs = write(
        temp.path(),
        "theirs.jsonl",
        &jsonl(&[("alpha", "theirs meaning")]),
    );
    odc(&fork_dir).arg("import").arg(&theirs).assert().success();
    odc(&fork_dir)
        .args(["commit", "-m", "theirs", "--timestamp", "1700000020"])
        .assert()
        .success();
    let theirs_hash = head_of(&fork_dir);
    import_commits(&repo_dir, &fork_dir);

    odc(&repo_dir)
        .args([
            "merge",
            "HEAD",
            &theirs_hash,
            "--strategy",
            "union",
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("merged"));
    odc(&repo_dir)
        .arg("lookup")
        .arg("alpha")
        .assert()
        .success()
        .stdout(predicate::str::contains("ours meaning"));
    odc(&repo_dir)
        .arg("lookup")
        .arg("alpha~2")
        .assert()
        .success()
        .stdout(predicate::str::contains("theirs meaning"));
    odc(&repo_dir).arg("verify").assert().success();
}

#[test]
fn patch_apply_leaves_a_clean_working_set_and_allows_merges() {
    let temp = TempDir::new().expect("tempdir");
    let repo_dir = seed(&temp, &[("alpha", "first"), ("beta", "second")]);
    let base_copy = temp.path().join("base-copy");
    fork(&repo_dir, &base_copy);
    let mirror = temp.path().join("mirror");
    fork(&base_copy, &mirror);

    let theirs = write(
        temp.path(),
        "theirs.jsonl",
        &jsonl(&[("alpha", "first"), ("beta", "second theirs")]),
    );
    odc(&repo_dir).arg("import").arg(&theirs).assert().success();
    odc(&repo_dir)
        .args(["commit", "-m", "theirs", "--timestamp", "1700000020"])
        .assert()
        .success();
    let patch_path = temp.path().join("p.odp");
    odc(&repo_dir)
        .args(["patch", "create", "HEAD~1", "HEAD", "-o"])
        .arg(&patch_path)
        .assert()
        .success();

    odc(&mirror)
        .args(["patch", "apply"])
        .arg(&patch_path)
        .assert()
        .success();
    odc(&mirror)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("working set clean"));

    // A divergent commit can be merged immediately (no ODC-E016).
    let other = write(
        temp.path(),
        "copy.jsonl",
        &jsonl(&[("alpha", "first copy"), ("beta", "second")]),
    );
    odc(&base_copy).arg("import").arg(&other).assert().success();
    odc(&base_copy)
        .args(["commit", "-m", "copy", "--timestamp", "1700000010"])
        .assert()
        .success();
    let other_hash = head_of(&base_copy);
    import_commits(&mirror, &base_copy);

    let output = odc(&mirror)
        .args(["merge", "HEAD", &other_hash, "--json"])
        .output()
        .expect("merge output");
    assert!(output.status.success(), "merge failed: {output:?}");
    let report = parse_json(&output);
    assert_eq!(report["kind"], "merged");
    odc(&mirror).arg("verify").assert().success();
}

#[test]
fn continuing_without_a_merge_is_rejected() {
    let temp = TempDir::new().expect("tempdir");
    let repo_dir = seed(&temp, &[("alpha", "first")]);
    odc(&repo_dir)
        .args(["merge", "--continue"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("ODC-E030"));
    odc(&repo_dir)
        .arg("conflicts")
        .assert()
        .success()
        .stdout(predicate::str::contains("no conflicts"));
}
