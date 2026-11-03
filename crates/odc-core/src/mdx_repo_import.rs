//! Import an MDX file into a fresh directory repository.
//!
//! The MDX importer has always targeted a standalone pack (a single read-only
//! `.odc` file). The Apple apps need an editable library instead, so this
//! module wraps the same `import_mdx` pipeline around a directory repository
//! and commits the result. Callers get a report that mirrors the pack report
//! plus the repository path and the commit that holds the entries.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::hash;
use crate::mdx_import::{import_mdx, MdxImportOptions, MdxImportReport};
use crate::repo::{Repo, DEFAULT_AUTHOR};
use crate::Result;

/// Outcome of [`import_mdx_to_repo`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdxRepoImportReport {
    /// What the MDX pipeline reported.
    pub report: MdxImportReport,
    /// Directory that now holds `odc.db`.
    pub repo_path: PathBuf,
    /// Hex hash of the commit that holds the imported entries.
    pub commit: String,
}

/// Import `mdx` into the directory repository `repo_dir`.
///
/// `repo_dir` must be absent or an empty directory. The function initialises
/// the repository, imports entries and resources, and commits the working set
/// so `HEAD` always names the imported data. A directory created by this call
/// is removed again when the import fails, so a failed run never leaves
/// something that looks like a valid library behind.
///
/// # Errors
///
/// * `Error::AlreadyInitialized` when `repo_dir` already holds files.
/// * Every error `import_mdx` can produce.
/// * Every error the repository can produce while committing.
pub fn import_mdx_to_repo(
    mdx: &Path,
    repo_dir: &Path,
    options: MdxImportOptions,
) -> Result<MdxRepoImportReport> {
    let created = prepare_empty_directory(repo_dir)?;
    let outcome = build_repo(mdx, repo_dir, options);
    match outcome {
        Ok(report) => Ok(report),
        Err(error) => {
            if created {
                let _ = std::fs::remove_dir_all(repo_dir);
            }
            Err(error)
        }
    }
}

fn build_repo(
    mdx: &Path,
    repo_dir: &Path,
    options: MdxImportOptions,
) -> Result<MdxRepoImportReport> {
    let mut repo = Repo::init(repo_dir)?;
    let report = import_mdx(mdx, &mut repo, options)?;
    // An empty MDX file leaves the working set equal to the genesis commit; a
    // commit of an empty change is an error, so only commit when needed.
    if !repo.working_status()?.is_clean() {
        let label = mdx
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("dictionary");
        repo.commit(&format!("import {label}"), DEFAULT_AUTHOR, now_unix())?;
    }
    let head = repo
        .head()?
        .ok_or_else(|| Error::Inconsistent("import repository has no HEAD".to_string()))?;
    Ok(MdxRepoImportReport {
        report,
        repo_path: repo_dir.to_path_buf(),
        commit: hash::to_hex(&head),
    })
}

/// Outcome of [`copy_pack_to_repo`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackRepoConversion {
    /// Directory that now holds `odc.db`.
    pub repo_path: PathBuf,
    /// Hex hash of the pack's HEAD commit.
    pub commit: String,
    /// Entries reachable from HEAD.
    pub entries: usize,
}

/// Copy a single-file pack into `repo_dir` as an editable directory repository.
///
/// The pack is a SQLite database, so copying its bytes to `odc.db` keeps every
/// block, entry, resource and commit. What changes is how the copy is opened:
/// `Repo::open` only marks a repository read-only when the path is a file, so
/// the copy inside `repo_dir` is writable. The source file is never touched.
///
/// # Errors
///
/// * `Error::AlreadyInitialized` when `repo_dir` already holds files.
/// * `Error::Io` when the pack cannot be copied.
/// * `Error::NotARepository` when the copy does not open as a repository.
pub fn copy_pack_to_repo(pack: &Path, repo_dir: &Path) -> Result<PackRepoConversion> {
    let created = prepare_empty_directory(repo_dir)?;
    let outcome = copy_pack(pack, repo_dir);
    match outcome {
        Ok(report) => Ok(report),
        Err(error) => {
            if created {
                let _ = std::fs::remove_dir_all(repo_dir);
            }
            Err(error)
        }
    }
}

fn copy_pack(pack: &Path, repo_dir: &Path) -> Result<PackRepoConversion> {
    if !pack.is_file() {
        return Err(Error::NotARepository(pack.display().to_string()));
    }
    std::fs::copy(pack, repo_dir.join(crate::store::DB_FILE))?;
    let repo = Repo::open(repo_dir)?;
    if repo.is_read_only() {
        return Err(Error::ReadOnly(repo_dir.display().to_string()));
    }
    let head = repo
        .head()?
        .ok_or_else(|| Error::Inconsistent("pack has no HEAD".to_string()))?;
    let entries = repo.entry_count()?;
    Ok(PackRepoConversion {
        repo_path: repo_dir.to_path_buf(),
        commit: hash::to_hex(&head),
        entries,
    })
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// Make sure `dir` is absent or empty, and report whether it was created.
fn prepare_empty_directory(dir: &Path) -> Result<bool> {
    if dir.exists() {
        if !dir.is_dir() {
            return Err(Error::AlreadyInitialized(dir.display().to_string()));
        }
        let mut entries = std::fs::read_dir(dir)?;
        if entries.next().is_some() {
            return Err(Error::AlreadyInitialized(dir.display().to_string()));
        }
        Ok(false)
    } else {
        std::fs::create_dir_all(dir)?;
        Ok(true)
    }
}
