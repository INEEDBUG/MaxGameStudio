//! Team and score facts derived from the same selected-tick parser sample.

use super::{
    damage_stats::DamageInput,
    highlight_inputs, kill_inputs,
    player_stats::{self, Player},
    round_compilation::{ClipOptions, MatchRounds},
    round_economy::{self, RoundEconomy},
    round_score::{self, RoundOutcome, Side, TeamKey},
    spec_slots::{SlotLookup, SpecSlots},
    RequestedCompilations,
};
use crate::{
    demo_inspection::{parse_team_ticks, player_name},
    header_fields,
    round_windows::{match_start_tick, match_summary, RoundWindow},
    Failure,
};
use parser::second_pass::game_events::GameEvent;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Timing<'a> {
    pub(super) demo_end: i64,
    pub(super) tick_rate: f64,
    pub(super) compilations: Option<&'a RequestedCompilations>,
}

struct RecordingInputs<'a, 'events> {
    request: &'a RequestedCompilations,
    requested: BTreeSet<&'a str>,
    kills: kill_inputs::Sources<'events>,
    highlight_events: Option<highlight_inputs::Events<'events>>,
    highlight_ticks: BTreeSet<i32>,
}

pub(super) fn collect(
    bytes: &[u8],
    huffman: &Vec<(u8, u8)>,
    events: &[GameEvent],
    timing: Timing<'_>,
    freeze_ticks: &BTreeMap<i64, i64>,
    visible: &[RoundWindow],
    rounds: &mut [Value],
) -> Result<Value, Failure> {
    let demo_end_tick = timing.demo_end;
    let start_tick = match_start_tick(events);
    let economy_ticks = round_economy::freeze_end_rounds(events, start_tick);
    let compilations = timing.compilations.map(|request| {
        let requested = request
            .players
            .iter()
            .map(|name| name.trim())
            .collect::<BTreeSet<_>>();
        let kills = kill_inputs::Sources::collect(events, start_tick, &requested, freeze_ticks);
        let mut highlight_ticks = kills.highlight_ticks();
        let highlight_events = (!highlight_ticks.is_empty()).then(|| {
            let events = highlight_inputs::Events::collect(events, start_tick, freeze_ticks);
            highlight_ticks.extend(events.defuse_ticks(&requested));
            events
        });
        RecordingInputs {
            request,
            requested,
            kills,
            highlight_events,
            highlight_ticks,
        }
    });
    let mut ticks = match_summary::summary_ticks(events, demo_end_tick);
    ticks.extend(freeze_ticks.values().map(|&tick| tick as i32));
    ticks.extend(economy_ticks.keys().copied());
    let mut properties = match_summary::PROPERTIES.to_vec();
    if !economy_ticks.is_empty() {
        properties.extend_from_slice(round_economy::PROPERTIES);
    }
    if let Some(recording) = &compilations {
        ticks.extend(recording.kills.ticks());
        properties.extend_from_slice(&kill_inputs::PROPERTIES);
        if !recording.highlight_ticks.is_empty() {
            ticks.extend(recording.highlight_ticks.iter().copied());
            for &property in highlight_inputs::PROPERTIES {
                if !properties.contains(&property) {
                    properties.push(property);
                }
            }
        }
    }
    ticks.sort_unstable();
    ticks.dedup();
    // An empty selection asks the parser for every tick.
    let sample = (!ticks.is_empty())
        .then(|| parse_team_ticks(bytes, huffman, ticks, &properties))
        .transpose()?;
    let header = header_fields(bytes)?;
    let (summary, members) =
        match_summary::summarize_with_members(&header, events, demo_end_tick, sample.as_ref());

    // The same player-end metadata supplies the released wheel's parse_player_info rows.
    let roster: BTreeMap<_, _> = sample
        .iter()
        .flat_map(|sample| &sample.player_md)
        .filter_map(|player| {
            let steamid = player.steamid?;
            let id = steamid.to_string();
            let team = i64::from(player.team_number?);
            if id.len() < 16 || !id.starts_with("7656") || Side::from_engine_team(team).is_none() {
                return None;
            }
            Some((
                steamid,
                (id, player.name.as_deref().and_then(player_name), team),
            ))
        })
        .collect();
    let observations = freeze_ticks
        .iter()
        .map(|(&round, &tick)| {
            let mut sides = [None; 2];
            if let Some(sample) = members.get(&(tick as i32)) {
                for &(steamid, side) in sample {
                    if let Some((_, _, group)) = steamid.and_then(|id| roster.get(&id)) {
                        sides[(*group - 2) as usize] = Side::from_engine_team(i64::from(side));
                    }
                }
            }
            (round, sides)
        })
        .collect();
    let counted_score = summary["team_a_score"]
        .as_i64()
        .zip(summary["team_b_score"].as_i64())
        .map(|(a, b)| [a, b]);
    let visible_winners: Vec<_> = visible
        .iter()
        .map(|round| {
            (
                round.round_number,
                round.winner_side.and_then(Side::from_engine_team),
            )
        })
        .collect();
    let scores = round_score::build(counted_score, &observations, &visible_winners);
    let players: Vec<_> = roster
        .values()
        .filter_map(|(steamid, name, group)| {
            Some(Player {
                steam_id64: steamid,
                name: (*name)?,
                final_group: *group,
                team_key: scores.team_key(*group)?,
            })
        })
        .collect();
    let visible_freeze_ticks: Vec<_> = visible.iter().map(|round| round.freeze_end_tick).collect();
    let mut player_stats = player_stats::build(
        &players,
        rounds,
        &scores.rounds,
        DamageInput {
            events,
            match_start_tick: start_tick,
            freeze_ticks: &visible_freeze_ticks,
        },
    );
    let economy = sample
        .as_ref()
        .filter(|_| !economy_ticks.is_empty())
        .map(|sample| {
            round_economy::collect(sample, &economy_ticks, &players, scores.halftime_round)
        })
        .unwrap_or_default();
    for stats in &mut player_stats {
        let name = stats["name"].as_str().expect("native stats player name");
        stats["average_equipment_value"] = json!(economy.average_equipment(name));
    }
    add_round_facts(rounds, &scores.rounds, &economy.rounds);
    let ranked_player_ids = player_stats::ranked_player_ids(&player_stats);
    let compilation_data = compilations
        .map(|recording| {
            let RecordingInputs {
                request,
                requested,
                kills,
                highlight_events,
                highlight_ticks,
            } = recording;
            let last_round_tail_ticks = tail_ticks(
                "CS2_INSIGHT_LAST_ROUND_KILL_BUFFER_SEC",
                "invalid last-round recording tail",
            )?;
            let offset = std::env::var("CS2_SPEC_PLAYER_SLOT_OFFSET").ok();
            let slots = SpecSlots::from_events(events, start_tick, offset.as_deref());
            let slots: BTreeMap<_, _> = players
                .iter()
                .filter_map(|player| match slots.lookup(player.name) {
                    SlotLookup::Known(slot) => Some((player.name.to_lowercase(), slot)),
                    SlotLookup::Missing | SlotLookup::Ambiguous => None,
                })
                .collect();
            let clip_rounds = MatchRounds::collect(
                events,
                freeze_ticks,
                &scores,
                i64::from(start_tick),
                demo_end_tick,
                timing.tick_rate,
                &requested,
            );
            let map = summary["map_name"].as_str().unwrap_or("unknown");
            let spatial = kills.snapshots(sample.as_ref());
            let highlights = highlight_events
                .zip(sample.as_ref())
                .and_then(|(events, sample)| {
                    let samples = highlight_inputs::Samples::collect(
                        sample,
                        &highlight_ticks,
                        &economy_ticks,
                    )?;
                    Some((events, samples))
                })
                .map(|(events, samples)| {
                    let mid_round_buffer_ticks = tail_ticks(
                        "CS2_INSIGHT_MID_ROUND_END_BUFFER_SEC",
                        "invalid middle-round recording tail",
                    )?;
                    Ok::<_, Failure>(highlight_inputs::Source {
                        events,
                        samples,
                        mid_round_buffer_ticks,
                    })
                })
                .transpose()?;
            let clips = players
                .iter()
                .filter(|player| requested.contains(player.name))
                .map(|player| {
                    (
                        player.name,
                        clip_rounds.clips(
                            player,
                            &slots,
                            map,
                            request.rounds.as_deref(),
                            (&kills, &spatial),
                            ClipOptions {
                                last_round_tail_ticks,
                                highlights: highlights.as_ref(),
                            },
                        ),
                    )
                })
                .collect::<BTreeMap<_, _>>();
            Ok::<_, Failure>((clips, slots, clip_rounds.timeline_data()))
        })
        .transpose()?;
    let players: Vec<_> = players
        .iter()
        .map(|player| {
            json!({
                "steam_id64": player.steam_id64,
                "name": player.name,
                "final_group": player.final_group,
                "team_key": player.team_key.label(),
            })
        })
        .collect();
    let mut facts = json!({
        "match_meta": {
            "map_name": summary["map_name"],
            "server_name": summary["server_name"],
            "demo_end_tick": demo_end_tick,
            "duration_mins": summary["duration_mins"],
            "total_rounds": scores.counted_rounds,
            "team_a_name": summary["team_a_name"],
            "team_b_name": summary["team_b_name"],
            "team_a_score": scores.match_score.map(|[a, _]| a),
            "team_b_score": scores.match_score.map(|[_, b]| b),
        },
        "team_a_group": scores.team_groups.map(|[a, _]| a),
        "team_b_group": scores.team_groups.map(|[_, b]| b),
        "team_assignment_source": scores.team_assignment_source(),
        "phase_meta": {
            "halftime_round": scores.halftime_round,
            "regulation_end_round": scores.regulation_end_round,
        },
        "players": players,
        "player_stats": player_stats,
        "ranked_player_ids": ranked_player_ids,
    });
    if let Some((clips, slots, timeline)) = compilation_data {
        facts["round_compilations"] = json!(clips);
        facts["spec_slots"] = json!(slots);
        facts["recording_timeline"] = timeline;
    }
    Ok(facts)
}

