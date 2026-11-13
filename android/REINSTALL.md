# Repeatable Echo deployment

The application is independent of a PC, Freebox Pop or remote signing service
after installation and authorized private credential provisioning. Internet
access is still needed for YouTube and its media. This is not a certified
Chromecast, a full FireOS backup, or a one-click ROM installer.

## Hardware and two entry paths

| Echo | Codename | Profile status |
| --- | --- | --- |
| Show 8, generation 1, 2019 | crown | Application tested on LineageOS 18.1 |
| Show 5, generation 1, 2019 | checkers | Preflight profile, not physically qualified here |
| Show 5, generation 2, 2021 | cronos | Next target, not physically qualified here |

These are MT8163 models listed by the
[LineageOS maintainer](https://github.com/amazon-oss/local_manifests/tree/lineage-18.1)
and [amonet maintainer](https://github.com/R0rt1z2/amonet).
Do not substitute a Show 8 from 2021, Show 5 from 2023, or an image for another
codename, even if the enclosure looks similar.

**Already on LineageOS 18.1:** use the application preflight below with an
already authorized TLS ADB or USB connection. The tool does not pair, connect,
root, reboot, change permissions or launch any application.

**Still on Amazon FireOS:** stop before deployment. Verify the physical model
and firmware, read the maintainer's model-specific instructions, plan backup
and recovery, then obtain explicit approval for that device's unlock and ROM
replacement. This can erase data, remove Amazon functionality and brick the
device. The application installer intentionally refuses this path. For the
Show 5 2021 use only the
[cronos unlock instructions](https://xdaforums.com/t/unlock-root-twrp-unbrick-amazon-echo-show-5-2nd-gen-2021-cronos.4772596/)
and an independently verified cronos LineageOS image. This repository does not
download or flash a ROM. After a separately qualified LineageOS installation,
join the common application path. No second-device ROM installation is claimed.

## Build and preflight

Build requirements and commands are in [README.md](README.md). Run deployment
guard tests with Python 3.10+ (standard library only):

~~~bash
python3 -m unittest discover -s android/tests -v
python3 android/deploy.py --help
bash android/install.sh --help
~~~

Select one reviewed APK. Keep its checksum and signer fingerprint in a trusted
local release record. Inspect with the Android SDK, then compare the signer
against the trusted builder, not just the APK being inspected:

~~~bash
sha256sum /path/to/receiver.apk
"$ANDROID_HOME/build-tools/36.0.0/apksigner" verify --print-certs /path/to/receiver.apk
python3 android/deploy.py --profile cronos \
  --serial AUTHORIZED_TLS_ENDPOINT_OR_USB_SERIAL \
  --expected-device VERIFIED_PHYSICAL_SERIAL \
  --apk /path/to/receiver.apk \
  --apk-sha256 TRUSTED_APK_SHA256 \
  --signer-sha256 TRUSTED_SIGNER_SHA256
~~~

Read-only is the default. It verifies the package, APK and signer, physical
serial, codename, LineageOS 18.1/API 30, ARM ABI and non-root ADB. If the same
APK is already installed, even `--apply` does nothing. If another APK exists,
it refuses: prepare a reviewed backup/upgrade instead. Split APK layouts and
legacy unencrypted network ADB on port 5555 are refused. Existing applications,
preferences, playback and volume are not touched by preflight.

Only when the target is ready and OpenChromecast is absent, repeat the same
command with `--apply`. The installed public APK checksum is read back; the app
is not started and no configuration is silently copied. This does not prevent
Android's own package-manager broadcasts. Configure the device locally next.

## Per-device setup

1. Open OpenChromecast and choose a unique friendly name. Let the app generate
   its own persistent UUID and YouTube Lounge state. Do not clone preferences,
   `userdata`, `boot`, `vendor`, `persist`, Home Assistant tokens or keystores.
2. Provision only explicitly authorized Cast credentials using a reviewed
   private process and [LEGACY_BUNDLE.md](LEGACY_BUNDLE.md). The APK defaults
   to self-signed authentication; official Google applications can reject it.
   Credentials are deliberately not in Git, CI artifacts or a public installer.
   A third-party shared legacy bundle is not a unique device identity: its
   license/reuse rights must be resolved separately, it can be revoked, and
   TLS-only signatures do not implement fresh nonce signing. A successful
   test on one Echo does not make this a certified or universal receiver.
3. Start the receiver explicitly. Enable automatic start only if wanted.
   For automatic video, opt in and grant the special native Android permission.
   Audio leaves the dashboard visible; video should return to it on stop/end.
4. Install/enroll Home Assistant Companion separately for this device. Discover
   this unique receiver through HA's native Google Cast integration. If Music
   Assistant supplies announcements, set only this player's announcement volume
   strategy to `none` when announcements must share the unchanged media level.
   Alarm and notification streams remain separate on Android by design.
5. Keep authorized administration temporary. Check this ROM's behavior after a
   separately approved reboot: some builds reopen legacy ADB. Do not assume
   application autostart also secures the operating system's debugging policy.

Optional on-demand voice button (0.1.15+): enable **Bouton micro flottant** in
OpenChromecast settings, separately from automatic video. It uses the native
overlay permission and launches Companion, not a microphone in this app. Move
it by dragging to an edge. Verify that it remains above the dashboard and a
video, survives Cast stop/start, and does not launch Assist while dragged.
Agree with the owner before an actual microphone test. Then tap and check the
preferred Assist pipeline, transcription, spoken reply and return to the prior
screen. A missing or unregistered Companion must not be treated as a successful
voice test. The switch or notification Hide action disables the button; neither
clears credentials nor stops Cast. This does not enable wake-word listening.

## Acceptance and recovery

Record the APK/source/signer, device profile, unique name, firmware, results and
private provisioning reference without exporting keys. Check discovery and
HA state; then test audio, video, stop/end/error dashboard return, official
YouTube phone connection and shared volume from phone, physical buttons and
HA announcements. Agree on test volume before playback. Separately validate
autostart, secure administration, restore and certificate window rollover.

The current crown deployment (0.1.14/code 15) has owner-confirmed official
YouTube playback and phone volume, plus device-readback tests for Cast, local
buttons and HA announcements. It used separately provisioned private legacy
credentials. This does not qualify a fresh cronos deployment, current-version
reboot/restore, other Google senders, DRM or multiroom.

For recovery, stop only this application and restore the reviewed APK/config
from that device's backup plan. Never uninstall or clear data as an implicit
rollback, and never copy another device's full data or keys. Keep a pre-upgrade
copy until that operation's functional and data checks pass; deleting it is not
evidence that restoration was tested.

## GitHub build artifacts

The separate Android workflow tests guards, Rust and Java, lints, builds ARMv7
and ARM64 and uploads a test APK plus SHA256 and signer fingerprints. It does
not contact any Echo or include credentials. A new workflow is not proof of a
successful run; inspect its checks for the exact commit.

CI debug signing is ephemeral, so these artifacts are for fresh test installs,
not upgrades of an Echo signed with a different local key. Stable upgrade
releases need a separately protected persistent signing key and a documented
release process. No private debug or production key is committed here.
