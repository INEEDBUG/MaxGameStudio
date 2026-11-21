//! Readiness for one off-line playback, independently of the public GSI status.

use serde_json::Value;
use std::{io, sync::Mutex, time::Instant};
use subtle::ConstantTimeEq;
use tokio::sync::watch;

use super::playback_spec::hexadecimal_id;

#[derive(Clone)]
pub(super) struct GameUpdate {
    pub received: Instant,
    pub map: String,
    pub round: Option<i64>,
    pub player: Option<u64>,
}

#[derive(Default)]
pub(super) struct PlaybackGsi(Mutex<Option<Feed>>);

struct Feed {
    token: String,
    updates: watch::Sender<Option<GameUpdate>>,
}

pub(super) struct PlaybackFeed<'a> {
    source: &'a PlaybackGsi,
    pub updates: watch::Receiver<Option<GameUpdate>>,
}

impl PlaybackGsi {
    /// The token is generated separately from the session ID, which is visible in runtime status.
    pub(super) fn begin(&self, token: String) -> io::Result<PlaybackFeed<'_>> {
        if !hexadecimal_id(&token, 64) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid playback token",
            ));
        }
        let mut active = self.0.lock().expect("playback GSI feed");
        if active.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Playback GSI is already active",
            ));
        }
        let (sender, updates) = watch::channel(None);
        *active = Some(Feed {
            token,
            updates: sender,
        });
        Ok(PlaybackFeed {
            source: self,
            updates,
        })
    }

    /// An authenticated menu/heartbeat is accepted without opening the readiness gate.
    pub(super) fn notify(&self, payload: &Value) -> Result<bool, ()> {
        let active = self.0.lock().expect("playback GSI feed");
        let feed = active.as_ref().ok_or(())?;
        let token = payload["auth"]["token"].as_str().ok_or(())?;
        if !bool::from(token.as_bytes().ct_eq(feed.token.as_bytes()))
            || payload["provider"]["appid"].as_u64() != Some(730)
        {
            return Err(());
        }
        let Some(map) = payload["map"]["name"]
            .as_str()
            .filter(|map| !map.is_empty())
        else {
            return Ok(false);
        };
        if payload["player"]["activity"]
            .as_str()
            .is_some_and(|activity| {
                activity.eq_ignore_ascii_case("menu") || activity.eq_ignore_ascii_case("loading")
            })
        {
            return Ok(false);
        }
        // Retain only fields the recorder consumes. The player may change while spectating.
        let player = &payload["player"]["steamid"];
        let player = player
            .as_u64()
            .or_else(|| player.as_str()?.parse().ok())
            .filter(|id| *id > 0);
        feed.updates.send_replace(Some(GameUpdate {
            received: Instant::now(),
            map: map.to_owned(),
            round: active_round(payload),
            player,
        }));
        Ok(true)
    }
}

fn active_round(payload: &Value) -> Option<i64> {
    if !matches!(
        payload["round"]["phase"].as_str(),
        Some("freezetime" | "live")
    ) {
        // At round end the map score can already include the round just played.
        // Wait for a fresh live or freeze-time observation before starting OBS.
        return None;
    }
    let score = |side: &str| {
        payload["map"][side]["score"]
            .as_i64()
            .filter(|score| *score >= 0)
    };
    score("team_ct")?
        .checked_add(score("team_t")?)?
        .checked_add(1)
}

impl Drop for PlaybackFeed<'_> {
    fn drop(&mut self) {
        *self.source.0.lock().expect("playback GSI feed") = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_the_current_authenticated_demo_opens_the_playback_gate() {
        let playback = PlaybackGsi::default();
        let token = "a".repeat(64);
        let old_token = "f".repeat(64);
        let feed = playback.begin(token.clone()).unwrap();
        assert!(playback.begin(old_token.clone()).is_err());

        let mut message = json!({
            "auth":{"token":old_token},
            "provider":{"appid":730},
            "map":{"name":"de_mirage","round":1,"team_ct":{"score":0},"team_t":{"score":1}},
            "round":{"phase":"live"},
            "player":{"steamid":"76561198000000000","activity":"playing"}
        });
        assert!(playback.notify(&message).is_err());
        message["auth"]["token"] = json!(token);
        message["player"]["activity"] = json!("menu");
        assert_eq!(playback.notify(&message), Ok(false));
        assert!(feed.updates.borrow().is_none());

        message["player"]["activity"] = json!("playing");
        let sent = Instant::now();
        assert_eq!(playback.notify(&message), Ok(true));
        {
            let observed = feed.updates.borrow();
            let update = observed.as_ref().unwrap();
            assert!(update.received >= sent);
            assert_eq!(update.map, "de_mirage");
            assert_eq!(update.round, Some(2));
            assert_eq!(update.player, Some(76561198000000000));
        }
        drop(feed);

        let next = playback.begin(old_token).unwrap();
        assert!(playback.notify(&message).is_err());
        assert!(next.updates.borrow().is_none());
    }

    #[test]
    fn scores_and_phase_resolve_one_active_round() {
        let mut payload = json!({
            "map":{"round":2,"team_ct":{"score":1},"team_t":{"score":1}},
            "round":{"phase":"live"}
        });
        assert_eq!(active_round(&payload), Some(3));
        assert_ne!(active_round(&payload), Some(2));
        payload["round"]["phase"] = json!("over");
        assert_eq!(active_round(&payload), None);
        payload["round"]["phase"] = json!("freezetime");
        assert_eq!(active_round(&payload), Some(3));
        payload["map"]["team_ct"]["score"] = Value::Null;
        assert_eq!(active_round(&payload), None);
    }
}
