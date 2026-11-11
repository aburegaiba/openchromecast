//! TLS transport + Cast V2 message framing + namespace dispatch.
//!
//! Cast V2 runs over TLS on port 8009. Each message is a `CastMessage`
//! (protobuf) prefixed with a 4-byte big-endian length. Before anything else,
//! the sender completes a device-auth handshake (`...tp.deviceauth`).

use crate::crypto::Identity;
use crate::media;
use crate::player::PlayerHandle;
use crate::proto::cast_channel::{
    AuthResponse, CastMessage, DeviceAuthMessage, HashAlgorithm, PayloadType, SignatureAlgorithm,
};
use crate::receiver;
use crate::state::AppState;
use crate::youtube;
use anyhow::{Context, Result, bail};
use prost::Message as _;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, mpsc};
use tokio_rustls::TlsAcceptor;
use tracing::{debug, info, warn};

// Standard Cast V2 namespaces.
pub const NS_CONNECTION: &str = "urn:x-cast:com.google.cast.tp.connection";
pub const NS_HEARTBEAT: &str = "urn:x-cast:com.google.cast.tp.heartbeat";
pub const NS_DEVICE_AUTH: &str = "urn:x-cast:com.google.cast.tp.deviceauth";
pub const NS_RECEIVER: &str = "urn:x-cast:com.google.cast.receiver";
pub const NS_MEDIA: &str = "urn:x-cast:com.google.cast.media";
pub const NS_YOUTUBE: &str = "urn:x-cast:com.google.youtube.mdx";

/// A sink for framed messages destined for one client connection.
pub type MessageSink = mpsc::Sender<Vec<u8>>;

/// Everything shared between connections and tasks.
#[derive(Clone)]
pub struct Shared {
    pub state: Arc<Mutex<AppState>>,
    pub identity: Arc<Identity>,
    pub player: PlayerHandle,
    #[cfg(feature = "embedded-player")]
    pub youtube: Option<Arc<youtube::Host>>,
}

/// Accept loop: spawn one task per incoming connection.
pub async fn run(listener: TcpListener, shared: Shared) -> Result<()> {
    run_with_clock(
        listener,
        shared,
        Arc::new(time::OffsetDateTime::now_utc),
        std::time::Duration::from_secs(60),
    )
    .await
}

pub(crate) async fn run_with_clock(
    listener: TcpListener,
    mut shared: Shared,
    clock: Arc<dyn Fn() -> time::OffsetDateTime + Send + Sync>,
    check_interval: std::time::Duration,
) -> Result<()> {
    let mut tick = tokio::time::interval(check_interval);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut renewal: Option<tokio::task::JoinHandle<Result<Identity>>> = None;
    loop {
        let renewed = async {
            match renewal.as_mut() {
                Some(task) => task.await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            accepted = listener.accept() => {
                let (tcp, peer) = accepted?;
                if !shared.identity.tls_valid_at(clock()) {
                    debug!("TLS certificate is not currently valid; waiting for renewal");
                    continue;
                }
                // The TLS handshake and device-auth response must use the same immutable epoch.
                let shared = shared.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_connection(tcp, peer, shared).await {
                        warn!("connection {peer} ended: {err:#}");
                    }
                });
            }
            _ = tick.tick() => {
                if renewal.is_none() && shared.identity.tls_renewal_due_at(clock()) {
                    let previous = shared.identity.clone();
                    let now = clock();
                    renewal = Some(tokio::task::spawn_blocking(move || previous.renew_tls_at(now)));
                }
            }
            result = renewed => {
                renewal = None;
                match result {
                    Ok(Ok(identity)) if identity.tls_valid_at(clock()) => {
                        shared.identity = Arc::new(identity);
                        info!("TLS certificate renewed for new Cast connections");
                    }
                    _ => warn!("TLS renewal failed; no invalid certificate will be served"),
                }
            }
        }
    }
}

