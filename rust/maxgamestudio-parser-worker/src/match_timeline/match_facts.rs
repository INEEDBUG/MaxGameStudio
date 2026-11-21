//! Team and score facts derived from the same selected-tick parser sample.

use super::{
    damage_stats::DamageInput,
    player_stats::{self, Player},
    round_economy::{self, RoundEconomy},
    round_score::{self, RoundOutcome, Side, TeamKey},
};
use crate::{
    demo_inspection::{parse_team_ticks, player_name},
    header_fields,
    round_windows::{match_start_tick, match_summary, RoundWindow},
    Failure,
};
use parser::second_pass::game_events::GameEvent;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub(super) fn collect(
    bytes: &[u8],
    huffman: &Vec<(u8, u8)>,
    events: &[GameEvent],
    demo_end_tick: i64,
    freeze_ticks: &BTreeMap<i64, i64>,
    visible: &[RoundWindow],
    rounds: &mut [Value],
) -> Result<Value, Failure> {
    let start_tick = match_start_tick(events);
    let economy_ticks = round_economy::freeze_end_rounds(events, start_tick);
    let mut ticks = match_summary::summary_ticks(events, demo_end_tick);
    ticks.extend(freeze_ticks.values().map(|&tick| tick as i32));
    ticks.extend(economy_ticks.keys().copied());
    ticks.sort_unstable();
    ticks.dedup();
    let mut properties = match_summary::PROPERTIES.to_vec();
    if !economy_ticks.is_empty() {
        properties.extend_from_slice(round_economy::PROPERTIES);
    }
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
    let player_stats = player_stats::build(
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
    add_round_facts(rounds, &scores.rounds, &economy);
    let ranked_player_ids = player_stats::ranked_player_ids(&player_stats);
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
    let facts = json!({
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
    Ok(facts)
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
