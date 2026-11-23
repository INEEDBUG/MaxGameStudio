//! Evidence for the ordinary highlight branch from the recording scan and selected rows.

use super::{
    highlights::{self, Encounter, Hurt, KillFacts, Recording, RoundFacts},
    kill_action_tags::Position,
    kill_compilation::{Fire, Kill},
    kill_inputs::{same_player, TaggedKill},
    player_stats::Player,
    round_score::{RoundScores, Side, TeamKey},
};
use crate::{
    demo_inspection::{named_column, player_name, team_at_column, CONTROLLER_TEAM},
    round_windows::{field, integer, winner},
};
use parser::{
    first_pass::prop_controller::{NAME_ID, TICK_ID},
    parse_demo::DemoOutput,
    second_pass::{
        game_events::GameEvent,
        variants::{VarVec, Variant},
    },
};
use serde_json::Value;
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
};

pub(super) const PROPERTIES: &[&str] = &["health", "is_alive"];

pub(super) struct Events<'a> {
    deaths: Vec<Death<'a>>,
    hurt: BTreeMap<&'a str, Vec<Hit<'a>>>,
    awp: BTreeMap<&'a str, Vec<i64>>,
    defuses: Vec<Defuse<'a>>,
    defuse_windows: BTreeMap<(&'a str, i64), [i64; 2]>,
    bomb_explodes: BTreeMap<i64, i64>,
    /// Scoreboard winners use round_end's already advanced round number.
    winners: Vec<(i64, i32)>,
    score_observed: bool,
}

struct Death<'a> {
    round: i64,
    tick: i64,
    attacker: Option<&'a str>,
    victim: Option<&'a str>,
    attacker_team: Option<i32>,
    victim_team: Option<i32>,
}

struct Hit<'a> {
    observation: Hurt<'a>,
    round: i64,
    event_round: i64,
    attacker: Option<&'a str>,
    attacker_team: Option<i32>,
    hostile: bool,
}

struct Defuse<'a> {
    actor: &'a str,
    tick: i64,
    round: i64,
    plant: Option<i64>,
}

