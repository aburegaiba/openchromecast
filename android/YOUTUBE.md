# Native YouTube Provider

The Android receiver embeds the MIT Vibecast YouTube provider at revision
`4cde20b3f49b296debec8cb9a2badb51c7bae940`. It handles YouTube's MDX/Lounge
session protocol and resolves media natively, without Python, Node, yt-dlp,
another receiver or a signature server. Its license is included in APK assets
as `THIRD_PARTY_NOTICES.txt`.

## Architecture

`youtube_host.rs` adapts the provider to the existing Cast transport, bounded
player bridge and Media3 host. It announces only the implemented app ID
`233637DE`, alongside Default Media Receiver. Other Android web apps remain
unavailable. New app sessions cancel the old provider and invalidate its player
commands. The private Lounge identity is stable in `no_backup/youtube`; never
export it in diagnostics or commit it. SponsorBlock is disabled by default.

Media3 receives the provider's inline DASH manifest in memory and HTTP request
headers. No external host is needed to serve the manifest. The adapter keeps
the provider's XML intact except parsed AdaptationSet ID attributes: its textual
IDs are mapped to unique integers for Media3's DASH parser. Media URLs and their
escaping are preserved; this is not a regex replacement over arbitrary XML.
Native commands keep
the same generation guard as ordinary media loads. Cast app STOP cancels the
provider; closing one Cast socket does not terminate Lounge playback. Sender
channels are bounded and detached on disconnect so they cannot keep a socket's
writer alive.

## Boundaries

This code does not obtain a Google-trusted Cast identity, bypass device
authentication or implement DIAL, YouTube TV codes, screen mirroring or every
Google Cast web app. By default it generates a self-signed identity; an optional
private legacy bundle supplies cached TLS-only signatures, not a fresh
nonce-bound signature. See `README.md` for that mode's limits. Official senders
may still reject either mode before MDX runs. Playback via a permissive test
client is not proof that the official YouTube phone app accepts the device.

Video uses the existing activity/notification behavior. Background takeover and
automatic return to Home Assistant are not certified by this integration.
Google-protected subscription playback and DRM are not claimed. The current
conservative capabilities request H.264/AAC at up to 720p.

## Verification

Run native unit/protocol tests, Java unit tests and Android lint before building
both ARM ABIs. Then validate on the physical device with media volume zero:
app availability, launch, real MDX reply without exporting values, public
YouTube LOAD and advancing Media3 position, pause/seek/stop, app switch,
unchanged receiver identity/startup preferences and Home Assistant discovery.
Test the official phone application separately; leave that requirement open
until actual client connection and playback are verified.

Version 0.1.12 accepts the standard `appId` array in availability queries and
answers `GET_DEVICE_INFO` with its actual identity and audio/video capabilities.
Android registers Default Media Receiver and YouTube DNS-SD subtypes on the
same instance. These are discovery fixes, not proof of official sender support.

On the tested ARMv7 Android 11 device, the 0.1.5 prototype completed MDX,
native YouTube LOAD, autoplay false, advancing playback, pause, seek, resume
and STOP. A separate casttube Lounge setPlaylist also triggered playback without
a Cast media LOAD. Video frames were nonblank and moving after explicitly
opening the existing video screen. Switching back to Default Media Receiver
and playing a synthetic HTTP WAV succeeded. Tests were muted, with no phone
acceptance or automatic background video takeover claim.

The 0.1.6 prototype also sends receiver-state changes to passive Cast clients
and media-state changes only to connections bound to the current transport.
This observer reads local memory once per second; it adds no cloud polling.
It is cancelled on socket close, and preserves Cast's incremental frame reader.
Two sockets using the same sender ID no longer overwrite each other's MDX
channel. Home Assistant observed playback started by another client, and its
pause/play/stop/turn-off actions were independently confirmed by the receiver.
