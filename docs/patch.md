# Patch transfer (M3)

ODC ships content-addressed blocks, so two repositories that share history
already share most of their bytes. A patch carries only the blocks that the
target commit adds on top of an exact base commit, plus the index delta of the
words whose content hash changed. The byte layout is frozen by
[`docs/odc-spec.md`](odc-spec.md) section 11 (`ODP1`); this document covers
the M3 usage, behaviour and limits.

## Usage

```console
# On a machine that has both revisions:
odc patch create <base> <target> -o fix.odp [--json]

# On the receiver, with HEAD at <base> or on a branch that descends from it:
odc patch verify fix.odp [--json]
odc patch apply fix.odp [--json]
```

Revisions follow the usual syntax (`HEAD`, `HEAD~1`, or a unique hash prefix
of at least four hex characters).

## Three-way application (M4)

When HEAD is exactly the base commit, `apply` advances HEAD to the target as
before. When HEAD descends from the base, the patch target is installed as a
commit but HEAD does not move; the target is then merged into HEAD three-way
(base = the patch base, ours = HEAD, theirs = target) and a merge commit is
written when the merge is unambiguous.

A conflicted merge leaves HEAD, the entry index and the working set untouched,
records the conflicts (see `docs/merge.md`), and exits with code 5. Finish it on
the receiver with:

```console
odc conflicts --json
odc resolve <word> --ours|--theirs|--union|--file payload.cbor
odc merge --continue
```

The target commit and its blocks stay in the repository so the resolution can
complete; a second `apply` of the same file therefore reports `ODC-E026`.

`create` reports the number of inline blocks, base blocks, index changes and
the file size. `apply` reports the blocks inserted, index changes and target
commit. `verify` only checks that the patch is self-consistent and that its
base blocks exist locally; it does not change the repository.

Every patch failure exits with status 4 and prints the stable error code on
stderr, matching `odc lookup`.

## What travels

* `base_blocks` lists the target blocks the receiver is expected to already
  have, in ascending hash order. Blocks that exist only in the base commit are
  not listed: the rebuilt target index never references them.
* `new_blocks` contains the target blocks that are absent from the base, in
  ascending hash order, each stored with its codec and raw size. Unchanged
  blocks are never carried.
* `index_rows` lists `(word, content_hash)` for every word that was added or
  modified relative to the base. Removal needs no row: the target index is
  rebuilt from the target block set, so words absent from the blocks simply
  disappear.

The patch is zstd-compressed per block and ends with
`BLAKE3("odc-patch-v1" || body)`.

## Apply semantics

`apply` refuses to touch the repository unless everything checks out:

1. The framing is complete (truncation is `ODC-E002`) and the checksum matches
   (`ODC-E023`).
2. The target commit must not exist yet (`ODC-E026`).
3. HEAD must equal `base`. If HEAD descends from the base, the patch needs a
   three-way merge (`ODC-E025`, M4); an unrelated HEAD is refused with
   `ODC-E024`.
4. Every base block must be present locally (`ODC-E009`); inline blocks are
   decoded and their content hash recomputed.
5. The target index and Merkle roots are rebuilt from the block set, the index
   delta must match exactly, and the recomputed `commit_hash` must equal
   `target`.

Only then is a single SQLite transaction opened: insert blocks, replace the
entries index, insert the commit and its compressed index snapshot, and move
`HEAD`. Any error rolls the transaction back, so a rejected patch leaves the
commit hash and entry count untouched.

## Size

Patch size is proportional to the number of blocks the edit touches, not to
the size of the dictionary. With 100,000 synthetic entries and 100 consecutive
edited definitions, the reference implementation produces a patch around
22 KiB (budget: under 50 KiB). Entries spread evenly across the dictionary each
touch their own block, so a scattered edit ships roughly one block per changed
region — this is inherent to the block-level format.

## Limits (M3)

* `create` requires the target to descend from the base: patches describe a
  forward step. Applying to a descendant HEAD is a three-way merge (see above).
  `ODC-E025` is no longer produced by this implementation; the code stays
  reserved in the error table.
* The base commit's blocks must still be stored locally, because the reference
  implementation rebuilds the historical block set from the block pool.
  `odc gc` keeps the current index and ref chains; creating a patch from a
  revision whose blocks were collected fails with `ODC-E009`.
* Section 11 does not carry commit metadata such as author, timestamp or
  message. The applied commit uses the default author and a deterministic
  `apply patch <target>` message; every field covered by `commit_hash`
  (parents, roots, entry count) is byte-identical to the target, so the commit
  hash of a round-tripped mirror equals the original.
* Flags bit 0 is always set by this implementation (full index delta). A patch
  with only the block difference (bit 0 clear) is accepted and applied using
  the rebuilt index.

## Error codes

| Code | Meaning |
| --- | --- |
| ODC-E002 | Truncated patch file |
| ODC-E003 | Trailing bytes after the checksum |
| ODC-E008 | Recomputed commit hash does not match the target |
| ODC-E009 | A base block is missing locally |
| ODC-E018 | Index delta or stored index does not match the commit |
| ODC-E023 | Patch checksum does not match the body |
| ODC-E024 | Base commit is absent or not HEAD |
| ODC-E025 | HEAD descends from the base; three-way merge required (M4) |
| ODC-E026 | Target commit already exists (patch already applied) |
| ODC-E027 | Malformed patch structure (bad sizes, non-UTF-8 word) |
