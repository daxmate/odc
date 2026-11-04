# Changelog

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

_暂无。_

## [0.12.0] - 2026-11-04

### Added
- macOS editor: create and delete entries, with session-scoped undo/redo (100 steps, 750 ms coalescing).
- XLSX import through the existing CSV mapping/preview pipeline (CoreXLSX 0.14.2, first non-empty worksheet).
- Per-entry HTML check and one-click repair in the editor, backed by a new `odc_html_process_json` C ABI; repairs are one undoable edit.

### Fixed
- Editor key validation now queries the repository (not only loaded pages), refuses unknown lookup failures, and skips placeholder keys that already exist.

## [0.11.0] - 2026-11-03

### Added

* CLI: `odc fetch --from <repo> --depth N` deepens a shallow export from another
  ODC repository without touching the network. Missing commits, their index
  snapshots and the source block store are copied inside one transaction;
  reaching genesis clears `meta.shallow` (see `docs/shallow.md`).

### Changed

* Apple: the Swift package is split into a UI-free `ODictCore` library (the
  `ODCFFI` wrappers, resource helpers, models and entry HTML) and `ODictKit`,
  the application layer built on top of it. `ODictKit` re-exports the core, so
  `import ODictKit` keeps the previous API for app code.

## [0.10.0] - 2026-11-03

### Added

* Core: `signing` module with Ed25519 detached signatures (sidecar format v1).
  A signature covers a 43-byte message (domain prefix `ODC-SIG-V1` + NUL +
  BLAKE3-256 of the target) and verification always recomputes the digest and
  size, so a sidecar cannot be replayed onto different bytes. New error codes
  `ODC-E032` .. `ODC-E035`.
* CLI: `odc keygen`, `odc sign` and `odc verify-signature`; `verify-signature`
  exits 1 for an invalid signature and 2 for usage / I/O errors, and honours
  `--pub` / `--pubkey` key pinning.
* Docs: `docs/signing.md`; spec section 17 freezes the sidecar layout, the
  domain-separated message and the error codes.
* Tests: fixed Ed25519 vector in `spec/testvectors/signature_ed25519.json`
  (cross-checked with OpenSSL 3 `pkeyutl`), plus core and CLI integration
  coverage.
* Core: `publishers` trust store (schema version 3; version 1 and 2
  databases gain the table on open) with manual and TOFU sources, plus the
  `release` module that writes and verifies `release.json` manifests.
* CLI: `odc publish` writes a pack, an optional base..HEAD patch, a manifest
  and a signature for every file; existing files are refused without
  `--force`.
* CLI: `odc verify-release` re-checks the manifest signature and recomputes
  every artifact size, BLAKE3 digest and inline signature; `--pub` /
  `--pubkey` pin the publisher, otherwise the trust store decides.
* CLI: `odc trust add|list|remove` manages the publisher trust store;
  `odc patch apply --sig` verifies and enforces trust (`--trust-new`)
  before touching HEAD or the working set. New error codes `ODC-E036` ..
  `ODC-E038`.
* FFI: `odc_keypair_generate`, `odc_sign_file`,
  `odc_verify_signature_file`, `odc_publishers_json`, `odc_trust_add`,
  `odc_trust_remove`, `odc_publish`, `odc_verify_release` and
  `odc_patch_apply_signed`, with matching declarations in `odc.h`.
* Docs: `docs/release.md` describes the release layout, manifest v1, TOFU
  rules and limits; the spec gains section 17.6 and the new error codes.
* Tests: core, CLI and FFI coverage for publish / verify-release / trust and
  for signed patch application, plus an end-to-end publish smoke in
* Apple: publish wizard (macOS). Publisher identities are Ed25519 keypairs
  kept in the keychain; the wizard calls `odc_publish` with an optional
  base..HEAD patch, remembers the last published head per library and shows a
  trust sheet (list / add / remove) for the current repository. The desktop
  app gains a Settings scene for the identity; the iOS reader stays read-only.
* Apple: signature-gated imports. A `.odc` pack with a `.sig` sidecar is
  verified before it is registered; a first-use key asks once, an unknown key
  is refused, and unsigned packs are marked "未签名" in the library.
* Apple: "应用更新…" on library rows verifies and applies a signed `.odp`
  patch through `odc_patch_apply_signed`, reports applied / merged / conflict
  counts and refreshes the open editor. Unsigned patches are refused because
  the FFI only exposes the signed entry point.
* Apple: FFI wrappers `verifySignatureFile`, `applySignedPatch` and `signFile`,
  with decoding for signature reports and patch reports.
* Apple app bundles: version 0.10.0 (build 12).

## [0.9.0] - 2026-11-03

### Added

