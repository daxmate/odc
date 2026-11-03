//! CLI integration tests for publish, verify-release, trust and signed patches.

use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn odc(dir: &Path) -> Command {
    let mut command = Command::cargo_bin("odc").expect("odc binary");
    command.arg("--repo").arg(dir);
    command
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

fn write(dir: &Path, name: &str, contents: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    fs::write(&path, contents).expect("fixture");
    path
}

fn read_json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("valid json")
}

fn head_hash(dir: &Path) -> String {
    let output = odc(dir)
        .args(["log", "--json"])
        .output()
        .expect("log output");
    let entries = read_json(&output.stdout);
    entries[0]["hash"].as_str().expect("head").to_string()
}

fn init_repo(dir: &Path, fixture: &Path) {
    odc(dir).arg("init").assert().success();
    odc(dir).arg("import").arg(fixture).assert().success();
    odc(dir)
        .args(["commit", "-m", "initial", "--timestamp", "1700000000"])
        .assert()
        .success();
}

fn keygen(temp: &TempDir, keys: &Path) {
    odc(temp.path())
        .args(["keygen", "--out"])
        .arg(keys)
        .assert()
        .success();
}

#[test]
fn publish_verify_and_trust_flow() {
    let temp = TempDir::new().expect("tempdir");
    let repo = temp.path().join("repo");
    let keys = temp.path().join("keys");
    let out = temp.path().join("release");
    let fixture = write(temp.path(), "v1.jsonl", &jsonl(0, 20, None));
    init_repo(&repo, &fixture);
    keygen(&temp, &keys);
    let public_key = fs::read_to_string(keys.join("default.pub"))
        .expect("public key file")
        .split_whitespace()
        .nth(1)
        .expect("hex")
        .to_string();

    let published = odc(&repo)
        .args(["publish", "--out"])
        .arg(&out)
        .args(["--name", "demo", "--label", "v1", "--json"])
        .env("ODC_KEY_DIR", &keys)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let report = read_json(&published);
    assert_eq!(report["publisher"]["public_key"], public_key);
    assert_eq!(report["artifacts"][0]["kind"], "pack");
    assert!(out.join("demo-v1.odc").exists());
    assert!(out.join("release.json").exists());

    let verified = odc(&repo)
        .args(["verify-release"])
        .arg(&out)
        .args(["--pubkey", &public_key, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let verified = read_json(&verified);
    assert_eq!(verified["valid"], true);
    assert_eq!(verified["publisher"]["public_key"], public_key);
    assert_eq!(verified["trust"], "pinned");

    odc(&repo)
        .args(["trust", "add", "--pub"])
        .arg(keys.join("default.pub"))
        .args(["--name", "alice"])
        .assert()
        .success();
    let listed = odc(&repo)
        .args(["trust", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let listed = read_json(&listed);
    assert_eq!(listed["publishers"][0]["public_key"], public_key);
    assert_eq!(listed["publishers"][0]["source"], "manual");
    assert_eq!(listed["publishers"][0]["name"], "alice");

    let verified = odc(&repo)
        .args(["verify-release"])
        .arg(&out)
        .args(["--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let verified = read_json(&verified);
    assert_eq!(verified["trust"], "trusted");

    odc(&repo)
        .args(["trust", "remove", &public_key])
        .assert()
        .success();
    let listed = odc(&repo)
        .args(["trust", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        read_json(&listed)["publishers"].as_array().unwrap().len(),
        0
    );
}

#[test]
fn publish_with_base_and_duplicate_refusal() {
    let temp = TempDir::new().expect("tempdir");
    let repo = temp.path().join("repo");
    let keys = temp.path().join("keys");
    let out = temp.path().join("release");
    let v1 = write(temp.path(), "v1.jsonl", &jsonl(0, 20, None));
    init_repo(&repo, &v1);
    let base = head_hash(&repo);
    let v2 = write(temp.path(), "v2.jsonl", &jsonl(0, 20, Some(3)));
    odc(&repo).arg("import").arg(&v2).assert().success();
    odc(&repo)
        .args(["commit", "-m", "second", "--timestamp", "1700000100"])
        .assert()
        .success();
    keygen(&temp, &keys);

    let published = odc(&repo)
        .args(["publish", "--out"])
        .arg(&out)
        .args(["--name", "demo", "--label", "v1", "--base", &base, "--json"])
        .env("ODC_KEY_DIR", &keys)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let report = read_json(&published);
    assert_eq!(report["base"], base);
    assert_eq!(report["artifacts"].as_array().unwrap().len(), 2);
    assert!(out.join("demo-v1.odp").exists());
    assert!(out.join("demo-v1.odp.sig").exists());

    odc(&repo)
        .args(["publish", "--out"])
        .arg(&out)
        .args(["--name", "demo", "--label", "v1", "--base", &base])
        .env("ODC_KEY_DIR", &keys)
        .assert()
        .failure();
}

#[test]
fn signed_patch_apply_honours_the_trust_store() {
    let temp = TempDir::new().expect("tempdir");
    let source = temp.path().join("source");
    let mirror = temp.path().join("mirror");
    let keys = temp.path().join("keys");
    let v1 = write(temp.path(), "v1.jsonl", &jsonl(0, 40, None));
    init_repo(&source, &v1);
    init_repo(&mirror, &v1);
    keygen(&temp, &keys);
    let base = head_hash(&source);
    assert_eq!(base, head_hash(&mirror));

    let v2 = write(temp.path(), "v2.jsonl", &jsonl(0, 40, Some(7)));
    odc(&source).arg("import").arg(&v2).assert().success();
    odc(&source)
        .args(["commit", "-m", "second", "--timestamp", "1700000100"])
        .assert()
        .success();
    let target = head_hash(&source);
    let patch = temp.path().join("fix.odp");
    odc(&source)
        .args(["patch", "create", &base, &target, "-o"])
        .arg(&patch)
        .assert()
        .success();
    let sig = temp.path().join("fix.odp.sig");
    odc(&source)
        .arg("sign")
        .arg(&patch)
        .args(["--out"])
        .arg(&sig)
        .env("ODC_KEY_DIR", &keys)
        .assert()
        .success();

    let before = head_hash(&mirror);
    odc(&mirror)
        .args(["patch", "apply"])
        .arg(&patch)
        .args(["--sig"])
        .arg(&sig)
        .assert()
        .failure()
        .code(4);
    assert_eq!(
        head_hash(&mirror),
        before,
        "rejected patch must not move HEAD"
    );
    let listed = odc(&mirror)
        .args(["trust", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        read_json(&listed)["publishers"].as_array().unwrap().len(),
        0
    );

    odc(&mirror)
        .args(["patch", "apply"])
        .arg(&patch)
        .args(["--sig"])
        .arg(&sig)
        .arg("--trust-new")
        .arg("--json")
        .assert()
        .success();
    assert_eq!(head_hash(&mirror), target);

    let listed = odc(&mirror)
        .args(["trust", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let listed = read_json(&listed);
    assert_eq!(listed["publishers"][0]["source"], "tofu");

    let v3 = write(temp.path(), "v3.jsonl", &jsonl(0, 40, Some(9)));
    odc(&source).arg("import").arg(&v3).assert().success();
    odc(&source)
        .args(["commit", "-m", "third", "--timestamp", "1700000200"])
        .assert()
        .success();
    let target3 = head_hash(&source);
    let patch2 = temp.path().join("fix2.odp");
    odc(&source)
        .args(["patch", "create", &target, &target3, "-o"])
        .arg(&patch2)
        .assert()
        .success();
    let sig2 = temp.path().join("fix2.odp.sig");
    odc(&source)
        .arg("sign")
        .arg(&patch2)
        .args(["--out"])
        .arg(&sig2)
        .env("ODC_KEY_DIR", &keys)
        .assert()
        .success();
    odc(&mirror)
        .args(["patch", "apply"])
        .arg(&patch2)
        .args(["--sig"])
        .arg(&sig2)
        .assert()
        .success();
    assert_eq!(head_hash(&mirror), target3);
}
