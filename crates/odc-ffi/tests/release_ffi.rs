//! C ABI coverage for signing, trust, publish and verify-release.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::Path;
use std::ptr;

use odc_core::{Def, Entry, Repo};
use odc_ffi::{
    odc_keypair_generate, odc_patch_apply_signed, odc_publish, odc_publishers_json, odc_repo_open,
    odc_sign_file, odc_string_free, odc_trust_add, odc_trust_remove, odc_verify_release,
    odc_verify_signature_file, OdcRepo,
};
use serde_json::Value;
use tempfile::TempDir;

fn entry(word: &str, sense: &str) -> Entry {
    Entry {
        word: word.to_string(),
        display: word.to_string(),
        phonetic: Vec::new(),
        defs: vec![Def {
            pos: "noun".to_string(),
            sense: sense.to_string(),
            ..Def::default()
        }],
        ..Entry::new(word.to_string())
    }
}

fn seeded_repo() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    let mut repo = Repo::init(dir.path()).expect("init");
    let entries = vec![entry("apple", "a fruit"), entry("banana", "a fruit")];
    repo.import(&entries, false).expect("import");
    repo.commit("initial", "tester", 1_700_000_000)
        .expect("commit");
    dir
}

fn open(dir: &Path) -> *mut OdcRepo {
    let path = CString::new(dir.to_str().expect("utf8")).expect("path");
    let mut error: *mut c_char = ptr::null_mut();
    let repo = unsafe { odc_repo_open(path.as_ptr(), &mut error) };
    assert!(take_error_if_any(error).is_none(), "open failed");
    assert!(!repo.is_null());
    repo
}

fn take_error_if_any(error: *mut c_char) -> Option<String> {
    if error.is_null() {
        return None;
    }
    let message = unsafe { CStr::from_ptr(error) }
        .to_string_lossy()
        .into_owned();
    unsafe { odc_string_free(error) };
    Some(message)
}

fn take_string(value: *mut c_char) -> String {
    assert!(!value.is_null(), "expected a string, got NULL");
    let text = unsafe { CStr::from_ptr(value) }
        .to_str()
        .expect("utf8")
        .to_owned();
    unsafe { odc_string_free(value) };
    text
}

fn take_json(value: *mut c_char) -> Value {
    serde_json::from_str(&take_string(value)).expect("json")
}

#[test]
fn keygen_sign_and_verify_roundtrip() {
    let temp = TempDir::new().expect("tempdir");
    let mut pem: *mut c_char = ptr::null_mut();
    let mut public: *mut c_char = ptr::null_mut();
    let mut error: *mut c_char = ptr::null_mut();
    let status = unsafe { odc_keypair_generate(&mut pem, &mut public, &mut error) };
    assert!(take_error_if_any(error).is_none(), "keygen failed");
    assert_eq!(status, 0);
    let key_pem = take_string(pem);
    let public_hex = take_string(public);
    assert!(key_pem.contains("BEGIN PRIVATE KEY"));
    assert_eq!(public_hex.len(), 64);

    let target = temp.path().join("artifact.bin");
    std::fs::write(&target, b"signed payload").expect("write");
    let path = CString::new(target.to_str().expect("utf8")).expect("path");
    let key = CString::new(key_pem.clone()).expect("pem");
    let signer = CString::new("alice").expect("signer");
    let mut signature_json: *mut c_char = ptr::null_mut();
    let status = unsafe {
        odc_sign_file(
            path.as_ptr(),
            key.as_ptr(),
            signer.as_ptr(),
            &mut signature_json,
            &mut error,
        )
    };
    assert!(take_error_if_any(error).is_none(), "sign failed");
    assert_eq!(status, 0);
    let signature_text = take_string(signature_json);
    let signature: Value = serde_json::from_str(&signature_text).expect("json");
    assert_eq!(signature["public_key"], public_hex);

    let signature_c = CString::new(signature_text.clone()).expect("sig");
    let expected = CString::new(public_hex.clone()).expect("hex");
    let mut verified: *mut c_char = ptr::null_mut();
    let status = unsafe {
        odc_verify_signature_file(
            path.as_ptr(),
            signature_c.as_ptr(),
            expected.as_ptr(),
            &mut verified,
            &mut error,
        )
    };
    assert!(take_error_if_any(error).is_none(), "verify failed");
    assert_eq!(status, 0);
    let verified = take_json(verified);
    assert_eq!(verified["valid"], true);
    assert_eq!(verified["public_key"], public_hex);

    std::fs::write(&target, b"tampered payload").expect("tamper");
    let mut verified: *mut c_char = ptr::null_mut();
    let status = unsafe {
        odc_verify_signature_file(
            path.as_ptr(),
            signature_c.as_ptr(),
            expected.as_ptr(),
            &mut verified,
            &mut error,
        )
    };
    assert!(take_error_if_any(error).is_none());
    assert_eq!(status, 0);
    let verified = take_json(verified);
    assert_eq!(verified["valid"], false);
    assert_eq!(verified["error_code"], "ODC-E033");
}

