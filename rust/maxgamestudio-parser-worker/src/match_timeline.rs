//! Build the named replay events from a single tick-ordered parser scan.

mod damage_stats;
mod match_facts;
mod player_stats;
mod round_economy;
mod round_score;

use crate::{
    analysis_events::{self, AnalysisRow, BombKind, EventKind, Position},
    replay_io,
    round_windows::{end_tick, windows, RoundWindow},
    Failure,
};
use parser::second_pass::parser_settings::create_huffman_lookup_table;
use serde_json::{json, Value};
use std::{collections::HashSet, path::Path};

pub(crate) fn inspect(
    path: &Path,
    tick_rate: f64,
    include_match_facts: bool,
) -> Result<Value, Failure> {
    if !tick_rate.is_finite() || !(1.0..=256.0).contains(&tick_rate) {
        return Err(Failure::request("unsupported Demo tick rate"));
    }
    replay_io::with_demo_bytes(path, |bytes| {
        let huffman = create_huffman_lookup_table();
        let demo_end = end_tick(bytes)?;
        let events = analysis_events::scan(bytes, &huffman, include_match_facts)?;
        let (start, freeze_ticks, windows) = windows(&events, tick_rate, demo_end);
        let mut rounds: Vec<_> = (0..windows.len()).map(|_| Timeline::default()).collect();
        for event in &events {
            let Some(row) = analysis_events::project(event) else {
                continue;
            };
            let tick = i64::from(row.tick);
            if tick <= 0 {
                continue;
            }
            if let Some(index) = windows
                .iter()
                .position(|window| window.start_tick <= tick && tick <= window.end_tick)
            {
                rounds[index].push(row, &event.name);
            }
        }
        let mut rounds = rounds
            .into_iter()
            .zip(&windows)
            .map(|(timeline, window)| timeline.finish(window, tick_rate))
            .collect::<Vec<_>>();
        let mut result = json!({"match_start_tick":start,"demo_end_tick":demo_end});
        if include_match_facts {
            result["match_facts"] = match_facts::collect(
                bytes,
                &huffman,
                &events,
                demo_end,
                &freeze_ticks,
                &windows,
                &mut rounds,
            )?;
        }
        result["rounds"] = Value::Array(rounds);
        Ok(result)
    })
}

#[derive(Default)]
struct Timeline {
    events: Vec<Value>,
    bombs: [Vec<Value>; 5],
    grenades: [Vec<Value>; 5],
    shots: Vec<Value>,
    releases: Vec<(&'static str, Vec<Value>)>,
}

impl Timeline {
    fn push(&mut self, row: AnalysisRow<'_>, source_event: &str) {
        let tick = row.tick;
        match row.kind {
            EventKind::Death {
                attacker,
                victim,
                assister,
                weapon,
                headshot,
            } => {
                let Some(target) = victim.name else { return };
                let mut event = json!({
                    "type":"kill", "tick":tick, "actor":attacker.name.unwrap_or("World"),
                    "target":target, "assister":assister.name.unwrap_or_default(), "weapon":weapon,
                    "headshot":headshot,
                });
                add_position(&mut event, "actor_", attacker.position);
                add_position(&mut event, "target_", victim.position);
                self.events.push(event);
            }
            EventKind::Bomb { kind, player, site } => {
                let (index, label) = match kind {
                    BombKind::Plant => (0, "plant"),
                    BombKind::Defuse => (1, "defuse"),
                    BombKind::Explode => (2, "explode"),
                    BombKind::Drop => (3, "bomb_drop"),
                    BombKind::Pickup => (4, "bomb_pickup"),
                };
                let mut event = json!({
                    "type":label, "tick":tick,
                    "actor":player.name.unwrap_or_default(), "site":site.map(|site| site.to_string()).unwrap_or_default(),
                });
                add_position(&mut event, "", player.position);
                self.bombs[index].push(event);
            }
            EventKind::WeaponFire {
                player,
                weapon,
                is_bullet,
                grenade_kind,
                yaw,
                pitch,
            } => {
                let Some(actor) = player.name else { return };
                if is_bullet {
                    let mut shot = json!({
                        "tick":tick,"actor":actor,"weapon":weapon,
                        "yaw":yaw.unwrap_or_default(),"pitch":pitch.unwrap_or_default(),
                    });
                    add_position(&mut shot, "", player.position);
                    self.shots.push(shot);
                }
                if let Some(kind) = grenade_kind {
                    let mut release = json!({
                        "tick":tick, "kind":kind, "actor":actor,
                        "steamid64":player.steamid64.map(|id| id.to_string()).unwrap_or_default(),
                    });
                    add_position(&mut release, "", player.position);
                    if let Some((_, releases)) =
                        self.releases.iter_mut().find(|(name, _)| *name == kind)
                    {
                        releases.push(release);
                    } else {
                        self.releases.push((kind, vec![release]));
                    }
                }
            }
            EventKind::GrenadeDetonation {
                player,
                kind,
                position,
            } => {
                // The released workspace appends each family before its stable tick/type sort.
                let family = match source_event {
                    "hegrenade_detonate" => 0,
                    "inferno_startburn" => 1,
                    "molotov_detonate" => 2,
                    "smokegrenade_detonate" => 3,
                    "flashbang_detonate" => 4,
                    _ => unreachable!("projected grenade detonation"),
                };
                let mut event = json!({
                    "type":"grenade", "kind":kind, "tick":tick,
                    "actor":player.name.unwrap_or_default(),
                });
                add_position(&mut event, "", position);
                self.grenades[family].push(event);
            }
        }
    }

