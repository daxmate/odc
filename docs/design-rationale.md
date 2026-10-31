# Design rationale

This document records *why* ODC looks the way it does. The normative rules live
in [odc-spec.md](odc-spec.md); this file explains the trade-offs behind them so
that future changes do not accidentally undo a decision.

Sources: the original design discussion with DeepSeek (transcribed in the
project memory file `odc-spec-draft-v0.1.md`) and the implementation
experience from M0. Nothing here is invented after the fact.

## 1. Content-defined chunking instead of fixed 64 KiB blocks

The first draft cut the dictionary into fixed 64 KiB blocks. That fails the
core use case — sharing small patches — as soon as one entry is inserted near
the front of the database:

* fixed blocks shift every following boundary when an entry is inserted or
  removed;
* every block hash changes;
* the "patch" degenerates into a full download.

ODC therefore uses a gear-hash CDC over the record byte stream
(`key || 0x00 || content_hash`), with min 16 KiB, max 256 KiB and a 16-bit
mask. Boundaries depend only on content, so a local edit changes one block,
not the whole file. Records are atomic: a record is never split across blocks.

Consequences we accept:

* chunking is slightly more work than copying fixed-size slices;
* block sizes vary, so storage statistics are approximate;
* the gear table itself is part of the format (`cdc::gear_table` derives it
  from BLAKE3 so every implementation can regenerate it).

## 2. `block_hash` hashes the raw, uncompressed block bytes

Compressing first and hashing the compressed bytes would make the hash depend
on the compressor version. Two implementations with different zstd levels
would then disagree on block identity, and patches would stop being portable.

Hashing raw bytes keeps block identity independent of the storage codec:
compression is a local storage decision, recorded in `blocks.codec`, and any
block can be re-compressed without changing its hash.

## 3. `commit_hash` covers content only

`commit_hash` is BLAKE3 over:

```
domain "odc-commit-v1" || parent || parent2 || entry_root || block_root || entry_count(u64 LE)
```

Timestamps, author and message are deliberately excluded. Two people importing
the same dictionary at different times must arrive at the same commit hash, or
"same content" would be indistinguishable from "same history". This mirrors
Git's tree hashing more than its commit hashing, and it is what makes the
determinism tests meaningful.

## 4. Structured payload plus `raw_html`

Dictionary exports (MDX, DSL, StarDict) carry presentation-specific HTML that
no schema can fully describe. ODC stores a structured payload:

```
display, phonetic[], defs[{pos, sense, examples[], labels[]}], extra fields
```

and keeps the source HTML in an optional `raw_html` field. This gives editors
something to diff field by field while still allowing lossless round-trips of
legacy data. Unknown keys are preserved in `extra` in canonical CBOR order, so
adding fields later does not break older readers.

## 5. SQLite as the container

A single `.odc` file is just a SQLite database. Reasons:

* one file, atomic transactions, no custom pack format to debug;
* indexes for word lookups come for free;
* `zstd`-compressed blocks are stored as blobs, so block identity stays
  separate from storage layout;
* any language with an SQLite binding can inspect a container without ODC
  tooling.

The spec deliberately treats the table layout as a reference implementation
detail: interoperability is defined by the logical structures (blocks, entries,
commits, patches), not by SQLite page layout.

## 6. Two-level diff

A commit stores a Merkle root over entries and a Merkle root over blocks.
Comparing two commits happens in two stages:

1. **block level** — skip identical blocks by hash (cheap, coarse);
2. **entry level** — compare `content_hash` per word and report
   added / removed / modified words (exact, user-facing).

This is what lets a 1M-entry dictionary be diffed in seconds, and it is the
data model the patch format (`ODP1`) reuses.

## 7. Error codes are part of the contract

Every failure maps to a stable code (`ODC-E001` ... ) printed by the CLI and
documented in the spec. Without that, scripts cannot distinguish "malformed
block" from "I/O error", and alternative implementations cannot be tested
against the same expectations.

## 8. The working set does not cache committed payloads

`staging` used to keep the uncompressed CBOR payload of every entry, including
entries already stored in `blocks`. On a 148k-entry dictionary that duplicated
~790 MiB of text next to a ~115 MiB block store, so half of `odc.db` was a
redundant cache.

Committed entries already have an exact location (`entries.word` →
`block_hash`, `offset`, `length`) and a `content_hash`. Keeping the body a
second time buys nothing: the only thing the working set needs for those rows
is the key, the display form and the content hash that `status` compares.
Uncommitted rows still carry their payload, because no block holds them yet.

When a payload is needed (commit re-chunks the whole stream, `working_entry`
serves a working-set read), it is materialised from the block and checked
against `content_hash`; a slice that does not decode to the expected record is
an `Inconsistent` error instead of silently returning the wrong word. Commit
snapshots those slices into a temporary table before rebuilding `entries`, then
reads them back through a one-block cache, so peak memory stays bounded by a
single block plus one record, as before.

Commit rebuilds `staging` after clearing the payloads, so the freed bytes leave
the table's b-tree instead of lingering as in-page free space. When a commit
frees at least 64 MiB of cached payloads it also runs `VACUUM`: a dictionary
import that used to leave half of `odc.db` as dead cache would otherwise keep
the file at its peak size, and paying for one rewrite after a bulk import is
cheaper than carrying that space forever. Small edits stay vacuum-free.

The same layout is preserved for old databases: opening one rebuilds `staging`
with a nullable `payload` column and copies every row, including uncommitted
payloads.