impl<'a> Events<'a> {
    pub(super) fn collect(
        events: &'a [GameEvent],
        match_start: i32,
        freezes: &BTreeMap<i64, i64>,
    ) -> Self {
        let after_start = || events.iter().filter(|event| event.tick >= match_start);
        let has = |event_name: &str, key: &str| {
            after_start().any(|event| {
                event.name == event_name && event.fields.iter().any(|prop| prop.name == key)
            })
        };
        let round_col = |name| has(name, "total_rounds_played");
        let hurt_rounds = round_col("player_hurt");
        let hurt_teams = has("player_hurt", "attacker_team") && has("player_hurt", "user_team");
        let hurt_damage = ["dmg_health", "damage", "health_damage"]
            .into_iter()
            .find(|key| has("player_hurt", key));
        let death_teams = round_col("player_death");
        let begin_rounds = round_col("bomb_begindefuse");
        let defused_rounds = round_col("bomb_defused");
        let exploded_rounds = round_col("bomb_exploded");
        let end_rounds = round_col("round_end");
        let score_observed = has("round_end", "winner");
        let mut data = Self {
            deaths: Vec::new(),
            hurt: BTreeMap::new(),
            awp: BTreeMap::new(),
            defuses: Vec::new(),
            defuse_windows: BTreeMap::new(),
            bomb_explodes: BTreeMap::new(),
            winners: Vec::new(),
            score_observed,
        };
        let mut plant_ticks = Vec::new();
        let mut beginnings = BTreeMap::<&str, Vec<(i64, i64)>>::new();
        let mut sequence = 0;
        for event in after_start() {
            let tick = i64::from(event.tick);
            match event.name.as_str() {
                "player_death" => data.deaths.push(Death {
                    round: event_round(event),
                    tick,
                    attacker: name(event, "attacker_name"),
                    victim: name(event, "user_name"),
                    attacker_team: death_teams.then(|| team(event, "attackerteam")).flatten(),
                    victim_team: death_teams.then(|| team(event, "userteam")).flatten(),
                }),
                "player_hurt" => {
                    let Some(victim) = name(event, "user_name") else {
                        continue;
                    };
                    let attacker = name(event, "attacker_name");
                    let (attacker_team, victim_team) = if hurt_teams {
                        (team(event, "attacker_team"), team(event, "user_team"))
                    } else {
                        (None, None)
                    };
                    let event_round = if hurt_rounds { event_round(event) } else { 0 };
                    let round = if event_round > 0 {
                        event_round
                    } else {
                        freezes
                            .iter()
                            .filter(|(_, frozen)| **frozen <= tick)
                            .max_by_key(|(round, frozen)| (**frozen, **round))
                            .map_or(0, |(&round, _)| round)
                    };
                    let observation = Hurt {
                        tick,
                        damage: hurt_damage
                            .and_then(|key| field(event, key))
                            .and_then(integer)
                            .unwrap_or_default(),
                        weapon: key(event, "weapon"),
                    };
                    data.hurt.entry(victim).or_default().push(Hit {
                        observation,
                        round,
                        event_round,
                        attacker,
                        attacker_team,
                        hostile: attacker.is_some_and(|name| name != victim)
                            && (attacker_team.is_none() || attacker_team != victim_team),
                    });
                }
                "weapon_fire" | "item_pickup" => {
                    let prop = if event.name == "item_pickup" {
                        "item"
                    } else {
                        "weapon"
                    };
                    if key(event, prop).eq_ignore_ascii_case("awp") {
                        if let Some(name) = name(event, "user_name") {
                            data.awp.entry(name).or_default().push(tick);
                        }
                    }
                }
                "bomb_planted" if tick > 0 => plant_ticks.push(tick),
                "bomb_begindefuse" if begin_rounds && tick > 0 => {
                    let round = event_round(event);
                    if let Some(actor) = name(event, "user_name").filter(|_| round > 0) {
                        let items = beginnings.entry(actor).or_default();
                        if !items.iter().any(|&(number, _)| number == round) {
                            items.push((round, tick));
                        }
                    }
                }
                "bomb_defused" if tick > 0 => {
                    if let Some(actor) = name(event, "user_name") {
                        if let Some((round, begin)) = beginnings.get(actor).and_then(|by_round| {
                            by_round.iter().copied().find(|&(round, begin)| {
                                begin <= tick && !data.defuse_windows.contains_key(&(actor, round))
                            })
                        }) {
                            data.defuse_windows.insert((actor, round), [begin, tick]);
                        }
                    }
                    if let Some(actor) = ["user_name", "defuser", "defuser_name", "player_name"]
                        .into_iter()
                        .find_map(|prop| name(event, prop))
                    {
                        let numbered = if defused_rounds {
                            event_round(event)
                        } else {
                            field(event, "round").and_then(integer).unwrap_or_default()
                        };
                        let round = if numbered > 0 {
                            numbered
                        } else {
                            freezes
                                .iter()
                                .rev()
                                .find(|(_, frozen)| **frozen <= tick)
                                .map_or(0, |(&round, _)| round)
                        };
                        let earlier = plant_ticks.partition_point(|&plant| plant < tick);
                        let plant = plant_ticks[..earlier].last().copied();
                        data.defuses.push(Defuse {
                            actor,
                            tick,
                            round,
                            plant,
                        });
                    }
                }
                "bomb_exploded" if exploded_rounds && tick > 0 => {
                    let round = event_round(event);
                    if round > 0 {
                        data.bomb_explodes.entry(round).or_insert(tick);
                    }
                }
                "round_end" => {
                    if let Some(side) = field(event, "winner").and_then(winner) {
                        let round = if end_rounds {
                            field(event, "total_rounds_played")
                                .and_then(integer)
                                .unwrap_or_default()
                        } else {
                            sequence += 1;
                            sequence
                        };
                        if round > 0 {
                            data.winners.push((round, side as i32));
                        }
                    }
                }
                _ => {}
            }
        }
        data
    }

    pub(super) fn defuse_ticks<'b>(
        &'b self,
        requested: &'b BTreeSet<&str>,
    ) -> impl Iterator<Item = i32> + 'b {
        self.defuses.iter().filter_map(|defuse| {
            (defuse.plant.is_some() && requested.iter().any(|name| same_player(defuse.actor, name)))
                .then_some(defuse.tick as i32)
        })
    }
}

pub(super) struct Samples<'a> {
    cohorts: BTreeMap<i32, Vec<Pawn<'a>>>,
    equipment: BTreeMap<i64, [i64; 2]>,
    teams: BTreeMap<String, BTreeMap<i64, i32>>,
}

#[derive(Clone, Copy)]
struct Pawn<'a> {
    name: &'a str,
    position: Position,
    yaw: Option<f64>,
    health: Option<i64>,
    alive: bool,
    team: Option<i32>,
    known: bool,
}

