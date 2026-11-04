//! Exercises the C ABI the way a C caller would: raw pointers, error out
//! parameters and manual string ownership.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::path::Path;
use std::ptr;
use std::sync::Mutex;

use odc_core::cbor::Value as CborValue;
use odc_core::mdx_scan::{scan, ScanOutcome};
use odc_core::{Def, Entry, Repo};
use odc_ffi::{
    odc_copy_pack_to_repo, odc_html_process_json, odc_import_mdx_to_odc,
    odc_import_mdx_to_odc_with_mdd, odc_import_mdx_to_odc_with_mdd_progress,
    odc_import_mdx_to_repo_with_mdd_progress, odc_repo_close, odc_repo_commit_json,
    odc_repo_discard_json, odc_repo_entries_page_json, odc_repo_entry_count,
    odc_repo_entry_delete_json, odc_repo_entry_diff_json, odc_repo_entry_json,
    odc_repo_entry_link_json, odc_repo_entry_link_working_json, odc_repo_entry_put_json,
    odc_repo_open, odc_repo_resource_copy, odc_repo_resource_count, odc_repo_resource_len,
    odc_repo_search_json, odc_repo_search_substring_json, odc_repo_working_changes_json,
    odc_repo_working_status_json, odc_string_free, odc_version,
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

fn search_substring(repo: *const odc_ffi::OdcRepo, query: &str, limit: i32) -> Value {
    let query = CString::new(query).expect("query");
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: all pointers are valid for the call.
    let payload =
        unsafe { odc_repo_search_substring_json(repo, query.as_ptr(), limit, &mut error) };
    assert!(
        take_error_if_any(error).is_none(),
        "substring search failed"
    );
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
fn substring_search_returns_matches_and_rejects_bad_limits() {
    let dir = seeded_repo();
    let repo = open(&dir);

    let results = search_substring(repo, "ppl", 10);
    let keys: Vec<&str> = results
        .as_array()
        .expect("array")
        .iter()
        .map(|item| item["key"].as_str().expect("key"))
        .collect();
    assert_eq!(keys, vec!["apple", "application"]);

    let results = search_substring(repo, "AFé", 10);
    assert_eq!(results[0]["key"], "café");

    // `%` is a literal wildcard: no seeded word contains one.
    assert!(search_substring(repo, "%", 10)
        .as_array()
        .expect("array")
        .is_empty());

    let query = CString::new("ppl").expect("query");
    for limit in [0, -1, 201] {
        let mut error: *mut c_char = ptr::null_mut();
        // SAFETY: all pointers are valid for the call.
        let payload =
            unsafe { odc_repo_search_substring_json(repo, query.as_ptr(), limit, &mut error) };
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

// --- MDX import ------------------------------------------------------------

fn import(mdx: &Path, out: &Path) -> (i32, Option<String>, Option<String>) {
    let mdx = CString::new(mdx.to_str().expect("utf-8 mdx path")).expect("mdx cstring");
    let out = CString::new(out.to_str().expect("utf-8 out path")).expect("out cstring");
    let mut report: *mut c_char = ptr::null_mut();
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: both paths outlive the call and the out pointers are writable.
    let code =
        unsafe { odc_import_mdx_to_odc(mdx.as_ptr(), out.as_ptr(), &mut report, &mut error) };
    let report = if report.is_null() {
        None
    } else {
        Some(take_string(report))
    };
    (code, report, take_error_if_any(error))
}

#[derive(Clone, Copy, Debug)]
struct Sample {
    stage: i32,
    done: u64,
    total: u64,
    volume_index: u32,
    volume_count: u32,
    context: usize,
}

static SAMPLES: Mutex<Vec<Sample>> = Mutex::new(Vec::new());

unsafe extern "C" fn record_sample(
    stage: i32,
    done: u64,
    total: u64,
    volume_index: u32,
    volume_count: u32,
    context: *mut c_void,
) {
    SAMPLES.lock().expect("samples").push(Sample {
        stage,
        done,
        total,
        volume_index,
        volume_count,
        context: context as usize,
    });
}

#[test]
fn import_progress_reports_every_stage() {
    SAMPLES.lock().expect("samples").clear();
    let dir = tempfile::tempdir().expect("temp dir");
    let mdx = dir.path().join("dictionary.mdx");
    write_mdx_fixture(&mdx, 5);
    let volume = dir.path().join("dictionary.mdd");
    odc_testkit::write_mdd(
        &volume,
        &[
            ("\\pic\\apple.png", b"png bytes"),
            ("\\pic\\pear.png", b"pear"),
        ],
    )
    .expect("mdd");
    let out = dir.path().join("dictionary.odc");

    let mdx_c = CString::new(mdx.to_str().expect("utf8")).expect("path");
    let out_c = CString::new(out.to_str().expect("utf8")).expect("path");
    let volumes = serde_json::json!([volume.to_str().expect("utf8")]).to_string();
    let volumes_c = CString::new(volumes).expect("volumes json");
    let mut report: *mut c_char = ptr::null_mut();
    let mut error: *mut c_char = ptr::null_mut();
    let marker = 0x5a5a_5a5a_u64;
    let context = std::ptr::from_ref(&marker).cast_mut().cast::<c_void>();
    // SAFETY: all pointers are live C strings, the out slots are writable and
    // the context outlives the call.
    let code = unsafe {
        odc_import_mdx_to_odc_with_mdd_progress(
            mdx_c.as_ptr(),
            out_c.as_ptr(),
            volumes_c.as_ptr(),
            Some(record_sample),
            context,
            &mut report,
            &mut error,
        )
    };
    assert_eq!(code, 0, "import failed: {:?}", take_error_if_any(error));
    let _ = take_string(report);

    let samples = SAMPLES.lock().expect("samples").clone();
    assert!(!samples.is_empty(), "the callback fired");
    assert_eq!(
        samples.first().map(|sample| sample.stage),
        Some(0),
        "preparing first"
    );
    assert_eq!(
        samples.last().map(|sample| sample.stage),
        Some(3),
        "committing last"
    );
    for sample in &samples {
        assert_eq!(
            sample.context, context as usize,
            "context is passed through"
        );
    }
    let mut last_stage = 0;
    let mut entries_done = 0;
    let mut entries_total = 0;
    let mut resources_done = 0;
    for sample in &samples {
        assert!(sample.stage >= last_stage, "stages never go back");
        last_stage = sample.stage;
        assert!(sample.done <= sample.total, "done stays within total");
        if sample.stage == 1 {
            assert!(sample.done >= entries_done, "entries never rewind");
            entries_done = sample.done;
            entries_total = sample.total;
        }
        if sample.stage == 2 {
            assert!(sample.done >= resources_done, "resources never rewind");
            assert_eq!(sample.volume_index, 1);
            assert_eq!(sample.volume_count, 1);
            resources_done = sample.done;
        }
    }
    assert_eq!(entries_total, 5, "entries total is the record count");
    assert_eq!(entries_done, 5, "every record is accounted for");
    assert_eq!(resources_done, 2, "every resource is accounted for");
    let last = samples.last().expect("final sample");
    assert_eq!(last.done, last.total, "the last sample is complete");
    assert_eq!(last.total, 7, "the final total is entries plus resources");
}

fn write_mdx_fixture(mdx: &Path, count: usize) {
    let entries: Vec<(String, String)> = (0..count)
        .map(|index| {
            let body = format!("<p>{index}</p>{}", "x".repeat(8 * 1024));
            (format!("word{index:04}"), body)
        })
        .collect();
    let borrowed: Vec<(&str, &str)> = entries
        .iter()
        .map(|(key, body)| (key.as_str(), body.as_str()))
        .collect();
    odc_testkit::write_v2_mdx(mdx, &borrowed).expect("write fixture");
}

fn corrupt_first_block(mdx: &Path) {
    let ScanOutcome::Scanned(section) = scan(mdx).expect("scan fixture") else {
        panic!("expected a scanned record section");
    };
    let block = &section.blocks[0];
    let mut bytes = std::fs::read(mdx).expect("read fixture");
    let start = usize::try_from(block.file_offset).expect("offset fits usize");
    let size = usize::try_from(block.compressed_size).expect("size fits usize");
    bytes[start + 8 + (size - 8) / 2] ^= 0xff;
    std::fs::write(mdx, bytes).expect("write corrupt fixture");
}

fn staging_files(directory: &Path) -> Vec<String> {
    std::fs::read_dir(directory)
        .expect("read dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".partial-"))
        .collect()
}

#[test]
fn import_mdx_writes_a_searchable_odc_file() {
    let dir = TempDir::new().expect("temp dir");
    let mdx = dir.path().join("fixture.mdx");
    write_mdx_fixture(&mdx, 40);
    let out = dir.path().join("dictionary.odc");

    let (code, report, error) = import(&mdx, &out);
    assert_eq!(code, 0, "import failed: {error:?}");
    assert!(error.is_none(), "unexpected error: {error:?}");

    let report: Value =
        serde_json::from_str(&report.expect("report json")).expect("report is json");
    assert_eq!(report["imported_entries"], 40);
    assert_eq!(report["skipped_blocks"], 0);
    assert_eq!(report["skipped_entries"], 0);
    assert_eq!(report["header"]["engine_version"], "2.0");
    assert!(report["output_bytes"].as_u64().expect("output bytes") > 0);
    assert!(out.is_file(), "the pack file must exist");

    // The exported single file opens as a repository.
    let path = CString::new(out.to_str().expect("utf-8 path")).expect("cstring");
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: the path is NUL-terminated and lives for the call.
    let repo = unsafe { odc_repo_open(path.as_ptr(), &mut error) };
    assert!(
        !repo.is_null(),
        "open failed: {:?}",
        take_error_if_any(error)
    );
    assert_eq!(count(repo), 40);

    let key = CString::new("word0000").expect("key");
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: live handle and NUL-terminated key.
    let entry = unsafe { odc_repo_entry_json(repo, key.as_ptr(), &mut error) };
    assert!(
        !entry.is_null(),
        "lookup failed: {:?}",
        take_error_if_any(error)
    );
    assert!(take_string(entry).contains("word0000"));

    // SAFETY: the handle is closed exactly once.
    unsafe { odc_repo_close(repo) };
}

#[test]
fn import_mdx_reports_skipped_corrupt_blocks() {
    let dir = TempDir::new().expect("temp dir");
    let mdx = dir.path().join("fixture.mdx");
    write_mdx_fixture(&mdx, 40);
    corrupt_first_block(&mdx);
    let out = dir.path().join("damaged.odc");

    let (code, report, error) = import(&mdx, &out);
    assert_eq!(code, 0, "import failed: {error:?}");
    let report: Value =
        serde_json::from_str(&report.expect("report json")).expect("report is json");
    assert_eq!(report["skipped_blocks"], 1);
    assert!(
        report["skipped_entries"].as_u64().expect("skipped") > 0,
        "the damaged block held entries"
    );
    assert_eq!(
        report["corrupt_blocks"].as_array().expect("blocks").len(),
        1
    );
    assert!(out.is_file());
}

#[test]
fn import_mdx_never_overwrites_an_existing_file() {
    let dir = TempDir::new().expect("temp dir");
    let mdx = dir.path().join("fixture.mdx");
    write_mdx_fixture(&mdx, 10);
    let out = dir.path().join("taken.odc");
    std::fs::write(&out, b"keep me").expect("seed output");

    let (code, report, error) = import(&mdx, &out);
    assert_ne!(code, 0, "an existing output must be rejected");
    assert!(report.is_none(), "no report on failure");
    let message = error.expect("error message");
    assert!(message.contains("already exists"), "unexpected: {message}");
    assert_eq!(std::fs::read(&out).expect("read output"), b"keep me");
    assert!(
        staging_files(dir.path()).is_empty(),
        "no staging file may survive a failed import"
    );
}

#[test]
fn import_mdx_reports_a_missing_source() {
    let dir = TempDir::new().expect("temp dir");
    let mdx = dir.path().join("absent.mdx");
    let out = dir.path().join("absent.odc");

    let (code, report, error) = import(&mdx, &out);
    assert_ne!(code, 0);
    assert!(report.is_none());
    assert!(error.expect("error message").contains("import failed"));
    assert!(!out.exists());
    assert!(staging_files(dir.path()).is_empty());
}

#[test]
fn import_mdx_accepts_a_null_report_pointer() {
    let dir = TempDir::new().expect("temp dir");
    let mdx = dir.path().join("fixture.mdx");
    write_mdx_fixture(&mdx, 5);
    let out = dir.path().join("quiet.odc");
    let mdx_c = CString::new(mdx.to_str().expect("utf-8 path")).expect("cstring");
    let out_c = CString::new(out.to_str().expect("utf-8 path")).expect("cstring");
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: a NULL report slot is explicitly supported.
    let code = unsafe {
        odc_import_mdx_to_odc(mdx_c.as_ptr(), out_c.as_ptr(), ptr::null_mut(), &mut error)
    };
    assert_eq!(code, 0, "import failed: {:?}", take_error_if_any(error));
    assert!(out.is_file());
}

#[test]
#[ignore = "set ODC_MDX_SMOKE to the path of a real MDX file"]
fn imports_a_real_mdx_file() {
    let source = std::env::var("ODC_MDX_SMOKE").expect("ODC_MDX_SMOKE must name a file");
    let dir = TempDir::new().expect("temp dir");
    let out = dir.path().join("smoke.odc");

    let (code, report, error) = import(Path::new(&source), &out);
    assert_eq!(code, 0, "import failed: {error:?}");
    let report: Value = serde_json::from_str(&report.expect("report")).expect("report is json");
    println!("report: {report}");
    println!(
        "output bytes: {}",
        std::fs::metadata(&out).expect("metadata").len()
    );

    let path = CString::new(out.to_str().expect("utf-8 path")).expect("cstring");
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: the path is NUL-terminated and lives for the call.
    let repo = unsafe { odc_repo_open(path.as_ptr(), &mut error) };
    assert!(
        !repo.is_null(),
        "open failed: {:?}",
        take_error_if_any(error)
    );
    let total = count(repo);
    println!("entries: {total}");
    // SAFETY: the handle is closed exactly once.
    unsafe { odc_repo_close(repo) };

    assert!(report["imported_entries"].as_u64().expect("imported") > 0);
    assert_eq!(
        i64::try_from(report["imported_entries"].as_u64().unwrap()).unwrap(),
        total
    );
}

#[test]
fn resource_accessors_read_through_the_abi() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut repo = Repo::init(dir.path()).expect("init");
    repo.put_resource("pic/apple.png", b"png bytes")
        .expect("put");
    repo.put_resource("sound/apple.mp3", b"mp3 bytes")
        .expect("put");
    let handle = open(&dir);

    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: live handle plus a writable error slot.
    let count = unsafe { odc_repo_resource_count(handle, &mut error) };
    assert!(take_error_if_any(error).is_none());
    assert_eq!(count, 2);

    let key = CString::new("pic/apple.png").expect("key");
    let mut length: u64 = 0;
    // SAFETY: live handle, valid key and writable length slot.
    let found = unsafe { odc_repo_resource_len(handle, key.as_ptr(), &mut length, &mut error) };
    assert!(take_error_if_any(error).is_none());
    assert_eq!(found, 1);
    assert_eq!(length, 9);

    let mut buffer = vec![0_u8; length as usize];
    let mut copied: usize = 0;
    // SAFETY: the buffer is exactly `length` bytes.
    let code = unsafe {
        odc_repo_resource_copy(
            handle,
            key.as_ptr(),
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut copied,
            &mut error,
        )
    };
    assert!(take_error_if_any(error).is_none());
    assert_eq!(code, 1);
    assert_eq!(copied, 9);
    assert_eq!(buffer, b"png bytes".to_vec());

    // A case-insensitive lookup reaches the same payload.
    let shout = CString::new("PIC/APPLE.PNG").expect("key");
    let mut other: u64 = 0;
    // SAFETY: same contract as above.
    let found = unsafe { odc_repo_resource_len(handle, shout.as_ptr(), &mut other, &mut error) };
    assert!(take_error_if_any(error).is_none());
    assert_eq!(found, 1);
    assert_eq!(other, 9);

    // A missing key reports 0 instead of an error.
    let missing = CString::new("pic/missing.png").expect("key");
    let mut unused: u64 = 0;
    // SAFETY: same contract as above.
    let found = unsafe { odc_repo_resource_len(handle, missing.as_ptr(), &mut unused, &mut error) };
    assert_eq!(found, 0);
    assert!(take_error_if_any(error).is_none());

    // Too small a buffer is an error, not a silent truncation.
    let mut tiny = [0_u8; 3];
    let mut copied: usize = 0;
    // SAFETY: the buffer is smaller than the payload on purpose.
    let code = unsafe {
        odc_repo_resource_copy(
            handle,
            key.as_ptr(),
            tiny.as_mut_ptr(),
            tiny.len(),
            &mut copied,
            &mut error,
        )
    };
    assert_eq!(code, -1);
    assert!(take_error_if_any(error).is_some());

    // SAFETY: the handle is closed exactly once.
    unsafe { odc_repo_close(handle) };
}

#[test]
fn import_with_explicit_mdd_volumes_stores_resources() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mdx = dir.path().join("dictionary.mdx");
    write_mdx_fixture(&mdx, 2);
    let volume = dir.path().join("dictionary.1.mdd");
    odc_testkit::write_mdd(&volume, &[("\\pic\\apple.png", b"png bytes")]).expect("mdd");
    let out = dir.path().join("dictionary.odc");

    let mdx_c = CString::new(mdx.to_str().expect("utf8")).expect("path");
    let out_c = CString::new(out.to_str().expect("utf8")).expect("path");
    let volumes = serde_json::json!([volume.to_str().expect("utf8")]).to_string();
    let volumes_c = CString::new(volumes).expect("json");
    let mut report: *mut c_char = ptr::null_mut();
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: every pointer is valid for the call.
    let code = unsafe {
        odc_import_mdx_to_odc_with_mdd(
            mdx_c.as_ptr(),
            out_c.as_ptr(),
            volumes_c.as_ptr(),
            &mut report,
            &mut error,
        )
    };
    assert_eq!(code, 0, "import failed: {:?}", take_error_if_any(error));
    let parsed: Value = serde_json::from_str(&take_string(report)).expect("report json");
    assert_eq!(parsed["resource_count"], 1);
    assert_eq!(parsed["mdd_files"][0]["entries"], 1);
    assert_eq!(parsed["warnings"].as_array().expect("warnings").len(), 0);

    let path = CString::new(out.to_str().expect("utf8")).expect("path");
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: path is a live C string, error is a writable slot.
    let handle = unsafe { odc_repo_open(path.as_ptr(), &mut error) };
    assert!(take_error_if_any(error).is_none());
    let key = CString::new("pic/apple.png").expect("key");
    let mut length: u64 = 0;
    // SAFETY: live handle, valid key, writable length.
    let found = unsafe { odc_repo_resource_len(handle, key.as_ptr(), &mut length, &mut error) };
    assert_eq!(found, 1);
    assert_eq!(length, 9);
    // SAFETY: the handle is closed exactly once.
    unsafe { odc_repo_close(handle) };
}
#[test]
fn editing_round_trip_over_the_abi() {
    let dir = seeded_repo();
    let repo = open(&dir);

    // Page one: two entries in key order.
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: repo is live and error is a writable slot.
    let first = unsafe { odc_repo_entries_page_json(repo, ptr::null(), 2, &mut error) };
    assert!(take_error_if_any(error).is_none(), "page failed");
    let first: Value = serde_json::from_str(&take_string(first)).expect("page json");
    let first = first.as_array().expect("page array");
    assert_eq!(first.len(), 2);
    assert_eq!(first[0]["key"], "apple");
    let cursor = CString::new(first[1]["key"].as_str().expect("cursor")).expect("cursor");

    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: cursor is NUL-terminated and outlives the call.
    let rest = unsafe { odc_repo_entries_page_json(repo, cursor.as_ptr(), 10, &mut error) };
    assert!(take_error_if_any(error).is_none(), "second page failed");
    let rest: Value = serde_json::from_str(&take_string(rest)).expect("page json");
    assert_eq!(rest.as_array().expect("array").len(), 2);
    assert_eq!(rest[0]["key"], "caf\u{00e9}");

    // Put a new entry: the ABI accepts the object entry_to_json emits.
    let body = serde_json::json!({
        "key": "apricot",
        "display": "Apricot",
        "phonetic": [],
        "defs": [{"pos": "noun", "sense": "a fruit", "examples": [], "labels": []}],
        "raw_html": null,
        "extra": {"source": "abi"}
    })
    .to_string();
    let body = CString::new(body).expect("entry json");
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: body is NUL-terminated and outlives the call.
    let put = unsafe { odc_repo_entry_put_json(repo, body.as_ptr(), &mut error) };
    assert!(take_error_if_any(error).is_none(), "put failed");
    let put: Value = serde_json::from_str(&take_string(put)).expect("put json");
    assert_eq!(put["key"], "apricot");
    assert_eq!(put["outcome"], "added");

    // Status and the change list agree.
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: repo is live and error is a writable slot.
    let status = unsafe { odc_repo_working_status_json(repo, &mut error) };
    assert!(take_error_if_any(error).is_none(), "status failed");
    let status: Value = serde_json::from_str(&take_string(status)).expect("status json");
    assert_eq!(status["added"], 1);
    assert_eq!(status["total"], 5);

    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: repo is live and error is a writable slot.
    let changes = unsafe { odc_repo_working_changes_json(repo, ptr::null(), 10, &mut error) };
    assert!(take_error_if_any(error).is_none(), "changes failed");
    let changes: Value = serde_json::from_str(&take_string(changes)).expect("changes json");
    assert_eq!(changes.as_array().expect("array").len(), 1);
    assert_eq!(changes[0]["key"], "apricot");
    assert_eq!(changes[0]["kind"], "added");

    // Field-level diff marks the entry as added.
    let key = CString::new("apricot").expect("key");
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: key is NUL-terminated and outlives the call.
    let diff = unsafe { odc_repo_entry_diff_json(repo, key.as_ptr(), &mut error) };
    assert!(take_error_if_any(error).is_none(), "diff failed");
    let diff: Value = serde_json::from_str(&take_string(diff)).expect("diff json");
    assert_eq!(diff["kind"], "added");
    let fields = diff["fields"].as_array().expect("fields");
    assert!(fields.iter().any(|field| field["field"] == "defs[0].sense"
        && field["changed"] == true
        && field["before"].is_null()));

    // Commit and then discard the next edit.
    let message = CString::new("add apricot").expect("message");
    let author = CString::new("tester").expect("author");
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: both strings are NUL-terminated and outlive the call.
    let commit =
        unsafe { odc_repo_commit_json(repo, message.as_ptr(), author.as_ptr(), &mut error) };
    assert!(take_error_if_any(error).is_none(), "commit failed");
    let commit: Value = serde_json::from_str(&take_string(commit)).expect("commit json");
    assert_eq!(commit["entry_count"], 5);
    assert_eq!(commit["author"], "tester");

    let body = CString::new("{\"key\":\"badger\",\"display\":\"Badger\"}").expect("entry");
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: body is NUL-terminated and outlives the call.
    let put = unsafe { odc_repo_entry_put_json(repo, body.as_ptr(), &mut error) };
    assert!(take_error_if_any(error).is_none(), "second put failed");
    let put: Value = serde_json::from_str(&take_string(put)).expect("put json");
    assert_eq!(put["outcome"], "added");

    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: repo is live and error is a writable slot.
    let discard = unsafe { odc_repo_discard_json(repo, &mut error) };
    assert!(take_error_if_any(error).is_none(), "discard failed");
    let discard: Value = serde_json::from_str(&take_string(discard)).expect("discard json");
    assert_eq!(discard["discarded"], true);

    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: repo is live and error is a writable slot.
    let status = unsafe { odc_repo_working_status_json(repo, &mut error) };
    let status: Value = serde_json::from_str(&take_string(status)).expect("status json");
    assert_eq!(status["added"], 0);
    assert_eq!(status["total"], 5);

    // SAFETY: the handle is closed exactly once and not used afterwards.
    unsafe { odc_repo_close(repo) };
}

#[test]
fn editing_calls_report_errors() {
    let dir = seeded_repo();
    let repo = open(&dir);

    let body = CString::new("{not json").expect("body");
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: body is NUL-terminated and outlives the call.
    let result = unsafe { odc_repo_entry_put_json(repo, body.as_ptr(), &mut error) };
    assert!(result.is_null());
    let message = take_error_if_any(error).expect("expected an error");
    assert!(message.contains("JSON"), "{message}");

    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: repo is live and error is a writable slot.
    let result = unsafe { odc_repo_entries_page_json(repo, ptr::null(), 0, &mut error) };
    assert!(result.is_null());
    let message = take_error_if_any(error).expect("expected an error");
    assert!(message.contains("limit"), "{message}");

    let message_text = CString::new("nothing").expect("message");
    let author = CString::new("tester").expect("author");
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: both strings are NUL-terminated and outlive the call.
    let result =
        unsafe { odc_repo_commit_json(repo, message_text.as_ptr(), author.as_ptr(), &mut error) };
    assert!(result.is_null());
    let message = take_error_if_any(error).expect("expected an error");
    assert!(message.contains("ODC-E012"), "{message}");

    let key = CString::new("missing").expect("key");
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: key is NUL-terminated and outlives the call.
    let result = unsafe { odc_repo_entry_delete_json(repo, key.as_ptr(), &mut error) };
    assert!(take_error_if_any(error).is_none(), "delete failed");
    let result: Value = serde_json::from_str(&take_string(result)).expect("delete json");
    assert_eq!(result["outcome"], "not_found");

    // SAFETY: the handle is closed exactly once and not used afterwards.
    unsafe { odc_repo_close(repo) };
}

#[test]
fn import_into_repo_writes_an_editable_library() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mdx = dir.path().join("dictionary.mdx");
    write_mdx_fixture(&mdx, 3);
    let repo_dir = dir.path().join("library.odc");
    let mdx_text = CString::new(mdx.to_str().expect("path")).expect("mdx path");
    let repo_text = CString::new(repo_dir.to_str().expect("path")).expect("repo path");
    let mut report: *mut c_char = ptr::null_mut();
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: both arguments are live C strings, the out slots are writable and
    // no callback or context is passed.
    let code = unsafe {
        odc_import_mdx_to_repo_with_mdd_progress(
            mdx_text.as_ptr(),
            repo_text.as_ptr(),
            ptr::null(),
            None,
            ptr::null_mut(),
            &mut report,
            &mut error,
        )
    };
    assert_eq!(code, 0, "import failed: {:?}", take_error_if_any(error));
    let payload: Value = serde_json::from_str(&take_string(report)).expect("report json");
    assert_eq!(payload["is_directory"], Value::Bool(true));
    assert_eq!(payload["imported_entries"], Value::from(3));
    assert_eq!(
        payload["repo_path"].as_str().expect("repo path"),
        repo_dir.to_str().expect("path")
    );
    assert_eq!(
        payload["commit"].as_str().expect("commit").len(),
        64,
        "commit is a 32-byte hex hash"
    );

    let repo = Repo::open(&repo_dir).expect("open library");
    assert!(!repo.is_read_only(), "an imported library must be editable");
    assert!(repo.head().expect("head").is_some());
    assert!(repo.entry_count().expect("count") >= 3);
}

#[test]
fn import_into_repo_refuses_an_existing_library() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mdx = dir.path().join("dictionary.mdx");
    write_mdx_fixture(&mdx, 1);
    let repo_dir = dir.path().join("library.odc");
    std::fs::create_dir_all(&repo_dir).expect("dir");
    std::fs::write(repo_dir.join("odc.db"), b"existing").expect("marker");
    let mdx_text = CString::new(mdx.to_str().expect("path")).expect("mdx path");
    let repo_text = CString::new(repo_dir.to_str().expect("path")).expect("repo path");
    let mut report: *mut c_char = ptr::null_mut();
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: as above.
    let code = unsafe {
        odc_import_mdx_to_repo_with_mdd_progress(
            mdx_text.as_ptr(),
            repo_text.as_ptr(),
            ptr::null(),
            None,
            ptr::null_mut(),
            &mut report,
            &mut error,
        )
    };
    assert_eq!(code, -1);
    assert!(report.is_null(), "a failed import leaves no report");
    let message = take_error_if_any(error).expect("error message");
    assert!(
        message.contains("already initialized"),
        "unexpected message: {message}"
    );
    assert_eq!(
        std::fs::read(repo_dir.join("odc.db")).expect("marker survives"),
        b"existing".to_vec()
    );
}

#[test]
fn copy_pack_to_repo_reports_an_editable_copy() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mdx = dir.path().join("dictionary.mdx");
    write_mdx_fixture(&mdx, 2);
    let pack = dir.path().join("export.odc");
    let pack_text = CString::new(pack.to_str().expect("path")).expect("pack path");
    let mdx_text = CString::new(mdx.to_str().expect("path")).expect("mdx path");
    let mut report: *mut c_char = ptr::null_mut();
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: live C strings and writable out slots.
    let code = unsafe {
        odc_import_mdx_to_odc(
            mdx_text.as_ptr(),
            pack_text.as_ptr(),
            &mut report,
            &mut error,
        )
    };
    assert_eq!(code, 0, "pack failed: {:?}", take_error_if_any(error));
    let _ = take_string(report);

    let copy_dir = dir.path().join("copy.odc");
    let copy_text = CString::new(copy_dir.to_str().expect("path")).expect("copy path");
    let mut copy_report: *mut c_char = ptr::null_mut();
    let mut copy_error: *mut c_char = ptr::null_mut();
    // SAFETY: as above.
    let code = unsafe {
        odc_copy_pack_to_repo(
            pack_text.as_ptr(),
            copy_text.as_ptr(),
            &mut copy_report,
            &mut copy_error,
        )
    };
    assert_eq!(code, 0, "copy failed: {:?}", take_error_if_any(copy_error));
    let payload: Value = serde_json::from_str(&take_string(copy_report)).expect("report json");
    assert_eq!(payload["is_directory"], Value::Bool(true));
    assert!(payload["entries"].as_u64().expect("entries") >= 2);

    let copy = Repo::open(&copy_dir).expect("open copy");
    assert!(!copy.is_read_only(), "the copy must be editable");
    drop(copy);
    let original = Repo::open(&pack).expect("open pack");
    assert!(original.is_read_only(), "the pack stays read-only");
}

