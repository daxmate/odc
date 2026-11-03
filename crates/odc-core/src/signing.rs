//! Detached Ed25519 signatures for ODC artifacts (v0.10 signing primitive).
//!
//! A signature lives next to its target in a sidecar file (`dict.odc` ->
//! `dict.odc.sig`), so the container format itself does not change. The
//! signature covers a domain-separated BLAKE3 digest of the whole target file;
//! verification always recomputes that digest and the file size, so a valid
//! signature over different bytes is rejected.
//!
//! The normative byte layout lives in `docs/odc-spec.md` ("Signature sidecar
//! v1"); the operator walkthrough lives in `docs/signing.md`.
//!
//! Trust is *not* established here: this module answers "is this signature
//! mathematically valid for these bytes", the caller decides which public keys
//! it trusts (TOFU and the release wizard belong to later milestones).

use crate::error::{Error, Result};
use crate::hash;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use pkcs8::{DecodePrivateKey, EncodePrivateKey, LineEnding};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// Suffix appended to the target path (`dict.odc` -> `dict.odc.sig`).
pub const SIGNATURE_SUFFIX: &str = ".sig";
/// ASCII domain-separation prefix; one NUL byte follows it on the wire.
pub const SIGNATURE_DOMAIN: &[u8] = b"ODC-SIG-V1\0";
/// Signature file version understood by this implementation.
pub const SIGNATURE_VERSION: u32 = 1;
/// Algorithm name recorded in every v1 signature file.
pub const SIGNATURE_ALGORITHM: &str = "ed25519";
/// Hash algorithm name recorded in every v1 signature file.
pub const SIGNATURE_HASH_ALGORITHM: &str = "blake3";
/// First token of a public key file line.
pub const PUBLIC_KEY_PREFIX: &str = "odc-ed25519";

/// Target description embedded in a signature file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignatureTargetV1 {
    /// Hash algorithm: currently always `blake3`.
    pub hash_algorithm: String,
    /// Lower-case hex BLAKE3-256 of the target bytes.
    pub hash: String,
    /// Target length in bytes.
    pub size: u64,
}

/// Parsed contents of a `.sig` sidecar (format version 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignatureV1 {
    /// Format version; must be 1.
    pub version: u32,
    /// Signature algorithm; must be `ed25519`.
    pub algorithm: String,
    /// Lower-case hex Ed25519 public key (32 bytes).
    pub public_key: String,
    /// Lower-case hex Ed25519 signature (64 bytes).
    pub signature: String,
    /// Digest and size of the signed target.
    pub target: SignatureTargetV1,
    /// RFC 3339 UTC creation time, e.g. `2026-11-03T12:00:00Z`.
    pub created_at: String,
    /// Optional human-readable signer name; omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signer: Option<String>,
}

/// Outcome of a successful verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedSignature {
    /// Public key that verified the signature.
    pub public_key: [u8; 32],
    /// Signer name recorded in the sidecar, when present.
    pub signer: Option<String>,
    /// Creation time recorded in the sidecar.
    pub created_at: String,
    /// BLAKE3 digest of the bytes that were verified.
    pub hash: [u8; 32],
    /// Number of bytes that were verified.
    pub size: u64,
}

/// A freshly generated keypair in every shape the CLI needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedKeypair {
    /// PKCS#8 PEM private key (`-----BEGIN PRIVATE KEY-----`).
    pub private_key_pem: String,
    /// Lower-case hex Ed25519 public key.
    pub public_key_hex: String,
    /// Public key file text without a name: `odc-ed25519 <hex>\n`.
    pub public_key_file: String,
}

impl SignatureV1 {
    /// Serialise to the canonical JSON form: two-space indent, one final newline.
    ///
    /// # Errors
    ///
    /// Returns [`Error::SignatureFormat`] (`ODC-E032`) when a field is invalid.
    pub fn to_json(&self) -> Result<String> {
        self.validate()?;
        let mut text = serde_json::to_string_pretty(self)
            .map_err(|error| Error::SignatureFormat(format!("cannot encode signature: {error}")))?;
        text.push('\n');
        Ok(text)
    }

    /// Parse and validate a signature file body.
    ///
    /// # Errors
    ///
    /// Returns [`Error::SignatureFormat`] (`ODC-E032`) for malformed JSON or an
    /// invalid field.
    pub fn from_json(text: &str) -> Result<Self> {
        let signature: Self = serde_json::from_str(text)
            .map_err(|error| Error::SignatureFormat(format!("invalid signature JSON: {error}")))?;
        signature.validate()?;
        Ok(signature)
    }

