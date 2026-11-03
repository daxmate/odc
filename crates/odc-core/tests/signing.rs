//! Signing primitives: roundtrips, tamper detection and the fixed vector.

use odc_core::signing::{self, SignatureTargetV1, SignatureV1};
use odc_core::{hash, Error};
use serde_json::Value;
use std::path::PathBuf;

fn vector_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spec/testvectors")
}

fn vector() -> Value {
    let path = vector_dir().join("signature_ed25519.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    serde_json::from_str(&text).expect("valid json")
}

fn keypair() -> (String, [u8; 32]) {
    let generated = signing::generate_keypair().expect("keygen");
    (
        generated.private_key_pem,
        signing::hex_bytes::<32>(&generated.public_key_hex, "public_key").expect("hex"),
    )
}

fn fixed_signature(public_key: &str, signature: &str, digest: &str, size: u64) -> SignatureV1 {
    SignatureV1 {
        version: 1,
        algorithm: "ed25519".to_string(),
        public_key: public_key.to_string(),
        signature: signature.to_string(),
        target: SignatureTargetV1 {
            hash_algorithm: "blake3".to_string(),
            hash: digest.to_string(),
            size,
        },
        created_at: "2026-11-03T12:00:00Z".to_string(),
        signer: Some("vector".to_string()),
    }
}

#[test]
fn vector_matches_fixed_ed25519_output() {
    let vector = vector();
    let pem = vector["private_key_pem"].as_str().expect("pem");
    let target = vector["target_utf8"].as_str().expect("target").as_bytes();
    let expected_key = vector["public_key_hex"].as_str().expect("key");
    let expected_hash = vector["blake3_hex"].as_str().expect("hash");
    let expected_signature = vector["signature_hex"].as_str().expect("signature");

    let signature = signing::sign_bytes(target, pem, Some("vector")).expect("sign");
    assert_eq!(signature.public_key, expected_key);
    assert_eq!(signature.target.hash, expected_hash);
    assert_eq!(signature.target.size, target.len() as u64);
    assert_eq!(
        signature.target.size,
        vector["target_size"].as_u64().expect("size")
    );
    assert_eq!(signature.signature, expected_signature);

    let pinned = signing::hex_bytes::<32>(expected_key, "public_key").expect("hex");
    let verified =
        signing::verify_bytes_with_key(target, &signature, Some(&pinned)).expect("verify");
    assert_eq!(verified.public_key, pinned);
    assert_eq!(hash::to_hex(&verified.hash), expected_hash);
    assert_eq!(verified.size, target.len() as u64);
}

#[test]
fn vector_signature_verifies_from_json() {
    let vector = vector();
    let target = vector["target_utf8"].as_str().expect("target").as_bytes();
    let sidecar = fixed_signature(
        vector["public_key_hex"].as_str().expect("key"),
        vector["signature_hex"].as_str().expect("signature"),
        vector["blake3_hex"].as_str().expect("hash"),
        vector["target_size"].as_u64().expect("size"),
    );
    let text = sidecar.to_json().expect("json");
    let parsed = SignatureV1::from_json(&text).expect("parse");
    assert_eq!(parsed, sidecar);
    signing::verify_bytes(target, &parsed).expect("verify");
}

#[test]
fn roundtrip_sign_and_verify() {
    let (pem, public_key) = keypair();
    let data = b"round trip payload";
    let signature = signing::sign_bytes(data, &pem, Some("alice")).expect("sign");
    assert_eq!(signature.signer.as_deref(), Some("alice"));
    let verified = signing::verify_bytes(data, &signature).expect("verify");
    assert_eq!(verified.public_key, public_key);
    assert_eq!(verified.signer.as_deref(), Some("alice"));
    assert_eq!(verified.size, data.len() as u64);
}

#[test]
fn tampering_one_byte_reports_e033() {
    let (pem, _) = keypair();
    let mut data = b"tamper me".to_vec();
    let signature = signing::sign_bytes(&data, &pem, None).expect("sign");
    data[0] ^= 0x01;
    let error = signing::verify_bytes(&data, &signature).expect_err("must fail");
    assert_eq!(error.code(), "ODC-E033");
    assert!(matches!(error, Error::SignatureTargetMismatch(_)));
}

#[test]
fn size_mismatch_reports_e033() {
    let (pem, _) = keypair();
    let data = b"size mismatch";
    let mut signature = signing::sign_bytes(data, &pem, None).expect("sign");
    signature.target.size += 1;
    let error = signing::verify_bytes(data, &signature).expect_err("must fail");
    assert_eq!(error.code(), "ODC-E033");
}

#[test]
fn swapped_public_key_reports_e034() {
    let (pem, _) = keypair();
    let (_, other_key) = keypair();
    let data = b"swap the key";
    let mut signature = signing::sign_bytes(data, &pem, None).expect("sign");
    signature.public_key = signing::hex_encode(&other_key);
    let error = signing::verify_bytes(data, &signature).expect_err("must fail");
    assert_eq!(error.code(), "ODC-E034");
    assert!(matches!(error, Error::SignatureInvalid(_)));
}

#[test]
fn pinned_key_mismatch_reports_e034() {
    let (pem, _) = keypair();
    let (_, other_key) = keypair();
    let data = b"pinned";
    let signature = signing::sign_bytes(data, &pem, None).expect("sign");
    let error =
        signing::verify_bytes_with_key(data, &signature, Some(&other_key)).expect_err("must fail");
    assert_eq!(error.code(), "ODC-E034");
}

#[test]
fn broken_json_and_hex_report_e032() {
    for text in [
        "",
        "{}",
        "{\"version\":1}",
        "not json at all",
        "{\"version\":\"one\"}",
    ] {
        let error = SignatureV1::from_json(text).expect_err("must fail");
        assert_eq!(error.code(), "ODC-E032", "input {text:?}");
    }

    let (pem, public_key) = keypair();
    let data = b"hex damage";
    let signature = signing::sign_bytes(data, &pem, None).expect("sign");
    let mut short = signature.clone();
    short.signature.pop();
    assert_eq!(
        signing::verify_bytes(data, &short)
            .expect_err("must fail")
            .code(),
        "ODC-E032"
    );
    let mut upper = signature.clone();
    upper.public_key = upper.public_key.to_uppercase();
    assert_eq!(
        signing::verify_bytes(data, &upper)
            .expect_err("must fail")
            .code(),
        "ODC-E032"
    );
    let mut bad_version = signature;
    bad_version.version = 2;
    assert_eq!(
        bad_version.to_json().expect_err("must fail").code(),
        "ODC-E032"
    );
    assert_eq!(public_key.len(), 32);
}

#[test]
fn bad_private_key_reports_e035() {
    let error = signing::sign_bytes(b"payload", "not a private key", None).expect_err("must fail");
    assert_eq!(error.code(), "ODC-E035");
    assert!(matches!(error, Error::KeyFormat(_)));

    let (pem, _) = keypair();
    let mut corrupted = pem.clone();
    corrupted.push_str("garbage");
    let error = signing::sign_bytes(b"payload", &corrupted, None).expect_err("must fail");
    assert_eq!(error.code(), "ODC-E035");
}

#[test]
fn sidecar_json_is_canonical() {
    let (pem, _) = keypair();
    let signature = signing::sign_bytes(b"canonical", &pem, None).expect("sign");
    let text = signature.to_json().expect("json");
    assert!(text.ends_with("}\n"));
    assert!(text.contains("\n  \"version\": 1,"));
    assert!(!text.contains("signer"), "signer omitted when absent");

    let with_signer = signing::sign_bytes(b"canonical", &pem, Some("bob")).expect("sign");
    let text = with_signer.to_json().expect("json");
    assert!(text.contains("\"signer\": \"bob\""));
    assert!(!text.contains("null"));
}

#[test]
fn sidecar_path_appends_suffix() {
    assert_eq!(
        signing::signature_path(std::path::Path::new("/tmp/dict.odc")),
        PathBuf::from("/tmp/dict.odc.sig")
    );
}

#[test]
fn malformed_inputs_never_panic() {
    let cases = [
        "{\"version\":1,\"algorithm\":\"ed25519\",\"public_key\":\"00\",\"signature\":\"00\",\"target\":{\"hash_algorithm\":\"blake3\",\"hash\":\"00\",\"size\":-1},\"created_at\":\"x\"}",
        "{\"version\":1,\"algorithm\":\"ed25519\",\"public_key\":\"zz\",\"signature\":\"zz\",\"target\":{\"hash_algorithm\":\"blake3\",\"hash\":\"zz\",\"size\":1},\"created_at\":\"2026-11-03T12:00:00Z\"}",
        "{\"version\":1,\"algorithm\":\"ed25519\",\"public_key\":\"\",\"signature\":\"\",\"target\":{},\"created_at\":\"2026-11-03T12:00:00Z\"}",
    ];
    for case in cases {
        let error = SignatureV1::from_json(case).expect_err("must fail");
        assert_eq!(error.code(), "ODC-E032");
    }
}