#[test]
fn working_link_prefers_uncommitted_edits() {
    fn lookup(repo: *const odc_ffi::OdcRepo, key: &str) -> Value {
        let key = CString::new(key).expect("key");
        let mut error: *mut c_char = ptr::null_mut();
        // SAFETY: `repo` is live, `key` is NUL-terminated and outlives the call.
        let payload = unsafe { odc_repo_entry_link_working_json(repo, key.as_ptr(), &mut error) };
        assert!(take_error_if_any(error).is_none(), "link failed");
        serde_json::from_str(&take_string(payload)).expect("link json")
    }

    let dir = seeded_repo();
    let repo = open(&dir);

    let committed = lookup(repo, "apple");
    assert_eq!(committed["status"], "found");
    assert_eq!(committed["entry"]["defs"][0]["sense"], "a fruit");

    let body = serde_json::json!({
        "key": "apple",
        "display": "Apple",
        "phonetic": [],
        "defs": [{"pos": "noun", "sense": "a working fruit", "examples": [], "labels": []}],
        "raw_html": null,
        "extra": {}
    })
    .to_string();
    let body = CString::new(body).expect("entry json");
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: body is NUL-terminated and outlives the call.
    let put = unsafe { odc_repo_entry_put_json(repo, body.as_ptr(), &mut error) };
    assert!(take_error_if_any(error).is_none(), "put failed");
    let _ = take_string(put);

    let modified = lookup(repo, "apple");
    assert_eq!(modified["status"], "found");
    assert_eq!(modified["entry"]["defs"][0]["sense"], "a working fruit");

    let key = CString::new("application").expect("key");
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: key is NUL-terminated and outlives the call.
    let deleted = unsafe { odc_repo_entry_delete_json(repo, key.as_ptr(), &mut error) };
    assert!(take_error_if_any(error).is_none(), "delete failed");
    let _ = take_string(deleted);

    let removed = lookup(repo, "application");
    assert_eq!(removed["status"], "deleted");
    assert_eq!(removed["requested"], "application");

    // Existing resolve still reads the committed body.
    let committed = lookup_committed(repo);
    assert_eq!(committed["entry"]["defs"][0]["sense"], "a fruit");

    // SAFETY: the handle is closed exactly once and not used afterwards.
    unsafe { odc_repo_close(repo) }
}

