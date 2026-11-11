//! Native YouTube app adapter. Session secrets and signed URLs are never logged.
use crate::{
    player::{PlayerHandle, PlayerSnapshot, PlayerState, bridge::Bridge},
    proto::cast_channel::CastMessage,
    server::{MessageSink, NS_MEDIA, NS_YOUTUBE, Shared, payload_json, send_json},
    state::{AppState, MediaInfo, Session},
};
use anyhow::{Result, ensure};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::Mutex;
use vibecast_apps_youtube::YouTube;
use vibecast_sdk::{
    AppContext, AppProvider, AppSession, LaunchCredentials, LoadRequest, PlaybackController,
    PlaybackMedia, PlaybackState, ReceiverContext, SenderChannel, StreamSource,
};

pub const APP_ID: &str = "233637DE";
const MAX_SENDERS: usize = 8;

struct Channels {
    transport: String,
    senders: Mutex<HashMap<String, MessageSink>>,
}

impl Channels {
    async fn register(&self, id: &str, tx: &MessageSink) -> Result<()> {
        ensure!(!id.is_empty() && id.len() <= 128, "invalid sender");
        let mut senders = self.senders.lock().await;
        ensure!(
            senders.contains_key(id) || senders.len() < MAX_SENDERS,
            "sender limit reached"
        );
        senders.insert(id.to_owned(), tx.clone());
        Ok(())
    }
}

#[async_trait]
impl SenderChannel for Channels {
    async fn send_custom(&self, namespace: &str, value: Value) {
        if namespace != NS_YOUTUBE && namespace != NS_MEDIA {
            return;
        }
        let targets = self.senders.lock().await.clone();
        for (id, tx) in targets {
            let _ = tokio::time::timeout(
                Duration::from_secs(1),
                send_json(&tx, &self.transport, &id, namespace, &value),
            )
            .await;
        }
    }
    async fn broadcast_custom(&self, namespace: &str, value: Value) {
        self.send_custom(namespace, value).await;
    }
}

#[derive(Clone)]
struct Active {
    context: AppContext,
    session: Arc<dyn AppSession>,
    channels: Arc<Channels>,
    valid: Arc<AtomicBool>,
}

pub struct Host {
    receiver: ReceiverContext,
    http: reqwest::Client,
    state: Arc<Mutex<AppState>>,
    bridge: Arc<Bridge>,
    player: PlayerHandle,
    active: Mutex<Option<Active>>,
}

