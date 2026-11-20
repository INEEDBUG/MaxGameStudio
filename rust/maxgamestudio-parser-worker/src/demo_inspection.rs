use crate::{case_fold::case_fold, header_fields, parser_inputs, replay_io, Failure};
use parser::{
    parse_demo::{Parser, ParsingMode},
    second_pass::parser_settings::{create_huffman_lookup_table, PlayerEndMetaData},
};
use serde::Serialize;
use serde_json::{json, Value};
use std::{collections::HashSet, path::Path};

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
        // A useful header and empty list let the caller request full inspection
        // on files without the fast player records.
        let players = parsed
            .map(|output| collect_players(output.player_md))
            .unwrap_or_default();
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
        let Some(name) = record.name else {
            continue;
        };
        let name =
            name.trim_matches(|ch: char| ch.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&ch));
        if name.is_empty() || name.eq_ignore_ascii_case("nan") {
            continue;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
