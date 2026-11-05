//! MDX (MDict) reader adapter.
//!
//! The parser itself lives in the `mdictlib` crate (MIT, see
//! `docs/mdx-compat.md` for the versions and codecs it covers). This module
//! keeps ODC's own contract in place: stable error codes, an explicit record
//! size limit, and the normalised-key collision policy used by
//! `odc import --format mdx`.

use crate::entry::{normalize_key, Entry};
use crate::error::{Error, Result};
use mdictlib::{ChecksumPolicy, MdxFile, OpenOptions};
use std::collections::HashSet;
use std::path::Path;

/// Default maximum size of one HTML record, in bytes (4 MiB).
///
/// This is the limit used by `MdxImportOptions::default()`, by
/// `odc import --format mdx` without `--max-record-bytes`, and by the
/// legacy FFI entry points. 4 MiB covers real dictionaries whose tail
/// entries are far larger than the typical record (LDOCE6 ships `go` at
/// 1.48 MiB) while still keeping a corrupt or hostile file from exhausting
/// memory.
pub const DEFAULT_MAX_RECORD_BYTES: usize = 4 * 1024 * 1024;

/// Hard upper bound for a configured record limit, in bytes (16 MiB).
///
/// A larger `max_record_bytes` is refused with `ODC-E016` so a bad
/// configuration cannot turn one record into an unbounded allocation.
pub const HARD_MAX_RECORD_BYTES: usize = 16 * 1024 * 1024;

/// Historical 1 MiB cap, kept as an alias of [`DEFAULT_MAX_RECORD_BYTES`].
#[deprecated(
    since = "0.14.0",
    note = "use DEFAULT_MAX_RECORD_BYTES or MdxImportOptions::max_record_bytes"
)]
pub const MAX_RECORD_BYTES: usize = DEFAULT_MAX_RECORD_BYTES;

/// Check a caller-supplied record limit.
///
/// # Errors
///
/// * [`Error::InvalidArgument`] when `value` is 0 or above
///   [`HARD_MAX_RECORD_BYTES`].
pub fn validate_max_record_bytes(value: usize) -> Result<()> {
    if value == 0 || value > HARD_MAX_RECORD_BYTES {
        return Err(Error::InvalidArgument(format!(
            "max_record_bytes must be between 1 and {HARD_MAX_RECORD_BYTES} bytes, got {value}"
        )));
    }
    Ok(())
}

/// Separator used to disambiguate normalised-key collisions (`U+001F`).
pub const COLLISION_SEPARATOR: char = '\u{1f}';

/// Header fields recorded when importing an MDX file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MdxHeader {
    /// `GeneratedByEngineVersion` from the header XML.
    pub engine_version: String,
    /// `RequiredEngineVersion` from the header XML.
    pub required_engine_version: String,
    /// Text encoding advertised by the header, if any.
    pub encoding: String,
    /// Value of the `Encrypted` attribute, if present.
    pub encrypted: Option<String>,
    /// Number of physical key records in the file.
    pub records: u64,
}

/// One raw record read from an MDX file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MdxRecord {
    /// Key exactly as stored in the dictionary (not normalised).
    pub key: String,
    /// Record body, usually a fragment of HTML.
    pub html: String,
}

/// An opened MDX file.
#[derive(Debug)]
pub struct MdxSource {
    file: MdxFile,
}

