//! CLI integration tests for `odc keygen|sign|verify-signature`.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;

fn odc() -> Command {
    Command::cargo_bin("odc").expect("odc binary")
}

fn read_json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("valid json")
}

#[test]
fn keygen_sign_verify_roundtrip() {
    let temp = TempDir::new().expect("temp");
    let keys = temp.path().join("keys");
    let keys_arg = keys.to_str().expect("utf8").to_string();

    odc()
        .args(["keygen", "--out", &keys_arg, "--name", "alice"])
        .assert()
        .success()
        .stdout(predicate::str::contains("public key hex: "));

    let key_path = keys.join("alice.key");
    let public_path = keys.join("alice.pub");
    assert!(key_path.exists());
    assert!(public_path.exists());
    assert_eq!(
        fs::metadata(&key_path).expect("meta").permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(&public_path)
            .expect("meta")
            .permissions()
            .mode()
            & 0o777,
        0o644
    );

    let target = temp.path().join("dict.odc");
    fs::write(&target, b"dictionary bytes").expect("target");
    let target_arg = target.to_str().expect("utf8").to_string();
    let key_arg = key_path.to_str().expect("utf8").to_string();

    let signed = odc()
        .args(["sign", &target_arg, "--key", &key_arg, "--signer", "alice"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let signed = read_json(&signed);
    assert_eq!(signed["version"], 1);
    assert_eq!(signed["algorithm"], "ed25519");
    assert_eq!(signed["signer"], "alice");
    let public_hex = signed["public_key"].as_str().expect("key").to_string();

    let sig_path = temp.path().join("dict.odc.sig");
    assert!(sig_path.exists());
    let sidecar = fs::read_to_string(&sig_path).expect("sidecar");
    assert!(sidecar.ends_with("}\n"));
    assert!(sidecar.contains("\"hash_algorithm\": \"blake3\""));

    let verified = odc()
        .args(["verify-signature", &target_arg, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let verified = read_json(&verified);
    assert_eq!(verified["valid"], true);
    assert_eq!(verified["public_key"], public_hex);
    assert_eq!(verified["size"], 16);
    assert_eq!(verified["signer"], "alice");

    let public_arg = public_path.to_str().expect("utf8").to_string();
    odc()
        .args(["verify-signature", &target_arg, "--pub", &public_arg])
        .assert()
        .success();

    odc()
        .args(["verify-signature", &target_arg, "--pubkey", &public_hex])
        .assert()
        .success();
}

#[test]
fn verify_reports_invalid_target_with_exit_code_one() {
    let temp = TempDir::new().expect("temp");
    let keys = temp.path().join("keys");
    odc()
        .args([
            "keygen",
            "--out",
            keys.to_str().expect("utf8"),
            "--name",
            "a",
        ])
        .assert()
        .success();
    let target = temp.path().join("dict.odc");
    fs::write(&target, b"before").expect("target");
    let target_arg = target.to_str().expect("utf8").to_string();
    let key_arg = keys.join("a.key").to_str().expect("utf8").to_string();
    odc()
        .args(["sign", &target_arg, "--key", &key_arg])
        .assert()
        .success();

    fs::write(&target, b"after!").expect("mutate");
    let output = odc()
        .args(["verify-signature", &target_arg, "--json"])
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();
    let report = read_json(&output);
    assert_eq!(report["valid"], false);
    assert_eq!(report["error_code"], "ODC-E033");

    odc()
        .args(["verify-signature", &target_arg])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("INVALID"));
}

#[test]
fn verify_reports_wrong_key_with_exit_code_one() {
    let temp = TempDir::new().expect("temp");
    let keys = temp.path().join("keys");
    odc()
        .args([
            "keygen",
            "--out",
            keys.to_str().expect("utf8"),
            "--name",
            "a",
        ])
        .assert()
        .success();
    odc()
        .args([
            "keygen",
            "--out",
            keys.to_str().expect("utf8"),
            "--name",
            "b",
        ])
        .assert()
        .success();

    let target = temp.path().join("dict.odc");
    fs::write(&target, b"payload").expect("target");
    let target_arg = target.to_str().expect("utf8").to_string();
    let key_a = keys.join("a.key").to_str().expect("utf8").to_string();
    odc()
        .args(["sign", &target_arg, "--key", &key_a])
        .assert()
        .success();

    let public_b = fs::read_to_string(keys.join("b.pub")).expect("pub");
    let other_hex = public_b.split_whitespace().nth(1).expect("hex").to_string();
    let output = odc()
        .args([
            "verify-signature",
            &target_arg,
            "--pubkey",
            &other_hex,
            "--json",
        ])
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();
    assert_eq!(read_json(&output)["error_code"], "ODC-E034");
}

#[test]
fn verify_usage_and_io_errors_exit_two() {
    let temp = TempDir::new().expect("temp");
    let target = temp.path().join("dict.odc");
    fs::write(&target, b"payload").expect("target");
    let target_arg = target.to_str().expect("utf8").to_string();

    // Missing sidecar.
    odc()
        .args(["verify-signature", &target_arg])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("cannot read"));

    // --pub and --pubkey together.
    odc()
        .args([
            "verify-signature",
            &target_arg,
            "--pubkey",
            &"11".repeat(32),
            "--pub",
            "whatever.pub",
        ])
        .assert()
        .code(2);

    // Malformed --pubkey hex.
    odc()
        .args(["verify-signature", &target_arg, "--pubkey", "abc"])
        .assert()
        .code(2);
}

#[test]
fn keygen_refuses_overwrite_without_force() {
    let temp = TempDir::new().expect("temp");
    let keys = temp.path().join("keys");
    let keys_arg = keys.to_str().expect("utf8").to_string();
    odc()
        .args(["keygen", "--out", &keys_arg, "--name", "dup"])
        .assert()
        .success();
    odc()
        .args(["keygen", "--out", &keys_arg, "--name", "dup"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("already exists"));
    odc()
        .args(["keygen", "--out", &keys_arg, "--name", "dup", "--force"])
        .assert()
        .success();
}

#[test]
fn sign_refuses_overwrite_without_force() {
    let temp = TempDir::new().expect("temp");
    let keys = temp.path().join("keys");
    odc()
        .args(["keygen", "--out", keys.to_str().expect("utf8")])
        .assert()
        .success();
    let target = temp.path().join("dict.odc");
    fs::write(&target, b"payload").expect("target");
    let target_arg = target.to_str().expect("utf8").to_string();
    let key_arg = keys.join("default.key").to_str().expect("utf8").to_string();

    odc()
        .args(["sign", &target_arg, "--key", &key_arg])
        .assert()
        .success();
    odc()
        .args(["sign", &target_arg, "--key", &key_arg])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("already exists"));
    odc()
        .args(["sign", &target_arg, "--key", &key_arg, "--force"])
        .assert()
        .success();
}

#[test]
fn sign_uses_env_key_dir_and_out_override() {
    let temp = TempDir::new().expect("temp");
    let keys = temp.path().join("envkeys");
    odc()
        .env("ODC_KEY_DIR", &keys)
        .args(["keygen"])
        .assert()
        .success();
    assert!(keys.join("default.key").exists());

    let target = temp.path().join("dict.odc");
    fs::write(&target, b"payload").expect("target");
    let target_arg = target.to_str().expect("utf8").to_string();
    let custom = temp.path().join("custom.sig");
    odc()
        .env("ODC_KEY_DIR", &keys)
        .args(["sign", &target_arg, "--out", custom.to_str().expect("utf8")])
        .assert()
        .success();
    assert!(custom.exists());
    assert!(!temp.path().join("dict.odc.sig").exists());

    odc()
        .args([
            "verify-signature",
            &target_arg,
            "--sig",
            custom.to_str().expect("utf8"),
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"valid\": true"));
}

#[test]
fn sign_reports_public_key_on_stdout() {
    let temp = TempDir::new().expect("temp");
    let keys = temp.path().join("keys");
    let output = odc()
        .args([
            "keygen",
            "--out",
            keys.to_str().expect("utf8"),
            "--name",
            "k",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(output).expect("utf8");
    let hex = text
        .lines()
        .find_map(|line| line.strip_prefix("public key hex: "))
        .expect("hex line")
        .to_string();
    assert_eq!(hex.len(), 64);
}