fn tail_ticks(key: &str, invalid: &'static str) -> Result<Option<i64>, Failure> {
    let Ok(seconds) = std::env::var(key) else {
        return Ok(None);
    };
    if seconds.is_empty() {
        return Ok(None);
    }
    seconds
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .map(|value| Some((64.0 * value) as i64))
        .ok_or_else(|| Failure::request(invalid))
}

fn add_round_facts(
    rounds: &mut [Value],
    scores: &[RoundOutcome],
    economy: &BTreeMap<i64, RoundEconomy>,
) {
    for (round, score) in rounds.iter_mut().zip(scores) {
        debug_assert_eq!(round["round_number"].as_i64(), Some(score.round_number));
        let [side_a, side_b] = score.sides;
        let [before_a, before_b] = score.score_before;
        let [after_a, after_b] = score.score_after;
        round["team_a_side"] = json!(side_a.label());
        round["team_b_side"] = json!(side_b.label());
        round["winner_team_key"] = json!(score.winner.map(TeamKey::label));
        round["team_a_score_before"] = json!(before_a);
        round["team_b_score_before"] = json!(before_b);
        round["team_a_score_after"] = json!(after_a);
        round["team_b_score_after"] = json!(after_b);
        if let Some(economy) = economy.get(&score.round_number) {
            economy.write(round, score.sides);
        }
    }
}
