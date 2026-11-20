use super::*;
use serde_json::{json, Value};

const PLAYER: &str = "b0000001-0000-4000-8000-000000000001";
const OTHER: &str = "b0000002-0000-4000-8000-000000000002";

// Synthetic identities and scores using the pinned Akari
// 2026-05-16-tencent-hn10 LCU page and q_450 / q_2400 game shapes.
fn game(id: u64, start: u64, mode: &str) -> Value {
    json!({
        "gameId": id, "gameCreation": start, "gameDuration": 953,
        "gameMode": mode, "platformId": "HN10", "endOfGameResult": "GameComplete",
        "participantIdentities": [
            {"participantId": 1, "player": {"puuid": OTHER}},
            {"participantId": 4, "player": {"puuid": PLAYER, "summonerName": "Not rendered"}}
        ],
        "participants": [
            {"participantId": 1, "championId": 1, "stats": {"win": false, "kills": 2, "deaths": 6, "assists": 3}},
            {"participantId": 4, "championId": 22, "stats": {"win": true, "kills": 7, "deaths": 8, "assists": 21}}
        ]
    })
}

fn project(platform: &str, account_id: u64, rows: Vec<Value>) -> Value {
    let reply: MatchHistoryResponse = serde_json::from_value(json!({
        "accountId": account_id, "platformId": platform, "games": {"games": rows}
    }))
    .unwrap();
    let shard = shard_from_platform("HN10").unwrap();
    let player = Puuid::parse(PLAYER).unwrap();
    serde_json::to_value(reply.project(&player, &shard, Some(42))).unwrap()
}

#[test]
fn projects_only_recent_verified_aram_matches_from_the_list_shape() {
    let mut kiwi = game(21, 1_778_860_291_493, "KIWI");
    kiwi["participantIdentities"] = json!([{"participantId": 9, "player": {"puuid": PLAYER}}]);
    kiwi["participants"][1]["participantId"] = json!(9);
    assert_eq!(
        project(
            "TENCENT_HN10",
            42,
            vec![
                game(20, 1_768_971_183_246, "ARAM"),
                game(19, 1_778_860_292_493, "URF"),
                kiwi,
            ]
        ),
        json!({
            "status": "ready",
            "matches": [
                {"gameId": "21", "gameMode": "KIWI", "startedAt": 1_778_860_291_493u64,
                 "durationSeconds": 953, "championId": 22, "outcome": "win", "kills": 7, "deaths": 8, "assists": 21},
                {"gameId": "20", "gameMode": "ARAM", "startedAt": 1_768_971_183_246u64,
                 "durationSeconds": 953, "championId": 22, "outcome": "win", "kills": 7, "deaths": 8, "assists": 21},
            ]
        })
    );
}

#[test]
fn distinguishes_an_empty_page_from_an_unavailable_or_mismatched_reply() {
    let eligible = game(20, 1_768_971_183_246, "ARAM");
    let other_mode = game(19, 1_768_971_183_247, "BRAWL");
    assert_eq!(
        project("HN10", 0, vec![eligible.clone()])["status"],
        "ready"
    );
    assert_eq!(
        project("HN10", 42, vec![]),
        json!({"status": "ready", "matches": []})
    );
    assert_eq!(
        project("HN10", 42, vec![other_mode]),
        json!({"status": "ready", "matches": []})
    );
    assert_eq!(
        project("HN1", 42, vec![eligible.clone()]),
        json!({"status": "unavailable"})
    );
    assert_eq!(
        project("HN10", 43, vec![]),
        json!({"status": "unavailable"})
    );
    let mut wrong_match_platform = eligible.clone();
    wrong_match_platform["platformId"] = json!("HN1");
    assert_eq!(
        project("HN10", 42, vec![wrong_match_platform]),
        json!({"status": "unavailable"})
    );
    let mut wrong_player = eligible.clone();
    wrong_player["participantIdentities"][1]["player"]["puuid"] = json!(OTHER);
    assert_eq!(
        project("HN10", 42, vec![wrong_player]),
        json!({"status": "unavailable"})
    );
    let mut ambiguous = eligible;
    ambiguous["participants"][0]["participantId"] = json!(4);
    assert_eq!(
        project("HN10", 42, vec![ambiguous]),
        json!({"status": "unavailable"})
    );
}

#[test]
fn ranks_and_deduplicates_matches_and_keeps_remakes_and_aborts_distinct() {
    let mut rows = (1..=8)
        .map(|id| game(id, 1_768_971_183_240 + id, "ARAM"))
        .collect::<Vec<_>>();
    rows[6]["participants"][1]["stats"]["gameEndedInEarlySurrender"] = json!(true);
    rows[5]["participants"][1]["stats"]["teamEarlySurrendered"] = json!(true);
    rows[4]["endOfGameResult"] = json!("Abort_AntiCheatExit");
    rows.push(rows[7].clone());
    let recent = project("HN10", 42, rows);
    let matches = recent["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 6);
    assert_eq!(
        matches
            .iter()
            .map(|row| row["gameId"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["8", "7", "6", "5", "4", "3"]
    );
    assert_eq!(matches[1]["outcome"], "remake");
    assert_eq!(matches[2]["outcome"], "loss");
    assert_eq!(matches[3]["outcome"], "abort");
    assert_eq!(
        project("HN10", 42, vec![game(1, 1_768_971_183_246, "ARAM"); 21]),
        json!({"status": "unavailable"})
    );
}
