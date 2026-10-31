//! Patch wire format (spec section 8).
//!
//! The format is fixed so that later milestones can implement transfer without
//! changing the specification, but M0 only exposes the entry points and
//! returns a precise "not implemented" error.

use crate::error::{Error, Result};

/// Patch magic, four ASCII bytes.
pub const MAGIC: [u8; 4] = *b"ODP1";

/// Patch layout version, independent from the container minor version.
pub const PATCH_VERSION: u8 = 1;

/// Logical description of a patch, mirroring the byte layout in the spec.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PatchPlan {
    /// Base commit hash the patch applies to.
    pub base: [u8; 32],
    /// Target commit hash produced by applying the patch.
    pub target: [u8; 32],
    /// Block hashes the receiver is expected to already have.
    pub base_blocks: Vec<[u8; 32]>,
    /// Blocks carried inline (raw block bytes, compressed per the codec field).
    pub new_blocks: Vec<Vec<u8>>,
    /// Index rows that must be inserted.
    pub index_rows: Vec<(String, [u8; 32])>,
    /// BLAKE3 over the canonical patch body, checked before applying.
    pub checksum: [u8; 32],
}

/// Create a patch between two commits.
///
/// Specified for M3; M0 always returns the not-implemented error.
pub fn create(_repo: &crate::repo::Repo, _base: &str, _target: &str) -> Result<PatchPlan> {
    Err(Error::NotImplemented("patch create"))
}

/// Apply a patch to a repository.
///
/// Specified for M3; M0 always returns the not-implemented error.
pub fn apply(_repo: &mut crate::repo::Repo, _bytes: &[u8]) -> Result<()> {
    Err(Error::NotImplemented("patch apply"))
}
