# Changelog

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
