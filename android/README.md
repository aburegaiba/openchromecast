# Experimental Android receiver

Local Android host for openchromecast's existing Rust Cast V2 implementation.
Media3 ExoPlayer owns audio/video playback; Android NSD owns `_googlecast._tcp`
discovery. This is a prototype, not a Google-certified Chromecast.

## Scope and limits

- Default Media Receiver (`CC1AD845`), HTTP/HTTPS media, play/pause/seek/stop,
  software volume/mute, and status from the real player.
- Audio stays in a foreground service while another app remains visible.
- The return button supports Home Assistant's regular and Minimal packages.
- Video opens from the receiver screen or its notification. No overlay,
  accessibility, microphone, storage, root or Google account permission.
- Stable discovery UUID in app-private preferences, no shared device identity.
- Self-signed device authentication is unchanged. Google SDK applications may
  reject it even when discovery and Home Assistant control work. No YouTube,
  Netflix/DRM, screen mirroring, multiroom or universal sender support claim.
- Explicit start only, no boot receiver. No system-volume change. Initial
  software volume is 15 percent of the existing device media volume.
- Targeted at Android 11 Echo hardware first. Other Android versions, long
  uptime beyond the generated TLS certificate lifetime and physical A/V remain
  unqualified until tested; installation alone is not acceptance.

No desktop tray, mpv, VLC executable or server-side relay is shipped in the APK.
The launcher icon is converted from the upstream MIT-licensed `assets/icon.ico`.
Android JNI links the same protocol sources, rather than a second hand-written
Cast protocol implementation. Desktop modules remain available on desktop.

## Build

Requirements: user Rust with `armv7-linux-androideabi` and
`aarch64-linux-android` targets, SDK 36, NDK 27+, JDK 21, Gradle 9.2.1.
Set `ANDROID_HOME`, `ANDROID_NDK_HOME`, `JAVA_HOME` and optionally `GRADLE`.

```bash
cargo test --manifest-path android/native/Cargo.toml --locked --lib
python3 android/tests/test_install.py -v
bash android/build.sh --help
bash android/build.sh --abi all
bash android/install.sh --help
```

The native lockfile is committed. Host SDK paths, caches, private signing keys
and generated libraries/APKs are ignored. Debug builds use the local Android
debug signing identity; do not treat it as a production release key.

Installation uses the existing authorized TLS ADB connection and an expected
physical serial. `install.sh` is read-only unless `--apply` is explicit. It
checks package/API/ABI before installing, and never starts/stops applications.
Review the APK signing certificate with the SDK's `apksigner` before applying.

## Acceptance and rollback

1. Verify the exact device SDK/ABI/serial, APK SHA256, package and signing
   certificate. Install `org.openchromecast.receiver` without touching existing
   apps. Stop any conflicting receiver only with the owner's approval.
2. Start explicitly. Verify no startup crash, TCP 8009 and genuine mDNS TXT
   identity, then test Cast GET_STATUS using a compatible sender.
3. Test a non-private audio file at an agreed low volume. Verify real position,
   pause/resume/seek/stop, not just a success response. Ask for audible result.
4. Test video from the notification and return to the prior app; separately
   verify a real phone Cast picker. A self-signed rejection is a known limit,
   not a reason to import another application's device credentials.
5. Stop from the UI or notification. Service destruction withdraws discovery,
   releases player/network resources and stops Rust. Restart the previous
   receiver if it had been stopped. Do not uninstall or clear data implicitly.

### Android 11 hardware check

The 0.1.1 prototype has been installed on ARMv7 Android 11. Its UI and native
library loaded, and the return button opened the installed Companion Minimal
package. A compatible Cast client prepared a 10-second synthetic WAV, played,
paused, sought, resumed and stopped it. The owner confirmed the short tones at
50 percent software volume with Companion in the foreground. This is an audio
acceptance result, not a Google SDK, phone-video, DRM or multiroom certification.

Android NSD currently announces the device's IPv6 addresses as well as IPv4,
while the native listener is IPv4-only. IPv6 connections remain unqualified and
must be addressed before claiming reliable discovery across all senders. Do not
enable additional globally reachable listeners or change firewall rules silently.
