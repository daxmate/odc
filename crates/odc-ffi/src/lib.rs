//! C ABI for [`odc_core`], built as a static library for Apple platforms.
//!
//! The surface is deliberately small: open a repository, count entries,
//! prefix-search keys, fetch one entry as JSON and free returned strings.
//!
//! # Error convention
//!
//! Every fallible call takes an `error_message: *mut *mut c_char` out
//! parameter. Success stores `NULL`; failure stores a heap-allocated,
//! NUL-terminated UTF-8 message that the caller releases with
//! [`odc_string_free`]. Passing a `NULL` `error_message` is allowed and
//! simply discards the diagnostic.
//!
//! Panics never cross the FFI boundary: each entry point runs inside
//! `catch_unwind` and reports a panic through the same channel.
//!
//! # Threading
//!
//! A handle must be used by one thread at a time. `OdcRepo` is neither
//! `Send` nor `Sync`; sharing it across threads requires external locking.

use std::any::Any;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::ptr;

use odc_core::cbor::Value as CborValue;
use odc_core::pack::PackOptions;
use odc_core::{import_mdx, Entry, MdxImportOptions, MdxImportReport, Repo};
use serde_json::{json, Map, Value as JsonValue};

/// Largest accepted `limit` for [`odc_repo_search_json`].
const MAX_SEARCH_LIMIT: i32 = 200;

/// Opaque repository handle exposed as `OdcRepo` in `odc.h`.
pub struct OdcRepo {
    repo: Repo,
}

/// Release a heap-allocated string previously returned by this library.
///
/// Passing `NULL` is a no-op.
///
/// # Safety
///
/// `s` must be `NULL` or a pointer returned by this library that has not
/// been freed yet. Any other pointer is undefined behaviour.
#[no_mangle]
pub unsafe extern "C" fn odc_string_free(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    guarded(ptr::null_mut(), (), || {
        // SAFETY: `s` was produced by `CString::into_raw` in this crate and
        // the caller transfers ownership exactly once.
        drop(unsafe { CString::from_raw(s) });
        Ok(())
    });
}

/// Version string of the FFI crate, e.g. `"0.1.0"`.
///
/// The pointer refers to static storage and must not be freed. This entry
/// point cannot panic, so it needs no `catch_unwind` wrapper.
#[no_mangle]
pub extern "C" fn odc_version() -> *const c_char {
    const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "\0");
    VERSION.as_ptr().cast()
}

/// Open an existing repository directory.
///
/// Returns `NULL` on failure.
///
/// # Safety
///
/// `path_utf8` must be `NULL` or a NUL-terminated UTF-8 string;
/// `error_message` must be `NULL` or point to writable storage for one
/// `char *`. On success the returned handle must be closed with
/// [`odc_repo_close`] and must not be used from multiple threads at once.
#[no_mangle]
pub unsafe extern "C" fn odc_repo_open(
    path_utf8: *const c_char,
    error_message: *mut *mut c_char,
) -> *mut OdcRepo {
    guarded(error_message, ptr::null_mut(), || {
        let path = text_arg(path_utf8, "path")?;
        let repo = Repo::open(Path::new(&path)).map_err(|error| error.to_string())?;
        Ok(Box::into_raw(Box::new(OdcRepo { repo })))
    })
}

/// Close a handle returned by [`odc_repo_open`].
///
/// Passing `NULL` is a no-op.
///
/// # Safety
///
/// `repo` must be `NULL` or a live handle that has not been closed yet and
/// is not in use by another thread.
#[no_mangle]
pub unsafe extern "C" fn odc_repo_close(repo: *mut OdcRepo) {
    if repo.is_null() {
        return;
    }
    guarded(ptr::null_mut(), (), || {
        // SAFETY: the caller guarantees `repo` came from `odc_repo_open`
        // and is closed exactly once.
        drop(unsafe { Box::from_raw(repo) });
        Ok(())
    });
}

/// Number of committed entries, or `0` on failure.
///
/// # Safety
///
/// `repo` must be `NULL` or a live handle; `error_message` must be
/// `NULL` or point to writable storage for one `char *`.
#[no_mangle]
pub unsafe extern "C" fn odc_repo_entry_count(
    repo: *const OdcRepo,
    error_message: *mut *mut c_char,
) -> i64 {
    guarded(error_message, 0, || {
        let repo = repo_ref(repo)?;
        let count = repo.repo.entry_count().map_err(|error| error.to_string())?;
        i64::try_from(count).map_err(|_| "entry count exceeds i64".to_string())
    })
}