impl Host {
    pub fn new(
        receiver: ReceiverContext,
        state: Arc<Mutex<AppState>>,
        bridge: Arc<Bridge>,
        player: PlayerHandle,
    ) -> Result<Arc<Self>> {
        let metadata = std::fs::symlink_metadata(&receiver.data_dir)?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "invalid YouTube data directory"
        );
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .build()?;
        let host = Arc::new(Self {
            receiver,
            http,
            state,
            bridge,
            player,
            active: Mutex::new(None),
        });
        let weak = Arc::downgrade(&host);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(500));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                let Some(host) = weak.upgrade() else {
                    break;
                };
                let active = host.active.lock().await.clone();
                if let Some(active) = active {
                    let snapshot = host.player.snapshot().await;
                    if snapshot.state == PlayerState::Idle
                        && !snapshot.end_error
                        && host
                            .state
                            .lock()
                            .await
                            .session
                            .as_ref()
                            .is_some_and(|s| s.media.is_some())
                    {
                        continue;
                    }
                    active
                        .session
                        .on_playback_update(&active.context, playback_state(snapshot))
                        .await;
                    if active.valid.load(Ordering::Acquire)
                        && let Some(status) =
                            crate::media::player_status(&host.player, &host.state).await
                    {
                        active
                            .channels
                            .send_custom(
                                NS_MEDIA,
                                json!({"type":"MEDIA_STATUS", "status":[status]}),
                            )
                            .await;
                    }
                }
            }
        });
        Ok(host)
    }

    pub async fn launch(&self, session: &Session) -> Result<()> {
        let mut active = self.active.lock().await;
        if active
            .as_ref()
            .is_some_and(|a| a.context.session_id == session.id)
        {
            return Ok(());
        }
        if let Some(old) = active.take() {
            old.valid.store(false, Ordering::Release);
            old.channels.senders.lock().await.clear();
            old.session.on_stop(&old.context).await;
        }
        if session.app_id != APP_ID {
            return Ok(());
        }
        let channels = Arc::new(Channels {
            transport: session.id.clone(),
            senders: Mutex::new(HashMap::new()),
        });
        let valid = Arc::new(AtomicBool::new(true));
        let controller = Arc::new(Controller {
            id: session.id.clone(),
            valid: valid.clone(),
            state: self.state.clone(),
            bridge: self.bridge.clone(),
            player: self.player.clone(),
        });
        let context = AppContext::new(
            &session.id,
            &session.id,
            APP_ID,
            self.http.clone(),
            self.receiver.clone(),
            channels.clone(),
        )
        .with_playback_controller(controller);
        let session = YouTube::new()
            .launch(&context, LaunchCredentials::default())
            .await?;
        *active = Some(Active {
            context,
            session,
            channels,
            valid,
        });
        Ok(())
    }

    pub async fn stop(&self) {
        if let Some(active) = self.active.lock().await.take() {
            active.valid.store(false, Ordering::Release);
            active.channels.senders.lock().await.clear();
            active.session.on_stop(&active.context).await;
        }
    }

    pub async fn disconnect(&self, tx: &MessageSink) {
        if let Some(active) = self.active.lock().await.as_ref() {
            active
                .channels
                .senders
                .lock()
                .await
                .retain(|_, channel| !channel.same_channel(tx));
        }
    }

    async fn bound(&self, msg: &CastMessage, tx: &MessageSink) -> Result<Option<Active>> {
        let active = self.active.lock().await.clone();
        let Some(active) = active.filter(|a| a.context.transport_id == msg.destination_id) else {
            return Ok(None);
        };
        active.channels.register(&msg.source_id, tx).await?;
        Ok(Some(active))
    }

    pub async fn handle_mdx(&self, msg: &CastMessage, tx: &MessageSink) -> Result<()> {
        if let Some(active) = self.bound(msg, tx).await? {
            active
                .session
                .on_message(&active.context, NS_YOUTUBE, &payload_json(msg)?)
                .await;
        }
        Ok(())
    }

    pub async fn handle_load(
        &self,
        msg: &CastMessage,
        tx: &MessageSink,
        shared: &Shared,
    ) -> Result<bool> {
        let payload = payload_json(msg)?;
        if payload["type"] != "LOAD" {
            return Ok(false);
        }
        let Some(active) = self.bound(msg, tx).await? else {
            return Ok(false);
        };
        let rid = payload.get("requestId").cloned().unwrap_or(json!(0));
        let request = serde_json::from_value::<LoadRequest>(payload);
        let failure = match request {
            Ok(request) => match tokio::time::timeout(
                Duration::from_secs(15),
                active.session.resolve_media(&active.context, &request),
            )
            .await
            {
                Ok(Ok(mut media)) if active.valid.load(Ordering::Acquire) => {
                    media.autoplay = request.autoplay;
                    active.context.playback_controller().load(media).await;
                    crate::media::respond_media_status(tx, msg, shared, &rid, 0).await?;
                    return Ok(true);
                }
                Ok(Ok(_)) => "CANCELLED",
                Ok(Err(error)) => error.reason(),
                Err(_) => "UPSTREAM_TIMEOUT",
            },
            Err(_) => "INVALID_REQUEST",
        };
        send_json(
            tx,
            &msg.destination_id,
            &msg.source_id,
            NS_MEDIA,
            &json!({"type":"LOAD_FAILED", "requestId":rid, "reason":failure}),
        )
        .await?;
        Ok(true)
    }
}

struct Controller {
    id: String,
    valid: Arc<AtomicBool>,
    state: Arc<Mutex<AppState>>,
    bridge: Arc<Bridge>,
    player: PlayerHandle,
}

impl Controller {
    async fn current(&self) -> bool {
        self.valid.load(Ordering::Acquire)
            && self
                .state
                .lock()
                .await
                .session
                .as_ref()
                .is_some_and(|s| s.id == self.id && s.app_id == APP_ID)
    }
}

fn load_event(media: &PlaybackMedia) -> Result<Value> {
    let stream = media
        .streams
        .first()
        .ok_or_else(|| anyhow::anyhow!("no stream"))?;
    ensure!(
        stream.drm.is_none() && stream.segment_query.is_none(),
        "unsupported protected stream"
    );
    let mut event = json!({"type":"load", "video":true, "autoplay":media.autoplay,
        "position":media.start_time, "title":media.title, "mime_type":stream.content_type,
        "request_headers":stream.request_headers});
    match &stream.source {
        StreamSource::Url(url) => {
            ensure!(url.starts_with("https://"), "insecure YouTube stream");
            event["url"] = json!(url);
        }
        StreamSource::InlineManifest(manifest) => {
            ensure!(
                stream.content_type == "application/dash+xml" && manifest.len() <= 1024 * 1024,
                "unsupported inline source"
            );
            event["manifest"] = json!(numeric_adaptation_ids(manifest)?);
        }
    }
    Ok(event)
}

