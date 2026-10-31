//! Exercises the C ABI the way a C caller would: raw pointers, error out
//! parameters and manual string ownership.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;

use odc_core::cbor::Value as CborValue;
use odc_core::{Def, Entry, Repo};
use odc_ffi::{
    odc_repo_close, odc_repo_entry_count, odc_repo_entry_json, odc_repo_open, odc_repo_search_json,
    odc_string_free, odc_version,
};
use serde_json::Value;
use tempfile::TempDir;

fn entry(word: &str, display: &str, sense: &str) -> Entry {
    Entry {
        word: word.to_string(),
        display: display.to_string(),
        phonetic: vec!["\u{2c8}\u{00e6}p.\u{0259}l".to_string()],
        defs: vec![Def {
            pos: "noun".to_string(),
            sense: sense.to_string(),
            examples: vec!["an apple a day".to_string()],
            labels: vec!["informal".to_string()],
        }],
        raw_html: None,
        extra: Vec::new(),
    }
}

fn seeded_repo() -> TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut repo = Repo::init(dir.path()).expect("init repo");
    let mut apple = entry("apple", "Apple", "a fruit");
    apple.extra = vec![("source".to_string(), CborValue::Text("testkit".to_string()))];
    repo.import(
        &[
            apple,
            entry("application", "application", "a program"),
            entry("caf\u{00e9}", "caf\u{00e9}", "coffee house"),
            entry(
                "\u{65e5}\u{672c}\u{8a9e}",
                "\u{65e5}\u{672c}\u{8a9e}",
                "Japanese",
            ),
        ],
        false,
    )
    .expect("import");
    repo.commit("seed", "test", 1_700_000_000).expect("commit");
    dir
}

fn open(dir: &TempDir) -> *mut odc_ffi::OdcRepo {
    let path = CString::new(dir.path().to_str().expect("utf8 path")).expect("path");
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: `path` is a valid NUL-terminated string and `error` is a
    // writable slot for one pointer.
    let repo = unsafe { odc_repo_open(path.as_ptr(), &mut error) };
    if let Some(message) = take_error_if_any(error) {
        panic!("unexpected open error: {message}");
    }
    assert!(!repo.is_null(), "open returned NULL without an error");
    repo
}

