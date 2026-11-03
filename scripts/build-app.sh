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
#
# Build artifacts:
#   intermediates: build/DerivedData
#   products:      build/Products/<sdk>/<Config>/<App>.app
#   (both overridable via DERIVED_DATA / PRODUCTS_DIR)
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

# Build artifacts stay inside the working tree: <root>/build/.
# Overridable through the environment so CI or one-off runs can relocate them.
DERIVED_DATA="${DERIVED_DATA:-$ROOT/build/DerivedData}"
PRODUCTS_DIR="${PRODUCTS_DIR:-$ROOT/build/Products}"
PRODUCT_PATHS=()

# copy_product <sdk> <bundle name> <source bundle>
# Copies the freshly built bundle to build/Products/<sdk>/<Config>/.
copy_product() {
  local sdk="$1" bundle="$2" src="$3"
  local dest_dir="$PRODUCTS_DIR/$sdk/$CONFIG"
  local dest="$dest_dir/$bundle.app"

  if [[ ! -d "$src" ]]; then
    echo "build-app.sh: expected product not found: $src" >&2
    exit 1
  fi

  # Refuse obviously unsafe destinations (empty, relative, or not an .app).
  case "$dest" in
    /*/*.app) ;;
    *)
      echo "build-app.sh: refusing unsafe product destination: $dest" >&2
      exit 1
      ;;
  esac

  mkdir -p "$dest_dir"
  if [[ -e "$dest" ]]; then
    rm -rf "$dest"
  fi
  ditto "$src" "$dest"
  PRODUCT_PATHS+=("$dest")
  echo "== product: $dest"
}

build_ios() {
  echo "== iOS simulator build"
  xcodebuild -project apps/apple/ODict.xcodeproj -scheme ODict \
    -configuration "$CONFIG" \
    -sdk iphonesimulator -destination 'generic/platform=iOS Simulator' \
    -derivedDataPath "$DERIVED_DATA" CODE_SIGNING_ALLOWED=NO build
  copy_product iphonesimulator "ODict" \
    "$DERIVED_DATA/Build/Products/$CONFIG-iphonesimulator/ODict.app"
}

build_macos() {
  echo "== macOS build"
  xcodebuild -project apps/apple/ODict.xcodeproj -scheme ODictDesktop \
    -configuration "$CONFIG" \
    -destination 'platform=macOS' \
    -derivedDataPath "$DERIVED_DATA" CODE_SIGNING_ALLOWED=NO build
  copy_product macosx "ODict Desktop" \
    "$DERIVED_DATA/Build/Products/$CONFIG/ODict Desktop.app"
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

if (( ${#PRODUCT_PATHS[@]} > 0 )); then
  echo "== products"
  for product in "${PRODUCT_PATHS[@]}"; do
    echo "$product"
  done
fi

echo "== done"