/// Prefix-search normalised keys and return a JSON array of
/// `{"key": "...", "display": "..."}` objects, in key order.
///
/// `limit` must be between 1 and 200 (inclusive). Returns `NULL` on
/// failure.
///
/// # Safety
///
/// `repo` must be `NULL` or a live handle; `query_utf8` must be `NULL`
/// or a NUL-terminated UTF-8 string; `error_message` must be `NULL` or
/// point to writable storage for one `char *`. The returned string must be
/// released with [`odc_string_free`].
#[no_mangle]
pub unsafe extern "C" fn odc_repo_search_json(
    repo: *const OdcRepo,
    query_utf8: *const c_char,
    limit: i32,
    error_message: *mut *mut c_char,
) -> *mut c_char {
    guarded(error_message, ptr::null_mut(), || {
        let repo = repo_ref(repo)?;
        let query = text_arg(query_utf8, "query")?;
        if query.trim().is_empty() {
            return Err("query must not be empty".to_string());
        }
        if !(1..=MAX_SEARCH_LIMIT).contains(&limit) {
            return Err(format!("limit must be between 1 and {MAX_SEARCH_LIMIT}"));
        }
        let limit = usize::try_from(limit).map_err(|_| "limit out of range".to_string())?;
        let matches = repo
            .repo
            .search_prefix(&query, limit)
            .map_err(|error| error.to_string())?;
        let items: Vec<JsonValue> = matches
            .into_iter()
            .map(|(key, display)| json!({ "key": key, "display": display }))
            .collect();
        to_c_string(&JsonValue::Array(items))
    })
}

/// Look up one entry by exact (normalised) key and return its JSON
/// representation, or `NULL` when the key is absent or an error occurs.
///
/// # Safety
///
/// `repo` must be `NULL` or a live handle; `key_utf8` must be `NULL`
/// or a NUL-terminated UTF-8 string; `error_message` must be `NULL` or
/// point to writable storage for one `char *`. The returned string must be
/// released with [`odc_string_free`].
#[no_mangle]
pub unsafe extern "C" fn odc_repo_entry_json(
    repo: *const OdcRepo,
    key_utf8: *const c_char,
    error_message: *mut *mut c_char,
) -> *mut c_char {
    guarded(error_message, ptr::null_mut(), || {
        let repo = repo_ref(repo)?;
        let key = text_arg(key_utf8, "key")?;
        if key.trim().is_empty() {
            return Err("key must not be empty".to_string());
        }
        let entry = repo
            .repo
            .show(&key)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("entry not found: {key}"))?;
        to_c_string(&entry_to_json(&entry))
    })
}

/// Import an MDX file into a standalone `.odc` file.
///
/// Returns `0` on success and `-1` on failure. On success `report_json_out`
/// receives a heap-allocated, NUL-terminated JSON report that the caller
/// releases with [`odc_string_free`]. On failure `report_json_out` is left
/// `NULL` and `error_message` carries the diagnostic.
///
/// The output is written atomically: the working repository is packed into a
/// temporary file inside the destination directory and renamed into place. An
/// existing file at `out_odc_path` is a hard error and is never replaced.
///
/// # Safety
///
/// `mdx_path` and `out_odc_path` must be NUL-terminated UTF-8 strings;
/// `report_json_out` and `error_message` must be `NULL` or point to writable
/// storage for one `char *`.
#[no_mangle]
pub unsafe extern "C" fn odc_import_mdx_to_odc(
    mdx_path: *const c_char,
    out_odc_path: *const c_char,
    report_json_out: *mut *mut c_char,
    error_message: *mut *mut c_char,
) -> i32 {
    // A stale report pointer must never survive a failed import.
    clear_error(report_json_out);
    guarded(error_message, -1, || {
        // SAFETY: both pointers follow the contract documented above.
        let mdx = unsafe { text_arg(mdx_path, "mdx_path") }?;
        let out = unsafe { text_arg(out_odc_path, "out_odc_path") }?;
        let converted = import_mdx_to_odc(Path::new(&mdx), Path::new(&out))?;
        let payload = import_report_to_json(&converted.report, converted.output_bytes);
        store_json(report_json_out, &payload)?;
        Ok(0)
    })
}

/// Outcome of one `odc_import_mdx_to_odc` run.
struct ConvertedMdx {
    report: MdxImportReport,
    output_bytes: u64,
}

