//! Assemble the legacy all-deaths clip from already collected recording events.

use super::{player_stats::Player, round_score::RoundScores};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    hash::{DefaultHasher, Hash, Hasher},
};

// Legacy recording clips use a fixed 64-tick reference rate.
const DEATH_LEAD_TICKS: i64 = 6 * 64;
const DEATH_TAIL_TICKS: i64 = 3 * 64;
const FINAL_DEATH_TAIL_TICKS: i64 = 44; // int(0.70 * 64)

pub(super) struct Facts<'a> {
    pub(super) deaths: &'a [Value],
    pub(super) freezes: &'a BTreeMap<i64, i64>,
    pub(super) scores: &'a RoundScores,
    /// Completed rounds on the recording scoreboard.
    pub(super) completed_rounds: i64,
    pub(super) last_round_tail_ticks: Option<i64>,
    pub(super) last_decision_round: Option<i64>,
    pub(super) demo_end_tick: i64,
    pub(super) slots: &'a BTreeMap<String, i64>,
}

#[derive(Hash)]
struct Death<'a> {
    round: i64,
    tick: i64,
    attacker: &'a str,
    attacker_steamid: &'a str,
}

impl<'a> Death<'a> {
    fn read(row: &'a Value, target: &str) -> Option<Self> {
        if row["victim_name"].as_str()? != target {
            return None;
        }
        let round = row["round_number"].as_i64()?;
        let tick = row["tick"].as_i64()?;
        let attacker = row["attacker_name"].as_str()?.trim();
        if round <= 0 || tick <= 0 || attacker.is_empty() || attacker == target {
            return None;
        }
        let attacker_steamid = row["attacker_steamid"]
            .as_str()
            .filter(|&id| id != "0")
            .unwrap_or_default();
        Some(Self {
            round,
            tick,
            attacker,
            attacker_steamid,
        })
    }
}

pub(super) fn build(
    facts: Facts<'_>,
    player: &Player<'_>,
    map: &str,
    target_slot: Option<i64>,
    post_match: impl Fn(i64) -> bool,
) -> Option<Value> {
    let mut deaths: Vec<_> = facts
        .deaths
        .iter()
        .filter_map(|row| Death::read(row, player.name))
        .filter(|death| facts.completed_rounds <= 0 || death.round <= facts.completed_rounds)
        .filter(|death| !post_match(death.round))
        .collect();
    deaths.sort_by_key(|death| (death.tick, death.round, death.attacker));
    let first = deaths.first()?;
    let source_ticks: Vec<_> = deaths
        .iter()
        .map(|death| {
            let tail = if death.round == facts.completed_rounds {
                facts.last_round_tail_ticks.unwrap_or(FINAL_DEATH_TAIL_TICKS).clamp(1, DEATH_TAIL_TICKS)
            } else {
                DEATH_TAIL_TICKS
            };
            [(death.tick - DEATH_LEAD_TICKS).max(0), death.tick + tail]
        })
        .collect();
    let start = source_ticks[0][0];
    let end = source_ticks.last().expect("clip has recorded deaths")[1];
    let clip_max = terminal_cap(&facts, &deaths);
    let round_won = facts
        .scores
        .rounds
        .iter()
        .find(|score| score.round_number == first.round)
        .and_then(|score| score.winner)
        .map(|winner| winner == player.team_key);
    let slot_by_name = |name: &str| facts.slots.get(&name.to_lowercase()).copied();
    let mut hash = DefaultHasher::new();
    ("all_deaths", map, player.steam_id64, &deaths).hash(&mut hash);
    let id = format!("c_{:08x}", hash.finish() as u32);
    Some(json!({
        "clip_id":id, "map_name":map, "round":first.round, "category":"compilation",
        "weapon_used":"", "kill_count":0, "start_tick":start,
        "end_tick":clip_max.map_or(end, |cap| end.min(cap)),
        "context_tags":["💀 全部死亡", format!("☠️ {} × {}", player.name, deaths.len())],
        "killer_name":null, "victims":vec![player.name; deaths.len()],
        "killers":deaths.iter().map(|death| death.attacker).collect::<Vec<_>>(),
        "killers_steamid64s":deaths.iter().map(|death| death.attacker_steamid).collect::<Vec<_>>(),
        "killer_steamid64":null, "victim_steamid64s":[],
        "kill_ticks":deaths.iter().map(|death| death.tick).collect::<Vec<_>>(),
        "target_spec_slot":target_slot, "kill_spec_slots":vec![target_slot; deaths.len()],
        "victim_spec_slots":vec![slot_by_name(player.name); deaths.len()], "killer_spec_slot":null,
        "killers_spec_slots":deaths.iter().map(|death| slot_by_name(death.attacker)).collect::<Vec<_>>(),
        "score_own":null, "score_opp":null, "round_won":round_won,
        "clip_min_tick":facts.freezes.get(&first.round), "clip_max_tick":clip_max,
        "death_tick":null, "ai_score":null, "ai_commentary":null,
        "source_ticks":source_ticks,
        "source_rounds":deaths.iter().map(|death| death.round).collect::<Vec<_>>(),
        "source_round_ends":[], "compilation_kind":"all_deaths", "fixed_segment_pacing":false,
        "freeze_to_death_round_filter":null, "freeze_to_death_round_windows":null,
        "flash_assisters":[], "kill_weapons":[], "kill_headshots":[], "kill_tag_lists":[], "shots_to_kill":[],
    }))
}

