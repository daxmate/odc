//! # ODC core
//!
//! Reference implementation of the Open Dictionary Container format:
//!
//! * content-defined chunking of dictionary entries (module `cdc`),
//! * canonical block encoding (module `block`),
//! * canonical CBOR payloads (modules `cbor` and `entry`),
//! * BLAKE3 hashing and Merkle roots (module `hash`),
//! * a SQLite-backed repository with commit / log / diff / verify
//!   (module `repo`),
//! * an MDX (MDict) import adapter (module `mdx`),
//! * `ODP1` patch creation and application (module `patch`).
//!
//! The normative description lives in `docs/odc-spec.md`.

pub mod block;
pub mod cbor;
pub mod cdc;
pub mod entry;
pub mod error;
pub mod hash;
pub mod html;
mod index;
pub mod link;
pub mod mdd_import;
pub mod mdx;
pub mod mdx_import;
pub mod mdx_repo_import;
pub mod mdx_scan;
pub mod merge;
pub mod pack;
pub mod patch;
pub mod repo;
pub mod snapshot;
pub mod store;
pub mod stylesheets;
pub mod varint;

pub use entry::{Def, Entry};
pub use error::{Error, Result};
pub use mdd_import::{MddImportReport, MddProgress, MddScan, MddVolumeReport};
pub use mdx_import::{import_mdx, ImportProgress, ImportStage, MdxImportOptions, MdxImportReport};
pub use mdx_repo_import::{
    copy_pack_to_repo, import_mdx_to_repo, MdxRepoImportReport, PackRepoConversion,
};
pub use repo::{
    CommitOutcome, CommitRecord, Diff, GcReport, ImportReport, Repo, Status, VerifyReport,
    DEFAULT_AUTHOR,
};

/// Container format version implemented by this crate.
pub const FORMAT_VERSION: &str = "0.1";
/// Major version component of the format version string.
pub const FORMAT_MAJOR: u16 = 0;
/// Minor version component of the format version string.
pub const FORMAT_MINOR: u16 = 1;
/// Block magic.
pub const BLOCK_MAGIC: [u8; 4] = block::MAGIC;
/// Patch magic.
pub const PATCH_MAGIC: [u8; 4] = patch::MAGIC;
