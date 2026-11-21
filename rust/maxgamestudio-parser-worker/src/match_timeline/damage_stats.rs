//! Capped health damage from the opt-in native hurt event stream.

use super::player_stats::{clean_name, Player};
use crate::{
    analysis_events::normalized_weapon,
    round_windows::{field, integer},
};
use parser::second_pass::{game_events::GameEvent, variants::Variant};
use std::collections::HashMap;

pub(super) struct DamageInput<'a> {
    pub(super) events: &'a [GameEvent],
    pub(super) match_start_tick: i32,
    pub(super) freeze_ticks: &'a [i64],
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Tally {
    pub(super) total: usize,
    pub(super) utility: usize,
}

pub(super) fn accumulate(
    input: DamageInput<'_>,
    players: &[&Player<'_>],
    by_name: &HashMap<String, usize>,
) -> Vec<Tally> {
    let mut tally = vec![Tally::default(); players.len()];
    if players.is_empty() {
        return tally;
    }
    let hurt = input
        .events
        .iter()
        .filter(|event| event.name == "player_hurt" && event.tick >= input.match_start_tick);
    let has_health = hurt
        .clone()
        .any(|event| event.fields.iter().any(|entry| entry.name == "health"));
    let mut freezes: Vec<_> = input
        .freeze_ticks
        .iter()
        .copied()
        .filter(|&tick| tick > 0)
        .collect();
    freezes.sort_unstable();
    let mut freezes = freezes.into_iter().peekable();
    let mut remaining_hp = HashMap::new();

    // Like the released reducer, consume post-start hurt rows in stable tick order.
    // Visible freeze ticks reset the HP table and also define the ADR denominator.
    for event in hurt {
        while freezes
            .next_if(|&tick| tick <= i64::from(event.tick))
            .is_some()
        {
            remaining_hp.clear();
        }
        let attacker = role_name(event, "attacker_name");
        let victim = role_name(event, "user_name");
        let victim_key = victim_key(event, victim);
        let health_after =
            has_health.then(|| field(event, "health").and_then(integer).unwrap_or(0).max(0));
        let attacker_index = by_name.get(&attacker.to_lowercase()).copied();
        let victim_index = by_name.get(&victim.to_lowercase()).copied();
        let teammates = attacker_index
            .zip(victim_index)
            .is_some_and(|(attacker, victim)| {
                players[attacker].team_key == players[victim].team_key
            });
        if attacker.is_empty() || victim.is_empty() || attacker == victim || teammates {
            if let Some(hp) = health_after.filter(|_| !victim_key.is_empty()) {
                remaining_hp.insert(victim_key, hp);
            }
            continue;
        }

        let previous = remaining_hp.get(&victim_key).copied().unwrap_or(100);
        let dealt = previous.min(reported_damage(event));
        remaining_hp.insert(victim_key, health_after.unwrap_or(previous - dealt));
        if let Some(attacker) = attacker_index.filter(|_| dealt > 0) {
            tally[attacker].total += dealt as usize;
            if matches!(
                normalized_weapon(event).as_str(),
                "hegrenade" | "inferno" | "molotov" | "incgrenade" | "incendiary"
            ) {
                tally[attacker].utility += dealt as usize;
            }
        }
    }
    tally
}

fn role_name<'a>(event: &'a GameEvent, key: &str) -> &'a str {
    match field(event, key) {
        Some(Variant::String(value)) => clean_name(value),
        _ => "",
    }
}

fn victim_key(event: &GameEvent, victim: &str) -> String {
    match field(event, "user_steamid") {
        Some(Variant::String(value)) if !clean_name(value).is_empty() => {
            clean_name(value).to_owned()
        }
        Some(Variant::U64(id)) => id.to_string(),
        Some(Variant::U32(id)) => id.to_string(),
        Some(Variant::I32(id)) => id.to_string(),
        _ => victim.to_lowercase(),
    }
}

