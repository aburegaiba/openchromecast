"""Exercise the deployment guard without a device, network or real APK."""

import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import zipfile


class InstallGuardsTest(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.script = Path(__file__).resolve().parents[1] / "install.sh"
        self.sdk = self.root / "sdk"
        inspector = self.sdk / "cmdline-tools/latest/bin/apkanalyzer"
        inspector.parent.mkdir(parents=True)
        inspector.write_text(f"#!{sys.executable}\nimport os\nprint(os.getenv('FAKE_PACKAGE', 'org.openchromecast.receiver'))\n")
        inspector.chmod(0o755)
        self.adb = self.root / "adb"
        self.adb.write_text(f"#!{sys.executable}\n" + """import os, sys
from pathlib import Path
args = sys.argv[3:]
if args == ['get-state']: print('device')
elif args[:2] == ['shell', 'getprop']:
    key = args[2]
    print({'ro.serialno':os.getenv('FAKE_DEVICE', 'test-device'),
           'ro.build.version.sdk':'30',
           'ro.product.cpu.abilist':os.getenv('FAKE_ABIS', 'armeabi-v7a')}.get(key, ''))
elif args[:1] == ['install']:
    Path(os.environ['FAKE_MUTATIONS']).write_text('installed')
    print('Success')
elif args[:3] == ['shell', 'pm', 'path']: print('package:/data/app/test/base.apk')
else: raise SystemExit(3)
""")
        self.adb.chmod(0o755)
        self.apk = self.root / "test.apk"
        with zipfile.ZipFile(self.apk, "w") as archive:
            archive.writestr("lib/armeabi-v7a/libopenchromecast_android.so", b"fixture")
        self.marker = self.root / "mutations"
        self.environment = dict(os.environ, ANDROID_HOME=str(self.sdk), FAKE_MUTATIONS=str(self.marker))

    def run_install(self, *extra):
        return subprocess.run(["bash", str(self.script), "--serial", "test-endpoint", "--expected-device", "test-device",
            "--adb", str(self.adb), "--apk", str(self.apk), *extra], env=self.environment,
            text=True, capture_output=True, check=False)

    def test_preflight_never_installs(self):
        result = self.run_install()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("preflight=ok", result.stdout)
        self.assertFalse(self.marker.exists())

    def test_apply_installs_only_after_checks(self):
        result = self.run_install("--apply")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.marker.exists())

    def test_wrong_device_never_installs(self):
        self.environment["FAKE_DEVICE"] = "other-device"
        result = self.run_install("--apply")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.marker.exists())

    def test_wrong_package_never_installs(self):
        self.environment["FAKE_PACKAGE"] = "org.other.app"
        result = self.run_install("--apply")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.marker.exists())

    def test_wrong_architecture_never_installs(self):
        self.environment["FAKE_ABIS"] = "x86_64"
        result = self.run_install("--apply")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.marker.exists())

    def test_help_has_no_prerequisites(self):
        result = subprocess.run(["bash", str(self.script), "--help"], env={"PATH":os.environ["PATH"]}, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0)
        self.assertIn("--apply", result.stdout)


if __name__ == "__main__":
    unittest.main()