fn take_error_if_any(error: *mut c_char) -> Option<String> {
    if error.is_null() {
        return None;
    }
    // SAFETY: errors are NUL-terminated strings owned by us.
    let message = unsafe { CStr::from_ptr(error) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: the pointer came from this library and is freed once.
    unsafe { odc_string_free(error) };
    Some(message)
}

fn take_string(value: *mut c_char) -> String {
    assert!(!value.is_null(), "expected a string, got NULL");
    // SAFETY: non-NULL result from this library, NUL-terminated.
    let text = unsafe { CStr::from_ptr(value) }
        .to_str()
        .expect("utf8 result")
        .to_owned();
    // SAFETY: the pointer came from this library and is freed once.
    unsafe { odc_string_free(value) };
    text
}

fn count(repo: *const odc_ffi::OdcRepo) -> i64 {
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: `repo` is live for the duration of the call.
    let total = unsafe { odc_repo_entry_count(repo, &mut error) };
    assert!(take_error_if_any(error).is_none(), "count failed");
    total
}

fn search(repo: *const odc_ffi::OdcRepo, query: &str, limit: i32) -> Value {
    let query = CString::new(query).expect("query");
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: all pointers are valid for the call.
    let payload = unsafe { odc_repo_search_json(repo, query.as_ptr(), limit, &mut error) };
    assert!(take_error_if_any(error).is_none(), "search failed");
    serde_json::from_str(&take_string(payload)).expect("json array")
}

#[test]
fn open_count_search_entry_and_close() {
    let dir = seeded_repo();
    let repo = open(&dir);

    assert_eq!(count(repo), 4);

    let results = search(repo, "app", 10);
    let keys: Vec<&str> = results
        .as_array()
        .expect("array")
        .iter()
        .map(|item| item["key"].as_str().expect("key"))
        .collect();
    assert_eq!(keys, vec!["apple", "application"]);
    assert_eq!(results[0]["display"], "Apple");

    let key = CString::new("apple").expect("key");
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: all pointers are valid for the call.
    let payload = unsafe { odc_repo_entry_json(repo, key.as_ptr(), &mut error) };
    assert!(take_error_if_any(error).is_none(), "entry failed");
    let entry: Value = serde_json::from_str(&take_string(payload)).expect("json object");
    assert_eq!(entry["key"], "apple");
    assert_eq!(entry["display"], "Apple");
    assert_eq!(entry["defs"][0]["sense"], "a fruit");
    assert_eq!(entry["defs"][0]["pos"], "noun");
    assert_eq!(entry["defs"][0]["labels"][0], "informal");
    assert_eq!(entry["extra"]["source"], "testkit");
    assert!(entry["raw_html"].is_null());

    // SAFETY: the handle is closed exactly once and not used afterwards.
    unsafe { odc_repo_close(repo) };
}

#[test]
fn search_limit_is_bounded() {
    let dir = seeded_repo();
    let repo = open(&dir);
    let query = CString::new("app").expect("query");

    for limit in [0, -1, 201] {
        let mut error: *mut c_char = ptr::null_mut();
        // SAFETY: all pointers are valid for the call.
        let payload = unsafe { odc_repo_search_json(repo, query.as_ptr(), limit, &mut error) };
        assert!(payload.is_null(), "limit {limit} should fail");
        let message = take_error_if_any(error).expect("error message");
        assert!(message.contains("limit"), "unexpected message: {message}");
    }

    // SAFETY: single close.
    unsafe { odc_repo_close(repo) };
}

#[test]
fn search_rejects_empty_query() {
    let dir = seeded_repo();
    let repo = open(&dir);
    let query = CString::new("").expect("query");
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: all pointers are valid for the call.
    let payload = unsafe { odc_repo_search_json(repo, query.as_ptr(), 10, &mut error) };
    assert!(payload.is_null());
    let message = take_error_if_any(error).expect("error message");
    assert!(message.contains("empty"), "unexpected message: {message}");
    // SAFETY: single close.
    unsafe { odc_repo_close(repo) };
}

#[test]
fn open_rejects_missing_directory() {
    let mut error: *mut c_char = ptr::null_mut();
    let path = CString::new("/tmp/odc-ffi-missing-repository").expect("path");
    // SAFETY: `path` is a valid NUL-terminated string.
    let repo = unsafe { odc_repo_open(path.as_ptr(), &mut error) };
    assert!(repo.is_null());
    let message = take_error_if_any(error).expect("error message");
    assert!(!message.is_empty());
}

#[test]
fn open_rejects_invalid_utf8() {
    let mut error: *mut c_char = ptr::null_mut();
    let path = CString::new(vec![0xff_u8]).expect("cstring");
    // SAFETY: the pointer is NUL-terminated; the bytes are not valid UTF-8,
    // which the implementation must report instead of panicking.
    let repo = unsafe { odc_repo_open(path.as_ptr(), &mut error) };
    assert!(repo.is_null());
    let message = take_error_if_any(error).expect("error message");
    assert!(message.contains("UTF-8"), "unexpected message: {message}");
}

#[test]
fn entry_reports_missing_key() {
    let dir = seeded_repo();
    let repo = open(&dir);
    let key = CString::new("zebra").expect("key");
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: all pointers are valid for the call.
    let payload = unsafe { odc_repo_entry_json(repo, key.as_ptr(), &mut error) };
    assert!(payload.is_null());
    let message = take_error_if_any(error).expect("error message");
    assert!(
        message.contains("not found"),
        "unexpected message: {message}"
    );
    // SAFETY: single close.
    unsafe { odc_repo_close(repo) };
}

#[test]
fn unicode_prefix_search_matches_case_folded_key() {
    let dir = seeded_repo();
    let repo = open(&dir);

    let results = search(repo, "CAF\u{00c9}", 10);
    let keys: Vec<&str> = results
        .as_array()
        .expect("array")
        .iter()
        .map(|item| item["key"].as_str().expect("key"))
        .collect();
    assert_eq!(keys, vec!["caf\u{00e9}"]);

    let results = search(repo, "\u{65e5}\u{672c}", 10);
    assert_eq!(results[0]["key"], "\u{65e5}\u{672c}\u{8a9e}");

    // SAFETY: single close.
    unsafe { odc_repo_close(repo) };
}

#[test]
fn null_handles_are_rejected_without_crashing() {
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: passing NULL is explicitly supported.
    let total = unsafe { odc_repo_entry_count(ptr::null(), &mut error) };
    assert_eq!(total, 0);
    let message = take_error_if_any(error).expect("error message");
    assert!(message.contains("NULL"), "unexpected message: {message}");

    let query = CString::new("app").expect("query");
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: NULL handle plus valid query pointer must not crash.
    let payload = unsafe { odc_repo_search_json(ptr::null(), query.as_ptr(), 10, &mut error) };
    assert!(payload.is_null());
    assert!(take_error_if_any(error).is_some());

    // SAFETY: NULL is a documented no-op for both functions.
    unsafe {
        odc_repo_close(ptr::null_mut());
        odc_string_free(ptr::null_mut());
    }
}

#[test]
fn version_matches_package_version() {
    // SAFETY: `odc_version` returns a pointer to static storage.
    let version = unsafe { CStr::from_ptr(odc_version()) }
        .to_str()
        .expect("utf8 version");
    assert_eq!(version, env!("CARGO_PKG_VERSION"));
}
