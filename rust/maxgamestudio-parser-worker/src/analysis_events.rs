//! One pinned-parser event batch for replay and recording source analysis.

use crate::{
    demo_inspection::{player_name, CONTROLLER_TEAM},
    parser_inputs,
    round_windows::{field, integer},
    Failure,
};
use parser::{
    first_pass::parser_settings::rm_user_friendly_names,
    parse_demo::{Parser, ParsingMode},
    second_pass::{game_events::GameEvent, variants::Variant},
};

const EVENTS: &[&str] = &[
    "round_announce_match_start",
    "round_start",
    "round_freeze_end",
    "round_end",
    "player_death",
    "bomb_planted",
    "bomb_defused",
    "bomb_exploded",
    "bomb_dropped",
    "bomb_pickup",
    "weapon_fire",
    "hegrenade_detonate",
    "inferno_startburn",
    "molotov_detonate",
    "smokegrenade_detonate",
    "flashbang_detonate",
];

/// The caller owns the returned events and assigns them to match windows.
pub(crate) fn scan(
    bytes: &[u8],
    huffman: &Vec<(u8, u8)>,
    include_hurt: bool,
    include_recording: bool,
) -> Result<Vec<GameEvent>, Failure> {
    let player_fields = [
        "X",
        "Y",
        "Z",
        "yaw",
        "pitch",
        "last_place_name",
        "team_num",
        CONTROLLER_TEAM,
        "user_id",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let other_fields = vec!["total_rounds_played".to_owned()];
    let unavailable = || Failure::replay_parse("Demo analysis event properties are unavailable");
    let player_props = rm_user_friendly_names(&player_fields).map_err(|_| unavailable())?;
    let other_props = rm_user_friendly_names(&other_fields).map_err(|_| unavailable())?;

    let mut inputs = parser_inputs(huffman);
    inputs.only_header = false;
    inputs.parse_ents = true;
    inputs.real_name_to_og_name = player_props
        .iter()
        .chain(&other_props)
        .cloned()
        .zip(player_fields.into_iter().chain(other_fields))
        .collect();
    inputs.wanted_player_props = player_props;
    inputs.wanted_other_props = other_props;
    inputs.wanted_events = EVENTS.iter().map(|name| (*name).to_owned()).collect();
    if include_hurt {
        inputs.wanted_events.push("player_hurt".to_owned());
    }
    if include_recording {
        inputs
            .wanted_events
            .extend(["player_blind", "item_pickup", "bomb_begindefuse"].map(str::to_owned));
    }

    let mut events = Parser::new(inputs, ParsingMode::Normal)
        .parse_demo(bytes)
        .map_err(|_| Failure::replay_parse("could not read Demo analysis events"))?
        .game_events;
    events.sort_by_key(|event| event.tick);
    Ok(events)
}

#[derive(Debug, PartialEq)]
pub(crate) struct AnalysisRow<'a> {
    pub(crate) tick: i32,
    pub(crate) total_rounds_played: Option<i64>,
    pub(crate) kind: EventKind<'a>,
}