    /// Check every field this implementation depends on.
    ///
    /// # Errors
    ///
    /// Returns [`Error::SignatureFormat`] (`ODC-E032`) when a field is missing,
    /// unsupported, or not the required lower-case hex.
    pub fn validate(&self) -> Result<()> {
        if self.version != SIGNATURE_VERSION {
            return Err(Error::SignatureFormat(format!(
                "unsupported signature version {} (expected {SIGNATURE_VERSION})",
                self.version
            )));
        }
        if self.algorithm != SIGNATURE_ALGORITHM {
            return Err(Error::SignatureFormat(format!(
                "unsupported signature algorithm {:?} (expected {SIGNATURE_ALGORITHM:?})",
                self.algorithm
            )));
        }
        if self.target.hash_algorithm != SIGNATURE_HASH_ALGORITHM {
            return Err(Error::SignatureFormat(format!(
                "unsupported target hash algorithm {:?} (expected {SIGNATURE_HASH_ALGORITHM:?})",
                self.target.hash_algorithm
            )));
        }
        hex_bytes::<32>(&self.public_key, "public_key")?;
        hex_bytes::<64>(&self.signature, "signature")?;
        hex_bytes::<32>(&self.target.hash, "target.hash")?;
        if OffsetDateTime::parse(&self.created_at, &Rfc3339).is_err() {
            return Err(Error::SignatureFormat(format!(
                "created_at is not an RFC 3339 timestamp: {:?}",
                self.created_at
            )));
        }
        if self.signer.as_deref() == Some("") {
            return Err(Error::SignatureFormat(
                "signer must not be an empty string".to_string(),
            ));
        }
        Ok(())
    }
}

/// Generate a fresh Ed25519 keypair.
///
/// The private key is PKCS#8 PEM; callers are responsible for writing it with
/// mode `0600`.
///
/// # Errors
///
/// Returns [`Error::KeyFormat`] (`ODC-E035`) if the PEM encoding fails.
pub fn generate_keypair() -> Result<GeneratedKeypair> {
    let signing_key = SigningKey::generate(&mut OsRng);
    let pem = signing_key
        .to_pkcs8_pem(LineEnding::LF)
        .map_err(|error| Error::KeyFormat(format!("cannot encode private key: {error}")))?;
    let public_key_hex = hex_encode(signing_key.verifying_key().as_bytes());
    Ok(GeneratedKeypair {
        private_key_pem: pem.to_string(),
        public_key_file: format!("{PUBLIC_KEY_PREFIX} {public_key_hex}\n"),
        public_key_hex,
    })
}

/// Render a public key file line: `odc-ed25519 <hex> [name]\n`.
///
/// # Errors
///
/// Returns [`Error::SignatureFormat`] (`ODC-E032`) when `public_key_hex` is not
/// 32 bytes of lower-case hex, or `name` is empty or contains whitespace.
pub fn public_key_file_text(public_key_hex: &str, name: Option<&str>) -> Result<String> {
    hex_bytes::<32>(public_key_hex, "public_key")?;
    if let Some(name) = name {
        if name.is_empty() || name.chars().any(char::is_whitespace) {
            return Err(Error::SignatureFormat(
                "public key name must be non-empty and contain no whitespace".to_string(),
            ));
        }
        Ok(format!("{PUBLIC_KEY_PREFIX} {public_key_hex} {name}\n"))
    } else {
        Ok(format!("{PUBLIC_KEY_PREFIX} {public_key_hex}\n"))
    }
}

/// Parse a public key file produced by `odc keygen` (`--pub PATH`).
///
/// # Errors
///
/// Returns [`Error::SignatureFormat`] (`ODC-E032`) when the text is not a single
/// well-formed `odc-ed25519 <hex> [name]` line.
pub fn parse_public_key_file(text: &str) -> Result<[u8; 32]> {
    let line = text.trim_end_matches(['\n', '\r']);
    if line.contains('\n') || line.contains('\r') {
        return Err(Error::SignatureFormat(
            "public key file must contain exactly one line".to_string(),
        ));
    }
    let mut parts = line.split_whitespace();
    let prefix = parts.next().unwrap_or_default();
    if prefix != PUBLIC_KEY_PREFIX {
        return Err(Error::SignatureFormat(format!(
            "public key file must start with {PUBLIC_KEY_PREFIX:?}"
        )));
    }
    let key = parts
        .next()
        .ok_or_else(|| Error::SignatureFormat("public key file has no key".to_string()))?;
    let bytes = hex_bytes::<32>(key, "public_key")?;
    let _name = parts.next();
    if parts.next().is_some() {
        return Err(Error::SignatureFormat(
            "public key file has trailing fields".to_string(),
        ));
    }
    Ok(bytes)
}