    fn finish(mut self, window: &RoundWindow, tick_rate: f64) -> Value {
        let mut planted_site = Value::String(String::new());
        for mut event in self.bombs.into_iter().flatten() {
            if event["type"] == "plant" && event["site"] != "" {
                planted_site = event["site"].clone();
            } else if event["site"] == "" {
                event["site"] = planted_site.clone();
            }
            self.events.push(event);
        }
        self.events.extend(self.grenades.into_iter().flatten());
        self.events.sort_by(|left, right| {
            left["tick"]
                .as_i64()
                .cmp(&right["tick"].as_i64())
                .then_with(|| left["type"].as_str().cmp(&right["type"].as_str()))
        });
        let mut seen = HashSet::new();
        let mut terminal_seen = HashSet::new();
        let mut events = Vec::new();
        for mut event in self.events {
            let tick = event["tick"].as_i64().expect("validated event tick");
            let kind = event["type"].as_str().expect("validated event type");
            if matches!(kind, "explode" | "defuse") && !terminal_seen.insert(kind.to_owned()) {
                continue;
            }
            if !seen.insert((
                kind.to_owned(),
                tick,
                event["actor"].as_str().unwrap_or_default().to_lowercase(),
                event["target"].as_str().unwrap_or_default().to_lowercase(),
                event["kind"].as_str().unwrap_or_default().to_owned(),
            )) {
                continue;
            }
            let seconds = (((tick - window.freeze_end_tick) as f64 / tick_rate).round_ties_even()
                as i64)
                .max(0);
            event["time_text"] = json!(format!("{:02}:{:02}", seconds / 60, seconds % 60));
            events.push(event);
        }
        let carrier = events.iter().find_map(|event| {
            let kind = event["type"].as_str().unwrap_or_default();
            if matches!(kind, "bomb_pickup" | "bomb_drop" | "plant") {
                event["actor"].as_str().filter(|name| !name.is_empty())
            } else {
                None
            }
        });
        let mut result = json!(window);
        result["bomb_initial_carrier"] = json!(carrier.unwrap_or_default());
        result["events"] = json!(events);
        result["shots"] = json!(self.shots);
        result["grenade_releases"] = json!(self
            .releases
            .into_iter()
            .flat_map(|(_, releases)| releases)
            .collect::<Vec<_>>());
        result
    }
}

fn add_position(event: &mut Value, prefix: &str, position: Option<Position>) {
    if let Some(position) = position {
        event[format!("{prefix}x")] = json!(position.x);
        event[format!("{prefix}y")] = json!(position.y);
        if let Some(z) = position.z {
            event[format!("{prefix}z")] = json!(z);
        }
    }
}