#[derive(Debug, PartialEq)]
pub(crate) enum EventKind<'a> {
    Death {
        attacker: Participant<'a>,
        victim: Participant<'a>,
        assister: Participant<'a>,
        weapon: String,
        headshot: bool,
    },
    Bomb {
        kind: BombKind,
        player: Participant<'a>,
        site: Option<char>,
    },
    WeaponFire {
        player: Participant<'a>,
        weapon: String,
        is_bullet: bool,
        grenade_kind: Option<&'static str>,
        yaw: Option<f64>,
        pitch: Option<f64>,
    },
    GrenadeDetonation {
        player: Participant<'a>,
        kind: &'static str,
        position: Option<Position>,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BombKind {
    Plant,
    Defuse,
    Explode,
    Drop,
    Pickup,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Participant<'a> {
    pub(crate) name: Option<&'a str>,
    pub(crate) steamid64: Option<u64>,
    pub(crate) user_id: Option<i32>,
    pub(crate) team_num: Option<i32>,
    pub(crate) position: Option<Position>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Position {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) z: Option<f64>,
}

/// Read the features shared by analysis callers while leaving source fields intact.
pub(crate) fn project(event: &GameEvent) -> Option<AnalysisRow<'_>> {
    let kind = match event.name.as_str() {
        "player_death" => EventKind::Death {
            attacker: participant(event, "attacker_"),
            victim: user(event, "player_name"),
            assister: participant(event, "assister_"),
            weapon: normalized_weapon(event),
            headshot: matches!(field(event, "headshot"), Some(Variant::Bool(true))),
        },
        "weapon_fire" => {
            let weapon = normalized_weapon(event);
            let grenade_kind = match weapon.as_str() {
                "smokegrenade" => Some("烟雾弹"),
                "flashbang" => Some("闪光弹"),
                "hegrenade" => Some("HE 手雷"),
                "molotov" | "incgrenade" | "incendiary" | "inferno" => Some("燃烧弹"),
                "decoy" => Some("诱饵弹"),
                _ => None,
            };
            let is_bullet = !matches!(
                weapon.as_str(),
                "" | "c4"
                    | "knife"
                    | "knife_t"
                    | "taser"
                    | "hegrenade"
                    | "flashbang"
                    | "smokegrenade"
                    | "molotov"
                    | "incgrenade"
                    | "incendiary"
                    | "decoy"
            );
            EventKind::WeaponFire {
                player: user(event, "name"),
                weapon,
                is_bullet,
                grenade_kind,
                yaw: field(event, "user_yaw").and_then(rounded_number),
                pitch: field(event, "user_pitch").and_then(rounded_number),
            }
        }
        "bomb_planted" => bomb(event, BombKind::Plant),
        "bomb_defused" => bomb(event, BombKind::Defuse),
        "bomb_exploded" => bomb(event, BombKind::Explode),
        "bomb_dropped" => bomb(event, BombKind::Drop),
        "bomb_pickup" => bomb(event, BombKind::Pickup),
        "hegrenade_detonate" => detonation(event, "HE 手雷"),
        "inferno_startburn" | "molotov_detonate" => detonation(event, "燃烧弹"),
        "smokegrenade_detonate" => detonation(event, "烟雾弹"),
        "flashbang_detonate" => detonation(event, "闪光弹"),
        _ => return None,
    };
    Some(AnalysisRow {
        tick: event.tick,
        total_rounds_played: field(event, "total_rounds_played").and_then(integer),
        kind,
    })
}

fn bomb(event: &GameEvent, kind: BombKind) -> EventKind<'_> {
    let place = [
        "user_last_place_name",
        "player_last_place_name",
        "last_place_name",
    ]
    .into_iter()
    .find_map(|key| field(event, key).and_then(clean_text))
    .unwrap_or_default();
    // The engine's numeric site is a map entity ID, independent of A or B.
    let site = if place.eq_ignore_ascii_case("bombsitea") {
        Some('A')
    } else if place.eq_ignore_ascii_case("bombsiteb") {
        Some('B')
    } else {
        match field(event, "site").and_then(clean_text) {
            Some("A" | "a") => Some('A'),
            Some("B" | "b") => Some('B'),
            _ => None,
        }
    };
    EventKind::Bomb {
        kind,
        player: user(event, "defuser"),
        site,
    }
}

fn detonation<'a>(event: &'a GameEvent, kind: &'static str) -> EventKind<'a> {
    let player = user(event, "name");
    let [x, y, z] = ["x", "y", "z"].map(|key| field(event, key).and_then(rounded_number));
    let position = x
        .zip(y)
        .map(|(x, y)| Position { x, y, z })
        .or(player.position);
    EventKind::GrenadeDetonation {
        player,
        kind,
        position,
    }
}

fn user<'a>(event: &'a GameEvent, name_fallback: &str) -> Participant<'a> {
    let mut player = participant(event, "user_");
    player.name = player.name.or_else(|| {
        ["player_name", name_fallback]
            .into_iter()
            .find_map(|key| field(event, key).and_then(clean_text))
    });
    player
}

fn participant<'a>(event: &'a GameEvent, prefix: &str) -> Participant<'a> {
    let mut player = Participant::default();
    let [mut x, mut y, mut z] = [None; 3];
    for property in &event.fields {
        let Some((key, value)) = property
            .name
            .strip_prefix(prefix)
            .zip(property.data.as_ref())
        else {
            continue;
        };
        match key {
            "name" => player.name = clean_text(value),
            "steamid" => player.steamid64 = steamid(value),
            "user_id" => {
                player.user_id = integer(value)
                    .and_then(|id| i32::try_from(id).ok())
                    .filter(|id| *id >= 0)
            }
            "team_num" => player.team_num = integer(value).and_then(|id| i32::try_from(id).ok()),
            "X" => x = rounded_number(value),
            "Y" => y = rounded_number(value),
            "Z" => z = rounded_number(value),
            _ => {}
        }
    }
    player.position = x.zip(y).map(|(x, y)| Position { x, y, z });
    player
}