* Core: MDX imports repair each entry body as an HTML5 fragment, remove
  `script` / `iframe` / `object` / `embed`, event handler attributes and
  `javascript:` URLs, normalise `<style>` blocks and `style` attributes with
  `lightningcss`, and format the result with `markup_fmt` in strict
  whitespace mode. Diagnostics are aggregated per code in
  `MdxImportReport::html`; a body whose formatting fails keeps its original
  text and is counted as failed.
* CLI: `odc check-html [PATH]` reports HTML and CSS problems without writing,
  either for one `.mdx` file or for every entry of a repository (`--json`,
  `--limit N`).
* CLI: `odc normalize` rewrites `raw_html` in the working set; dry run by
  default, `--apply` commits the result.
* CLI: `odc import --html <format|check|off>` selects the HTML mode.

### Changed

* MDX imports (CLI and FFI) now normalise HTML by default; `--html off`
  restores the previous byte-for-byte behaviour.
* Apple app bundles: version 0.9.0 (build 11).

## [0.8.0] - 2026-11-03

### Added

* Core: `import_mdx_to_repo` imports an MDX file straight into a directory
  repository and commits the result, so `HEAD` always names the imported
  entries. A non-empty destination is refused and a directory created by a
  failed import is removed again.
* Core: `copy_pack_to_repo` turns a single-file pack into an editable
  directory repository without touching the original file.
* FFI: `odc_import_mdx_to_repo_with_mdd_progress` and `odc_copy_pack_to_repo`
  expose both paths to the Apple apps; the pack import entry points stay
  unchanged for exports.
* Apple: importing an MDX dictionary now registers an editable directory
  library item, so the editor is available right after the import.
* Apple: the library offers "转为可编辑副本" for read-only packs; the pack is
  kept and the copy becomes a new editable item.

### Changed

* Apple: the MDX importer writes a repository directory instead of a
  standalone `.odc`; packs remain the export format.

## [0.7.0] - 2026-11-02

### Added

* macOS: the main window restores its size and position between launches, and
  the sidebar width is carried over through the underlying split view.
* The reader remembers the dictionary, entry and search term in
  `session.json` (next to `library.json`) and restores them after the library
  loads; a dictionary or entry that no longer exists is skipped silently.
* Word book: sort by recency or alphabetically, filter by all / bookmarks /
  history and by dictionary. Both choices are persisted.
* Search field: a "recently viewed" list of up to eight entries appears above
  the recent searches while the field is empty.

## [0.6.0] - 2026-11-02

### Added

* macOS: dictionary editor with per-entry forms, working-set status and diff,
  commit/discard, and a CSV import sheet with column mapping.
* Core: editing is guarded by the pack/workset marker, so a dictionary copied
  from a single-file pack can be edited without losing its committed index.

### Fixed

* Core: a packed library copy no longer reports every committed entry as
  deleted; committing an empty working set over a populated HEAD is refused
  instead of writing an empty index and erasing the dictionary.

### Added

* Apple: the sidebar shows recent searches when the query is empty; up to ten
  entries are persisted locally and tapping one restores the query.

## [0.5.1] - 2026-11-02

### Fixed

* Entry detail renders as a single HTML document in a single web view, so the
  page scrolls natively again instead of a pinned inner element. Dictionaries
  that share the page are separated with Shadow DOM, which keeps each
  dictionary's stylesheet from leaking into its neighbours. Regression tests
  now guard the single-document shape, per-section style isolation, body/html
  mapping and scrolling.

## [0.5.0] - 2026-11-01

### Changed

* Core: commit indexes are stored as deltas against the first parent, with a
  full snapshot every 32 commits, so a history no longer costs a full index per
  commit. Twelve commits of three hundred entries take 11,551 compressed bytes
  instead of 126,496. Existing repositories keep reading unchanged: entries
  written before the frame are read as full snapshots, so no migration or
  re-import is needed. `gc` rewrites the oldest retained entry as a full
  anchor before dropping its parents, and `pack --shallow` materialises HEAD's
  index because a single-commit file has no parent to replay.

## [0.4.0] - 2026-11-01

### Added

* Core, FFI (`odc_repo_search_substring_json`) and Apple: substring search over
  normalised keys and display spellings, used when a query has no prefix hits.

## [0.3.0] - 2026-11-01

### Added

* Wordbook: bookmarks and recent lookups moved out of the library sheet into
  their own sheet, reachable from a toolbar bookmark button on both platforms
  (⌘B on macOS), so kept words are one click away from the main screen.
* Reader memory: bookmarks and a 200-entry query history live in `reader.json`
  next to `library.json`, written atomically and reset to an empty state when
  the file cannot be decoded. The library sheet lists dictionaries, the wordbook
  sheet lists recent entries and bookmarks, and the entry toolbar toggles a
  bookmark for the entry on screen.
