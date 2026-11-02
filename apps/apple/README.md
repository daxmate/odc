# ODict for Apple platforms

SwiftUI front ends for ODC: **ODict** (iOS) and **ODict Desktop** (macOS), both
built on the shared `ODictKit` Swift package, which wraps the `ODCFFI`
XCFramework produced from `crates/odc-ffi`.

The readers open a dictionary, search it (prefix, then substring) and render
entries. **ODict Desktop** also has an editing mode for repository
directories: paged browsing, structured field editing, field diffs, commit and
discard, plus CSV import into the working set. Single-file `.odc` packs stay
read-only, and the iOS app has no editing entry point.

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

ODict Desktop converts a `.mdx` dictionary into a standalone `.odc` file, so
the CLI is no longer required for the first step.

1. `Open Dictionary or MDX…` (⌘O) accepts `.odc` files, repository
   directories and `.mdx` files. The iOS target keeps rejecting `.mdx`.
2. `DictionaryStore.importMDX(url:)` runs `odc_import_mdx_to_odc` on a
   background thread and publishes the stage (准备 / 导入中 / 完成). The core
   converts and packs in a single call, so the UI never invents a percentage.
3. The output lands next to the source as `<name>.odc`; when that directory
   is not writable it falls back to `~/Documents`. An existing file is never
   replaced — the app uses `-1`, `-2`, … instead.
4. Corrupt record blocks are skipped (the core imports a repaired copy) and
   the completion alert reports `已导入 N 条，跳过 M 个损坏块（K 条）` plus
   any scan warning.
5. The converted document opens automatically through the normal path.

The same call is available without the GUI:

```rust
let report = odc_import_mdx_to_odc(mdx_path, out_odc_path, &mut report_json, &mut error);
```

`cargo test -p odc-ffi --test ffi import_mdx` covers the clean, corrupt,
existing-target and missing-source paths; the `imports_a_real_mdx_file` test
is opt-in through `ODC_MDX_SMOKE=<path to .mdx>` and `-- --ignored`.
