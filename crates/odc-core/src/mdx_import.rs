//! MDX import pipeline shared by the CLI and (later) the FFI layer.
//!
//! The entry point is `import_mdx`: it scans version 2 files for corrupt
//! record blocks, imports through a repaired temporary copy when needed, drops
//! the sentinel entries, and reports what happened. The CLI used to carry this
//! logic in `main.rs`; it now only prints the returned
//! `MdxImportReport`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::html::{self, HtmlImportSummary, HtmlMode, HtmlOptions};
use crate::mdd_import::{self, MddImportReport, MddScan, MddVolumeReport};
use crate::mdx::{
    entry_from_record_with_limit, validate_max_record_bytes, MdxHeader, MdxKeys, MdxSource,
    DEFAULT_MAX_RECORD_BYTES,
};
use crate::mdx_scan::{self, CorruptBlock};
use crate::repo::Repo;
use crate::stylesheets;

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
    /// Largest record body accepted, in bytes.
    ///
    /// Defaults to [`DEFAULT_MAX_RECORD_BYTES`] (4 MiB) and may be raised up
    /// to [`HARD_MAX_RECORD_BYTES`] (16 MiB). Records above the limit are
    /// skipped and listed in
    /// [`MdxImportReport::skipped_oversized_samples`].
    pub max_record_bytes: usize,
    /// How entry bodies are repaired, sanitised and formatted.
    ///
    /// Defaults to [`HtmlMode::Format`]: MDX imports normalise HTML unless the
    /// caller opts out.
    pub html: HtmlOptions,
    /// Called with samples of the import progress.
    ///
    /// Samples are stage-scoped: `done` and `total` count records of the
    /// stage they belong to (see [`ImportProgress`]). The callback runs on the
    /// importing thread and must stay cheap; the core already limits it to
    /// roughly ten samples per second, plus every stage change and the final
    /// `done == total` sample.
    pub progress: Option<Box<dyn FnMut(ImportProgress)>>,
    /// Import the sibling MDD volumes (default `true`).
    ///
    /// The volumes are `<stem>.mdd`, `<stem>.1.mdd`, `<stem>.2.mdd` … next
    /// to the MDX file; a volume that cannot be read only produces a warning.
    pub import_mdd: bool,
    /// Explicit MDD volumes to read, in import order.
    ///
    /// `None` discovers the volumes next to the MDX file. A caller that
    /// already listed the files (a sandboxed app, for instance) passes them
    /// here; unreadable entries still only produce warnings.
    pub mdd_volumes: Option<Vec<PathBuf>>,
}

impl Default for MdxImportOptions {
    fn default() -> Self {
        Self {
            skip_corrupt_blocks: true,
            prune: false,
            limit: None,
            max_record_bytes: DEFAULT_MAX_RECORD_BYTES,
            html: HtmlOptions::default(),
            progress: None,
            import_mdd: true,
            mdd_volumes: None,
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
            .field("max_record_bytes", &self.max_record_bytes)
            .field("progress", &self.progress.is_some())
            .field("import_mdd", &self.import_mdd)
            .field("mdd_volumes", &self.mdd_volumes)
            .finish()
    }
}

/// Stage of an `import_mdx` call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportStage {
    /// Opening the MDD volumes to learn how many resources exist.
    Preparing,
    /// Reading dictionary records into the working set.
    Entries,
    /// Copying MDD payloads into the block store.
    Resources,
    /// Every record and resource is written; committing and packing follow
    /// in the caller.
    Committing,
}

impl ImportStage {
    /// Stable numeric identifier, mirrored by `odc.h`.
    pub const fn code(self) -> i32 {
        match self {
            Self::Preparing => 0,
            Self::Entries => 1,
            Self::Resources => 2,
            Self::Committing => 3,
        }
    }
}

