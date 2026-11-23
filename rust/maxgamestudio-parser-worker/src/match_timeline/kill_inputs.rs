//! Read the recording-specific events and only the spatial samples their tags need.

use super::{highlights::is_knife, kill_action_tags as tags, kill_compilation};
use crate::{
    analysis_events::{self, EventKind},
    demo_inspection::{named_column, player_name},
    round_windows::{field, integer},
};
use parser::{
    first_pass::prop_controller::{NAME_ID, TICK_ID},
    parse_demo::DemoOutput,
    second_pass::{
        game_events::GameEvent,
        variants::{VarVec, Variant},
    },
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) const PROPERTIES: [&str; 4] = ["X", "Y", "Z", "yaw"];

pub(super) struct Sources<'a> {
    kills: BTreeMap<&'a str, Vec<RawKill<'a>>>,
    fires: BTreeMap<&'a str, Vec<(i64, String)>>,
    blinds: BTreeMap<&'a str, Vec<tags::BlindSample>>,
    wanted: BTreeMap<i32, BTreeSet<&'a str>>,
}

struct RawKill<'a> {
    round: i64,
    event: &'a GameEvent,
    victim: &'a str,
    victim_steamid: String,
    weapon: String,
    headshot: bool,
}

pub(super) struct TaggedKill<'a> {
    kill: &'a RawKill<'a>,
    tags: Vec<String>,
    pub(super) ending_knife: bool,
}

impl TaggedKill<'_> {
    pub(super) fn source_event(&self) -> &GameEvent {
        self.kill.event
    }

    pub(super) fn as_clip_input(&self) -> kill_compilation::Kill<'_> {
        kill_compilation::Kill {
            round: self.kill.round,
            tick: i64::from(self.kill.event.tick),
            victim: self.kill.victim,
            victim_steamid: &self.kill.victim_steamid,
            weapon: &self.kill.weapon,
            headshot: self.kill.headshot,
            tags: &self.tags,
        }
    }
}

impl<'a> Sources<'a> {
    pub(super) fn collect(
        events: &'a [GameEvent],
        match_start: i32,
        requested: &BTreeSet<&str>,
        freezes: &BTreeMap<i64, i64>,
    ) -> Self {
        let mut data = Self {
            kills: BTreeMap::new(),
            fires: BTreeMap::new(),
            blinds: BTreeMap::new(),
            wanted: BTreeMap::new(),
        };
        let flash_duration = ["blind_duration", "duration"].into_iter().find(|key| {
            events.iter().any(|event| {
                event.name == "player_blind" && event.fields.iter().any(|field| field.name == *key)
            })
        });
        for event in events
            .iter()
            .filter(|row| row.tick >= match_start && row.tick > 0)
        {
            if event.name == "player_blind" {
                if let Some((name, column)) = name_field(event, "user_name").zip(flash_duration) {
                    let duration = field(event, column).and_then(finite).unwrap_or_default();
                    data.blinds.entry(name).or_default().push(tags::BlindSample {
                        tick: i64::from(event.tick),
                        duration,
                    });
                }
                continue;
            }
            if event.name == "weapon_fire" {
                if let Some(name) =
                    name_field(event, "user_name").filter(|name| requested.contains(name))
                {
                    data.fires
                        .entry(name)
                        .or_default()
                        .push((i64::from(event.tick), weapon(event)));
                }
                continue;
            }
            if event.name != "player_death" {
                continue;
            }
            let Some(row) = analysis_events::project(event) else {
                continue;
            };
            let EventKind::Death { attacker, victim, headshot, .. } = row.kind else {
                continue;
            };
            let (Some(actor), Some(victim_name)) = (attacker.name, victim.name) else {
                continue;
            };
            if actor == victim_name || !requested.contains(actor) {
                continue;
            }
            let tick = i64::from(event.tick);
            let raw_round = row.total_rounds_played.unwrap_or_default().saturating_add(1);
            // In a transition, the engine may advance the counter before the next freeze ends.
            let round = if raw_round > 1
                && freezes.get(&raw_round).is_some_and(|freeze| tick < *freeze)
            {
                raw_round - 1
            } else {
                raw_round
            };
            let weapon = weapon(event);
            for offset in tags::ATTACKER_LOOKBACKS {
                if offset > 16 && !matches!(weapon.as_str(), "awp" | "ssg08") {
                    continue;
                }
                let sample_tick = event.tick - offset as i32;
                if sample_tick > 0 {
                    data.wanted.entry(sample_tick).or_default().insert(actor);
                }
            }
            data.wanted
                .entry(event.tick)
                .or_default()
                .insert(victim_name);
            data.kills.entry(actor).or_default().push(RawKill {
                round,
                event,
                victim: victim_name,
                victim_steamid: victim.steamid64.map(|id| id.to_string()).unwrap_or_default(),
                weapon,
                headshot,
            });
        }
        data
    }

