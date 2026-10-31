#!/usr/bin/env bash
# Build ODCFFI.xcframework from crates/odc-ffi.
#
# Slices:
#   macOS            aarch64-apple-darwin + x86_64-apple-darwin (universal)
#   iOS device       aarch64-apple-ios
#   iOS simulator    aarch64-apple-ios-sim + x86_64-apple-ios (universal)
#
# Usage:
#   scripts/build-odc-xcframework.sh [--install-targets]
#
# Requires rustup; the Homebrew "rust" formula has no iOS standard library.
# Intermediate artifacts go to apps/apple/build/ and the framework to
# apps/apple/ODictKit/Frameworks/; both are gitignored.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRATE_DIR="$ROOT/crates/odc-ffi"
HEADERS_DIR="$CRATE_DIR/include"
BUILD_DIR="$ROOT/apps/apple/build"
CARGO_TARGET_DIR="$BUILD_DIR/cargo-target"
LIPO_DIR="$BUILD_DIR/lipo"
OUT_DIR="$ROOT/apps/apple/ODictKit/Frameworks"
STATIC_LIB="libodc_ffi.a"
DIST_SERVER="${RUSTUP_DIST_SERVER:-https://rsproxy.cn}"
# Keep the C objects (SQLite, zstd) and the linker on the same minimum OS
# version; mismatched deployment targets make ld reject libSystem symbols.
IOS_MIN_VERSION="${IPHONEOS_DEPLOYMENT_TARGET:-15.0}"
MACOS_MIN_VERSION="${MACOSX_DEPLOYMENT_TARGET:-11.0}"

MACOS_TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
IOS_DEVICE_TARGETS=(aarch64-apple-ios)
IOS_SIM_TARGETS=(aarch64-apple-ios-sim x86_64-apple-ios)
ALL_TARGETS=("${MACOS_TARGETS[@]}" "${IOS_DEVICE_TARGETS[@]}" "${IOS_SIM_TARGETS[@]}")

INSTALL_TARGETS=0

usage() {
    cat <<'EOF'
Build ODCFFI.xcframework from crates/odc-ffi.

Usage:
  scripts/build-odc-xcframework.sh [--install-targets]

Slices: macOS universal, iOS device, iOS simulator universal.
Requires rustup (Homebrew rust has no iOS standard library).
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --install-targets) INSTALL_TARGETS=1 ;;
        -h|--help) usage; exit 0 ;;
        *)
            echo "error: unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
    shift
done

if [ ! -f "$HEADERS_DIR/odc.h" ] || [ ! -f "$HEADERS_DIR/module.modulemap" ]; then
    echo "error: missing headers in $HEADERS_DIR" >&2
    exit 1
fi

if ! command -v xcodebuild >/dev/null 2>&1; then
    echo "error: xcodebuild not found; install Xcode command line tools." >&2
    exit 1
fi
if ! command -v lipo >/dev/null 2>&1; then
    echo "error: lipo not found; install Xcode command line tools." >&2
    exit 1
fi

# Prefer the rustup-managed toolchain; never touch the Homebrew rust install.
if [ -x "$HOME/.cargo/bin/rustup" ]; then
    export PATH="$HOME/.cargo/bin:$PATH"
fi

if ! command -v rustup >/dev/null 2>&1; then
    echo "error: rustup is required to build the iOS slices." >&2
    if command -v cargo >/dev/null 2>&1; then
        echo "found: $(cargo --version 2>/dev/null) at $(command -v cargo)" >&2
        echo "that toolchain is likely Homebrew rust and ships no iOS standard library." >&2
    fi
    echo "install rustup without removing Homebrew rust:" >&2
    echo "  brew install rustup" >&2
    echo "  export PATH=\"\$HOME/.cargo/bin:\$PATH\"" >&2
    echo "  export RUSTUP_DIST_SERVER=$DIST_SERVER" >&2
    echo "  rustup default stable" >&2
    exit 2
fi