/// Handle one Cast connection: TLS, device auth, then the message loop.
async fn handle_connection(tcp: TcpStream, peer: SocketAddr, shared: Shared) -> Result<()> {
    info!("incoming Cast connection from {peer}");
    let acceptor = tls_acceptor(&shared.identity)?;
    let stream = match acceptor.accept(tcp).await {
        Ok(s) => s,
        Err(e) => {
            // Probing connections (VLC, Google Home, Chrome, ...) frequently
            // open a socket and abort before the handshake completes. That is
            // normal discovery noise, so log it at debug, not as a warning.
            debug!("TLS handshake with {peer} failed: {e}");
            return Ok(());
        }
    };
    info!("TLS established with {peer}");

    let (mut read_half, write_half) = tokio::io::split(stream);
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(64);

    // Writer task: owns the TLS write half, drains the outbound queue.
    let mut write_half = write_half;
    let writer = tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            if write_half.write_all(&frame).await.is_err() {
                break;
            }
        }
    });

    #[cfg(feature = "embedded-player")]
    let (transport_tx, transport_rx) = tokio::sync::watch::channel(None);
    #[cfg(feature = "embedded-player")]
    let observer = observe_receiver(
        shared.state.clone(),
        shared.player.clone(),
        transport_rx,
        tx.clone(),
    )
    .await;

    // Read + dispatch loop. Track whether this connection owns the active
    // session's transport channel.
    let mut transport: Option<String> = None;
    loop {
        let msg = match read_message(&mut read_half).await {
            Ok(m) => m,
            Err(err) => {
                debug!("read from {peer} failed: {err:#}");
                break;
            }
        };
        if msg.namespace == NS_CONNECTION
            && let Ok(payload) = payload_json(&msg)
            && payload.get("type").and_then(|v| v.as_str()) == Some("CONNECT")
        {
            transport = Some(msg.destination_id.clone());
            // Count live connections bound to the active session. A sender
            // (e.g. VLC) re-opens its transport with the same session id when
            // auto-advancing to the next track — without this the moment the
            // old transport closes we'd tear down playback of the new track.
            if let Some(dest) = &transport {
                let mut st = shared.state.lock().await;
                if st.session.as_ref().is_some_and(|s| s.id == *dest) {
                    #[cfg(feature = "embedded-player")]
                    transport_tx.send_replace(transport.clone());
                    *st.session_connections.entry(dest.clone()).or_insert(0) += 1;
                }
            }
        }
        if let Err(err) = dispatch(&msg, &tx, &shared).await {
            warn!("dispatch error on {peer}: {err:#}");
        }
    }

    #[cfg(feature = "embedded-player")]
    observer.abort();
    #[cfg(feature = "embedded-player")]
    if let Some(host) = &shared.youtube {
        host.disconnect(&tx).await;
    }
    drop(tx);
    let _ = writer.await;

    // Drop this connection's claim on the session it was bound to. Only stop
    // playback when the LAST connection to the session is gone: senders (VLC
    // auto-advance) re-open a new transport with the same session id before
    // closing the old one, so a single transport close must not kill playback.
    let teardown = {
        let mut st = shared.state.lock().await;
        let mut do_teardown = false;
        if let Some(t) = &transport {
            if let Some(n) = st.session_connections.get_mut(t) {
                *n = n.saturating_sub(1);
            }
            if st.session_connections.get(t).copied().unwrap_or(0) == 0 {
                st.session_connections.remove(t);
            }
            if st.session.as_ref().is_some_and(|s| s.id == *t)
                && st.session_connections.get(t).copied().unwrap_or(0) == 0
            {
                // Lounge continues independently when the Cast transport closes.
                #[cfg(feature = "embedded-player")]
                {
                    do_teardown = shared.youtube.is_none()
                        || st.session.as_ref().is_none_or(|s| s.app_id != "233637DE");
                }
                #[cfg(not(feature = "embedded-player"))]
                {
                    do_teardown = true;
                }
            }
        }
        if do_teardown {
            st.session = None;
        }
        do_teardown
    };
    if teardown {
        info!("session no longer has any connections; stopping playback");
        let _ = shared.player.stop().await;
    }

    info!("connection {peer} closed");
    Ok(())
}

