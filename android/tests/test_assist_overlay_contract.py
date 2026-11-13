"""Validate the manifest boundary without Android, a microphone or credentials."""

from pathlib import Path
import unittest
import xml.etree.ElementTree as ET


class AssistOverlayContractTest(unittest.TestCase):
    def setUp(self):
        path = Path(__file__).parents[1] / "app/src/main/AndroidManifest.xml"
        self.root = ET.parse(path).getroot()
        self.android = "{http://schemas.android.com/apk/res/android}"

    def test_overlay_never_requests_microphone_or_accessibility(self):
        permissions = {item.get(self.android + "name") for item in self.root.findall("uses-permission")}
        self.assertNotIn("android.permission.RECORD_AUDIO", permissions)
        self.assertNotIn("android.permission.BIND_ACCESSIBILITY_SERVICE", permissions)
        self.assertIn("android.permission.SYSTEM_ALERT_WINDOW", permissions)
        self.assertIn("android.permission.FOREGROUND_SERVICE_SPECIAL_USE", permissions)

    def test_shortcut_service_is_private_and_not_a_microphone_or_media_service(self):
        services = {item.get(self.android + "name"): item for item in self.root.findall("application/service")}
        shortcut = services[".AssistOverlayService"]
        self.assertEqual(shortcut.get(self.android + "exported"), "false")
        self.assertEqual(shortcut.get(self.android + "foregroundServiceType"), "specialUse")
        self.assertEqual(services[".ReceiverService"].get(self.android + "foregroundServiceType"), "connectedDevice|mediaPlayback")

    def test_shortcut_resume_does_not_export_an_external_start_command(self):
        receivers = {item.get(self.android + "name"): item for item in self.root.findall("application/receiver")}
        receiver = receivers[".AssistOverlayBootReceiver"]
        self.assertEqual(receiver.get(self.android + "exported"), "false")
        self.assertEqual({item.get(self.android + "name") for item in receiver.findall("intent-filter/action")},
                         {"android.intent.action.BOOT_COMPLETED", "android.intent.action.MY_PACKAGE_REPLACED"})


if __name__ == "__main__":
    unittest.main()
