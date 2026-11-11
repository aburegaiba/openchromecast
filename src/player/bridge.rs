//! Bounded command/status bridge for a player owned by an embedding host.

use super::{PlayerCommand, PlayerHandle, PlayerSnapshot, PlayerState};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{Mutex, mpsc};

const MAX_EVENTS: usize = 64;

#[derive(Default)]
struct State {
    generation: u64,
    events: VecDeque<Value>,
    snapshot: PlayerSnapshot,
}

#[derive(Default)]
pub struct Bridge {
    state: StdMutex<State>,
}

impl Bridge {
    pub fn load_resolved(&self, mut event: Value) -> Result<()> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if event["type"] != "load" || state.events.len() >= MAX_EVENTS {
            bail!("invalid or full native load queue");
        }
        state.generation += 1;
        state.snapshot = PlayerSnapshot {
            state: PlayerState::Buffering,
            ..state.snapshot.clone()
        };
        state.snapshot.end_error = false;
        event["generation"] = json!(state.generation);
        state.events.push_back(event);
        Ok(())
    }

    pub fn event(&self, event: Value) -> Result<()> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.events.len() >= MAX_EVENTS {
            bail!("host player command queue is full");
        }
        state.events.push_back(event);
        Ok(())
    }

    pub fn poll(&self) -> Vec<Value> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .events
            .drain(..)
            .collect()
    }

    pub fn update(&self, generation: u64, snapshot: PlayerSnapshot) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.generation == generation {
            state.snapshot = snapshot;
        }
    }

    fn command(&self, command: PlayerCommand) -> Result<()> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.events.len() >= MAX_EVENTS {
            bail!("host player command queue is full");
        }
        let mut event = match command {
            PlayerCommand::Load {
                url,
                position,
                autoplay,
                video,
                title,
                ..
            } => {
                state.generation += 1;
                state.snapshot.state = PlayerState::Buffering;
                state.snapshot.end_error = false;
                json!({"type":"load", "url":url, "position":position,
                    "autoplay":autoplay, "video":video, "title":title})
            }
            PlayerCommand::Play => json!({"type":"play"}),
            PlayerCommand::Pause => json!({"type":"pause"}),
            PlayerCommand::Seek(position) => json!({"type":"seek", "position":position}),
            PlayerCommand::Stop => {
                state.generation += 1;
                state.snapshot.state = PlayerState::Idle;
                json!({"type":"stop"})
            }
            PlayerCommand::SetVolume(volume) => {
                json!({"type":"volume", "volume":volume.clamp(0.0, 1.0)})
            }
            PlayerCommand::SetMute(muted) => json!({"type":"mute", "muted":muted}),
            PlayerCommand::GetSnapshot(reply) => {
                let _ = reply.send(state.snapshot.clone());
                return Ok(());
            }
        };
        event["generation"] = json!(state.generation);
        state.events.push_back(event);
        Ok(())
    }

    pub fn spawn(self: &Arc<Self>) -> PlayerHandle {
        let (tx, mut rx) = mpsc::channel(32);
        let snapshot = Arc::new(Mutex::new(PlayerSnapshot::default()));
        let published = snapshot.clone();
        let bridge = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    cmd = rx.recv() => match cmd {
                        Some(cmd) => if bridge.command(cmd).is_err() {
                            let mut s = bridge.state.lock().unwrap_or_else(|e| e.into_inner());
                            s.snapshot.state = PlayerState::Ended;
                            s.snapshot.end_error = true;
                        },
                        None => break,
                    },
                    _ = tick.tick() => {
                        let current = bridge.state.lock().unwrap_or_else(|e| e.into_inner()).snapshot.clone();
                        *published.lock().await = current;
                    }
                }
            }
        });
        PlayerHandle::new(tx, snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn actor_publishes_host_snapshot() {
        let bridge = Arc::new(Bridge::default());
        let player = bridge.spawn();
        bridge.update(
            0,
            PlayerSnapshot {
                state: PlayerState::Paused,
                position: 4.0,
                ..Default::default()
            },
        );
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        assert_eq!(player.snapshot().await.state, PlayerState::Paused);
        assert_eq!(player.snapshot().await.position, 4.0);
    }

    #[test]
    fn queue_is_bounded_and_poll_drains_it() {
        let bridge = Bridge::default();
        for _ in 0..MAX_EVENTS {
            bridge.event(json!({"type":"ready"})).unwrap();
        }
        assert!(bridge.event(json!({})).is_err());
        assert_eq!(bridge.poll().len(), MAX_EVENTS);
        assert!(bridge.poll().is_empty());
    }

    #[test]
    fn stale_status_cannot_replace_a_new_load_or_stop() {
        let bridge = Bridge::default();
        bridge
            .command(PlayerCommand::Load {
                url: "https://example.test/media.mp3".into(),
                position: 0.0,
                autoplay: true,
                video: false,
                title: None,
                cover: None,
            })
            .unwrap();
        let generation = bridge.poll()[0]["generation"].as_u64().unwrap();
        bridge.command(PlayerCommand::Stop).unwrap();
        bridge.update(
            generation,
            PlayerSnapshot {
                state: PlayerState::Playing,
                ..Default::default()
            },
        );
        assert_eq!(
            bridge.state.lock().unwrap().snapshot.state,
            PlayerState::Idle
        );
    }

    #[test]
    fn volume_is_clamped_before_delivery() {
        let bridge = Bridge::default();
        bridge.command(PlayerCommand::SetVolume(8.0)).unwrap();
        assert_eq!(bridge.poll()[0]["volume"], 1.0);
    }

    #[test]
    fn resolved_loads_use_the_same_generation_guard_as_standard_loads() {
        let bridge = Bridge::default();
        bridge
            .load_resolved(json!({"type":"load", "manifest":"<MPD/>"}))
            .unwrap();
        let generation = bridge.poll()[0]["generation"].as_u64().unwrap();
        bridge.command(PlayerCommand::Stop).unwrap();
        bridge.update(
            generation,
            PlayerSnapshot {
                state: PlayerState::Playing,
                ..Default::default()
            },
        );
        assert_eq!(
            bridge.state.lock().unwrap().snapshot.state,
            PlayerState::Idle
        );
        assert!(bridge.load_resolved(json!({"type":"not_load"})).is_err());
    }
}
