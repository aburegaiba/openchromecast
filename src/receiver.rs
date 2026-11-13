//! Receiver control namespace: `urn:x-cast:com.google.cast.receiver`.
//!
//! Handles `GET_STATUS`, `GET_APP_AVAILABILITY`, `LAUNCH`, `STOP`, `SET_VOLUME`.

use crate::server::{
    MessageSink, NS_DISCOVERY, NS_MEDIA, NS_RECEIVER, NS_YOUTUBE, Shared, payload_json, send_json,
};
use crate::state::{AppState, Session};
use anyhow::Result;
use serde_json::{Value, json};
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Clone)]
pub struct DiscoveryInfo {
    pub id: Uuid,
    pub name: String,
    pub model: String,
    pub capabilities: u32,
}

pub async fn handle_discovery(
    msg: &crate::proto::cast_channel::CastMessage,
    tx: &MessageSink,
    shared: &Shared,
) -> Result<()> {
    let payload = payload_json(msg)?;
    if payload["type"] != "GET_DEVICE_INFO" {
        return Ok(());
    }
    if let Some(info) = &shared.discovery {
        let rid = payload.get("requestId").cloned().unwrap_or(json!(0));
        send_json(
            tx,
            "receiver-0",
            &msg.source_id,
            NS_DISCOVERY,
            &json!({
                "type":"DEVICE_INFO", "requestId":rid,
                "deviceId":info.id.simple().to_string(), "deviceModel":info.model,
                "friendlyName":info.name, "deviceCapabilities":info.capabilities,
                "deviceIconUrl":"", "controlNotifications":1,
                "receiverMetricsId":"", "wifiProximityId":""
            }),
        )
        .await?;
    }
    Ok(())
}

pub async fn handle(
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
        "GET_STATUS" => {
            let st = shared.state.lock().await;
            let status = receiver_status(&st);
            send_json(
                tx,
                "receiver-0",
                &msg.source_id,
                NS_RECEIVER,
                &json!({ "requestId": rid, "status": status, "type": "RECEIVER_STATUS" }),
            )
            .await?;
        }
        "GET_APP_AVAILABILITY" => {
            let app_ids = app_ids_from(&payload);
            let availability: Value = app_ids
                .iter()
                .map(|a| {
                    (
                        a.as_str(),
                        if available(a, shared) {
                            "APP_AVAILABLE"
                        } else {
                            "APP_UNAVAILABLE"
                        },
                    )
                })
                .collect();
            info!("app availability requested: {app_ids:?}");
            send_json(
                tx,
                "receiver-0",
                &msg.source_id,
                NS_RECEIVER,
                &json!({ "requestId": rid, "availability": availability,
                    "type": "GET_APP_AVAILABILITY", "responseType": "GET_APP_AVAILABILITY" }),
            )
            .await?;
        }
        "LAUNCH" => {
            let app_id = payload
                .get("appId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !available(&app_id, shared) {
                send_json(
                    tx,
                    "receiver-0",
                    &msg.source_id,
                    NS_RECEIVER,
                    &json!({"type":"LAUNCH_ERROR", "requestId":rid, "reason":"NOT_FOUND"}),
                )
                .await?;
                return Ok(());
            }
            let session = {
                let mut st = shared.state.lock().await;
                if app_id == "233637DE" && st.session.as_ref().is_some_and(|s| s.app_id == app_id) {
                    st.session.clone().unwrap()
                } else {
                    st.media_session_counter += 1;
                    let session = Session::new(app_id.clone(), st.media_session_counter);
                    // A fresh session starts with no bound connections.
                    st.session_connections.clear();
                    st.session = Some(session.clone());
                    session
                }
            };
            #[cfg(feature = "embedded-player")]
            if let Some(host) = &shared.youtube {
                host.launch(&session).await?;
            }
            info!("launched app '{app_id}' session {}", session.id);
            let st = shared.state.lock().await;
            let status = receiver_status(&st);
            send_json(
                tx,
                "receiver-0",
                &msg.source_id,
                NS_RECEIVER,
                &json!({ "requestId": rid, "status": status, "type": "RECEIVER_STATUS" }),
            )
            .await?;
        }
        "STOP" => {
            info!("stopping current app");
            #[cfg(feature = "embedded-player")]
            if let Some(host) = &shared.youtube {
                host.stop().await;
            }
            let _ = shared.player.stop().await;
            let mut st = shared.state.lock().await;
            st.session = None;
            st.session_connections.clear();
            drop(st);
            let st = shared.state.lock().await;
            let status = receiver_status(&st);
            send_json(
                tx,
                "receiver-0",
                &msg.source_id,
                NS_RECEIVER,
                &json!({ "requestId": rid, "status": status, "type": "RECEIVER_STATUS" }),
            )
            .await?;
        }
        "SET_VOLUME" => {
            let vol = payload.get("volume");
            let mut st = shared.state.lock().await;
            if let Some(l) = vol.and_then(|v| v.get("level")).and_then(|v| v.as_f64()) {
                st.volume = l as f32;
            }
            if let Some(m) = vol.and_then(|v| v.get("muted")).and_then(|v| v.as_bool()) {
                st.muted = m;
            }
            drop(st);

            if let Some(level) = vol.and_then(|v| v.get("level")).and_then(|v| v.as_f64()) {
                let _ = shared.player.set_volume(level as f32).await;
            }
            if let Some(muted) = vol.and_then(|v| v.get("muted")).and_then(|v| v.as_bool()) {
                let _ = shared.player.set_mute(muted).await;
            }

            let st = shared.state.lock().await;
            let status = receiver_status(&st);
            send_json(
                tx,
                "receiver-0",
                &msg.source_id,
                NS_RECEIVER,
                &json!({ "requestId": rid, "status": status, "type": "RECEIVER_STATUS" }),
            )
            .await?;
        }
        other => warn!("unhandled receiver message type: {other}"),
    }
    Ok(())
}

