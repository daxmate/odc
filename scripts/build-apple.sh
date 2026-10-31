#!/usr/bin/env bash
# Build the Apple apps end to end: FFI -> Xcode project -> iOS + macOS builds.
# Safe to re-run; all intermediates live under apps/apple/build/ (gitignored).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

export PATH="$HOME/.cargo/bin:$PATH"

echo "== ODCFFI.xcframework"
bash scripts/build-odc-xcframework.sh

echo "== xcodegen"
(cd apps/apple && xcodegen generate)

DERIVED_DATA="$ROOT/apps/apple/build/DerivedData"

echo "== iOS simulator build"
xcodebuild -project apps/apple/ODict.xcodeproj -scheme ODict \
    -sdk iphonesimulator -destination 'generic/platform=iOS Simulator' \
    -derivedDataPath "$DERIVED_DATA" CODE_SIGNING_ALLOWED=NO build

echo "== macOS build"
xcodebuild -project apps/apple/ODict.xcodeproj -scheme ODictDesktop \
    -destination 'platform=macOS' \
    -derivedDataPath "$DERIVED_DATA" CODE_SIGNING_ALLOWED=NO build

echo "== done"