#[cfg(feature = "embedded-player")]
async fn observe_receiver(
    state: Arc<Mutex<AppState>>,
    player: PlayerHandle,
    transport: tokio::sync::watch::Receiver<Option<String>>,
    tx: MessageSink,
) -> tokio::task::JoinHandle<()> {
    let mut previous = receiver::receiver_status(&*state.lock().await);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        tick.tick().await;
        let mut previous_media = None;
        loop {
            tick.tick().await;
            let (status, session_id) = {
                let state = state.lock().await;
                (
                    receiver::receiver_status(&state),
                    state.session.as_ref().map(|s| s.id.clone()),
                )
            };
            if previous != status {
                previous = status.clone();
                if send_json(
                    &tx,
                    "receiver-0",
                    "*",
                    NS_RECEIVER,
                    &serde_json::json!({"type":"RECEIVER_STATUS", "status":status}),
                )
                .await
                .is_err()
                {
                    break;
                }
            }
            let bound = transport.borrow().clone();
            if session_id.is_none() || bound != session_id {
                previous_media = None;
                continue;
            }
            let Some(media) = media::player_status(&player, &state).await else {
                continue;
            };
            if media["playerState"] == "IDLE"
                && !media["media"].is_null()
                && media.get("idleReason").is_none()
            {
                continue;
            }
            if state.lock().await.session.as_ref().map(|s| &s.id) != session_id.as_ref() {
                continue;
            }
            if previous_media.as_ref() != Some(&media) {
                previous_media = Some(media.clone());
                if send_json(
                    &tx,
                    session_id.as_deref().unwrap(),
                    "*",
                    NS_MEDIA,
                    &serde_json::json!({"type":"MEDIA_STATUS", "status":[media]}),
                )
                .await
                .is_err()
                {
                    break;
                }
            }
        }
    })
}

#[cfg(all(test, feature = "embedded-player"))]
mod observer_tests {
    use super::*;
    use serde_json::{Value, json};

    async fn frame(rx: &mut mpsc::Receiver<Vec<u8>>, namespace: &str) -> Value {
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let data = rx.recv().await.unwrap();
                let message = CastMessage::decode(&data[4..]).unwrap();
                if message.namespace == namespace {
                    assert_eq!(message.destination_id, "*");
                    return payload_json(&message).unwrap();
                }
            }
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn observer_publishes_changes_without_an_initial_unsolicited_frame() {
        let state = Arc::new(Mutex::new(AppState::default()));
        let (tx, mut rx) = mpsc::channel(8);
        let (_, transport) = tokio::sync::watch::channel(None);
        let task = observe_receiver(state.clone(), PlayerHandle::spawn_null(), transport, tx).await;
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv())
                .await
                .is_err()
        );
        state.lock().await.volume = 0.7;
        let value = frame(&mut rx, NS_RECEIVER).await;
        assert!((value["status"]["volume"]["level"].as_f64().unwrap() - 0.7).abs() < 0.001);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(1100), rx.recv())
                .await
                .is_err()
        );
        task.abort();
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn media_status_requires_binding_to_the_current_transport() {
        let state = Arc::new(Mutex::new(AppState::default()));
        let (tx, mut rx) = mpsc::channel(8);
        let (bound, transport) = tokio::sync::watch::channel(None);
        let task = observe_receiver(state.clone(), PlayerHandle::spawn_null(), transport, tx).await;
        let session = crate::state::Session::new("CC1AD845".into(), 1);
        let id = session.id.clone();
        state.lock().await.session = Some(session);
        assert_eq!(
            frame(&mut rx, NS_RECEIVER).await["status"]["applications"][0]["appId"],
            "CC1AD845"
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(1100), rx.recv())
                .await
                .is_err()
        );
        bound.send_replace(Some(id));
        let status = frame(&mut rx, NS_MEDIA).await;
        assert_eq!(status["status"][0]["playerState"], "IDLE");
        assert_eq!(status["status"][0]["idleReason"], json!("CANCELLED"));
        state.lock().await.session = Some(crate::state::Session::new("CC1AD845".into(), 2));
        frame(&mut rx, NS_RECEIVER).await;
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(1100), rx.recv())
                .await
                .is_err()
        );
        task.abort();
    }
}

fn tls_acceptor(identity: &Identity) -> Result<TlsAcceptor> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(
            vec![rustls::pki_types::CertificateDer::from(
                identity.tls_cert_der().to_vec(),
            )],
            identity.tls_private_key_der()?,
        )?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

