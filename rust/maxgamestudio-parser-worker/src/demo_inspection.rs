use crate::{
    case_fold::case_fold,
    header_fields, parser_inputs, replay_io,
    round_windows::{end_tick, field, integer, match_start_tick, match_summary},
    Failure,
};
use parser::{
    first_pass::{
        parser_settings::rm_user_friendly_names,
        prop_controller::{NAME_ID, TICK_ID},
    },
    parse_demo::{DemoOutput, Parser, ParsingMode},
    second_pass::{
        game_events::GameEvent,
        parser_settings::{create_huffman_lookup_table, PlayerEndMetaData},
        variants::{VarVec, Variant},
    },
};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, HashMap, HashSet},
    path::Path,
};

pub(crate) const CONTROLLER_TEAM: &str = "CCSPlayerController.m_iTeamNum";

#[derive(Debug, Serialize)]
struct Player {
    name: String,
    team: i32,
    kills: u32,
    deaths: u32,
    assists: u32,
    user_id: Option<u32>,
    steam_id: Option<String>,
}

pub(crate) fn inspect(path: &Path) -> Result<Value, Failure> {
    replay_io::with_demo_bytes(path, |bytes| {
        let header = header_fields(bytes)?;
        let huffman = create_huffman_lookup_table();
        let parsed = Parser::new(parser_inputs(&huffman), ParsingMode::Normal).parse_demo(bytes);
        let players = parsed
            .map(|output| collect_players(output.player_md))
            .unwrap_or_default();
        if players.is_empty() {
            let (roster, summary) = inspect_death_roster(bytes, &huffman, Some(&header))?;
            if let Some(summary) = summary.filter(|_| !roster.is_empty()) {
                return Ok(json!({ "players": roster, "match_meta": summary }));
            }
        }
        let map = header
            .get("map_name")
            .filter(|name| !name.is_empty())
            .map_or("unknown", String::as_str);
        Ok(json!({
            "players": players,
            "match_meta": {
                "map_name": map,
                "server_name": header.get("server_name").map_or("", String::as_str),
                "target_player": "",
                "target_player_user_id": null,
                "target_steam_id": null,
                "total_rounds": null,
                "target_kills": null,
                "target_deaths": null,
                "team_a_score": null,
                "team_b_score": null,
                "team_a_name": null,
                "team_b_name": null,
                "match_date": "",
                "duration_mins": null,
                "demo_end_tick": null,
                "inspection_level": "header_signon",
            }
        }))
    })
}

fn collect_players(records: Vec<PlayerEndMetaData>) -> Vec<Player> {
    let mut seen = HashSet::new();
    let mut players = Vec::new();
    for record in records {
        let Some(name) = record.name.as_deref().and_then(player_name) else {
            continue;
        };
        let steamid = record.steamid.filter(|&id| id > 0);
        let folded = case_fold(name);
        if !seen.insert((folded.clone(), steamid)) {
            continue;
        }
        let team = record
            .team_number
            .filter(|team| matches!(team, 2 | 3))
            .unwrap_or(0);
        players.push((
            folded,
            Player {
                name: name.to_owned(),
                team,
                kills: 0,
                deaths: 0,
                assists: 0,
                user_id: None,
                steam_id: steamid.map(|id| id.to_string()),
            },
        ));
    }
    players.sort_by(|(left_name, left), (right_name, right)| {
        (left.team, left_name).cmp(&(right.team, right_name))
    });
    players.into_iter().map(|(_, player)| player).collect()
}

pub(crate) fn player_name(value: &str) -> Option<&str> {
    let value =
        value.trim_matches(|ch: char| ch.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&ch));
    (!value.is_empty() && !value.eq_ignore_ascii_case("nan")).then_some(value)
}

pub(crate) fn inspect_roster(path: &Path) -> Result<Value, Failure> {
    replay_io::with_demo_bytes(path, |bytes| {
        header_fields(bytes)?;
        let huffman = create_huffman_lookup_table();
        let (players, _) = inspect_death_roster(bytes, &huffman, None)?;
        Ok(json!({ "players": players }))
    })
}

