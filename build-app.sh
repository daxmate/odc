#!/usr/bin/env bash
# Build ODict for Apple platforms.
#
# Usage:
#   ./build-app.sh                 # FFI + project + iOS + macOS
#   ./build-app.sh ios             # FFI + project + iOS simulator build
#   ./build-app.sh macos           # FFI + project + macOS build
#   ./build-app.sh --no-ffi        # reuse existing ODCFFI.xcframework
#   ./build-app.sh --open          # open the Xcode project afterwards
#   ./build-app.sh --help
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

TARGET="all"
BUILD_FFI=1
OPEN_PROJECT=0

usage() {
  sed -n '2,9p' "$0" | sed 's/^# //'
}

while (($#)); do
  case "$1" in
    all|ios|macos) TARGET="$1" ;;
    --no-ffi) BUILD_FFI=0 ;;
    --open) OPEN_PROJECT=1 ;;
    -h|--help) usage; exit 0 ;;
    *)
      echo "build-app.sh: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

command -v xcodebuild >/dev/null 2>&1 || {
  echo "build-app.sh: xcodebuild not found (install Xcode)" >&2
  exit 1
}
command -v xcodegen >/dev/null 2>&1 || {
  echo "build-app.sh: xcodegen not found (brew install xcodegen)" >&2
  exit 1
}

export PATH="$HOME/.cargo/bin:$PATH"

FRAMEWORK="apps/apple/ODictKit/Frameworks/ODCFFI.xcframework"
if (( BUILD_FFI )); then
  echo "== ODCFFI.xcframework"
  bash scripts/build-odc-xcframework.sh
elif [[ ! -d "$FRAMEWORK" ]]; then
  echo "build-app.sh: $FRAMEWORK is missing; run without --no-ffi" >&2
  exit 1
fi

echo "== xcodegen"
(cd apps/apple && xcodegen generate)

DERIVED_DATA="$ROOT/apps/apple/build/DerivedData"

build_ios() {
  echo "== iOS simulator build"
  xcodebuild -project apps/apple/ODict.xcodeproj -scheme ODict \
    -sdk iphonesimulator -destination 'generic/platform=iOS Simulator' \
    -derivedDataPath "$DERIVED_DATA" CODE_SIGNING_ALLOWED=NO build
}

build_macos() {
  echo "== macOS build"
  xcodebuild -project apps/apple/ODict.xcodeproj -scheme ODictDesktop \
    -destination 'platform=macOS' \
    -derivedDataPath "$DERIVED_DATA" CODE_SIGNING_ALLOWED=NO build
}

case "$TARGET" in
  all)
    build_ios
    build_macos
    ;;
  ios) build_ios ;;
  macos) build_macos ;;
esac

if (( OPEN_PROJECT )); then
  open apps/apple/ODict.xcodeproj
fi

echo "== done"
