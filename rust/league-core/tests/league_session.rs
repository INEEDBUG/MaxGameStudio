use maxgamestudio_league_core::league_session::LeagueSession;
use serde_json::{json, Value};

fn event(uri: &str, event_type: &str, data: Value) -> Vec<u8> {
    serde_json::to_vec(&json!([8, "OnJsonApiEvent", {
        "uri": uri, "eventType": event_type, "data": data,
    }]))
    .unwrap()
}

fn game(id: u64, mode: &str) -> Vec<u8> {
    event(
        "/lol-gameflow/v1/session",
        "Update",
        json!({"gameData":{"gameId":id,"queue":{"gameMode":mode}}}),
    )
}

fn select(champion: i64) -> Vec<u8> {
    event(
        "/lol-champ-select/v1/session",
        "Update",
        json!({
            "localPlayerCellId":1,
            "myTeam":[{"cellId":1,"championId":champion}],
            "actions":[],
        }),
    )
}

#[test]
fn observes_hextech_champion_changes_and_clears_deleted_selections() {
    let mut state = LeagueSession::default();
    let generation = state.connect();
    state.observe(generation, &game(45, "KIWI")).unwrap();
    state
        .observe(
            generation,
            &event(
                "/lol-gameflow/v1/gameflow-phase",
                "Update",
                json!("ChampSelect"),
            ),
        )
        .unwrap();
    state.observe(generation, &select(22)).unwrap();
    assert_eq!(state.game_mode(), Some("KIWI"));
    assert_eq!(state.resg_champion(), Some(22));
    state.observe(generation, &select(73)).unwrap();
    assert_eq!(state.resg_champion(), Some(73));
    state
        .observe(
            generation,
            &event("/lol-champ-select/v1/session", "Delete", json!({})),
        )
        .unwrap();
    assert_eq!(state.resg_champion(), None);
}

#[test]
fn reconnect_and_new_game_do_not_reuse_a_previous_champion() {
    let mut state = LeagueSession::default();
    let old = state.connect();
    state.observe(old, &game(1, "KIWI")).unwrap();
    state
        .observe(
            old,
            &event(
                "/lol-gameflow/v1/gameflow-phase",
                "Update",
                json!("ChampSelect"),
            ),
        )
        .unwrap();
    state.observe(old, &select(22)).unwrap();
    state.observe(old, &game(2, "KIWI")).unwrap();
    assert_eq!(state.resg_champion(), None);

    let fresh = state.connect();
    assert!(!state.observe(old, &game(1, "KIWI")).unwrap());
    assert!(!state.disconnect(old));
    assert_eq!(state.game_mode(), None);
    assert!(state.observe(fresh, &game(3, "ARAM")).unwrap());
    assert_eq!(state.game_mode(), Some("ARAM"));
    assert!(state.disconnect(fresh));
    assert_eq!(state.game_mode(), None);
    assert!(!state.observe(fresh, &game(3, "ARAM")).unwrap());
}

#[test]
fn only_tracks_the_game_data_required_by_the_toolbox() {
    let mut state = LeagueSession::default();
    let generation = state.connect();
    assert!(!state
        .observe(generation, b"[0,\"welcome\",1,\"League\"]")
        .unwrap());
    assert!(!state
        .observe(generation, &event("/other", "Update", json!({})))
        .unwrap());
    assert!(state.observe(generation, b"[").is_err());
}
