//! Build the named replay events from a single tick-ordered parser scan.

mod damage_stats;
mod death_compilation;
mod kill_action_tags;
mod kill_compilation;
mod kill_inputs;
mod match_facts;
mod player_stats;
mod rival_compilation;
mod round_compilation;
mod round_economy;
mod round_score;
mod spec_slots;

use crate::{
    analysis_events::{self, AnalysisRow, BombKind, EventKind, Position},
    replay_io,
    round_windows::{end_tick, windows, RoundWindow},
    Failure,
};
use parser::second_pass::parser_settings::create_huffman_lookup_table;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, HashSet},
    path::Path,
};

#[derive(Debug, Deserialize)]
pub(crate) struct RequestedCompilations {
    players: Vec<String>,
    rounds: Option<Vec<i64>>,
}

pub(crate) fn inspect(
    path: &Path,
    tick_rate: f64,
    include_match_facts: bool,
    compilations: Option<&RequestedCompilations>,
) -> Result<Value, Failure> {
    if !tick_rate.is_finite() || !(1.0..=256.0).contains(&tick_rate) {
        return Err(Failure::request("unsupported Demo tick rate"));
    }
    replay_io::with_demo_bytes(path, |bytes| {
        let huffman = create_huffman_lookup_table();
        let demo_end = end_tick(bytes)?;
        let include_facts = include_match_facts || compilations.is_some();
        let events = analysis_events::scan(bytes, &huffman, include_facts, compilations.is_some())?;
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
        if include_facts {
            let facts = match_facts::collect(
                bytes,
                &huffman,
                &events,
                match_facts::Timing {
                    demo_end,
                    tick_rate,
                    compilations,
                },
                &freeze_ticks,
                &windows,
                &mut rounds,
            )?;
            let team_labels =
                [("team_a_name", "A 队"), ("team_b_name", "B 队")].map(|(key, fallback)| {
                    facts["match_meta"][key]
                        .as_str()
                        .map(player_stats::clean_name)
                        .filter(|name| !name.is_empty())
                        .unwrap_or(fallback)
                });
            for (round, window) in rounds.iter_mut().zip(&windows) {
                let duration = ((window.end_tick - window.freeze_end_tick) as f64 / tick_rate)
                    .round_ties_even()
                    .max(0.0) as i64;
                annotate_round(round, team_labels, duration);
            }
            result["match_facts"] = facts;
        }
        result["rounds"] = Value::Array(rounds);
        Ok(result)
    })
}

fn annotate_round(round: &mut Value, team_labels: [&str; 2], duration: i64) {
    let mut kills = BTreeMap::<&str, (usize, usize)>::new();
    let mut headshot = false;
    let mut plant = None;
    let timeline = round["events"].as_array().expect("projected round events");
    for (index, event) in timeline.iter().enumerate() {
        match event["type"].as_str() {
            Some("kill") => {
                let actor = event["actor"].as_str().unwrap_or_default();
                if !actor.is_empty() && actor != "World" {
                    kills
                        .entry(player_stats::clean_name(actor))
                        .or_insert((0, index))
                        .0 += 1;
                    headshot |= event["headshot"] == true;
                }
            }
            Some("plant") => {
                plant.get_or_insert(event);
            }
            _ => (),
        }
    }
    let (top_player, top_kills) = kills
        .into_iter()
        .max_by_key(|(_, (count, first))| (*count, Reverse(*first)))
        .map_or(("", 0), |(name, (count, _))| (name, count));
    let site = plant
        .and_then(|event| event["site"].as_str())
        .map(player_stats::clean_name)
        .filter(|site| !site.is_empty());
    let winner = match round["winner_team_key"].as_str() {
        Some("a") => team_labels[0],
        Some("b") => team_labels[1],
        _ => "本回合胜方",
    };
    let headline = if top_kills >= 2 {
        let numeral = ["双", "三", "四", "五"][top_kills.min(5) - 2];
        format!("{top_player} {numeral}杀帮助 {winner} 拿下回合")
    } else if let Some(site) = site {
        format!("{winner} 在 {site} 区下包后赢下回合")
    } else {
        format!("{winner} 赢下第 {} 回合", round["round_number"])
    };
    let mut tags = Vec::new();
    if top_kills > 0 {
        tags.push("首杀".to_owned());
    }
    if top_kills >= 2 {
        tags.push(format!("{top_kills}K"));
    }
    if headshot {
        tags.push("爆头".to_owned());
    }
    if plant.is_some() {
        tags.push("下包".to_owned());
    }
    round["site"] = json!(site);
    round["headline"] = json!(headline);
    round["tags"] = json!(tags);
    round["duration_seconds"] = json!(duration);
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