#[test]
fn trust_add_list_and_remove() {
    let dir = seeded_repo();
    let repo = open(dir.path());
    let mut pem: *mut c_char = ptr::null_mut();
    let mut public: *mut c_char = ptr::null_mut();
    let mut error: *mut c_char = ptr::null_mut();
    let status = unsafe { odc_keypair_generate(&mut pem, &mut public, &mut error) };
    assert!(take_error_if_any(error).is_none());
    assert_eq!(status, 0);
    let _key_pem = take_string(pem);
    let public_hex = take_string(public);
    let key = CString::new(public_hex.clone()).expect("hex");
    let name = CString::new("alice").expect("name");

    let mut added: *mut c_char = ptr::null_mut();
    let status =
        unsafe { odc_trust_add(repo, key.as_ptr(), name.as_ptr(), &mut added, &mut error) };
    assert!(take_error_if_any(error).is_none(), "trust add failed");
    assert_eq!(status, 0);
    let added = take_json(added);
    assert_eq!(added["outcome"], "added");
    assert_eq!(added["publisher"]["source"], "manual");
    assert_eq!(added["publisher"]["name"], "alice");

    let mut listed: *mut c_char = ptr::null_mut();
    let status = unsafe { odc_publishers_json(repo, &mut listed, &mut error) };
    assert!(take_error_if_any(error).is_none());
    assert_eq!(status, 0);
    let listed = take_json(listed);
    assert_eq!(listed["publishers"][0]["public_key"], public_hex);

    let mut removed: *mut c_char = ptr::null_mut();
    let status = unsafe { odc_trust_remove(repo, key.as_ptr(), &mut removed, &mut error) };
    assert!(take_error_if_any(error).is_none());
    assert_eq!(status, 0);
    let removed = take_json(removed);
    assert_eq!(removed["removed"], true);

    let mut again: *mut c_char = ptr::null_mut();
    let status = unsafe { odc_trust_add(repo, key.as_ptr(), ptr::null(), &mut again, &mut error) };
    assert!(take_error_if_any(error).is_none());
    assert_eq!(status, 0);
    let again = take_json(again);
    assert_eq!(again["outcome"], "added");
}

#[test]
fn publish_and_verify_release_roundtrip() {
    let dir = seeded_repo();
    let repo = open(dir.path());
    let out_dir = dir.path().join("release");

    let mut pem: *mut c_char = ptr::null_mut();
    let mut public: *mut c_char = ptr::null_mut();
    let mut error: *mut c_char = ptr::null_mut();
    let status = unsafe { odc_keypair_generate(&mut pem, &mut public, &mut error) };
    assert!(take_error_if_any(error).is_none());
    assert_eq!(status, 0);
    let key_pem = take_string(pem);
    let public_hex = take_string(public);

    let out = CString::new(out_dir.to_str().expect("utf8")).expect("out");
    let name = CString::new("demo").expect("name");
    let label = CString::new("v1").expect("label");
    let key = CString::new(key_pem).expect("pem");
    let signer = CString::new("alice").expect("signer");
    let mut published: *mut c_char = ptr::null_mut();
    let status = unsafe {
        odc_publish(
            repo,
            out.as_ptr(),
            name.as_ptr(),
            label.as_ptr(),
            ptr::null(),
            key.as_ptr(),
            signer.as_ptr(),
            ptr::null(),
            &mut published,
            &mut error,
        )
    };
    assert!(take_error_if_any(error).is_none(), "publish failed");
    assert_eq!(status, 0);
    let published = take_json(published);
    assert_eq!(published["publisher"]["public_key"], public_hex);
    assert_eq!(published["artifacts"].as_array().unwrap().len(), 1);
    assert!(out_dir.join("demo-v1.odc").exists());

    let mut invalid: *mut c_char = ptr::null_mut();
    let status = unsafe {
        odc_publish(
            repo,
            out.as_ptr(),
            name.as_ptr(),
            label.as_ptr(),
            ptr::null(),
            key.as_ptr(),
            signer.as_ptr(),
            ptr::null(),
            &mut invalid,
            &mut error,
        )
    };
    assert_eq!(status, -1);
    let message = take_error_if_any(error).expect("error message");
    assert!(message.contains("ODC-E016"), "unexpected error: {message}");

    let expected = CString::new(public_hex.clone()).expect("hex");
    let mut verified: *mut c_char = ptr::null_mut();
    let status =
        unsafe { odc_verify_release(out.as_ptr(), expected.as_ptr(), &mut verified, &mut error) };
    assert!(take_error_if_any(error).is_none());
    assert_eq!(status, 0);
    let verified = take_json(verified);
    assert_eq!(verified["valid"], true);
    assert_eq!(verified["publisher"]["public_key"], public_hex);

    let mut other_pem: *mut c_char = ptr::null_mut();
    let mut other_public: *mut c_char = ptr::null_mut();
    let status = unsafe { odc_keypair_generate(&mut other_pem, &mut other_public, &mut error) };
    assert!(take_error_if_any(error).is_none());
    assert_eq!(status, 0);
    let _other_key = take_string(other_pem);
    let other_hex = CString::new(take_string(other_public)).expect("hex");

    let mut rejected: *mut c_char = ptr::null_mut();
    let status =
        unsafe { odc_verify_release(out.as_ptr(), other_hex.as_ptr(), &mut rejected, &mut error) };
    assert!(take_error_if_any(error).is_none());
    assert_eq!(status, 0);
    let rejected = take_json(rejected);
    assert_eq!(rejected["valid"], false);
    assert_eq!(rejected["error_code"], "ODC-E036");
}