fn clean_text(value: &Variant) -> Option<&str> {
    let Variant::String(text) = value else {
        return None;
    };
    player_name(text)
        .filter(|name| !name.eq_ignore_ascii_case("nat") && !name.eq_ignore_ascii_case("none"))
}

fn steamid(value: &Variant) -> Option<u64> {
    let value = match value {
        Variant::String(text) => {
            let text = text.trim();
            text.strip_suffix(".0").unwrap_or(text).parse().ok()?
        }
        Variant::U64(id) => *id,
        Variant::I32(_) | Variant::U32(_) => u64::try_from(integer(value)?).ok()?,
        _ => return None,
    };
    (value > 0).then_some(value)
}

pub(crate) fn normalized_weapon(event: &GameEvent) -> String {
    let mut weapon = field(event, "weapon")
        .and_then(clean_text)
        .unwrap_or_default()
        .to_lowercase();
    for prefix in ["weapon_", "item_"] {
        if weapon.starts_with(prefix) {
            drop(weapon.drain(..prefix.len()));
        }
    }
    weapon
}

fn rounded_number(value: &Variant) -> Option<f64> {
    let value = match value {
        Variant::F32(value) => f64::from(*value),
        Variant::I32(value) => f64::from(*value),
        Variant::U32(value) => f64::from(*value),
        _ => return None,
    };
    // A widened f32 times 100 is exact in f64, including halfway cases.
    value
        .is_finite()
        .then(|| (value * 100.0).round_ties_even() / 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{replay_io, round_windows::match_start_tick};
    use parser::second_pass::{
        game_events::EventField, parser_settings::create_huffman_lookup_table,
    };
    use serde_json::{json, Value};
    use std::path::PathBuf;

    #[test]
    fn death_preserves_engine_zero_and_exact_unsigned_id_with_rounded_coordinates() {
        let source = event(
            "player_death",
            vec![
                ("attacker_name", Variant::String("\u{1c}one  ".into())),
                (
                    "attacker_steamid",
                    Variant::String("76561198000000001".into()),
                ),
                ("attacker_user_id", Variant::I32(0)),
                ("attacker_X", Variant::F32(1.125)),
                ("attacker_Y", Variant::F32(-2.375)),
                ("attacker_Z", Variant::F32(f32::NAN)),
                ("user_name", Variant::String("two".into())),
                ("user_steamid", Variant::String("0".into())),
                ("user_user_id", Variant::I32(-1)),
                ("weapon", Variant::String(" Weapon_Item_AWP ".into())),
                ("headshot", Variant::Bool(true)),
                ("total_rounds_played", Variant::U32(3)),
            ],
        );
        let row = project(&source).unwrap();
        assert_eq!((row.tick, row.total_rounds_played), (180, Some(3)));
        let EventKind::Death {
            attacker,
            victim,
            assister,
            weapon,
            headshot,
        } = row.kind
        else {
            panic!("expected a death row")
        };
        assert_eq!(attacker.name, Some("one"));
        assert_eq!(attacker.steamid64, Some(76561198000000001));
        assert_eq!(attacker.user_id, Some(0));
        assert_eq!(
            attacker.position,
            Some(Position {
                x: 1.12,
                y: -2.38,
                z: None
            })
        );
        assert_eq!(victim.name, Some("two"));
        assert_eq!((victim.steamid64, victim.user_id), (None, None));
        assert_eq!(assister.name, None);
        assert_eq!(weapon, "awp");
        assert!(headshot);
    }

    #[test]
    fn bomb_entity_site_and_grenade_release_keep_distinct_meaning() {
        let explosion = event("bomb_exploded", vec![("site", Variant::I32(2))]);
        assert!(matches!(
            project(&explosion).unwrap().kind,
            EventKind::Bomb {
                kind: BombKind::Explode,
                site: None,
                ..
            }
        ));
        let plant = event(
            "bomb_planted",
            vec![("user_last_place_name", Variant::String("BombsiteB".into()))],
        );
        assert!(matches!(
            project(&plant).unwrap().kind,
            EventKind::Bomb {
                kind: BombKind::Plant,
                site: Some('B'),
                ..
            }
        ));
        let fire = event(
            "weapon_fire",
            vec![("weapon", Variant::String("weapon_incgrenade".into()))],
        );
        assert!(matches!(
            project(&fire).unwrap().kind,
            EventKind::WeaponFire {
                is_bullet: false,
                grenade_kind: Some("燃烧弹"),
                yaw: None,
                ..
            }
        ));
    }

    #[test]
    #[ignore = "requires the existing pinned public demo via MGS_ANALYSIS_EVENTS_FIXTURE"]
    fn pinned_public_demo_matches_legacy_event_rows() {
        let path = PathBuf::from(
            std::env::var_os("MGS_ANALYSIS_EVENTS_FIXTURE").expect("public demo fixture path"),
        );
        let huffman = create_huffman_lookup_table();
        let events = replay_io::with_demo_bytes(&path, |bytes| scan(bytes, &huffman, false, false))
            .expect("pinned public demo scan");
        let start = match_start_tick(&events);
        assert_eq!(start, 1761);
        assert!(events.windows(2).all(|pair| pair[0].tick <= pair[1].tick));

        // Goldens from the released wheel's shared batch and dedicated death parse.
        // The hashes cover ordered public Demo rows without copying names or IDs into the test.
        for (name, count, expected) in [
            ("player_death", 73, 0xb751d06e53ae4e60),
            ("bomb_planted", 7, 0x69c0d11e3ffdb3bb),
            ("bomb_defused", 1, 0x4ca3dd1d79ffe8f8),
            ("bomb_exploded", 2, 0x8d618b28b18a6621),
            ("bomb_dropped", 14, 0x8767f6924324f008),
            ("bomb_pickup", 20, 0x5ef7b0156f615470),
            ("weapon_fire", 1590, 0x3f19c1cbf390faf8),
            ("hegrenade_detonate", 34, 0xe3f5896eb5bf5f47),
            ("inferno_startburn", 25, 0x1e901b42fa83c993),
            ("molotov_detonate", 0, 0xcbf29ce484222325),
            ("smokegrenade_detonate", 40, 0xb4f7e48783211171),
            ("flashbang_detonate", 48, 0xa7676116704c3913),
            ("round_announce_match_start", 1, 0x916fa48f310dc5ce),
            ("round_start", 9, 0xd1fd395d7d537fd7),
            ("round_freeze_end", 9, 0x0e46a15dba97f94b),
            ("round_end", 10, 0x150b6d1f4b537d51),
        ] {
            let matching: Vec<_> = events
                .iter()
                .filter(|row| row.tick >= start && row.name == name)
                .collect();
            assert_eq!(matching.len(), count, "{name} count");
            let hash = matching.iter().fold(0xcbf29ce484222325, |hash, row| {
                oracle_row(row)
                    .to_string()
                    .bytes()
                    .chain(std::iter::once(b'\n'))
                    .fold(hash, |hash: u64, byte| {
                        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
                    })
            });
            assert_eq!(hash, expected, "{name} rows differ from the released wheel");
        }
    }

    fn oracle_row(event: &GameEvent) -> Value {
        let data = match project(event).map(|row| row.kind) {
            Some(EventKind::Death {
                attacker,
                victim,
                assister,
                weapon,
                headshot,
            }) => json!([
                oracle_actor(attacker),
                oracle_actor(victim),
                assister.name.unwrap_or_default(),
                assister.steamid64.map(|id| id.to_string()),
                weapon,
                headshot,
                attacker.user_id,
                victim.user_id
            ]),
            Some(EventKind::Bomb { player, site, .. }) => json!([oracle_actor(player), site]),
            Some(EventKind::WeaponFire {
                player,
                weapon,
                is_bullet,
                grenade_kind,
                yaw,
                pitch,
            }) => json!([
                oracle_actor(player),
                weapon,
                is_bullet,
                grenade_kind,
                yaw.map(oracle_cents),
                pitch.map(oracle_cents)
            ]),
            Some(EventKind::GrenadeDetonation {
                player, position, ..
            }) => json!([
                player.name.unwrap_or_default(),
                player.steamid64.map(|id| id.to_string()),
                position.map(oracle_position)
            ]),
            None => json!([]),
        };
        json!([
            event.tick,
            field(event, "total_rounds_played").and_then(integer),
            data
        ])
    }

    fn oracle_actor(player: Participant<'_>) -> Value {
        json!([
            player.name.unwrap_or_default(),
            player.steamid64.map(|id| id.to_string()),
            player.position.map(oracle_position)
        ])
    }

    fn oracle_position(position: Position) -> Value {
        json!([
            oracle_cents(position.x),
            oracle_cents(position.y),
            position.z.map(oracle_cents)
        ])
    }

    fn oracle_cents(value: f64) -> i64 {
        (value * 100.0).round() as i64
    }

    fn event(name: &str, fields: Vec<(&str, Variant)>) -> GameEvent {
        GameEvent {
            name: name.to_owned(),
            tick: 180,
            fields: fields
                .into_iter()
                .map(|(name, data)| EventField {
                    name: name.to_owned(),
                    data: Some(data),
                })
                .collect(),
        }
    }
}