/// Read one length-prefixed `CastMessage`.
async fn read_message<S: AsyncRead + Unpin>(stream: &mut S) -> Result<CastMessage> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 1 << 20 {
        bail!("frame too large: {len} bytes");
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    CastMessage::decode(&buf[..]).context("failed to decode CastMessage")
}

/// Serialize a message and enqueue it on the outbound sink.
pub async fn send_message(tx: &MessageSink, msg: &CastMessage) -> Result<()> {
    let bytes = msg.encode_to_vec();
    let mut framed = Vec::with_capacity(bytes.len() + 4);
    framed.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    framed.extend_from_slice(&bytes);
    tx.send(framed).await?;
    Ok(())
}

/// Build and enqueue a JSON `CastMessage`.
pub async fn send_json(
    tx: &MessageSink,
    source: &str,
    destination: &str,
    namespace: &str,
    payload: &serde_json::Value,
) -> Result<()> {
    let payload_utf8 = serde_json::to_string(payload)?;
    let msg = CastMessage {
        protocol_version: 0,
        source_id: source.to_string(),
        destination_id: destination.to_string(),
        namespace: namespace.to_string(),
        payload_type: PayloadType::String as i32,
        payload_utf8: Some(payload_utf8),
        payload_binary: None,
        ..Default::default()
    };
    send_message(tx, &msg).await
}

/// Parse the UTF-8 JSON payload of a message.
pub fn payload_json(msg: &CastMessage) -> Result<serde_json::Value> {
    let s = msg
        .payload_utf8
        .as_deref()
        .context("message has no UTF-8 payload")?;
    Ok(serde_json::from_str(s)?)
}

/// Route a message by namespace.
async fn dispatch(msg: &CastMessage, tx: &MessageSink, shared: &Shared) -> Result<()> {
    let kind = msg
        .payload_utf8
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_string))
        .unwrap_or_default();
    debug!(
        "ns={} src={} dst={} type={}",
        msg.namespace, msg.source_id, msg.destination_id, kind
    );
    match msg.namespace.as_str() {
        NS_DEVICE_AUTH => handle_device_auth(msg, tx, shared).await,
        NS_CONNECTION => handle_connection_ns(msg).await,
        NS_HEARTBEAT => handle_heartbeat(msg, tx).await,
        NS_RECEIVER => receiver::handle(msg, tx, shared).await,
        NS_MEDIA => {
            #[cfg(feature = "embedded-player")]
            if let Some(host) = &shared.youtube
                && host.handle_load(msg, tx, shared).await?
            {
                return Ok(());
            }
            media::handle(msg, tx, shared).await
        }
        NS_YOUTUBE => youtube::handle(msg, tx, shared).await,
        other => {
            warn!("unhandled namespace: {other}");
            Ok(())
        }
    }
}

/// Device-auth handshake: sign the sender's nonce and present our certificate.
///
/// Ground truth from Chromium's `cast_auth_util.cc`: the sender expects the
/// `AuthResponse` to echo `sender_nonce` (field 5), carry a certificate chain
/// (`client_auth_certificate` + `intermediate_certificate`) that paths to a
/// trusted Cast root CA, and sign `sender_nonce || peer_cert_der` (the TLS
/// cert) with RSASSA-PKCS#1 v1.5 + SHA-256. The incoming challenge is parsed
/// *tolerantly* (only the nonce is needed), so field-number drift between Cast
/// SDK versions is harmless.
async fn handle_device_auth(msg: &CastMessage, tx: &MessageSink, shared: &Shared) -> Result<()> {
    let payload = msg
        .payload_binary
        .as_deref()
        .context("device auth message has no binary payload")?;

    // DeviceAuthMessage.challenge is a length-delimited sub-message at field 1.
    if let Some(challenge) = extract_length_delimited(payload, 1) {
        let nonce = extract_nonce(challenge).unwrap_or_default();

        // The signature covers (sender_nonce || TLS peer cert).
        let mut sig_input = nonce.clone();
        sig_input.extend_from_slice(shared.identity.tls_cert_der());
        let signature = shared.identity.sign(&sig_input);

        let response = DeviceAuthMessage {
            response: Some(AuthResponse {
                signature,
                client_auth_certificate: shared.identity.auth_cert_der().to_vec(),
                intermediate_certificate: shared.identity.intermediate_ders().to_vec(),
                signature_algorithm: Some(SignatureAlgorithm::RsassaPkcs1v15 as i32),
                sender_nonce: Some(nonce.clone()),
                hash_algorithm: Some(HashAlgorithm::Sha256 as i32),
                crl: None,
            }),
            ..Default::default()
        };

        let reply = CastMessage {
            protocol_version: 0,
            source_id: msg.destination_id.clone(),
            destination_id: msg.source_id.clone(),
            namespace: NS_DEVICE_AUTH.to_string(),
            payload_type: PayloadType::Binary as i32,
            payload_utf8: None,
            payload_binary: Some(response.encode_to_vec()),
            ..Default::default()
        };
        send_message(tx, &reply).await?;
        info!(
            "completed device auth (nonce {}B, sig input {}B)",
            nonce.len(),
            sig_input.len()
        );
    }
    Ok(())
}