#[derive(Default)]
struct Freeze<'a> {
    equipment: [i64; 2],
    teams: BTreeMap<&'a str, TeamAtFreeze>,
}

struct TeamAtFreeze {
    alive: Option<Option<i32>>,
    first: Option<i32>,
}

impl<'a> Samples<'a> {
    pub(super) fn collect(
        output: &'a DemoOutput,
        ticks: &BTreeSet<i32>,
        economy_ticks: &BTreeMap<i32, i64>,
    ) -> Option<Self> {
        let column = |id| output.df.get(&id).and_then(|column| column.data.as_ref());
        let (Some(VarVec::I32(sample_ticks)), Some(VarVec::String(names))) =
            (column(TICK_ID), column(NAME_ID))
        else {
            return None;
        };
        let [x, y, z, yaw] = ["X", "Y", "Z", "yaw"].map(|prop| named_column(output, prop));
        let health = named_column(output, "health")?;
        let VarVec::Bool(alive) = named_column(output, "is_alive")? else {
            return None;
        };
        let equipment = named_column(output, "current_equip_value");
        let team = named_column(output, "team_num");
        let controller = named_column(output, CONTROLLER_TEAM);
        if (team.is_none() && controller.is_none()) || [x, y, z, yaw].iter().any(Option::is_none) {
            return None;
        }
        let mut cohorts = BTreeMap::<i32, Vec<Pawn>>::new();
        let mut freezes = BTreeMap::<i32, Freeze>::new();
        for (index, &tick) in sample_ticks.iter().enumerate() {
            let Some(tick) = tick else {
                continue;
            };
            if !ticks.contains(&tick) && !economy_ticks.contains_key(&tick) {
                continue;
            }
            let pawn_team = team_at_column(team, index);
            let controller_team = team_at_column(controller, index);
            let team = pawn_team.or(controller_team);
            let alive = alive.get(index).copied().flatten().unwrap_or_default();
            let label = names
                .get(index)
                .and_then(Option::as_deref)
                .and_then(player_name);
            if economy_ticks.contains_key(&tick) {
                let freeze = freezes.entry(tick).or_default();
                if alive {
                    if let (Some(team), Some(equipment)) = (team, amount(equipment, index)) {
                        freeze.equipment[(team - 2) as usize] += equipment;
                    }
                }
                if let Some(name) = label {
                    let row = freeze.teams.entry(name).or_insert(TeamAtFreeze {
                        alive: None,
                        first: team,
                    });
                    if alive {
                        row.alive.get_or_insert(team);
                    }
                }
            }
            if let Some(name) = label.filter(|_| ticks.contains(&tick)) {
                let position = Position {
                    x: real(x, index),
                    y: real(y, index),
                    z: real(z, index),
                };
                let known = !(pawn_team.is_none()
                    && controller_team.is_some()
                    && (position.x.is_none() || position.y.is_none()));
                let row = Pawn {
                    name,
                    position,
                    yaw: real(yaw, index),
                    health: amount(Some(health), index),
                    alive,
                    team,
                    known,
                };
                let by_tick = cohorts.entry(tick).or_default();
                if let Some(previous) = by_tick.iter_mut().find(|r| r.name == name) {
                    *previous = row;
                } else {
                    by_tick.push(row);
                }
            }
        }
        let mut equipment = BTreeMap::new();
        let mut teams = BTreeMap::<String, BTreeMap<i64, i32>>::new();
        for (tick, observed) in freezes {
            let round = economy_ticks[&tick];
            equipment.insert(round, observed.equipment);
            for (name, value) in observed.teams {
                if let Some(team) = value.alive.flatten().or(value.first) {
                    teams
                        .entry(name.to_lowercase())
                        .or_default()
                        .entry(round)
                        .or_insert(team);
                }
            }
        }
        Some(Self {
            cohorts,
            equipment,
            teams,
        })
    }

    fn at(&self, tick: i64) -> Option<&[Pawn<'a>]> {
        self.cohorts
            .get(&i32::try_from(tick).ok()?)
            .map(Vec::as_slice)
    }
}

fn name<'a>(event: &'a GameEvent, prop: &str) -> Option<&'a str> {
    let Variant::String(value) = field(event, prop)? else {
        return None;
    };
    player_name(value)
}

fn key<'a>(event: &'a GameEvent, prop: &str) -> &'a str {
    let raw = name(event, prop).unwrap_or_default();
    raw.strip_prefix("weapon_").unwrap_or(raw)
}

fn team(event: &GameEvent, prop: &str) -> Option<i32> {
    i32::try_from(field(event, prop).and_then(integer)?).ok()
}

