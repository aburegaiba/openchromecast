//! Android embeds the existing Cast transport and owns discovery and playback.
#![allow(dead_code)]

mod device_info;
#[cfg(test)]
mod protocol_tests;

#[path = "../../../src/cover.rs"]
mod cover;
#[path = "../../../src/crypto.rs"]
mod crypto;
#[path = "../../../src/media.rs"]
mod media;
#[path = "../../../src/player/mod.rs"]
mod player;
#[path = "../../../src/proto.rs"]
mod proto;
#[path = "../../../src/receiver.rs"]
mod receiver;
#[path = "../../../src/server.rs"]
mod server;
#[path = "../../../src/state.rs"]
mod state;
#[path = "../../../src/youtube.rs"]
mod youtube;

use jni::{
    JNIEnv,
    objects::{JClass, JString},
    sys::{jboolean, jfloat, jint, jlong, jstring},
};
use player::{PlayerSnapshot, PlayerState, bridge::Bridge};
use serde_json::json;
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::Notify;

struct Session {
    bridge: Arc<Bridge>,
    shutdown: Arc<Notify>,
    finished: Arc<AtomicBool>,
}
static SESSION: OnceLock<Mutex<Option<Session>>> = OnceLock::new();
fn session() -> &'static Mutex<Option<Session>> {
    SESSION.get_or_init(|| Mutex::new(None))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_openchromecast_receiver_NativeBridge_running(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    session()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .is_some_and(|s| !s.finished.load(Ordering::Acquire)) as jboolean
}

fn response(env: &JNIEnv, text: &str) -> jstring {
    env.new_string(text)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_openchromecast_receiver_NativeBridge_start(
    mut env: JNIEnv,
    _class: JClass,
    port: jint,
    device_id: JString,
    name: JString,
    initial_volume: jfloat,
    data_dir: JString,
) -> jstring {
    let result = (|| -> anyhow::Result<()> {
        let device_id: String = env.get_string(&device_id)?.into();
        let name: String = env.get_string(&name)?.into();
        let data_dir: String = env.get_string(&data_dir)?.into();
        start(
            port,
            device_info::DeviceInfo::new(&device_id, name)?,
            initial_volume,
            std::path::PathBuf::from(data_dir),
        )
    })();
    response(
        &env,
        &result.err().map(|e| e.to_string()).unwrap_or_default(),
    )
}

fn start(
    port: jint,
    info: device_info::DeviceInfo,
    initial_volume: f32,
    data_dir: std::path::PathBuf,
) -> anyhow::Result<()> {
    anyhow::ensure!((1024..=65535).contains(&port), "invalid port");
    anyhow::ensure!(
        initial_volume.is_finite() && (0.0..=1.0).contains(&initial_volume),
        "invalid initial volume"
    );
    let mut guard = session().lock().unwrap_or_else(|e| e.into_inner());
    anyhow::ensure!(
        guard
            .as_ref()
            .is_none_or(|s| s.finished.load(Ordering::Acquire)),
        "receiver already started or stopping"
    );
    let bridge = Arc::new(Bridge::default());
    let shutdown = Arc::new(Notify::new());
    let worker_bridge = bridge.clone();
    let worker_shutdown = shutdown.clone();
    let finished = Arc::new(AtomicBool::new(false));
    let worker_finished = finished.clone();
    std::thread::Builder::new()
        .name("cast-receiver".into())
        .spawn(move || {
            let run = || -> anyhow::Result<()> {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()?;
                rt.block_on(async {
                    // Bind before announcing readiness. Port conflicts are real startup errors.
                    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port as u16)).await?;
                    let metadata = tokio::net::TcpListener::bind(("0.0.0.0", device_info::HTTP_PORT)).await?;
                    let identity =
                        tokio::task::spawn_blocking(crypto::Identity::generate).await??;
                    let state = Arc::new(tokio::sync::Mutex::new(state::AppState {
                            volume: initial_volume,
                            ..Default::default()
                        }));
                    let player = worker_bridge.spawn();
                    let mut context = vibecast_sdk::ReceiverContext::new(
                        &info.name, "OpenChromecast Android", info.id.to_string(), data_dir);
                    context.capabilities.video_codecs = vec!["h264".into()];
                    context.capabilities.max_resolution = vibecast_sdk::Resolution::new(1280, 720);
                    let host = youtube::Host::new(context, state.clone(), worker_bridge.clone(), player.clone())?;
                    let shared = server::Shared {
                        state,
                        identity: Arc::new(identity),
                        player,
                        youtube: Some(host.clone()),
                    };
                    worker_bridge.event(json!({"type":"ready"}))?;
                    let result = tokio::select! {
                        result = server::run(listener, shared) => result,
                        result = axum::serve(metadata, device_info::router(info)) => result.map_err(Into::into),
                        _ = worker_shutdown.notified() => Ok(()),
                    };
                    host.stop().await;
                    result
                })
            };
            if let Err(error) = run() {
                // Do not log media URLs or credentials carried in Cast messages.
                let _ = worker_bridge.event(json!({"type":"error", "message":error.to_string()}));
            }
            let _ = worker_bridge.event(json!({"type":"stopped"}));
            worker_finished.store(true, Ordering::Release);
            // Leave terminal events available to the host until stop() clears them.
        })?;
    *guard = Some(Session {
        bridge,
        shutdown,
        finished,
    });
    Ok(())
}

#[cfg(test)]
mod startup_tests {
    use super::*;

    #[test]
    fn rejects_invalid_initial_volume_before_starting_a_receiver() {
        for volume in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
            let info =
                device_info::DeviceInfo::new("e8d8571a824a45df88929e1c220f602d", "Receiver".into())
                    .unwrap();
            assert!(start(8009, info, volume, std::path::PathBuf::from("unused")).is_err());
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_openchromecast_receiver_NativeBridge_poll(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let events = session()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|s| s.bridge.poll())
        .unwrap_or_default();
    response(
        &env,
        &serde_json::to_string(&events).unwrap_or_else(|_| "[]".into()),
    )
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_openchromecast_receiver_NativeBridge_stop(
    _env: JNIEnv,
    _class: JClass,
) {
    if let Some(s) = session().lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        s.shutdown.notify_one();
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_openchromecast_receiver_NativeBridge_status(
    mut env: JNIEnv,
    _class: JClass,
    generation: jlong,
    state: JString,
    position: jfloat,
    duration: jfloat,
    volume: jfloat,
    muted: jboolean,
    error: jboolean,
) {
    let Ok(text) = env.get_string(&state) else {
        return;
    };
    let state = match text.to_str().unwrap_or("") {
        "PLAYING" => PlayerState::Playing,
        "PAUSED" => PlayerState::Paused,
        "BUFFERING" => PlayerState::Buffering,
        "ENDED" => PlayerState::Ended,
        _ => PlayerState::Idle,
    };
    if ![position, duration, volume].iter().all(|v| v.is_finite()) {
        return;
    }
    if let Some(s) = session().lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        s.bridge.update(
            generation as u64,
            PlayerSnapshot {
                state,
                position: position.max(0.0),
                duration: duration.max(0.0),
                volume: volume.clamp(0.0, 1.0),
                muted: muted != 0,
                end_error: error != 0,
            },
        );
    }
}