fn reported_damage(event: &GameEvent) -> i64 {
    ["dmg_health", "health_damage", "damage"]
        .into_iter()
        .find_map(|key| {
            field(event, key).filter(|value| !matches!(value, Variant::F32(n) if n.is_nan()))
        })
        .and_then(integer)
        .unwrap_or(0)
        .max(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::match_timeline::round_score::TeamKey::{A, B};
    use parser::second_pass::game_events::EventField;

    #[test]
    fn damage_caps_use_health_source_order_and_visible_freeze_resets() {
        let roster = players();
        let refs = roster.iter().collect::<Vec<_>>();
        let names = names(&roster);
        let mut alias = hurt(
            190,
            ("Delta", "Bravo"),
            "2",
            "ak47",
            ("dmg_health", -3),
            Some(40),
        );
        alias
            .fields
            .push(field_value("health_damage", Variant::I32(80)));
        let events = [
            hurt(
                90,
                ("Alpha", "Charlie"),
                "3",
                "ak47",
                ("dmg_health", 100),
                Some(0),
            ),
            hurt(
                110,
                ("Alpha", "Charlie"),
                "3",
                "ak47",
                ("dmg_health", 10),
                Some(90),
            ),
            hurt(
                150,
                ("Bravo", "Alpha"),
                "1",
                "ak47",
                ("dmg_health", 60),
                Some(40),
            ),
            hurt(
                160,
                ("Charlie", "Alpha"),
                "1",
                "weapon_HEGRENADE",
                ("dmg_health", 123),
                Some(0),
            ),
            hurt(
                170,
                ("", "Delta"),
                "4",
                "world",
                ("dmg_health", 20),
                Some(80),
            ),
            hurt(
                170,
                ("Alpha", "Delta Alias"),
                "4",
                "incendiary",
                ("dmg_health", 120),
                Some(0),
            ),
            hurt(
                180,
                ("Alpha", "Alpha"),
                "1",
                "world",
                ("dmg_health", 60),
                Some(25),
            ),
            hurt(
                181,
                ("Delta", "Alpha"),
                "1",
                "awp",
                ("health_damage", 50),
                Some(0),
            ),
            alias,
            hurt(
                191,
                ("Delta", "Bravo"),
                "2",
                "inferno",
                ("damage", 100),
                Some(0),
            ),
            hurt(
                220,
                ("Alpha", "Ghost"),
                "",
                "item_hegrenade",
                ("dmg_health", 30),
                Some(70),
            ),
            hurt(
                221,
                ("Alpha", "ghost"),
                "",
                "smokegrenade",
                ("dmg_health", 100),
                Some(0),
            ),
            hurt(
                400,
                ("Alpha", "Delta"),
                "4",
                "weapon_molotov",
                ("dmg_health", 200),
                Some(0),
            ),
            hurt(
                501,
                ("Bravo", "Charlie"),
                "3",
                "ak47",
                ("dmg_health", 30),
                None,
            ),
            hurt(
                502,
                ("Bravo", "Charlie"),
                "3",
                "ak47",
                ("dmg_health", 20),
                None,
            ),
        ];
        let totals = accumulate(
            DamageInput {
                events: &events,
                match_start_tick: 100,
                freeze_ticks: &[150, 400],
            },
            &refs,
            &names,
        );
        // Captured from the released capped damage reducer with the same ordered hits.
        assert_eq!(
            totals,
            [
                Tally {
                    total: 290,
                    utility: 210
                },
                Tally {
                    total: 30,
                    utility: 0
                },
                Tally {
                    total: 40,
                    utility: 40
                },
                Tally {
                    total: 65,
                    utility: 40
                },
            ]
        );
    }

    #[test]
    fn entire_stream_without_health_uses_estimated_remaining_hp() {
        let roster = players();
        let refs = roster.iter().collect::<Vec<_>>();
        let events = [
            hurt(
                201,
                ("Alpha", "Charlie"),
                "3",
                "ak47",
                ("dmg_health", 28),
                None,
            ),
            hurt(
                202,
                ("Alpha", "Charlie"),
                "3",
                "hegrenade",
                ("dmg_health", 150),
                None,
            ),
            hurt(
                203,
                ("Alpha", "Charlie"),
                "3",
                "hegrenade",
                ("dmg_health", 1),
                None,
            ),
        ];
        let totals = accumulate(
            DamageInput {
                events: &events,
                match_start_tick: 100,
                freeze_ticks: &[150, 400],
            },
            &refs,
            &names(&roster),
        );
        assert_eq!(
            totals[0],
            Tally {
                total: 100,
                utility: 72
            }
        );
    }

    fn players() -> [Player<'static>; 4] {
        [
            ("Alpha", "1", A, 2),
            ("Bravo", "2", A, 2),
            ("Charlie", "3", B, 3),
            ("Delta", "4", B, 3),
        ]
        .map(|(name, steam_id64, team_key, final_group)| Player {
            name,
            steam_id64,
            team_key,
            final_group,
        })
    }

    fn names(players: &[Player<'_>]) -> HashMap<String, usize> {
        players
            .iter()
            .enumerate()
            .map(|(index, player)| (player.name.to_lowercase(), index))
            .collect()
    }

    fn hurt(
        tick: i32,
        roles: (&str, &str),
        victim_steamid: &str,
        weapon: &str,
        damage: (&str, i32),
        health: Option<i32>,
    ) -> GameEvent {
        let mut fields = vec![
            field_value("attacker_name", Variant::String(roles.0.into())),
            field_value("user_name", Variant::String(roles.1.into())),
            field_value("user_steamid", Variant::String(victim_steamid.into())),
            field_value("weapon", Variant::String(weapon.into())),
            field_value(damage.0, Variant::I32(damage.1)),
        ];
        if let Some(health) = health {
            fields.push(field_value("health", Variant::I32(health)));
        }
        GameEvent {
            name: "player_hurt".into(),
            tick,
            fields,
        }
    }

    fn field_value(name: &str, value: Variant) -> EventField {
        EventField {
            name: name.into(),
            data: Some(value),
        }
    }
}
