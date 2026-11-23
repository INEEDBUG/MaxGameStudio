//! Recover the visible match rounds and file playback boundary.
use crate::{parser_inputs, replay_io, Failure};
use parser::{
    first_pass::parser_settings::rm_user_friendly_names,
    parse_demo::{Parser, ParsingMode},
    second_pass::{
        game_events::GameEvent, parser_settings::create_huffman_lookup_table, variants::Variant,
    },
};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

const ROUNDS_PLAYED: &str = "total_rounds_played";

#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct RoundWindow {
    pub(crate) round_number: i64,
    pub(crate) start_tick: i64,
    pub(crate) freeze_end_tick: i64,
    round_end_tick: i64,
    pub(crate) end_tick: i64,
    pub(crate) winner_side: Option<i64>,
    end_reason: Option<String>,
    record_end_tick: i64,
}

pub(crate) fn inspect(
    path: &Path,
    tick_rate: f64,
    include_match_summary: bool,
) -> Result<Value, Failure> {
    if !tick_rate.is_finite() || !(1.0..=256.0).contains(&tick_rate) {
        return Err(Failure::request("unsupported Demo tick rate"));
    }
    replay_io::with_demo_bytes(path, |bytes| {
        let demo_end_tick = end_tick(bytes)?;
        let names = vec![ROUNDS_PLAYED.to_owned()];
        let internal = rm_user_friendly_names(&names)
            .map_err(|_| Failure::replay_parse("round properties are unavailable"))?;
        let huffman = create_huffman_lookup_table();
        let mut inputs = parser_inputs(&huffman);
        inputs.only_header = false;
        inputs.parse_ents = true;
        inputs.real_name_to_og_name = internal.iter().cloned().zip(names).collect();
        inputs.wanted_other_props = internal;
        inputs.wanted_events = [
            "round_announce_match_start",
            "round_start",
            "round_end",
            "round_freeze_end",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        if include_match_summary {
            inputs.wanted_events.push("player_death".to_owned());
        }
        let mut events = match Parser::new(inputs, ParsingMode::Normal).parse_demo(bytes) {
            Ok(output) => output.game_events,
            Err(_) if include_match_summary && demo_end_tick == 0 => Vec::new(),
            Err(_) => return Err(Failure::replay_parse("could not read round events")),
        };
        let summary = include_match_summary
            .then(|| match_summary::build(bytes, &huffman, demo_end_tick, &events))
            .transpose()?;
        events.sort_by_key(|event| event.tick);
        let (start, _, rounds) = windows(&events, tick_rate, demo_end_tick);
        let mut result =
            json!({"match_start_tick":start, "demo_end_tick":demo_end_tick, "rounds":rounds});
        if let Some(summary) = summary {
            result["match_meta"] = summary;
        }
        Ok(result)
    })
}

/// Build the playback windows from a tick-ordered match event scan.
pub(crate) fn windows(
    events: &[GameEvent],
    tick_rate: f64,
    demo_end: i64,
) -> (i64, BTreeMap<i64, i64>, Vec<RoundWindow>) {
    let start = i64::from(match_start_tick(events));
    let mut freezetime = BTreeMap::<i64, i64>::new();
    let mut restarts = BTreeSet::new();
    let mut decisions = BTreeMap::<i64, &GameEvent>::new();
    let mut unnamed_ends = 0;
    let freeze_events = events
        .iter()
        .filter(|event| event.name == "round_freeze_end" && i64::from(event.tick) >= start);
    let numbered_freezes = freeze_events
        .clone()
        .any(|e| field(e, ROUNDS_PLAYED).is_some());
    for event in events.iter().filter(|event| i64::from(event.tick) >= start) {
        let tick = i64::from(event.tick);
        match event.name.as_str() {
            "round_freeze_end" if tick > 0 && numbered_freezes => {
                let round_number = field(event, ROUNDS_PLAYED)
                    .and_then(integer)
                    .unwrap_or(0)
                    .saturating_add(1);
                freezetime.entry(round_number).or_insert(tick);
            }
            "round_start" => {
                restarts.insert(tick);
            }
            "round_end" => {
                let round_number = match field(event, ROUNDS_PLAYED) {
                    Some(value) => integer(value).unwrap_or(0),
                    None => {
                        unnamed_ends += 1;
                        unnamed_ends
                    }
                };
                if round_number > 0 && tick > 0 {
                    decisions.insert(round_number, event);
                }
            }
            _ => {}
        }
    }
    if !numbered_freezes {
        freezetime.extend(
            freeze_events
                .map(|event| i64::from(event.tick))
                .filter(|tick| *tick > 0)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .zip(1..)
                .map(|(tick, round)| (round, tick)),
        );
    }
    let mut rounds: Vec<_> = freezetime
        .iter()
        .filter_map(|(&round_number, &freeze)| {
            let ending = decisions.get(&round_number)?;
            let decision = i64::from(ending.tick);
            if decision <= freeze {
                return None;
            }
            let previous = freezetime
                .get(&round_number.saturating_sub(1))
                .copied()
                .unwrap_or(start.max(0));
            let first = if previous < freeze {
                restarts.range((previous + 1)..freeze).next().copied()
            } else {
                None
            };
            Some(RoundWindow {
                round_number,
                start_tick: first.filter(|tick| *tick > 0).unwrap_or(freeze),
                freeze_end_tick: freeze,
                round_end_tick: decision,
                end_tick: decision,
                end_reason: field(ending, "reason").and_then(reason),
                winner_side: field(ending, "winner").and_then(winner),
                record_end_tick: decision,
            })
        })
        .collect();
    for index in 1..rounds.len() {
        let next = rounds[index].start_tick;
        let prior = &mut rounds[index - 1];
        if next > prior.end_tick {
            prior.end_tick = next - 1;
        }
    }
    if let Some(last) = rounds.last_mut() {
        if demo_end > last.round_end_tick {
            last.end_tick =
                demo_end.min(last.round_end_tick + (tick_rate * 3.0).round_ties_even() as i64);
        }
    }
    for round in &mut rounds {
        round.record_end_tick = round.end_tick;
    }
    (start, freezetime, rounds)
}

pub(crate) fn match_start_tick(events: &[GameEvent]) -> i32 {
    events
        .iter()
        .filter(|event| event.name == "round_announce_match_start")
        .map(|event| event.tick)
        .max()
        .unwrap_or(0)
}

pub(crate) fn field<'a>(event: &'a GameEvent, name: &str) -> Option<&'a Variant> {
    event
        .fields
        .iter()
        .find(|field| field.name == name)?
        .data
        .as_ref()
}

