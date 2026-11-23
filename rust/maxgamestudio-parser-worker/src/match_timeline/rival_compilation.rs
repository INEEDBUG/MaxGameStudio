//! Assemble the legacy repeated-opponent kill and death compilations.

use super::{
    kill_compilation::{terminal_clip_cap, Kill},
    player_stats::Player,
    round_score::RoundScores,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    hash::{DefaultHasher, Hash, Hasher},
};

const PAIR_THRESHOLD: usize = 8;
const TICK_RATE: i64 = 64;
const TAIL_TICKS: i64 = 3 * TICK_RATE;
const LAST_ROUND_TAIL_TICKS: i64 = 44; // int(0.70 * 64)

pub(super) struct Facts<'a> {
    /// Finalized kills in legacy round-bucket traversal order.
    pub(super) kills: &'a [Kill<'a>],
    pub(super) deaths: &'a [Value],
    pub(super) freezes: &'a BTreeMap<i64, i64>,
    pub(super) scores: &'a RoundScores,
    /// Completed rounds on the recording scoreboard.
    pub(super) completed_rounds: i64,
    pub(super) last_decision_round: Option<i64>,
    pub(super) demo_end_tick: i64,
    pub(super) slots: &'a BTreeMap<String, i64>,
    pub(super) last_round_tail_ticks: Option<i64>,
}

enum Kind {
    RivalKills,
    NemesisDeaths,
}

struct OpponentEvents<'a> {
    name: &'a str,
    anchors: Vec<(i64, i64)>,
}

pub(super) fn build(
    facts: Facts<'_>,
    player: &Player<'_>,
    map: &str,
    target_slot: Option<i64>,
    post_match: impl Fn(i64) -> bool,
) -> Vec<Value> {
    let kills = facts
        .kills
        .iter()
        .map(|kill| (kill.victim.trim(), (kill.round, kill.tick)));
    let deaths = facts.deaths.iter().filter_map(|row| {
        if row["victim_name"].as_str()?.trim() != player.name {
            return None;
        }
        let attacker = row["attacker_name"].as_str()?.trim();
        (attacker != player.name).then(|| {
            (
                attacker,
                (
                    row["round_number"].as_i64().unwrap_or_default(),
                    row["tick"].as_i64().unwrap_or_default(),
                ),
            )
        })
    });
    let groups = [
        (Kind::RivalKills, group_by_opponent(kills)),
        (Kind::NemesisDeaths, group_by_opponent(deaths)),
    ];
    let terminal_round = facts
        .last_decision_round
        .or_else(|| facts.freezes.keys().next_back().copied())
        .unwrap_or_default()
        .max(facts.completed_rounds);
    let mut clips = Vec::new();
    for (kind, opponents) in groups {
        for mut opponent in opponents {
            opponent.anchors.retain(|&(round, _)| {
                (facts.completed_rounds <= 0 || round <= facts.completed_rounds)
                    && !post_match(round)
            });
            if opponent.anchors.len() >= PAIR_THRESHOLD {
                opponent.anchors.sort_unstable();
                clips.push(clip(
                    &facts,
                    player,
                    map,
                    target_slot,
                    &kind,
                    opponent,
                    terminal_round,
                ));
            }
        }
    }
    clips
}

fn group_by_opponent<'a>(
    events: impl Iterator<Item = (&'a str, (i64, i64))>,
) -> Vec<OpponentEvents<'a>> {
    let mut groups = Vec::<OpponentEvents>::new();
    let mut indices = BTreeMap::new();
    for (name, anchor) in events {
        if name.is_empty() {
            continue;
        }
        // Python's grouping uses first encounter order, including groups whose
        // first event will subsequently be filtered out as postmatch.
        let index = *indices.entry(name).or_insert_with(|| {
            let index = groups.len();
            groups.push(OpponentEvents {
                name,
                anchors: Vec::new(),
            });
            index
        });
        groups[index].anchors.push(anchor);
    }
    groups
}