fn import_mdx_to_odc(mdx: &Path, out: &Path) -> Result<ConvertedMdx, String> {
    if out.exists() {
        return Err(format!("output file already exists: {}", out.display()));
    }
    let Some(name) = out.file_name().and_then(|name| name.to_str()) else {
        return Err(format!("output path has no file name: {}", out.display()));
    };
    let parent = out
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| format!("output path has no parent directory: {}", out.display()))?;
    if !parent.is_dir() {
        return Err(format!(
            "output directory does not exist: {}",
            parent.display()
        ));
    }

    // The working repository lives in the system temp directory and is removed
    // on every exit path: success, error and panic.
    let work = TemporaryDirectory::create()?;
    let mut repo = Repo::init(work.path().join("repo"))
        .map_err(|error| format!("cannot create a working repository: {error}"))?;
    let report = import_mdx(
        mdx,
        &mut repo,
        MdxImportOptions {
            skip_corrupt_blocks: true,
            ..MdxImportOptions::default()
        },
    )
    .map_err(|error| format!("import failed: {error}"))?;

    // `pack` exports what HEAD reaches, so the working set has to become a
    // commit first. An empty MDX file leaves nothing to commit.
    if !repo
        .status()
        .map_err(|error| format!("cannot inspect the working set: {error}"))?
        .is_clean()
    {
        let label = mdx
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("dictionary");
        repo.commit(
            &format!("import {label}"),
            odc_core::DEFAULT_AUTHOR,
            now_unix(),
        )
        .map_err(|error| format!("cannot commit the import: {error}"))?;
    }

    // Packing beside the destination keeps the final rename on one filesystem,
    // which is what makes it atomic.
    let staging = parent.join(format!(".{name}.partial-{}", unique_token()));
    let packed = match repo.pack(
        &staging,
        PackOptions {
            shallow: true,
            force: false,
        },
    ) {
        Ok(packed) => packed,
        Err(error) => {
            let _ = std::fs::remove_file(&staging);
            return Err(format!("cannot write {}: {error}", out.display()));
        }
    };
    drop(repo);

    if out.exists() {
        let _ = std::fs::remove_file(&staging);
        return Err(format!("output file already exists: {}", out.display()));
    }
    if let Err(error) = std::fs::rename(&staging, out) {
        let _ = std::fs::remove_file(&staging);
        return Err(format!("cannot move the import into place: {error}"));
    }

    Ok(ConvertedMdx {
        report,
        output_bytes: packed.bytes,
    })
}

/// Scratch directory removed when dropped, whatever the exit path.
struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn create() -> Result<Self, String> {
        let path = std::env::temp_dir().join(format!("odc-mdx-import-{}", unique_token()));
        std::fs::create_dir_all(&path).map_err(|error| {
            format!(
                "cannot create the temporary directory {}: {error}",
                path.display()
            )
        })?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Seconds since the Unix epoch, clamped to the `i64` range.
fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

/// Process-unique token used for scratch names.
fn unique_token() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    format!(
        "{}-{nanos}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

fn import_report_to_json(report: &MdxImportReport, output_bytes: u64) -> JsonValue {
    let corrupt: Vec<JsonValue> = report
        .corrupt_blocks
        .iter()
        .map(|block| {
            json!({
                "ordinal": block.ordinal,
                "file_offset": block.file_offset,
                "compressed_size": block.compressed_size,
                "declared_size": block.declared_size,
                "decompressed_size": block.decompressed_size,
                "decompressed_offset": block.decompressed_offset,
                "detail": block.detail,
            })
        })
        .collect();
    json!({
        "imported_entries": report.imported_entries,
        "written": report.written,
        "removed": report.removed,
        "skipped_blocks": report.skipped_blocks,
        "skipped_entries": report.skipped_entries,
        "corrupt_blocks": corrupt,
        "deduplicated": report.deduplicated,
        "skipped_empty": report.skipped_empty,
        "skipped_oversized": report.skipped_oversized,
        "working_set": report.working_set,
        "scan_warning": report.scan_warning.as_deref(),
        "header": {
            "engine_version": report.header.engine_version.as_str(),
            "required_engine_version": report.header.required_engine_version.as_str(),
            "encoding": report.header.encoding.as_str(),
            "encrypted": report.header.encrypted.as_deref(),
            "records": report.header.records,
        },
        "output_bytes": output_bytes,
    })
}

/// Store `value` in `slot` for the caller to free; a `NULL` slot releases it.
fn store_json(slot: *mut *mut c_char, value: &JsonValue) -> Result<(), String> {
    let pointer = to_c_string(value)?;
    if slot.is_null() {
        // SAFETY: `pointer` came from `CString::into_raw` above.
        unsafe { drop(CString::from_raw(pointer)) };
        return Ok(());
    }
    // SAFETY: the contract requires writable storage for one `char *`.
    unsafe { *slot = pointer };
    Ok(())
}

/// Run `body` with panic isolation and translate the outcome into the C
/// error convention.
fn guarded<T, F>(error_message: *mut *mut c_char, on_failure: T, body: F) -> T
where
    F: FnOnce() -> Result<T, String>,
{
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(Ok(value)) => {
            clear_error(error_message);
            value
        }
        Ok(Err(message)) => {
            report_error(error_message, &message);
            on_failure
        }
        Err(payload) => {
            report_error(error_message, &panic_text(payload.as_ref()));
            on_failure
        }
    }
}

fn clear_error(slot: *mut *mut c_char) {
    if slot.is_null() {
        return;
    }
    // SAFETY: the FFI contract requires `slot` to be a valid out pointer or
    // NULL, checked above.
    unsafe { *slot = ptr::null_mut() };
}

fn report_error(slot: *mut *mut c_char, message: &str) {
    if slot.is_null() {
        return;
    }
    let text = CString::new(message.replace('\0', " ")).unwrap_or_default();
    // SAFETY: as in `clear_error`; ownership of the string moves to the
    // caller, who releases it with `odc_string_free`.
    unsafe { *slot = text.into_raw() };
}

fn panic_text(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        format!("panic: {message}")
    } else if let Some(message) = payload.downcast_ref::<String>() {
        format!("panic: {message}")
    } else {
        "panic: non-string payload".to_string()
    }
}

