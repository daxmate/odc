# Signed releases (v0.10 P2)

A release is a directory that can be copied to a web server or a USB stick and
verified offline: one full pack, an optional incremental patch, a manifest and
a detached signature for every file. Signing reuses the sidecar format frozen
in [docs/signing.md](signing.md); nothing about the container format changes.

## Directory layout

```text
demo-2026.11/
  demo-2026.11.odc        full pack (every commit reachable from HEAD)
  demo-2026.11.odc.sig    sidecar signature for the pack
  demo-2026.11.odp        incremental patch base..HEAD (optional)
  demo-2026.11.odp.sig    sidecar signature for the patch
  release.json            manifest (signed by the same key)
  release.json.sig        sidecar signature for the manifest
```

File names are derived as <name>-<label>.odc and <name>-<label>.odp. The name
defaults to the repository directory name; the label is mandatory and must be a
plain file-name component (letters, digits, dots, dashes and underscores are
fine; separators, whitespace and control characters are rejected).

## Manifest v1

```json
{
  "format": "odc-release",
  "version": 1,
  "name": "demo",
  "label": "2026.11",
  "created_at": "2026-11-03T12:00:00Z",
  "head": "<64 hex>",
  "base": "<64 hex, optional>",
  "artifacts": [
    {
      "kind": "pack",
      "path": "demo-2026.11.odc",
      "size": 12345,
      "blake3": "<64 hex>",
      "signature": { "version": 1, "algorithm": "ed25519", "public_key": "..." }
    }
  ],
  "notes": "optional text",
  "publisher": { "public_key": "<64 hex>", "name": "alice" }
}
```

Rules enforced by the reference implementation:

* format is literally odc-release and version is 1.
* head and base are lower-case hex commit hashes.
* artifacts contains exactly one pack and at most one patch.
* path is a plain file name inside the release directory (no separators).
* Every signature uses the publisher key, covers the listed blake3 and size,
  and the inline copy must match the sidecar on disk.
* The manifest is serialised as UTF-8 with two-space indentation and a final
  newline, then signed byte for byte; release.json.sig is that signature.

created_at and signer are metadata: any edit to the manifest invalidates the
signature, including whitespace.

## Publishing

```console
odc --repo ~/dict publish --out dist/demo-2026.11 --label 2026.11 \
    --name demo --base v0.9 --signer alice --notes "first signed build"
```

* The pack is a full export (every commit reachable from HEAD), exactly like
  odc pack with default options.
* With --base the release also carries a base..HEAD patch, so existing readers
  can update with odc patch apply.
* Every artifact gets a sidecar signature, and the manifest is signed last.
* --key selects the private key (default <key dir>/default.key; the key
  directory follows the keygen rules in docs/signing.md).
* Existing files are never replaced unless --force is passed. The check runs
  before anything is written, so a refused run leaves the directory untouched.
* --json prints out_dir, manifest, head, base, artifacts (kind, path, size,
  blake3) and the publisher key.

## The publisher trust store

Each repository keeps the publishers it is willing to accept in the publishers
table of odc.db. Rows carry the lower-case hex public key, an optional name,
the Unix timestamp and the source: manual or tofu.

```console
odc --repo ~/dict trust add --pub ~/.config/odc/keys/default.pub --name alice
odc --repo ~/dict trust add --pubkey <64 hex>
odc --repo ~/dict trust list [--json]
odc --repo ~/dict trust remove <64 hex>
```

* Re-adding the same key is idempotent; the existing record is returned.
* A name that already belongs to a different key is refused with ODC-E036.
* Removing a key only stops future acceptances; it does not rewrite history.

## TOFU (trust on first use)

Signature verification for patches and releases uses this policy:

1. If the signing key is already trusted, the artifact is accepted.
2. If the trust store is empty and --trust-new is set, the key is recorded with
   source tofu and the artifact is accepted. This happens once per repository.
3. Otherwise the artifact is refused with ODC-E036 and the caller must run
   odc trust add explicitly.

Naming rules apply to TOFU as well: a signer name already owned by another key
makes the recording fail with ODC-E036.

## Verifying a release

```console
odc verify-release dist/demo-2026.11 --pubkey <64 hex>
odc --repo ~/dict verify-release dist/demo-2026.11
```

Verification always:

1. Parses and validates release.json.
2. Checks release.json.sig against the publisher key, recomputing the digest.
3. Re-reads every artifact, recomputing size and BLAKE3 and re-verifying the
   inline signature with the publisher key.

With --pub or --pubkey the publisher key is pinned and the trust store is not
consulted. Without a pin the publisher must already be trusted in --repo.

| Code | Meaning |
| --- | --- |
| 0 | release valid |
| 1 | release invalid; JSON reports valid:false, error_code and error |
| 2 | usage or I/O error |

## Signed patches

```console
odc sign fix.odp --out fix.odp.sig
odc --repo ~/dict patch apply fix.odp --sig fix.odp.sig --trust-new
```

The signature and trust checks run before patch::apply, so a rejected patch
leaves HEAD, the working set and blocks untouched. Without --sig the command
keeps its previous behaviour.

## Limits

* Nothing is encrypted: a release is public once published. The trust store
  only controls which signer a repository accepts.
* There is no key rotation or revocation in v1; removing a key does not
  invalidate releases that were verified earlier.
* Publishing reads the pack back to hash and sign it, so release size is bounded
  by memory.
* Directory traversal is rejected, but a manifest can still point at any plain
  file name inside the release directory.

## App 发布向导（macOS）

`ODict Desktop` 把 P2 的签名发布功能封装成图形界面：钥匙串保存 Ed25519 身份，编辑器工具栏的
「发布…」调用 `odc_publish`（可选 base..HEAD 增量包），「信任…」管理当前库的
publishers。步骤、限制与增量包的记录方式见
[`apps/apple/README.md`](../apps/apple/README.md)。

