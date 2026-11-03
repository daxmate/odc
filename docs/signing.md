# Detached signatures (v0.10 P1)

ODC release artifacts can carry a detached Ed25519 signature in a sidecar file,
so the container format itself is unchanged: `dict.odc` keeps its bytes and
`dict.odc.sig` sits next to it. The normative layout is frozen by
[`docs/odc-spec.md`](odc-spec.md) ("Signature sidecar v1"); this document covers
the commands, the verification flow and the limits of the trust model.

Signing is deliberately separate from packaging: you can sign any file (a
`.odc` pack, a patch, a tarball) and a recipient without the repository can
verify it.

## Commands

```console
# 1. Generate a keypair (PKCS#8 PEM private key + one-line public key file).
odc keygen --name release
#   private key: ~/.config/odc/keys/release.key   (mode 0600)
#   public key:  ~/.config/odc/keys/release.pub   (mode 0644)

# 2. Sign a file; the sidecar defaults to <FILE>.sig.
odc sign dict.odc --key ~/.config/odc/keys/release.key --signer "alice"

# 3. Verify (trusting the key embedded in the sidecar).
odc verify-signature dict.odc --json

# 3b. Verify while pinning a key you already trust.
odc verify-signature dict.odc --pub ~/.config/odc/keys/release.pub
odc verify-signature dict.odc --pubkey <64 hex characters>
```

Key directory resolution for `keygen` / `sign`:

1. `--out DIR` (keygen only),
2. `$ODC_KEY_DIR`,
3. `$HOME/.config/odc/keys`.

The default key name is `default`; `odc sign` without `--key` uses
`<key dir>/default.key`. Existing files are never overwritten without
`--force`.

### Exit codes

| Code | Meaning |
| --- | --- |
| 0 | signature valid (`verify-signature`), or command succeeded |
| 1 | signature invalid; JSON reports `{"valid": false, "error_code": "ODC-E0xx"}` |
| 2 | usage or I/O error (missing file, bad `--pubkey` hex, conflicting flags) |

With `--json` a valid run prints
`{"valid": true, "public_key": ..., "created_at": ..., "hash": ..., "size": ...}`
(plus `signer` when recorded).

## What is signed

The Ed25519 message is **43 bytes**:

```
"ODC-SIG-V1" || 0x00        (11 bytes, domain separator)
BLAKE3-256(target file)     (32 bytes)
```

Verification recomputes the BLAKE3 digest **and** the file size and rejects the
sidecar when either differs (`ODC-E033`), so a signature cannot be replayed
onto a different file even when the signature bytes are intact. The JSON
structure itself is not signed: `created_at` and `signer` are informational and
must not be treated as authenticated.

## Failure codes

| Code | Cause |
| --- | --- |
| ODC-E032 | sidecar is not valid v1 JSON (bad field, upper-case hex, wrong length) |
| ODC-E033 | target hash or size mismatch |
| ODC-E034 | Ed25519 verification failed, or the sidecar key differs from the pinned key |
| ODC-E035 | private key is not valid PKCS#8 Ed25519 PEM |

## Threat model and limits

* **What it proves**: the holder of the matching private key signed these exact
  bytes.
* **What it does not prove**: *who* that key belongs to. There is no CA, no
  revocation list and no timestamping authority in v1. `created_at` is
  self-asserted and comes from the signer's clock.
* **Trust on first use (TOFU)**, key pinning stores and the release wizard are
  planned for later v0.10 phases; for now pin keys out of band (for example in
  a repository README or a signed message).
* A leaked private key can sign anything until recipients stop trusting the
  matching public key; there is no key-compromise recovery in v1.
* Verification reads the whole target into memory; very large artifacts need
  the same care as `odc verify`.

## Trust store and releases

A valid sidecar only proves that some key signed the bytes; it says nothing
about whether that key should be trusted. v0.10 P2 adds two layers on top:

* odc publish packages a full pack (and optionally a patch) with a manifest
  and a signature per file.
* odc trust add/list/remove maintains a per-repository list of accepted
  publisher keys, seeded by trust-on-first-use when a repository has none.

See [docs/release.md](release.md) for the manifest schema, the TOFU rules and
the verification flow. The sidecar layout itself is unchanged by that work.
