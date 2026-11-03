//! Release bundles: manifest v1, publishing and verification.
//!
//! A release directory holds one full pack (.odc), an optional incremental
//! patch (.odp), a release.json manifest and a detached signature for every
//! file, including the manifest itself. The schema is frozen by
//! docs/release.md; the sidecar layout it embeds is defined by the signing
//! module.
//!
//! Publishing never touches the repository beyond reading reachable objects.
//! Verification recomputes every digest and size, so a manifest cannot be
//! replayed onto different bytes.

use crate::error::{Error, Result};
use crate::hash;
use crate::pack::PackOptions;
use crate::patch;
use crate::repo::{canonical_public_key, Publisher, Repo, TrustOutcome, TRUST_SOURCE_TOFU};
use crate::signing::{self, SignatureV1, VerifiedSignature};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// Manifest file name inside a release directory.
pub const MANIFEST_FILE: &str = "release.json";
/// Detached signature file for the manifest.
pub const MANIFEST_SIGNATURE_FILE: &str = "release.json.sig";
/// Manifest format identifier written into the format field.
pub const RELEASE_FORMAT: &str = "odc-release";
/// Manifest format version written into the version field.
pub const RELEASE_VERSION: u32 = 1;
/// Artifact kind for a full pack.
pub const ARTIFACT_KIND_PACK: &str = "pack";
/// Artifact kind for an incremental patch.
pub const ARTIFACT_KIND_PATCH: &str = "patch";

/// Publisher block embedded in a manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleasePublisher {
    /// Lower-case hex Ed25519 public key (32 bytes).
    pub public_key: String,
    /// Optional display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// One signed file listed in a manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseArtifact {
    /// Either pack or patch.
    pub kind: String,
    /// File name relative to the release directory.
    pub path: String,
    /// Size in bytes.
    pub size: u64,
    /// Lower-case hex BLAKE3-256 of the file.
    pub blake3: String,
    /// Inline copy of the sidecar signature.
    pub signature: SignatureV1,
}

/// Parsed release.json (manifest v1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseManifest {
    /// Always odc-release.
    pub format: String,
    /// Always 1.
    pub version: u32,
    /// Dictionary name, used to build file names.
    pub name: String,
    /// Release label, used to build file names.
    pub label: String,
    /// RFC 3339 UTC creation time.
    pub created_at: String,
    /// HEAD commit contained in the pack, lower-case hex.
    pub head: String,
    /// Base commit of the incremental patch, when one is published.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// Signed files; exactly one pack, at most one patch.
    pub artifacts: Vec<ReleaseArtifact>,
    /// Free-form note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Signing key that produced every signature in this release.
    pub publisher: ReleasePublisher,
}

impl ReleaseManifest {
    /// Serialise to the canonical form: two-space indent, one final newline.
    ///
    /// # Errors
    ///
    /// Returns Error::ReleaseManifestInvalid when a field is invalid.
    pub fn to_json(&self) -> Result<String> {
        self.validate()?;
        let mut text = serde_json::to_string_pretty(self).map_err(|error| {
            Error::ReleaseManifestInvalid(format!("cannot encode manifest: {error}"))
        })?;
        text.push('\n');
        Ok(text)
    }

