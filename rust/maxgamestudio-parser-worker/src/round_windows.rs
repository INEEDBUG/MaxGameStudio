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
struct RoundWindow {
    round_number: i64,
    start_tick: i64,
    freeze_end_tick: i64,
    round_end_tick: i64,
    end_tick: i64,
    winner_side: Option<i64>,
    end_reason: Option<String>,
    record_end_tick: i64,
}

pub(crate) fn inspect(path: &Path, tick_rate: f64) -> Result<Value, Failure> {
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
        let parsed = Parser::new(inputs, ParsingMode::Normal)
            .parse_demo(bytes)
            .map_err(|_| Failure::replay_parse("could not read round events"))?;
        let (start, rounds) = windows(parsed.game_events, tick_rate, demo_end_tick);
        Ok(json!({"match_start_tick":start, "demo_end_tick":demo_end_tick, "rounds":rounds}))
    })
}

fn windows(mut events: Vec<GameEvent>, tick_rate: f64, demo_end: i64) -> (i64, Vec<RoundWindow>) {
    events.sort_by_key(|event| event.tick);
    let start = events
        .iter()
        .filter(|event| event.name == "round_announce_match_start")
        .map(|event| i64::from(event.tick))
        .max()
        .unwrap_or(0);
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
    (start, rounds)
}

fn field<'a>(event: &'a GameEvent, name: &str) -> Option<&'a Variant> {
    event
        .fields
        .iter()
        .find(|field| field.name == name)?
        .data
        .as_ref()
}

fn integer(value: &Variant) -> Option<i64> {
    match value {
        Variant::I32(n) => Some(i64::from(*n)),
        Variant::U32(n) => Some(i64::from(*n)),
        Variant::U64(n) => i64::try_from(*n).ok(),
        Variant::F32(n) if n.is_finite() => Some(*n as i64),
        Variant::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn winner(value: &Variant) -> Option<i64> {
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

fn end_tick(bytes: &[u8]) -> Result<i64, Failure> {
    let bad_frame = || Failure::replay_parse("invalid Demo frame boundary");
    let mut frames = bytes
        .get(16..)
        .filter(|_| bytes.starts_with(b"PBDEMS2\0"))
        .ok_or_else(bad_frame)?;
    let mut end = 0;
    while !frames.is_empty() {
        read_varint(&mut frames).ok_or_else(bad_frame)?;
        let tick = read_varint(&mut frames)
            .and_then(|tick| u32::try_from(tick).ok())
            .ok_or_else(bad_frame)?;
        let size = read_varint(&mut frames)
            .filter(|len| *len <= 256 * 1024 * 1024)
            .ok_or_else(bad_frame)?;
        frames = frames.get(size as usize..).ok_or_else(bad_frame)?;
        if tick != u32::MAX {
            end = end.max(i64::from(tick));
        }
    }
    Ok(end)
}

fn read_varint(bytes: &mut &[u8]) -> Option<u64> {
    let mut value = 0;
    for shift in (0..=63).step_by(7) {
        let (&byte, tail) = bytes.split_first()?;
        *bytes = tail;
        if shift == 63 && byte > 1 {
            return None;
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return Some(value);
        }
    }
    None
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
        let events = vec![
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
        let (start, rounds) = windows(events, 64.0, 150);
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
