#!/usr/bin/env bash
# Build ODict for Apple platforms.
#
# Usage:
#   ./scripts/build-app.sh [all|ios|macos] [--release] [--no-ffi] [--open] [--help]
#
#   all             FFI + project + iOS + macOS（默认）
#   ios             FFI + project + iOS simulator build
#   macos           FFI + project + macOS build
#   --release       Xcode 用 Release 配置（默认 Debug）
#   --no-ffi        reuse existing ODCFFI.xcframework
#   --open          open the Xcode project afterwards
#   --help          显示本帮助
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TARGET="all"
BUILD_FFI=1
OPEN_PROJECT=0
RELEASE=0
CONFIG="Debug"

usage() {
  cat <<'EOF'
Usage: ./scripts/build-app.sh [all|ios|macos] [--release] [--no-ffi] [--open] [--help]

  all             FFI + project + iOS + macOS (default)
  ios             FFI + project + iOS simulator build
  macos           FFI + project + macOS build
  --release       build Xcode targets with Release configuration (default Debug)
  --no-ffi        reuse existing ODCFFI.xcframework
  --open          open the Xcode project afterwards
  --help          show this help
EOF
}

while (($#)); do
  case "$1" in
    all|ios|macos) TARGET="$1" ;;
    --no-ffi) BUILD_FFI=0 ;;
    --release) RELEASE=1 ;;
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

if (( RELEASE )); then
  CONFIG="Release"
fi

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
    -configuration "$CONFIG" \
    -sdk iphonesimulator -destination 'generic/platform=iOS Simulator' \
    -derivedDataPath "$DERIVED_DATA" CODE_SIGNING_ALLOWED=NO build
}

build_macos() {
  echo "== macOS build"
  xcodebuild -project apps/apple/ODict.xcodeproj -scheme ODictDesktop \
    -configuration "$CONFIG" \
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
