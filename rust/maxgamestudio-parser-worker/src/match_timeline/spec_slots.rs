//! Resolve console spec_player slots from the match's raw death-event user IDs.

use crate::{
    demo_inspection::player_name,
    round_windows::{field, integer},
};
use parser::second_pass::{game_events::GameEvent, variants::Variant};
use std::{borrow::Cow, collections::BTreeMap};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SlotLookup {
    Known(i64),
    Missing,
    Ambiguous,
}

pub(super) struct SpecSlots<'a> {
    players: BTreeMap<&'a str, Observation>,
    offset: Option<i64>,
}

#[derive(Clone, Copy)]
struct Observation {
    user_id: i32,
    steamid: Option<u64>,
    ambiguous: bool,
}

impl<'a> SpecSlots<'a> {
    /// Events are in parser order. The configured value, if present, replaces offset inference.
    pub(super) fn from_events(
        events: &'a [GameEvent],
        match_start_tick: i32,
        configured_offset: Option<&str>,
    ) -> Self {
        let mut players = BTreeMap::<&str, Observation>::new();
        for event in events.iter().filter(|event| {
            event.name == "player_death"
                && (match_start_tick <= 0 || event.tick >= match_start_tick)
        }) {
            // Apply the victim before the attacker, as the released event lookup does.
            for (name_key, id_key, steam_key) in [
                ("user_name", "user_user_id", "user_steamid"),
                ("attacker_name", "attacker_user_id", "attacker_steamid"),
            ] {
                let Some(Variant::String(name)) = field(event, name_key) else {
                    continue;
                };
                let Some(name) = player_name(name) else {
                    continue;
                };
                let Some(user_id) = field(event, id_key)
                    .and_then(integer)
                    .and_then(|value| i32::try_from(value).ok())
                    .filter(|&value| value >= 0)
                else {
                    continue;
                };
                let steamid = match field(event, steam_key) {
                    Some(Variant::U64(id)) if *id != 0 => Some(*id),
                    _ => None,
                };
                players
                    .entry(name)
                    .and_modify(|previous| {
                        let same_player = previous.steamid.is_some() && previous.steamid == steamid;
                        previous.ambiguous |= conflicting_steamids(previous.steamid, steamid)
                            || (previous.user_id != user_id && !same_player);
                        previous.user_id = user_id;
                        previous.steamid = steamid.or(previous.steamid);
                    })
                    .or_insert(Observation {
                        user_id,
                        steamid,
                        ambiguous: false,
                    });
            }
        }
        let offset = match configured_offset {
            Some(value) => parse_offset(value),
            None => {
                // The legacy helper accepts a Demo path but uses only the observed IDs.
                let min = players.values().map(|row| row.user_id).min();
                let max = players.values().map(|row| row.user_id).max();
                let add_one = min == Some(0)
                    || matches!(min.zip(max), Some((min, max)) if min >= 2 && max >= 11);
                Some(i64::from(add_one))
            }
        };
        Self { players, offset }
    }

    pub(super) fn lookup(&self, name: &str) -> SlotLookup {
        let Some(name) = player_name(name) else {
            return SlotLookup::Missing;
        };
        let observation = if let Some(exact) = self.players.get(name) {
            exact
        } else {
            let key = name.to_lowercase();
            let mut candidates = self.players.iter().filter_map(|(candidate, record)| {
                (candidate.to_lowercase() == key).then_some(record)
            });
            let Some(first) = candidates.next() else {
                return SlotLookup::Missing;
            };
            let mut steamid = first.steamid;
            for other in candidates {
                if other.ambiguous
                    || first.user_id != other.user_id
                    || conflicting_steamids(steamid, other.steamid)
                {
                    return SlotLookup::Ambiguous;
                }
                steamid = steamid.or(other.steamid);
            }
            first
        };
        if observation.ambiguous {
            return SlotLookup::Ambiguous;
        }
        self.offset
            .and_then(|offset| i64::from(observation.user_id).checked_add(offset))
            .map(SlotLookup::Known)
            .unwrap_or(SlotLookup::Missing)
    }
}

fn conflicting_steamids(left: Option<u64>, right: Option<u64>) -> bool {
    matches!((left, right), (Some(left), Some(right)) if left != right)
}

