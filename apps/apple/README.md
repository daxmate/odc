# ODict for Apple platforms

SwiftUI front ends for ODC: **ODict** (iOS) and **ODict Desktop** (macOS), both
built on the shared `ODictKit` Swift package.

The package has two layers: `ODictCore` is the UI-free Swift binding over the
`ODCFFI` XCFramework produced from `crates/odc-ffi`, and `ODictKit` is the
application layer on top of it (stores, editing, publishing and SwiftUI views).
Apps that only need to open, search and read entries can depend on `ODictCore`
alone; `ODictKit` re-exports it, so `import ODictKit` still sees the whole API.

The readers open a dictionary, search it (prefix, then substring) and render
entries. **ODict Desktop** also has an editing mode for repository
directories: paged browsing, structured field editing, field diffs, commit and
discard, plus CSV and Excel import into the working set (`.xlsx` files are
read from their first non-empty worksheet only). The editor also checks and
repairs the selected entry's HTML on demand (「检查 HTML」/「修复 HTML」), using
the same `process_html` core as `odc check-html`; a repair is one undoable
form edit and is not written to the working set until you save. Single-file
`.odc` packs stay read-only, and the iOS app has no editing entry point.

## Prerequisites

* Xcode 27 (Swift 6, iOS 17 / macOS 14 deployment targets)
* `xcodegen` (`brew install xcodegen`)
* `rustup` with the five Apple targets that
  `scripts/build-odc-xcframework.sh` lists (the Homebrew `rust` formula has no
  iOS standard library)

## Build

```bash
# 1. Rust C ABI -> apps/apple/ODictKit/Frameworks/ODCFFI.xcframework (gitignored)
bash scripts/build-odc-xcframework.sh

# 2. Generate the Xcode project from project.yml
cd apps/apple && xcodegen generate

# 3. Build both targets
xcodebuild -project apps/apple/ODict.xcodeproj -scheme ODict \
  -sdk iphonesimulator -destination 'generic/platform=iOS Simulator' \
  CODE_SIGNING_ALLOWED=NO build
xcodebuild -project apps/apple/ODict.xcodeproj -scheme ODictDesktop \
  -destination 'platform=macOS' CODE_SIGNING_ALLOWED=NO build
```

`./scripts/build-app.sh` runs those four steps in one go (root `./build-app.sh` is a compatible alias). Steps 1 and 2 must
be re-run after changing the Rust side; `ODict.xcodeproj` is generated and is
not meant to be edited by hand.

To build and install the macOS app in one go, pass `--install`:

```bash
./scripts/build-app.sh --install           # build both targets, install macOS only
./scripts/build-app.sh macos --install     # build macOS, then install it
```

`--install` copies the built bundle over the previous one and launches it. The
previous bundle is moved to `/tmp/ODict Desktop-old-<timestamp>.app` instead of
being deleted, so it can be restored. Supported environment variables:

- `ODC_INSTALL_DIR` — install destination (default `/Applications`).
- `ODC_SKIP_LAUNCH=1` — install without launching the app.

Installation is macOS only: `ios --install` exits with status 2 because the iOS
app is not available yet.

Package tests (macOS slice, no simulator needed):

```bash
swift test --package-path apps/apple/ODictKit
```

## Layout

```
apps/apple/
  project.yml                  xcodegen source of truth
  ODictKit/                    Swift package: models, FFI wrapper, store, views
    Frameworks/ODCFFI.xcframework   generated, gitignored
  ODict/                       iOS app target (entry point + Info.plist)
  ODictDesktop/                macOS app target (entry point, Info.plist, entitlements)
  Shared/Assets.xcassets       AppIcon (iOS) and AppIconDesktop (macOS)
```

## How opening works

1. The picker returns a security-scoped URL (a `.odc` file, a `.mdx` file, or
   a repository directory produced by the CLI).