    /// Parse and validate a manifest.
    ///
    /// # Errors
    ///
    /// Returns Error::ReleaseManifestInvalid for malformed JSON or any invalid
    /// field.
    pub fn from_json(text: &str) -> Result<Self> {
        let manifest: Self = serde_json::from_str(text).map_err(|error| {
            Error::ReleaseManifestInvalid(format!("cannot parse JSON: {error}"))
        })?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Check every field against the v1 schema.
    ///
    /// # Errors
    ///
    /// Returns Error::ReleaseManifestInvalid naming the first problem.
    pub fn validate(&self) -> Result<()> {
        if self.format != RELEASE_FORMAT {
            return Err(Error::ReleaseManifestInvalid(format!(
                "format {:?} is not {RELEASE_FORMAT:?}",
                self.format
            )));
        }
        if self.version != RELEASE_VERSION {
            return Err(Error::ReleaseManifestInvalid(format!(
                "unsupported manifest version {} (expected {RELEASE_VERSION})",
                self.version
            )));
        }
        validate_component("name", &self.name)
            .map_err(|error| Error::ReleaseManifestInvalid(error.to_string()))?;
        validate_component("label", &self.label)
            .map_err(|error| Error::ReleaseManifestInvalid(error.to_string()))?;
        if OffsetDateTime::parse(&self.created_at, &Rfc3339).is_err() {
            return Err(Error::ReleaseManifestInvalid(format!(
                "created_at is not an RFC 3339 timestamp: {:?}",
                self.created_at
            )));
        }
        signing::hex_bytes::<32>(&self.head, "head")
            .map_err(|error| Error::ReleaseManifestInvalid(error.to_string()))?;
        if let Some(base) = &self.base {
            signing::hex_bytes::<32>(base, "base")
                .map_err(|error| Error::ReleaseManifestInvalid(error.to_string()))?;
        }
        canonical_public_key(&self.publisher.public_key)
            .map_err(|error| Error::ReleaseManifestInvalid(error.to_string()))?;
        if self.artifacts.is_empty() {
            return Err(Error::ReleaseManifestInvalid(
                "manifest lists no artifacts".to_string(),
            ));
        }
        let mut packs = 0;
        let mut patches = 0;
        for artifact in &self.artifacts {
            match artifact.kind.as_str() {
                ARTIFACT_KIND_PACK => packs += 1,
                ARTIFACT_KIND_PATCH => patches += 1,
                other => {
                    return Err(Error::ReleaseManifestInvalid(format!(
                        "unknown artifact kind {other:?}"
                    )));
                }
            }
            validate_artifact_path(&artifact.path)?;
            signing::hex_bytes::<32>(&artifact.blake3, "artifacts[].blake3")
                .map_err(|error| Error::ReleaseManifestInvalid(error.to_string()))?;
            artifact.signature.validate().map_err(|error| {
                Error::ReleaseManifestInvalid(format!("artifact {}: {error}", artifact.path))
            })?;
            if artifact.signature.public_key != self.publisher.public_key {
                return Err(Error::ReleaseManifestInvalid(format!(
                    "artifact {} is signed by a different key",
                    artifact.path
                )));
            }
            if artifact.signature.target.hash != artifact.blake3
                || artifact.signature.target.size != artifact.size
            {
                return Err(Error::ReleaseManifestInvalid(format!(
                    "artifact {} signature does not cover the listed digest",
                    artifact.path
                )));
            }
        }
        if packs != 1 {
            return Err(Error::ReleaseManifestInvalid(format!(
                "manifest must list exactly one pack artifact, found {packs}"
            )));
        }
        if patches > 1 {
            return Err(Error::ReleaseManifestInvalid(
                "manifest lists more than one patch artifact".to_string(),
            ));
        }
        Ok(())
    }
}

/// Options for publish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishOptions {
    /// Dictionary name (defaults to the repository directory name).
    pub name: String,
    /// Release label, required.
    pub label: String,
    /// Base revision for an incremental patch; None publishes a full pack only.
    pub base: Option<String>,
    /// PKCS#8 PEM private key used for every signature.
    pub key_pem: String,
    /// Optional signer name recorded in every signature.
    pub signer: Option<String>,
    /// Optional note stored in the manifest.
    pub notes: Option<String>,
    /// Replace existing files in the output directory.
    pub force: bool,
}

/// One artifact written by publish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedArtifact {
    /// Either pack or patch.
    pub kind: String,
    /// File name inside the release directory.
    pub path: String,
    /// Size in bytes.
    pub size: u64,
    /// Lower-case hex BLAKE3-256.
    pub blake3: String,
}

/// Result of publish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishReport {
    /// Directory that received the release files.
    pub out_dir: PathBuf,
    /// Manifest that was written.
    pub manifest_path: PathBuf,
    /// HEAD commit included in the pack, lower-case hex.
    pub head: String,
    /// Base commit of the patch, when one was written.
    pub base: Option<String>,
    /// Artifacts in manifest order.
    pub artifacts: Vec<PublishedArtifact>,
    /// Signing public key, lower-case hex.
    pub public_key: String,
}

/// One artifact accepted by verify_release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedArtifact {
    /// Either pack or patch.
    pub kind: String,
    /// File name inside the release directory.
    pub path: String,
    /// Size in bytes, as listed and re-checked.
    pub size: u64,
    /// Lower-case hex BLAKE3-256, as listed and re-checked.
    pub blake3: String,
}

/// Result of verify_release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseVerifyReport {
    /// Release directory that was verified.
    pub dir: PathBuf,
    /// Dictionary name from the manifest.
    pub name: String,
    /// Release label from the manifest.
    pub label: String,
    /// HEAD commit recorded in the manifest.
    pub head: String,
    /// Base commit recorded in the manifest.
    pub base: Option<String>,
    /// created_at recorded in the manifest.
    pub created_at: String,
    /// Publisher key that signed the manifest.
    pub publisher_key: String,
    /// Publisher name, when recorded.
    pub publisher_name: Option<String>,
    /// Verified artifacts in manifest order.
    pub artifacts: Vec<VerifiedArtifact>,
}

