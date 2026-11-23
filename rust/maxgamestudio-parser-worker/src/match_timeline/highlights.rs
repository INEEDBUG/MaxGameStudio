//! Ordinary multi-kill highlight clips from complete recording observations.

use super::kill_compilation::Kill;
use serde_json::{json, Value};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, HashSet},
    hash::{DefaultHasher, Hash, Hasher},
};

const RATE: i64 = 64;
const LEAD: i64 = 5 * RATE;
const TAIL: i64 = 3 * RATE;

pub(super) struct KillFacts<'a> {
    /// Finalized recording kill, after the freeze correction and terminal knife split.
    pub(super) kill: &'a Kill<'a>,
    pub(super) weapon_label: &'a str,
    pub(super) shots_to_kill: i64,
    /// Raw assisted-flash name, even when the kill action tag fails its quality gate.
    pub(super) flash_assister: Option<&'a str>,
    pub(super) noscope: bool,
    pub(super) encounter: Option<Encounter>,
    pub(super) victim_awp_facing: bool,
    pub(super) victim_distance_2d: Option<f64>,
    pub(super) victim_yaw_delta: Option<f64>,
    pub(super) vertical_delta_16: Option<f64>,
    pub(super) jumped: bool,
    /// Latest preceding teammate hit on this victim, including hits on the kill tick.
    pub(super) teammate_hurt_tick: Option<i64>,
}

pub(super) struct Encounter {
    /// Legacy pre-kill snapshot count without the target.
    pub(super) mates: usize,
    pub(super) enemies: usize,
    /// Nearby enemy count only when the living target has an isolated teammate formation.
    pub(super) isolated_enemy_count: Option<usize>,
}

pub(super) struct Hurt<'a> {
    pub(super) tick: i64,
    pub(super) damage: i64,
    pub(super) weapon: &'a str,
}

/// Required evidence for the ordinary highlight branch. An absent field describes
/// an absent observation in the recorded match after applying the legacy fallbacks.
pub(super) struct RoundFacts<'a> {
    pub(super) number: i64,
    pub(super) score: Option<[i64; 2]>,
    pub(super) won: Option<bool>,
    pub(super) first_death_tick: Option<i64>,
    pub(super) target_death_tick: Option<i64>,
    pub(super) freeze_end_tick: Option<i64>,
    pub(super) next_freeze_end_tick: Option<i64>,
    pub(super) round_end_tick: Option<i64>,
    /// Analyzer's cap map, which can fall back to freeze ticks when the map has no decisions.
    pub(super) cap_round_end_tick: Option<i64>,
    pub(super) bomb_explode_tick: Option<i64>,
    pub(super) initial_health: Option<i64>,
    pub(super) final_health: Option<i64>,
    /// The target's and opponents' equipment at freeze end, in that order.
    pub(super) equipment: Option<[i64; 2]>,
    pub(super) previous_killers: &'a [&'a str],
    pub(super) teammate_kills: usize,
    pub(super) target_hurt: &'a [Hurt<'a>],
    pub(super) defuse_window: Option<[i64; 2]>,
    /// This round's legacy highlight also overlays a successful defuse annotation.
    pub(super) merged_defuse: bool,
    pub(super) post_match: bool,
}

pub(super) struct Recording<'a> {
    pub(super) player_steamid: &'a str,
    pub(super) map_name: &'a str,
    pub(super) target_slot: Option<i64>,
    pub(super) slots: &'a BTreeMap<String, i64>,
    pub(super) terminal_round: Option<i64>,
    pub(super) demo_end_tick: i64,
    pub(super) mid_round_buffer_ticks: Option<i64>,
}

