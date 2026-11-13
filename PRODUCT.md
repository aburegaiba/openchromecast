# OpenChromecast Android host
<!-- impeccable:product-schema 1 -->

## Platform
android

## Users
Owners of Android tablets and Echo screens already displaying Home Assistant.

## Product Purpose
Receive local Cast media alongside the existing dashboard application.

## Stack
Existing Rust Cast implementation, Android Views/Material controls, Media3.

## Capabilities and Constraints
Keep the existing Home Assistant application and other installed apps intact.
The audio/video prototype and sender compatibility must be verified separately
from installation. Temporary handover from another receiver and audible tests
need explicit approval. No DRM or complete Cast compatibility claim.

## Operating Context
Local network; explicit start/stop and Android notification. No server-side
media relay, account credentials, root or microphone. Optional boot resume is
explicitly enabled in receiver settings; it never opens another screen.
Optional automatic video presentation uses Android's special display
permission, granted through system settings. Video presentation itself does
not draw overlay windows. A separate opt-in floating Assist shortcut can use
that permission, independently of Cast playback. It launches Companion's
preferred Assist pipeline on an explicit tap, without microphone permission
or capture in this application. It hides on locked/screen-off devices.
The player task closes on STOP/end/error, revealing the previous application.
