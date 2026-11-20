//! Track the current ARAM session from the local League client's game events.

use crate::resg::select_resg_champion;
use serde_json::Value;

#[derive(Default)]
struct Snapshot {
    phase: Option<String>,
    gameflow: Value,
    champ_select: Value,
    puuid: Option<String>,
}

#[derive(Default)]
pub struct LeagueSession {
    generation: u64,
    snapshot: Option<Snapshot>,
}

impl LeagueSession {
    /// Allocate a new session for the current live connection.
    pub fn connect(&mut self) -> u64 {
        self.generation += 1;
        self.snapshot = Some(Snapshot::default());
        self.generation
    }

    pub fn disconnect(&mut self, generation: u64) -> bool {
        generation == self.generation && self.snapshot.take().is_some()
    }

    pub fn phase(&self) -> Option<&str> {
        self.snapshot.as_ref()?.phase.as_deref()
    }

    pub fn game_mode(&self) -> Option<&str> {
        self.snapshot.as_ref()?.gameflow["gameData"]["queue"]["gameMode"].as_str()
    }

    pub fn resg_champion(&self) -> Option<u32> {
        let snapshot = self.snapshot.as_ref()?;
        select_resg_champion(
            snapshot.phase.as_deref(),
            &snapshot.gameflow,
            &snapshot.champ_select,
            snapshot.puuid.as_deref(),
        )
    }

    /// Apply an event from the current connection. Ignore stale connections and
    /// routes outside the game's session state.
    pub fn observe(&mut self, generation: u64, frame: &[u8]) -> Result<bool, serde_json::Error> {
        let Some(snapshot) = self
            .snapshot
            .as_mut()
            .filter(|_| generation == self.generation)
        else {
            return Ok(false);
        };
        let mut envelope: Value = serde_json::from_slice(frame)?;
        if envelope[0].as_u64() != Some(8) {
            return Ok(false);
        }
        let Some(event) = envelope.get_mut(2).and_then(Value::as_object_mut) else {
            return Ok(false);
        };
        let data = match event.get("eventType").and_then(Value::as_str) {
            Some("Delete") => Value::Null,
            Some("Create" | "Update") => event.remove("data").unwrap_or_default(),
            _ => return Ok(false),
        };
        match event.get("uri").and_then(Value::as_str) {
            Some("/lol-gameflow/v1/gameflow-phase") => {
                if snapshot.phase.as_deref() == Some("ChampSelect") && data != "ChampSelect" {
                    snapshot.champ_select = Value::Null;
                }
                snapshot.phase = data.as_str().map(str::to_owned);
            }
            Some("/lol-gameflow/v1/session") => {
                let old_id = snapshot.gameflow["gameData"]["gameId"].as_u64();
                let new_id = data["gameData"]["gameId"].as_u64();
                if data.is_null() || (old_id.is_some() && old_id != new_id) {
                    snapshot.champ_select = Value::Null;
                }
                snapshot.gameflow = data;
            }
            Some("/lol-champ-select/v1/session") => snapshot.champ_select = data,
            Some("/lol-summoner/v1/current-summoner") => {
                snapshot.puuid = data["puuid"].as_str().map(str::to_owned);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
}