fn parse_offset(value: &str) -> Option<i64> {
    let value = player_name(value).unwrap_or_default();
    let numeric = if value.contains('_') {
        let bytes = value.as_bytes();
        if bytes.iter().enumerate().any(|(index, &byte)| {
            byte == b'_'
                && !(index > 0
                    && index + 1 < bytes.len()
                    && bytes[index - 1].is_ascii_digit()
                    && bytes[index + 1].is_ascii_digit())
        }) {
            return Some(0);
        }
        Cow::Owned(value.replace('_', ""))
    } else {
        Cow::Borrowed(value)
    };
    let offset = numeric.parse::<f64>().unwrap_or_default();
    if offset.is_nan() {
        return Some(0);
    }
    (offset.is_finite() && offset < i64::MAX as f64).then(|| offset.max(0.0) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        analysis_events::{self, EventKind},
        replay_io,
        round_windows::match_start_tick,
    };
    use parser::second_pass::{
        game_events::EventField, parser_settings::create_huffman_lookup_table,
    };

    fn death(tick: i32, name: &str, user_id: i32, steamid: u64) -> GameEvent {
        GameEvent {
            name: "player_death".to_owned(),
            tick,
            fields: [
                ("user_name", Variant::String(name.to_owned())),
                ("user_user_id", Variant::I32(user_id)),
                ("user_steamid", Variant::U64(steamid)),
            ]
            .map(|(name, value)| EventField {
                name: name.to_owned(),
                data: Some(value),
            })
            .into(),
        }
    }

    #[test]
    fn resolves_exact_and_reconnected_names_and_retains_ambiguity() {
        let events = [
            death(10, "Warmup", 0, 101),
            death(100, "Alpha", 1, 102),
            death(110, "Echo", 2, 103),
            death(120, "echo", 3, 104),
            death(130, "Alpha", 4, 102),
            death(140, "Duplicate", 6, 105),
            death(150, "Duplicate", 7, 106),
            death(160, "Untracked", 8, 0),
            death(170, "Untracked", 9, 0),
            death(180, "Invalid", -1, 107),
        ];
        let slots = SpecSlots::from_events(&events, 100, None);
        assert_eq!(slots.lookup(" alpha "), SlotLookup::Known(4));
        assert_eq!(slots.lookup("Echo"), SlotLookup::Known(2));
        assert_eq!(slots.lookup("echo"), SlotLookup::Known(3));
        assert_eq!(slots.lookup("ECHO"), SlotLookup::Ambiguous);
        assert_eq!(slots.lookup("Duplicate"), SlotLookup::Ambiguous);
        assert_eq!(slots.lookup("Untracked"), SlotLookup::Ambiguous);
        assert_eq!(slots.lookup("Warmup"), SlotLookup::Missing);
        assert_eq!(slots.lookup("Invalid"), SlotLookup::Missing);
    }

    #[test]
    fn applies_observed_id_and_explicit_offset_rules() {
        let zero_based = [death(100, "Alpha", 0, 101)];
        let shifted = [death(100, "Alpha", 2, 101), death(110, "Echo", 11, 102)];
        let unshifted = [death(100, "Alpha", 1, 101), death(110, "Echo", 11, 102)];
        for (events, configured, expected) in [
            (zero_based.as_slice(), None, SlotLookup::Known(1)),
            (&zero_based, Some(""), SlotLookup::Known(0)),
            (&zero_based, Some("bad offset"), SlotLookup::Known(0)),
            (&zero_based, Some("-2"), SlotLookup::Known(0)),
            (&zero_based, Some("3.8"), SlotLookup::Known(3)),
            (&zero_based, Some("1_0"), SlotLookup::Known(10)),
            (&zero_based, Some("inf"), SlotLookup::Missing),
            (&shifted, None, SlotLookup::Known(3)),
            (&unshifted, None, SlotLookup::Known(1)),
        ] {
            assert_eq!(
                SpecSlots::from_events(events, 100, configured).lookup("Alpha"),
                expected
            );
        }
    }

    #[test]
    #[ignore = "requires the pinned public demo via MGS_ANALYSIS_EVENTS_FIXTURE"]
    fn pinned_public_demo_matches_legacy_event_slots() {
        let path = std::path::PathBuf::from(
            std::env::var_os("MGS_ANALYSIS_EVENTS_FIXTURE").expect("public demo fixture path"),
        );
        replay_io::with_demo_bytes(&path, |bytes| {
            let huffman = create_huffman_lookup_table();
            let events = analysis_events::scan(bytes, &huffman, true, false)?;
            let start = match_start_tick(&events);
            assert_eq!(start, 1761);
            let slots = SpecSlots::from_events(&events, start, None);
            let mut observed = BTreeMap::new();
            for event in events
                .iter()
                .filter(|event| event.tick >= start && event.name == "player_death")
            {
                let Some(row) = analysis_events::project(event) else {
                    continue;
                };
                if let EventKind::Death {
                    attacker, victim, ..
                } = row.kind
                {
                    for player in [attacker, victim] {
                        if let Some((steamid, name)) =
                            player.steamid64.filter(|id| *id != 0).zip(player.name)
                        {
                            observed.insert(steamid, slots.lookup(name));
                        }
                    }
                }
            }
            let expected = [1, 9, 3, 8, 5, 2, 10, 4, 7, 6].map(SlotLookup::Known);
            assert_eq!(observed.into_values().collect::<Vec<_>>(), expected);
            Ok(())
        })
        .unwrap();
    }
}
