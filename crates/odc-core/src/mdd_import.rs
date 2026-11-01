//! MDD (MDict resource) import.
//!
//! An MDX dictionary keeps its images, audio clips, stylesheets and fonts in
//! one or more sibling MDD files: `base.mdd`, `base.1.mdd`, `base.2.mdd` and
//! so on. This module discovers those volumes, copies every payload into the
//! repository block store and records the normalised resource path, so the
//! reader can resolve `\\pic\\apple.png` without the original files.
//!
//! The parser itself lives in the `mdictlib` crate; nothing here re-implements
//! the MDD container.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use mdictlib::MddFile;

use crate::error::Result;
use crate::repo::Repo;

/// Resources written per transaction; keeps peak memory flat on huge volumes.
const RESOURCE_BATCH: usize = 256;

/// One MDD volume that was imported.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MddVolumeReport {
    /// File the resources came from.
    pub path: PathBuf,
    /// Resources accepted from this volume.
    pub entries: usize,
    /// Payload bytes accepted from this volume.
    pub bytes: u64,
}

/// Outcome of importing the MDD volumes that sit next to an MDX file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MddImportReport {
    /// Volumes imported in order: main volume first, then ascending numbers.
    pub volumes: Vec<MddVolumeReport>,
    /// Resources written across every volume.
    pub resources: usize,
    /// Payload bytes written across every volume.
    pub resource_bytes: u64,
    /// Volumes (or resources) that were skipped, one line each.
    pub warnings: Vec<String>,
}

/// One progress sample from [`import_scanned`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MddProgress {
    /// Physical resources processed across all volumes so far.
    pub done: u64,
    /// Physical resources in the readable volumes.
    pub total: u64,
    /// One-based index of the volume being imported.
    pub volume_index: u32,
    /// Number of readable volumes.
    pub volume_count: u32,
}

/// Totals learned by opening the volumes without reading payloads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MddScan {
    /// Readable volumes and their physical resource counts, in import order.
    pub volumes: Vec<(PathBuf, u64)>,
    /// Sum of the counts above.
    pub total: u64,
    /// Problems found while opening the volumes, one line each.
    pub warnings: Vec<String>,
}

/// Normalise an MDD resource path for storage and lookup.
///
/// MDD keys use backslashes and often carry a leading slash; the repository
/// stores one canonical form so `\\pic\\apple.png`, `/pic/apple.png` and
/// `pic/apple.png` all resolve to the same row. Case is preserved: the
/// lookup does the case-insensitive comparison, not the index.
#[must_use]
pub fn normalize_resource_key(raw: &str) -> String {
    let mut key = raw.replace('\\', "/");
    while let Some(rest) = key.strip_prefix("./") {
        key = rest.to_string();
    }
    key.trim_start_matches('/').to_string()
}

/// Volume files that belong to `mdx`, in import order.
///
/// The main volume (`<stem>.mdd`) comes first, then the numbered ones in
/// ascending numeric order, so `.10` sorts after `.2`.
#[must_use]
pub fn discover_volumes(mdx: &Path) -> Vec<PathBuf> {
    let Some(stem) = mdx.file_stem().and_then(|stem| stem.to_str()) else {
        return Vec::new();
    };
    let Some(directory) = mdx.parent() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut main: Option<PathBuf> = None;
    let mut numbered: Vec<(u32, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        match volume_number(name, stem) {
            Some(0) => {
                if main.is_none() {
                    main = Some(path);
                }
            }
            Some(number) => numbered.push((number, path)),
            None => {}
        }
    }
    numbered.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    let mut volumes: Vec<PathBuf> = main.into_iter().collect();
    volumes.extend(numbered.into_iter().map(|(_, path)| path));
    volumes
}

/// Volume number encoded in `name` for the dictionary called `stem`.
///
/// Returns `0` for the main volume, the suffix number for `stem.N.mdd`, and
/// `None` when the file does not belong to this dictionary.
fn volume_number(name: &str, stem: &str) -> Option<u32> {
    let lowered = name.to_lowercase();
    let base = lowered.strip_suffix(".mdd")?;
    let expected = stem.to_lowercase();
    if base == expected {
        return Some(0);
    }
    let suffix = base.strip_prefix(&format!("{expected}."))?;
    if suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    suffix.parse::<u32>().ok()
}

