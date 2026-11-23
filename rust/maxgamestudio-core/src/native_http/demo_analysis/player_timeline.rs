//! Project one player's observed match events into the two timeline formats.

use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashSet};

pub(super) struct Input<'a> {
    pub demo_path: &'a str,
    pub map_name: Option<&'a str>,
    pub target_name: &'a str,
    pub target_steamid64: Option<&'a str>,
    pub target_spec_slot: Option<i64>,
    pub initial_side: Option<&'a str>,
    pub tick_rate: f64,
    pub total_rounds: usize,
    pub deaths: &'a [Value],
    pub rounds: &'a [Value],
    pub clips: &'a [Value],
    pub spec_slots: &'a Map<String, Value>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EventKind {
    Kill,
    Death,
    Assist,
}

impl EventKind {
    fn name(self) -> &'static str {
        match self {
            Self::Kill => "kill",
            Self::Death => "death",
            Self::Assist => "assist",
        }
    }
}

#[derive(Clone, Copy)]
struct Player<'a> {
    name: Option<&'a str>,
    steamid: Option<u64>,
}

impl Player<'_> {
    fn same_identity(self, other: Self) -> bool {
        match (self.steamid, other.steamid) {
            (Some(left), Some(right)) => left == right,
            _ => self.name.is_some() && self.name == other.name,
        }
    }

    fn known(self) -> bool {
        self.steamid.is_some() || self.name.is_some()
    }

    fn steam_string(self) -> Option<String> {
        self.steamid.map(|id| id.to_string())
    }
}

struct Event<'a> {
    kind: EventKind,
    id: String,
    tick: i64,
    attacker: Player<'a>,
    victim: Player<'a>,
    assister: Player<'a>,
    attacker_is_target: bool,
    victim_is_target: bool,
    assister_is_target: bool,
    attacker_slot: Option<i64>,
    victim_slot: Option<i64>,
    data: &'a Value,
}

struct Clip<'a> {
    id: Option<&'a str>,
    round: Option<usize>,
    ticks: Option<(i64, i64)>,
    highlight_tags: &'a [Value],
}

impl<'a> Clip<'a> {
    fn read(data: &'a Value) -> Self {
        Self {
            id: text(&data["clip_id"]),
            round: round_number(&data["round"]),
            ticks: integer(&data["start_tick"]).zip(integer(&data["end_tick"])),
            highlight_tags: if data["category"] == "highlight" {
                values(&data["context_tags"])
            } else {
                &[]
            },
        }
    }

    fn relates(&self, round: usize, tick: i64) -> bool {
        self.round == Some(round)
            || self
                .ticks
                .is_some_and(|(start, end)| start <= tick && tick <= end)
    }
}

#[derive(Default)]
struct Totals {
    kills: usize,
    deaths: usize,
    assists: usize,
}

