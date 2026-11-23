//! Assemble an all-kills clip from finalized kill annotations and raw weapon fires.

use super::{player_stats::Player, round_score::RoundScores};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    hash::{DefaultHasher, Hash, Hasher},
};

const KILL_LEAD_TICKS: i64 = 5 * 64;
const KILL_TAIL_TICKS: i64 = 3 * 64;
const FINAL_KILL_TAIL_TICKS: i64 = 44; // int(0.70 * 64)
const SHOT_WINDOW_TICKS: i64 = 2 * 64;

/// A kill after the recording-round correction, terminal knife split and action tagging.
#[derive(Hash)]
pub(super) struct Kill<'a> {
    pub(super) round: i64,
    pub(super) tick: i64,
    pub(super) victim: &'a str,
    pub(super) victim_steamid: &'a str,
    pub(super) weapon: &'a str,
    pub(super) headshot: bool,
    pub(super) tags: &'a [String],
}

/// A raw weapon_fire for the target. Weapon keys use the legacy normalization.
pub(super) struct Fire<'a> {
    pub(super) tick: i64,
    pub(super) weapon: &'a str,
}

pub(super) struct Facts<'a> {
    pub(super) kills: &'a [Kill<'a>],
    /// The target's complete post-warmup weapon_fire stream in tick order.
    pub(super) fire: &'a [Fire<'a>],
    pub(super) freezes: &'a BTreeMap<i64, i64>,
    pub(super) scores: &'a RoundScores,
    pub(super) completed_rounds: i64,
    pub(super) last_round_tail_ticks: Option<i64>,
    pub(super) last_decision_round: Option<i64>,
    pub(super) demo_end_tick: i64,
    pub(super) slots: &'a BTreeMap<String, i64>,
}

pub(super) fn build(
    facts: Facts<'_>,
    player: &Player<'_>,
    map: &str,
    target_slot: Option<i64>,
    post_match: impl Fn(i64) -> bool,
) -> Option<Value> {
    debug_assert!(facts
        .fire
        .windows(2)
        .all(|pair| pair[0].tick <= pair[1].tick));
    let mut kills: Vec<_> = facts
        .kills
        .iter()
        .filter(|kill| kill.tick > 0 && !kill.victim.is_empty())
        .filter(|kill| facts.completed_rounds <= 0 || kill.round <= facts.completed_rounds)
        .filter(|kill| !post_match(kill.round))
        .collect();
    kills.sort_by_key(|kill| (kill.tick, kill.round, kill.victim));
    let first = kills.first()?;
    let source_ticks: Vec<_> = kills
        .iter()
        .map(|kill| {
            let tail = if kill.round == facts.completed_rounds {
                facts.last_round_tail_ticks.unwrap_or(FINAL_KILL_TAIL_TICKS).clamp(1, KILL_TAIL_TICKS)
            } else {
                KILL_TAIL_TICKS
            };
            [(kill.tick - KILL_LEAD_TICKS).max(0), kill.tick + tail]
        })
        .collect();
    let end = source_ticks.last().expect("clip has recorded kills")[1];
    let terminal_round = facts
        .last_decision_round
        .or_else(|| facts.freezes.keys().next_back().copied())
        .unwrap_or_default()
        .max(facts.completed_rounds);
    let clip_max = terminal_clip_cap(
        kills.iter().map(|kill| (kill.round, kill.tick)),
        terminal_round,
        facts.demo_end_tick,
    );
    let round_won = facts
        .scores
        .rounds
        .iter()
        .find(|score| score.round_number == first.round)
        .and_then(|score| score.winner)
        .map(|winner| winner == player.team_key);
    let slot_by_name = |name: &str| facts.slots.get(&name.to_lowercase()).copied();
    let mut hash = DefaultHasher::new();
    ("all_kills", map, player.steam_id64, &kills).hash(&mut hash);
    let id = format!("c_{:08x}", hash.finish() as u32);
    Some(json!({
        "clip_id":id, "map_name":map, "round":first.round, "category":"compilation",
        "weapon_used":"", "kill_count":kills.len(), "start_tick":source_ticks[0][0],
        "end_tick":clip_max.map_or(end, |cap| end.min(cap)),
        "context_tags":["🎬 全部击杀", format!("🎯 {} × {}", player.name, kills.len())],
        "killer_name":null, "killers":vec![player.name; kills.len()], "killers_steamid64s":[],
        "victims":kills.iter().map(|kill| kill.victim).collect::<Vec<_>>(),
        "victim_steamid64s":kills.iter().map(|kill| kill.victim_steamid).collect::<Vec<_>>(),
        "killer_steamid64":null,
        "kill_ticks":kills.iter().map(|kill| kill.tick).collect::<Vec<_>>(),
        "target_spec_slot":target_slot, "kill_spec_slots":vec![target_slot; kills.len()],
        "victim_spec_slots":kills.iter().map(|kill| slot_by_name(kill.victim)).collect::<Vec<_>>(),
        "killer_spec_slot":null, "killers_spec_slots":vec![slot_by_name(player.name); kills.len()],
        "score_own":null, "score_opp":null, "round_won":round_won,
        "clip_min_tick":facts.freezes.get(&first.round), "clip_max_tick":clip_max,
        "death_tick":null, "ai_score":null, "ai_commentary":null,
        "source_ticks":source_ticks,
        "source_rounds":kills.iter().map(|kill| kill.round).collect::<Vec<_>>(),
        "source_round_ends":[], "compilation_kind":"all_kills", "fixed_segment_pacing":false,
        "freeze_to_death_round_filter":null, "freeze_to_death_round_windows":null, "flash_assisters":[],
        "kill_weapons":kills.iter().map(|kill| kill.weapon).collect::<Vec<_>>(),
        "kill_headshots":kills.iter().map(|kill| kill.headshot).collect::<Vec<_>>(),
        "kill_tag_lists":kills.iter().map(|kill| kill.tags).collect::<Vec<_>>(),
        "shots_to_kill":kills.iter().map(|kill| shots_before(facts.fire, kill)).collect::<Vec<_>>(),
    }))
}