fn numeric_adaptation_ids(manifest: &str) -> Result<String> {
    let document = roxmltree::Document::parse(manifest)?;
    ensure!(
        document.root_element().has_tag_name("MPD"),
        "invalid DASH root"
    );
    let attributes: Vec<_> = document
        .descendants()
        .filter(|node| node.has_tag_name("AdaptationSet"))
        .filter_map(|node| node.attribute_node("id"))
        .collect();
    let mut used: std::collections::HashSet<u32> = attributes
        .iter()
        .filter_map(|attribute| attribute.value().parse().ok())
        .collect();
    let mut edits = Vec::new();
    let mut next = 0u32;
    for attribute in attributes {
        if attribute.value().parse::<u32>().is_ok() {
            continue;
        }
        while used.contains(&next) {
            next = next
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("too many adaptation sets"))?;
        }
        used.insert(next);
        edits.push((attribute.range(), format!("id=\"{next}\"")));
    }
    // Modify parsed attribute ranges only; media URLs and all XML escaping stay intact.
    let mut result = manifest.to_owned();
    for (range, replacement) in edits.into_iter().rev() {
        result.replace_range(range, &replacement);
    }
    Ok(result)
}

#[async_trait]
impl PlaybackController for Controller {
    async fn load(&self, media: PlaybackMedia) {
        let event = load_event(&media);
        let mut state = self.state.lock().await;
        if !self.valid.load(Ordering::Acquire) {
            return;
        }
        if let Some(session) = state
            .session
            .as_mut()
            .filter(|s| s.id == self.id && s.app_id == APP_ID)
        {
            let Ok(event) = event else {
                let _ = self.bridge.event(json!({"type":"playback_error"}));
                return;
            };
            if self.bridge.load_resolved(event).is_ok() {
                session.media = Some(MediaInfo {
                    content_id: media.content_id.unwrap_or_default(),
                    content_type: "video/youtube".into(),
                    stream_type: "BUFFERED".into(),
                });
            }
        }
    }
    async fn play(&self) {
        if self.current().await {
            let _ = self.player.play().await;
        }
    }
    async fn pause(&self) {
        if self.current().await {
            let _ = self.player.pause().await;
        }
    }
    async fn seek(&self, position: f64) {
        if position.is_finite() && self.current().await {
            let _ = self.player.seek(position.max(0.0) as f32).await;
        }
    }
    async fn stop(&self) {
        if self.current().await {
            let _ = self.player.stop().await;
            if let Some(session) = self
                .state
                .lock()
                .await
                .session
                .as_mut()
                .filter(|s| s.id == self.id)
            {
                session.media = None;
            }
        }
    }
}