fn clip(
    facts: &Facts<'_>,
    player: &Player<'_>,
    map: &str,
    target_slot: Option<i64>,
    kind: &Kind,
    opponent: OpponentEvents<'_>,
    terminal_round: i64,
) -> Value {
    let count = opponent.anchors.len();
    let name = opponent.name;
    let (compilation_kind, label, counter, lead_seconds, killer, victim, killer_name, kill_count) =
        match kind {
            Kind::RivalKills => (
                "rival_kills",
                "🥩 亲儿子喂饭",
                "👉",
                5,
                player.name,
                name,
                None,
                count,
            ),
            Kind::NemesisDeaths => (
                "nemesis_deaths",
                "☠️ 本命苦主",
                "💀",
                6,
                name,
                player.name,
                Some(name),
                0,
            ),
        };
    let completed_tail = facts
        .last_round_tail_ticks
        .unwrap_or(LAST_ROUND_TAIL_TICKS)
        .clamp(1, TAIL_TICKS);
    let source_ticks: Vec<_> = opponent
        .anchors
        .iter()
        .map(|&(round, tick)| {
            let tail = if facts.completed_rounds > 0 && round == facts.completed_rounds {
                completed_tail
            } else {
                TAIL_TICKS
            };
            [
                tick.saturating_sub(lead_seconds * TICK_RATE).max(0),
                tick.saturating_add(tail),
            ]
        })
        .collect();
    // This builder is only reached for pairs with at least eight retained events.
    let first_round = opponent.anchors[0].0;
    let (last_round, _) = opponent.anchors[count - 1];
    let end = source_ticks[count - 1][1];
    let clip_max =
        terminal_clip_cap(
            opponent.anchors.iter().copied().filter(|&(round, tick)| {
                tick > 0 && (round == terminal_round || round == last_round)
            }),
            terminal_round,
            facts.demo_end_tick,
        );
    let round_won = facts
        .scores
        .rounds
        .iter()
        .find(|score| score.round_number == first_round)
        .and_then(|score| score.winner)
        .map(|winner| winner == player.team_key);
    let slot_by_name = |name: &str| facts.slots.get(&name.to_lowercase()).copied();
    let mut hash = DefaultHasher::new();
    (
        compilation_kind,
        map,
        player.steam_id64,
        name,
        &opponent.anchors,
    )
        .hash(&mut hash);
    let id = format!("c_{:08x}", hash.finish() as u32);
    json!({
        "clip_id":id, "map_name":map, "round":first_round, "category":"compilation",
        "weapon_used":"", "kill_count":kill_count, "start_tick":source_ticks[0][0],
        "end_tick":clip_max.map_or(end, |cap| end.min(cap)),
        "context_tags":[label, format!("{counter} {name} × {count}")],
        "killer_name":killer_name, "victims":vec![victim; count], "killers":vec![killer; count],
        "killers_steamid64s":[], "victim_steamid64s":[], "killer_steamid64":null,
        "kill_ticks":opponent.anchors.iter().map(|&(_,tick)| tick).collect::<Vec<_>>(),
        "target_spec_slot":target_slot, "kill_spec_slots":vec![target_slot; count],
        "victim_spec_slots":vec![slot_by_name(victim); count],
        "killer_spec_slot":killer_name.and_then(slot_by_name),
        "killers_spec_slots":vec![slot_by_name(killer); count],
        "score_own":null, "score_opp":null, "round_won":round_won,
        "clip_min_tick":facts.freezes.get(&first_round), "clip_max_tick":clip_max,
        "death_tick":null, "ai_score":null, "ai_commentary":null,
        "source_ticks":source_ticks,
        "source_rounds":opponent.anchors.iter().map(|&(round,_)| round).collect::<Vec<_>>(),
        "source_round_ends":[], "compilation_kind":compilation_kind, "fixed_segment_pacing":false,
        "freeze_to_death_round_filter":null, "freeze_to_death_round_windows":null, "flash_assisters":[],
        "kill_weapons":[], "kill_headshots":[], "kill_tag_lists":[], "shots_to_kill":[],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::match_timeline::round_score::{self, Side, TeamKey};

    #[test]
    fn pair_threshold_is_after_postmatch_filtering_and_ranges_keep_terminal_tail() {
        let player = Player {
            name: "Target",
            steam_id64: "5",
            final_group: 2,
            team_key: TeamKey::A,
        };
        let kill = |round, tick, victim| Kill {
            round,
            tick,
            victim,
            victim_steamid: "22",
            weapon: "glock",
            headshot: true,
            tags: &[],
        };
        let mut kills: Vec<_> = (100..107).map(|tick| kill(2, tick, "Rival")).collect();
        kills.push(kill(3, 1500, "Rival"));
        kills.push(kill(4, 1510, "Rival"));
        kills.extend((200..207).map(|tick| kill(2, tick, "Short")));
        kills.push(kill(4, 1511, "Short"));
        let deaths: Vec<_> = (1440..1448)
            .rev()
            .map(|tick| {
                json!({
                    "round_number":3, "tick":tick, "attacker_name":"Nemesis",
                    "attacker_steamid":"41", "victim_name":"Target",
                })
            })
            .collect();
        let freezes = BTreeMap::from([(2, 95), (3, 1000)]);
        let slots = BTreeMap::from([("target".to_owned(), 5), ("rival".to_owned(), 2)]);
        let scores = round_score::build(
            Some([2, 1]),
            &BTreeMap::new(),
            &[(2, Some(Side::T)), (3, Some(Side::Ct))],
        );
        let clips = build(
            Facts {
                kills: &kills,
                deaths: &deaths,
                freezes: &freezes,
                scores: &scores,
                completed_rounds: 3,
                last_decision_round: None,
                demo_end_tick: 1520,
                slots: &slots,
                last_round_tail_ticks: None,
            },
            &player,
            "de_test",
            Some(6),
            |_| false,
        );
        assert_eq!(clips.len(), 2);
        let rival = &clips[0];
        assert_eq!(rival["compilation_kind"], "rival_kills");
        assert_eq!(rival["kill_count"], 8);
        assert_eq!(rival["source_rounds"], json!([2, 2, 2, 2, 2, 2, 2, 3]));
        assert_eq!(rival["source_ticks"][0], json!([0, 292]));
        assert_eq!(rival["source_ticks"][7], json!([1180, 1544]));
        assert_eq!(rival["clip_min_tick"], 95);
        assert_eq!(rival["clip_max_tick"], 1520);
        assert_eq!(rival["end_tick"], 1520);
        assert_eq!(rival["victim_spec_slots"], json!(vec![2; 8]));
        assert_eq!(rival["killers_spec_slots"], json!(vec![5; 8]));
        assert_eq!(rival["kill_spec_slots"], json!(vec![6; 8]));
        assert_eq!(rival["victim_steamid64s"], json!([]));
        assert_eq!(rival["kill_weapons"], json!([]));

        let nemesis = &clips[1];
        assert_eq!(nemesis["compilation_kind"], "nemesis_deaths");
        assert_eq!(nemesis["kill_count"], 0);
        assert_eq!(nemesis["killer_name"], "Nemesis");
        assert_eq!(nemesis["killer_steamid64"], Value::Null);
        assert_eq!(nemesis["killer_spec_slot"], Value::Null);
        assert_eq!(nemesis["killers_steamid64s"], json!([]));
        assert_eq!(
            nemesis["kill_ticks"],
            json!([1440, 1441, 1442, 1443, 1444, 1445, 1446, 1447])
        );
        assert_eq!(nemesis["source_ticks"][0], json!([1056, 1484]));
        assert_eq!(nemesis["end_tick"], 1491);
    }
}
