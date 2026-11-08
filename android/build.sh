#!/usr/bin/env bash
set -euo pipefail

usage() {
    printf '%s\n' 'Usage: build.sh [--abi armeabi-v7a|arm64-v8a|all] [--offline] [--help]' \
        'Requires: Rust Android targets, JDK 21, Android SDK 36, NDK 27+, Gradle 9.2.1.' \
        'Set ANDROID_HOME, ANDROID_NDK_HOME, JAVA_HOME; optional GRADLE executable.'
}
abi=all
offline=()
while (($#)); do
    case "$1" in
        -h|--help) usage; exit 0 ;;
        --abi) [[ $# -ge 2 ]] || { usage >&2; exit 2; }; abi=$2; shift 2 ;;
        --offline) offline=(--offline); shift ;;
        *) usage >&2; exit 2 ;;
    esac
done
case "$abi" in armeabi-v7a|arm64-v8a|all) ;; *) usage >&2; exit 2 ;; esac
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
: "${ANDROID_HOME:?Set ANDROID_HOME}"
: "${ANDROID_NDK_HOME:?Set ANDROID_NDK_HOME}"
: "${JAVA_HOME:?Set JAVA_HOME}"
toolchain="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin"
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-2}
export CARGO_TARGET_DIR="$root/native/target"
export CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_LINKER="$toolchain/armv7a-linux-androideabi26-clang"
export CC_armv7_linux_androideabi="$CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_LINKER"
export AR_armv7_linux_androideabi="$toolchain/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$toolchain/aarch64-linux-android26-clang"
export CC_aarch64_linux_android="$CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER"
export AR_aarch64_linux_android="$toolchain/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS='-C link-arg=-Wl,-z,max-page-size=16384'

for pair in 'armeabi-v7a armv7-linux-androideabi' 'arm64-v8a aarch64-linux-android'; do
    read -r android_abi rust_target <<< "$pair"
    [[ "$abi" == all || "$abi" == "$android_abi" ]] || continue
    cargo build --manifest-path "$root/native/Cargo.toml" --locked \
        "${offline[@]}" --release --target "$rust_target"
    install -d "$root/app/src/main/jniLibs/$android_abi"
    install -m 644 "$CARGO_TARGET_DIR/$rust_target/release/libopenchromecast_android.so" \
        "$root/app/src/main/jniLibs/$android_abi/"
done
"${GRADLE:-gradle}" -p "$root" --no-daemon --console=plain --max-workers=2 \
    "${offline[@]}" :app:testDebugUnitTest :app:lintDebug :app:assembleDebug
sha256sum "$root/app/build/outputs/apk/debug/app-debug.apk"
