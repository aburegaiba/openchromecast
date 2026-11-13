"""Fresh-deployment contract, using synthetic APKs and no Android/network tools."""

import hashlib
import importlib.util
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zipfile


SPEC = importlib.util.spec_from_file_location("deploy", Path(__file__).parents[1] / "deploy.py")
deploy = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(deploy)


class DeploymentTest(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.apk = Path(directory.name) / "fixture.apk"
        with zipfile.ZipFile(self.apk, "w") as archive:
            archive.writestr("lib/armeabi-v7a/libopenchromecast_android.so", "synthetic")
        self.hash = hashlib.sha256(self.apk.read_bytes()).hexdigest()
        self.signer = "a" * 64
        self.args = deploy.parser().parse_args([
            "--profile", "cronos", "--serial", "fixture:12345", "--expected-device", "fixture",
            "--apk", str(self.apk), "--apk-sha256", self.hash, "--signer-sha256", self.signer])
        self.props = {"ro.serialno": "fixture", "ro.product.device": "cronos",
                      "ro.build.version.sdk": "30", "ro.lineage.version": "18.1-fixture",
                      "ro.product.cpu.abilist": "armeabi-v7a"}
        self.installed = None
        self.uid = "2000"
        self.state = "device"
        self.location = "package:/data/app/~~synthetic==/org.openchromecast.receiver-fixture/base.apk"
        self.install_result_hash = self.hash
        self.calls = []
        self.command_patch = patch.object(deploy, "command", self.command)
        self.command_patch.start()
        self.addCleanup(self.command_patch.stop)
        self.env_patch = patch.dict(os.environ, ANDROID_HOME="/synthetic/sdk")
        self.env_patch.start()
        self.addCleanup(self.env_patch.stop)

    def command(self, args):
        self.calls.append(args)
        if args[1:3] == ["manifest", "application-id"]:
            return deploy.PACKAGE
        if args[1:3] == ["verify", "--print-certs"]:
            return "Signer #1 certificate SHA-256 digest: " + self.signer
        if args[0] == "bash":
            self.installed = self.install_result_hash
            return "installed"
        tail = args[3:]
        if tail == ["get-state"]:
            return self.state
        if tail[:2] == ["shell", "getprop"]:
            return self.props[tail[2]]
        if tail == ["shell", "id", "-u"]:
            return self.uid
        if tail == ["shell", "pm", "path", deploy.PACKAGE]:
            return self.location if self.installed else ""
        if tail[:2] == ["shell", "sha256sum"]:
            return self.installed + "  /synthetic/base.apk"
        self.fail(f"Unexpected command: {args}")

    def assert_no_install(self):
        self.assertFalse(any(call[0] == "bash" for call in self.calls))

    def test_dry_run_never_installs(self):
        deploy.deploy(self.args)
        self.assert_no_install()

    def test_apply_installs_and_verifies_hash(self):
        self.args.apply = True
        deploy.deploy(self.args)
        self.assertEqual(self.installed, self.hash)

    def test_existing_identical_apk_is_noop_even_with_apply(self):
        self.installed = self.hash
        self.args.apply = True
        deploy.deploy(self.args)
        self.assert_no_install()

    def test_existing_different_apk_refuses_upgrade(self):
        self.installed = "b" * 64
        self.args.apply = True
        with self.assertRaisesRegex(deploy.Refused, "different receiver"):
            deploy.deploy(self.args)
        self.assert_no_install()

    def test_wrong_device_model_stock_os_and_abi_refused(self):
        for key, bad in (("ro.serialno", "other"), ("ro.product.device", "athena"),
                         ("ro.lineage.version", ""), ("ro.build.version.sdk", "33"),
                         ("ro.product.cpu.abilist", "x86_64")):
            with self.subTest(key=key):
                original = self.props[key]
                self.props[key] = bad
                with self.assertRaises(deploy.Refused):
                    deploy.deploy(self.args)
                self.props[key] = original
                self.assert_no_install()

    def test_hash_and_signer_refused(self):
        for field in ("apk_sha256", "signer_sha256"):
            with self.subTest(field=field):
                original = getattr(self.args, field)
                setattr(self.args, field, "b" * 64)
                with self.assertRaises(deploy.Refused):
                    deploy.deploy(self.args)
                setattr(self.args, field, original)
                self.assert_no_install()

    def test_legacy_adb_refused_before_contact(self):
        self.args.serial = "fixture:5555"
        with self.assertRaisesRegex(deploy.Refused, "Legacy"):
            deploy.deploy(self.args)
        self.assertEqual(self.calls, [])

    def test_profile_accepts_lineage_prefix(self):
        self.props["ro.product.device"] = "lineage_cronos"
        deploy.deploy(self.args)
        self.assert_no_install()

    def test_root_and_unauthorized_adb_refused(self):
        self.uid = "0"
        with self.assertRaisesRegex(deploy.Refused, "non-root"):
            deploy.deploy(self.args)
        self.uid = "2000"
        self.state = "unauthorized"
        with self.assertRaisesRegex(deploy.Refused, "not authorized"):
            deploy.deploy(self.args)
        self.assert_no_install()

    def test_split_and_unsafe_installed_paths_refused(self):
        self.installed = self.hash
        for location in ("package:/data/app/fixture/base.apk\npackage:/data/app/fixture/split.apk",
                         "package:/data/app/fixture/../base.apk", "package:/data/app/$(unsafe)/base.apk"):
            with self.subTest(location=location):
                self.location = location
                with self.assertRaises(deploy.Refused):
                    deploy.deploy(self.args)
                self.assert_no_install()

    def test_post_install_checksum_must_match(self):
        self.args.apply = True
        self.install_result_hash = "b" * 64
        with self.assertRaisesRegex(deploy.Refused, "checksum does not match"):
            deploy.deploy(self.args)


if __name__ == "__main__":
    unittest.main()