fn inspect_death_roster(
    bytes: &[u8],
    huffman: &Vec<(u8, u8)>,
    summary_header: Option<&BTreeMap<String, String>>,
) -> Result<(Vec<Player>, Option<Value>), Failure> {
    let mut inputs = parser_inputs(huffman);
    inputs.only_header = false;
    inputs.parse_ents = true;
    inputs.wanted_events = ["player_death", "round_announce_match_start"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    if summary_header.is_some() {
        inputs.wanted_events.push("round_end".to_owned());
    }
    let events = match Parser::new(inputs, ParsingMode::Normal).parse_demo(bytes) {
        Ok(output) => output.game_events,
        // A header-only recording has no gameplay at any positive tick.
        Err(_) if end_tick(bytes)? == 0 => return Ok((Vec::new(), None)),
        Err(_) => {
            return Err(Failure::replay_parse(
                "could not read Demo player death events",
            ))
        }
    };
    let start_tick = match_start_tick(&events);
    let deaths: Vec<_> = events
        .iter()
        .filter(|event| event.name == "player_death")
        .collect();
    let mut team_ticks: Vec<_> = deaths
        .iter()
        .map(|event| event.tick)
        .filter(|&tick| start_tick <= 0 || tick >= start_tick)
        .collect();
    if team_ticks.is_empty() {
        return Ok((Vec::new(), None));
    }
    if start_tick > 0 {
        team_ticks.push(start_tick);
    }
    let summary_end = summary_header.map(|_| end_tick(bytes)).transpose()?;
    let properties = if let Some(demo_end) = summary_end {
        // The fallback already reads the death-event teams. Fetch the round-end
        // teams in the same selected-tick scan for the summary.
        team_ticks.extend(match_summary::summary_ticks(&events, demo_end));
        match_summary::PROPERTIES
    } else {
        &[]
    };
    team_ticks.sort_unstable();
    team_ticks.dedup();
    let team_output = parse_team_ticks(bytes, huffman, team_ticks, properties)?;
    let teams = team_samples(&team_output)?;
    let roster = death_roster(deaths, start_tick, &teams);
    let summary = summary_header.zip(summary_end).map(|(header, demo_end)| {
        match_summary::summarize(header, &events, demo_end, Some(&team_output))
    });
    Ok((roster, summary))
}

pub(crate) fn parse_team_ticks(
    bytes: &[u8],
    huffman: &Vec<(u8, u8)>,
    ticks: Vec<i32>,
    extra_props: &[&str],
) -> Result<DemoOutput, Failure> {
    let wanted = ["team_num", CONTROLLER_TEAM, "name"]
        .into_iter()
        .chain(extra_props.iter().copied())
        .map(str::to_owned)
        .collect();
    let props = rm_user_friendly_names(&wanted)
        .map_err(|_| Failure::replay_parse("Demo player team properties are unavailable"))?;
    let mut inputs = parser_inputs(huffman);
    inputs.parse_ents = true;
    inputs.real_name_to_og_name = props.iter().cloned().zip(wanted).collect();
    inputs.wanted_player_props = props;
    inputs.wanted_ticks = ticks;
    Parser::new(inputs, ParsingMode::Normal)
        .parse_demo(bytes)
        .map_err(|_| Failure::replay_parse("could not read Demo player teams"))
}

type Teams<'a> = BTreeMap<i32, Vec<(&'a str, i32)>>;

fn team_samples(output: &DemoOutput) -> Result<Teams<'_>, Failure> {
    let mut teams = Teams::new();
    if output.df.is_empty() {
        return Ok(teams);
    }
    let column = |id| output.df.get(&id).and_then(|column| column.data.as_ref());
    let (Some(VarVec::I32(ticks)), Some(VarVec::String(names))) =
        (column(TICK_ID), column(NAME_ID))
    else {
        return Err(Failure::replay_parse(
            "Demo team rows are missing ticks or player names",
        ));
    };
    if ticks.len() != names.len() {
        return Err(Failure::replay_parse(
            "Demo team rows have inconsistent lengths",
        ));
    }
    let pawn_team = named_column(output, "team_num");
    let controller_team = named_column(output, CONTROLLER_TEAM);
    for (index, (tick, name)) in ticks.iter().zip(names).enumerate() {
        let (Some(tick), Some(name)) = (tick, name.as_deref().and_then(player_name)) else {
            continue;
        };
        if let Some(team) =
            team_at_column(pawn_team, index).or_else(|| team_at_column(controller_team, index))
        {
            record_named(teams.entry(*tick).or_default(), name, team);
        }
    }
    Ok(teams)
}