fn event_round(event: &GameEvent) -> i64 {
    field(event, "total_rounds_played")
        .and_then(integer)
        .unwrap_or_default()
        .saturating_add(1)
}

fn real(column: Option<&VarVec>, index: usize) -> Option<f64> {
    let VarVec::F32(values) = column? else {
        return None;
    };
    values
        .get(index)?
        .filter(|value| value.is_finite())
        .map(f64::from)
}

fn amount(column: Option<&VarVec>, index: usize) -> Option<i64> {
    match column? {
        VarVec::I32(values) => values.get(index)?.map(i64::from),
        VarVec::U32(values) => values.get(index)?.map(i64::from),
        _ => None,
    }
}

/// A selected tick and event batch belong to the same Demo.
pub(super) struct Source<'a, 'sample> {
    pub(super) events: Events<'a>,
    pub(super) samples: Samples<'sample>,
    pub(super) mid_round_buffer_ticks: Option<i64>,
}

pub(super) struct ClipInput<'a> {
    pub(super) player: &'a Player<'a>,
    pub(super) kills: &'a [Kill<'a>],
    pub(super) tagged: &'a [TaggedKill<'a>],
    pub(super) fires: &'a [Fire<'a>],
    pub(super) freezes: &'a BTreeMap<i64, i64>,
    pub(super) decisions: &'a BTreeMap<i64, i64>,
    pub(super) scores: &'a RoundScores,
    pub(super) recording: Recording<'a>,
}

impl Source<'_, '_> {
    pub(super) fn clips(&self, input: ClipInput<'_>) -> Vec<Value> {
        let actor = input.player.name;
        let Some(teams) = self.target_teams(input.player, input.scores, input.freezes) else {
            return Vec::new();
        };
        let scores = legacy_scores(&self.events, &teams);
        let final_score = scores.values().copied().max_by_key(|[a, b]| a + b);
        let completed = final_score.map_or(0, |[a, b]| a + b);
        let cap_rounds = if input.decisions.is_empty() {
            freeze_caps(input.freezes)
        } else {
            input.decisions.clone()
        };
        let last_decision = cap_rounds.keys().next_back().copied().unwrap_or_default();
        let terminal = completed.max(last_decision);
        let recording = Recording {
            terminal_round: (terminal > 0).then_some(terminal),
            mid_round_buffer_ticks: self.mid_round_buffer_ticks,
            ..input.recording
        };

        let mut buckets = BTreeMap::<i64, Vec<(&Kill, &TaggedKill)>>::new();
        for (kill, tagged) in input.kills.iter().zip(input.tagged) {
            buckets.entry(kill.round).or_default().push((kill, tagged));
        }
        let mut clips = Vec::new();
        for (number, kills) in buckets.into_iter().filter(|(_, kills)| kills.len() >= 2) {
            let score = scores.get(&number).copied();
            if post_match(number, score, completed, final_score) {
                continue;
            }
            if self.defuse_overlay(actor, number) != Some(false) {
                continue;
            }
            let labels: Vec<_> = kills
                .iter()
                .map(|(kill, _)| display_weapon(kill.weapon))
                .collect();
            let contexts: Option<Vec<_>> = kills
                .iter()
                .zip(&labels)
                .map(|(&(kill, tagged), label)| {
                    self.kill_facts(kill, tagged, label, &input, &teams)
                })
                .collect();
            let Some(contexts) = contexts else {
                continue;
            };
            let Some(first) = kills.iter().map(|(kill, _)| kill.tick).min() else {
                continue;
            };
            let Some(last) = kills.iter().map(|(kill, _)| kill.tick).max() else {
                continue;
            };
            let team = teams.get(&number).copied();
            let equipment = team.and_then(|side| {
                let values = self.samples.equipment.get(&number).copied()?;
                Some(if side == 2 {
                    values
                } else {
                    [values[1], values[0]]
                })
            });
            let own_deaths = self
                .events
                .deaths
                .iter()
                .filter(|death| death.victim == Some(actor));
            let previous_killers: Vec<_> = own_deaths
                .clone()
                .filter(|death| death.round == number - 1)
                .filter_map(|death| death.attacker.filter(|&attacker| attacker != actor))
                .collect();
            let target_death_tick = own_deaths
                .rev()
                .find(|death| death.round == number && death.tick > 0)
                .map(|death| death.tick);
            let first_death_tick = self
                .events
                .deaths
                .iter()
                .find(|death| {
                    death.round == number
                        && death.attacker.is_some()
                        && death.attacker != death.victim
                })
                .map(|death| death.tick);
            let teammate_kills = self
                .events
                .deaths
                .iter()
                .filter(|death| {
                    death.round == number
                        && team.is_some()
                        && team == death.attacker_team
                        && death.attacker_team != death.victim_team
                        && death.attacker.is_some_and(|attacker| attacker != actor)
                        && death.attacker != death.victim
                })
                .count();
            let target_hurt: Vec<_> = self
                .events
                .hurt
                .get(actor)
                .into_iter()
                .flatten()
                .filter(|hit| hit.round == number)
                .map(|hit| Hurt {
                    tick: hit.observation.tick,
                    damage: hit.observation.damage,
                    weapon: hit.observation.weapon,
                })
                .collect();
            let outcome = score.zip(scores.get(&(number + 1)).copied()).and_then(
                |([own, opp], [own_after, opp_after])| {
                    if own_after > own {
                        Some(true)
                    } else if opp_after > opp {
                        Some(false)
                    } else {
                        None
                    }
                },
            );
            let initial_health = self
                .samples
                .at(first)
                .or_else(|| self.samples.at(first - 8))
                .and_then(|rows| player_row(rows, actor))
                .and_then(|row| row.health);
            let final_health = self
                .samples
                .at(last)
                .and_then(|rows| player_row(rows, actor))
                .and_then(|row| row.health);
            let round = RoundFacts {
                number,
                score,
                won: outcome,
                first_death_tick,
                target_death_tick,
                freeze_end_tick: input.freezes.get(&number).copied(),
                next_freeze_end_tick: input.freezes.get(&(number + 1)).copied(),
                round_end_tick: input.decisions.get(&number).copied(),
                cap_round_end_tick: cap_rounds.get(&number).copied(),
                bomb_explode_tick: self.events.bomb_explodes.get(&number).copied(),
                initial_health,
                final_health,
                equipment,
                previous_killers: &previous_killers,
                teammate_kills,
                target_hurt: &target_hurt,
                defuse_window: self.events.defuse_windows.get(&(actor, number)).copied(),
                // Both cases were decided from authoritative observations just above.
                merged_defuse: false,
                post_match: false,
            };
            clips.extend(highlights::build(&round, &contexts, &recording));
        }
        clips
    }

