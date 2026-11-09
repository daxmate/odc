//! Directory-shaped import entry points.
//!
//! The pack import stays the export format; the Apple apps open editable
//! libraries, so these calls produce a repository directory instead.

use std::ffi::c_char;
use std::os::raw::c_void;
use std::path::Path;

use serde_json::{json, Value as JsonValue};

use odc_core::{
    copy_pack_to_repo, import_mdx_to_repo, ImportProgress, MdxImportOptions,
    DEFAULT_MAX_RECORD_BYTES,
};

use crate::{
    clear_error, guarded, import_report_to_json, mdd_paths, profile_arg, store_json, text_arg,
    FfiProgress, OdcImportProgressFn,
};

/// Import an MDX file into a new directory repository.
///
/// `repo_dir` must be absent or empty; a directory that already holds files
/// is refused so an existing library is never overwritten. On success the
/// repository holds one commit with the imported entries and resources, and
/// the report carries the same fields as `odc_import_mdx_to_odc` plus
/// `repo_path`, `commit` and `is_directory`.
///
/// Returns `0` on success and `-1` on failure. On success `report_json_out`
/// receives a heap-allocated, NUL-terminated JSON report that the caller
/// releases with [`crate::odc_string_free`]. On failure `report_json_out` is
/// left `NULL` and `error_message` carries the diagnostic.
///
/// # Safety
///
/// `mdx_path`, `repo_dir` and `mdd_paths_json` must be `NULL` or
/// NUL-terminated UTF-8 strings; `report_json_out` and `error_message` must
/// be `NULL` or point to writable storage for one `char *`. `context` is
/// handed to `callback` untouched and must stay valid until this call returns.
#[no_mangle]
pub unsafe extern "C" fn odc_import_mdx_to_repo_with_mdd_progress(
    mdx_path: *const c_char,
    repo_dir: *const c_char,
    mdd_paths_json: *const c_char,
    callback: Option<OdcImportProgressFn>,
    context: *mut c_void,
    report_json_out: *mut *mut c_char,
    error_message: *mut *mut c_char,
) -> i32 {
    // SAFETY: forwarded unchanged; the contract is the one documented above.
    unsafe {
        import_mdx_into_repo(
            mdx_path,
            repo_dir,
            mdd_paths_json,
            std::ptr::null(),
            std::ptr::null(),
            callback,
            context,
            report_json_out,
            error_message,
        )
    }
}

/// Import an MDX file into a new directory repository with explicit options.
///
/// Same contract and report as
/// [`odc_import_mdx_to_repo_with_mdd_progress`], plus options control:
/// `options_json` is `NULL`, empty, or a JSON object such as
/// `{"max_record_bytes": 8388608}`. Unknown keys are rejected so a typo
/// never silently falls back to the default. The record limit defaults to
/// 4 MiB and may not exceed 16 MiB; a larger value fails with `ODC-E016`.
///
/// # Safety
///
/// Every pointer follows the contract of
/// [`odc_import_mdx_to_repo_with_mdd_progress`]; `options_json` must
/// additionally be `NULL` or a NUL-terminated UTF-8 JSON object.
#[no_mangle]
pub unsafe extern "C" fn odc_import_mdx_to_repo_with_options(
    mdx_path: *const c_char,
    repo_dir: *const c_char,
    mdd_paths_json: *const c_char,
    options_json: *const c_char,
    callback: Option<OdcImportProgressFn>,
    context: *mut c_void,
    report_json_out: *mut *mut c_char,
    error_message: *mut *mut c_char,
) -> i32 {
    // SAFETY: the pointers follow the contract documented above.
    unsafe {
        import_mdx_into_repo(
            mdx_path,
            repo_dir,
            mdd_paths_json,
            options_json,
            std::ptr::null(),
            callback,
            context,
            report_json_out,
            error_message,
        )
    }
}

/// Import an MDX file with a profile into a new directory repository,
/// reporting progress.
///
/// Same contract and report as [`odc_import_mdx_to_repo_with_mdd_progress`],
/// plus `profile_path`: `NULL` for an import without a profile, or a
/// NUL-terminated path to a `*.profile.json` document. The profile decides the
/// L1 body of every record it claims; unclaimed records keep the built-in HTML
/// pass and count as unadapted. The report gains `"contract"` (null without a
/// profile), `p0_failed` and `p0_failed_samples`.
///
/// # Safety
///
/// Every pointer follows the contract of
/// [`odc_import_mdx_to_repo_with_mdd_progress`]; `profile_path` must
/// additionally be `NULL` or a NUL-terminated UTF-8 path.
#[no_mangle]
pub unsafe extern "C" fn odc_import_mdx_to_repo_with_profile_progress(
    mdx_path: *const c_char,
    repo_dir: *const c_char,
    mdd_paths_json: *const c_char,
    profile_path: *const c_char,
    callback: Option<OdcImportProgressFn>,
    context: *mut c_void,
    report_json_out: *mut *mut c_char,
    error_message: *mut *mut c_char,
) -> i32 {
    // SAFETY: forwarded unchanged; the contract is the one documented above.
    unsafe {
        import_mdx_into_repo(
            mdx_path,
            repo_dir,
            mdd_paths_json,
            std::ptr::null(),
            profile_path,
            callback,
            context,
            report_json_out,
            error_message,
        )
    }
}