fn playback_state(snapshot: PlayerSnapshot) -> PlaybackState {
    use vibecast_sdk::{IdleReason, PlayerState as State};
    let state = match snapshot.state {
        PlayerState::Playing => State::Playing,
        PlayerState::Paused => State::Paused,
        PlayerState::Loading | PlayerState::Buffering => State::Buffering,
        _ => State::Idle,
    };
    PlaybackState {
        player_state: state,
        current_time: f64::from(snapshot.position),
        duration: (snapshot.duration > 0.0).then_some(f64::from(snapshot.duration)),
        idle_reason: if snapshot.end_error {
            Some(IdleReason::Error)
        } else if snapshot.state == PlayerState::Ended {
            Some(IdleReason::Finished)
        } else {
            None
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vibecast_sdk::{PlaybackStream, StreamType};

    #[test]
    fn adaptive_source_preserves_manifest_and_request_headers() {
        let mut stream = PlaybackStream::inline_manifest("<MPD/>", "application/dash+xml");
        stream
            .request_headers
            .insert("User-Agent".into(), "Synthetic".into());
        let media = PlaybackMedia::new("synthetic", vec![stream], StreamType::Buffered);
        let event = load_event(&media).unwrap();
        assert_eq!(event["manifest"], "<MPD/>");
        assert_eq!(event["request_headers"]["User-Agent"], "Synthetic");
        assert!(event.get("url").is_none());
    }

    #[test]
    fn rejects_empty_oversized_and_insecure_sources() {
        let mut media = PlaybackMedia::new("synthetic", vec![], StreamType::Buffered);
        assert!(load_event(&media).is_err());
        media.streams = vec![PlaybackStream::url("http://example.test", "video/mp4")];
        assert!(load_event(&media).is_err());
        media.streams = vec![PlaybackStream::inline_manifest(
            "x".repeat(1024 * 1024 + 1),
            "application/dash+xml",
        )];
        assert!(load_event(&media).is_err());
    }

    #[test]
    fn playback_error_is_never_a_successful_end() {
        let state = playback_state(PlayerSnapshot {
            state: PlayerState::Ended,
            end_error: true,
            ..Default::default()
        });
        assert_eq!(state.idle_reason, Some(vibecast_sdk::IdleReason::Error));
    }

    #[test]
    fn media3_adaptation_ids_are_numeric_without_rewriting_urls() {
        let manifest = "<MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\"><Period><AdaptationSet id=\"video\"><BaseURL>https://media.example/v?a=1&amp;b=2</BaseURL></AdaptationSet><AdaptationSet id='audio-0'/><AdaptationSet id=\"0\"/></Period></MPD>";
        let result = numeric_adaptation_ids(manifest).unwrap();
        let document = roxmltree::Document::parse(&result).unwrap();
        let ids: Vec<u32> = document
            .descendants()
            .filter(|n| n.has_tag_name("AdaptationSet"))
            .map(|n| n.attribute("id").unwrap().parse().unwrap())
            .collect();
        assert_eq!(ids, vec![1, 2, 0]);
        assert!(result.contains("https://media.example/v?a=1&amp;b=2"));
        assert!(numeric_adaptation_ids("<bad").is_err());
        assert!(numeric_adaptation_ids("<html/>").is_err());
    }

    #[tokio::test]
    async fn media3_error_status_is_error_not_finished() {
        let bridge = Arc::new(Bridge::default());
        let player = bridge.spawn();
        bridge.update(
            0,
            PlayerSnapshot {
                state: PlayerState::Ended,
                end_error: true,
                ..Default::default()
            },
        );
        let mut session = Session::new(APP_ID.into(), 1);
        session.media = Some(MediaInfo {
            content_id: "synthetic".into(),
            content_type: "video/youtube".into(),
            stream_type: "BUFFERED".into(),
        });
        let state = Mutex::new(AppState {
            session: Some(session),
            ..Default::default()
        });
        tokio::time::sleep(Duration::from_millis(150)).await;
        let status = crate::media::player_status(&player, &state).await.unwrap();
        assert_eq!(status["idleReason"], "ERROR");
    }

    #[tokio::test]
    async fn sender_limit_and_disconnect_do_not_keep_writer_alive() {
        let channels = Channels {
            transport: "synthetic".into(),
            senders: Mutex::new(HashMap::new()),
        };
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        for id in 0..MAX_SENDERS {
            channels.register(&id.to_string(), &tx).await.unwrap();
        }
        assert!(channels.register("extra", &tx).await.is_err());
        channels
            .senders
            .lock()
            .await
            .retain(|_, sender| !sender.same_channel(&tx));
        drop(tx);
        assert!(rx.is_closed());
    }

    #[tokio::test]
    async fn cancelled_or_replaced_sessions_cannot_load_media() {
        let bridge = Arc::new(Bridge::default());
        let session = Session::new(APP_ID.into(), 1);
        let state = Arc::new(Mutex::new(AppState {
            session: Some(session.clone()),
            ..Default::default()
        }));
        let valid = Arc::new(AtomicBool::new(true));
        let controller = Controller {
            id: session.id,
            valid: valid.clone(),
            state: state.clone(),
            bridge: bridge.clone(),
            player: bridge.spawn(),
        };
        let media = PlaybackMedia::new(
            "synthetic",
            vec![PlaybackStream::url(
                "https://example.test/media",
                "video/mp4",
            )],
            StreamType::Buffered,
        );
        controller.load(media.clone()).await;
        assert_eq!(bridge.poll().len(), 1);
        valid.store(false, Ordering::Release);
        controller.load(media.clone()).await;
        assert!(bridge.poll().is_empty());
        valid.store(true, Ordering::Release);
        state.lock().await.session = Some(Session::new(APP_ID.into(), 2));
        controller.load(media).await;
        assert!(bridge.poll().is_empty());
    }
}