    fn target_teams(
        &self,
        actor: &Player<'_>,
        scores: &RoundScores,
        freezes: &BTreeMap<i64, i64>,
    ) -> Option<BTreeMap<i64, i32>> {
        let observed = self.samples.teams.get(&actor.name.to_lowercase());
        if let Some(map) = observed.filter(|map| map.len() >= (freezes.len() / 2).max(1)) {
            return Some(map.clone());
        }
        // Use the fixed-roster observations only when the pawn/controller evidence
        // identified the two groups. Synthesized roster order would change a tag.
        scores.team_groups?;
        let column = usize::from(actor.team_key == TeamKey::B);
        let by_round: BTreeMap<_, _> = scores
            .rounds
            .iter()
            .map(|round| {
                let side = match round.sides[column] {
                    Side::T => 2,
                    Side::Ct => 3,
                };
                (round.round_number, side)
            })
            .collect();
        let fallback = freezes
            .keys()
            .filter_map(|&round| {
                let side = by_round
                    .range(..=round)
                    .next_back()
                    .or_else(|| by_round.first_key_value())?
                    .1;
                Some((round, *side))
            })
            .collect::<BTreeMap<_, _>>();
        if fallback.is_empty() {
            observed.cloned()
        } else {
            Some(fallback)
        }
    }

    fn defuse_overlay(&self, actor: &str, round: i64) -> Option<bool> {
        for defuse in self
            .events
            .defuses
            .iter()
            .filter(|defuse| defuse.round == round && same_player(actor, defuse.actor))
        {
            let Some(plant) = defuse.plant else {
                continue;
            };
            if defuse.tick - plant >= 39 * 64 {
                return Some(true);
            }
            if ninja_defuse(self.samples.at(defuse.tick)?, defuse.actor) {
                return Some(true);
            }
        }
        Some(false)
    }

