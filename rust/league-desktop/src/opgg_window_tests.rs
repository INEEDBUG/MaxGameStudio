use super::*;
use serde_json::json;

fn fixture(mode: &str, phase: &str, game: u64) -> (GameflowSession, Value) {
    let flow = serde_json::from_value(json!({
        "phase": phase,
        "gameData": {
            "gameId": game, "isCustomGame": false,
            "queue": {"gameMode": mode, "type": "ARAM"}
        }
    }))
    .unwrap();
    let select = json!({
        "gameId": game, "isCustomGame": false, "isSpectating": false,
        "localPlayerCellId": 1,
        "myTeam": [{"cellId": 0, "championId": 55}, {"cellId": 1, "championId": 22}]
    });
    (flow, select)
}

fn client() -> ClientInstance {
    ClientInstance {
        pid: 14,
        created_at: 1000,
    }
}

#[test]
fn targets_only_current_local_aram_or_kiwi_drafts() {
    for (mode, expected) in [
        ("ARAM", true),
        ("KIWI", true),
        ("CLASSIC", false),
        ("URF", false),
    ] {
        let (flow, select) = fixture(mode, "ChampSelect", 45);
        let target = OpggTarget::from_sample(client(), &flow, &select);
        assert_eq!(target.is_some(), expected);
        if let Some(target) = target {
            assert_eq!(target.view(), json!({"gameMode": mode, "championId": 22}));
        }
    }
    for phase in [
        "Lobby",
        "ReadyCheck",
        "GameStart",
        "InProgress",
        "EndOfGame",
    ] {
        let (flow, select) = fixture("ARAM", phase, 45);
        assert!(OpggTarget::from_sample(client(), &flow, &select).is_none());
    }
    let (flow, mut select) = fixture("ARAM", "ChampSelect", 45);
    select["isSpectating"] = json!(true);
    assert!(OpggTarget::from_sample(client(), &flow, &select).is_none());
    select["isSpectating"] = json!(false);
    select["gameId"] = json!(46);
    assert!(OpggTarget::from_sample(client(), &flow, &select).is_none());
    select["gameId"] = json!(45);
    select["localPlayerCellId"] = json!(2);
    assert!(OpggTarget::from_sample(client(), &flow, &select).is_none());
}

#[test]
fn opens_only_the_public_mode_or_validated_champion_route() {
    let (flow, select) = fixture("ARAM", "ChampSelect", 45);
    let target = OpggTarget::from_sample(client(), &flow, &select).unwrap();
    assert_eq!(
        target.guide_url(Some("MonkeyKing")),
        "https://op.gg/lol/modes/aram/monkeyking/build"
    );
    for alias in [
        "",
        "..",
        "foo/../../",
        "https://elsewhere.invalid",
        "one%2ftwo",
        "A?query=1",
        "i#x",
    ] {
        assert_eq!(
            target.guide_url(Some(alias)),
            "https://op.gg/lol/modes/aram"
        );
    }
    let (flow, mut select) = fixture("KIWI", "ChampSelect", 45);
    let target = OpggTarget::from_sample(client(), &flow, &select).unwrap();
    assert_eq!(
        target.guide_url(Some("Ashe")),
        "https://op.gg/lol/modes/aram-mayhem/ashe/build"
    );

    select["myTeam"][1]["championId"] = json!(0);
    let unassigned = OpggTarget::from_sample(client(), &flow, &select).unwrap();
    assert_eq!(
        unassigned.view(),
        json!({"gameMode": "KIWI", "championId": null})
    );
    assert_eq!(
        unassigned.guide_url(Some("Ashe")),
        "https://op.gg/lol/modes/aram-mayhem"
    );
}

#[test]
fn opens_at_most_once_per_draft_and_closes_when_ineligible() {
    let (flow, select) = fixture("ARAM", "ChampSelect", 45);
    let first = OpggTarget::from_sample(client(), &flow, &select).unwrap();
    let (flow, select) = fixture("KIWI", "ChampSelect", 46);
    let second = OpggTarget::from_sample(client(), &flow, &select).unwrap();
    let mut state = OpggState::new(OpggPreferences {
        enabled: true,
        auto_show: false,
    });
    assert_eq!(state.decision(None), OpggDecision::Close);
    assert_eq!(state.decision(Some(first)), OpggDecision::Ignore);
    state.preferences.auto_show = true;
    assert_eq!(state.decision(Some(first)), OpggDecision::Show);
    assert_eq!(state.decision(Some(first)), OpggDecision::Ignore);
    assert_eq!(state.decision(None), OpggDecision::Close);
    assert_eq!(state.decision(Some(first)), OpggDecision::Ignore);
    assert_eq!(state.decision(Some(second)), OpggDecision::Show);
    state.set_enabled(false);
    assert_eq!(state.decision(Some(second)), OpggDecision::Close);
    state.set_enabled(true);
    assert_eq!(state.decision(Some(second)), OpggDecision::Show);
}