pub(super) fn project(input: Input<'_>) -> Value {
    let target = Player {
        name: Some(input.target_name.trim()).filter(|name| !name.is_empty()),
        steamid: input.target_steamid64.and_then(parse_steamid),
    };
    let rate = Some(input.tick_rate).filter(|rate| rate.is_finite() && *rate > 0.0);
    let mut facts = BTreeMap::new();
    for row in input.rounds {
        if let Some(round) = round_number(&row["round_number"]) {
            facts.entry(round).or_insert(row);
        }
    }
    let freeze = |round| {
        facts
            .get(&round)
            .and_then(|row| nonnegative(&row["freeze_end_tick"]))
    };
    let clips: Vec<_> = input.clips.iter().map(Clip::read).collect();
    let mut by_round: BTreeMap<usize, Vec<Event<'_>>> = BTreeMap::new();
    let mut sequence = 0;
    for row in input.deaths {
        let Some((mut round, tick)) =
            round_number(&row["round_number"]).zip(nonnegative(&row["tick"]))
        else {
            continue;
        };
        while round > 1 && freeze(round).is_some_and(|start| tick < start) {
            round -= 1;
        }
        if round > input.total_rounds {
            continue;
        }
        if let Some(event) = Event::new(row, round, tick, sequence + 1, target, &input) {
            by_round.entry(round).or_default().push(event);
            sequence += 1;
        }
    }

    let mut legacy_rounds = Vec::new();
    let mut killfeed_rounds = Vec::new();
    let mut totals = Totals::default();
    for round in 1..=input.total_rounds {
        let round_facts = facts.get(&round).copied().unwrap_or(&Value::Null);
        let start = freeze(round);
        let end = nonnegative(&round_facts["end_tick"]);
        let side = read_side(&round_facts["side"]);
        let winner = read_side(&round_facts["winner"]);
        let won = winner.zip(side).map(|(winner, side)| winner == side);
        let score_t = nonnegative(&round_facts["score_t"]);
        let score_ct = nonnegative(&round_facts["score_ct"]);
        let score_text = match (score_t, score_ct) {
            (Some(t), Some(ct)) => format!("{t}:{ct}"),
            _ => "--".to_owned(),
        };
        let mut events = by_round.remove(&round).unwrap_or_default();
        events.sort_by(|left, right| {
            left.tick
                .cmp(&right.tick)
                .then_with(|| left.id.cmp(&right.id))
        });
        let first_kill_tick = events
            .iter()
            .find(|event| event.kind == EventKind::Kill)
            .map(|event| event.tick);
        let record_end = end.zip(rate).map(|(end, rate)| {
            let last_kill = events
                .iter()
                .rev()
                .find(|event| event.kind == EventKind::Kill && event.tick > 0)
                .map(|event| event.tick);
            recording_end(end, start, freeze(round.saturating_add(1)), last_kill, rate)
        });

        let mut counts = Totals::default();
        let mut headshots = 0;
        let mut round_clips = Vec::new();
        let mut clip_ids = HashSet::new();
        let mut legacy = Vec::new();
        let mut killfeed = Vec::new();
        for event in &events {
            match event.kind {
                EventKind::Kill => counts.kills += 1,
                EventKind::Death => counts.deaths += 1,
                EventKind::Assist => counts.assists += 1,
            }
            let related = clip_links(clips.iter().filter(|clip| clip.relates(round, event.tick)));
            round_clips.extend(related.iter().copied().filter(|id| clip_ids.insert(*id)));
            let (left, right) = event.views(
                round,
                start,
                rate,
                first_kill_tick,
                &related,
                target.name.unwrap_or_default(),
            );
            legacy.push(left);
            killfeed.push(right);
            if event.kind != EventKind::Assist && event.data["headshot"] == true {
                headshots += 1;
            }
        }
        round_clips.extend(
            clips
                .iter()
                .filter(|clip| clip.round == Some(round))
                .filter_map(|clip| clip.id)
                .filter(|id| clip_ids.insert(*id)),
        );
        let tags = highlight_tags(clips.iter().filter(|clip| clip.round == Some(round)));
        totals.kills += counts.kills;
        totals.deaths += counts.deaths;
        totals.assists += counts.assists;
        legacy_rounds.push(json!({
            "round": round, "round_start_tick": start, "round_end_tick": end,
            "record_end_tick": record_end, "winner": winner, "target_won_round": won,
            "score_t": score_t, "score_ct": score_ct,
            "target_kills": counts.kills, "target_deaths": counts.deaths, "target_assists": counts.assists,
            "events": legacy, "related_clip_ids": round_clips, "highlight_tags": tags,
        }));
        killfeed_rounds.push(json!({
            "round_number": round, "side": side, "result": won.map(|won| if won { "win" } else { "loss" }),
            "score_text": score_text, "start_tick": start, "end_tick": end, "record_end_tick": record_end,
            "focused_player": target.name.unwrap_or_default(), "target_player_spec_slot": input.target_spec_slot,
            "summary": {"kills": counts.kills, "deaths": counts.deaths, "assists": counts.assists},
            "player_stats": {"kills": counts.kills, "deaths": counts.deaths, "assists": counts.assists,
                             "headshots": headshots},
            "events": killfeed, "related_clip_ids": round_clips, "highlight_tags": tags,
        }));
    }
    json!({
        "timeline": {
            "demo_path": input.demo_path, "map_name": input.map_name.filter(|name| !name.is_empty()),
            "target_player": {
                "name": target.name.unwrap_or_default(), "steamid": target.steam_string(),
                "user_id": input.target_spec_slot, "team": input.initial_side.and_then(side_name),
            },
            "rounds": legacy_rounds,
            "summary": {"round_count": input.total_rounds, "kill_count": totals.kills,
                        "death_count": totals.deaths, "assist_count": totals.assists},
        },
        "round_timeline": killfeed_rounds,
    })
}