pub(crate) fn integer(value: &Variant) -> Option<i64> {
    match value {
        Variant::I32(n) => Some(i64::from(*n)),
        Variant::U32(n) => Some(i64::from(*n)),
        Variant::U64(n) => i64::try_from(*n).ok(),
        Variant::F32(n) if n.is_finite() => Some(*n as i64),
        Variant::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

pub(crate) fn winner(value: &Variant) -> Option<i64> {
    let side = match value {
        Variant::String(name) => match name.trim().to_ascii_uppercase().as_str() {
            "CT" => 3,
            "T" | "TERRORIST" | "TERRORISTS" => 2,
            _ => return None,
        },
        _ => integer(value)?,
    };
    matches!(side, 2 | 3).then_some(side)
}

fn reason(value: &Variant) -> Option<String> {
    match value {
        Variant::String(s) if !s.trim().is_empty() && !s.trim().eq_ignore_ascii_case("nan") => {
            Some(s.trim().to_owned())
        }
        _ => integer(value).map(|value| value.to_string()),
    }
}

pub(crate) fn end_tick(bytes: &[u8]) -> Result<i64, Failure> {
    crate::demo_boundary::end_tick(std::io::Cursor::new(bytes), bytes.len() as u64)
        .map_err(|_| Failure::replay_parse("invalid Demo frame boundary"))
}

pub(crate) mod match_summary {
    use super::{field, integer, match_start_tick, GameEvent, Variant};
    use crate::{
        demo_inspection::{
            named_column, parse_team_ticks, player_name, team_at_column, CONTROLLER_TEAM,
        },
        header_fields, Failure,
    };
    use parser::{
        first_pass::prop_controller::{STEAMID_ID, TICK_ID},
        parse_demo::DemoOutput,
        second_pass::variants::VarVec,
    };
    use serde_json::{json, Value};
    use std::collections::{BTreeMap, BTreeSet, HashMap};

    const CLAN_NAME: &str = "CCSTeam.m_szClanTeamname";
    const CLAN_SIDE: &str = "CCSTeam.m_iTeamNum";
    pub(crate) const PROPERTIES: &[&str] = &["steamid", CLAN_NAME, CLAN_SIDE];
    const LEGACY_TICKS_PER_MINUTE: i64 = 64 * 60;

    pub(crate) type MembersByTick = BTreeMap<i32, Vec<(Option<u64>, i32)>>;

    #[derive(Default)]
    struct TeamObservations {
        members: MembersByTick,
        names: [Option<String>; 2],
    }

    pub(super) fn build(
        bytes: &[u8],
        huffman: &Vec<(u8, u8)>,
        demo_end_tick: i64,
        events: &[GameEvent],
    ) -> Result<Value, Failure> {
        let header = header_fields(bytes)?;
        let ticks = summary_ticks(events, demo_end_tick);
        let output = if ticks.is_empty() {
            None
        } else {
            // Empty tick lists select all rows, so only call the parser when at least one is needed.
            Some(parse_team_ticks(bytes, huffman, ticks, PROPERTIES)?)
        };
        Ok(summarize(&header, events, demo_end_tick, output.as_ref()))
    }

    pub(crate) fn summary_ticks(events: &[GameEvent], demo_end_tick: i64) -> Vec<i32> {
        if demo_end_tick <= 0 {
            return Vec::new();
        }
        let start = match_start_tick(events);
        let mut ticks = BTreeSet::from([start.max(1)]);
        if let Some(decisions) = round_decisions(events, start) {
            ticks.extend(decisions.into_iter().map(|(tick, _)| tick));
        }
        ticks.into_iter().collect()
    }

    pub(crate) fn summarize(
        header: &BTreeMap<String, String>,
        events: &[GameEvent],
        demo_end_tick: i64,
        output: Option<&DemoOutput>,
    ) -> Value {
        summarize_with_members(header, events, demo_end_tick, output).0
    }

    pub(crate) fn summarize_with_members(
        header: &BTreeMap<String, String>,
        events: &[GameEvent],
        demo_end_tick: i64,
        output: Option<&DemoOutput>,
    ) -> (Value, MembersByTick) {
        let start = match_start_tick(events);
        let observations = output
            .map(|output| observe_teams(output, start.max(1)))
            .unwrap_or_default();
        let scores = round_decisions(events, start)
            .as_deref()
            .and_then(|decisions| fixed_team_scores(start, decisions, &observations.members));
        let duration_mins = duration(header, start, events);
        let [team_a_name, team_b_name] = observations.names;
        let map_name = header
            .get("map_name")
            .filter(|name| !name.is_empty())
            .map_or("unknown", String::as_str);
        let server_name = header.get("server_name").map_or("", |name| {
            name.trim_matches(|ch: char| ch.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&ch))
        });
        let summary = json!({
            "map_name": map_name, "server_name": server_name, "target_player": "",
            "target_player_user_id": null, "target_steam_id": null,
            "total_rounds": scores.map(|[a,b]| a+b),
            "target_kills": 0, "target_deaths": 0,
            "team_a_score": scores.map(|[a,_]| a),
            "team_b_score": scores.map(|[_,b]| b),
            "team_a_name": team_a_name.as_deref().unwrap_or("Team A"),
            "team_b_name": team_b_name.as_deref().unwrap_or("Team B"),
            "match_date": "", "duration_mins": duration_mins, "demo_end_tick": demo_end_tick,
        });
        (summary, observations.members)
    }

    fn round_decisions(events: &[GameEvent], start: i32) -> Option<Vec<(i32, i32)>> {
        let mut decisions = BTreeMap::new();
        for event in events
            .iter()
            .filter(|event| event.name == "round_end" && event.tick >= start)
        {
            let side = field(event, "winner").and_then(winning_side)?;
            if event.tick <= 0 || decisions.insert(event.tick, side).is_some() {
                return None;
            }
        }
        Some(decisions.into_iter().collect())
    }

    fn winning_side(value: &Variant) -> Option<i32> {
        let side = if let Variant::String(name) = value {
            match name.trim().to_ascii_uppercase().as_str() {
                "CT" | "CTS" | "COUNTER-TERRORISTS" | "COUNTER_TERRORISTS" => 3,
                "T" | "TS" | "TERRORIST" | "TERRORISTS" => 2,
                name => {
                    let number = name.parse::<f64>().ok()?;
                    if !number.is_finite() {
                        return None;
                    }
                    number as i64
                }
            }
        } else {
            integer(value)?
        };
        matches!(side, 2 | 3).then_some(side as i32)
    }

    fn observe_teams(output: &DemoOutput, opening_tick: i32) -> TeamObservations {
        let mut observations = TeamObservations::default();
        let column = |id| output.df.get(&id).and_then(|column| column.data.as_ref());
        let Some(VarVec::I32(ticks)) = column(TICK_ID) else {
            return observations;
        };
        let steamids = match column(STEAMID_ID) {
            Some(VarVec::U64(ids)) => Some(ids),
            _ => None,
        };
        let pawn_side = named_column(output, "team_num");
        let controller_side = named_column(output, CONTROLLER_TEAM);
        let clan_side = named_column(output, CLAN_SIDE);
        let clan_names = match named_column(output, CLAN_NAME) {
            Some(VarVec::String(names)) => Some(names),
            _ => None,
        };
        for (index, tick) in ticks.iter().copied().enumerate() {
            let Some(tick) = tick else {
                continue;
            };
            if let Some(side) =
                team_at_column(pawn_side, index).or_else(|| team_at_column(controller_side, index))
            {
                let steamid = steamids
                    .and_then(|ids| ids.get(index))
                    .copied()
                    .flatten()
                    .filter(|&id| id > 0);
                observations
                    .members
                    .entry(tick)
                    .or_default()
                    .push((steamid, side));
            }
            if tick == opening_tick && observations.names.iter().any(Option::is_none) {
                if let Some((side, name)) = team_at_column(clan_side, index).zip(
                    clan_names
                        .and_then(|names| names.get(index))
                        .and_then(Option::as_deref)
                        .and_then(player_name),
                ) {
                    if !matches!(name.to_ascii_lowercase().as_str(), "t" | "ct" | "terrorist") {
                        observations.names[(side - 2) as usize] = Some(name.to_owned());
                    }
                }
            }
        }
        observations
    }

    fn fixed_team_scores(
        start: i32,
        decisions: &[(i32, i32)],
        members: &MembersByTick,
    ) -> Option<[usize; 2]> {
        if decisions.is_empty() {
            return Some([0, 0]);
        }
        if start <= 0 {
            return None;
        }
        let mut opening = HashMap::new();
        for &(id, side) in members.get(&start)? {
            if let Some(id) = id {
                if opening.insert(id, side).is_some_and(|prior| prior != side) {
                    return None;
                }
            }
        }
        if ![2, 3]
            .iter()
            .all(|side| opening.values().any(|value| value == side))
        {
            return None;
        }
        let mut scores = [0, 0];
        for &(tick, side) in decisions {
            let observed = members.get(&tick)?;
            let first_winner = observed.iter().find(|(_, current)| *current == side)?.0?;
            let initial_team = *opening.get(&first_winner)?;
            for &(id, current) in observed {
                if let Some(known_team) = id.and_then(|id| opening.get(&id)) {
                    if (*known_team == initial_team) != (current == side) {
                        return None;
                    }
                }
            }
            scores[(initial_team - 2) as usize] += 1;
        }
        Some(scores)
    }

    fn duration(header: &BTreeMap<String, String>, start: i32, events: &[GameEvent]) -> i64 {
        if !events.iter().any(|event| event.name == "round_end") {
            return 0;
        }
        let header_minutes = header
            .get("playback_time")
            .and_then(|value| value.trim().parse::<f64>().ok())
            .filter(|seconds| seconds.is_finite())
            .map_or(0, |seconds| (seconds / 60.0).floor() as i64);
        let last_tick = events
            .iter()
            .filter(|event| {
                event.name == "round_end" || (event.name == "player_death" && event.tick >= start)
            })
            .map(|event| i64::from(event.tick))
            .max()
            .unwrap_or(0);
        let event_minutes = (last_tick - i64::from(start.max(0))).max(0) / LEGACY_TICKS_PER_MINUTE;
        header_minutes.max(event_minutes)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use parser::second_pass::game_events::EventField;

        #[test]
        fn counts_fixed_memberships_across_halftime_and_an_overtime_swap() {
            let start = 100;
            let demo_trace = [
                (3900, "T", 2),
                (8000, "CT", 2),
                (12000, "CT", 3),
                (16000, "CT", 3),
                (20000, "T", 3),
                (24000, "T", 2),
            ];
            let (a, b) = (76561198000000001, 76561198000000002);
            let mut members = MembersByTick::from([(start, vec![(Some(a), 2), (Some(b), 3)])]);
            let mut events = Vec::new();
            for (tick, winner, a_side) in demo_trace {
                members.insert(tick, vec![(Some(a), a_side), (Some(b), 5 - a_side)]);
                events.push(GameEvent {
                    name: "round_end".to_owned(),
                    fields: vec![EventField {
                        name: "winner".to_owned(),
                        data: Some(Variant::String(winner.to_owned())),
                    }],
                    tick,
                });
            }
            events.push(GameEvent {
                name: "player_death".to_owned(),
                fields: Vec::new(),
                tick: 24600,
            });
            let decisions = round_decisions(&events, start).unwrap();
            assert_eq!(decisions.len(), 6);
            assert_eq!(fixed_team_scores(start, &decisions, &members), Some([4, 2]));
            assert_eq!(duration(&BTreeMap::new(), start, &events), 6);

            // A mixed or unavailable roster must leave the scores unknown.
            members.insert(24000, vec![(Some(a), 2), (Some(b), 2)]);
            assert_eq!(fixed_team_scores(start, &decisions, &members), None);
            members.remove(&24000);
            assert_eq!(fixed_team_scores(start, &decisions, &members), None);
            assert_eq!(fixed_team_scores(start, &[], &members), Some([0, 0]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parser::second_pass::game_events::EventField;

    fn event(name: &str, tick: i32, fields: Vec<(&str, Variant)>) -> GameEvent {
        GameEvent {
            name: name.to_owned(),
            tick,
            fields: fields
                .into_iter()
                .map(|(name, value)| EventField {
                    name: name.to_owned(),
                    data: Some(value),
                })
                .collect(),
        }
    }

    #[test]
    fn excludes_restarts_and_keeps_visible_round_tails_within_the_demo() {
        let mut events = vec![
            event("round_announce_match_start", 20, vec![]),
            event("round_announce_match_start", 5, vec![]),
            event("round_freeze_end", 10, vec![]),
            event(
                "round_freeze_end",
                30,
                vec![(ROUNDS_PLAYED, Variant::U32(0))],
            ),
            event(
                "round_end",
                80,
                vec![
                    (ROUNDS_PLAYED, Variant::U32(1)),
                    ("winner", Variant::String("CT".to_owned())),
                ],
            ),
            event("round_start", 91, vec![]),
            event(
                "round_freeze_end",
                100,
                vec![(ROUNDS_PLAYED, Variant::U32(1))],
            ),
            event(
                "round_end",
                140,
                vec![
                    (ROUNDS_PLAYED, Variant::U32(2)),
                    ("winner", Variant::U32(2)),
                ],
            ),
        ];
        events.sort_by_key(|event| event.tick);
        let (start, _, rounds) = windows(&events, 64.0, 150);
        assert_eq!(start, 20);
        assert_eq!(rounds.len(), 2);
        assert_eq!(
            (
                rounds[0].start_tick,
                rounds[0].round_end_tick,
                rounds[0].end_tick
            ),
            (30, 80, 90)
        );
        assert_eq!(
            (
                rounds[1].start_tick,
                rounds[1].round_end_tick,
                rounds[1].end_tick
            ),
            (91, 140, 150)
        );
        assert_eq!(
            (rounds[0].winner_side, rounds[1].winner_side),
            (Some(3), Some(2))
        );
    }
}