* Library: opening a second dictionary no longer replaces the first. Up to
  eight stay open at once, the library sheet lists every entry with its open
  state and highlights the active one, and closing the active dictionary falls
  back to another open one instead of the empty state.
* Library: the set of open dictionaries is remembered in `library.json` and
  reopened on the next launch, in open order and with the last active one in
  front. Copies that no longer open are skipped and cleaned up instead of
  blocking the launch; `library.json` files written before this release still
  load and fall back to the single last-opened dictionary.
* Wordbook: the toolbar button shows how many entries are bookmarked (hidden
  while the wordbook is empty), on both platforms.
* Search: one query now runs against every open dictionary (at most 20 hits
  each) and the sidebar groups the results under each dictionary's name.
  Picking a hit switches dictionaries first, so history and bookmarks are
  recorded against the dictionary the entry came from.

### Changed

* Sidebar: the dictionary group is gone. The sidebar is the search field plus
  the hits grouped by dictionary; opening, switching and closing dictionaries
  stays in the library sheet, whose rows already carry open/switch/current and
  close.
* Entry reader: every open dictionary's entry for the selected word now shares
  one scrolling page. Sections are separated by a rule and titled with the
  dictionary name; a word only one open dictionary carries keeps the plain
  look, without title or rule.

## [0.2.0] - 2026-11-01

### Added

* Entry reader: the dictionary's external stylesheets are read out of the
  repository and inlined ahead of the entry, after the reader defaults, so a
  dictionary's own typography wins at equal specificity. Missing, unreadable or
  oversized sheets (2 MiB budget per entry) degrade to the defaults instead of
  a blank panel.
* Entry reader: MDict "@@@LINK=" redirect entries are followed by the core (up
  to eight hops, cycles and missing targets reported), and the search field
  follows with the entry the chain reached.
* Search field: pressing return opens the first result (onSubmit), on both
  macOS and iOS; an empty result list is a no-op.
* MDX import copies stylesheets that entries link to but the MDD volumes do not
  carry, reading them from beside the .mdx file (LDOCE6++ ships ldoce6ec.css
  that way). MdxImportReport.stylesheets counts them.
* App storage: a conversion is written to `Application Support/ODict/Imports`
  instead of the folder holding the source `.mdx`, and the staged `.odc` is
  deleted once the library holds its own copy.
* App storage: on launch, entries in `library.json` whose copy still lives
  outside the library folder are copied in, opened to prove they work, and the
  index is rewritten. A copy that cannot be read keeps its entry and is listed
  in the library sheet with the reason; source files are never deleted.

### Changed

* Import banner: the resources stage counts the MDD payloads it is copying
  ("导入资源 12,345 / 26,780 个" plus the volume when there are several) instead
  of repeating the finished entry count, so a long media copy is visible.
* Import banner: the packing stage that follows has no samples of its own, so
  it shows an indeterminate spinner with the elapsed seconds rather than a
  100% bar the core never measured. The detail names what is being written and
  asks the reader not to quit.
* Import summary: the note under the entry clears itself after six seconds;
  the button still dismisses it at once, and failures stay until dismissed.
* macOS search: Return in the sidebar field opens the first hit. SwiftUI does
  not deliver that key to `onSubmit(of: .search)`, so the list watches the key
  locally and swallows it only while the sidebar search field has focus.
* `MDXImporter.outputURL` no longer prefers the source folder even when it is
  writable; conversions go into the app's storage unless the caller names a
  directory.
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

* MDD resources: importing an MDX file now also imports the sibling MDD
  volumes (`<name>.mdd`, `<name>.1.mdd` … main volume first, then ascending
  numbers) into a new `resources` table, so images, audio and fonts live inside
  the `.odc` file. A broken volume is a warning, never a failed import; the
  import report carries `mdd_files`, `resource_count`, `resource_bytes` and
  `warnings`. The C ABI gains `odc_repo_resource_count`,
  `odc_repo_resource_len`, `odc_repo_resource_copy` and
  `odc_import_mdx_to_odc_with_mdd`.
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

* Import progress for MDX and MDD: `MdxImportOptions::progress` now yields
  `ImportProgress { stage, done, total, volume_index, volume_count }` — the MDD
  volumes are pre-scanned before the first record, so a caller knows the whole
  job up front. Samples are capped at roughly ten per second, with every stage
  change and the final `done == total` always delivered. The C ABI gains
  `odc_import_mdx_to_odc_with_mdd_progress` (NULL callback = the old entry
  point), `odc import --progress` prints stage and percentage per line, and the
  Apple banner shows one determinate bar with the entry and resource counts.
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