/// `GET_APP_AVAILABILITY` may carry `appId` (string) or `appIds` (array).
fn supports_app(app_id: &str, embedded: bool) -> bool {
    // The Android prototype implements Default Media Receiver, not arbitrary web apps.
    !embedded || app_id == "CC1AD845"
}

fn available(app_id: &str, shared: &Shared) -> bool {
    #[cfg(feature = "embedded-player")]
    if app_id == "233637DE" {
        return shared.youtube.is_some();
    }
    let _ = shared;
    supports_app(app_id, cfg!(feature = "embedded-player"))
}

fn app_ids_from(payload: &Value) -> Vec<String> {
    let ids = payload.get("appId").or_else(|| payload.get("appIds"));
    if let Some(s) = ids.and_then(|v| v.as_str()) {
        vec![s.to_string()]
    } else if let Some(arr) = ids.and_then(|v| v.as_array()) {
        arr.iter()
            .filter_map(|v| v.as_str())
            .map(|s| s.to_string())
            .collect()
    } else {
        vec![]
    }
}

/// Build the `RECEIVER_STATUS.status` object from the shared state.
pub(crate) fn receiver_status(state: &AppState) -> Value {
    let apps = match &state.session {
        Some(s) => json!([{
            "appId": s.app_id,
            "displayName": s.display_name,
            "isIdleScreen": false,
            "namespaces": s.namespaces.iter().map(|n| json!({"name": n})).collect::<Vec<_>>(),
            "sessionId": s.id,
            "statusText": s.status_text,
            "transportId": s.id,
        }]),
        None => json!([]),
    };
    json!({
        "activeInput": "input1",
        "applications": apps,
        "standBy": "no",
        "userEq": { "high_shelf": 0.0, "low_shelf": 0.0 },
        "volume": {
            "controlType": "attenuation",
            "level": state.volume,
            "muted": state.muted,
            "stepInterval": state.volume_step
        }
    })
}

impl Session {
    pub(crate) fn new(app_id: String, media_session_id: u32) -> Self {
        let id = Uuid::new_v4().simple().to_string();
        let (display_name, namespaces) = namespaces_for(&app_id);
        Session {
            id,
            app_id,
            display_name,
            namespaces,
            status_text: "Ready to cast".to_string(),
            media_session_id,
            media: None,
            queue: Vec::new(),
            queue_index: 0,
            sequence_number: None,
            active_track_ids: None,
            credentials: None,
        }
    }
}

/// Known receiver apps and the namespaces they announce.
fn namespaces_for(app_id: &str) -> (String, Vec<String>) {
    match app_id {
        "233637DE" | "YouTube" | "youtube" => {
            ("YouTube".into(), vec![NS_MEDIA.into(), NS_YOUTUBE.into()])
        }
        "CC1AD845" => ("Default Media Receiver".into(), vec![NS_MEDIA.into()]),
        _ => (app_id.to_string(), vec![NS_MEDIA.into(), NS_YOUTUBE.into()]),
    }
}

#[cfg(test)]
mod embedded_tests {
    use super::*;

    #[test]
    fn availability_accepts_the_standard_app_id_array_and_legacy_forms() {
        for value in [
            json!({"appId":["CC1AD845","233637DE"]}),
            json!({"appIds":["CC1AD845","233637DE"]}),
        ] {
            assert_eq!(app_ids_from(&value), vec!["CC1AD845", "233637DE"]);
        }
        assert_eq!(app_ids_from(&json!({"appId":"CC1AD845"})), vec!["CC1AD845"]);
        assert!(app_ids_from(&json!({"appId":42})).is_empty());
    }
    #[test]
    fn android_does_not_claim_unimplemented_web_receivers() {
        assert!(supports_app("CC1AD845", true));
        assert!(!supports_app("YouTube", true));
        assert!(!supports_app("233637DE", true));
        assert!(supports_app("historical-desktop-app", false));
    }
}