2. `DocumentLibrary` copies it into
   `Application Support/ODict/<uuid>.odc` before opening, so the app never
   reopens the user's file inside the sandbox. A picked single file is copied
   into that directory as `odc.db`, which is the layout `odc_repo_open`
   expects.
3. `OdcDictionary` (main-actor confined, because the C handle is not thread
   safe) opens the copy; `DictionaryStore` drives the UI.

An MDX conversion is written to `Application Support/ODict/Imports`, never next
to the source `.mdx`; once the library holds its own copy the staged `.odc` is
deleted. `library.json` (in the same folder) is the index; nothing outside that
folder is needed to reopen a dictionary.

### Startup migration

`DictionaryStore.performRestore` asks `LibraryStore.migrateExternalCopies` to
copy any entry whose `storedName` still resolves outside the library folder
(an absolute path from an older build, or a name with `..`) into
`<uuid>.odc` inside it, opens the copy to prove it works, and only then
rewrites the index. The move is idempotent: an entry already inside is skipped,
and a name that is taken gets a fresh identifier instead of being overwritten.

A copy that cannot be read — deleted, an iCloud placeholder that was never
downloaded, or damaged — leaves its entry untouched and shows up in the library
sheet ("未能迁入词典库") with the reason; the failure is logged under
`com.daxmate.odict` `library`. The original file is only read, never moved or
deleted.

## Signed imports and incremental updates (0.10.0)

**Imports.** When a picked `.odc` has a `.odc.sig` next to it, the copy is
verified before anything is registered:

* valid signature + trusted key → the import continues and the row shows
  "已签名 · 发布者名称";
* valid signature + empty trust store → one "首次信任发布者" alert (signer,
  fingerprint, public key); "信任并继续" adds the key with source `manual` and
  resumes the import, "取消" deletes the copy and imports nothing;
* valid signature + unknown key in a non-empty store → refused ("未知发布者");
  the row's "信任管理…" opens the repository's trust sheet;
* invalid signature or missing key → refused with the `ODC-E0xx` code;
* no `.sig` → the import is allowed and the row is marked "未签名", with a
  one-time explanation.

**Updates.** "应用更新…" on a library row picks a `.odp`, verifies its
`.odp.sig`, applies it through `odc_patch_apply_signed` and reports the
applied / merged / conflict counts. Conflicts stay in the repository for the
existing conflict workflow and are never resolved automatically. The open
editor and entry list refresh after a successful apply. A patch without a
sidecar is refused: the FFI only exposes the signed apply entry point.

Version 0.14.0 (build 16).

## Known limitations (v0)

* **iOS stays read-only.** Editing and MDX import are macOS-only, and sync is
  out of scope.
* **Session restore is best effort.** The last dictionary, entry and search
  term are kept in `session.json`; an entry or dictionary that was removed is
  skipped. The macOS sidebar width is restored through the AppKit split view
  behind `NavigationSplitView`, so it depends on AppKit still providing one.
* **Prefix and substring search, no fuzzy matching.** The core exposes
  `search_prefix` and `search_substring`; a query with no prefix hits falls
  back to case-insensitive substring matches over normalised keys and display
  forms (ASCII folding for display). There is no fuzzy or edit-distance
  matching yet.
* **Copies are sandbox-local.** Closing a dictionary deletes its copy; reopen
  from the picker to use it again. The copy is not indexed for re-opening from
  Recents yet.
* **Single-file containers are a design intent.** `docs/design-rationale.md`
  describes a `.odc` file as a bare SQLite database, but the current core opens
  a repository *directory*. The app accepts both: a file is wrapped into a
  directory as `odc.db`; a directory is copied as-is.
* **No App Store path decided.** ODC is GPL-3.0-or-later. Shipping a GPL app on
  the App Store requires a licensing decision for the Rust core and the FFI
  binary; sort that out before distribution.
* macOS builds are unsigned (`CODE_SIGNING_ALLOWED=NO`); the desktop target
  ships with an app-sandbox entitlement and
  `files.user-selected.read-write` (read-write so an imported MDX can be
  converted next to its source; the picker still gates every path).