/// Import every volume in `volumes` into `repo`.
///
/// A volume that cannot be opened — or that fails halfway — is reported as a
/// warning and skipped; the resources it wrote before failing stay, because
/// each batch commits on its own. Duplicate keys keep the first payload seen,
/// which is what makes the main volume win over the numbered ones.
///
/// # Errors
///
/// Only repository failures are fatal; MDD problems never abort an import.
pub fn import_volumes(
    repo: &mut Repo,
    volumes: &[PathBuf],
    warnings: &mut Vec<String>,
) -> Result<MddImportReport> {
    let scan = scan_volumes(volumes);
    warnings.extend(scan.warnings.iter().cloned());
    import_scanned(repo, &scan, warnings, |_| {})
}

/// Open every volume without reading payloads to learn the resource totals.
///
/// A volume that cannot be opened becomes a warning and is left out of the
/// totals, exactly like [`import_scanned`] would skip it. Parsing the key
/// table is all this does, so a 180k-resource volume costs milliseconds.
pub fn scan_volumes(volumes: &[PathBuf]) -> MddScan {
    let mut scan = MddScan::default();
    for path in volumes {
        match MddFile::open(path) {
            Ok(file) => {
                let resources = file.len();
                scan.total += resources;
                scan.volumes.push((path.clone(), resources));
            }
            Err(error) => scan.warnings.push(format!("{}: {error}", path.display())),
        }
    }
    scan
}

/// Import the volumes a [`scan_volumes`] call already opened.
///
/// `progress` receives one sample per volume switch and one per written batch.
/// A volume that fails halfway is counted as if its scanned resources were
/// processed, so the sample never stalls and never moves backwards.
///
/// # Errors
///
/// Only repository failures are fatal; MDD problems never abort an import.
pub fn import_scanned<F>(
    repo: &mut Repo,
    scan: &MddScan,
    warnings: &mut Vec<String>,
    mut progress: F,
) -> Result<MddImportReport>
where
    F: FnMut(MddProgress),
{
    let volume_count = u32::try_from(scan.volumes.len()).unwrap_or(u32::MAX);
    let mut report = MddImportReport::default();
    let mut seen: HashSet<String> = HashSet::new();
    let mut done = 0_u64;
    for (position, (path, resources)) in scan.volumes.iter().enumerate() {
        let volume_index = u32::try_from(position + 1).unwrap_or(u32::MAX);
        progress(MddProgress {
            done,
            total: scan.total,
            volume_index,
            volume_count,
        });
        let base = done;
        match import_volume(repo, path, &mut seen, &mut |processed| {
            progress(MddProgress {
                done: base + processed,
                total: scan.total,
                volume_index,
                volume_count,
            });
        }) {
            Ok(volume) => {
                report.resources += volume.entries;
                report.resource_bytes += volume.bytes;
                report.volumes.push(volume);
            }
            Err(VolumeFailure::Mdd(detail)) => {
                warnings.push(format!("{}: {detail}", path.display()));
            }
            Err(VolumeFailure::Repo(error)) => return Err(error),
        }
        done = base + resources;
    }
    Ok(report)
}

/// Why one volume could not be imported.
enum VolumeFailure {
    /// The MDD file itself was unreadable or corrupt.
    Mdd(String),
    /// The repository refused a write.
    Repo(crate::Error),
}

impl From<crate::Error> for VolumeFailure {
    fn from(error: crate::Error) -> Self {
        Self::Repo(error)
    }
}

fn import_volume(
    repo: &mut Repo,
    path: &Path,
    seen: &mut HashSet<String>,
    progress: &mut dyn FnMut(u64),
) -> std::result::Result<MddVolumeReport, VolumeFailure> {
    let file = MddFile::open(path).map_err(|error| VolumeFailure::Mdd(error.to_string()))?;
    let mut batch: Vec<(String, Vec<u8>)> = Vec::with_capacity(RESOURCE_BATCH);
    let mut entries = 0_usize;
    let mut bytes = 0_u64;
    let mut processed = 0_u64;
    for resource in file.resources() {
        let resource = resource.map_err(|error| VolumeFailure::Mdd(error.to_string()))?;
        processed += 1;
        let key = normalize_resource_key(resource.key());
        if key.is_empty() || !seen.insert(key.clone()) {
            continue;
        }
        let payload = resource.into_parts().1;
        entries += 1;
        bytes += payload.len() as u64;
        batch.push((key, payload));
        if batch.len() >= RESOURCE_BATCH {
            repo.put_resources(&batch)?;
            batch.clear();
            progress(processed);
        }
    }
    if !batch.is_empty() {
        repo.put_resources(&batch)?;
    }
    progress(processed);
    Ok(MddVolumeReport {
        path: path.to_path_buf(),
        entries,
        bytes,
    })
}