fn shots_before(fire: &[Fire<'_>], kill: &Kill<'_>) -> usize {
    let start = fire.partition_point(|shot| shot.tick < kill.tick - SHOT_WINDOW_TICKS);
    let end = fire.partition_point(|shot| shot.tick <= kill.tick);
    fire[start..end]
        .iter()
        .filter(|shot| shot.weapon == kill.weapon)
        .count()
}

/// The terminal round determines the EOF cap, with the latest later source
/// round used only when the compilation contains no terminal-round event.
pub(super) fn terminal_clip_cap(
    round_ticks: impl IntoIterator<Item = (i64, i64)>,
    terminal_round: i64,
    demo_end_tick: i64,
) -> Option<i64> {
    if terminal_round <= 0 {
        return None;
    }
    let mut terminal_tick = None::<i64>;
    let mut later = None::<(i64, i64)>;
    for (round, tick) in round_ticks {
        if round == terminal_round {
            terminal_tick = terminal_tick.max(Some(tick));
        } else if round > terminal_round {
            later = later.max(Some((round, tick)));
        }
    }
    let tick = terminal_tick.or_else(|| later.map(|(_, tick)| tick))?;
    Some((tick + 1).max(demo_end_tick))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::match_timeline::round_score::{self, Side, TeamKey};

    #[test]
    fn keeps_legacy_shot_window_metadata_order_and_recording_score_cap() {
        let player = Player {
            name: "Target",
            steam_id64: "5",
            final_group: 2,
            team_key: TeamKey::A,
        };
        let headshot = ["爆头".to_owned()];
        let smoke = ["🌫️ 混烟".to_owned()];
        let kills = [
            (2, 1500, "Gamma", "3", "ak47", false, [].as_slice()),
            (2, 1500, "Alpha", "2", "ak47", true, headshot.as_slice()),
            (2, 1500, "Alpha", "4", "ak47", false, smoke.as_slice()),
            (1, 100, "Bravo", "0", "hegrenade", false, [].as_slice()),
            (2, 0, "Skipped", "5", "ak47", false, [].as_slice()),
            (2, 1500, "", "6", "ak47", false, [].as_slice()),
            (3, 1510, "Alpha", "2", "ak47", false, [].as_slice()),
        ]
        .map(
            |(round, tick, victim, victim_steamid, weapon, headshot, tags)| Kill {
                round,
                tick,
                victim,
                victim_steamid,
                weapon,
                headshot,
                tags,
            },
        );
        let fire = [
            (50, "hegrenade"),
            (100, "hegrenade"),
            (1371, "ak47"),
            (1372, "ak47"),
            (1499, "m4a1"),
            (1500, "ak47"),
            (1501, "ak47"),
        ]
        .map(|(tick, weapon)| Fire { tick, weapon });
        let freezes = BTreeMap::from([(1, 100), (2, 1000)]);
        let scores = round_score::build(
            Some([2, 1]),
            &BTreeMap::new(),
            &[(1, Some(Side::T)), (2, Some(Side::Ct))],
        );
        let slots = BTreeMap::from([("target".to_owned(), 5), ("alpha".to_owned(), 2)]);
        let clip = build(
            Facts {
                kills: &kills,
                fire: &fire,
                freezes: &freezes,
                scores: &scores,
                completed_rounds: 2,
                last_round_tail_ticks: None,
                last_decision_round: Some(2),
                demo_end_tick: 1520,
                slots: &slots,
            },
            &player,
            "de_test",
            Some(6),
            |_| false,
        )
        .unwrap();
        assert_eq!(
            clip["source_ticks"],
            json!([[0, 292], [1180, 1544], [1180, 1544], [1180, 1544]])
        );
        assert_eq!(clip["clip_max_tick"], 1520);
        assert_eq!(clip["end_tick"], 1520);
        assert_eq!(clip["source_rounds"], json!([1, 2, 2, 2]));
        assert_eq!(clip["victim_steamid64s"], json!(["0", "2", "4", "3"]));
        assert_eq!(
            clip["kill_tag_lists"],
            json!([[], ["爆头"], ["🌫️ 混烟"], []])
        );
        assert_eq!(clip["kill_headshots"], json!([false, true, false, false]));
        assert_eq!(clip["shots_to_kill"], json!([2, 2, 2, 2]));
        assert_eq!(clip["victim_spec_slots"], json!([null, 2, 2, null]));
        assert_eq!(clip["killers_spec_slots"], json!([5, 5, 5, 5]));
        assert_eq!(clip["kill_spec_slots"], json!([6, 6, 6, 6]));
    }
}