/// # Safety
///
/// `repo` must be `NULL` or a live handle obtained from
/// [`odc_repo_open`] that is not closed and not used concurrently.
unsafe fn repo_ref<'a>(repo: *const OdcRepo) -> Result<&'a OdcRepo, String> {
    if repo.is_null() {
        return Err("repository handle is NULL".to_string());
    }
    // SAFETY: the caller guarantees the pointer is live for the duration of
    // the call and is used by one thread at a time.
    Ok(unsafe { &*repo })
}

/// # Safety
///
/// `value` must be `NULL` or a NUL-terminated C string valid for the
/// duration of the call.
unsafe fn text_arg(value: *const c_char, name: &str) -> Result<String, String> {
    if value.is_null() {
        return Err(format!("{name} must not be NULL"));
    }
    // SAFETY: non-NULL pointer to a NUL-terminated string per the contract.
    let text = unsafe { CStr::from_ptr(value) };
    text.to_str()
        .map(str::to_owned)
        .map_err(|_| format!("{name} is not valid UTF-8"))
}

fn to_c_string(value: &JsonValue) -> Result<*mut c_char, String> {
    let encoded = serde_json::to_string(value).map_err(|error| error.to_string())?;
    CString::new(encoded)
        .map(CString::into_raw)
        .map_err(|_| "JSON payload contains an interior NUL byte".to_string())
}

fn entry_to_json(entry: &Entry) -> JsonValue {
    let defs: Vec<JsonValue> = entry
        .defs
        .iter()
        .map(|def| {
            json!({
                "pos": def.pos,
                "sense": def.sense,
                "examples": def.examples,
                "labels": def.labels,
            })
        })
        .collect();
    let mut extra = Map::new();
    for (name, value) in &entry.extra {
        extra.insert(name.clone(), cbor_to_json(value));
    }
    json!({
        "key": entry.word,
        "display": entry.display,
        "phonetic": entry.phonetic,
        "defs": defs,
        "raw_html": entry.raw_html,
        "extra": JsonValue::Object(extra),
    })
}

/// Map a canonical CBOR value onto JSON.
///
/// Text-keyed maps become objects; any other map becomes an array of
/// `[key, value]` pairs. Byte strings become arrays of integers, since JSON
/// has no byte type.
fn cbor_to_json(value: &CborValue) -> JsonValue {
    match value {
        CborValue::Null => JsonValue::Null,
        CborValue::Bool(flag) => JsonValue::Bool(*flag),
        CborValue::Int(number) => JsonValue::from(*number),
        CborValue::Text(text) => JsonValue::String(text.clone()),
        CborValue::Bytes(bytes) => {
            JsonValue::Array(bytes.iter().map(|byte| JsonValue::from(*byte)).collect())
        }
        CborValue::Array(items) => JsonValue::Array(items.iter().map(cbor_to_json).collect()),
        CborValue::Map(pairs) => {
            let mut object = Map::new();
            let mut text_keys = true;
            for (key, value) in pairs {
                if let CborValue::Text(name) = key {
                    object.insert(name.clone(), cbor_to_json(value));
                } else {
                    text_keys = false;
                    break;
                }
            }
            if text_keys {
                JsonValue::Object(object)
            } else {
                JsonValue::Array(
                    pairs
                        .iter()
                        .map(|(key, value)| json!([cbor_to_json(key), cbor_to_json(value)]))
                        .collect(),
                )
            }
        }
    }
}
