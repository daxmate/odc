//! CLI integration tests for `odc fetch --depth`.

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

/// Five commits: genesis plus four imports that each add one word.
fn seed(dir: &Path) {
    odc(dir).arg("init").assert().success();
    let words = [
        "apple",
        "apple\nbook",
        "apple\nbook\ncat",
        "apple\nbook\ncat\ndog",
    ];
    for (step, body) in words.iter().enumerate() {
        let list: Vec<&str> = body.split('\n').collect();
        let file = write(dir, &format!("step-{step}.jsonl"), &entries(&list));
        odc(dir).arg("import").arg(&file).assert().success();
        odc(dir)
            .args(["commit", "-m", &format!("step {step}")])
            .assert()
            .success();
    }
}

#[test]
fn fetch_deepens_a_shallow_pack_and_reports_json() {
    let work = TempDir::new().expect("temp dir");
    let repo = work.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    seed(&repo);
    let out = work.path().join("head.odc");

    odc(&repo)
        .args(["pack", "--shallow", "-o"])
        .arg(&out)
        .assert()
        .success()
        .stdout(predicate::str::contains("packed shallow"));

    odc(&out)
        .args(["fetch", "--from"])
        .arg(&repo)
        .args(["--depth", "3"])
        .assert()
        .success()
        .stdout(predicate::str::contains("fetched to depth 3"))
        .stdout(predicate::str::contains("still shallow"));

    odc(&out)
        .args(["log", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("step 2"))
        .stdout(predicate::str::contains("step 1"))
        .stdout(predicate::str::contains("step 0").not());

    odc(&out)
        .args(["verify"])
        .assert()
        .success()
        .stdout(predicate::str::contains("verify: OK"));

    odc(&out)
        .args(["fetch", "--from"])
        .arg(&repo)
        .args(["--depth", "99", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"complete\": true"))
        .stdout(predicate::str::contains("\"commits_added\": 2"));

    odc(&out)
        .args(["log"])
        .assert()
        .success()
        .stdout(predicate::str::contains("step 0"))
        .stdout(predicate::str::contains("(shallow)").not());

    odc(&out)
        .args(["verify"])
        .assert()
        .success()
        .stdout(predicate::str::contains("verify: OK ("));
}

#[test]
fn fetch_refuses_a_complete_repository() {
    let work = TempDir::new().expect("temp dir");
    let repo = work.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    seed(&repo);

    odc(&repo)
        .args(["fetch", "--from"])
        .arg(&repo)
        .args(["--depth", "3"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains(
            "fetch requires a shallow repository",
        ));
}

#[test]
fn fetch_rejects_depth_zero() {
    let work = TempDir::new().expect("temp dir");
    let repo = work.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    seed(&repo);
    let out = work.path().join("head.odc");
    odc(&repo)
        .args(["pack", "--shallow", "-o"])
        .arg(&out)
        .assert()
        .success();

    odc(&out)
        .args(["fetch", "--from"])
        .arg(&repo)
        .args(["--depth", "0"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("depth must be at least 1"));
}