impl MdxSource {
    /// Open an MDX file.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Mdx`] when the header, key blocks or block index are
    /// unreadable (bad magic, truncated file, unsupported version, broken
    /// encryption or compression stream).
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let options = OpenOptions::new().with_checksum_policy(ChecksumPolicy::Verify);
        let file = MdxFile::open_with_options(path, &options).map_err(map_error)?;
        Ok(Self { file })
    }

    /// Header metadata of the opened file.
    #[must_use]
    pub fn header(&self) -> MdxHeader {
        let header = self.file.header();
        MdxHeader {
            engine_version: header.generated_by_engine_version().to_string(),
            required_engine_version: header.required_engine_version().to_string(),
            encoding: header.encoding_label().unwrap_or_default().to_string(),
            encrypted: header.attribute("Encrypted").map(str::to_string),
            records: self.file.len(),
        }
    }

    /// Number of physical key records.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.file.len()
    }

    /// True when the file declares no records.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.file.is_empty()
    }

    /// Iterate records in physical (dictionary) order.
    ///
    /// Duplicate keys are yielded as often as the file stores them; the
    /// importer resolves them through [`MdxKeys`].
    pub fn records(&self) -> impl Iterator<Item = Result<MdxRecord>> + '_ {
        self.file.entries().map(|item| {
            let entry = item.map_err(map_error)?;
            Ok(MdxRecord {
                key: entry.key().to_string(),
                html: entry.text().to_string(),
            })
        })
    }
}

/// Convert one MDX record into an ODC entry.
///
/// The normalised key becomes the entry key (duplicates are resolved by
/// [`MdxKeys`]), `display` keeps the original spelling and `raw_html` keeps
/// the record body. Structural fields (`defs`, `phonetic`) stay empty: MDX
/// HTML is not parsed during import.
///
/// # Errors
///
/// * [`Error::InvalidArgument`] when the key is empty once trimmed.
/// * [`Error::EntryTooLarge`] when the body exceeds [`DEFAULT_MAX_RECORD_BYTES`].
pub fn entry_from_record(record: &MdxRecord) -> Result<Entry> {
    entry_from_record_with_limit(record, DEFAULT_MAX_RECORD_BYTES)
}

/// Convert one MDX record into an ODC entry with an explicit size limit.
///
/// [`entry_from_record`] is the same call with
/// [`DEFAULT_MAX_RECORD_BYTES`]; the importer passes
/// [`MdxImportOptions::max_record_bytes`](crate::mdx_import::MdxImportOptions::max_record_bytes) instead.
///
/// # Errors
///
/// * [`Error::InvalidArgument`] when the key is empty once trimmed.
/// * [`Error::EntryTooLarge`] when the body exceeds `max_record_bytes`.
pub fn entry_from_record_with_limit(record: &MdxRecord, max_record_bytes: usize) -> Result<Entry> {
    let key = record.key.trim();
    if key.is_empty() {
        return Err(Error::InvalidArgument(
            "MDX record with an empty key".to_string(),
        ));
    }
    if record.html.len() > max_record_bytes {
        return Err(Error::EntryTooLarge {
            word: key.to_string(),
            bytes: record.html.len(),
            limit: max_record_bytes,
        });
    }
    let mut entry = Entry::new(record.key.clone());
    entry.raw_html = Some(record.html.clone());
    Ok(entry)
}

/// Allocates unique normalised keys inside one MDX import.
///
/// The first occurrence of a normalised key keeps it verbatim. Later
/// occurrences (case variants, full-width spellings, ...) receive a
/// `U+001F<n>` suffix, starting at `2`, in file order. The display form is
/// never changed.
#[derive(Debug, Default)]
pub struct MdxKeys {
    seen: HashSet<String>,
}

impl MdxKeys {
    /// Create an empty allocator.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of keys allocated so far.
    #[must_use]
    pub fn len(&self) -> usize {
        self.seen.len()
    }

    /// True when nothing has been allocated.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }

    /// Return a unique key for `word` and whether a collision suffix was used.
    ///
    /// `word` must already be normalised (see [`normalize_key`]).
    pub fn allocate(&mut self, word: &str) -> (String, bool) {
        if self.seen.insert(word.to_string()) {
            return (word.to_string(), false);
        }
        let mut index = 2u32;
        loop {
            let candidate = format!("{word}{COLLISION_SEPARATOR}{index}");
            if self.seen.insert(candidate.clone()) {
                return (candidate, true);
            }
            index += 1;
        }
    }
}

/// Normalised key for an MDX key string.
#[must_use]
pub fn key_for(raw: &str) -> String {
    normalize_key(raw)
}

fn map_error(error: mdictlib::Error) -> Error {
    Error::Mdx(error.to_string())
}