export RUSTUP_DIST_SERVER="$DIST_SERVER"
export RUSTUP_UPDATE_ROOT="${RUSTUP_UPDATE_ROOT:-https://rsproxy.cn/rustup}"

installed_targets="$(rustup target list --installed)"
missing_targets=()
for target in "${ALL_TARGETS[@]}"; do
    if ! printf '%s\n' "$installed_targets" | grep -qx "$target"; then
        missing_targets+=("$target")
    fi
done

if [ ${#missing_targets[@]} -gt 0 ]; then
    if [ "$INSTALL_TARGETS" -eq 1 ]; then
        echo "== installing rust targets via $RUSTUP_DIST_SERVER =="
        rustup target add "${missing_targets[@]}"
    else
        echo "error: missing rust standard libraries: ${missing_targets[*]}" >&2
        echo "install them with:" >&2
        echo "  RUSTUP_DIST_SERVER=$DIST_SERVER rustup target add ${missing_targets[*]}" >&2
        echo "or re-run this script with --install-targets" >&2
        exit 2
    fi
fi

# Keep cargo's cache between runs; only the assembled artifacts are rebuilt.
echo "== preparing $BUILD_DIR =="
rm -rf "$LIPO_DIR"
mkdir -p "$CARGO_TARGET_DIR" "$LIPO_DIR"

export CARGO_TARGET_DIR
export IPHONEOS_DEPLOYMENT_TARGET="$IOS_MIN_VERSION"
export MACOSX_DEPLOYMENT_TARGET="$MACOS_MIN_VERSION"
export CARGO_TARGET_AARCH64_APPLE_DARWIN_RUSTFLAGS="-Clink-arg=-mmacosx-version-min=$MACOS_MIN_VERSION"
export CARGO_TARGET_X86_64_APPLE_DARWIN_RUSTFLAGS="-Clink-arg=-mmacosx-version-min=$MACOS_MIN_VERSION"
export CARGO_TARGET_AARCH64_APPLE_IOS_RUSTFLAGS="-Clink-arg=-mios-version-min=$IOS_MIN_VERSION"
export CARGO_TARGET_X86_64_APPLE_IOS_RUSTFLAGS="-Clink-arg=-mios-simulator-version-min=$IOS_MIN_VERSION"
export CARGO_TARGET_AARCH64_APPLE_IOS_SIM_RUSTFLAGS="-Clink-arg=-mios-simulator-version-min=$IOS_MIN_VERSION"

for target in "${ALL_TARGETS[@]}"; do
    echo "== cargo build --release -p odc-ffi --target $target =="
    cargo build --release -p odc-ffi --target "$target"
done

make_universal() {
    local name="$1"
    shift
    local inputs=()
    local output_dir="$LIPO_DIR/$name"
    mkdir -p "$output_dir"
    for target in "$@"; do
        inputs+=("$CARGO_TARGET_DIR/$target/release/$STATIC_LIB")
    done
    lipo -create -output "$output_dir/$STATIC_LIB" "${inputs[@]}"
    echo "   $name: $(lipo -info "$output_dir/$STATIC_LIB" | sed 's/^[^:]*: //')"
}

echo "== lipo =="
make_universal "macos" "${MACOS_TARGETS[@]}"
make_universal "ios-device" "${IOS_DEVICE_TARGETS[@]}"
make_universal "ios-simulator" "${IOS_SIM_TARGETS[@]}"

echo "== xcodebuild -create-xcframework =="
mkdir -p "$OUT_DIR"
rm -rf "$OUT_DIR/ODCFFI.xcframework"
xcodebuild -create-xcframework \
    -library "$LIPO_DIR/macos/$STATIC_LIB" -headers "$HEADERS_DIR" \
    -library "$LIPO_DIR/ios-device/$STATIC_LIB" -headers "$HEADERS_DIR" \
    -library "$LIPO_DIR/ios-simulator/$STATIC_LIB" -headers "$HEADERS_DIR" \
    -output "$OUT_DIR/ODCFFI.xcframework"

echo "== done: $OUT_DIR/ODCFFI.xcframework =="
