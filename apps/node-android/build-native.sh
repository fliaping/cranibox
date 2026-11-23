#!/bin/sh
# Builds the Cranibox node for Android and places it where Gradle packages native libraries.
# The node is an executable; naming it lib*.so lets Android extract it into the app's native
# library directory, the only app directory it may execute from (Android 10+).
#
# macOS / x86_64 Linux: install the NDK (Android Studio › SDK Manager, or sdkmanager "ndk;27.2.12479018")
# and `rustup target add aarch64-linux-android`; ANDROID_NDK_HOME is found under ANDROID_HOME.
#
# arm64 Linux has no official NDK compilers. Use the host clang + lld with the NDK sysroot:
#   CC_aarch64_linux_android / CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER pointing at a wrapper that runs
#   clang --target=aarch64-linux-android31 --sysroot=<ndk>/toolchains/llvm/prebuilt/linux-x86_64/sysroot
#         -resource-dir=<ndk>/toolchains/llvm/prebuilt/linux-x86_64/lib/clang/18 -fuse-ld=lld
#         --rtlib=compiler-rt --unwindlib=libunwind
#   (this script keeps any such variables already set).
set -eu

API=31
TARGET=aarch64-linux-android
PROFILE=${PROFILE:-release}
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
OUT="$HERE/app/src/main/jniLibs/arm64-v8a"

if [ -z "${CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER:-}" ]; then
  SDK=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}
  if [ -z "$SDK" ]; then
    case "$(uname -s)" in
      Darwin) SDK="$HOME/Library/Android/sdk" ;;
      *) SDK="$HOME/Android/Sdk" ;;
    esac
  fi
  NDK=${ANDROID_NDK_HOME:-$(ls -d "$SDK"/ndk/27.* 2>/dev/null | sort | tail -1)}
  [ -d "$NDK" ] || { echo "NDK not found; set ANDROID_NDK_HOME" >&2; exit 1; }
  case "$(uname -s)" in
    Darwin) HOST=darwin-x86_64 ;;
    Linux) HOST=linux-x86_64 ;;
    *) echo "unsupported host" >&2; exit 1 ;;
  esac
  BIN="$NDK/toolchains/llvm/prebuilt/$HOST/bin"
  [ -x "$BIN/${TARGET}${API}-clang" ] || { echo "NDK compiler is not executable on this host; set the target linker explicitly (see this script's header)" >&2; exit 1; }
  export CC_aarch64_linux_android="$BIN/${TARGET}${API}-clang"
  export AR_aarch64_linux_android="$BIN/llvm-ar"
  export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CC_aarch64_linux_android"
fi

case "$PROFILE" in
  release) FLAG=--release ;;
  debug) FLAG= ;;
  *) echo "PROFILE must be release or debug" >&2; exit 1 ;;
esac
cd "$ROOT"
# shellcheck disable=SC2086
cargo build --locked -p cranibox --no-default-features --target "$TARGET" $FLAG
mkdir -p "$OUT"
cp "${CARGO_TARGET_DIR:-$ROOT/target}/$TARGET/$PROFILE/cranibox" "$OUT/libcranibox.so"
echo "$OUT/libcranibox.so"