#[test]
fn patch_apply_signed_enforces_trust() {
    let dir = TempDir::new().expect("tempdir");
    let source = dir.path().join("source");
    let mirror = dir.path().join("mirror");
    let entries = vec![entry("apple", "a fruit"), entry("banana", "a fruit")];

    let mut src = Repo::init(&source).expect("init source");
    src.import(&entries, false).expect("import source");
    src.commit("initial", "tester", 1_700_000_000)
        .expect("commit source");
    let base = src.head().expect("head").expect("commit");

    let mut target = Repo::init(&mirror).expect("init mirror");
    target.import(&entries, false).expect("import mirror");
    target
        .commit("initial", "tester", 1_700_000_000)
        .expect("commit mirror");
    drop(target);

    src.import(&[entry("cherry", "a fruit")], false)
        .expect("import second");
    src.commit("second", "tester", 1_700_000_100)
        .expect("commit second");
    let plan = odc_core::patch::create(&src, &odc_core::hash::to_hex(&base), "HEAD").expect("plan");
    let bytes = odc_core::patch::encode(&plan).expect("encode");
    let patch_path = dir.path().join("fix.odp");
    std::fs::write(&patch_path, &bytes).expect("write patch");

    let mut pem: *mut c_char = ptr::null_mut();
    let mut public: *mut c_char = ptr::null_mut();
    let mut error: *mut c_char = ptr::null_mut();
    let status = unsafe { odc_keypair_generate(&mut pem, &mut public, &mut error) };
    assert!(take_error_if_any(error).is_none());
    assert_eq!(status, 0);
    let key_pem = take_string(pem);
    let _public_hex = take_string(public);

    let patch_c = CString::new(patch_path.to_str().expect("utf8")).expect("path");
    let key_c = CString::new(key_pem).expect("pem");
    let signer_c = CString::new("alice").expect("signer");
    let mut signature_json: *mut c_char = ptr::null_mut();
    let status = unsafe {
        odc_sign_file(
            patch_c.as_ptr(),
            key_c.as_ptr(),
            signer_c.as_ptr(),
            &mut signature_json,
            &mut error,
        )
    };
    assert!(take_error_if_any(error).is_none());
    assert_eq!(status, 0);
    let sig_path = dir.path().join("fix.odp.sig");
    std::fs::write(&sig_path, take_string(signature_json)).expect("write sig");
    let sig_c = CString::new(sig_path.to_str().expect("utf8")).expect("path");

    let before = Repo::open(&mirror)
        .expect("reopen mirror")
        .head()
        .expect("head")
        .expect("commit");
    let repo = open(&mirror);
    let mut out: *mut c_char = ptr::null_mut();
    let status = unsafe {
        odc_patch_apply_signed(
            repo,
            patch_c.as_ptr(),
            sig_c.as_ptr(),
            0,
            &mut out,
            &mut error,
        )
    };
    assert_eq!(status, -1);
    let message = take_error_if_any(error).expect("error message");
    assert!(message.contains("ODC-E036"), "unexpected error: {message}");
    let after = Repo::open(&mirror)
        .expect("reopen mirror twice")
        .head()
        .expect("head")
        .expect("commit");
    assert_eq!(after, before, "rejected patch must not move HEAD");

    let mut applied: *mut c_char = ptr::null_mut();
    let status = unsafe {
        odc_patch_apply_signed(
            repo,
            patch_c.as_ptr(),
            sig_c.as_ptr(),
            1,
            &mut applied,
            &mut error,
        )
    };
    assert!(take_error_if_any(error).is_none(), "apply with tofu failed");
    assert_eq!(status, 0);
    let applied = take_json(applied);
    assert_eq!(applied["applied"], true);
}