    fn kill_facts<'b>(
        &self,
        kill: &'b Kill<'b>,
        tagged: &'b TaggedKill<'_>,
        label: &'b str,
        input: &ClipInput<'_>,
        teams: &BTreeMap<i64, i32>,
    ) -> Option<KillFacts<'b>> {
        let actor = input.player.name;
        let fires = input.fires;
        let tick = kill.tick;
        let event = tagged.source_event();
        let raw_flag = |key| match field(event, key) {
            Some(Variant::Bool(value)) => *value,
            value => value.and_then(integer).is_some_and(|value| value != 0),
        };
        let now = self.samples.at(tick);
        let attacker = now.and_then(|rows| player_row(rows, actor));
        let victim = now.and_then(|rows| player_row(rows, kill.victim));
        let geometry = attacker.zip(victim);
        let aim = geometry.and_then(|(attacker, victim)| {
            let (dx, dy) = displacement(victim.position, attacker.position)?;
            Some((dx.hypot(dy), dx, dy, victim.yaw))
        });
        let victim_yaw_delta = geometry
            .and_then(|(attacker, victim)| Some(angle_difference(attacker.yaw?, victim.yaw?)));
        let vertical_delta_16 = attacker.and_then(|current| {
            let earlier = self
                .samples
                .at((tick - 16).max(0))
                .and_then(|rows| player_row(rows, actor))?;
            Some(current.position.z? - earlier.position.z?)
        });
        let jumped = attacker
            .and_then(|current| {
                // vel_z is absent from the pinned wheel. Legacy jump uses exact -16 Z.
                let earlier = self
                    .samples
                    .at(tick - 16)
                    .and_then(|rows| player_row(rows, actor))?;
                Some((current.position.z? - earlier.position.z?).abs() > 20.0)
            })
            .unwrap_or_default();
        let pre = self
            .samples
            .at(tick - 8)
            .or_else(|| self.samples.at(tick - 16))
            .or_else(|| {
                matches!(kill.weapon, "awp" | "ssg08")
                    .then(|| {
                        self.samples
                            .at(tick - 24)
                            .or_else(|| self.samples.at(tick - 32))
                    })
                    .flatten()
            });
        // The other legacy pre-kill fallbacks can depend on ticks scheduled by
        // single-kill/fail processing. Leave that candidate unavailable.
        let encounter = encounter(pre?, actor);
        let awp_window = input
            .freezes
            .get(&kill.round)
            .copied()
            .unwrap_or_default()
            .max(tick - 5 * 64)..=tick;
        let recent_awp = self
            .events
            .awp
            .get(kill.victim)
            .is_some_and(|ticks| ticks.iter().any(|tick| awp_window.contains(tick)));
        let victim_awp_facing = recent_awp
            && aim.is_some_and(|(distance, dx, dy, yaw)| {
                distance >= 1.0
                    && yaw
                        .is_some_and(|yaw| angle_difference(dy.atan2(dx).to_degrees(), yaw) <= 45.0)
            });
        let teammate_hurt_tick = self.events.hurt.get(kill.victim).and_then(|hits| {
            hits.iter()
                .rev()
                .find(|hit| {
                    hit.observation.tick <= tick
                        && hit.hostile
                        && hit.attacker != Some(actor)
                        && hit
                            .attacker_team
                            .zip(teams.get(&hit.event_round).copied())
                            .is_none_or(|(attacker_side, target_side)| attacker_side == target_side)
                })
                .map(|hit| hit.observation.tick)
        });
        let lo = fires.partition_point(|fire| fire.tick < tick - 2 * 64);
        let hi = fires.partition_point(|fire| fire.tick <= tick);
        let shots = fires[lo..hi]
            .iter()
            .filter(|fire| fire.weapon == kill.weapon)
            .count();
        Some(KillFacts {
            kill,
            weapon_label: label,
            shots_to_kill: shots as i64,
            flash_assister: raw_flag("assistedflash")
                .then(|| name(event, "assister_name"))
                .flatten(),
            noscope: raw_flag("noscope"),
            encounter,
            victim_awp_facing,
            victim_distance_2d: aim.map(|(distance, ..)| distance),
            victim_yaw_delta,
            vertical_delta_16,
            jumped,
            teammate_hurt_tick,
        })
    }
}

fn player_row<'a, 'row>(pawns: &'row [Pawn<'a>], actor: &str) -> Option<&'row Pawn<'a>> {
    pawns
        .iter()
        .find(|row| row.name == actor)
        .or_else(|| pawns.iter().find(|row| same_player(row.name, actor)))
}

fn displacement(from: Position, to: Position) -> Option<(f64, f64)> {
    Some((to.x? - from.x?, to.y? - from.y?))
}

fn angle_difference(a: f64, b: f64) -> f64 {
    ((a - b + 180.0).rem_euclid(360.0) - 180.0).abs()
}