impl<'a> Event<'a> {
    fn new(
        row: &'a Value,
        round: usize,
        tick: i64,
        sequence: usize,
        target: Player<'a>,
        input: &Input<'_>,
    ) -> Option<Self> {
        let attacker = Player {
            name: text(&row["attacker_name"]),
            steamid: steamid(&row["attacker_steamid"]),
        };
        let victim = Player {
            name: text(&row["victim_name"]),
            steamid: steamid(&row["victim_steamid"]),
        };
        let assister = Player {
            name: text(&row["assister_name"]).filter(|name| {
                !matches!(name.to_ascii_lowercase().as_str(), "nan" | "nat" | "none")
            }),
            steamid: steamid(&row["assister_steamid"]),
        };
        let distinct_attacker = attacker.known() && !attacker.same_identity(victim);
        let attacker_is_target = distinct_attacker && attacker.same_identity(target);
        let victim_is_target = victim.same_identity(target);
        let assister_is_target = distinct_attacker && assister.same_identity(target);
        let kind = if attacker_is_target {
            EventKind::Kill
        } else if victim_is_target {
            EventKind::Death
        } else if assister_is_target {
            EventKind::Assist
        } else {
            return None;
        };
        let target_slot_key = target.name.map(str::to_lowercase);
        let slot = |person: Player<'_>, own| {
            if own {
                input.target_spec_slot
            } else {
                let key = person.name?.to_lowercase();
                // A name-only slot cannot distinguish two same-name players.
                if target_slot_key.as_deref() == Some(key.as_str()) {
                    None
                } else {
                    input.spec_slots.get(&key).and_then(nonnegative)
                }
            }
        };
        Some(Self {
            kind,
            id: format!("r{round}-t{tick}-{}-{sequence}", kind.name()),
            tick,
            attacker,
            victim,
            assister,
            attacker_is_target,
            victim_is_target,
            assister_is_target,
            attacker_slot: slot(attacker, attacker_is_target),
            victim_slot: slot(victim, victim_is_target),
            data: row,
        })
    }

    fn views(
        &self,
        round: usize,
        freeze: Option<i64>,
        rate: Option<f64>,
        first_kill: Option<i64>,
        related: &[&str],
        target: &str,
    ) -> (Value, Value) {
        let tick = self.tick;
        let elapsed = freeze
            .zip(rate)
            .filter(|(start, _)| tick >= *start)
            .map(|(start, rate)| (tick - start) as f64 / rate)
            .filter(|time| time.is_finite());
        let suggestion = rate
            .map(|rate| {
                let first = tick
                    .saturating_sub((rate * 6.0) as i64)
                    .max(freeze.unwrap_or(0));
                let last = tick.saturating_add((rate * 4.0) as i64);
                (first, last)
            })
            .filter(|(first, last)| first < last);
        let suggested = suggestion.map(|(first, last)| {
            json!({
                "start_tick": first, "end_tick": last, "pre_seconds": 6.0, "post_seconds": 4.0,
            })
        });
        let kill = self.kind == EventKind::Kill;
        let death = self.kind == EventKind::Death;
        let assist = self.kind == EventKind::Assist;
        let can_record = !assist && suggested.is_some();
        let weapon = text(&self.data["weapon"])
            .map(str::to_lowercase)
            .unwrap_or_default();
        let key = weapon.strip_prefix("weapon_").unwrap_or(&weapon);
        let key = Some(key).filter(|key| !key.is_empty());
        let label = weapon_label(key.unwrap_or_default());
        let headshot = self.data["headshot"] == true;
        let hs_text = if headshot { " 爆头" } else { "" };
        let attacker = self.attacker.name.unwrap_or_default();
        let victim = self.victim.name.unwrap_or_default();
        let killer = self.attacker.known() && !self.attacker.same_identity(self.victim);
        let description = match self.kind {
            EventKind::Kill => format!(
                "{target} 使用 {label}{hs_text} 击杀 {}",
                self.victim.name.unwrap_or("?")
            ),
            EventKind::Death => format!(
                "{target} 被 {} 使用 {label}{hs_text} 击杀",
                self.attacker.name.filter(|_| killer).unwrap_or("?")
            ),
            EventKind::Assist => format!("{target} 助攻 {attacker} 击杀 {victim}"),
        };
        let side = match self.kind {
            EventKind::Kill => read_side(&self.data["attacker_team"]),
            EventKind::Death => read_side(&self.data["victim_team"]),
            EventKind::Assist => None,
        };
        let legacy = json!({
            "id": self.id, "type": self.kind.name(), "round": round, "tick": tick,
            "round_time_seconds": elapsed, "attacker_name": self.attacker.name,
            "attacker_steamid": self.attacker.steam_string(), "attacker_spec_slot": self.attacker_slot,
            "victim_name": self.victim.name, "victim_steamid": self.victim.steam_string(), "victim_spec_slot": self.victim_slot,
            "assister_name": self.assister.name, "assister_steamid": self.assister.steam_string(),
            "weapon": key, "headshot": headshot, "is_target_attacker": self.attacker_is_target,
            "is_target_victim": self.victim_is_target, "is_target_assister": self.assister_is_target,
            "side": side, "description": description, "recordable": can_record, "suggested_clip": suggested,
            "related_clip_ids": related,
        });
        let time_text = elapsed
            .map(|seconds| {
                let seconds = seconds as i64;
                format!("{:02}:{:02}", seconds / 60, seconds % 60)
            })
            .unwrap_or_else(|| "--:--".to_owned());
        let mut feed = json!({
            "id": self.id, "type": if assist { "assist_only" } else { self.kind.name() },
            "round": round, "tick": tick, "time_text": time_text,
            "attacker_name": if death && !killer { "" } else { attacker }, "victim_name": victim,
            "assister_name": if assist { Some(target) } else { self.assister.name },
            "weapon_name": label, "weapon_key": key, "weapon_icon": null,
            "is_player_kill": kill, "is_player_death": death, "can_record": can_record,
            "record_type": (!assist).then(|| self.kind.name()), "start_tick": suggestion.map(|ticks| ticks.0),
            "end_tick": suggestion.map(|ticks| ticks.1), "suggested_clip": suggested, "related_clip_ids": related,
        });
        if assist {
            feed["assist_note"] = json!(format!("助攻：协助 {attacker} 击杀 {victim}"));
        } else {
            feed["attacker_steamid"] = if death && !killer {
                json!("")
            } else {
                json!(self.attacker.steam_string())
            };
            feed["attacker_spec_slot"] = json!(self.attacker_slot.filter(|_| !death || killer));
            feed["victim_steamid"] = json!(self.victim.steam_string());
            feed["victim_spec_slot"] = json!(self.victim_slot);
        }
        let mut modifiers = Map::new();
        for (long, short) in [
            ("is_headshot", "headshot"),
            ("is_through_smoke", "through_smoke"),
            ("is_blind", "attacker_blind"),
            ("is_noscope", "no_scope"),
            ("is_wallbang", "through_wall"),
            ("is_jump_kill", "airborne"),
            ("is_flash_assist", "flash_assisted"),
        ] {
            let flag = !assist && self.data[short] == true;
            feed[long] = json!(flag);
            if !assist {
                modifiers.insert(short.to_owned(), json!(flag));
            }
        }
        if !assist {
            modifiers.insert(
                "first_kill".to_owned(),
                json!(kill && first_kill == Some(tick)),
            );
            modifiers.insert("trade_kill".to_owned(), json!(false));
        }
        feed["modifiers"] = Value::Object(modifiers);
        (legacy, feed)
    }
}

