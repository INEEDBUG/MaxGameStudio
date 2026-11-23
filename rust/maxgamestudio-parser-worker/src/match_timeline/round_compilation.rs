//! Freeze-to-death uses the raw round number on a death event. The visible
//! replay windows can assign the same transition tick to an earlier round.

use super::{
    death_compilation,
    kill_compilation, kill_inputs,
    player_stats::Player,
    rival_compilation,
    round_score::{RoundScores, TeamKey},
};
use crate::{
    analysis_events::{self, EventKind},
    round_windows::{field, integer, winner},
};
use parser::second_pass::{game_events::GameEvent, variants::Variant};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    hash::{DefaultHasher, Hash, Hasher},
};

pub(super) struct MatchRounds<'a> {
    freezes: &'a BTreeMap<i64, i64>,
    starts: BTreeMap<i64, i64>,
    decisions: BTreeMap<i64, i64>,
    deaths: BTreeMap<&'a str, BTreeMap<i64, i64>>,
    requested_deaths: Vec<Value>,
    completed_rounds: i64,
    scores: &'a RoundScores,
    match_end: i64,
    tick_rate: f64,
}

impl<'a> MatchRounds<'a> {
    pub(super) fn collect(
        events: &'a [GameEvent],
        freezes: &'a BTreeMap<i64, i64>,
        scores: &'a RoundScores,
        match_start: i64,
        match_end: i64,
        tick_rate: f64,
        requested: &BTreeSet<&str>,
    ) -> Self {
        let mut starts = BTreeSet::new();
        let mut decisions = BTreeMap::new();
        let mut deaths = BTreeMap::<&str, BTreeMap<i64, i64>>::new();
        let mut requested_deaths = Vec::new();
        let mut completed_rounds = 0;
        let mut unnumbered = 0;
        for event in events {
            let tick = i64::from(event.tick);
            if tick <= 0 || tick < match_start {
                continue;
            }
            match event.name.as_str() {
                "round_start" => {
                    starts.insert(tick);
                }
                "round_end" => {
                    let round = field(event, "total_rounds_played")
                        .and_then(integer)
                        .unwrap_or_else(|| {
                            unnumbered += 1;
                            unnumbered
                        });
                    if round > 0 {
                        decisions.entry(round).or_insert(tick);
                        if field(event, "winner").and_then(winner).is_some() {
                            completed_rounds += 1;
                        }
                    }
                }
                "player_death" => {
                    let Some(row) = analysis_events::project(event) else {
                        continue;
                    };
                    let EventKind::Death {
                        attacker,
                        victim,
                        assister,
                        weapon,
                        headshot,
                    } = row.kind
                    else {
                        continue;
                    };
                    let round = row
                        .total_rounds_played
                        .unwrap_or_default()
                        .saturating_add(1);
                    if let Some(name) = victim.name.map(str::trim).filter(|name| !name.is_empty()) {
                        deaths.entry(name).or_default().entry(round).or_insert(tick);
                    }
                    if [&attacker, &victim, &assister]
                        .into_iter()
                        .filter_map(|actor| actor.name)
                        .any(|name| requested.contains(name.trim()))
                    {
                        let any = |fields: &[&str]| {
                            fields.iter().any(|key| match field(event, key) {
                                Some(Variant::Bool(flag)) => *flag,
                                Some(value) => integer(value).is_some_and(|number| number != 0),
                                None => false,
                            })
                        };
                        let penetrations = field(event, "penetrated_objects")
                            .or_else(|| field(event, "penetrated"));
                        requested_deaths.push(json!({
                            "round_number":round, "tick":tick,
                            "attacker_name":attacker.name, "victim_name":victim.name, "assister_name":assister.name,
                            "attacker_steamid":attacker.steamid64.map(|id| id.to_string()),
                            "victim_steamid":victim.steamid64.map(|id| id.to_string()),
                            "assister_steamid":assister.steamid64.map(|id| id.to_string()),
                            "attacker_team":attacker.team_num, "victim_team":victim.team_num,
                            "weapon":weapon, "headshot":headshot,
                            "no_scope":any(&["noscope"]), "through_smoke":any(&["thrusmoke","through_smoke"]),
                            "through_wall":penetrations.and_then(integer).is_some_and(|count| count > 0),
                            "attacker_blind":any(&["attackerblind"]),
                            "airborne":any(&["attackerinair","attacker_in_air","inair"]),
                            "flash_assisted":any(&["assistedflash"]),
                        }));
                    }
                }
                _ => {}
            }
        }
        let starts = freezes
            .iter()
            .filter_map(|(&round, &freeze)| {
                let previous = freezes
                    .get(&round.saturating_sub(1))
                    .copied()
                    .unwrap_or(match_start);
                if previous >= freeze {
                    return None;
                }
                Some((round, *starts.range((previous + 1)..freeze).next()?))
            })
            .collect();
        Self {
            freezes,
            starts,
            decisions,
            deaths,
            requested_deaths,
            completed_rounds,
            scores,
            match_end,
            tick_rate,
        }
    }

