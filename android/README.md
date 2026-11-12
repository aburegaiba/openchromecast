# Experimental Android receiver

Local Android host for openchromecast's existing Rust Cast V2 implementation.
Media3 ExoPlayer owns audio/video playback; Android NSD owns `_googlecast._tcp`
discovery. This is a prototype, not a Google-certified Chromecast.

## Scope and limits

- Default Media Receiver (`CC1AD845`), HTTP/HTTPS media, play/pause/seek/stop,
  software volume/mute, and status from the real player.
- Native YouTube app (`233637DE`) through the pinned MIT Vibecast provider:
  MDX/Lounge and public video resolution, rendered by Media3. See
  [the integration and its boundaries](YOUTUBE.md). No separate server is needed.
- Audio stays in a foreground service while another app remains visible.
- The return button supports Home Assistant's regular and Minimal packages.
- Video opens from the receiver screen or its notification. Optional automatic
  video presentation is disabled by default and requires Android's special
  display-over-other-apps permission, granted through system settings. No
  overlay window is drawn. The private player activity has its own task;
  STOP, end and player errors remove it and reveal the previous application.
  No accessibility, microphone, storage, root or Google account permission.
- Stable discovery UUID in app-private preferences, no shared device identity.
- Read-only device metadata on LAN TCP 8008 at `/setup/eureka_info`, sharing
  the NSD name and UUID. Other paths return 404 and write methods return 405;
  no setup, reboot, account or YouTube application API is exposed. This allows
  compatible discovery clients to identify the receiver without claiming
  Google certification. The Cast listener remains on TCP 8009.
- Self-signed device authentication remains the default. An explicitly imported
  private legacy bundle can supply cached TLS-only signatures, with local window
  rotation; see [the opt-in and its trust limits](LEGACY_BUNDLE.md). Google SDK applications may
  still reject it even when discovery and Home Assistant control work. No official
  YouTube phone acceptance, Netflix/DRM, screen mirroring, multiroom or universal
  sender support claim.
- Optional automatic boot resume, disabled by default. It resumes only after
  the switch is enabled and the receiver has been explicitly started; stopping
  from the screen or notification prevents the next resume. The boot receiver
  never opens an activity or takes over Home Assistant. No system-volume change. Initial
  software volume defaults to 15 percent of the existing device media volume;
  later sender volume/mute choices are persisted without changing system volume.
- Targeted at Android 11 Echo hardware first. Other Android versions, real
  multi-day certificate rotation and broader physical A/V remain unqualified
  until tested; installation alone is not acceptance.

No desktop tray, mpv, VLC executable or server-side relay is shipped in the APK.
The launcher icon is converted from the upstream MIT-licensed `assets/icon.ico`.
Android JNI links the same protocol sources, rather than a second hand-written
Cast protocol implementation. Desktop modules remain available on desktop.

## Build

Version 0.1.11 adds opt-in legacy bundle loading and local certificate-window
rotation using the pinned MIT Vibecast security parser. It never fetches or
ships credentials, never calls a remote signer and does not require another
Cast device. Existing connections retain their matching TLS/auth snapshot.
Expiration triggers selection from locally stored windows, including on a new
connection before the periodic timer fires; no valid window means refusal,
not a self-signed fallback. Synthetic tests cover exact expiration, backwards
clock jumps, invalid signatures/keys and active simulated playback across rotation.
Actual official-sender acceptance and real multi-day operation must be checked
separately. No claim of nonce-bound authentication is made for a legacy bundle.

Version 0.1.8 added optional per-device PEM loading from the app-private
`no_backup/identity/` directory: `certificate.pem` contains the leaf and
intermediates, and `key.pem` is its matching RSA PKCS#8 or PKCS#1 key.
Directories must be application-owned mode 0700, and regular single-link files
mode 0600, at most 64 KiB each. Symlinks and incomplete/invalid configuration
are refused; only an absent identity directory keeps the self-signed default.
Stage authorized material through a reviewed private provisioning process,
never terminal output, shared storage or a repository. Android backup excludes
this data. This loader does not obtain credentials, unwrap Widevine keys, import
precomputed signature bundles or validate Google's trust chain. Validate trust
and a real sender separately. This supplied-identity mode is host-tested with
synthetic keys, not qualified with real device credentials. The absent-directory
self-signed default was checked on Android in 0.1.9.

Version 0.1.9 renews short-lived TLS material locally, checking every minute
with a 24-hour renewal margin. Supplied device-auth keys and full certificate
chains stay unchanged; default self-signed mode refreshes its ephemeral auth
and TLS material together. Existing connections retain the matching immutable
TLS/auth snapshot, while new connections use the new certificate. Renewal does
not stop the player, contact a provisioning server, renew a device-auth
certificate or establish Google trust. Invalid TLS material is not served while
renewal is pending. Clock-jump and two-epoch loopback tests use synthetic keys
and an injected clock; they do not prove real multi-day uptime.