fn recording_end(
    end: i64,
    freeze: Option<i64>,
    next: Option<i64>,
    last_kill: Option<i64>,
    rate: f64,
) -> i64 {
    let buffered = end.saturating_add((3.0 * rate) as i64);
    if let Some(next) = next.filter(|next| *next > end) {
        if last_kill.is_some_and(|tick| tick > end) {
            return next;
        }
        return buffered.min(next);
    }
    let cap = freeze.unwrap_or(end).saturating_add((130.0 * rate) as i64);
    let tail = last_kill.map_or(buffered, |tick| tick.saturating_add((2.5 * rate) as i64));
    buffered.max(tail).min(cap)
}

fn clip_links<'a>(clips: impl Iterator<Item = &'a Clip<'a>>) -> Vec<&'a str> {
    let mut seen = HashSet::new();
    clips
        .filter_map(|clip| clip.id)
        .filter(|id| seen.insert(*id))
        .collect()
}

fn highlight_tags<'a>(clips: impl Iterator<Item = &'a Clip<'a>>) -> Vec<&'a str> {
    const COVERAGE: [(&str, &str); 13] = [
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
    let tags: Vec<_> = clips
        .flat_map(|clip| clip.highlight_tags)
        .filter_map(text)
        .collect();
    let all: HashSet<_> = tags.iter().copied().collect();
    let mut seen = HashSet::new();
    tags.into_iter()
        .filter(|tag| {
            seen.insert(*tag)
                && !COVERAGE
                    .iter()
                    .any(|(covered, stronger)| tag == covered && all.contains(stronger))
        })
        .collect()
}

fn values(data: &Value) -> &[Value] {
    data.as_array().map_or(&[], Vec::as_slice)
}

fn text(data: &Value) -> Option<&str> {
    data.as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn integer(data: &Value) -> Option<i64> {
    data.as_i64().or_else(|| text(data)?.parse().ok())
}

fn nonnegative(data: &Value) -> Option<i64> {
    integer(data).filter(|number| *number >= 0)
}

fn round_number(data: &Value) -> Option<usize> {
    usize::try_from(integer(data)?)
        .ok()
        .filter(|round| *round > 0)
}

fn parse_steamid(value: &str) -> Option<u64> {
    value.trim().parse::<u64>().ok().filter(|id| *id > 0)
}

fn steamid(data: &Value) -> Option<u64> {
    data.as_u64()
        .filter(|id| *id > 0)
        .or_else(|| text(data).and_then(parse_steamid))
}

fn side_name(value: &str) -> Option<&'static str> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("T") || value == "2" {
        Some("T")
    } else if value.eq_ignore_ascii_case("CT") || value == "3" {
        Some("CT")
    } else {
        None
    }
}