/// Minimal, tolerant protobuf walker: return the bytes of the first
/// length-delimited field with number `target` at this level.
/// Unknown fields of any wire type are skipped, so schema drift is harmless.
fn extract_length_delimited(data: &[u8], target: u32) -> Option<&[u8]> {
    length_delimited_fields(data)
        .into_iter()
        .find(|(field, _)| *field == target)
        .map(|(_, bytes)| bytes)
}

/// Pick the sender nonce out of an `AuthChallenge`: the first length-delimited
/// field that is random binary (non-UTF-8). Falls back to the first field.
fn extract_nonce(challenge: &[u8]) -> Option<Vec<u8>> {
    let fields = length_delimited_fields(challenge);
    fields
        .iter()
        .find(|(_, b)| b.len() >= 8 && std::str::from_utf8(b).is_err())
        .or_else(|| fields.first())
        .map(|(_, b)| b.to_vec())
}

/// Walk one protobuf message level and return every (field_number, bytes)
/// pair for length-delimited fields, skipping all other wire types.
fn length_delimited_fields(data: &[u8]) -> Vec<(u32, &[u8])> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < data.len() {
        let Some((tag, next)) = read_varint(data, pos) else {
            break;
        };
        pos = next;
        let field = (tag >> 3) as u32;
        let wire = (tag & 0x07) as u8;
        match wire {
            0 => {
                let Some((_, next)) = read_varint(data, pos) else {
                    break;
                };
                pos = next;
            }
            1 => {
                let Some(p) = pos.checked_add(8) else { break };
                pos = p;
            }
            2 => {
                let Some((len, next)) = read_varint(data, pos) else {
                    break;
                };
                pos = next;
                let Some(end) = pos.checked_add(len as usize) else {
                    break;
                };
                if end <= data.len() {
                    out.push((field, &data[pos..end]));
                }
                pos = end;
            }
            5 => {
                let Some(p) = pos.checked_add(4) else { break };
                pos = p;
            }
            _ => break,
        }
    }
    out
}

/// Decode a base-128 varint at `pos`.
fn read_varint(data: &[u8], pos: usize) -> Option<(u64, usize)> {
    let mut value: u64 = 0;
    let mut shift = 0;
    let mut p = pos;
    loop {
        let b = *data.get(p)?;
        value |= ((b & 0x7f) as u64) << shift;
        p += 1;
        if b & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
    Some((value, p))
}

/// Connection-namespace bookkeeping (CONNECT / CLOSE).
async fn handle_connection_ns(msg: &CastMessage) -> Result<()> {
    let payload = payload_json(msg)?;
    let kind = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match kind {
        "CONNECT" => info!("CONNECT {} -> {}", msg.source_id, msg.destination_id),
        "CLOSE" => info!("CLOSE {} -> {}", msg.source_id, msg.destination_id),
        other => warn!("unhandled connection message type: {other}"),
    }
    Ok(())
}

/// Heartbeat namespace: answer PING with PONG.
async fn handle_heartbeat(msg: &CastMessage, tx: &MessageSink) -> Result<()> {
    let payload = payload_json(msg)?;
    let kind = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if kind == "PING" {
        let pong = serde_json::json!({ "type": "PONG" });
        send_json(tx, &msg.destination_id, &msg.source_id, NS_HEARTBEAT, &pong).await?;
    }
    Ok(())
}
