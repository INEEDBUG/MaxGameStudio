//! Freeze-end equipment totals and fixed-team buy votes from sampled player rows.

use super::{
    player_stats::{clean_name, Player},
    round_score::{Side, TeamKey},
};
use crate::{
    demo_inspection::{named_column, team_at_column, CONTROLLER_TEAM},
    round_windows::{field, integer},
};
use parser::{
    first_pass::prop_controller::{NAME_ID, TICK_ID},
    parse_demo::DemoOutput,
    second_pass::{game_events::GameEvent, variants::VarVec},
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

const EQUIPMENT: &str = "current_equip_value";
const SPENT: &str = "cash_spent_this_round";
const BALANCE: &str = "start_balance";
const ALIVE: &str = "is_alive";
pub(super) const PROPERTIES: &[&str] = &[EQUIPMENT, SPENT, BALANCE, ALIVE];

pub(super) fn freeze_end_rounds(events: &[GameEvent], start: i32) -> BTreeMap<i32, i64> {
    let freezes = events
        .iter()
        .filter(|event| event.name == "round_freeze_end" && event.tick >= start);
    let numbered = freezes
        .clone()
        .any(|event| field(event, "total_rounds_played").is_some());
    let by_tick: BTreeMap<_, _> = freezes
        .filter(|event| event.tick > 0)
        .map(|event| {
            (
                event.tick,
                field(event, "total_rounds_played").and_then(integer),
            )
        })
        .collect();
    by_tick
        .into_iter()
        .enumerate()
        .map(|(index, (tick, played))| {
            let round = if numbered {
                played.unwrap_or_default().saturating_add(1)
            } else {
                index as i64 + 1
            };
            (tick, round)
        })
        .collect()
}

#[derive(Clone, Copy)]
enum Buy {
    Pistol,
    Full,
    Force,
    Semi,
    Eco,
}

// The released workspace breaks ties in this order.
const BUY_LABELS: [&str; 5] = ["pistol", "full", "force", "semi", "eco"];

#[derive(Default)]
pub(super) struct RoundEconomy {
    equipment_by_side: [Option<i64>; 2],
    votes_by_team: [[usize; BUY_LABELS.len()]; 2],
}

impl RoundEconomy {
    pub(super) fn write(&self, round: &mut Value, sides: [Side; 2]) {
        for (index, (equipment_key, economy_key)) in [
            ("team_a_equipment_value", "team_a_economy"),
            ("team_b_equipment_value", "team_b_economy"),
        ]
        .into_iter()
        .enumerate()
        {
            if let Some(total) = self.equipment_by_side[side_index(sides[index])] {
                round[equipment_key] = json!(total.max(0));
            }
            let votes = self.votes_by_team[index];
            if let Some(&max_votes) = votes.iter().max().filter(|&&count| count > 0) {
                let buy = votes.iter().position(|&count| count == max_votes).unwrap();
                round[economy_key] = json!(BUY_LABELS[buy]);
            }
        }
    }
}

#[derive(Default)]
pub(super) struct MatchEconomy {
    pub(super) rounds: BTreeMap<i64, RoundEconomy>,
    players: BTreeMap<String, PlayerEquipment>,
}

struct PlayerEquipment {
    team_key: TeamKey,
    total: i64,
    observations: usize,
}

impl MatchEconomy {
    pub(super) fn average_equipment(&self, name: &str) -> i64 {
        self.players
            .get(&clean_name(name).to_lowercase())
            .filter(|player| player.observations > 0)
            .map(|player| {
                (player.total as f64 / player.observations as f64).round_ties_even() as i64
            })
            .unwrap_or_default()
    }
}

pub(super) fn collect(
    output: &DemoOutput,
    tick_to_round: &BTreeMap<i32, i64>,
    players: &[Player<'_>],
    halftime: Option<i64>,
) -> MatchEconomy {
    let column = |id| output.df.get(&id).and_then(|column| column.data.as_ref());
    let Some(VarVec::I32(ticks)) = column(TICK_ID) else {
        return MatchEconomy::default();
    };
    let names = match column(NAME_ID) {
        Some(VarVec::String(values)) => values.as_slice(),
        _ => &[],
    };
    let equipment = named_column(output, EQUIPMENT);
    let spent = named_column(output, SPENT);
    let balance = named_column(output, BALANCE);
    let alive = match named_column(output, ALIVE) {
        Some(VarVec::Bool(values)) => Some(values),
        _ => None,
    };
    let pawn_team = named_column(output, "team_num");
    let controller_team = named_column(output, CONTROLLER_TEAM);
    let rows = ticks.iter().enumerate().filter_map(|(index, &tick)| {
        let tick = tick?;
        let side =
            team_at_column(pawn_team, index).or_else(|| team_at_column(controller_team, index));
        Some(Row {
            tick,
            name: names.get(index).and_then(Option::as_deref),
            side: side.and_then(|team| Side::from_engine_team(i64::from(team))),
            alive: alive.map_or(Some(true), |values| values.get(index).copied().flatten()),
            equipment: amount_at(equipment, index),
            spent: amount_at(spent, index),
            balance: amount_at(balance, index),
        })
    });
    summarize(rows, tick_to_round, players, halftime)
}

struct Row<'a> {
    tick: i32,
    name: Option<&'a str>,
    side: Option<Side>,
    alive: Option<bool>,
    equipment: Option<i64>,
    spent: Option<i64>,
    balance: Option<i64>,
}

fn summarize<'a>(
    rows: impl IntoIterator<Item = Row<'a>>,
    tick_to_round: &BTreeMap<i32, i64>,
    players: &[Player<'_>],
    halftime: Option<i64>,
) -> MatchEconomy {
    let players = players
        .iter()
        .map(|player| {
            (
                clean_name(player.name).to_lowercase(),
                PlayerEquipment {
                    team_key: player.team_key,
                    total: 0,
                    observations: 0,
                },
            )
        })
        .collect();
    let mut economy = MatchEconomy {
        rounds: BTreeMap::new(),
        players,
    };
    let mut equipment_by_tick = BTreeMap::<i32, [Option<i64>; 2]>::new();
    for row in rows {
        let Some(&round) = tick_to_round.get(&row.tick) else {
            continue;
        };
        let by_side = equipment_by_tick.entry(row.tick).or_default();
        let equipment = row
            .alive
            .and_then(|alive| if alive { row.equipment } else { Some(0) });
        if let (Some(side), Some(amount)) = (row.side, equipment) {
            *by_side[side_index(side)].get_or_insert(0) += amount;
        }
        let Some(player) = row
            .name
            .map(clean_name)
            .filter(|name| !name.is_empty())
            .and_then(|name| economy.players.get_mut(&name.to_lowercase()))
        else {
            continue;
        };
        // The average includes repeated freezes, dead players and freezes
        // outside the displayed rounds. Missing equipment contributes zero.
        if round != 0 {
            player.total += row.equipment.unwrap_or_default().max(0);
            player.observations += 1;
        }
        if let Some(buy) = buy_type(&row, round, halftime) {
            let team = usize::from(player.team_key == TeamKey::B);
            economy.rounds.entry(round).or_default().votes_by_team[team][buy as usize] += 1;
        }
    }
    for (tick, totals) in equipment_by_tick {
        // The latest observed freeze replaces the sums for that round.
        economy
            .rounds
            .entry(tick_to_round[&tick])
            .or_default()
            .equipment_by_side = totals;
    }
    economy
}

fn amount_at(column: Option<&VarVec>, index: usize) -> Option<i64> {
    match column? {
        VarVec::I32(values) => values.get(index)?.map(i64::from),
        VarVec::U32(values) => values.get(index)?.map(i64::from),
        _ => None,
    }
}

fn side_index(side: Side) -> usize {
    usize::from(side == Side::Ct)
}

fn buy_type(row: &Row<'_>, round: i64, halftime: Option<i64>) -> Option<Buy> {
    if round == 1 || halftime == Some(round) {
        return Some(Buy::Pistol);
    }
    let equipment = row.equipment?.max(0);
    if equipment >= 4000 {
        return Some(Buy::Full);
    }
    let spent = row.spent?.max(0);
    if spent < 1000 && equipment < 1000 {
        return Some(Buy::Eco);
    }
    let balance = row.balance?.max(0);
    if balance > 0 && spent as f64 / balance as f64 >= 0.80 {
        return Some(Buy::Force);
    }
    Some(Buy::Semi)
}

#[cfg(test)]
mod tests {
    use super::*;
    use parser::second_pass::{game_events::EventField, variants::Variant};

    #[test]
    fn matches_legacy_votes_and_totals_when_players_are_missing_at_halftime() {
        let freezes = [(20, 0), (100, 0), (200, 1), (220, 1), (300, 2)]
            .into_iter()
            .map(|(tick, played)| GameEvent {
                name: "round_freeze_end".to_owned(),
                tick,
                fields: vec![EventField {
                    name: "total_rounds_played".to_owned(),
                    data: Some(Variant::I32(played)),
                }],
            })
            .collect::<Vec<_>>();
        let tick_to_round = freeze_end_rounds(&freezes, 50);
        assert_eq!(
            tick_to_round,
            BTreeMap::from([(100, 1), (200, 2), (220, 2), (300, 3)])
        );
        let players = [
            ("Alpha", TeamKey::A),
            ("Bravo", TeamKey::B),
            ("Echo", TeamKey::A),
            ("Delta", TeamKey::B),
        ]
        .map(|(name, team_key)| Player {
            name,
            steam_id64: "",
            final_group: if team_key == TeamKey::A { 3 } else { 2 },
            team_key,
        });
        let (t, ct) = (Side::T, Side::Ct);
        let rows = [
            (20, "Alpha", t, true, 99000, 0, 0),
            (100, "Alpha", t, true, 4000, 1000, 2600),
            (100, "Bravo", ct, true, 2000, 900, 1500),
            (200, "Alpha", t, true, 900, 500, 1600),
            (200, "Echo", t, true, 4100, 3900, 5000),
            (200, "Bravo", ct, true, 2400, 1800, 2000),
            (220, "Alpha", t, true, 650, 250, 900),
            (220, "Echo", t, true, 4300, 800, 5100),
            (220, "Bravo", ct, true, 2100, 700, 1000),
            (220, "Bot", t, true, 1000, 0, 0),
            (220, "Delta", ct, false, 5000, 0, 0),
            (300, "Alpha", ct, true, 1500, 0, 0),
            (300, "Delta", t, false, 5000, 0, 0),
        ]
        .map(|(tick, name, side, alive, equipment, spent, balance)| Row {
            tick,
            name: Some(name),
            side: Some(side),
            alive: Some(alive),
            equipment: Some(equipment),
            spent: Some(spent),
            balance: Some(balance),
        });
        let actual = summarize(rows, &tick_to_round, &players, Some(3));
        for (number, sides, values, buys) in [
            (1, [t, ct], [4000, 2000], ["pistol", "pistol"]),
            (2, [t, ct], [5950, 2100], ["full", "full"]),
            (3, [ct, t], [1500, 0], ["pistol", "pistol"]),
        ] {
            let mut round = json!({});
            actual.rounds[&number].write(&mut round, sides);
            assert_eq!(
                round,
                json!({
                    "team_a_equipment_value": values[0],
                    "team_b_equipment_value": values[1],
                    "team_a_economy": buys[0],
                    "team_b_economy": buys[1],
                })
            );
        }
    }

    #[test]
    fn omits_unobserved_equipment_and_votes_but_keeps_observed_zero() {
        let rows = [
            Row {
                tick: 200,
                name: Some("Alpha"),
                side: Some(Side::T),
                alive: Some(true),
                equipment: None,
                spent: None,
                balance: None,
            },
            Row {
                tick: 200,
                name: None,
                side: Some(Side::Ct),
                alive: Some(true),
                equipment: Some(0),
                spent: None,
                balance: None,
            },
        ];
        let players = [Player {
            name: "Alpha",
            steam_id64: "",
            final_group: 2,
            team_key: TeamKey::A,
        }];
        let actual = summarize(rows, &BTreeMap::from([(200, 2)]), &players, None);
        let mut round = json!({});
        actual.rounds[&2].write(&mut round, [Side::T, Side::Ct]);
        assert_eq!(round, json!({"team_b_equipment_value": 0}));
    }

    #[test]
    fn average_includes_missing_signed_and_repeated_samples_like_legacy() {
        let players = [
            ("Alpha", TeamKey::A, 2),
            ("Bravo", TeamKey::B, 3),
            ("Charlie", TeamKey::A, 2),
            ("Delta", TeamKey::B, 3),
        ]
        .map(|(name, team_key, final_group)| Player {
            name,
            team_key,
            final_group,
            steam_id64: "",
        });
        let rows = [
            (20, "Alpha", true, Some(1000)),
            (100, "Alpha", true, Some(2)),
            (100, "Bravo", true, Some(5)),
            (100, "Charlie", true, Some(7)),
            (200, "ALPHA", true, None),
            (220, "Alpha", true, Some(-75)),
            (220, "Bravo", true, Some(0)),
            (220, "Charlie", true, Some(0)),
            (300, "Alpha", false, Some(8)),
            (350, "Alpha", true, Some(999)),
            (360, "Alpha", true, Some(999)),
        ]
        .map(|(tick, name, alive, equipment)| Row {
            tick,
            name: Some(name),
            side: Some(Side::T),
            alive: Some(alive),
            equipment,
            spent: Some(0),
            balance: Some(0),
        });
        let tick_to_round = BTreeMap::from([(100, 1), (200, 2), (220, 2), (300, 3), (350, 0)]);
        let actual = summarize(rows, &tick_to_round, &players, None);
        assert_eq!(
            players.map(|player| actual.average_equipment(player.name)),
            [2, 2, 4, 0]
        );
    }
}