fn lookup_committed(repo: *const odc_ffi::OdcRepo) -> Value {
    let key = CString::new("apple").expect("key");
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: `repo` is live and the string outlives the call.
    let payload = unsafe { odc_repo_entry_link_json(repo, key.as_ptr(), &mut error) };
    assert!(take_error_if_any(error).is_none(), "committed link failed");
    serde_json::from_str(&take_string(payload)).expect("link json")
}

fn html_process(html: &CStr, mode: &CStr) -> (serde_json::Value, Option<String>) {
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: both arguments are NUL-terminated and outlive the call.
    let payload = unsafe { odc_html_process_json(html.as_ptr(), mode.as_ptr(), &mut error) };
    let message = take_error_if_any(error);
    if message.is_some() {
        assert!(payload.is_null(), "failed call must not return JSON");
        return (Value::Null, message);
    }
    let value = serde_json::from_str(&take_string(payload)).expect("html json");
    (value, None)
}

#[test]
fn html_check_reports_without_rewriting() {
    let html = CString::new("<p>hi</p><script>alert(1)</script>").expect("html");
    let mode = CString::new("check").expect("mode");
    let (value, error) = html_process(&html, &mode);
    assert!(error.is_none(), "check failed: {error:?}");
    assert_eq!(value["html"], "<p>hi</p><script>alert(1)</script>");
    assert_eq!(value["changed"], false);
    let diagnostics = value["diagnostics"].as_array().expect("diagnostics");
    assert!(!diagnostics.is_empty(), "script removal must be reported");
    assert!(diagnostics.iter().all(|item| {
        item["code"].is_string() && item["message"].is_string() && item.get("detail").is_some()
    }));
    assert!(diagnostics
        .iter()
        .any(|item| item["code"] == "html.removed_element"));
}