/// Sign arbitrary bytes with a PKCS#8 PEM Ed25519 private key.
///
/// # Errors
///
/// Returns [`Error::KeyFormat`] (`ODC-E035`) for a malformed private key and
/// [`Error::SignatureFormat`] (`ODC-E032`) if the timestamp cannot be encoded.
pub fn sign_bytes(data: &[u8], key_pem: &str, signer: Option<&str>) -> Result<SignatureV1> {
    let signing_key = signing_key_from_pem(key_pem)?;
    let digest = hash::blake3(data);
    let signature = signing_key.sign(&signing_message(&digest));
    Ok(SignatureV1 {
        version: SIGNATURE_VERSION,
        algorithm: SIGNATURE_ALGORITHM.to_string(),
        public_key: hex_encode(signing_key.verifying_key().as_bytes()),
        signature: hex_encode(&signature.to_bytes()),
        target: SignatureTargetV1 {
            hash_algorithm: SIGNATURE_HASH_ALGORITHM.to_string(),
            hash: hash::to_hex(&digest),
            size: data.len() as u64,
        },
        created_at: now_rfc3339()?,
        signer: signer.map(str::to_string),
    })
}

/// Sign the contents of `path`.
///
/// # Errors
///
/// I/O failures surface as [`Error::Io`] (`ODC-E019`); key and timestamp
/// problems behave exactly like [`sign_bytes`].
pub fn sign_file(path: &Path, key_pem: &str, signer: Option<&str>) -> Result<SignatureV1> {
    let data = fs::read(path)?;
    sign_bytes(&data, key_pem, signer)
}

/// Verify a signature against `data`, trusting the key embedded in the sidecar.
///
/// # Errors
///
/// * [`Error::SignatureFormat`] (`ODC-E032`) - malformed sidecar.
/// * [`Error::SignatureTargetMismatch`] (`ODC-E033`) - hash or size differs.
/// * [`Error::SignatureInvalid`] (`ODC-E034`) - Ed25519 verification failed.
pub fn verify_bytes(data: &[u8], sig: &SignatureV1) -> Result<VerifiedSignature> {
    verify_bytes_with_key(data, sig, None)
}

/// Verify a signature against `data`, optionally pinning the trusted key.
///
/// When `expected_public_key` is `Some`, a sidecar signed by a different key is
/// rejected with [`Error::SignatureInvalid`] (`ODC-E034`) even if its own
/// signature is mathematically valid.
///
/// # Errors
///
/// Same as [`verify_bytes`].
pub fn verify_bytes_with_key(
    data: &[u8],
    sig: &SignatureV1,
    expected_public_key: Option<&[u8; 32]>,
) -> Result<VerifiedSignature> {
    sig.validate()?;
    let digest = hash::blake3(data);
    let signed_hash = hex_bytes::<32>(&sig.target.hash, "target.hash")?;
    let size = data.len() as u64;
    if signed_hash != digest || sig.target.size != size {
        return Err(Error::SignatureTargetMismatch(format!(
            "target mismatch: signature covers {} ({} bytes), file is {} ({} bytes)",
            sig.target.hash,
            sig.target.size,
            hash::to_hex(&digest),
            size
        )));
    }

    let key_bytes = hex_bytes::<32>(&sig.public_key, "public_key")?;
    if let Some(expected) = expected_public_key {
        if expected != &key_bytes {
            return Err(Error::SignatureInvalid(format!(
                "signature key {} does not match the trusted key {}",
                sig.public_key,
                hash::to_hex(expected)
            )));
        }
    }
    let verifying_key = VerifyingKey::from_bytes(&key_bytes)
        .map_err(|error| Error::SignatureInvalid(format!("invalid public key: {error}")))?;
    let signature_bytes = hex_bytes::<64>(&sig.signature, "signature")?;
    let signature = Signature::from_bytes(&signature_bytes);
    verifying_key
        .verify(&signing_message(&digest), &signature)
        .map_err(|_| Error::SignatureInvalid("Ed25519 verification failed".to_string()))?;

    Ok(VerifiedSignature {
        public_key: key_bytes,
        signer: sig.signer.clone(),
        created_at: sig.created_at.clone(),
        hash: digest,
        size,
    })
}

/// Verify the contents of `path` against `sig`.
///
/// # Errors
///
/// I/O failures surface as [`Error::Io`] (`ODC-E019`); verification failures
/// behave exactly like [`verify_bytes`].
pub fn verify_file(path: &Path, sig: &SignatureV1) -> Result<VerifiedSignature> {
    verify_file_with_key(path, sig, None)
}

/// Verify the contents of `path` against `sig`, optionally pinning the key.
///
/// # Errors
///
/// Same as [`verify_file`].
pub fn verify_file_with_key(
    path: &Path,
    sig: &SignatureV1,
    expected_public_key: Option<&[u8; 32]>,
) -> Result<VerifiedSignature> {
    let data = fs::read(path)?;
    verify_bytes_with_key(&data, sig, expected_public_key)
}

