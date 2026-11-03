//! Release bundles and the publisher trust store.

use odc_core::entry::{Def, Entry};
use odc_core::release::{self, PublishOptions};
use odc_core::repo::{Repo, TrustOutcome};
use odc_core::signing;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let dir = std::env::temp_dir().join(format!(
        "odc-release-{label}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn cleanup(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
}

fn entry(index: usize) -> Entry {
    let mut item = Entry::new(format!("word-{index:06}"));
    item.defs.push(Def::new(format!("definition {index:06}")));
    item
}

fn seed(dir: &Path, count: usize, message: &str, ts: i64) -> Repo {
    let mut repo = Repo::init(dir).expect("init");
    let entries: Vec<Entry> = (0..count).map(entry).collect();
    repo.import(&entries, true).expect("import");
    repo.commit(message, "tester", ts).expect("commit");
    repo
}

fn keypair() -> (String, [u8; 32]) {
    let pair = signing::generate_keypair().expect("keypair");
    let key = signing::hex_bytes::<32>(&pair.public_key_hex, "public_key").expect("hex");
    (pair.private_key_pem, key)
}

#[test]
fn trust_store_crud_is_idempotent() {
    let dir = temp_dir("trust-crud");
    let mut repo = seed(&dir, 2, "initial", 1_700_000_000);
    let (_, key) = keypair();
    let hex = signing::hex_encode(&key);

    let first = repo
        .trust_publisher(&hex, "alice", "manual")
        .expect("trust");
    match first {
        TrustOutcome::Added(publisher) => {
            assert_eq!(publisher.public_key, hex);
            assert_eq!(publisher.name, "alice");
            assert_eq!(publisher.source, "manual");
            assert!(publisher.added_at > 0);
        }
        other => panic!("unexpected outcome: {other:?}"),
    }
    assert!(repo.is_trusted(&hex).expect("is_trusted"));
    assert!(!repo.trust_store_is_empty().expect("empty"));
    assert_eq!(repo.publishers().expect("list").len(), 1);

    let again = repo
        .trust_publisher(&hex, "alice", "manual")
        .expect("trust again");
    assert!(matches!(again, TrustOutcome::AlreadyTrusted(_)));
    assert_eq!(repo.publishers().expect("list").len(), 1);

    assert!(repo.remove_publisher(&hex).expect("remove"));
    assert!(!repo.remove_publisher(&hex).expect("remove again"));
    assert!(repo.trust_store_is_empty().expect("empty again"));
    cleanup(&dir);
}

#[test]
fn trust_name_conflict_is_reported() {
    let dir = temp_dir("trust-conflict");
    let mut repo = seed(&dir, 1, "initial", 1_700_000_000);
    let (_, first) = keypair();
    let (_, second) = keypair();
    repo.trust_publisher(&signing::hex_encode(&first), "alice", "manual")
        .expect("trust first");
    let outcome = repo
        .trust_publisher(&signing::hex_encode(&second), "alice", "manual")
        .expect("second");
    match outcome {
        TrustOutcome::NameConflict { existing } => {
            assert_eq!(existing.public_key, signing::hex_encode(&first));
        }
        other => panic!("unexpected outcome: {other:?}"),
    }
    cleanup(&dir);
}

#[test]
fn old_database_gains_the_publishers_table() {
    let dir = temp_dir("trust-migrate");
    let _repo = seed(&dir, 1, "initial", 1_700_000_000);
    let db = dir.join("odc.db");
    {
        let connection = rusqlite::Connection::open(&db).expect("db");
        connection
            .execute_batch(
                "DROP TABLE publishers;
                 UPDATE meta SET value = x'32' WHERE key = 'schema_version';",
            )
            .expect("downgrade");
    }
    let mut repo = Repo::open(&dir).expect("reopen");
    let (_, key) = keypair();
    let outcome = repo
        .trust_publisher(&signing::hex_encode(&key), "", "manual")
        .expect("trust after migration");
    assert!(matches!(outcome, TrustOutcome::Added(_)));
    let connection = rusqlite::Connection::open(&db).expect("db");
    let version: Vec<u8> = connection
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .expect("version");
    assert_eq!(String::from_utf8_lossy(&version), "3");
    cleanup(&dir);
}

#[test]
fn tofu_policy_covers_three_branches() {
    let dir = temp_dir("trust-tofu");
    let mut repo = seed(&dir, 1, "initial", 1_700_000_000);
    let (_, key) = keypair();
    let hex = signing::hex_encode(&key);

    let error = release::enforce_publisher_trust(&mut repo, &hex, None, false)
        .expect_err("unknown without trust-new");
    assert_eq!(error.code(), "ODC-E036");

    let publisher =
        release::enforce_publisher_trust(&mut repo, &hex, Some("alice"), true).expect("tofu");
    assert_eq!(publisher.source, "tofu");
    assert_eq!(publisher.name, "alice");

    let (_, other) = keypair();
    let other_hex = signing::hex_encode(&other);
    let error = release::enforce_publisher_trust(&mut repo, &other_hex, None, true)
        .expect_err("non-empty store rejects a new key");
    assert_eq!(error.code(), "ODC-E036");

    let trusted =
        release::enforce_publisher_trust(&mut repo, &hex, None, false).expect("already trusted");
    assert_eq!(trusted.public_key, hex);
    cleanup(&dir);
}

#[test]
fn publish_and_verify_roundtrip() {
    let dir = temp_dir("publish");
    let mut repo = seed(&dir, 5, "initial", 1_700_000_000);
    let first_head = repo.head().expect("head").expect("commit");
    repo.import(&[entry(99)], false).expect("import second");
    repo.commit("second", "tester", 1_700_000_100)
        .expect("commit second");
    let second_head = repo.head().expect("head").expect("commit");
    assert_ne!(first_head, second_head);

    let out = dir.join("release");
    let (key_pem, key) = keypair();
    let options = PublishOptions {
        name: "demo".to_string(),
        label: "2026.11".to_string(),
        base: Some(signing::hex_encode(&first_head)),
        key_pem,
        signer: Some("alice".to_string()),
        notes: Some("release notes".to_string()),
        force: false,
    };
    let report = release::publish(&repo, &out, &options).expect("publish");
    assert_eq!(report.head, signing::hex_encode(&second_head));
    assert_eq!(report.base, Some(signing::hex_encode(&first_head)));
    assert_eq!(report.artifacts.len(), 2);
    assert_eq!(report.public_key, signing::hex_encode(&key));
    assert!(out.join("demo-2026.11.odc").exists());
    assert!(out.join("demo-2026.11.odp").exists());
    assert!(out.join("demo-2026.11.odc.sig").exists());
    assert!(out.join("release.json").exists());
    assert!(out.join("release.json.sig").exists());

    let manifest_text = fs::read_to_string(out.join("release.json")).expect("manifest");
    assert!(manifest_text.ends_with('\n'));
    let manifest: serde_json::Value = serde_json::from_str(&manifest_text).expect("json");
    assert_eq!(manifest["format"], "odc-release");
    assert_eq!(manifest["version"], 1);
    assert_eq!(manifest["name"], "demo");
    assert_eq!(manifest["label"], "2026.11");
    assert_eq!(manifest["artifacts"][0]["kind"], "pack");
    assert_eq!(manifest["artifacts"][1]["kind"], "patch");
    assert_eq!(
        manifest["publisher"]["public_key"],
        signing::hex_encode(&key)
    );

    let verified = release::verify_release(&out, Some(&key)).expect("verify");
    assert_eq!(verified.head, signing::hex_encode(&second_head));
    assert_eq!(verified.base, Some(signing::hex_encode(&first_head)));
    assert_eq!(verified.name, "demo");
    assert_eq!(verified.label, "2026.11");
    assert_eq!(verified.publisher_key, signing::hex_encode(&key));
    assert_eq!(verified.publisher_name.as_deref(), Some("alice"));
    assert_eq!(verified.artifacts.len(), 2);

    let other = keypair();
    let error = release::verify_release(&out, Some(&other.1)).expect_err("wrong pin");
    assert_eq!(error.code(), "ODC-E036");
    cleanup(&dir);
}

#[test]
fn publish_refuses_to_overwrite_without_force() {
    let dir = temp_dir("publish-force");
    let repo = seed(&dir, 2, "initial", 1_700_000_000);
    let out = dir.join("release");
    let (key_pem, _) = keypair();
    let options = PublishOptions {
        name: "demo".to_string(),
        label: "v1".to_string(),
        base: None,
        key_pem: key_pem.clone(),
        signer: None,
        notes: None,
        force: false,
    };
    release::publish(&repo, &out, &options).expect("first publish");
    let error = release::publish(&repo, &out, &options).expect_err("second publish");
    assert_eq!(error.code(), "ODC-E016");

    let forced = PublishOptions {
        force: true,
        ..options
    };
    release::publish(&repo, &out, &forced).expect("forced publish");
    cleanup(&dir);
}

#[test]
fn tampered_artifact_is_rejected() {
    let dir = temp_dir("publish-artifact");
    let repo = seed(&dir, 2, "initial", 1_700_000_000);
    let out = dir.join("release");
    let (key_pem, key) = keypair();
    let options = PublishOptions {
        name: "demo".to_string(),
        label: "v1".to_string(),
        base: None,
        key_pem,
        signer: None,
        notes: None,
        force: false,
    };
    release::publish(&repo, &out, &options).expect("publish");
    let pack = out.join("demo-v1.odc");
    let mut bytes = fs::read(&pack).expect("pack");
    bytes.push(0);
    fs::write(&pack, &bytes).expect("tamper");

    let error = release::verify_release(&out, Some(&key)).expect_err("tampered pack");
    assert_eq!(error.code(), "ODC-E038");

    fs::remove_file(&pack).expect("remove");
    let error = release::verify_release(&out, Some(&key)).expect_err("missing pack");
    assert_eq!(error.code(), "ODC-E038");
    cleanup(&dir);
}

#[test]
fn tampered_manifest_is_rejected() {
    let dir = temp_dir("publish-manifest");
    let repo = seed(&dir, 2, "initial", 1_700_000_000);
    let out = dir.join("release");
    let (key_pem, key) = keypair();
    let options = PublishOptions {
        name: "demo".to_string(),
        label: "v1".to_string(),
        base: None,
        key_pem,
        signer: None,
        notes: None,
        force: false,
    };
    release::publish(&repo, &out, &options).expect("publish");
    let manifest = out.join("release.json");
    let mut text = fs::read_to_string(&manifest).expect("manifest");
    text.push(' ');
    fs::write(&manifest, text).expect("tamper");

    let error = release::verify_release(&out, Some(&key)).expect_err("tampered manifest");
    assert_eq!(error.code(), "ODC-E037");
    cleanup(&dir);
}

#[test]
fn malformed_manifests_are_rejected() {
    let dir = temp_dir("manifest-bad");
    let repo = seed(&dir, 1, "initial", 1_700_000_000);
    let out = dir.join("release");
    let (key_pem, key) = keypair();
    let options = PublishOptions {
        name: "demo".to_string(),
        label: "v1".to_string(),
        base: None,
        key_pem,
        signer: None,
        notes: None,
        force: false,
    };
    release::publish(&repo, &out, &options).expect("publish");
    let manifest = out.join("release.json");
    let original: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest).expect("manifest")).expect("json");

    let mut wrong_version = original.clone();
    wrong_version["version"] = serde_json::json!(2);
    fs::write(&manifest, wrong_version.to_string()).expect("write");
    let error = release::verify_release(&out, Some(&key)).expect_err("bad version");
    assert_eq!(error.code(), "ODC-E037");

    let mut bad_key = original;
    bad_key["publisher"]["public_key"] = serde_json::json!("zzzz");
    fs::write(&manifest, bad_key.to_string()).expect("write");
    let error = release::verify_release(&out, Some(&key)).expect_err("bad publisher");
    assert_eq!(error.code(), "ODC-E037");
    cleanup(&dir);
}