## Importing MDX (macOS)

ODict Desktop imports a `.mdx` dictionary as an **editable repository
directory**, so the CLI is no longer required for the first step and the
editor works on the imported copy right away.

1. `Open Dictionary or MDX…` (⌘O) accepts `.odc` files, repository
   directories and `.mdx` files. The iOS target keeps rejecting `.mdx`.
2. `DictionaryStore.importMDX(url:)` runs
   `odc_import_mdx_to_repo_with_mdd_progress` on a background thread and
   publishes the stage (准备 / 导入中 / 完成). The core imports and commits in a
   single call, so the UI never invents a percentage.
3. The conversion lands in the library's `Imports` folder as `<name>.odc/`,
   a directory holding `odc.db`. A directory that already holds files is
   refused; the app picks `-1`, `-2`, … instead of overwriting one.
4. Corrupt record blocks are skipped (the core imports a repaired copy) and
   the completion alert reports `已导入 N 条，跳过 M 个损坏块（K 条）` plus
   any scan warning.
5. The copy is registered as a directory library item (`sourceIsDirectory`)
   and opens automatically, so the editor is available immediately.

Packs remain the export format (`odc pack`). A pack imported earlier keeps its
read-only behaviour; the library's **转为可编辑副本** button copies it into a
new directory item and leaves the pack untouched.

The same calls are available without the GUI:

```c
int32_t odc_import_mdx_to_repo_with_mdd_progress(
    const char *mdx_path, const char *repo_dir, const char *mdd_paths_json,
    odc_import_progress_fn callback, void *context, char **report_json_out,
    char **error_message);
int32_t odc_copy_pack_to_repo(
    const char *pack_path, const char *repo_dir, char **report_json_out,
    char **error_message);
```

`cargo test -p odc-ffi --test ffi import_mdx` covers the clean, corrupt,
existing-target and missing-source paths; `crates/odc-core/tests/mdx_repo_import.rs`
covers the editable import and the pack copy. The `imports_a_real_mdx_file` test
is opt-in through `ODC_MDX_SMOKE=<path to .mdx>` and `-- --ignored`.

## 发布（macOS）

Publishing is desktop-only: the iOS reader never signs anything.

1. **生成或导入发布者身份.** Open *Settings* (Cmd+,) or the editor toolbar
   -> 发布… -> 打开发布者设置. 生成新密钥 creates an Ed25519 keypair through
   ODCFFI; 导入私钥 accepts a PKCS#8 `.pem` / `.key` file. The private key
   only ever lives in the keychain (service `com.daxmate.odict.publisher`), and
   the UI shows just the name, fingerprint and public key. 移除密钥 asks for
   confirmation and leaves published artifacts untouched.
2. **发布…** in the editor toolbar (a writable directory repository is
   required). Fill in the version label (defaults to today, `yyyy.MM.dd`),
   optional release notes and the output directory. The wizard calls
   `odc_publish`; existing files are never overwritten, so a collision keeps the
   sheet open with the ODC-E0xx error.
3. **增量包.** After a successful publish the app remembers
   `lastPublishedHead`, the label and the time per library in UserDefaults.
   The next time 同时生成增量包 becomes available and uses that head as
   `--base`, so readers can update with the patch instead of the full pack.
4. **信任管理.** 信任… lists the repository's publishers (name, fingerprint,
   source manual/tofu, added time) and lets you add a 64-hex public key by hand
   or remove one after confirmation. The first signed import asks for trust
   (TOFU); the list is empty until then.

Limits:

* There is no verify-release screen in the app yet; use
  `odc verify-release` from the CLI for now. Signed imports and "应用更新…"
  cover the reader-facing flows.
* The publish sheet always signs with the stored keychain identity; signer
  names are not prompted, so manifests carry no signer string.
* Keychain items are per-app and per-user; moving to another Mac means
  exporting the key and importing it again.
* Removing a key does not rewrite history: existing releases stay valid for
  anyone who already trusts that key.