#[test]
fn html_format_repairs_and_formats() {
    let html = CString::new("<p>hi</p><script>alert(1)</script>").expect("html");
    let mode = CString::new("format").expect("mode");
    let (value, error) = html_process(&html, &mode);
    assert!(error.is_none(), "format failed: {error:?}");
    assert_eq!(value["changed"], true);
    let repaired = value["html"].as_str().expect("html string");
    assert!(repaired.contains("hi"), "content must survive: {repaired}");
    assert!(
        !repaired.contains("script"),
        "script must be removed: {repaired}"
    );
}

#[test]
fn html_rejects_unknown_mode() {
    let html = CString::new("<p>hi</p>").expect("html");
    let mode = CString::new("bogus").expect("mode");
    let (value, error) = html_process(&html, &mode);
    assert!(value.is_null());
    let message = error.expect("error message");
    assert!(message.contains("unknown HTML mode"), "{message}");
}

#[test]
fn html_check_accepts_an_empty_body() {
    let html = CString::new("").expect("html");
    let mode = CString::new("check").expect("mode");
    let (value, error) = html_process(&html, &mode);
    assert!(error.is_none(), "check failed: {error:?}");
    assert_eq!(value["html"], "");
    assert_eq!(value["changed"], false);
    assert_eq!(
        value["diagnostics"].as_array().expect("diagnostics").len(),
        0
    );
}

#[test]
fn html_rejects_null_and_invalid_utf8() {
    let mode = CString::new("check").expect("mode");
    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: a NULL body is explicitly rejected before any dereference.
    let payload = unsafe { odc_html_process_json(ptr::null(), mode.as_ptr(), &mut error) };
    assert!(payload.is_null());
    let message = take_error_if_any(error).expect("error message");
    assert!(message.contains("html must not be NULL"), "{message}");

    let invalid =
        CString::new(vec![b'<', b'p', b'>', 0xff, b'<', b'/', b'p', b'>']).expect("bytes");
    let (value, error) = html_process(&invalid, &mode);
    assert!(value.is_null());
    assert!(error.expect("error message").contains("not valid UTF-8"));
}