    pub(super) fn ticks(&self) -> impl Iterator<Item = i32> + '_ {
        self.wanted.keys().copied()
    }

    pub(super) fn highlight_ticks(&self) -> BTreeSet<i32> {
        let mut ticks = BTreeSet::new();
        for kills in self.kills.values() {
            for kill in kills {
                // The old analyzer schedules -8 and -16 for all kills. The two
                // older fallbacks are explicitly scheduled only for snipers.
                let offsets = if matches!(kill.weapon.as_str(), "awp" | "ssg08") {
                    &[0, 8, 16, 24, 32][..]
                } else {
                    &[0, 8, 16][..]
                };
                ticks.extend(
                    offsets
                        .iter()
                        .map(|offset| (kill.event.tick - offset).max(0)),
                );
            }
        }
        ticks
    }

    pub(super) fn snapshots<'b>(&self, output: Option<&'b DemoOutput>) -> SpatialRows<'b> {
        let mut rows = SpatialRows::new();
        let Some(output) = output else {
            return rows;
        };
        let column = |id| output.df.get(&id).and_then(|column| column.data.as_ref());
        let (Some(VarVec::I32(ticks)), Some(VarVec::String(names))) =
            (column(TICK_ID), column(NAME_ID))
        else {
            return rows;
        };
        let [x, y, z, yaw] = PROPERTIES.map(|name| named_column(output, name));
        for (index, (tick, name)) in ticks.iter().zip(names).enumerate() {
            let (Some(tick), Some(name)) = (tick, name.as_deref().and_then(player_name)) else {
                continue;
            };
            if self.wanted.get(tick).is_some_and(|wanted| {
                wanted.contains(name) || wanted.iter().any(|target| same_player(target, name))
            }) {
                rows.entry(*tick).or_default().insert(
                    name,
                    tags::Snapshot {
                        position: tags::Position {
                            x: sample(x, index),
                            y: sample(y, index),
                            z: sample(z, index),
                        },
                        yaw: sample(yaw, index),
                        // The pinned wheel's `vel_z` is absent. Legacy jumping uses a Z difference.
                        vel_z: None,
                    },
                );
            }
        }
        rows
    }

    pub(super) fn kills_for(
        &self,
        actor: &str,
        rows: &SpatialRows<'_>,
        decisions: &BTreeMap<i64, i64>,
    ) -> Vec<TaggedKill<'_>> {
        let kills = self.kills.get(actor).map(Vec::as_slice).unwrap_or_default();
        let mut first_tick = BTreeMap::<i64, i64>::new();
        for kill in kills {
            let tick = i64::from(kill.event.tick);
            first_tick
                .entry(kill.round)
                .and_modify(|first| *first = (*first).min(tick))
                .or_insert(tick);
        }
        kills
            .iter()
            .map(|kill| {
                let tick = i64::from(kill.event.tick);
                let ending_knife = decisions.get(&kill.round) == Some(&tick)
                    && first_tick[&kill.round] < tick
                    && is_knife(&kill.weapon);
                let at = |tick: i64, name: &str| {
                    let players = rows.get(&i32::try_from(tick).ok()?)?;
                    players
                        .get(name)
                        .or_else(|| players.iter().find_map(|(label, row)| same_player(label, name).then_some(row)))
                        .copied()
                };
                let snapshots = tags::Snapshots {
                    attacker: tags::ATTACKER_LOOKBACKS.map(|offset| at(tick - offset, actor)),
                    victim: at(tick, kill.victim),
                };
                let event = kill.event;
                let flag = |key| {
                    field(event, key).is_some_and(|value| match value {
                        Variant::Bool(value) => *value,
                        value => integer(value).is_some_and(|value| value != 0),
                    })
                };
                let count = |key| field(event, key).and_then(integer).unwrap_or_default();
                let tags = tags::tags(
                    &tags::Kill {
                        tick,
                        weapon: &kill.weapon,
                        headshot: kill.headshot,
                        noscope: flag("noscope"),
                        through_smoke: flag("thrusmoke"),
                        attacker_blind: flag("attackerblind"),
                        assisted_flash: flag("assistedflash"),
                        attacker_in_air: flag("attackerinair") || flag("attacker_in_air"),
                        penetrated: count("penetrated"),
                        penetrated_objects: count("penetrated_objects"),
                        attacker_position: position(event, ["attacker_X", "attacker_Y", "attacker_Z"]),
                        victim_position: position(event, ["user_X", "user_Y", "user_Z"]),
                    },
                    &snapshots,
                    self.blinds.get(kill.victim).map(Vec::as_slice).unwrap_or_default(),
                );
                TaggedKill { kill, tags, ending_knife }
            })
            .collect()
    }

    pub(super) fn fires_for(&self, actor: &str) -> Vec<kill_compilation::Fire<'_>> {
        self.fires
            .get(actor)
            .into_iter()
            .flatten()
            .map(|(tick, weapon)| kill_compilation::Fire { tick: *tick, weapon })
            .collect()
    }
}

pub(super) type SpatialRows<'a> = BTreeMap<i32, BTreeMap<&'a str, tags::Snapshot>>;

fn sample(column: Option<&VarVec>, index: usize) -> Option<f64> {
    let VarVec::F32(values) = column? else {
        return None;
    };
    values.get(index)?.filter(|value| value.is_finite()).map(f64::from)
}

pub(super) fn same_player(first: &str, second: &str) -> bool {
    if first.is_ascii() && second.is_ascii() {
        return first.eq_ignore_ascii_case(second);
    }
    first.to_lowercase() == second.to_lowercase()
}

fn position(event: &GameEvent, fields: [&str; 3]) -> tags::Position {
    let [x, y, z] = fields.map(|key| field(event, key).and_then(finite));
    tags::Position { x, y, z }
}

fn finite(value: &Variant) -> Option<f64> {
    let number = match value {
        Variant::F32(value) => f64::from(*value),
        value => integer(value)? as f64,
    };
    number.is_finite().then_some(number)
}

fn name_field<'a>(event: &'a GameEvent, key: &str) -> Option<&'a str> {
    let Variant::String(name) = field(event, key)? else {
        return None;
    };
    player_name(name)
}

fn weapon(event: &GameEvent) -> String {
    let name = name_field(event, "weapon").unwrap_or_default().to_lowercase();
    name.strip_prefix("weapon_").map(str::to_owned).unwrap_or(name)
}
