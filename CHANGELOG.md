# Changelog

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

* Storage layout version 2 (`meta.schema_version = 2`): blocks are addressed by
  a local integer id instead of a 32-byte hash in every entry row, `entries.content_hash`
  and `field_hashes` keep only the first 8 bytes of BLAKE3, and the
  `entries_block` index is gone. Commit hashes are unchanged: the Merkle leaf
  still uses the full digest computed from the payload.
* Blocks are written with zstd level 9 instead of 3 (roughly 8% smaller on
  dictionary-sized payloads).
* CDC block target is 1 MiB instead of 256 KiB, so each zstd frame covers a
  longer window at the cost of coarser dedupe granularity.
* A successful commit drops the working set instead of keeping an index mirror;
  the rows are rebuilt from `entries` on the next write.
* Version 1 databases stay readable and are rewritten in place by the first
  write. On LDOCE6 the import drops from 191.4 MiB to ~121 MiB.

### Added

* Apple clients: `apps/apple/` ships the **ODict** (iOS) and **ODict Desktop**
  (macOS) SwiftUI targets on a shared `ODictKit` package. v0 opens a
  dictionary, prefix-searches it through `ODCFFI` and renders entries
  read-only. `build-app.sh` builds both targets end to end.
* ODict Desktop imports MDX: the new `odc_import_mdx_to_odc(mdx_path,
  out_odc_path, report_json_out, error_message)` C entry point imports a
  version 1.2 / 2.0 dictionary into a temporary repository, commits it, packs
  a shallow `.odc` and renames it into place atomically (an existing target
  is an error, never overwritten). The Swift side converts on a background
  thread, writes `<name>.odc` next to the source (falling back to
  `~/Documents`, `-1`/`-2` suffixes on collision), reports the stage without
  faking a percentage, and opens the result. Corrupt record blocks are
  skipped and summarised in the completion alert. iOS keeps rejecting
  `.mdx`.

* MDX (MDict 1.2 / 2.0) import through `odc import --format mdx`, wrapping
  the `mdictlib` reader (MIT). Duplicate normalised keys receive a
  `U+001F<n>` suffix; empty keys and records over 1 MiB are skipped and
  counted.
* MDX 2.0 corrupt record blocks no longer abort the import. `odc-core` scans
  the record section first (`mdx_scan`), and imports through a temporary
  repaired copy in which each damaged block is replaced by a sentinel payload
  of exactly its declared size, so every key and record offset still lines
  up. Sentinel entries are dropped and reported: `odc import` now ends with
  `mdx: skipped 1 corrupt record block (N entries)`. A version 2 file whose
  record section cannot be located still imports as-is, exactly as before,
  with a one-line warning that recovery is unavailable. Details, measured
  data and limits live in `docs/mdx-compat.md`.
* MDX import is now a public core API: `odc_core::import_mdx(path, repo,
  MdxImportOptions) -> MdxImportReport`, with `skip_corrupt_blocks`,
  `prune`, `limit` and a progress callback. The CLI only formats the report.
* `odc lookup` (exit code 4 when the word is missing), `odc search --prefix`
  over the entries index, and `odc export --format jsonl|csv` with a JSONL
  round-trip back into a fresh repository.
* `odc patch create|apply|verify` implements the frozen `ODP1` transfer
  format: patches carry only the target blocks that the base lacks plus the
  index delta, verify their checksum and Merkle roots, and apply atomically
  on an exact base. A descendant HEAD is merged three-way (M4); `docs/patch.md` documents usage, error codes and limits.
* `odc status` reports committed entry and block counts.
* `crates/odc-testkit` generates synthetic MDX 1.2 / 2.0 fixtures for tests
  and the e2e script; `examples/lookup_bench.rs` reports p50/p95 lookup
  latency.
* Spec section 16 documents the MDX mapping; `docs/mdx-compat.md` records the
  parser comparison and the measured import figures.
* `crates/odc-ffi` exposes the core through a C ABI (`staticlib` and
  `cdylib`) with `include/odc.h` and an `ODCFFI` module map;
  `scripts/build-odc-xcframework.sh` assembles macOS, iOS device and iOS
  simulator slices into `ODCFFI.xcframework`.

* `odc pack [-o FILE] [--shallow] [--force]`: exports a repository as one
  portable SQLite `.odc` file. `--shallow` writes only HEAD and records
  `meta.shallow`; `log` marks the boundary, `verify` and `gc` treat missing
  parents as expected. `scripts/bench-shallow.sh` reports the size baseline.

* CI: the `apple` job builds every XCFramework slice and runs the
  `ODictKit` Swift tests on macOS.

* `odc merge A B [--strategy auto|ours|theirs|union] [--json]`: three-way
  merge over the `parent` / `parent2` DAG with a deterministic merge base,
  fast-forward, word-level and field-level automatic merges.
* `odc resolve`, `odc merge --continue` and `odc conflicts` for the conflict
  workflow: conflicts live in the frozen `conflicts` table, block the merge
  commit, and make the command exit with code 5.
* Applying a patch whose base is an ancestor of HEAD now merges the target
  three-way instead of failing with `ODC-E025`.
* `docs/merge.md` documents the semantics, the conflict JSON schema, exit
  codes and the union `~N` suffix rule.

### Changed

* `Error` gained `Mdx` (`ODC-E021`) and `EntryTooLarge` (`ODC-E022`).
* `odc import --format mdx` keeps its previous stdout lines and adds the
  corrupt-block summary after them; the engine banner still precedes the
  import, and per-block diagnostics go to stderr.
* `Error` also gained `MergeInProgress` (`ODC-E028`), `UnresolvedConflicts`
  (`ODC-E029`) and `NoMergeInProgress` (`ODC-E030`). `PatchNeedsMerge`
  (`ODC-E025`) stays reserved but is no longer produced.
* `patch::apply` returns `ApplyOutcome` (`Applied` or `Merged`) and no longer
  rejects a descendant HEAD.
* CSV import accepts an optional `raw_html` column.
* The working set no longer caches the uncompressed payload of committed
  entries: `staging.payload` is nullable, `commit` clears it after rebuilding
  the index, and reads materialise the body from the block when needed
  (verified against `content_hash`). Databases written with the old schema are
  migrated on open. A commit that frees at least 64 MiB of cached payloads also
  vacuums the database so the file shrinks back.

## [0.1.0] - 2026-10-31

### Added

* Normative format specification (`docs/odc-spec.md`) for ODC v0.1: blocks
  (`ODB1`), canonical CBOR payloads, content-defined chunking, BLAKE3 Merkle
  rules, SQLite container layout and error codes.
* `odc-core` reference implementation: varint, canonical CBOR subset, block
  encoding, gear-hash chunking, hashing, SQLite repository with
  init / import / commit / log / show / diff / status / verify.
* `odc` CLI (`crates/odc-cli`) covering the same operations, with `--json`
  output for log, show and diff.
* Machine-readable test vectors under `spec/testvectors/`, including
  hand-computed varint and block encodings.
* End-to-end smoke test (`scripts/e2e.sh`) over 10,000 synthetic entries.

### Changed

* Relicensed from `MIT OR Apache-2.0` to `GPL-3.0-or-later`.
* CI now runs fmt, clippy (`-D warnings`), debug and release tests, rustdoc
  (`-D warnings`) and `cargo deny` on Linux and macOS.

### Security

* All decoders return typed `ODC-E0xx` errors for malformed input; negative
  tests cover truncated blocks, bad magic, offset-index mismatches, duplicate
  keys and corrupted repository indexes.
