#!/usr/bin/env bash
# Build ODict for Apple platforms.
#
# Usage:
#   ./scripts/build-app.sh [all|ios|macos] [--release] [--no-ffi] [--open] [--install] [--help]
#
#   all             FFI + project + iOS + macOS（默认）
#   ios             FFI + project + iOS simulator build
#   macos           FFI + project + macOS build
#   --release       Xcode 用 Release 配置（默认 Debug）
#   --no-ffi        reuse existing ODCFFI.xcframework
#   --open          open the Xcode project afterwards
#   --install       install the macOS app to $ODC_INSTALL_DIR and launch it
#                   (macOS only: the iOS app is not available yet)
#   --help          显示本帮助
#
# Environment:
#   ODC_INSTALL_DIR   destination directory for --install（默认 /Applications）
#   ODC_SKIP_LAUNCH   set to 1 to skip launching the app after --install
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
INSTALL=0
CONFIG="Debug"

APP_NAME="ODict Desktop"
APP_BUNDLE_ID="com.daxmate.odict.desktop"
INSTALL_DIR="${ODC_INSTALL_DIR:-/Applications}"
MACOS_PRODUCT=""

usage() {
  cat <<'EOF'
Usage: ./scripts/build-app.sh [all|ios|macos] [--release] [--no-ffi] [--open] [--install] [--help]

  all             FFI + project + iOS + macOS (default)
  ios             FFI + project + iOS simulator build
  macos           FFI + project + macOS build
  --release       build Xcode targets with Release configuration (default Debug)
  --no-ffi        reuse existing ODCFFI.xcframework
  --open          open the Xcode project afterwards
  --install       install the macOS app to $ODC_INSTALL_DIR (default /Applications)
                  and launch it; macOS only, the iOS app is not available yet
  --help          show this help

Environment:
  ODC_INSTALL_DIR   install destination for --install (default /Applications)
  ODC_SKIP_LAUNCH   set to 1 to skip launching the app after --install
EOF
}

while (($#)); do
  case "$1" in
    all|ios|macos) TARGET="$1" ;;
    --no-ffi) BUILD_FFI=0 ;;
    --release) RELEASE=1 ;;
    --open) OPEN_PROJECT=1 ;;
    --install) INSTALL=1 ;;
    -h|--help) usage; exit 0 ;;
    *)
      echo "build-app.sh: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

if (( INSTALL )) && [[ "$TARGET" == "ios" ]]; then
  echo "build-app.sh: iOS 端尚未开发，--install 仅支持 macOS" >&2
  exit 2
fi

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
  copy_product macosx "$APP_NAME" \
    "$DERIVED_DATA/Build/Products/$CONFIG/$APP_NAME.app"
  MACOS_PRODUCT="$PRODUCTS_DIR/macosx/$CONFIG/$APP_NAME.app"
}

# install_macos installs the freshly built macOS app to $ODC_INSTALL_DIR.
# The old bundle is moved to /tmp instead of being deleted, so it stays recoverable.
install_macos() {
  local src="$MACOS_PRODUCT"
  local dest="$INSTALL_DIR/$APP_NAME.app"

  if [[ ! -d "$src" ]]; then
    echo "build-app.sh: macOS product not found: $src" >&2
    exit 1
  fi

  echo "== install macOS app: $dest"

  # 1) Quit any running copy. Failures must not abort the install.
  osascript -e "quit app id \"$APP_BUNDLE_ID\"" >/dev/null 2>&1 || true
  pkill -x "$APP_NAME" >/dev/null 2>&1 || true
  sleep 1

  # 2) Non-destructive replacement: move the old bundle aside (never rm -rf).
  if [[ -e "$dest" ]]; then
    local backup="/tmp/$APP_NAME-old-$(date +%Y%m%d-%H%M%S).app"
    if ! mv "$dest" "$backup"; then
      echo "build-app.sh: cannot move old app out of the way: $dest" >&2
      exit 1
    fi
    echo "== previous app moved to: $backup"
  fi

  # 3) Copy the new bundle into place.
  if ! mkdir -p "$INSTALL_DIR"; then
    echo "build-app.sh: cannot create install directory: $INSTALL_DIR" >&2
    exit 1
  fi
  if ! ditto "$src" "$dest"; then
    echo "build-app.sh: ditto failed: $src -> $dest" >&2
    exit 1
  fi
  echo "== installed: $dest"

  # 4) Launch unless explicitly skipped.
  if [[ "${ODC_SKIP_LAUNCH:-0}" == "1" ]]; then
    echo "== ODC_SKIP_LAUNCH=1: skipping launch"
  elif ! open "$dest"; then
    echo "build-app.sh: launch failed: $dest" >&2
    exit 1
  fi
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

if (( INSTALL )); then
  if [[ "$TARGET" == "all" ]]; then
    echo "== --install: installing the macOS app only (the iOS app is not available yet)"
  fi
  install_macos
fi

echo "== done"