/// One progress sample from [`MdxImportOptions::progress`].
///
/// `done` and `total` are **stage-scoped counts of records**, never bytes:
///
/// * [`ImportStage::Preparing`] — `done` is `0` and `total` is the number of
///   resources the MDD pre-scan found (`0` when there are no volumes).
/// * [`ImportStage::Entries`] — `total` is `min(limit, records in the file)`
///   and `done` counts processed records, including the empty, oversized and
///   duplicated ones, so the last sample satisfies `done == total`.
/// * [`ImportStage::Resources`] — `done` and `total` count MDD resources,
///   `volume_index` is one-based and `volume_count` is the number of readable
///   volumes. A volume that fails halfway is counted as processed so the
///   sample never moves backwards.
/// * [`ImportStage::Committing`] — `done == total`, the sum of the entry and
///   resource totals.
///
/// A caller that wants a single fraction keeps the totals it has seen:
/// `(entries_done + resources_done) / (entries_total + resources_total)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportProgress {
    /// Which part of the import produced this sample.
    pub stage: ImportStage,
    /// Records processed so far in this stage.
    pub done: u64,
    /// Records this stage will process in total.
    pub total: u64,
    /// One-based index of the volume being imported.
    pub volume_index: u32,
    /// Number of readable volumes; `0` outside the resources stage.
    pub volume_count: u32,
}

/// Rate limiter for [`MdxImportOptions::progress`].
///
/// Stage changes and complete samples always go out; everything else is
/// capped at about ten samples per second so that a dictionary with 180k
/// resources cannot flood a UI thread.
struct ProgressLimiter {
    last_sent: Option<Instant>,
    last_stage: Option<ImportStage>,
}

impl ProgressLimiter {
    /// Shortest gap between two ordinary samples.
    const MIN_INTERVAL: Duration = Duration::from_millis(100);

    fn new() -> Self {
        Self {
            last_sent: None,
            last_stage: None,
        }
    }

    fn allows(&mut self, sample: &ImportProgress) -> bool {
        self.allows_at(Instant::now(), sample)
    }

    fn allows_at(&mut self, now: Instant, sample: &ImportProgress) -> bool {
        let stage_changed = self.last_stage != Some(sample.stage);
        let complete = sample.total > 0 && sample.done >= sample.total;
        let elapsed = match self.last_sent {
            Some(last) => now.duration_since(last) >= Self::MIN_INTERVAL,
            None => true,
        };
        if stage_changed || complete || elapsed {
            self.last_sent = Some(now);
            self.last_stage = Some(sample.stage);
            true
        } else {
            false
        }
    }
}

/// Delivers samples to the caller, applying [`ProgressLimiter`].
struct ProgressSink {
    callback: Option<Box<dyn FnMut(ImportProgress)>>,
    limiter: ProgressLimiter,
}

impl ProgressSink {
    fn new(callback: Option<Box<dyn FnMut(ImportProgress)>>) -> Self {
        Self {
            callback,
            limiter: ProgressLimiter::new(),
        }
    }

    fn send(&mut self, sample: ImportProgress) {
        if self.callback.is_none() || !self.limiter.allows(&sample) {
            return;
        }
        if let Some(callback) = self.callback.as_mut() {
            callback(sample);
        }
    }
}

/// One record dropped because its body exceeded `max_record_bytes`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OversizedRecord {
    /// Key of the skipped record, trimmed as stored in the file.
    pub key: String,
    /// Body size in bytes, as read from the file.
    pub bytes: usize,
}

