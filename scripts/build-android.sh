#!/usr/bin/env bash
#
# Build the JanusOS Android application.
#
# Two artifacts come out of this script:
#
#   * app/build/outputs/apk/<mode>/app-<mode>.apk
#       installable on any Android 12+ device with `adb install`
#   * android/device/janus/titan/janus_core/JanusCore.apk
#       the same APK staged for the AOSP image import (only with --stage)
#
# Prerequisites:
#   rustup target add aarch64-linux-android
#   cargo install cargo-ndk
#   Android SDK (platform 34, build-tools 34.0.0) + NDK r26/r27
#   JDK 17 and the Gradle wrapper (gradle/wrapper/gradle-wrapper.jar)
#
# Usage:
#   scripts/build-android.sh                 # arm64 debug APK
#   scripts/build-android.sh --release       # minified release APK
#   scripts/build-android.sh --full          # include the 34 MB module catalogue
#   scripts/build-android.sh --abi x86_64    # emulator build
#   scripts/build-android.sh --stage         # copy APK into the AOSP overlay
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

ABI="arm64-v8a"
BUILD_TYPE="debug"
FULL_CATALOGUE="false"
STAGE="false"

usage() {
    sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    exit "${1:-0}"
}

while [ $# -gt 0 ]; do
    case "$1" in
        --release) BUILD_TYPE="release" ;;
        --debug) BUILD_TYPE="debug" ;;
        --full) FULL_CATALOGUE="true" ;;
        --stage) STAGE="true" ;;
        --abi)
            shift
            ABI="${1:?--abi needs a value (arm64-v8a, x86_64, armeabi-v7a)}"
            ;;
        -h|--help) usage 0 ;;
        *) echo "Unknown argument: $1" >&2; usage 1 ;;
    esac
    shift
done

case "$ABI" in
    arm64-v8a) RUST_TARGET="aarch64-linux-android" ;;
    x86_64) RUST_TARGET="x86_64-linux-android" ;;
    armeabi-v7a) RUST_TARGET="armv7-linux-androideabi" ;;
    *) echo "Unsupported ABI: $ABI" >&2; exit 1 ;;
esac

need() {
    command -v "$1" >/dev/null 2>&1 || {
        echo "Missing prerequisite: $1${2:+ — $2}" >&2
        exit 1
    }
}

need cargo "install Rust: https://rustup.rs"
need cargo-ndk "cargo install cargo-ndk"
need java "install JDK 17"
[ -n "${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}" ] || {
    echo "Missing ANDROID_NDK_HOME (or ANDROID_NDK_ROOT)" >&2
    exit 1
}
[ -f gradle/wrapper/gradle-wrapper.jar ] || {
    echo "gradle/wrapper/gradle-wrapper.jar is missing; run 'gradle wrapper --gradle-version 8.5' once" >&2
    exit 1
}

if ! rustup target list --installed | grep -qx "$RUST_TARGET"; then
    echo "Rust target $RUST_TARGET is not installed; run:" >&2
    echo "  rustup target add $RUST_TARGET" >&2
    exit 1
fi

echo "==> Building libjanus_android.so for $ABI ($RUST_TARGET, release)"
cargo ndk -t "$ABI" -o app/build/janusJniLibs build --release -p janus-android

echo "==> Staging native library"
mkdir -p "app/build/janusJniLibs/$ABI"
cp "target/$RUST_TARGET/release/libjanus_android.so" "app/build/janusJniLibs/$ABI/"

echo "==> Assembling $BUILD_TYPE APK (fullCatalogue=$FULL_CATALOGUE)"
GRADLE_TASK="assemble$(echo "${BUILD_TYPE:0:1}" | tr '[:lower:]' '[:upper:]')${BUILD_TYPE:1}"
./gradlew --no-daemon "$GRADLE_TASK" \
    -PjanusRustAbi="$ABI" \
    -PjanusFullCatalogue="$FULL_CATALOGUE"

APK="app/build/outputs/apk/$BUILD_TYPE/app-$BUILD_TYPE.apk"
[ -f "$APK" ] || { echo "Expected APK not found: $APK" >&2; exit 1; }

SIZE="$(du -h "$APK" | cut -f1)"
echo "==> APK ready: $APK ($SIZE)"

if [ "$STAGE" = "true" ]; then
    STAGE_PATH="android/device/janus/titan/janus_core/JanusCore.apk"
    cp "$APK" "$STAGE_PATH"
    echo "==> Staged for the AOSP image: $STAGE_PATH"
    echo "    Lunch and build in the Radxa CM5 tree:"
    echo "      lunch janus_titan-user$([ "$BUILD_TYPE" = "release" ] && echo "" || echo "debug")"
    echo "      m JanusCore"
    echo "    Note: the platform re-signs the APK with the platform key."
fi