/// Write a signed release into out_dir.
///
/// Nothing is written when a target file already exists and options.force is
/// false: every path is checked before the first byte is produced.
///
/// # Errors
///
/// Returns Error::InvalidArgument for a missing HEAD or a conflicting file, and
/// propagates pack, patch and signing failures unchanged.
pub fn publish(repo: &Repo, out_dir: &Path, options: &PublishOptions) -> Result<PublishReport> {
    let head = repo.head()?.ok_or_else(|| {
        Error::InvalidArgument("publish requires at least one commit".to_string())
    })?;
    validate_component("name", &options.name)?;
    validate_component("label", &options.label)?;
    let head_hex = hash::to_hex(&head);

    let base_hex = match &options.base {
        Some(revision) => Some(hash::to_hex(&repo.resolve(revision)?)),
        None => None,
    };

    let pack_name = format!("{}-{}.odc", options.name, options.label);
    let patch_name = format!("{}-{}.odp", options.name, options.label);
    let mut planned = vec![pack_name.clone(), format!("{pack_name}.sig")];
    if base_hex.is_some() {
        planned.push(patch_name.clone());
        planned.push(format!("{patch_name}.sig"));
    }
    planned.push(MANIFEST_FILE.to_string());
    planned.push(MANIFEST_SIGNATURE_FILE.to_string());

    if out_dir.exists() && !out_dir.is_dir() {
        return Err(Error::InvalidArgument(format!(
            "output path {} is not a directory",
            out_dir.display()
        )));
    }
    if !out_dir.exists() {
        fs::create_dir_all(out_dir)?;
    }
    if !options.force {
        for name in &planned {
            let path = out_dir.join(name);
            if path.exists() {
                return Err(Error::InvalidArgument(format!(
                    "refusing to overwrite {} (pass --force)",
                    path.display()
                )));
            }
        }
    }

    let mut artifacts: Vec<ReleaseArtifact> = Vec::new();
    let mut published: Vec<PublishedArtifact> = Vec::new();

    let pack_path = out_dir.join(&pack_name);
    repo.pack(
        &pack_path,
        PackOptions {
            shallow: false,
            force: options.force,
        },
    )?;
    let (pack_size, pack_hash, pack_signature) =
        sign_existing_file(&pack_path, &options.key_pem, options.signer.as_deref())?;
    let public_key = pack_signature.public_key.clone();
    artifacts.push(ReleaseArtifact {
        kind: ARTIFACT_KIND_PACK.to_string(),
        path: pack_name.clone(),
        size: pack_size,
        blake3: pack_hash.clone(),
        signature: pack_signature,
    });
    published.push(PublishedArtifact {
        kind: ARTIFACT_KIND_PACK.to_string(),
        path: pack_name,
        size: pack_size,
        blake3: pack_hash,
    });

    if let Some(base) = &options.base {
        let plan = patch::create(repo, base, "HEAD")?;
        let bytes = patch::encode(&plan)?;
        let patch_path = out_dir.join(&patch_name);
        fs::write(&patch_path, &bytes)?;
        let signature = signing::sign_bytes(&bytes, &options.key_pem, options.signer.as_deref())?;
        signing::write_signature_file(&signing::signature_path(&patch_path), &signature)?;
        let size = bytes.len() as u64;
        let digest = hash::to_hex(&hash::blake3(&bytes));
        published.push(PublishedArtifact {
            kind: ARTIFACT_KIND_PATCH.to_string(),
            path: patch_name.clone(),
            size,
            blake3: digest.clone(),
        });
        artifacts.push(ReleaseArtifact {
            kind: ARTIFACT_KIND_PATCH.to_string(),
            path: patch_name,
            size,
            blake3: digest,
            signature,
        });
    }

    let manifest = ReleaseManifest {
        format: RELEASE_FORMAT.to_string(),
        version: RELEASE_VERSION,
        name: options.name.clone(),
        label: options.label.clone(),
        created_at: now_rfc3339()?,
        head: head_hex.clone(),
        base: base_hex.clone(),
        artifacts,
        notes: options.notes.clone(),
        publisher: ReleasePublisher {
            public_key: public_key.clone(),
            name: options.signer.clone(),
        },
    };
    let manifest_text = manifest.to_json()?;
    let manifest_path = out_dir.join(MANIFEST_FILE);
    fs::write(&manifest_path, manifest_text.as_bytes())?;
    let manifest_signature = signing::sign_bytes(
        manifest_text.as_bytes(),
        &options.key_pem,
        options.signer.as_deref(),
    )?;
    signing::write_signature_file(&out_dir.join(MANIFEST_SIGNATURE_FILE), &manifest_signature)?;

    Ok(PublishReport {
        out_dir: out_dir.to_path_buf(),
        manifest_path,
        head: head_hex,
        base: base_hex,
        artifacts: published,
        public_key,
    })
}

