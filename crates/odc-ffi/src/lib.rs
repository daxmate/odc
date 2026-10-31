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
use std::path::Path;
use std::ptr;

use odc_core::cbor::Value as CborValue;
use odc_core::{Entry, Repo};
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
