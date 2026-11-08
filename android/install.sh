#!/usr/bin/env bash
set -euo pipefail

usage() {
    printf '%s\n' 'Usage: install.sh --serial TLS_ENDPOINT --expected-device DEVICE_SERIAL [--adb PATH] [--apk PATH] [--apply]' \
        'Default is read-only preflight. --apply installs only org.openchromecast.receiver.' \
        'Never connects/pairs, starts apps, stops another receiver or clears data.'
}
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
adb=${ADB:-adb}
serial= expected= apply=false
apk="$root/app/build/outputs/apk/debug/app-debug.apk"
while (($#)); do
    case "$1" in
        -h|--help) usage; exit 0 ;;
        --serial|--expected-device|--adb|--apk)
            [[ $# -ge 2 && -n "$2" ]] || { usage >&2; exit 2; }
            case "$1" in --serial) serial=$2 ;; --expected-device) expected=$2 ;; --adb) adb=$2 ;; --apk) apk=$2 ;; esac
            shift 2 ;;
        --apply) apply=true; shift ;;
        *) usage >&2; exit 2 ;;
    esac
done
[[ -n "$serial" && -n "$expected" ]] || { usage >&2; exit 2; }
: "${ANDROID_HOME:?Set ANDROID_HOME for APK inspection}"
[[ -f "$apk" ]] || { printf '%s\n' 'APK missing' >&2; exit 1; }
package=$("$ANDROID_HOME/cmdline-tools/latest/bin/apkanalyzer" manifest application-id "$apk")
[[ "$package" == org.openchromecast.receiver ]] || { printf '%s\n' 'Unexpected APK package' >&2; exit 1; }
state=$("$adb" -s "$serial" get-state | tr -d '\r')
[[ "$state" == device ]] || { printf '%s\n' 'Device not authorized' >&2; exit 1; }
device=$("$adb" -s "$serial" shell getprop ro.serialno | tr -d '\r')
[[ "$device" == "$expected" ]] || { printf '%s\n' 'Device identity mismatch' >&2; exit 1; }
sdk=$("$adb" -s "$serial" shell getprop ro.build.version.sdk | tr -d '\r')
[[ "$sdk" =~ ^[0-9]+$ && "$sdk" -ge 26 ]] || { printf '%s\n' 'Unsupported SDK' >&2; exit 1; }
abis=$("$adb" -s "$serial" shell getprop ro.product.cpu.abilist | tr -d '\r')
contents=$(unzip -Z1 "$apk")
compatible=false
for abi in armeabi-v7a arm64-v8a; do
    if [[ ",$abis," == *",$abi,"* && "$contents" == *"lib/$abi/libopenchromecast_android.so"* ]]; then compatible=true; fi
done
[[ "$compatible" == true ]] || { printf '%s\n' 'APK ABI mismatch' >&2; exit 1; }
printf 'package=%s sdk=%s abis=%s\n' "$package" "$sdk" "$abis"
sha256sum "$apk"
if [[ "$apply" == false ]]; then printf '%s\n' 'preflight=ok; no device changes'; exit 0; fi
install_path=$apk
if [[ "$adb" == *.exe ]]; then install_path=$(wslpath -w "$apk"); fi
"$adb" -s "$serial" install -r "$install_path"
installed_path=$("$adb" -s "$serial" shell pm path org.openchromecast.receiver | tr -d '\r')
[[ "$installed_path" == package:* ]] || { printf '%s\n' 'Installed package not found on reread' >&2; exit 1; }
printf '%s\n' "$installed_path"
printf '%s\n' 'installed=true; not started; existing applications untouched'
