//! MDX import pipeline shared by the CLI and (later) the FFI layer.
//!
//! The entry point is `import_mdx`: it scans version 2 files for corrupt
//! record blocks, imports through a repaired temporary copy when needed, drops
//! the sentinel entries, and reports what happened. The CLI used to carry this
//! logic in `main.rs`; it now only prints the returned
//! `MdxImportReport`.

use std::path::Path;

use crate::error::{Error, Result};
use crate::mdx::{entry_from_record, MdxHeader, MdxKeys, MdxSource};
use crate::mdx_scan::{self, CorruptBlock};
use crate::repo::Repo;

/// Options for `import_mdx`.
pub struct MdxImportOptions {
    /// Repair corrupt record blocks instead of aborting (default `true`).
    ///
    /// Only version 2 files can be repaired; version 1.2 files always use the
    /// strict path documented in `docs/mdx-compat.md`.
    pub skip_corrupt_blocks: bool,
    /// Remove working-set entries missing from the import.
    pub prune: bool,
    /// Stop after this many accepted entries.
    pub limit: Option<usize>,
    /// Called with the number of entries written so far.
    pub progress: Option<Box<dyn FnMut(usize)>>,
}

impl Default for MdxImportOptions {
    fn default() -> Self {
        Self {
            skip_corrupt_blocks: true,
            prune: false,
            limit: None,
            progress: None,
        }
    }
}

impl std::fmt::Debug for MdxImportOptions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MdxImportOptions")
            .field("skip_corrupt_blocks", &self.skip_corrupt_blocks)
            .field("prune", &self.prune)
            .field("limit", &self.limit)
            .field("progress", &self.progress.is_some())
            .finish()
    }
}

/// Outcome of one `import_mdx` call.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MdxImportReport {
    /// Entries accepted from the file (after dropping empty, oversized and
    /// sentinel records).
    pub imported_entries: usize,
    /// Rows inserted or replaced in the working set.
    pub written: usize,
    /// Rows removed by `prune`.
    pub removed: usize,
    /// Number of record blocks that had to be replaced.
    pub skipped_blocks: usize,
    /// Number of entries that lived inside replaced record blocks.
    pub skipped_entries: usize,
    /// Details for every replaced record block.
    pub corrupt_blocks: Vec<CorruptBlock>,
    /// Keys that collided after normalisation and were suffixed.
    pub deduplicated: usize,
    /// Records dropped because their key was empty.
    pub skipped_empty: usize,
    /// Records dropped because they exceeded the record size limit.
    pub skipped_oversized: usize,
    /// Working set size after the import.
    pub working_set: usize,
    /// Header metadata of the imported file.
    pub header: MdxHeader,
}

/// Import an MDX file into `repo`.
///
/// # Errors
///
/// * `Error::Mdx` when the file cannot be opened, or when a version 2
///   record section does not match the documented layout.
/// * Any error produced by the repository while writing entries.
pub fn import_mdx(
    path: &Path,
    repo: &mut Repo,
    options: MdxImportOptions,
) -> Result<MdxImportReport> {
    let MdxImportOptions {
        skip_corrupt_blocks,
        prune,
        limit,
        mut progress,
    } = options;

    // The repaired copy owns a temporary file that is removed when dropped, so
    // it stays alive until the import (including error paths) has finished.
    let repair = if skip_corrupt_blocks {
        match mdx_scan::scan(path)? {
            Some(section) if section.has_corrupt_blocks() => {
                let copy = mdx_scan::write_repaired_copy(path, &section)?;
                Some((copy, section.corrupt))
            }
            _ => None,
        }
    } else {
        None
    };
    let (corrupt_blocks, source_path) = match &repair {
        Some((copy, corrupt)) => (corrupt.clone(), copy.path()),
        None => (Vec::new(), path),
    };
    let skipped_blocks = corrupt_blocks.len();

    let source = MdxSource::open(source_path)?;
    let header = source.header();
    let mut records = source.records();
    let mut keys = MdxKeys::new();
    let mut taken = 0_usize;
    let mut skipped_empty = 0_usize;
    let mut skipped_oversized = 0_usize;
    let mut skipped_entries = 0_usize;
    let mut deduplicated = 0_usize;

    let report = repo.import_stream(
        || loop {
            if limit.is_some_and(|cap| taken >= cap) {
                return Ok(None);
            }
            let Some(record) = records.next() else {
                return Ok(None);
            };
            let record = record?;
            if record.key.trim().is_empty() {
                skipped_empty += 1;
                continue;
            }
            if skipped_blocks > 0 && mdx_scan::is_sentinel_payload(&record.html) {
                skipped_entries += 1;
                continue;
            }
            let mut entry = match entry_from_record(&record) {
                Ok(entry) => entry,
                Err(Error::EntryTooLarge { .. }) => {
                    skipped_oversized += 1;
                    continue;
                }
                Err(error) => return Err(error),
            };
            let (key, collided) = keys.allocate(&entry.word);
            entry.word = key;
            if collided {
                deduplicated += 1;
            }
            taken += 1;
            return Ok(Some(entry));
        },
        prune,
        |written| {
            if let Some(callback) = progress.as_mut() {
                callback(written);
            }
        },
    )?;

    Ok(MdxImportReport {
        imported_entries: taken,
        written: report.written,
        removed: report.removed,
        skipped_blocks,
        skipped_entries,
        corrupt_blocks,
        deduplicated,
        skipped_empty,
        skipped_oversized,
        working_set: report.total,
        header,
    })
}
