//! Directory-shaped import entry points.
//!
//! The pack import stays the export format; the Apple apps open editable
//! libraries, so these calls produce a repository directory instead.

use std::ffi::c_char;
use std::os::raw::c_void;
use std::path::Path;

use serde_json::json;

use odc_core::{copy_pack_to_repo, import_mdx_to_repo, ImportProgress, MdxImportOptions};

use crate::{
    clear_error, guarded, import_report_to_json, mdd_paths, store_json, text_arg, FfiProgress,
    OdcImportProgressFn,
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
    // A stale report pointer must never survive a failed import.
    clear_error(report_json_out);
    guarded(error_message, -1, || {
        // SAFETY: the pointers follow the contract documented above.
        let mdx = unsafe { text_arg(mdx_path, "mdx_path") }?;
        // SAFETY: as above.
        let repo_dir = unsafe { text_arg(repo_dir, "repo_dir") }?;
        // SAFETY: as above.
        let volumes = unsafe { mdd_paths(mdd_paths_json) }?;
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
                mdd_volumes: volumes,
                progress,
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