fn terminal_cap(facts: &Facts<'_>, deaths: &[Death<'_>]) -> Option<i64> {
    let terminal = facts
        .last_decision_round
        .or_else(|| facts.freezes.keys().next_back().copied())
        .unwrap_or_default()
        .max(facts.completed_rounds);
    if terminal <= 0 {
        return None;
    }
    let latest_in_round = |round| {
        deaths
            .iter()
            .rev()
            .find(|death| death.round == round)
            .map(|death| death.tick)
    };
    let tick = latest_in_round(terminal).or_else(|| {
        let latest_round = deaths.iter().map(|death| death.round).max()?;
        (latest_round > terminal)
            .then(|| latest_in_round(latest_round))
            .flatten()
    })?;
    Some((tick + 1).max(facts.demo_end_tick))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::match_timeline::round_score::{self, Side, TeamKey};

    #[test]
    fn retains_legacy_death_order_and_terminal_bounds() {
        let player = Player {
            name: "Target",
            steam_id64: "5",
            final_group: 2,
            team_key: TeamKey::A,
        };
        let deaths = [
            (2, 1450, "Gamma", "3", "Target"),
            (2, 1450, "Alpha", "2", "Target"),
            (2, 1450, "Alpha", "4", "Target"),
            (1, 95, "Bravo", "0", "Target"),
            (1, 96, "Target", "5", "Target"),
            (1, 97, "", "5", "Target"),
            (0, 95, "Bravo", "9", "Target"),
            (1, 0, "Bravo", "9", "Target"),
            (1, 30, "Bravo", "9", "Another player"),
            (3, 1510, "Alpha", "2", "Target"),
        ]
        .map(|(round, tick, attacker, steamid, victim)| {
            json!({"round_number":round,"tick":tick,"attacker_name":attacker,
                "attacker_steamid":steamid,"victim_name":victim})
        });
        let freezes = BTreeMap::from([(1, 100), (2, 1000)]);
        let scores = round_score::build(
            Some([2, 1]),
            &BTreeMap::new(),
            &[(1, Some(Side::T)), (2, Some(Side::Ct))],
        );
        let slots = BTreeMap::from([("target".to_owned(), 5), ("alpha".to_owned(), 2)]);
        let clip = build(
            Facts {
                deaths: &deaths,
                freezes: &freezes,
                scores: &scores,
                completed_rounds: 2,
                last_round_tail_ticks: None,
                last_decision_round: Some(2),
                demo_end_tick: 1470,
                slots: &slots,
            },
            &player,
            "de_test",
            Some(6),
            |_| false,
        )
        .unwrap();
        assert_eq!(clip["round"], 1);
        assert_eq!(clip["round_won"], true);
        assert_eq!(clip["clip_min_tick"], 100);
        assert_eq!(clip["clip_max_tick"], 1470);
        assert_eq!(clip["end_tick"], 1470);
        assert_eq!(
            clip["source_ticks"],
            json!([[0, 287], [1066, 1494], [1066, 1494], [1066, 1494]])
        );
        assert_eq!(clip["source_rounds"], json!([1, 2, 2, 2]));
        assert_eq!(clip["kill_ticks"], json!([95, 1450, 1450, 1450]));
        assert_eq!(clip["killers_steamid64s"], json!(["", "2", "4", "3"]));
        assert_eq!(clip["killers_spec_slots"], json!([null, 2, 2, null]));
        assert_eq!(clip["victim_spec_slots"], json!([5, 5, 5, 5]));
        assert_eq!(clip["kill_spec_slots"], json!([6, 6, 6, 6]));
    }
}
