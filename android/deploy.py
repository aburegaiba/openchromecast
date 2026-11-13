#!/usr/bin/env python3
"""Guard a fresh Echo deployment; never provision identities or alter a ROM."""

import argparse
import hashlib
import os
from pathlib import Path
import re
import subprocess
import sys
import zipfile


PACKAGE = "org.openchromecast.receiver"
PROFILES = {
    "crown": "Echo Show 8 (2019, generation 1)",
    "checkers": "Echo Show 5 (2019, generation 1)",
    "cronos": "Echo Show 5 (2021, generation 2)",
}
HASH = re.compile(r"[0-9a-f]{64}")
APK_PATH = re.compile(r"package:(/data/app/[A-Za-z0-9_./=+~-]+/base\.apk)")


class Refused(RuntimeError):
    pass


def digest(path):
    checksum = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            checksum.update(block)
    return checksum.hexdigest()


def command(args):
    try:
        result = subprocess.run(args, capture_output=True, text=True, timeout=120, check=False)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise Refused("A required tool failed or timed out; no automatic retry") from error
    if result.returncode:
        raise Refused("A required tool refused the operation; inspect it locally")
    return result.stdout.strip().replace("\r", "")


def parser():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("--profile", choices=PROFILES, required=True)
    cli.add_argument("--serial", required=True, help="Already authorized TLS ADB endpoint or USB transport")
    cli.add_argument("--expected-device", required=True, help="Physical serial verified on the target")
    cli.add_argument("--adb", default=os.environ.get("ADB", "adb"))
    cli.add_argument("--apk", type=Path, required=True)
    cli.add_argument("--apk-sha256", required=True, help="Trusted, independently reviewed APK checksum")
    cli.add_argument("--signer-sha256", required=True, help="Trusted APK signer certificate fingerprint")
    cli.add_argument("--apksigner", type=Path, help="SDK apksigner executable")
    cli.add_argument("--apply", action="store_true", help="Install only if absent; otherwise read-only")
    return cli


def deploy(args):
    expected_hash = args.apk_sha256.lower()
    expected_signer = args.signer_sha256.lower().replace(":", "")
    if not HASH.fullmatch(expected_hash) or not HASH.fullmatch(expected_signer):
        raise Refused("Expected checksums must be SHA256 fingerprints")
    if args.serial.endswith(":5555"):
        raise Refused("Legacy unencrypted network ADB is not supported; use authorized TLS or USB")
    if args.apk.is_symlink() or not args.apk.is_file() or digest(args.apk) != expected_hash:
        raise Refused("APK missing, symlinked or checksum mismatch")
    sdk = Path(os.environ.get("ANDROID_HOME", ""))
    if not os.environ.get("ANDROID_HOME"):
        raise Refused("Set ANDROID_HOME to inspect the APK")
    analyzer = sdk / "cmdline-tools/latest/bin/apkanalyzer"
    signer = args.apksigner or sdk / "build-tools/36.0.0/apksigner"
    if command([str(analyzer), "manifest", "application-id", str(args.apk)]) != PACKAGE:
        raise Refused("Unexpected APK package")
    signed = command([str(signer), "verify", "--print-certs", str(args.apk)])
    fingerprints = re.findall(r"^Signer #\d+ certificate SHA-256 digest: ([0-9a-fA-F]{64})$", signed, re.M)
    if [value.lower() for value in fingerprints] != [expected_signer]:
        raise Refused("Unexpected APK signer (exactly one trusted signer is required)")

    def adb(*parts):
        return command([args.adb, "-s", args.serial, *parts])

    def prop(name):
        return adb("shell", "getprop", name)

    if adb("get-state") != "device":
        raise Refused("Target is not authorized")
    if prop("ro.serialno") != args.expected_device:
        raise Refused("Physical device identity mismatch")
    model = prop("ro.product.device").lower()
    if model not in (args.profile, "lineage_" + args.profile):
        raise Refused("Device model does not match the selected profile")
    if prop("ro.build.version.sdk") != "30" or not prop("ro.lineage.version").startswith("18.1-"):
        raise Refused("Only LineageOS 18.1 / Android 11 is qualified for this preflight; stop on stock FireOS")
    if adb("shell", "id", "-u") != "2000":
        raise Refused("Use non-root ADB shell; this installer never changes ADB privileges")
    abis = prop("ro.product.cpu.abilist").split(",")
    with zipfile.ZipFile(args.apk) as archive:
        compatible = any(abi in abis and f"lib/{abi}/libopenchromecast_android.so" in archive.namelist()
                         for abi in ("armeabi-v7a", "arm64-v8a"))
    if not compatible:
        raise Refused("APK ABI mismatch")

    def installed_hash():
        location = adb("shell", "pm", "path", PACKAGE)
        if not location:
            return None
        match = APK_PATH.fullmatch(location)
        if not match or ".." in match[1].split("/"):
            raise Refused("Unexpected installed APK layout; refuse split or unsafe paths")
        value = adb("shell", "sha256sum", match[1]).split()
        if not value or not HASH.fullmatch(value[0]):
            raise Refused("Cannot verify the installed public APK")
        return value[0]

    current = installed_hash()
    if current is not None and current != expected_hash:
        raise Refused("A different receiver is already installed; use the reviewed backup/upgrade procedure")
    print(f"profile={args.profile}; model={PROFILES[args.profile]}; apk_sha256={expected_hash}")
    if current == expected_hash:
        print("result=already_installed; changes=none; settings_and_playback_untouched")
    elif not args.apply:
        print("result=ready_for_fresh_install; changes=none; rerun with --apply to install only")
    else:
        if digest(args.apk) != expected_hash:
            raise Refused("APK changed after preflight")
        command(["bash", str(Path(__file__).with_name("install.sh")), "--adb", args.adb,
                 "--serial", args.serial, "--expected-device", args.expected_device,
                 "--apk", str(args.apk), "--apply"])
        if installed_hash() != expected_hash:
            raise Refused("Installation attempted but installed APK checksum does not match")
        print("result=installed_and_hash_verified; not_started; settings_untouched")
    print("Acceptance, unique name, private identity and Home Assistant enrollment remain per-device steps; see REINSTALL.md")


def main():
    args = parser().parse_args()
    try:
        deploy(args)
    except (Refused, OSError, zipfile.BadZipFile) as error:
        print(f"REFUSED: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