/// Verify a release directory.
///
/// Every artifact is re-read: its size and BLAKE3 digest must match the
/// manifest and the inline signature must verify with the publisher key. When
/// expected is set the manifest must be signed by that key. Trust-store
/// decisions belong to the caller; this function is purely cryptographic.
///
/// # Errors
///
/// ODC-E036 for a pinned-key mismatch, ODC-E037 for a malformed manifest or an
/// invalid manifest signature, ODC-E038 for a missing or altered artifact.
pub fn verify_release(dir: &Path, expected: Option<&[u8; 32]>) -> Result<ReleaseVerifyReport> {
    let manifest_path = dir.join(MANIFEST_FILE);
    let manifest_bytes = fs::read(&manifest_path).map_err(|error| {
        Error::ReleaseManifestInvalid(format!("cannot read {}: {error}", manifest_path.display()))
    })?;
    let manifest_text = String::from_utf8_lossy(&manifest_bytes).to_string();
    let manifest = ReleaseManifest::from_json(&manifest_text)?;

    let key_hex = canonical_public_key(&manifest.publisher.public_key)?;
    let key_bytes = signing::hex_bytes::<32>(&key_hex, "publisher.public_key")
        .map_err(|error| Error::ReleaseManifestInvalid(error.to_string()))?;
    if let Some(expected) = expected {
        if expected != &key_bytes {
            return Err(Error::PublisherUntrusted(format!(
                "manifest is signed by {key_hex}, expected {}",
                signing::hex_encode(expected)
            )));
        }
    }

    let signature_path = dir.join(MANIFEST_SIGNATURE_FILE);
    let manifest_signature = signing::read_signature_file(&signature_path).map_err(|error| {
        Error::ReleaseManifestInvalid(format!("cannot read {}: {error}", signature_path.display()))
    })?;
    signing::verify_bytes_with_key(&manifest_bytes, &manifest_signature, Some(&key_bytes))
        .map_err(|error| {
            Error::ReleaseManifestInvalid(format!("manifest signature does not verify: {error}"))
        })?;

    let mut artifacts = Vec::new();
    for artifact in &manifest.artifacts {
        let path = dir.join(&artifact.path);
        let bytes = fs::read(&path).map_err(|error| {
            Error::ReleaseArtifactMismatch(format!("cannot read {}: {error}", artifact.path))
        })?;
        let size = bytes.len() as u64;
        if size != artifact.size {
            return Err(Error::ReleaseArtifactMismatch(format!(
                "{} is {size} bytes, manifest lists {}",
                artifact.path, artifact.size
            )));
        }
        let digest = hash::to_hex(&hash::blake3(&bytes));
        if digest != artifact.blake3 {
            return Err(Error::ReleaseArtifactMismatch(format!(
                "{} has hash {digest}, manifest lists {}",
                artifact.path, artifact.blake3
            )));
        }
        signing::verify_bytes_with_key(&bytes, &artifact.signature, Some(&key_bytes)).map_err(
            |error| {
                Error::ReleaseArtifactMismatch(format!(
                    "{} signature does not verify: {error}",
                    artifact.path
                ))
            },
        )?;
        artifacts.push(VerifiedArtifact {
            kind: artifact.kind.clone(),
            path: artifact.path.clone(),
            size,
            blake3: digest,
        });
    }

    Ok(ReleaseVerifyReport {
        dir: dir.to_path_buf(),
        name: manifest.name,
        label: manifest.label,
        head: manifest.head,
        base: manifest.base,
        created_at: manifest.created_at,
        publisher_key: key_hex,
        publisher_name: manifest.publisher.name,
        artifacts,
    })
}

