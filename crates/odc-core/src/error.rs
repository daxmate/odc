//! Error type shared by every ODC layer.
//!
//! Every variant maps to a stable error code documented in
//! `docs/odc-spec.md` section 10. Codes are part of the public contract: the
//! CLI prints them, and other implementations are expected to use the same
//! identifiers.

use std::fmt;

/// Convenient result alias used across the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// All errors produced by the ODC core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A varint was truncated or used a non-minimal encoding.
    MalformedVarint,
    /// Input ended before a structure was complete.
    Truncated(&'static str),
    /// Extra bytes remained after a complete structure was decoded.
    TrailingData(&'static str),
    /// The four-byte magic (or the format version) did not match.
    BadMagic(String),
    /// A CBOR major type that ODC does not permit was encountered.
    UnsupportedCbor(u8),
    /// Structural CBOR problem (duplicate keys, indefinite length, ...).
    Cbor(&'static str),
    /// Block records were not strictly sorted by key bytes.
    UnsortedRecords,
    /// The same key appeared twice inside one block.
    DuplicateKey(String),
    /// Recomputing a hash did not reproduce the stored value.
    HashMismatch(String),
    /// A referenced block is not stored in the repository.
    UnknownBlock(String),
    /// A block declared a compression codec this build does not know.
    UnknownCodec(i64),
    /// A commit hash or revision could not be resolved.
    UnknownCommit(String),
    /// `commit` was called while the working set equals HEAD.
    NoChanges,
    /// A write was attempted on a packed, read-only distribution file.
    ReadOnly(String),
    /// `init` was called on an already initialised repository.
    AlreadyInitialized(String),
    /// The directory does not contain an ODC repository.
    NotARepository(String),
    /// The container minor/major version is outside the supported window.
    UnsupportedVersion {
        found: String,
        supported: &'static str,
    },
    /// Caller supplied an unusable value (bad hash, bad range, ...).
    InvalidArgument(String),
    /// A feature whose wire format is specified but not implemented in M0.
    NotImplemented(&'static str),
    /// Internal index invariant was violated.
    Inconsistent(String),
    /// Filesystem failure.
    Io(String),
    /// SQLite failure.
    Sqlite(String),
    /// An MDX file could not be read (header, key block or record block).
    Mdx(String),
    /// One MDX record exceeded the import size limit.
    EntryTooLarge {
        /// Key of the offending record.
        word: String,
        /// Record size in bytes.
        bytes: usize,
        /// Configured limit in bytes.
        limit: usize,
    },
    /// A patch body did not match its trailing checksum.
    PatchChecksum,
    /// The patch base is not the repository HEAD.
    PatchBaseMismatch(String),
    /// HEAD descends from the base, so applying needs a three-way merge.
    PatchNeedsMerge(String),
    /// The patch target commit is already present locally.
    PatchApplied(String),
    /// The patch byte stream is structurally invalid.
    PatchFormat(String),
    /// A merge is already in progress.
    MergeInProgress(String),
    /// `merge --continue` was asked to finish while conflicts remain.
    UnresolvedConflicts(usize),
    /// `resolve` or `merge --continue` found no merge to work on.
    NoMergeInProgress,
    /// A `.sig` sidecar is not valid v1 signature JSON.
    SignatureFormat(String),
    /// The signed digest or size does not match the target file.
    SignatureTargetMismatch(String),
    /// Ed25519 verification failed (wrong key, corrupt signature, ...).
    SignatureInvalid(String),
    /// A private or public key is not in the expected PKCS#8 / hex form.
    KeyFormat(String),
}

impl Error {
    /// Stable machine-readable error code (see spec section 10).
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::MalformedVarint => "ODC-E001",
            Self::Truncated(_) => "ODC-E002",
            Self::TrailingData(_) => "ODC-E003",
            Self::BadMagic(_) => "ODC-E004",
            Self::UnsupportedCbor(_) | Self::Cbor(_) => "ODC-E005",
            Self::UnsortedRecords => "ODC-E006",
            Self::DuplicateKey(_) => "ODC-E007",
            Self::HashMismatch(_) => "ODC-E008",
            Self::UnknownBlock(_) => "ODC-E009",
            Self::UnknownCodec(_) => "ODC-E010",
            Self::UnknownCommit(_) => "ODC-E011",
            Self::NoChanges => "ODC-E012",
            Self::ReadOnly(_) => "ODC-E031",
            Self::AlreadyInitialized(_) => "ODC-E013",
            Self::NotARepository(_) => "ODC-E014",
            Self::UnsupportedVersion { .. } => "ODC-E015",
            Self::InvalidArgument(_) => "ODC-E016",
            Self::NotImplemented(_) => "ODC-E017",
            Self::Inconsistent(_) => "ODC-E018",
            Self::Io(_) => "ODC-E019",
            Self::Sqlite(_) => "ODC-E020",
            Self::Mdx(_) => "ODC-E021",
            Self::EntryTooLarge { .. } => "ODC-E022",
            Self::PatchChecksum => "ODC-E023",
            Self::PatchBaseMismatch(_) => "ODC-E024",
            Self::PatchNeedsMerge(_) => "ODC-E025",
            Self::PatchApplied(_) => "ODC-E026",
            Self::PatchFormat(_) => "ODC-E027",
            Self::MergeInProgress(_) => "ODC-E028",
            Self::UnresolvedConflicts(_) => "ODC-E029",
            Self::NoMergeInProgress => "ODC-E030",
            Self::SignatureFormat(_) => "ODC-E032",
            Self::SignatureTargetMismatch(_) => "ODC-E033",
            Self::SignatureInvalid(_) => "ODC-E034",
            Self::KeyFormat(_) => "ODC-E035",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::MalformedVarint => "malformed varint (truncated or non-canonical)".to_string(),
            Self::Truncated(what) => format!("truncated {what}"),
            Self::TrailingData(what) => format!("unexpected trailing data after {what}"),
            Self::BadMagic(detail) => format!("bad magic or version: {detail}"),
            Self::UnsupportedCbor(major) => {
                format!("unsupported CBOR major type {major} (ODC allows only the core model)")
            }
            Self::Cbor(detail) => format!("CBOR error: {detail}"),
            Self::UnsortedRecords => "records are not strictly sorted by key bytes".to_string(),
            Self::DuplicateKey(key) => format!("duplicate key in block: {key}"),
            Self::HashMismatch(detail) => format!("hash mismatch: {detail}"),
            Self::UnknownBlock(hash) => format!("unknown block {hash}"),
            Self::UnknownCodec(codec) => format!("unknown block codec {codec}"),
            Self::UnknownCommit(rev) => format!("unknown commit {rev}"),
            Self::NoChanges => "nothing to commit: working set is identical to HEAD".to_string(),
            Self::ReadOnly(path) => {
                format!("repository is read-only (packed distribution file): {path}")
            }
            Self::AlreadyInitialized(path) => format!("repository already initialized at {path}"),
            Self::NotARepository(path) => format!("not an ODC repository: {path}"),
            Self::UnsupportedVersion { found, supported } => {
                format!("unsupported format version {found}; supported {supported}")
            }
            Self::InvalidArgument(detail) => format!("invalid argument: {detail}"),
            Self::NotImplemented(feature) => {
                format!("{feature} is specified but not implemented in M0")
            }
            Self::Inconsistent(detail) => format!("index inconsistency: {detail}"),
            Self::Io(detail) => format!("io error: {detail}"),
            Self::Sqlite(detail) => format!("sqlite error: {detail}"),
            Self::Mdx(detail) => format!("MDX error: {detail}"),
            Self::EntryTooLarge { word, bytes, limit } => {
                format!("entry {word} is {bytes} bytes, over the {limit}-byte MDX limit")
            }
            Self::PatchChecksum => "patch checksum does not match its body".to_string(),
            Self::PatchBaseMismatch(detail) => format!("patch base does not match HEAD: {detail}"),
            Self::PatchNeedsMerge(detail) => {
                format!("patch needs a three-way merge (M4): {detail}")
            }
            Self::PatchApplied(detail) => format!("patch is already applied: {detail}"),
            Self::PatchFormat(detail) => format!("malformed patch: {detail}"),
            Self::MergeInProgress(detail) => {
                format!("a merge is already in progress: {detail}")
            }
            Self::UnresolvedConflicts(remaining) => {
                format!("merge has {remaining} unresolved conflict(s)")
            }
            Self::NoMergeInProgress => "no merge is in progress".to_string(),
            Self::SignatureFormat(detail) => format!("malformed signature: {detail}"),
            Self::SignatureTargetMismatch(detail) => {
                format!("signature target does not match: {detail}")
            }
            Self::SignatureInvalid(detail) => format!("signature verification failed: {detail}"),
            Self::KeyFormat(detail) => format!("invalid key material: {detail}"),
        };
        write!(f, "{}: {message}", self.code())
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

impl From<rusqlite::Error> for Error {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value.to_string())
    }
}