pub(crate) fn named_column<'a>(output: &'a DemoOutput, name: &str) -> Option<&'a VarVec> {
    let prop = output
        .prop_controller
        .prop_infos
        .iter()
        .find(|prop| prop.prop_friendly_name == name)?;
    output.df.get(&prop.id)?.data.as_ref()
}

pub(crate) fn team_at_column(column: Option<&VarVec>, index: usize) -> Option<i32> {
    let team = match column? {
        VarVec::I32(values) => values.get(index)?.as_ref().map(|&team| i64::from(team)),
        VarVec::U32(values) => values.get(index)?.as_ref().map(|&team| i64::from(team)),
        _ => None,
    }?;
    playable_team(team)
}

fn playable_team(team: i64) -> Option<i32> {
    matches!(team, 2 | 3).then_some(team as i32)
}

struct Stats {
    sequence: usize,
    team: Option<i32>,
    kills: u32,
    deaths: u32,
    assists: u32,
}

fn touch<'stats, 'name>(
    stats: &'stats mut HashMap<&'name str, Stats>,
    name: &'name str,
) -> &'stats mut Stats {
    let sequence = stats.len();
    stats.entry(name).or_insert(Stats {
        sequence,
        team: None,
        kills: 0,
        deaths: 0,
        assists: 0,
    })
}

fn death_roster(mut deaths: Vec<&GameEvent>, start_tick: i32, teams: &Teams<'_>) -> Vec<Player> {
    let mut ids = Vec::new();
    for event in &deaths {
        for (name, steam) in [
            ("user_name", "user_steamid"),
            ("attacker_name", "attacker_steamid"),
        ] {
            if let (Some(name), Some(id)) = (named_role(event, &[name]), steam_id(event, steam)) {
                record_named(&mut ids, name, id);
            }
        }
    }
    deaths.sort_by_key(|event| event.tick);
    let mut stats = HashMap::new();
    for event in deaths
        .into_iter()
        .filter(|event| start_tick <= 0 || event.tick >= start_tick)
    {
        let attacker = named_role(event, &["attacker_name"]);
        let victim = named_role(event, &["user_name"]);
        let assister = named_role(event, &["assister_name", "assister", "assistor_name"]);
        for (role, fields) in [
            (attacker, &["attackerteam"][..]),
            (victim, &["userteam"][..]),
            (
                assister,
                &["assister_team", "assisterteam", "assistersteam"][..],
            ),
        ] {
            if let Some(name) = role {
                let team = teams
                    .get(&event.tick)
                    .and_then(|sample| lookup_named(sample, name))
                    .or_else(|| {
                        fields.iter().find_map(|key| {
                            field(event, key).and_then(integer).and_then(playable_team)
                        })
                    });
                if let Some(team) = team {
                    touch(&mut stats, name).team.get_or_insert(team);
                }
            }
        }
        if let Some(name) = victim {
            touch(&mut stats, name).deaths += 1;
        }
        if let Some(name) = attacker.filter(|_| attacker != victim) {
            touch(&mut stats, name).kills += 1;
        }
        if let Some(name) = assister.filter(|_| assister != victim) {
            touch(&mut stats, name).assists += 1;
        }
    }
    if start_tick > 0 {
        if let Some(starting_teams) = teams.get(&start_tick) {
            for (player, team) in starting_teams {
                let player = player.to_lowercase();
                let first = stats
                    .iter_mut()
                    .filter(|(name, _)| name.to_lowercase() == player)
                    .min_by_key(|(_, stats)| stats.sequence);
                if let Some((_, record)) = first {
                    record.team = Some(*team);
                }
            }
        }
    }

    let mut rows: Vec<_> = stats
        .into_iter()
        .map(|(name, stats)| {
            (
                Reverse(stats.kills),
                name.to_lowercase(),
                stats.sequence,
                name,
                stats,
            )
        })
        .collect();
    rows.sort_unstable_by(|left, right| {
        (&left.0, &left.1, left.2).cmp(&(&right.0, &right.1, right.2))
    });
    rows.into_iter()
        .map(|(_, _, _, name, stats)| Player {
            name: name.to_owned(),
            team: stats.team.unwrap_or(0),
            kills: stats.kills,
            deaths: stats.deaths,
            assists: stats.assists,
            user_id: None,
            steam_id: lookup_named(&ids, name).map(|id| id.to_string()),
        })
        .collect()
}

fn named_role<'a>(event: &'a GameEvent, fields: &[&str]) -> Option<&'a str> {
    fields.iter().find_map(|key| match field(event, key) {
        Some(Variant::String(name)) => player_name(name),
        _ => None,
    })
}

