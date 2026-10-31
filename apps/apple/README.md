# ODict for Apple platforms

SwiftUI front ends for ODC: **ODict** (iOS) and **ODict Desktop** (macOS), both
built on the shared `ODictKit` Swift package, which wraps the `ODCFFI`
XCFramework produced from `crates/odc-ffi`.

This is the v0 slice: open a dictionary, prefix-search it, render entries.
It is read-only — no editing, importing, word books or sync.

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

`scripts/build-apple.sh` runs those four steps in one go. Steps 1 and 2 must
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

1. The picker returns a security-scoped URL (a `.odc` file, or a repository
   directory produced by the CLI).
2. `DocumentLibrary` copies it into
   `Application Support/ODict/<uuid>.odc` before opening, so the app never
   reopens the user's file inside the sandbox. A picked single file is copied
   into that directory as `odc.db`, which is the layout `odc_repo_open`
   expects.
3. `OdcDictionary` (main-actor confined, because the C handle is not thread
   safe) opens the copy; `DictionaryStore` drives the UI.

## Known limitations (v0)

* **Read-only.** Editing, importing, word books and sync are out of scope.
* **Prefix search only.** The core exposes `search_prefix`; there is no fuzzy
  or full-text matching yet.
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
  `files.user-selected.read-only`.
