# odc-ffi

C ABI for [`odc-core`](../odc-core), packaged as a static library for Apple
platforms (and any other C host). The Apple client links `ODCFFI.xcframework`,
which is produced by `scripts/build-odc-xcframework.sh`.

## Layout

- `src/lib.rs` — the FFI surface
- `include/odc.h` — public header (the contract)
- `include/module.modulemap` — Swift module `ODCFFI`
- `tests/ffi.rs` — calls the exported functions the way a C caller would

## API

```c
typedef struct OdcRepo OdcRepo;

OdcRepo *odc_repo_open(const char *path_utf8, char **error_message);
void odc_repo_close(OdcRepo *repo);
int64_t odc_repo_entry_count(const OdcRepo *repo, char **error_message);
char *odc_repo_search_json(const OdcRepo *repo, const char *query_utf8,
                           int32_t limit, char **error_message);
char *odc_repo_entry_json(const OdcRepo *repo, const char *key_utf8,
                          char **error_message);
void odc_string_free(char *s);
const char *odc_version(void);
```

### Error convention

Fallible calls take `char **error_message`. On success it is set to `NULL`;
on failure it receives a heap-allocated UTF-8 message that the caller releases
with `odc_string_free`. Passing `NULL` for `error_message` discards the
diagnostic. Panics are caught at the boundary and surface as error strings; no
panic ever unwinds into C.

### Memory ownership

Strings returned by `odc_repo_search_json`, `odc_repo_entry_json` and
`error_message` are owned by the caller. `odc_version` returns static
storage and must not be freed. `odc_string_free(NULL)` and
`odc_repo_close(NULL)` are no-ops.

### Threading

An `OdcRepo` handle may be used by one thread at a time. The crate does not
add `Send`/`Sync`; callers that need cross-thread access must serialise
calls themselves.

### Search semantics

`odc_repo_search_json` performs a prefix match on normalised keys (NFKC +
case folding) and reuses `odc_core::Repo::search_prefix`, which queries the
`entries` index in key order. It is **not** a linear scan, but it is still a
prefix index: there is no full-text or fuzzy matching. `limit` must be
between 1 and 200 inclusive.

Result shape:

```json
[{"key": "apple", "display": "Apple"}, {"key": "application", "display": "application"}]
```

### Entry JSON

```json
{
  "key": "apple",
  "display": "Apple",
  "phonetic": ["..."],
  "defs": [{"pos": "noun", "sense": "a fruit", "examples": ["..."], "labels": []}],
  "raw_html": null,
  "extra": {}
}
```

`extra` holds unknown payload fields preserved by the core. CBOR maps with
text keys become JSON objects, other maps become arrays of `[key, value]`
pairs, and byte strings become arrays of integers (JSON has no byte type).
Missing keys return `NULL` with an `entry not found: <key>` message.

## Building

Host builds and tests use the regular workspace toolchain:

```bash
cargo test -p odc-ffi
```

The XCFramework requires `rustup` (the Homebrew `rust` formula ships no iOS
standard library) plus the five targets:

```bash
brew install rustup
export PATH="$HOME/.cargo/bin:$PATH"
export RUSTUP_DIST_SERVER=https://rsproxy.cn
rustup default stable
rustup target add aarch64-apple-darwin x86_64-apple-darwin \
                  aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
```

Then:

```bash
bash scripts/build-odc-xcframework.sh                 # check targets, build
bash scripts/build-odc-xcframework.sh --install-targets  # add missing targets first
```

Slices are built for a fixed minimum OS version so the C objects (SQLite,
zstd) and the Rust objects agree: `IPHONEOS_DEPLOYMENT_TARGET` defaults to
`15.0` and `MACOSX_DEPLOYMENT_TARGET` to `11.0`. Override either env var if
the app targets something newer; the framework's minimum OS version follows
those values.

Output: `apps/apple/ODictKit/Frameworks/ODCFFI.xcframework` (gitignored).
Intermediate artifacts live in `apps/apple/build/` (also gitignored) and can
be deleted at any time; the script rebuilds from scratch and is safe to
re-run.
