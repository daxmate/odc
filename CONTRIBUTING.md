# Contributing to ODC

Thanks for helping out. ODC is a specification plus a reference implementation;
changes that affect the wire format are reviewed more strictly than
implementation-only changes.

## Prerequisites

* Rust stable (the repository pins no toolchain; `rust-toolchain.toml` requests
  `stable` with `rustfmt` and `clippy`).
* SQLite is vendored through `rusqlite`'s `bundled` feature, so no system
  SQLite is required.
* For `cargo deny`: `cargo install cargo-deny --locked`.

## Build and test

```bash
cargo build --all
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all                 # debug
cargo test --all --release       # release (CI runs this on Linux)
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
cargo deny check
```

Every command above runs in CI (see `.github/workflows/ci.yml`). A pull request
is not ready for review until the same commands pass locally.

## End-to-end smoke test

```bash
bash scripts/e2e.sh
```

The script builds `odc`, imports 10,000 synthetic entries, commits, diffs,
shows an entry, and verifies the repository. It writes only into a temporary
directory and is safe to run repeatedly.

## Commit conventions

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
feat(cli): add `odc show --json`
fix(core): reject non-minimal varints
test: cover truncated block headers
docs: explain the CDC boundary rule
```

Keep commits focused. A commit that changes the wire format must also update
`docs/odc-spec.md`, the affected test vectors, and `CHANGELOG.md`.

## Project layout

| Path | Contents |
| --- | --- |
| `docs/odc-spec.md` | Normative specification (the only source of truth) |
| `spec/testvectors/` | Machine-readable vectors used by `crates/odc-core/tests/vectors.rs` |
| `crates/odc-core/` | Chunking, encoding, hashing, SQLite repository |
| `crates/odc-cli/` | The `odc` command line tool |
| `scripts/e2e.sh` | End-to-end smoke test |

## Adding a test vector

Vectors exist so that independent implementations can be checked against the
specification. They must be **computed from the specification**, never copied
out of the Rust encoder (that would only test the implementation against
itself).

1. Pick the vector family under `spec/testvectors/` (`varint_encoding`,
   `block_encoding`, `payload_encoding`, `commit_hash`, `cdc_boundaries`, ...).
2. Compute the expected bytes by hand or with a small independent script that
   follows `docs/odc-spec.md` literally. Record the derivation in the vector's
   `note` field.
3. Add the entry and a matching assertion in
   `crates/odc-core/tests/vectors.rs` (or `vectors_extra.rs` for the
   hand-computed families).
4. Run `cargo test --all`; a failing vector means either the vector or the
   specification is wrong — resolve that before touching the encoder.

## Testing expectations

* New public behaviour needs a unit or integration test.
* Codec changes need property tests (`tests/properties.rs`).
* Error paths need negative tests (`tests/negative.rs`): malformed input must
  return an `Error` with a stable code, never panic.
* CLI changes need `assert_cmd` integration tests (`crates/odc-cli/tests`).

## License

By contributing you agree that your contributions are licensed under
GPL-3.0-or-later, as described in `LICENSE`.