/// Apply the TOFU policy to the key that signed an artifact.
///
/// The key must already be trusted, or the store must be empty and
/// allow_trust_new set, in which case it is recorded with source tofu.
///
/// # Errors
///
/// Returns Error::PublisherUntrusted (ODC-E036) when the key is unknown and
/// cannot be recorded.
pub fn enforce_publisher_trust(
    repo: &mut Repo,
    public_key: &str,
    name: Option<&str>,
    allow_trust_new: bool,
) -> Result<Publisher> {
    let key = canonical_public_key(public_key)?;
    if let Some(existing) = repo.publisher(&key)? {
        return Ok(existing);
    }
    if !allow_trust_new {
        return Err(Error::PublisherUntrusted(format!(
            "{key} is not in the trust store; run odc trust add --pubkey {key}"
        )));
    }
    if !repo.trust_store_is_empty()? {
        return Err(Error::PublisherUntrusted(format!(
            "{key} is not in the trust store and the store is not empty"
        )));
    }
    match repo.trust_publisher(&key, name.unwrap_or(""), TRUST_SOURCE_TOFU)? {
        TrustOutcome::Added(publisher) | TrustOutcome::AlreadyTrusted(publisher) => Ok(publisher),
        TrustOutcome::NameConflict { existing } => Err(Error::PublisherUntrusted(format!(
            "name {:?} is already trusted for {}",
            existing.name, existing.public_key
        ))),
    }
}

/// Require that public_key is already trusted.
///
/// Used by verify-release when the caller did not pin a key.
///
/// # Errors
///
/// Returns Error::PublisherUntrusted (ODC-E036) for an unknown key.
pub fn require_trusted_publisher(repo: &Repo, public_key: &str) -> Result<Publisher> {
    let key = canonical_public_key(public_key)?;
    if let Some(publisher) = repo.publisher(&key)? {
        return Ok(publisher);
    }
    if repo.trust_store_is_empty()? {
        return Err(Error::PublisherUntrusted(format!(
            "{key} is not in the trust store (the store is empty; pin the key with --pubkey or run odc trust add)"
        )));
    }
    Err(Error::PublisherUntrusted(format!(
        "{key} is not in the trust store"
    )))
}

/// Verify a signed patch and record its publisher when TOFU allows it.
///
/// Nothing is applied here; callers verify first and then hand the bytes to
/// patch::apply.
///
/// # Errors
///
/// Propagates signature errors (ODC-E032 .. ODC-E034) and
/// Error::PublisherUntrusted (ODC-E036).
pub fn verify_patch_signature(
    repo: &mut Repo,
    patch_path: &Path,
    signature_path: &Path,
    allow_trust_new: bool,
) -> Result<VerifiedSignature> {
    let bytes = fs::read(patch_path)?;
    let signature = signing::read_signature_file(signature_path)?;
    let verified = signing::verify_bytes(&bytes, &signature)?;
    let public_key = signing::hex_encode(&verified.public_key);
    enforce_publisher_trust(
        repo,
        &public_key,
        signature.signer.as_deref(),
        allow_trust_new,
    )?;
    Ok(verified)
}

fn sign_existing_file(
    path: &Path,
    key_pem: &str,
    signer: Option<&str>,
) -> Result<(u64, String, SignatureV1)> {
    let bytes = fs::read(path)?;
    let signature = signing::sign_bytes(&bytes, key_pem, signer)?;
    signing::write_signature_file(&signing::signature_path(path), &signature)?;
    Ok((
        bytes.len() as u64,
        hash::to_hex(&hash::blake3(&bytes)),
        signature,
    ))
}

fn validate_component(field: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        return Err(Error::InvalidArgument(format!("{field} must not be empty")));
    }
    if value == "."
        || value == ".."
        || value
            .chars()
            .any(|ch| ch == '/' || ch == '\\' || ch.is_whitespace() || ch.is_control())
    {
        return Err(Error::InvalidArgument(format!(
            "{field} must be a plain file-name component: {value:?}"
        )));
    }
    Ok(())
}

fn validate_artifact_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path == "."
        || path == ".."
        || path
            .chars()
            .any(|ch| ch == '/' || ch == '\\' || ch.is_control())
    {
        return Err(Error::ReleaseManifestInvalid(format!(
            "artifact path must be a plain file name: {path:?}"
        )));
    }
    Ok(())
}

fn now_rfc3339() -> Result<String> {
    let now = OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .map_err(|error| {
            Error::ReleaseManifestInvalid(format!("cannot round created_at: {error}"))
        })?;
    now.format(&Rfc3339).map_err(|error| {
        Error::ReleaseManifestInvalid(format!("cannot format created_at: {error}"))
    })
}