/// How many oversized records are listed in
/// [`MdxImportReport::skipped_oversized_samples`].
///
/// [`MdxImportReport::skipped_oversized`] still carries the full count.
pub const SKIPPED_OVERSIZED_LIST_LIMIT: usize = 100;
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
    /// Details of the oversized records, at most
    /// [`SKIPPED_OVERSIZED_LIST_LIMIT`] entries in file order.
    pub skipped_oversized_samples: Vec<OversizedRecord>,
    /// Working set size after the import.
    pub working_set: usize,
    /// Header metadata of the imported file.
    pub header: MdxHeader,
    /// Why the record section could not be inspected, when that happened.
    ///
    /// The import then read the original file unchanged, so corrupt-block
    /// recovery was unavailable for that file.
    pub scan_warning: Option<String>,
    /// MDD volumes imported next to the MDX file, in import order.
    pub mdd_files: Vec<MddVolumeReport>,
    /// Resources reachable from the entries after the import.
    pub resource_count: usize,
    /// Payload bytes of those resources.
    pub resource_bytes: u64,
    /// Non-fatal problems: unreadable or corrupt MDD volumes, per line.
    pub warnings: Vec<String>,
    /// Stylesheets copied from beside the .mdx because the volumes lacked
    /// them; zero when every sheet came out of the .mdd.
    pub stylesheets: usize,
    /// HTML repair, sanitiser and formatter tallies for this import.
    pub html: HtmlImportSummary,
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
        max_record_bytes,
        progress,
        import_mdd,
        mdd_volumes,
        html,
    } = options;
    validate_max_record_bytes(max_record_bytes)?;
    let mut reporter = ProgressSink::new(progress);
    let mut html_summary = HtmlImportSummary::default();

    // The repaired copy owns a temporary file that is removed when dropped, so
    // it stays alive until the import (including error paths) has finished.
    let mut scan_warning = None;
    let repair = if skip_corrupt_blocks {
        match mdx_scan::scan(path)? {
            mdx_scan::ScanOutcome::Scanned(section) if section.has_corrupt_blocks() => {
                let copy = mdx_scan::write_repaired_copy(path, &section)?;
                let mdx_scan::RecordSection { corrupt, .. } = *section;
                Some((copy, corrupt))
            }
            // A record section we cannot locate is not an import failure: read
            // the original file exactly as ODC did before this feature.
            mdx_scan::ScanOutcome::Unscannable(detail) => {
                scan_warning = Some(detail);
                None
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

    // The MDD volumes are opened before the first record so the caller learns
    // the size of the whole job up front and can show one determinate bar from
    // the first sample. Opening a volume only parses its key table, no payload
    // is read, so the pre-scan stays cheap.
    let mut warnings = Vec::new();
    let mdd_paths = if import_mdd {
        Some(mdd_volumes.unwrap_or_else(|| mdd_import::discover_volumes(path)))
    } else {
        None
    };
    let scan = match &mdd_paths {
        Some(volumes) => mdd_import::scan_volumes(volumes),
        None => MddScan::default(),
    };
    warnings.extend(scan.warnings.iter().cloned());
    reporter.send(ImportProgress {
        stage: ImportStage::Preparing,
        done: 0,
        total: scan.total,
        volume_index: 0,
        volume_count: scan.volumes.len() as u32,
    });
    let mut records = source.records();
    let mut keys = MdxKeys::new();
    let mut taken = 0_usize;
    let mut skipped_empty = 0_usize;
    let mut skipped_oversized = 0_usize;
    let mut skipped_oversized_samples: Vec<OversizedRecord> = Vec::new();
    let mut skipped_entries = 0_usize;
    let mut deduplicated = 0_usize;
    let mut processed = 0_u64;
    let entries_total = limit.map_or(header.records, |cap| (cap as u64).min(header.records));

    let mut style_keys = std::collections::BTreeSet::new();
    let report = repo.import_stream(
        || loop {
            if limit.is_some_and(|cap| taken >= cap) {
                return Ok(None);
            }
            let Some(record) = records.next() else {
                return Ok(None);
            };
            let mut record = record?;
            processed += 1;
            reporter.send(ImportProgress {
                stage: ImportStage::Entries,
                done: processed.min(entries_total),
                total: entries_total,
                volume_index: 0,
                volume_count: 0,
            });
            if record.key.trim().is_empty() {
                skipped_empty += 1;
                continue;
            }
            if skipped_blocks > 0 && mdx_scan::is_sentinel_payload(&record.html) {
                skipped_entries += 1;
                continue;
            }
            // The sentinel check ran on the original payload; from here on the
            // body is a normal entry and may be repaired, sanitised and
            // formatted. A formatting failure keeps the original text.
            if html.mode != HtmlMode::Off {
                let outcome = html::process_html(&record.html, &html);
                let changed = outcome.changed && !outcome.failed;
                if changed {
                    record.html.clone_from(&outcome.html);
                }
                html_summary.record(record.key.trim(), &outcome, changed, html.max_samples);
            }
            let mut entry = match entry_from_record_with_limit(&record, max_record_bytes) {
                Ok(entry) => entry,
                Err(Error::EntryTooLarge { word, bytes, .. }) => {
                    skipped_oversized += 1;
                    if skipped_oversized_samples.len() < SKIPPED_OVERSIZED_LIST_LIMIT {
                        skipped_oversized_samples.push(OversizedRecord { key: word, bytes });
                    }
                    continue;
                }
                Err(error) => return Err(error),
            };
            let (key, collided) = keys.allocate(&entry.word);
            entry.word = key;
            if collided {
                deduplicated += 1;
            }
            for key in stylesheets::link_targets(entry.raw_html.as_deref().unwrap_or_default()) {
                style_keys.insert(key);
            }
            taken += 1;
            return Ok(Some(entry));
        },
        prune,
        |_| {},
    )?;

    // Resources are written after the entries, in the same call: a failed MDX
    // import therefore never leaves a resource set behind. Volumes commit one
    // batch at a time, so a corrupt volume keeps what it already wrote.
    let mdd = match &mdd_paths {
        Some(_) => mdd_import::import_scanned(repo, &scan, &mut warnings, |sample| {
            reporter.send(ImportProgress {
                stage: ImportStage::Resources,
                done: sample.done,
                total: sample.total,
                volume_index: sample.volume_index,
                volume_count: sample.volume_count,
            });
        })?,
        None => MddImportReport::default(),
    };

    // Sheets the volumes do not carry are read from beside the .mdx; without
    // this an entry that links its CSS out of band renders unstyled.
    let imported_stylesheets = import_sibling_stylesheets(path, &style_keys, repo)?;

    // Everything the core owns is written; committing the working set and
    // packing the container happen in the caller, hence a stage of their own.
    let complete = entries_total + scan.total;
    reporter.send(ImportProgress {
        stage: ImportStage::Committing,
        done: complete,
        total: complete,
        volume_index: 0,
        volume_count: 0,
    });

    Ok(MdxImportReport {
        stylesheets: imported_stylesheets,
        imported_entries: taken,
        written: report.written,
        removed: report.removed,
        skipped_blocks,
        skipped_entries,
        corrupt_blocks,
        deduplicated,
        skipped_empty,
        skipped_oversized,
        skipped_oversized_samples,
        working_set: report.total,
        header,
        scan_warning,
        mdd_files: mdd.volumes,
        resource_count: mdd.resources,
        resource_bytes: mdd.resource_bytes,
        warnings,
        html: html_summary,
    })
}

/// Copies stylesheets the entries link to but the MDD volumes do not carry.
///
/// MDict keeps CSS next to the .mdx as often as inside the .mdd; the volumes
/// are authoritative, so a sibling file only fills a gap.
fn import_sibling_stylesheets(
    mdx: &Path,
    keys: &std::collections::BTreeSet<String>,
    repo: &mut Repo,
) -> Result<usize> {
    let Some(dir) = mdx.parent() else {
        return Ok(0);
    };
    let mut imported = 0usize;
    for key in keys {
        if repo.resource(key)?.is_some() {
            continue;
        }
        let candidate = dir.join(key);
        if !candidate.is_file() {
            continue;
        }
        if let Ok(bytes) = std::fs::read(&candidate) {
            repo.put_resource(key, &bytes)?;
            imported += 1;
        }
    }
    Ok(imported)
}