    pub(super) fn timeline_data(self) -> Value {
        let rounds = self
            .freezes
            .iter()
            .map(|(&round, &freeze)| {
                let score = self
                    .scores
                    .rounds
                    .iter()
                    .find(|score| score.round_number == round);
                let winner = score.and_then(|score| {
                    score
                        .winner
                        .map(|winner| score.sides[usize::from(winner == TeamKey::B)].label())
                });
                json!({
                    "round_number":round, "freeze_end_tick":freeze,
                    "end_tick":self.decisions.get(&round), "winner":winner,
                    "side_a":score.map(|score| score.sides[0].label()),
                    "side_b":score.map(|score| score.sides[1].label()),
                    "score_t":score.map(|score| score.score_before[0]),
                    "score_ct":score.map(|score| score.score_before[1]),
                })
            })
            .collect::<Vec<_>>();
        json!({"deaths":self.requested_deaths,"rounds":rounds})
    }

    pub(super) fn clips(
        &self,
        player: &Player<'_>,
        slots: &BTreeMap<String, i64>,
        map: &str,
        selected: Option<&[i64]>,
        (kills, spatial): (&kill_inputs::Sources<'_>, &kill_inputs::SpatialRows<'_>),
        last_round_tail_ticks: Option<i64>,
    ) -> Vec<Value> {
        let slot = slots.get(&player.name.to_lowercase()).copied();
        let tagged = kills.kills_for(player.name, spatial, &self.decisions);
        let kill_events = tagged
            .iter()
            .map(kill_inputs::TaggedKill::as_clip_input)
            .collect::<Vec<_>>();
        let fires = kills.fires_for(player.name);
        let all_kills = kill_compilation::build(
            kill_compilation::Facts {
                kills: &kill_events,
                fire: &fires,
                freezes: self.freezes,
                scores: self.scores,
                completed_rounds: self.completed_rounds,
                last_round_tail_ticks,
                last_decision_round: self.decisions.keys().next_back().copied(),
                demo_end_tick: self.match_end,
                slots,
            },
            player,
            map,
            slot,
            |round| self.post_match(round),
        );
        let deaths = death_compilation::build(
            death_compilation::Facts {
                deaths: &self.requested_deaths,
                freezes: self.freezes,
                scores: self.scores,
                completed_rounds: self.completed_rounds,
                last_round_tail_ticks,
                last_decision_round: self.decisions.keys().next_back().copied(),
                demo_end_tick: self.match_end,
                slots,
            },
            player,
            map,
            slot,
            |round| self.post_match(round),
        );
        let mut clips = rival_compilation::build(
            rival_compilation::Facts {
                kills: &kill_events,
                deaths: &self.requested_deaths,
                freezes: self.freezes,
                scores: self.scores,
                completed_rounds: self.completed_rounds,
                last_decision_round: self.decisions.keys().next_back().copied(),
                demo_end_tick: self.match_end,
                slots,
                last_round_tail_ticks,
            },
            player,
            map,
            slot,
            |round| self.post_match(round),
        );
        clips.extend(all_kills);
        clips.extend(deaths);
        clips.extend(self.freeze_to_death(player, slot, map, selected));
        clips.sort_by_key(|clip| (clip["round"].as_i64(), clip["start_tick"].as_i64()));
        clips
    }