fn steam_id(event: &GameEvent, key: &str) -> Option<u64> {
    let id = match field(event, key)? {
        Variant::U64(value) => *value,
        value => u64::try_from(integer(value)?).ok()?,
    };
    (id > 0).then_some(id)
}

fn record_named<'a, T>(entries: &mut Vec<(&'a str, T)>, name: &'a str, value: T) {
    if let Some((_, existing)) = entries.iter_mut().find(|(key, _)| *key == name) {
        *existing = value;
    } else {
        entries.push((name, value));
    }
}

fn lookup_named<T: Copy>(entries: &[(&str, T)], name: &str) -> Option<T> {
    entries
        .iter()
        .find(|(key, _)| *key == name)
        .or_else(|| {
            let name = name.to_lowercase();
            entries.iter().find(|(key, _)| key.to_lowercase() == name)
        })
        .map(|(_, value)| *value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use parser::second_pass::game_events::EventField;

    fn record(
        name: Option<&str>,
        steamid: Option<u64>,
        team_number: Option<i32>,
    ) -> PlayerEndMetaData {
        PlayerEndMetaData {
            name: name.map(str::to_owned),
            steamid,
            team_number,
        }
    }

    #[test]
    fn fast_player_rows_trim_fold_deduplicate_and_sort_like_python() {
        let rows = collect_players(vec![
            record(None, Some(13), None),
            record(Some("nAn "), Some(13), Some(1)),
            record(Some(" ß\u{1c} "), Some(42), Some(3)),
            record(Some("SS"), Some(42), Some(2)),
            record(Some("Σ"), Some(3), Some(2)),
            record(Some("ς"), Some(3), Some(2)),
            record(Some("Zulu"), Some(u64::MAX), Some(2)),
            record(Some("alice"), Some(0), Some(1)),
            record(Some("ALICE"), None, Some(2)),
            record(Some("ALICE"), Some(1), None),
        ]);
        let compact = rows
            .into_iter()
            .map(|row| (row.name, row.team, row.steam_id))
            .collect::<Vec<_>>();
        assert_eq!(
            compact,
            [
                ("alice".to_owned(), 0, None),
                ("ALICE".to_owned(), 0, Some("1".to_owned())),
                ("Zulu".to_owned(), 2, Some(u64::MAX.to_string())),
                ("Σ".to_owned(), 2, Some("3".to_owned())),
                ("ß".to_owned(), 3, Some("42".to_owned())),
            ]
        );
    }

    #[test]
    fn death_roster_skips_warmup_counts_world_and_self_deaths_and_keeps_opening_teams() {
        let death = |tick, attacker: &str, victim: &str, assister: &str| GameEvent {
            name: "player_death".to_owned(),
            tick,
            fields: [
                ("attacker_name", attacker),
                ("user_name", victim),
                ("assister_name", assister),
            ]
            .into_iter()
            .map(|(name, value)| EventField {
                name: name.to_owned(),
                data: Some(Variant::String(value.to_owned())),
            })
            .collect(),
        };
        let mut warmup = death(2, "Alpha", "Warmup", "");
        warmup.fields.push(EventField {
            name: "attacker_steamid".to_owned(),
            data: Some(Variant::U64(u64::MAX)),
        });
        let events = [
            warmup,
            death(11, " Alpha ", "Beta", "Gamma"),
            death(12, "Beta", "Beta", "Beta"),
            death(13, "", "Alpha", ""),
        ];
        let teams = Teams::from([
            (10, vec![("ALPHA", 2), ("Beta", 3), ("Gamma", 2)]),
            (11, vec![("Alpha", 3), ("Beta", 2), ("Gamma", 3)]),
        ]);
        let actual = death_roster(events.iter().collect(), 10, &teams);
        assert_eq!(
            actual
                .iter()
                .map(|row| (
                    row.name.as_str(),
                    row.team,
                    row.kills,
                    row.deaths,
                    row.assists
                ))
                .collect::<Vec<_>>(),
            [
                ("Alpha", 2, 1, 1, 0),
                ("Beta", 3, 0, 2, 0),
                ("Gamma", 2, 0, 0, 1)
            ]
        );
        assert_eq!(
            actual[0].steam_id.as_deref(),
            Some(u64::MAX.to_string().as_str())
        );
    }
}