pub(super) fn build(
    round: &RoundFacts<'_>,
    kills: &[KillFacts<'_>],
    recording: &Recording<'_>,
) -> Option<Value> {
    if kills.len() < 2 || round.post_match || round.merged_defuse {
        return None;
    }
    debug_assert!(kills.iter().all(|kill| kill.kill.round == round.number));
    let mut kills: Vec<_> = kills.iter().collect();
    kills.sort_by_key(|kill| kill.kill.tick);
    let first = kills[0].kill.tick;
    let last = kills[kills.len() - 1].kill.tick;
    let mut tags = highlight_tags(round, &kills, first, last);
    if round.won == Some(false) {
        tags.retain(|tag| !win_only(tag));
    }
    if kills.len() == 2 && tags.iter().all(|tag| boring_double_tag(tag)) {
        return None;
    }

    let mut end = last.saturating_add(TAIL);
    if round.won == Some(false) && tags.iter().any(|tag| nice_try(tag)) {
        if let Some(death) = round.target_death_tick.filter(|&tick| tick > last) {
            end = end.max(death.saturating_add(TAIL));
        }
    }
    let cap = clip_cap(round, recording, last);
    let [own, opposing] = round.score.map_or([None, None], |score| score.map(Some));
    let mut seen = HashSet::new();
    let flash_assisters: Vec<_> = kills
        .iter()
        .filter_map(|kill| kill.flash_assister)
        .filter(|name| !name.is_empty() && seen.insert(*name))
        .collect();
    let mut hash = DefaultHasher::new();
    (
        "highlight",
        recording.map_name,
        recording.player_steamid,
        round.number,
    )
        .hash(&mut hash);
    for kill in &kills {
        kill.kill.hash(&mut hash);
    }
    let clip_id = format!("c_{:08x}", hash.finish() as u32);
    Some(json!({
        "clip_id":clip_id, "map_name":recording.map_name, "round":round.number,
        "category":"highlight", "weapon_used":weapon_label(&kills), "kill_count":kills.len(),
        "start_tick":first.saturating_sub(LEAD).max(0),
        "end_tick":cap.map_or(end, |cap| end.min(cap)),
        "context_tags":deduplicate_tags(tags),
        "killer_name":null, "killers":[], "killers_steamid64s":[], "killer_steamid64":null,
        "victims":kills.iter().map(|kill| kill.kill.victim).collect::<Vec<_>>(),
        "victim_steamid64s":kills.iter().map(|kill| kill.kill.victim_steamid).collect::<Vec<_>>(),
        "kill_ticks":kills.iter().map(|kill| kill.kill.tick).collect::<Vec<_>>(),
        "target_spec_slot":recording.target_slot, "kill_spec_slots":vec![recording.target_slot; kills.len()],
        "victim_spec_slots":kills.iter().map(|kill| recording.slots.get(&kill.kill.victim.to_lowercase())).collect::<Vec<_>>(),
        "killer_spec_slot":null, "killers_spec_slots":[],
        "score_own":own, "score_opp":opposing, "round_won":round.won,
        "clip_min_tick":round.freeze_end_tick, "clip_max_tick":cap, "death_tick":round.target_death_tick,
        "ai_score":null, "ai_commentary":null, "source_ticks":[], "source_rounds":[],
        "source_round_ends":[], "compilation_kind":null, "fixed_segment_pacing":false,
        "freeze_to_death_round_filter":null, "freeze_to_death_round_windows":null,
        "flash_assisters":flash_assisters, "kill_weapons":[], "kill_headshots":[],
        "kill_tag_lists":[], "shots_to_kill":[],
    }))
}

fn highlight_tags(
    round: &RoundFacts<'_>,
    kills: &[&KillFacts<'_>],
    first: i64,
    last: i64,
) -> Vec<String> {
    let n = kills.len();
    let count = match n {
        2 => "双杀",
        3 => "三杀",
        4 => "四杀",
        _ => "五杀 (ACE)",
    };
    let mut tags = vec![count.to_owned()];
    if kills.iter().any(|kill| is_knife(kill.kill.weapon)) {
        tags.push("🔪 刀杀".to_owned());
    }
    if last.saturating_sub(first) <= if n == 2 { 3 * RATE } else { 10 * RATE } {
        tags.push("爆发刷屏".to_owned());
    }
    if kills.iter().all(|kill| kill.kill.headshot) {
        tags.push("枪枪爆头".to_owned());
    }
    if round.first_death_tick == Some(first) {
        tags.push("⚔️ 首杀".to_owned());
    }

    let [our_equipment, their_equipment] = round.equipment.unwrap_or_default();
    let pistol_round = matches!(round.number, 1 | 13);
    let eco = !pistol_round && our_equipment <= 8_000 && their_equipment >= 15_000;
    if pistol_round {
        tags.push("🔫 手枪局专家".to_owned());
    } else if eco {
        tags.push("💸 ECO翻盘".to_owned());
    } else if our_equipment >= 15_000 && their_equipment <= 8_000 {
        tags.push("🔫 ECO特种兵".to_owned());
    }

    let [own, opposing] = round.score.unwrap_or_default();
    let enemy_match_point = opposing >= 12 && opposing % 3 == 0 && own < opposing;
    let own_match_point = own >= 12 && own % 3 == 0 && own > opposing;
    if enemy_match_point {
        if opposing - own == 1 {
            tags.extend(["🛡️ 赛点救世主", "命悬一线"].map(str::to_owned));
        } else {
            tags.extend(["📈 绝地追分", "拒绝下班"].map(str::to_owned));
        }
    } else if own_match_point {
        if own - opposing == 1 {
            tags.extend(["🗡️ 赛点终结者", "一锤定音"].map(str::to_owned));
        }
    } else if own == opposing && own >= 12 {
        tags.extend(["⚔️ 加时生死战", "大心脏"].map(str::to_owned));
    }
    if own >= 8 && opposing <= 2 {
        tags.extend(["🔥 顺风局战神", "无情碾压"].map(str::to_owned));
    }
    if own == opposing && own >= 10 {
        tags.push("⛰️ 天王山之战".to_owned());
    }
    if round.final_health.is_some_and(|hp| (1..=15).contains(&hp)) {
        tags.push("❤️ 极限锁血战神".to_owned());
    }

    let failed_duel = add_encounter_tags(&mut tags, kills, round.won);
    add_weapon_tags(&mut tags, kills);

    for kill in kills {
        tags.extend(kill.kill.tags.iter().filter(|tag| *tag != "爆头").cloned());
    }
    if kills.iter().any(|kill| {
        is_knife(kill.kill.weapon) && kill.victim_yaw_delta.is_some_and(|yaw| yaw.abs() < 45.0)
    }) {
        tags.push("🔙 背刺".to_owned());
    }
    if kills.iter().any(|kill| {
        round
            .round_end_tick
            .is_some_and(|end| within(kill.kill.tick, end, 5 * RATE))
            || round
                .bomb_explode_tick
                .is_some_and(|end| within(kill.kill.tick, end, 3 * RATE))
    }) {
        tags.push("🔔 极限操作".to_owned());
    }
    if kills
        .iter()
        .any(|kill| round.previous_killers.contains(&kill.kill.victim.trim()))
    {
        tags.push("🧾 上回合的债".to_owned());
    }
    if kills.iter().any(|kill| {
        kill.teammate_hurt_tick
            .is_some_and(|hit| within(hit, kill.kill.tick, 160))
    }) {
        tags.push("⚰️ 补枪".to_owned());
    }
    if n >= 5 && round.teammate_kills == 0 {
        tags.push("🧹 清盘".to_owned());
    }
    if (1..=3_000).contains(&our_equipment) && their_equipment >= 12_000 {
        tags.push("👢 光脚干皮鞋".to_owned());
    }
    if round
        .initial_health
        .is_some_and(|hp| (1..=20).contains(&hp))
    {
        tags.push("❤️‍🩹 残血绝地反击".to_owned());
    }
    if ironshirt(round, last) {
        tags.push("🪨 挨揍王".to_owned());
    }
    if round.defuse_window.is_some_and(|[start, end]| {
        kills
            .iter()
            .any(|kill| (start..=end).contains(&kill.kill.tick))
    }) {
        tags.push("💣 拆包开光".to_owned());
    }

    if round.won == Some(false) {
        if let Some(enemies) = failed_duel {
            tags.push(if enemies == 2 {
                "😤 1v2 饮恨".to_owned()
            } else {
                format!("💀 1v{enemies} 封神未遂")
            });
        }
        if eco {
            tags.push("💸 ECO反击".to_owned());
        }
        if enemy_match_point {
            tags.push(
                if opposing - own == 1 {
                    "🛡️ 赛点失守"
                } else {
                    "📉 绝地追分未果"
                }
                .to_owned(),
            );
        } else if own == opposing && own >= 10 {
            tags.push("⛰️ 天王山饮恨".to_owned());
        }
    }
    tags
}

fn add_encounter_tags(
    tags: &mut Vec<String>,
    kills: &[&KillFacts<'_>],
    won: Option<bool>,
) -> Option<usize> {
    let mut epic = 0;
    let mut near_win = 0;
    let mut duo = 0;
    let mut three_vs_five = false;
    let mut duel = false;
    let mut isolated_kills = 0;
    let mut nearby = 0;
    for (index, kill) in kills.iter().enumerate() {
        let Some(encounter) = &kill.encounter else {
            continue;
        };
        let remaining = kills.len() - index;
        match (encounter.mates, encounter.enemies) {
            (0, 1) => duel = true,
            (0, enemies) if enemies >= 2 => {
                if remaining >= enemies {
                    epic = epic.max(enemies);
                }
                if remaining >= enemies - 1 {
                    near_win = near_win.max(enemies);
                }
            }
            (1, enemies) if enemies >= 4 => duo = duo.max(enemies),
            (2, 5) => three_vs_five = true,
            _ => {}
        }
        if let Some(isolated) = encounter.isolated_enemy_count {
            isolated_kills += 1;
            nearby = nearby.max(isolated);
        }
    }
    if epic >= 2 {
        tags.push(format!("🔥 1v{epic} 史诗残局"));
    }
    if duo >= 4 {
        tags.push(format!("🔥 2v{duo} 兄弟齐心"));
    }
    if three_vs_five {
        tags.push("🔥 3v5 绝地反击".to_owned());
    }
    if duel && won == Some(true) {
        tags.push("🐂 1v1 斗牛".to_owned());
    }
    if isolated_kills >= 2 && nearby >= 3 {
        tags.push("🪖 一人成军".to_owned());
    }
    (near_win >= 2).then_some(near_win)
}

fn add_weapon_tags(tags: &mut Vec<String>, kills: &[&KillFacts<'_>]) {
    let long_weapon = |weapon| matches!(weapon, "awp" | "ssg08" | "deagle" | "revolver");
    let mut previous = None;
    if kills
        .iter()
        .filter(|kill| long_weapon(kill.kill.weapon))
        .any(|kill| {
            previous
                .replace(kill.kill.tick)
                .is_some_and(|tick| within(tick, kill.kill.tick, 2))
        })
    {
        tags.push("🍡 一石二鸟".to_owned());
    }
    let regular: Vec<_> = kills
        .iter()
        .filter(|kill| !matches!(kill.kill.weapon, "awp" | "ssg08" | "scar20" | "g3sg1"))
        .collect();
    let precise = regular.iter().any(|kill| {
        kill.kill.headshot
            && kill.shots_to_kill <= 3
            && matches!(
                kill.kill.weapon,
                "ak47"
                    | "m4a1"
                    | "m4a1_silencer"
                    | "sg556"
                    | "aug"
                    | "famas"
                    | "galilar"
                    | "deagle"
                    | "revolver"
            )
    });
    if precise || (regular.len() >= 2 && regular.iter().all(|kill| kill.kill.headshot)) {
        tags.push("💥 颗秒".to_owned());
        if kills.iter().any(|kill| kill.victim_awp_facing) {
            tags.push("🔪 手撕大狙".to_owned());
        }
    }
    if kills.iter().any(|kill| {
        long_weapon(kill.kill.weapon)
            && kill
                .victim_distance_2d
                .is_some_and(|distance| distance > 1500.0)
    }) {
        tags.push("🔭 百步穿杨".to_owned());
    }
    if kills.iter().any(|kill| {
        matches!(kill.kill.weapon, "awp" | "ssg08")
            && (kill.noscope || kill.kill.tags.iter().any(|tag| tag == "盲狙"))
            && kill
                .vertical_delta_16
                .is_some_and(|delta| delta.abs() > 15.0)
    }) {
        tags.extend(["✈️ 飞天盲狙", "冷神附体"].map(str::to_owned));
    }
    if kills.iter().any(|kill| kill.jumped) {
        tags.push("🪂 跳杀".to_owned());
    }
}

fn ironshirt(round: &RoundFacts<'_>, last: i64) -> bool {
    if round.target_death_tick.is_some_and(|tick| tick <= last) {
        return false;
    }
    let mut count = 0;
    let mut damage = 0;
    for hit in round.target_hurt {
        if hit.tick <= last
            && !matches!(
                hit.weapon,
                "hegrenade"
                    | "inferno"
                    | "molotov"
                    | "incgrenade"
                    | "flashbang"
                    | "c4"
                    | "planted_c4"
                    | "world"
            )
        {
            count += 1;
            damage += hit.damage;
        }
    }
    count >= 4 && damage >= 95
}

fn within(from: i64, to: i64, max: i64) -> bool {
    (0..=max).contains(&to.saturating_sub(from))
}

fn is_knife(weapon: &str) -> bool {
    [
        "knife",
        "bayonet",
        "karambit",
        "butterfly",
        "stiletto",
        "falchion",
        "bowie",
        "huntsman",
        "daggers",
        "shadow",
        "navaja",
        "ursus",
        "nomad",
        "skeleton",
        "survival",
        "paracord",
        "canis",
        "cord",
        "widowmaker",
        "gypsy",
        "outdoor",
        "css",
        "kukri",
    ]
    .iter()
    .any(|alias| weapon.contains(alias))
}

fn win_only(tag: &str) -> bool {
    matches!(
        tag,
        "💸 ECO翻盘"
            | "🛡️ 赛点救世主"
            | "命悬一线"
            | "📈 绝地追分"
            | "拒绝下班"
            | "🗡️ 赛点终结者"
            | "一锤定音"
            | "⚔️ 加时生死战"
            | "大心脏"
            | "🔥 3v5 绝地反击"
    ) || tag.starts_with("🔥 1v")
        || tag.starts_with("🔥 2v")
}

fn boring_double_tag(tag: &str) -> bool {
    matches!(
        tag,
        "双杀" | "爆头" | "枪枪爆头" | "⚔️ 破局首杀" | "🔥 顺风局战神" | "无情碾压"
    )
}

fn nice_try(tag: &str) -> bool {
    matches!(
        tag,
        "😤 1v2 饮恨" | "💸 ECO反击" | "🛡️ 赛点失守" | "📉 绝地追分未果" | "⛰️ 天王山饮恨"
    ) || tag.starts_with("💀 1v")
}

fn deduplicate_tags(mut tags: Vec<String>) -> Vec<String> {
    let coverage = [
        ("🔫 手枪哥", "🔫 手枪局专家"),
        ("🔫 手枪哥", "🔫 ECO特种兵"),
        ("爆头", "🔫 手枪哥"),
        ("爆头", "👃 零距离"),
        ("爆头", "NiKo Play"),
        ("爆头", "沙鹰爆头"),
        ("爆头", "💥 颗秒"),
        ("爆头", "枪枪爆头"),
        ("🫵 贴脸超度", "👃 零距离"),
        ("🧱 穿墙杀", "🎯 超远穿墙"),
        ("🧱 穿墙杀", "🔀 连穿"),
        ("🙈 盲狙", "✈️ 飞天盲狙"),
        ("🔙 偷背身", "🔙 背刺"),
    ];
    let suppressed: HashSet<_> = coverage
        .into_iter()
        .filter_map(|(tag, by)| tags.iter().any(|value| value == by).then_some(tag))
        .collect();
    let mut seen = HashSet::new();
    tags.retain(|tag| !suppressed.contains(tag.as_str()) && seen.insert(tag.clone()));
    tags
}

fn weapon_label(kills: &[&KillFacts<'_>]) -> String {
    let mut weapons = BTreeMap::new();
    for (index, kill) in kills.iter().enumerate() {
        let key = kill.kill.weapon.trim();
        if !key.is_empty() {
            let entry = weapons.entry(key).or_insert((0, index, kill.weapon_label));
            entry.0 += 1;
        }
    }
    let mut ranked: Vec<_> = weapons.into_values().collect();
    ranked.sort_by_key(|&(count, first, _)| (Reverse(count), first));
    ranked
        .into_iter()
        .map(|(_, _, label)| label)
        .collect::<Vec<_>>()
        .join(" / ")
}

fn clip_cap(round: &RoundFacts<'_>, recording: &Recording<'_>, last: i64) -> Option<i64> {
    if recording.terminal_round == Some(round.number) {
        return (last != 0).then_some(last.saturating_add(1).max(recording.demo_end_tick));
    }
    let round_end = round.cap_round_end_tick?;
    let next_freeze = round.next_freeze_end_tick.filter(|&tick| tick > round_end);
    let limit = round_end.saturating_add(recording.mid_round_buffer_ticks.unwrap_or(TAIL));
    let cap = if last > round_end {
        next_freeze.unwrap_or_else(|| limit.max(last.saturating_add(TAIL)))
    } else {
        limit
    };
    Some(next_freeze.map_or(cap, |next| cap.min(next)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round() -> RoundFacts<'static> {
        RoundFacts {
            number: 4,
            score: Some([2, 1]),
            won: Some(false),
            first_death_tick: None,
            target_death_tick: None,
            freeze_end_tick: Some(500),
            next_freeze_end_tick: Some(1530),
            round_end_tick: None,
            cap_round_end_tick: Some(1500),
            bomb_explode_tick: None,
            initial_health: None,
            final_health: None,
            equipment: None,
            previous_killers: &[],
            teammate_kills: 0,
            target_hurt: &[],
            defuse_window: None,
            merged_defuse: false,
            post_match: false,
        }
    }

    fn detail<'a>(kill: &'a Kill<'a>) -> KillFacts<'a> {
        KillFacts {
            kill,
            weapon_label: "AK-47",
            shots_to_kill: 4,
            flash_assister: None,
            noscope: false,
            encounter: None,
            victim_awp_facing: false,
            victim_distance_2d: None,
            victim_yaw_delta: None,
            vertical_delta_16: None,
            jumped: false,
            teammate_hurt_tick: None,
        }
    }

    #[test]
    fn a_double_needs_a_real_tag_and_a_lost_clutch_extends_to_the_capped_death_tail() {
        let kills = [
            Kill {
                round: 4,
                tick: 1270,
                victim: "Second",
                victim_steamid: "32",
                weapon: "ak47",
                headshot: false,
                tags: &[],
            },
            Kill {
                round: 4,
                tick: 1000,
                victim: "First",
                victim_steamid: "31",
                weapon: "ak47",
                headshot: false,
                tags: &[],
            },
        ];
        let mut facts = kills.each_ref().map(detail);
        let slots = BTreeMap::from([("second".to_owned(), 2)]);
        let recording = Recording {
            player_steamid: "22",
            map_name: "de_example",
            target_slot: Some(5),
            slots: &slots,
            terminal_round: Some(9),
            demo_end_tick: 10000,
            mid_round_buffer_ticks: None,
        };
        let mut round = round();
        assert!(build(&round, &facts, &recording).is_none());
        facts[1].encounter = Some(Encounter {
            mates: 0,
            enemies: 3,
            isolated_enemy_count: None,
        });
        facts[0].flash_assister = Some("Helper");
        round.target_death_tick = Some(1450);
        let clip = build(&round, &facts, &recording).unwrap();
        assert_eq!(clip["context_tags"], json!(["双杀", "💀 1v3 封神未遂"]));
        assert_eq!(clip["kill_ticks"], json!([1000, 1270]));
        assert_eq!(clip["victim_steamid64s"], json!(["31", "32"]));
        assert_eq!(clip["victim_spec_slots"], json!([null, 2]));
        assert_eq!(clip["flash_assisters"], json!(["Helper"]));
        assert_eq!(clip["start_tick"], 680);
        assert_eq!(clip["end_tick"], 1530);
        assert_eq!(clip["clip_max_tick"], 1530);
        assert_eq!(clip["death_tick"], 1450);
        round.merged_defuse = true;
        assert!(build(&round, &facts, &recording).is_none());
    }
}