/// Shared body of the directory-repository import entry points.
///
/// # Safety
///
/// Every pointer follows the contract of the public wrappers above.
#[allow(clippy::too_many_arguments)]
unsafe fn import_mdx_into_repo(
    mdx_path: *const c_char,
    repo_dir: *const c_char,
    mdd_paths_json: *const c_char,
    options_json: *const c_char,
    profile_path: *const c_char,
    callback: Option<OdcImportProgressFn>,
    context: *mut c_void,
    report_json_out: *mut *mut c_char,
    error_message: *mut *mut c_char,
) -> i32 {
    // A stale report pointer must never survive a failed import.
    clear_error(report_json_out);
    guarded(error_message, -1, || {
        // SAFETY: the pointers follow the contract documented above.
        let mdx = unsafe { text_arg(mdx_path, "mdx_path") }?;
        // SAFETY: as above.
        let repo_dir = unsafe { text_arg(repo_dir, "repo_dir") }?;
        // SAFETY: as above.
        let volumes = unsafe { mdd_paths(mdd_paths_json) }?;
        // SAFETY: as above.
        let max_record_bytes = unsafe { import_options(options_json) }?;
        // SAFETY: as above.
        let profile = unsafe { profile_arg(profile_path) }?;
        let progress = callback.map(|callback| {
            Box::new(move |progress: ImportProgress| {
                let mut sink = FfiProgress { callback, context };
                sink.send(progress);
            }) as Box<dyn FnMut(ImportProgress)>
        });
        let imported = import_mdx_to_repo(
            Path::new(&mdx),
            Path::new(&repo_dir),
            MdxImportOptions {
                skip_corrupt_blocks: true,
                max_record_bytes,
                mdd_volumes: volumes,
                progress,
                profile,
                ..MdxImportOptions::default()
            },
        )
        .map_err(|error| format!("import failed: {error}"))?;
        let mut payload = import_report_to_json(&imported.report, 0);
        if let Some(object) = payload.as_object_mut() {
            object.insert(
                "repo_path".to_string(),
                json!(imported.repo_path.to_string_lossy()),
            );
            object.insert("commit".to_string(), json!(imported.commit));
            object.insert("is_directory".to_string(), json!(true));
        }
        store_json(report_json_out, &payload)?;
        Ok(0)
    })
}

/// Parse `options_json` for [`odc_import_mdx_to_repo_with_options`].
///
/// `NULL` or an empty string selects the default record limit. Unknown keys
/// are rejected: silently ignoring a typo would hide a misconfigured limit.
/// The value itself is range-checked by `import_mdx` (`ODC-E016`).
///
/// # Safety
///
/// `value` must be `NULL` or a NUL-terminated UTF-8 string holding a JSON
/// object.
unsafe fn import_options(value: *const c_char) -> Result<usize, String> {
    if value.is_null() {
        return Ok(DEFAULT_MAX_RECORD_BYTES);
    }
    let text = unsafe { text_arg(value, "options_json") }?;
    if text.trim().is_empty() {
        return Ok(DEFAULT_MAX_RECORD_BYTES);
    }
    let parsed: JsonValue = serde_json::from_str(&text)
        .map_err(|error| format!("options_json is not JSON: {error}"))?;
    let object = parsed
        .as_object()
        .ok_or_else(|| "options_json must be a JSON object".to_string())?;
    for key in object.keys() {
        if key != "max_record_bytes" {
            return Err(format!("unknown import option: {key}"));
        }
    }
    let Some(raw) = object.get("max_record_bytes") else {
        return Ok(DEFAULT_MAX_RECORD_BYTES);
    };
    let number = raw
        .as_u64()
        .ok_or_else(|| "max_record_bytes must be a non-negative integer".to_string())?;
    usize::try_from(number).map_err(|_| format!("max_record_bytes is too large: {number}"))
}

/// Copy a single-file pack into a new directory repository.
///
/// The source pack is never modified, and `repo_dir` must be absent or empty.
/// The report is `{"repo_path", "commit", "entries", "is_directory"}`.
///
/// Returns `0` on success and `-1` on failure, following the same pointer
/// contract as [`odc_import_mdx_to_repo_with_mdd_progress`].
///
/// # Safety
///
/// `pack_path` and `repo_dir` must be `NULL` or NUL-terminated UTF-8 strings;
/// `report_json_out` and `error_message` must be `NULL` or point to writable
/// storage for one `char *`.
#[no_mangle]
pub unsafe extern "C" fn odc_copy_pack_to_repo(
    pack_path: *const c_char,
    repo_dir: *const c_char,
    report_json_out: *mut *mut c_char,
    error_message: *mut *mut c_char,
) -> i32 {
    clear_error(report_json_out);
    guarded(error_message, -1, || {
        // SAFETY: the pointers follow the contract documented above.
        let pack = unsafe { text_arg(pack_path, "pack_path") }?;
        // SAFETY: as above.
        let repo_dir = unsafe { text_arg(repo_dir, "repo_dir") }?;
        let converted = copy_pack_to_repo(Path::new(&pack), Path::new(&repo_dir))
            .map_err(|error| format!("cannot convert the pack: {error}"))?;
        let payload = json!({
            "repo_path": converted.repo_path.to_string_lossy(),
            "commit": converted.commit,
            "entries": converted.entries,
            "is_directory": true,
        });
        store_json(report_json_out, &payload)?;
        Ok(0)
    })
}