fn encounter(pawns: &[Pawn<'_>], actor: &str) -> Option<Encounter> {
    if pawns.iter().any(|row| !row.known) {
        return None;
    }
    let player = player_row(pawns, actor)?;
    let my_team = player.team?;
    let mut mates = 0;
    let mut enemies = 0;
    let mut nearby = 0;
    let mut closest_mate: Option<f64> = None;
    for row in pawns.iter().filter(|row| row.alive && row.name != actor) {
        let Some(team) = row.team else {
            continue;
        };
        let distance = displacement(player.position, row.position).map(|(x, y)| x.hypot(y));
        if team == my_team {
            mates += 1;
            if let Some(distance) = distance {
                closest_mate =
                    Some(closest_mate.map_or(distance, |previous| previous.min(distance)));
            }
        } else {
            enemies += 1;
            if distance.is_some_and(|distance| distance <= 1200.0) {
                nearby += 1;
            }
        }
    }
    Some(Encounter {
        mates,
        enemies,
        isolated_enemy_count: closest_mate
            .filter(|distance| *distance >= 1400.0)
            .map(|_| nearby),
    })
}

fn ninja_defuse(pawns: &[Pawn<'_>], defuser: &str) -> bool {
    let Some(player) = player_row(pawns, defuser).filter(|pawn| pawn.alive) else {
        return false;
    };
    let Some(team) = player.team else {
        return false;
    };
    let mut enemies = 0;
    for row in pawns
        .iter()
        .filter(|row| row.alive && !same_player(row.name, defuser))
    {
        match row.team {
            None => return false,
            Some(side) if side == team => continue,
            Some(_) => {}
        }
        let Some((x, y)) = displacement(player.position, row.position) else {
            return false;
        };
        let Some((z, rz)) = player.position.z.zip(row.position.z) else {
            return false;
        };
        let dz = z - rz;
        if (x * x + y * y + dz * dz).sqrt() >= 1000.0 {
            return false;
        }
        enemies += 1;
    }
    enemies >= 2
}

fn legacy_scores(events: &Events<'_>, teams: &BTreeMap<i64, i32>) -> BTreeMap<i64, [i64; 2]> {
    if !events.score_observed {
        return BTreeMap::new();
    }
    let mut current = [0, 0];
    let mut scores = BTreeMap::from([(1, current)]);
    for &(round, winner) in &events.winners {
        scores.insert(round, current);
        if let Some((_, &team)) = teams.range(..=round).next_back() {
            current[usize::from(winner != team)] += 1;
        }
        scores.insert(round + 1, current);
    }
    scores
}

fn post_match(
    round: i64,
    score: Option<[i64; 2]>,
    completed: i64,
    final_score: Option<[i64; 2]>,
) -> bool {
    (completed > 0 && round > completed)
        || score.is_some_and(|[own, opp]| {
            (own.max(opp) >= 13 && own.min(opp) <= 11)
                || (completed > 0 && own + opp == completed && final_score == score)
        })
}

fn freeze_caps(freezes: &BTreeMap<i64, i64>) -> BTreeMap<i64, i64> {
    let mut entries = freezes.iter().peekable();
    let mut caps = BTreeMap::new();
    while let Some((&round, &tick)) = entries.next() {
        let value = entries
            .peek()
            .map_or(tick + 30 * 64, |(_, next)| **next - 5 * 64);
        caps.insert(round, value);
    }
    caps
}

fn display_weapon(key: &str) -> Cow<'_, str> {
    Cow::Borrowed(match key {
        "ak47" => "AK-47",
        "m4a1" => "M4A4",
        "m4a1_silencer" => "消音 M4A1-S",
        "sg556" => "SG 553",
        "aug" => "AUG",
        "famas" => "法玛斯 (FAMAS)",
        "galilar" => "加利尔 (Galil)",
        "awp" => "大狙 (AWP)",
        "ssg08" => "鸟狙 (SSG08)",
        "scar20" => "连狙 (SCAR-20)",
        "g3sg1" => "连狙 (G3SG1)",
        "mac10" => "吹风机 (MAC-10)",
        "mac_10" => "吹风机 (MAC-10)",
        "mp9" => "小蜜蜂 (MP9)",
        "mp7" => "MP7",
        "mp5sd" => "MP5-SD",
        "ump45" => "车王 (UMP-45)",
        "p90" => "P90",
        "bizon" => "野牛 (PP-Bizon)",
        "deagle" => "沙鹰",
        "revolver" => "左轮 (R8)",
        "usp_silencer" => "消音 USP-S",
        "hkp2000" => "P2000",
        "glock" => "格洛克 (Glock-18)",
        "elite" => "双持 (Dual Berettas)",
        "p250" => "P250",
        "fiveseven" => "五七 (Five-SeveN)",
        "tec9" => "TEC-9",
        "cz75a" => "CZ-75",
        "nova" => "Nova",
        "xm1014" => "XM1014",
        "sawedoff" => "截短霰弹枪",
        "mag7" => "MAG-7",
        "m249" => "M249",
        "negev" => "内格夫 (Negev)",
        "hegrenade" => "手雷",
        "flashbang" => "闪光弹",
        "smokegrenade" => "烟雾弹",
        "inferno" => "燃烧弹",
        "molotov" => "燃烧瓶",
        "incgrenade" => "燃烧弹",
        "decoy" => "诱饵弹",
        "taser" => "电击枪 (Zeus)",
        "knife" => "刀",
        "knife_t" => "刀",
        "knife_ct" => "刀",
        "bayonet" => "刺刀",
        "knife_karambit" => "爪子刀",
        "knife_m9_bayonet" => "M9 刺刀",
        "knife_butterfly" => "蝴蝶刀",
        "knife_flip" => "折叠刀",
        "knife_gut" => "穿肠刀",
        "knife_tactical" => "猎杀者匕首",
        "knife_falchion" => "弯刀",
        "knife_survival_bowie" => "博伊猎刀",
        "knife_push" => "暗影双匕",
        "knife_cord" => "系绳匕首",
        "knife_canis" => "求生匕首",
        "knife_ursus" => "熊刀",
        "knife_gypsy_jackknife" => "流浪者匕首",
        "knife_outdoor" => "户外匕首",
        "knife_stiletto" => "短剑",
        "knife_widowmaker" => "锯齿爪刀",
        "knife_skeleton" => "骷髅匕首",
        "knife_css" => "经典刀",
        "knife_kukri" => "廓尔喀刀",
        "world" => "坠落/世界伤害",
        "c4" => "C4 爆炸",
        "planted_c4" => "C4 爆炸",
        "defuse_kit" => "拆弹器",
        _ => {
            let key = key.replace('_', " ");
            let mut chars = key.chars();
            let Some(first) = chars.next() else {
                return Cow::Borrowed("");
            };
            let mut label = first.to_uppercase().collect::<String>();
            label.push_str(&chars.as_str().to_lowercase());
            return Cow::Owned(label);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use parser::second_pass::game_events::EventField;

    fn event(name: &str, tick: i32, fields: &[(&str, Variant)]) -> GameEvent {
        GameEvent {
            name: name.to_owned(),
            tick,
            fields: fields
                .iter()
                .map(|(key, value)| EventField {
                    name: (*key).to_owned(),
                    data: Some(value.clone()),
                })
                .collect(),
        }
    }

    #[test]
    fn legacy_hurt_roles_and_completed_defuse_windows_use_their_actual_columns() {
        let row = |name, tick, actor| {
            event(
                name,
                tick,
                &[
                    ("user_name", Variant::String(actor)),
                    ("total_rounds_played", Variant::I32(0)),
                ],
            )
        };
        let hurt = event(
            "player_hurt",
            120,
            &[
                ("user_name", Variant::String("Target".to_owned())),
                ("attacker_name", Variant::String("Attacker".to_owned())),
                ("user_team_num", Variant::I32(2)),
                ("attacker_team_num", Variant::I32(2)),
                ("dmg_health", Variant::I32(23)),
            ],
        );
        let events = [
            event("bomb_planted", 100, &[]),
            hurt,
            row("bomb_begindefuse", 130, "Target".to_owned()),
            row("bomb_begindefuse", 140, "Target".to_owned()),
            row("bomb_defused", 200, "Target".to_owned()),
        ];
        let collected = Events::collect(&events, 1, &BTreeMap::from([(1, 110)]));
        let hit = &collected.hurt["Target"][0];
        assert_eq!(hit.round, 1);
        assert_eq!(hit.event_round, 0);
        assert_eq!(hit.observation.damage, 23);
        assert!(hit.hostile);
        assert_eq!(hit.attacker_team, None);
        assert_eq!(
            collected.defuse_windows.get(&("Target", 1)),
            Some(&[130, 200])
        );
        assert_eq!(
            collected
                .defuse_ticks(&BTreeSet::from(["target"]))
                .collect::<Vec<_>>(),
            [200]
        );
    }
}
