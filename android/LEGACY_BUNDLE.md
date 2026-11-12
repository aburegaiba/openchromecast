# Explicit legacy certificate bundles

This experimental mode accepts an authorized, externally supplied bundle in
`no_backup/identity/legacy-bundle.json`. The APK contains no bundle and performs
no download, provisioning, signing-server request or automatic import. Default
self-signed and per-device PEM modes remain available. Never package private
credentials in a repository, APK, Android shared storage or command-line argument.

The JSON format is the one parsed by the pinned MIT `vibecast-security` library:
`cpu` and `ica` contain device/intermediate certificate PEM strings; `certs` is
a list of `pu` (TLS certificate PEM), `pr` (matching RSA private key PEM),
`sig_sha1` and `sig_sha256` (base64 cached signatures over that exact TLS DER).
No credential example is included. Optional `crl` is preserved if supplied;
loading does not establish revocation status or Google's trust.

The app requires application-owned 0700 directories and a regular, single-link,
0600 file, at most 32 MiB. PEM credentials alongside the bundle are rejected as
ambiguous. Existing invalid or incomplete configuration fails closed. Only an
absent identity directory keeps the default. Up to 4096 windows and 64 KiB per
field are accepted. At import, validate provenance, an independently trusted
artifact pin, Google's public roots and reuse rights outside the app. The loader
checks all cached signatures, TLS key matches and maximum four-day TLS windows.

For a controlled import, stage a new directory inside the app's private
`no_backup` parent, transfer bytes on stdin over an authorized administration
channel, verify hash/owner/modes, then rename without replacing an existing
identity. Restart only the receiver service. Do not copy another device's data,
Google account, Home Assistant registration or anonymous YouTube Lounge state.
Disabling this mode requires stopping the receiver and moving only this new
identity directory aside, preserving it privately; never clear application data.

## Rotation and authentication

Each TLS connection retains one immutable certificate and corresponding auth
response. Periodic checks select a currently valid local window, with an
exclusive expiration boundary. A connection arriving after expiration selects
the next valid window immediately rather than waiting for the next minute.
Overlapping windows prefer the one with the latest expiration. Clock jumps
outside all stored windows refuse new connections. Playback state is independent
of rotation and no remote service is consulted. Existing TLS connections keep
their old snapshot; they are not interrupted to switch certificates.

Legacy responses choose SHA-1 or SHA-256 as requested and deliberately omit
`sender_nonce`: no device-auth signing key is supplied by this format. They do
not pretend to sign a fresh nonce. A recognized certificate chain and valid
TLS-only signature do not establish modern nonce-bound authentication, Google
certification, revocation status or compatibility with official YouTube/other
applications. Shared identities may be revoked and stored future windows are
not a guarantee of future acceptance. A third-party bundle is not licensed by
this repository's MIT source license.

Host tests use newly generated fictitious keys only. They cover cryptographic
binding, exact boundaries, wrong keys/signatures, file privacy/ambiguity and a
real loopback TLS rotation with an already-active simulated player and retained
old connection. Test official senders separately on the actual device. No
system clock adjustment or real multi-day acceptance is claimed by these tests.