/// Sidecar path for `target`: `<target>.sig`.
#[must_use]
pub fn signature_path(target: &Path) -> PathBuf {
    let mut name = target.as_os_str().to_os_string();
    name.push(SIGNATURE_SUFFIX);
    PathBuf::from(name)
}

/// Read and parse a signature file.
///
/// # Errors
///
/// I/O failures surface as [`Error::Io`]; malformed content as
/// [`Error::SignatureFormat`] (`ODC-E032`).
pub fn read_signature_file(path: &Path) -> Result<SignatureV1> {
    let text = fs::read_to_string(path)?;
    SignatureV1::from_json(&text)
}

/// Write a signature file in canonical form.
///
/// # Errors
///
/// I/O failures surface as [`Error::Io`]; an invalid in-memory signature as
/// [`Error::SignatureFormat`] (`ODC-E032`).
pub fn write_signature_file(path: &Path, signature: &SignatureV1) -> Result<()> {
    fs::write(path, signature.to_json()?)?;
    Ok(())
}

/// The 43-byte message actually fed to Ed25519: the domain prefix followed by
/// the 32-byte BLAKE3 digest.
#[must_use]
pub fn signing_message(digest: &[u8; 32]) -> Vec<u8> {
    let mut message = Vec::with_capacity(SIGNATURE_DOMAIN.len() + digest.len());
    message.extend_from_slice(SIGNATURE_DOMAIN);
    message.extend_from_slice(digest);
    message
}

/// Lower-case hex encoding of arbitrary bytes.
#[must_use]
pub fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

/// Parse exactly `N` bytes of lower-case hex.
///
/// # Errors
///
/// Returns [`Error::SignatureFormat`] (`ODC-E032`) for wrong length, upper-case
/// digits, or non-hex characters.
pub fn hex_bytes<const N: usize>(value: &str, field: &str) -> Result<[u8; N]> {
    if value.len() != N * 2 {
        return Err(Error::SignatureFormat(format!(
            "{field} must be {} hex characters, got {}",
            N * 2,
            value.len()
        )));
    }
    let bytes = value.as_bytes();
    let mut out = [0u8; N];
    for index in 0..N {
        let high = hex_digit(bytes[index * 2], field)?;
        let low = hex_digit(bytes[index * 2 + 1], field)?;
        out[index] = (high << 4) | low;
    }
    Ok(out)
}

fn hex_digit(byte: u8, field: &str) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(Error::SignatureFormat(format!(
            "{field} must be lower-case hex (offending byte {:?})",
            char::from(byte)
        ))),
    }
}

fn signing_key_from_pem(pem: &str) -> Result<SigningKey> {
    SigningKey::from_pkcs8_pem(pem)
        .map_err(|error| Error::KeyFormat(format!("cannot parse PKCS#8 private key: {error}")))
}

fn now_rfc3339() -> Result<String> {
    // The sidecar format fixes whole seconds; drop the sub-second part so the
    // JSON always carries an RFC 3339 timestamp like 2026-11-03T12:00:00Z.
    let now = OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .map_err(|error| Error::SignatureFormat(format!("cannot round created_at: {error}")))?;
    now.format(&Rfc3339)
        .map_err(|error| Error::SignatureFormat(format!("cannot format created_at: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_prefix_is_eleven_bytes() {
        assert_eq!(SIGNATURE_DOMAIN.len(), 11);
        assert_eq!(SIGNATURE_DOMAIN, b"ODC-SIG-V1\0");
    }

    #[test]
    fn signing_message_is_domain_plus_digest() {
        let digest = [7u8; 32];
        let message = signing_message(&digest);
        assert_eq!(message.len(), 43);
        assert_eq!(&message[..11], SIGNATURE_DOMAIN);
        assert_eq!(&message[11..], &digest);
    }

    #[test]
    fn hex_helpers_round_trip() {
        let bytes = [0x00u8, 0x0f, 0xa5, 0xff];
        assert_eq!(hex_encode(&bytes), "000fa5ff");
        assert_eq!(hex_bytes::<4>("000fa5ff", "test").expect("hex"), bytes);
        assert!(hex_bytes::<4>("000FA5FF", "test").is_err());
        assert!(hex_bytes::<4>("zz", "test").is_err());
    }

    #[test]
    fn public_key_file_shapes() {
        let hex = "11".repeat(32);
        let line = public_key_file_text(&hex, Some("alice")).expect("line");
        assert_eq!(line, format!("odc-ed25519 {hex} alice\n"));
        assert_eq!(parse_public_key_file(&line).expect("parse"), [0x11u8; 32]);
        assert!(parse_public_key_file("nope").is_err());
        assert!(parse_public_key_file(&format!("odc-ed25519 {hex} a b")).is_err());
    }
}