The 0.1.9 Android check passed silent audio, automatic synthetic video and
dashboard return on STOP, end and player error. Native YouTube started, but the
separate Home Assistant control recipe timed out waiting for its next command;
those controls remain qualified on 0.1.7 only. That partial recipe also found
the Android media volume unexpectedly nonzero at cleanup. It was restored to
zero/muted; the cause is unresolved, so further media tests must not rely on
the initial mute reading alone. No official phone, fresh boot or restore claim.

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
4. Test video from the notification and return to the prior app. For a kiosk,
   explicitly enable automatic video and grant the native Android permission,
   then put the dashboard in front. Verify video takeover without an ADB/UI
   launch, STOP/end/error returning to that task, audio leaving it visible,
   and disabled/revoked permission falling back to the notification. Back must
   not cause the same media to repeatedly reopen. Separately verify a real
   phone Cast picker. A self-signed rejection is a known limit, not a reason to
   import another application's device credentials.
5. Stop from the UI or notification. Service destruction withdraws discovery,
   releases player/network resources and stops Rust. Restart the previous
   receiver if it had been stopped. Do not uninstall or clear data implicitly.
6. For unattended devices, enable automatic start and start the receiver, then
   reboot normally. Verify a foreground receiver service, the same discovery
   UUID and a real Cast connection without opening this app. A force-stopped app
   still requires an explicit user launch before Android delivers boot events.

The idle service uses the `connectedDevice` foreground type. It does not launch
a `mediaPlayback` service directly from `BOOT_COMPLETED`; this matters on
Android 15 and later. Versions other than the tested Echo Android 11 remain
unqualified. Automatic video is a separate opt-in and never runs from the
boot receiver. Disable its switch and revoke the special permission in Android
settings to return to notification-only background video.

### Android 11 hardware check

The 0.1.1 prototype has been installed on ARMv7 Android 11. Its UI and native
library loaded, and the return button opened the installed Companion Minimal
package. A compatible Cast client prepared a 10-second synthetic WAV, played,
paused, sought, resumed and stopped it. The owner confirmed the short tones at
50 percent software volume with Companion in the foreground. This is an audio
acceptance result, not a Google SDK, phone-video, DRM or multiroom certification.

Version 0.1.10 adds dual-stack listeners on the existing Cast and metadata ports.
IPv4 behavior is preserved. IPv6 loopback, link-local peers and peers in the
active Android interface's /64-or-narrower prefixes are accepted; other IPv6
peers are closed before TLS or HTTP. Prefixes come from LinkProperties, not a
static subnet list. Missing/invalid updates revoke old prefixes, and late events
for a previous default network are ignored. This is an application accept policy,
not a kernel packet firewall: TCP handshakes can still complete before rejection.
It neither supplies Google credentials nor changes the router's firewall. Host
tests cover both address families and deny-before-protocol behavior. On Android
11 hardware, metadata and pinned public device-auth passed over IPv4, both global
IPv6 addresses and scoped link-local IPv6. Nonexistent paths and write methods
still returned 404/405. mDNS exposed the same UUID on one IPv4 and three IPv6
addresses. Denial of non-local peers and network-transition ordering were tested
on the host, not from an Internet client or by changing the device's Wi-Fi.
No media, reboot or restore recipe was repeated for this version. Google's root
trust still fails for the generated identity; this is not phone acceptance.

The 0.1.2 prototype also passed a real Android 11 reboot with automatic start
enabled: the receiver resumed as a foreground service without opening its UI,
kept its discovery UUID, and left Companion visible. Android delivered the boot
event later than the first 35-second post-connection check; acceptance must wait
for actual boot broadcast delivery rather than treating that short timeout as
proof of a service failure. Device-auth nonce/signature still passed, while the
self-signed chain remained untrusted by Google Cast roots.

The 0.1.4 prototype was checked through Home Assistant's native Cast integration
on the same hardware. HTTP discovery returned the persistent NSD identity;
the initial software volume was correctly reported as 15 percent. Silent PCM
playback advanced, and STOP remained IDLE with position zero instead of
returning to a stale PLAYING state. The embedded STOP path no longer inherits
the desktop VLC debounce or suppresses authoritative terminal states; desktop
behavior is unchanged. TLS regression coverage checks that distinction.

A silent H.264/MP4 test also advanced and rendered a visible test pattern when
this application's screen was already open. STOP closed the video activity
and returned to this application's main screen; returning to Companion was
explicit. This does not validate automatic video takeover from background,
phone senders or YouTube. A further real reboot preserved the discovery UUID
and resumed the foreground receiver while Companion remained visible. Wait
for the actual service and listeners, not just the process PID, before checking
boot acceptance. Device-auth nonce/signature passed again, but the Google Cast
root trust check still failed as expected for the generated self-signed identity.

The 0.1.7 prototype was checked on the same Android 11 hardware with automatic
video explicitly enabled through the native permission screen. Audio left the
dashboard visible. Synthetic video opened without a launcher command, rendered
changing pixels, and returned to the dashboard on STOP, natural end and player
error. Back hid the player without interrupting playback or reopening it.
Disabled automatic video and revoked permission both prevented takeover.
Native YouTube Lounge playback also opened the video task; Home Assistant
pause/play/stop/turn-off commands were independently confirmed, including a
return to the same dashboard task. All tests used zero system/software volume.
The control surface was checked at normal and enlarged font size; settings
were restored. Other hardware, a fresh reboot of this version, restoration and
official Google phone acceptance remain unqualified. Public device-auth still
passes its nonce/signature checks but fails the Google trust-chain check.