    fn freeze_to_death(
        &self,
        player: &Player<'_>,
        spec_slot: Option<i64>,
        map: &str,
        selected: Option<&[i64]>,
    ) -> Option<Value> {
        let selection = selected.map(|rounds| {
            rounds
                .iter()
                .copied()
                .filter(|round| *round > 0)
                .collect::<BTreeSet<_>>()
        });
        let death_ticks = self.deaths.get(player.name);
        let pre = self.ticks(8.0).max(1);
        let windows = self
            .freezes
            .iter()
            .filter(|(&round, &freeze)| {
                freeze > 0
                    && selection
                        .as_ref()
                        .is_none_or(|selection| selection.contains(&round))
                    && !self.post_match(round)
            })
            .map(|(&round, &freeze)| {
                let start = freeze.saturating_sub(pre).max(0);
                let death = death_ticks.and_then(|deaths| deaths.get(&round).copied());
                let end = self.record_end(round, freeze, death).max(start + 1);
                Window {
                    round,
                    freeze_end_tick: freeze,
                    start_tick: start,
                    end_tick: end,
                    death_tick: death,
                    round_start_tick: self.starts.get(&round).copied(),
                    round_end_tick: self.decisions.get(&round).copied(),
                }
            })
            .collect::<Vec<_>>();
        let mut segments = Vec::new();
        let mut open = None;
        for (index, window) in windows.iter().enumerate() {
            let start: &usize = open.get_or_insert(index);
            let next = windows.get(index + 1);
            if window.death_tick.is_some() || next.is_none_or(|next| next.round != window.round + 1)
            {
                let first = &windows[*start];
                let end = self.record_end(window.round, window.freeze_end_tick, window.death_tick);
                let end = window
                    .death_tick
                    .map_or(end, |_| end.max(first.start_tick + 1));
                if end > first.start_tick {
                    segments.push((first, window, end));
                }
                open = None;
            }
        }
        let (first, _, _) = segments.first().copied()?;
        let (_, _, end) = segments.last().copied()?;
        let death = segments
            .iter()
            .rev()
            .find_map(|(_, last, _)| last.death_tick);
        let round_won = self
            .scores
            .rounds
            .iter()
            .find(|score| score.round_number == first.round)
            .and_then(|score| score.winner)
            .map(|winner| winner == player.team_key);
        let cap = death.map(|tick| tick.saturating_add(self.ticks(0.70)));
        let mut hash = DefaultHasher::new();
        (
            map,
            player.steam_id64,
            first.round,
            first.start_tick,
            end,
            selection.as_ref(),
        )
            .hash(&mut hash);
        let id = format!("c_{:08x}", hash.finish() as u32);
        Some(json!({
            "clip_id":id, "map_name":map, "round":first.round, "category":"compilation",
            "weapon_used":"", "kill_count":0, "start_tick":first.start_tick,
            "end_tick":cap.map_or(end, |cap| end.min(cap)),
            "context_tags":["🎬 回合合集"],
            "killer_name":null, "victims":[], "killers":[], "killers_steamid64s":[],
            "killer_steamid64":null, "victim_steamid64s":[],
            "kill_ticks":segments.iter().map(|(first,last,end)| last.death_tick.unwrap_or((end-1).max(first.start_tick))).collect::<Vec<_>>(),
            "target_spec_slot":spec_slot, "kill_spec_slots":vec![spec_slot; segments.len()],
            "victim_spec_slots":[], "killer_spec_slot":null, "killers_spec_slots":[],
            "score_own":null, "score_opp":null, "round_won":round_won,
            "clip_min_tick":first.freeze_end_tick, "clip_max_tick":cap, "death_tick":death,
            "ai_score":null, "ai_commentary":null,
            "source_ticks":segments.iter().map(|(first,_,end)| [first.start_tick,*end]).collect::<Vec<_>>(),
            "source_rounds":segments.iter().map(|(first,_,_)| first.round).collect::<Vec<_>>(),
            "source_round_ends":segments.iter().map(|(_,last,_)| last.round).collect::<Vec<_>>(),
            "compilation_kind":"freeze_to_death","fixed_segment_pacing":true,
            "freeze_to_death_round_filter":selection,
            "freeze_to_death_round_windows":windows,
            "flash_assisters":[], "kill_weapons":[], "kill_headshots":[], "kill_tag_lists":[], "shots_to_kill":[],
        }))
    }

    fn ticks(&self, seconds: f64) -> i64 {
        (self.tick_rate * seconds) as i64
    }

    fn post_match(&self, round: i64) -> bool {
        if self
            .scores
            .counted_rounds
            .is_some_and(|done| done > 0 && round > done)
        {
            return true;
        }
        self.scores
            .rounds
            .iter()
            .find(|score| score.round_number == round)
            .is_some_and(|score| {
                let [a, b] = score.score_before;
                (a.max(b) >= 13 && a.min(b) <= 11)
                    || self
                        .scores
                        .match_score
                        .is_some_and(|final_score| score.score_before == final_score && a + b > 0)
            })
    }

    fn record_end(&self, round: i64, freeze: i64, death: Option<i64>) -> i64 {
        let next_freeze = self.freezes.get(&round.saturating_add(1)).copied();
        let end = match death {
            Some(death) => {
                let raw = death.saturating_add(self.ticks(2.0).max(1));
                let cap = next_freeze
                    .filter(|next| *next > freeze)
                    .map(|next| next.saturating_sub(self.ticks(5.0)));
                cap.map_or(raw, |cap| raw.min(cap))
            }
            None => next_freeze.map_or_else(
                || freeze.saturating_add(self.ticks(150.0)),
                |next| next.saturating_sub(self.ticks(5.0)).max(0),
            ),
        };
        if self.match_end > 0 {
            end.min(self.match_end)
        } else {
            end
        }
    }
}

#[derive(Serialize)]
struct Window {
    round: i64,
    freeze_end_tick: i64,
    start_tick: i64,
    end_tick: i64,
    death_tick: Option<i64>,
    round_start_tick: Option<i64>,
    round_end_tick: Option<i64>,
}