fn read_side(data: &Value) -> Option<&'static str> {
    match data.as_i64() {
        Some(2) => Some("T"),
        Some(3) => Some("CT"),
        _ => text(data).and_then(side_name),
    }
}

fn weapon_label(key: &str) -> &str {
    match key.strip_prefix("weapon_").unwrap_or(key) {
        "ak47" => "AK-47",
        "m4a1" => "M4A4",
        "m4a1_silencer" => "消音 M4A1-S",
        "sg556" => "SG 553",
        "aug" => "AUG",
        "famas" => "法玛斯 (FAMAS)",
        "galilar" => "加利尔 (Galil)",
        "awp" => "大狙 (AWP)",
        "ssg08" => "鸟狙 (SSG08)",
        "scar20" => "连狙 (SCAR-20)",
        "g3sg1" => "连狙 (G3SG1)",
        "mac10" => "吹风机 (MAC-10)",
        "mac_10" => "吹风机 (MAC-10)",
        "mp9" => "小蜜蜂 (MP9)",
        "mp7" => "MP7",
        "mp5sd" => "MP5-SD",
        "ump45" => "车王 (UMP-45)",
        "p90" => "P90",
        "bizon" => "野牛 (PP-Bizon)",
        "deagle" => "沙鹰",
        "revolver" => "左轮 (R8)",
        "usp_silencer" => "消音 USP-S",
        "hkp2000" => "P2000",
        "glock" => "格洛克 (Glock-18)",
        "elite" => "双持 (Dual Berettas)",
        "p250" => "P250",
        "fiveseven" => "五七 (Five-SeveN)",
        "tec9" => "TEC-9",
        "cz75a" => "CZ-75",
        "nova" => "Nova",
        "xm1014" => "XM1014",
        "sawedoff" => "截短霰弹枪",
        "mag7" => "MAG-7",
        "m249" => "M249",
        "negev" => "内格夫 (Negev)",
        "hegrenade" => "手雷",
        "flashbang" => "闪光弹",
        "smokegrenade" => "烟雾弹",
        "inferno" => "燃烧弹",
        "molotov" => "燃烧瓶",
        "incgrenade" => "燃烧弹",
        "decoy" => "诱饵弹",
        "taser" => "电击枪 (Zeus)",
        "knife" => "刀",
        "knife_t" => "刀",
        "knife_ct" => "刀",
        "bayonet" => "刺刀",
        "knife_karambit" => "爪子刀",
        "knife_m9_bayonet" => "M9 刺刀",
        "knife_butterfly" => "蝴蝶刀",
        "knife_flip" => "折叠刀",
        "knife_gut" => "穿肠刀",
        "knife_tactical" => "猎杀者匕首",
        "knife_falchion" => "弯刀",
        "knife_survival_bowie" => "博伊猎刀",
        "knife_push" => "暗影双匕",
        "knife_cord" => "系绳匕首",
        "knife_canis" => "求生匕首",
        "knife_ursus" => "熊刀",
        "knife_gypsy_jackknife" => "流浪者匕首",
        "knife_outdoor" => "户外匕首",
        "knife_stiletto" => "短剑",
        "knife_widowmaker" => "锯齿爪刀",
        "knife_skeleton" => "骷髅匕首",
        "knife_css" => "经典刀",
        "knife_kukri" => "廓尔喀刀",
        "world" => "坠落/世界伤害",
        "c4" => "C4 爆炸",
        "planted_c4" => "C4 爆炸",
        "defuse_kit" => "拆弹器",
        "" => "?",
        _ => key,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_source_ticks_and_identity_without_inventing_missing_round_facts() {
        let deaths = json!([
            {"round_number":2,"tick":175,"attacker_name":"Same",
             "attacker_steamid":"76561198000000001",
             "victim_name":"SAME","victim_steamid":"76561198000000002",
             "weapon":" weapon_ak47 ","headshot":true,"through_smoke":true,"attacker_team":2},
            {"round_number":2,"tick":205,"attacker_name":"Foe","victim_name":"Same",
             "victim_steamid":"76561198000000001","weapon":"world","victim_team":3},
            {"round_number":2,"tick":209,"attacker_name":"Foe","victim_name":"Else",
             "assister_name":"Same","assister_steamid":"76561198000000001","headshot":true},
            {"round_number":3,"tick":215,"attacker_name":"Same","attacker_steamid":"76561198000000002",
             "victim_name":"Else"}
        ]);
        let rounds = json!([
            {"round_number":1,"freeze_end_tick":100,"end_tick":170,
             "side":"T","winner":"T","score_t":0,"score_ct":0},
            {"round_number":2,"freeze_end_tick":200,"end_tick":420,
             "side":"CT","winner":"T","score_t":1,"score_ct":0},
            {"round_number":3,"winner":"T"}
        ]);
        let clips = json!([
            {"clip_id":"highlight","round":1,"category":"highlight","context_tags":["爆头","💥 颗秒"]},
            {"clip_id":"next","round":2,"start_tick":166,"end_tick":180}
        ]);
        let slots = json!({"same":8,"foe":9});
        let output = project(Input {
            demo_path: "fixture.dem",
            map_name: Some("de_fixture"),
            target_name: "Same",
            target_steamid64: Some("76561198000000001"),
            target_spec_slot: Some(4),
            initial_side: Some("T"),
            tick_rate: 64.0,
            total_rounds: 3,
            deaths: deaths.as_array().unwrap(),
            rounds: rounds.as_array().unwrap(),
            clips: clips.as_array().unwrap(),
            spec_slots: slots.as_object().unwrap(),
        });
        let first = &output["round_timeline"][0];
        assert_eq!(first["record_end_tick"], 200);
        assert_eq!(first["events"][0]["id"], "r1-t175-kill-1");
        assert_eq!(first["events"][0]["attacker_spec_slot"], 4);
        assert_eq!(first["events"][0]["victim_steamid"], "76561198000000002");
        assert!(first["events"][0]["victim_spec_slot"].is_null());
        assert_eq!(first["events"][0]["weapon_name"], "AK-47");
        assert_eq!(first["events"][0]["modifiers"]["through_smoke"], true);
        assert_eq!(first["events"][0]["modifiers"]["first_kill"], true);
        assert_eq!(first["events"][0]["suggested_clip"]["start_tick"], 100);
        assert_eq!(first["related_clip_ids"], json!(["highlight", "next"]));
        assert_eq!(first["highlight_tags"], json!(["💥 颗秒"]));
        let second = &output["round_timeline"][1];
        assert_eq!(second["score_text"], "1:0");
        assert_eq!(second["result"], "loss");
        assert_eq!(second["side"], "CT");
        assert_eq!(second["events"][0]["attacker_spec_slot"], 9);
        assert_eq!(second["events"][1]["type"], "assist_only");
        assert_eq!(second["events"][1]["can_record"], false);
        assert_eq!(second["events"][1]["modifiers"], json!({}));
        assert_eq!(second["player_stats"]["headshots"], 0);
        let third = &output["round_timeline"][2];
        assert!(third["start_tick"].is_null() && third["end_tick"].is_null());
        assert!(third["record_end_tick"].is_null() && third["result"].is_null());
        assert_eq!(third["score_text"], "--");
        assert_eq!(third["events"], json!([]));
        assert_eq!(
            output["timeline"]["summary"],
            json!({
                "round_count":3,"kill_count":1,"death_count":1,"assist_count":1,
            })
        );
        assert_eq!(
            output["timeline"]["rounds"][1]["events"][1]["type"],
            "assist"
        );
    }
}
