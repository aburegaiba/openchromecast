//! YouTube-specific namespace: `urn:x-cast:com.google.youtube.mdx`.
//!
//! When the sender launches the `YouTube` receiver app, it uses this namespace
//! for playback-queue / remote-control style commands. This is the least
//! documented part of the protocol and is still being reverse-engineered:
//! unknown message types are logged verbatim so captures can be analyzed.

#[cfg(feature = "embedded-player")]
#[path = "../android/native/src/youtube_host.rs"]
mod native;
#[cfg(feature = "embedded-player")]
pub use native::Host;

use crate::server::{MessageSink, Shared};
#[cfg(not(feature = "embedded-player"))]
use crate::server::{NS_YOUTUBE, payload_json, send_json};
use anyhow::Result;
#[cfg(not(feature = "embedded-player"))]
use serde_json::json;
#[cfg(not(feature = "embedded-player"))]
use tracing::{info, warn};

pub async fn handle(
    msg: &crate::proto::cast_channel::CastMessage,
    tx: &MessageSink,
    shared: &Shared,
) -> Result<()> {
    #[cfg(feature = "embedded-player")]
    {
        if let Some(host) = &shared.youtube {
            host.handle_mdx(msg, tx).await?;
        }
        Ok(())
    }
    #[cfg(not(feature = "embedded-player"))]
    legacy_handle(msg, tx, shared).await
}

#[cfg(not(feature = "embedded-player"))]
async fn legacy_handle(
    msg: &crate::proto::cast_channel::CastMessage,
    tx: &MessageSink,
    shared: &Shared,
) -> Result<()> {
    let payload = payload_json(msg)?;
    let kind = payload
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let rid = payload.get("requestId").cloned().unwrap_or(json!(0));

    match kind.as_str() {
        "getMdxSessionStatus" => {
            let device_id = shared
                .state
                .lock()
                .await
                .session
                .as_ref()
                .map(|s| s.id.clone());
            let resp = json!({
                "type": "getMdxSessionStatusResponse",
                "requestId": rid,
                "data": {
                    "playlist": [],
                    "deviceId": device_id,
                    "deviceModel": "Chromecast Ultra",
                    "deviceType": "REMOTE_CONTROL",
                    "castAppUrl": "https://www.youtube.com/tv",
                    "capabilities": {
                        "canReceivePlaylistUpdates": true,
                        "supportsQueueing": true,
                        "supportsRecentVideo": true,
                        "supportsWatchLater": true,
                    }
                }
            });
            send_json(tx, "receiver-0", &msg.source_id, NS_YOUTUBE, &resp).await?;
        }
        // Acknowledged implicitly; log the payload for reverse engineering.
        "clearPlaylist" | "setPlaylist" | "addVideo" | "removeVideo" | "playVideo"
        | "playlistChanged" | "getQueue" | "removePlaylist" | "queueNext" | "queuePrevious" => {
            info!("[youtube.mdx] {kind}: {payload}");
        }
        other => {
            warn!("[youtube.mdx] unhandled message type: {other} ({payload})");
        }
    }
    Ok(())
}
