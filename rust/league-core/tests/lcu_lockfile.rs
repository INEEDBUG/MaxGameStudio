use maxgamestudio_league_core::lcu_lockfile::LcuLockfile;

#[test]
fn reads_the_client_lockfile_with_or_without_windows_newline() {
    for ending in ["", "\r\n"] {
        let fixture = format!("LeagueClient:8735:51936:fixture-token_1:https{ending}");
        let state = LcuLockfile::parse(&fixture).unwrap();
        assert_eq!((state.pid, state.port), (8735, 51936));
        assert!(state.token == "fixture-token_1");
    }
}

#[test]
fn malformed_or_unrelated_lockfiles_cannot_select_a_listener() {
    for fixture in [
        "RiotClientServices:8735:51936:fixture:https",
        "LeagueClient:0:51936:fixture:https",
        "LeagueClient:8735:0:fixture:https",
        "LeagueClient:8735:100000:fixture:https",
        "LeagueClient:8735:51936::https",
        "LeagueClient:8735:51936:invalid token:https",
        "LeagueClient:8735:51936:fixture:http",
        "LeagueClient:8735:51936:fixture:https:trailing",
        "LeagueClient:8735:51936:fixture:https\nLeagueClient:1:2:fixture:https",
    ] {
        assert!(
            LcuLockfile::parse(fixture).is_none(),
            "invalid fixture was parsed"
        );
    }
}
