//! CLI integration tests for `odc pack` and shallow repositories.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn odc(path: &Path) -> Command {
    let mut command = Command::cargo_bin("odc").expect("odc binary");
    command.arg("--repo").arg(path);
    command
}

fn write(dir: &Path, name: &str, contents: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    fs::write(&path, contents).expect("write fixture");
    path
}

fn entries(words: &[&str]) -> String {
    let mut out = String::new();
    for word in words {
        out.push_str(&format!(
            "{{\"word\":\"{word}\",\"display\":\"{word}\",\"phonetic\":[],\"defs\":[{{\"pos\":\"noun\",\"sense\":\"{word} sense\",\"examples\":[],\"labels\":[]}}]}}\n"
        ));
    }
    out
}

/// Repository with three commits: genesis, "one" (apple, book) and
/// "two" (apple, book, cat).
fn seed(dir: &Path) {
    odc(dir).arg("init").assert().success();
    let first = write(dir, "one.jsonl", &entries(&["apple", "book"]));
    odc(dir).arg("import").arg(&first).assert().success();
    odc(dir).args(["commit", "-m", "one"]).assert().success();
    let second = write(dir, "two.jsonl", &entries(&["apple", "book", "cat"]));
    odc(dir).arg("import").arg(&second).assert().success();
    odc(dir).args(["commit", "-m", "two"]).assert().success();
}

#[test]
fn shallow_pack_logs_head_only_and_marks_the_boundary() {
    let temp = TempDir::new().expect("tempdir");
    let repo = temp.path().join("repo");
    seed(&repo);
    let out = temp.path().join("head.odc");

    odc(&repo)
        .args(["pack", "--shallow", "-o"])
        .arg(&out)
        .assert()
        .success()
        .stdout(predicate::str::contains("packed shallow"));

    assert!(out.is_file());
    assert!(!Path::new(&format!("{}-wal", out.display())).exists());

    odc(&out)
        .arg("log")
        .assert()
        .success()
        .stdout(predicate::str::contains("two"))
        .stdout(predicate::str::contains("one").not())
        .stderr(predicate::str::contains("(shallow)"));
    odc(&out)
        .arg("verify")
        .assert()
        .success()
        .stdout(predicate::str::contains("verify: OK"));
    odc(&out)
        .args(["show", "cat"])
        .assert()
        .success()
        .stdout(predicate::str::contains("cat"));
    odc(&out)
        .args(["search", "--prefix", "ca"])
        .assert()
        .success()
        .stdout(predicate::str::contains("cat"));
    odc(&out)
        .args(["export", "--format", "jsonl"])
        .assert()
        .success()
        .stdout(predicate::str::contains("cat"));
}

#[test]
fn full_pack_keeps_history() {
    let temp = TempDir::new().expect("tempdir");
    let repo = temp.path().join("repo");
    seed(&repo);
    let out = temp.path().join("full.odc");

    odc(&repo)
        .args(["pack", "-o"])
        .arg(&out)
        .assert()
        .success()
        .stdout(predicate::str::contains("packed full"));

    odc(&out)
        .arg("log")
        .assert()
        .success()
        .stdout(predicate::str::contains("two"))
        .stdout(predicate::str::contains("one"))
        .stderr(predicate::str::contains("(shallow)").not());
    odc(&out)
        .arg("verify")
        .assert()
        .success()
        .stdout(predicate::str::contains("verify: OK"));
}

#[test]
fn pack_default_path_and_existing_output_rules() {
    let temp = TempDir::new().expect("tempdir");
    let repo = temp.path().join("repo");
    seed(&repo);

    odc(&repo).args(["pack", "--shallow"]).assert().success();
    let default = temp.path().join("repo.odc");
    assert!(default.is_file(), "default output beside the repository");

    odc(&repo)
        .args(["pack", "--shallow"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
    odc(&repo)
        .args(["pack", "--shallow", "--force"])
        .assert()
        .success();
}

#[test]
fn shallow_repository_survives_gc() {
    let temp = TempDir::new().expect("tempdir");
    let repo = temp.path().join("repo");
    seed(&repo);
    let out = temp.path().join("head.odc");
    odc(&repo)
        .args(["pack", "--shallow", "-o"])
        .arg(&out)
        .assert()
        .success();

    odc(&out)
        .args(["gc", "--keep-last", "1"])
        .assert()
        .success();
    odc(&out)
        .arg("verify")
        .assert()
        .success()
        .stdout(predicate::str::contains("verify: OK"));
    odc(&out)
        .args(["show", "cat"])
        .assert()
        .success()
        .stdout(predicate::str::contains("cat sense"));
}

#[test]
fn shallow_patch_apply_advances_head() {
    let temp = TempDir::new().expect("tempdir");
    let repo = temp.path().join("repo");
    seed(&repo);

    // Snapshot the repository at "two", then keep editing the source.
    let packed = temp.path().join("two.odc");
    odc(&repo)
        .args(["pack", "--shallow", "-o"])
        .arg(&packed)
        .assert()
        .success();

    let third = write(
        &repo,
        "three.jsonl",
        &entries(&["apple", "book", "cat", "dog"]),
    );
    odc(&repo).arg("import").arg(&third).assert().success();
    odc(&repo)
        .args(["commit", "-m", "three"])
        .assert()
        .success();

    let patch = temp.path().join("fix.odp");
    odc(&repo)
        .args(["patch", "create", "HEAD~1", "HEAD", "-o"])
        .arg(&patch)
        .assert()
        .success();

    odc(&packed)
        .arg("patch")
        .arg("apply")
        .arg(&patch)
        .assert()
        .success();
    odc(&packed)
        .args(["show", "dog"])
        .assert()
        .success()
        .stdout(predicate::str::contains("dog sense"));
    odc(&packed)
        .arg("verify")
        .assert()
        .success()
        .stdout(predicate::str::contains("verify: OK"));
    odc(&packed)
        .arg("log")
        .assert()
        .success()
        .stdout(predicate::str::contains("apply patch"))
        .stdout(predicate::str::contains("two"));
}
