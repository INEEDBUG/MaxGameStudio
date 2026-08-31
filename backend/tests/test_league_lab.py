import asyncio
import json
import stat
import subprocess
import time

import httpx
import pytest
from pydantic import ValidationError

from app import league_lab
from app.league_lab import LeagueLabService, LeagueLabSettings, parse_league_client_command_line


def test_player_tags_are_account_scoped_and_manageable(monkeypatch, tmp_path):
    monkeypatch.setattr(league_lab, "_player_tags_path", lambda: tmp_path / "league-player-tags.json")
    monkeypatch.setattr(league_lab.league_lab_service, "current_summoner", {"puuid": "owner-a"})

    asyncio.run(league_lab.save_league_player_tag("target", league_lab.PlayerTagBody(label="队友", note="稳定")))
    assert league_lab._find_player_tag(league_lab._read_player_tags(), "target") == {
        "label": "队友",
        "note": "稳定",
        "color": "emerald",
    }

    monkeypatch.setattr(league_lab.league_lab_service, "current_summoner", {"puuid": "owner-b"})
    assert league_lab._find_player_tag(league_lab._read_player_tags(), "target") == {}
    asyncio.run(league_lab.save_league_player_tag("target", league_lab.PlayerTagBody(label="对手")))

    current = asyncio.run(league_lab.list_league_player_tags(current_account_only=True))
    assert current["total"] == 1
    assert current["rows"][0]["owner_puuid"] == "owner-b"
    assert current["rows"][0]["tag"]["label"] == "对手"
    assert asyncio.run(league_lab.list_league_player_tags(current_account_only=False))["total"] == 2

    updated = asyncio.run(league_lab.update_managed_league_player_tag(
        "owner-b::target", league_lab.PlayerTagBody(label="宿敌", color="rose")
    ))
    assert updated["tag"] == {"label": "宿敌", "note": "", "color": "rose"}

    deleted = asyncio.run(league_lab.delete_league_player_tag("owner-b::target"))
    assert deleted == {"deleted": True, "key": "owner-b::target"}
    assert asyncio.run(league_lab.list_league_player_tags(current_account_only=True))["total"] == 0


def test_player_tag_import_uses_current_account_and_paginates(monkeypatch, tmp_path):
    monkeypatch.setattr(league_lab, "_player_tags_path", lambda: tmp_path / "league-player-tags.json")
    monkeypatch.setattr(league_lab.league_lab_service, "current_summoner", {"puuid": "owner"})
    body = league_lab.PlayerTagsImportBody(rows=[
        league_lab.PlayerTagImportRow(puuid="one", label="Alpha"),
        league_lab.PlayerTagImportRow(puuid="two", label="Beta"),
    ])
    result = asyncio.run(league_lab.import_league_player_tags(body))
    assert result == {"imported": 2, "total": 2}
    page = asyncio.run(league_lab.list_league_player_tags(page=1, page_size=1, query="beta"))
    assert page["total"] == 1
    assert page["rows"][0]["puuid"] == "two"


def test_player_search_history_is_account_scoped_and_manageable(monkeypatch, tmp_path):
    monkeypatch.setattr(league_lab, "_player_search_history_path", lambda: tmp_path / "search-history.json")
    monkeypatch.setattr(league_lab.league_lab_service, "current_summoner", {"puuid": "owner-a"})
    league_lab._remember_player_search({
        "server_id": "hn1",
        "summoner": {"puuid": "target", "game_name": "Target", "tag_line": "CN1", "profile_icon_id": 12},
    })

    first = asyncio.run(league_lab.league_player_search_history())
    assert first["count"] == 1
    assert first["players"][0]["game_name"] == "Target"
    assert "owner_puuid" not in first["players"][0]

    pinned = asyncio.run(league_lab.pin_league_player_search_history(
        "target", league_lab.SearchHistoryPinBody(pinned=True), server_id="hn1"
    ))
    assert pinned["pinned"] is True
    assert asyncio.run(league_lab.league_player_search_history())["players"][0]["pinned"] is True

    monkeypatch.setattr(league_lab.league_lab_service, "current_summoner", {"puuid": "owner-b"})
    assert asyncio.run(league_lab.league_player_search_history())["count"] == 0
    monkeypatch.setattr(league_lab.league_lab_service, "current_summoner", {"puuid": "owner-a"})
    removed = asyncio.run(league_lab.delete_league_player_search_history("target", server_id="hn1"))
    assert removed["removed"] is True
    assert asyncio.run(league_lab.league_player_search_history())["count"] == 0


def test_friend_search_surface_hides_spectator_key_and_revalidates_launch(monkeypatch):
    friend_payload = [{
        "puuid": "friend-1",
        "gameName": "Friend",
        "gameTag": "CN1",
        "icon": 22,
        "availability": "dnd",
        "lol": {"gameStatus": "inGame", "spectatorKey": "private-spectator-key"},
    }]
    calls = []
    monkeypatch.setattr(
        league_lab.league_lab_service,
        "settings",
        LeagueLabSettings(toolkit_account_actions_enabled=True),
    )

    async def request(method, path, *, json_body=None, params=None):
        calls.append((method, path, json_body))
        if (method, path) == ("GET", "/lol-chat/v1/friends"):
            return friend_payload
        if (method, path) == ("GET", "/lol-gameflow/v1/gameflow-phase"):
            return "Lobby"
        if (method, path) == ("POST", "/lol-spectator/v1/spectate/launch"):
            return None
        raise AssertionError(f"unexpected LCU request: {method} {path}")

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    result = asyncio.run(league_lab.league_player_friends())
    assert result == {"friends": [{
        "puuid": "friend-1",
        "game_name": "Friend",
        "tag_line": "CN1",
        "profile_icon_id": 22,
        "availability": "dnd",
        "game_status": "inGame",
        "spectatable": True,
    }], "count": 1}
    assert "private-spectator-key" not in json.dumps(result)

    launched = asyncio.run(league_lab.spectate_league_friend("friend-1"))
    assert launched == {"puuid": "friend-1", "launched": True}
    assert calls[-1] == (
        "POST",
        "/lol-spectator/v1/spectate/launch",
        {"puuid": "friend-1", "spectatorKey": "private-spectator-key"},
    )


def test_parse_league_client_command_line_extracts_lcu_credentials():
    parsed = parse_league_client_command_line(
        '"LeagueClientUx.exe" --app-port=54321 --remoting-auth-token=secret_token '
        '--region=CN --rso_platform_id=HN1 --app-pid=1234 '
        '--riotclient-app-port=60001 --riotclient-auth-token=riot_secret'
    )

    assert parsed is not None
    assert parsed.port == 54321
    assert parsed.token == "secret_token"
    assert parsed.region == "CN"
    assert parsed.platform_id == "HN1"
    assert parsed.riot_client_port == 60001
    assert parsed.riot_client_token == "riot_secret"
    assert parsed.pid == 0
    assert "secret_token" not in parsed.base_url
    assert "riot_secret" not in parsed.riot_client_base_url


def test_parse_league_client_command_line_rejects_incomplete_input():
    assert parse_league_client_command_line("--app-port=54321") is None


def test_respawn_timer_reads_local_live_client_data(monkeypatch):
    service = LeagueLabService()
    service.settings = LeagueLabSettings(respawn_timer_enabled=True)
    service.phase = "InProgress"
    service.current_summoner = {"game_name": "Tester", "tag_line": "CN1"}

    class FakeResponse:
        def raise_for_status(self):
            return None

        def json(self):
            return [{"riotId": "Tester#CN1", "isDead": True, "respawnTimer": 12.4}]

    class FakeClient:
        def __init__(self, **kwargs):
            pass

        async def __aenter__(self):
            return self

        async def __aexit__(self, *args):
            return False

        async def get(self, url):
            assert url == "https://127.0.0.1:2999/liveclientdata/playerlist"
            return FakeResponse()

    monkeypatch.setattr(league_lab.httpx, "AsyncClient", FakeClient)
    asyncio.run(service._refresh_respawn_timer())

    assert service.respawn_timer == {"available": True, "dead": True, "time_left": 12.4, "total_time": 12.4}


def test_respawn_timer_is_off_by_default():
    service = LeagueLabService()
    service.phase = "InProgress"
    asyncio.run(service._refresh_respawn_timer())
    assert service.respawn_timer["available"] is False


def test_discovery_uses_thread_compatible_subprocess(monkeypatch):
    command = b"1234\r\n"

    def fake_run(*args, **kwargs):
        return subprocess.CompletedProcess(args[0], 0, stdout=command)

    monkeypatch.setattr(league_lab.os, "name", "nt")
    monkeypatch.setattr(league_lab.subprocess, "run", fake_run)
    monkeypatch.setattr(
        league_lab,
        "_read_windows_process_command_line",
        lambda pid: 'LeagueClientUx.exe --app-port=54321 --remoting-auth-token=memory_only' if pid == 1234 else "",
    )
    parsed = asyncio.run(league_lab.discover_lcu_credentials())
    assert parsed is not None
    assert parsed.port == 54321
    assert parsed.token == "memory_only"
    assert parsed.pid == 1234


def test_discovery_returns_every_running_league_client_without_tokens_leaking(monkeypatch):
    def fake_run(*args, **kwargs):
        return subprocess.CompletedProcess(args[0], 0, stdout=b"1234\r\n5678\r\n")

    monkeypatch.setattr(league_lab.os, "name", "nt")
    monkeypatch.setattr(league_lab.subprocess, "run", fake_run)
    monkeypatch.setattr(
        league_lab,
        "_read_windows_process_command_line",
        lambda pid: f"LeagueClientUx.exe --app-port={pid} --remoting-auth-token=memory_{pid}",
    )

    clients = asyncio.run(league_lab.discover_lcu_clients())

    assert [(client.pid, client.port) for client in clients] == [(1234, 1234), (5678, 5678)]


def test_discovery_falls_back_to_cim_when_native_command_line_is_denied(monkeypatch):
    calls = 0

    def fake_run(*args, **kwargs):
        nonlocal calls
        calls += 1
        if calls == 1:
            return subprocess.CompletedProcess(args[0], 0, stdout=b"4321\r\n")
        payload = json.dumps({
            "ProcessId": 4321,
            "CommandLine": "LeagueClientUx.exe --app-port=54321 --remoting-auth-token=cim_memory_only",
        }).encode()
        return subprocess.CompletedProcess(args[0], 0, stdout=payload)

    monkeypatch.setattr(league_lab.os, "name", "nt")
    monkeypatch.setattr(league_lab.subprocess, "run", fake_run)
    monkeypatch.setattr(league_lab, "_read_windows_process_command_line", lambda _pid: "")

    clients = asyncio.run(league_lab.discover_lcu_clients())

    assert calls == 2
    assert [(client.pid, client.port, client.token) for client in clients] == [
        (4321, 54321, "cim_memory_only")
    ]


def test_status_distinguishes_elevated_client_from_missing_client(monkeypatch):
    service = LeagueLabService()
    monkeypatch.setattr(league_lab, "_league_client_window_is_present", lambda: True)

    status = service.status()

    assert status["connected"] is False
    assert status["client_window_detected"] is True
    assert status["requires_elevation"] is True


def test_client_list_identifies_accounts_without_exposing_credentials(monkeypatch):
    clients = [
        league_lab.LcuCredentials(1111, "secret-one", "CN", "HN1", pid=101),
        league_lab.LcuCredentials(2222, "secret-two", "CN", "HN2", pid=202),
    ]

    async def discover():
        return clients

    class FakeResponse:
        def __init__(self, url):
            self.url = url
            self.content = b"{}"

        def raise_for_status(self):
            return None

        def json(self):
            if self.url.endswith("current-summoner"):
                return {"gameName": f"Player-{self.url.split(':')[2].split('/')[0]}", "tagLine": "CN1"}
            return "Lobby"

    class FakeClient:
        def __init__(self, **kwargs):
            pass

        async def __aenter__(self):
            return self

        async def __aexit__(self, *args):
            return False

        async def get(self, url, **kwargs):
            assert kwargs.get("headers", {}).get("Authorization", "").startswith("Basic ")
            return FakeResponse(url)

    monkeypatch.setattr(league_lab, "discover_lcu_clients", discover)
    monkeypatch.setattr(league_lab.httpx, "AsyncClient", FakeClient)
    result = asyncio.run(league_lab.league_clients())

    assert [row["pid"] for row in result["clients"]] == [101, 202]
    serialized = json.dumps(result)
    assert "secret-one" not in serialized
    assert "secret-two" not in serialized


def test_select_client_uses_exact_running_pid(monkeypatch):
    service = LeagueLabService()
    clients = [
        league_lab.LcuCredentials(1111, "one", pid=101),
        league_lab.LcuCredentials(2222, "two", pid=202),
    ]

    async def discover():
        return clients

    async def refresh_state():
        service.summoner_name = "Selected"

    monkeypatch.setattr(league_lab, "discover_lcu_clients", discover)
    monkeypatch.setattr(service, "_replace_credentials", lambda value: setattr(service, "credentials", value))
    monkeypatch.setattr(service, "_refresh_state", refresh_state)

    asyncio.run(service.select_client(202))

    assert service.credentials == clients[1]
    assert service._selected_client_pid == 202
    assert service.summoner_name == "Selected"


def test_detected_riot_client_launcher_uses_argument_array_without_shell(tmp_path, monkeypatch):
    executable = tmp_path / "RiotClientServices.exe"
    executable.write_bytes(b"fixture")

    async def installations():
        return {
            "riot": {
                "kind": "riot",
                "label": "Riot Client",
                "path": str(executable),
                "args": ["--launch-product=league_of_legends", "--launch-patchline=live"],
            }
        }

    captured = {}

    class FakeProcess:
        def poll(self):
            return None

    def popen(args, **kwargs):
        captured["args"] = args
        captured["kwargs"] = kwargs
        return FakeProcess()

    monkeypatch.setattr(league_lab, "detect_client_installations", installations)
    monkeypatch.setattr(league_lab.subprocess, "Popen", popen)
    result = asyncio.run(league_lab.launch_detected_client("riot"))

    assert result == {"started": True, "kind": "riot", "label": "Riot Client"}
    assert captured["args"] == [
        str(executable.resolve()),
        "--launch-product=league_of_legends",
        "--launch-patchline=live",
    ]
    assert "shell" not in captured["kwargs"]


def test_detected_tencent_client_launcher_uses_windows_shell_execute(tmp_path, monkeypatch):
    executable = tmp_path / "Tencent League" / "Launcher" / "Client.exe"
    executable.parent.mkdir(parents=True)
    executable.write_bytes(b"fixture")

    async def installations():
        return {
            "tcls": {
                "kind": "tcls",
                "label": "Tencent TCLS",
                "path": str(executable),
                "args": [],
            }
        }

    captured = {}

    def shell_execute(path, args):
        captured["path"] = path
        captured["args"] = args

    monkeypatch.setattr(league_lab, "detect_client_installations", installations)
    monkeypatch.setattr(league_lab, "_shell_execute_windows", shell_execute)
    result = asyncio.run(league_lab.launch_detected_client("tcls"))

    assert result == {"started": True, "kind": "tcls", "label": "Tencent TCLS"}
    assert captured == {"path": str(executable.resolve()), "args": []}


def test_replay_download_prepares_metadata_then_starts_rofl_download(monkeypatch):
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        calls.append((method, path, json_body))
        if path == "/lol-replays/v1/configuration":
            return {"isReplaysEnabled": True, "gameVersion": "15.16.1"}
        return None

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    result = asyncio.run(league_lab.download_league_replay(
        123456,
        league_lab.LeagueReplayPrepare(game_type="MATCHED_GAME", queue_id=420, game_end=999999),
    ))

    assert result == {"game_id": 123456, "state": "downloading"}
    assert calls == [
        ("GET", "/lol-replays/v1/configuration", None),
        (
            "POST",
            "/lol-replays/v2/metadata/123456/create",
            {"gameVersion": "15.16.1", "gameType": "MATCHED_GAME", "queueId": 420, "gameEnd": 999999},
        ),
        (
            "POST",
            "/lol-replays/v1/rofls/123456/download",
            {"componentType": "replay-button_match-history"},
        ),
    ]


def test_replay_watch_is_manual_and_uses_league_replay_endpoint(monkeypatch):
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        calls.append((method, path, json_body))
        return None

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    result = asyncio.run(league_lab.watch_league_replay(123456))

    assert result == {"game_id": 123456, "state": "watching"}
    assert calls == [
        ("POST", "/lol-replays/v1/rofls/123456/watch", {"componentType": "replay-button_match-history"}),
    ]


def test_full_champion_mastery_is_named_and_sorted(monkeypatch):
    async def request(method, path, *, json_body=None, params=None):
        assert path == "/lol-champion-mastery/v1/player-puuid/champion-mastery"
        return [
            {"championId": 2, "championPoints": 500},
            {"championId": 1, "championPoints": 1200},
        ]

    async def names():
        return {1: "Annie", 2: "Olaf"}

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    monkeypatch.setattr(league_lab, "_champion_names", names)
    result = asyncio.run(league_lab.league_player_mastery("player-puuid"))

    assert [(row["championName"], row["championPoints"]) for row in result["mastery"]] == [
        ("Annie", 1200),
        ("Olaf", 500),
    ]


def test_encountered_games_are_indexed_per_local_account_and_removable(tmp_path, monkeypatch):
    path = tmp_path / "recent.json"
    monkeypatch.setattr(league_lab, "_recent_players_path", lambda: path)
    monkeypatch.setattr(league_lab.league_lab_service, "current_summoner", {"puuid": "self-player"})
    match = {
        "game_id": 9876,
        "played_at": 123456,
        "game_mode": "CLASSIC",
        "queue_id": 420,
        "participants": [
            {"puuid": "self-player", "champion_id": 1, "kills": 10, "deaths": 2, "assists": 8, "win": True},
            {"puuid": "target-player", "champion_id": 2, "kills": 3, "deaths": 7, "assists": 4, "win": False},
        ],
    }

    league_lab._index_match_encounters([match], "self-player")
    page = asyncio.run(league_lab.league_player_encounters("target-player"))

    assert page["total"] == 1
    assert page["games"][0]["target"]["kills"] == 3
    assert page["games"][0]["self"]["kills"] == 10

    removed = asyncio.run(league_lab.delete_league_player_encounter("target-player", "9876"))
    assert removed["removed"] is True
    assert asyncio.run(league_lab.league_player_encounters("target-player"))["total"] == 0


def test_settings_are_persisted_without_lcu_credentials(tmp_path, monkeypatch):
    monkeypatch.setattr(LeagueLabService, "_settings_path", staticmethod(lambda: tmp_path / "league-lab.json"))
    service = LeagueLabService()
    service.credentials = None
    updated = service.update_settings(
        LeagueLabSettings(safety_migration_version=1, automation_enabled=True, auto_accept_enabled=True, invitation_strategy="accept")
    )

    content = (tmp_path / "league-lab.json").read_text(encoding="utf-8")
    assert updated.auto_accept_enabled is True
    assert "secret" not in content.lower()
    assert LeagueLabService().settings.invitation_strategy == "accept"


def test_legacy_settings_are_safely_disabled_once_on_upgrade(tmp_path, monkeypatch):
    path = tmp_path / "league-lab.json"
    legacy = LeagueLabSettings(
        automation_enabled=True,
        auto_accept_enabled=True,
        auto_select_enabled=True,
        auto_honor_enabled=True,
        toolkit_account_actions_enabled=True,
        in_game_send_enabled=True,
    ).model_dump()
    # A pre-v1 settings file must take the full safety migration path.
    legacy["safety_migration_version"] = 0
    legacy["opgg_auto_apply_runes"] = True
    legacy["opgg_window_enabled"] = True
    legacy["auto_select_profiles"]["aram"]["pick"]["enabled"] = True
    legacy["auto_select_profiles"]["aram"]["pick"]["bench_handle_trade_enabled"] = True
    legacy["auto_select_profiles"]["aram"]["ban"]["enabled"] = True
    path.write_text(json.dumps(legacy), encoding="utf-8")
    monkeypatch.setattr(LeagueLabService, "_settings_path", staticmethod(lambda: path))

    migrated = LeagueLabService().settings

    assert migrated.safety_migration_version == 2
    assert migrated.automation_enabled is False
    assert migrated.auto_accept_enabled is False
    assert migrated.auto_select_enabled is False
    assert migrated.auto_honor_enabled is False
    assert migrated.toolkit_account_actions_enabled is True
    assert migrated.in_game_send_enabled is False
    assert not hasattr(migrated, "opgg_auto_apply_runes")
    assert migrated.auto_select_profiles["aram"].pick.enabled is False
    assert migrated.auto_select_profiles["aram"].pick.bench_handle_trade_enabled is False
    assert migrated.auto_select_profiles["aram"].ban.enabled is False
    persisted = json.loads(path.read_text(encoding="utf-8"))
    assert persisted["safety_migration_version"] == 2
    assert persisted["automation_enabled"] is False
    assert not any(key.startswith("opgg_") for key in persisted)


def test_completed_safety_migration_preserves_later_explicit_opt_in(tmp_path, monkeypatch):
    path = tmp_path / "league-lab.json"
    opted_in = LeagueLabSettings(
        safety_migration_version=1,
        automation_enabled=True,
        auto_accept_enabled=True,
    )
    path.write_text(opted_in.model_dump_json(), encoding="utf-8")
    monkeypatch.setattr(LeagueLabService, "_settings_path", staticmethod(lambda: path))

    loaded = LeagueLabService().settings

    assert loaded.safety_migration_version == 2
    assert loaded.automation_enabled is True
    assert loaded.auto_accept_enabled is True
    assert loaded.toolkit_account_actions_enabled is True


def test_new_install_defaults_keep_automation_off_but_enable_account_gate(tmp_path, monkeypatch):
    path = tmp_path / "league-lab.json"
    monkeypatch.setattr(LeagueLabService, "_settings_path", staticmethod(lambda: path))

    settings = LeagueLabService().settings

    assert settings.automation_enabled is False
    assert settings.auto_accept_enabled is False
    assert settings.auto_select_enabled is False
    assert settings.auto_honor_enabled is False
    assert settings.toolkit_account_actions_enabled is True
    assert settings.in_game_send_enabled is False
    assert settings.terminate_game_shortcut_enabled is False
    assert not path.exists()


def test_champion_config_prefers_ranked_position_loadout():
    service = LeagueLabService()
    service.phase = "ChampSelect"
    service.settings = LeagueLabSettings(
        automation_enabled=True,
        auto_champion_config_enabled=True,
        champion_loadouts=[
        {"champion_id": 22, "config_key": "default", "primary_style_id": 8000, "sub_style_id": 8100, "selected_perk_ids": [8005], "spell1_id": 4, "spell2_id": 7},
        {"champion_id": 22, "config_key": "ranked-jungle", "primary_style_id": 8100, "sub_style_id": 8300, "selected_perk_ids": [8112], "spell1_id": 11, "spell2_id": 4},
        ],
    )
    writes = []

    async def fake_request(method, path, *, json_body=None):
        if path == "/lol-champ-select/v1/current-champion": return 22
        if path == "/lol-champ-select/v1/session": return {"localPlayerCellId": 1, "myTeam": [{"cellId": 1, "assignedPosition": "JUNGLE"}]}
        if path == "/lol-gameflow/v1/session": return {"gameData": {"queue": {"gameMode": "CLASSIC", "type": "RANKED_SOLO_5x5"}}}
        if path == "/lol-perks/v1/pages": return [{"id": 9, "isEditable": True}]
        writes.append((method, path, json_body))
        return {}

    service.request = fake_request
    asyncio.run(service._run_champion_config())
    assert ("PATCH", "/lol-champ-select/v1/session/my-selection", {"spell1Id": 11, "spell2Id": 4}) in writes
    rune_write = next(body for method, path, body in writes if method == "PUT" and path == "/lol-perks/v1/pages/9")
    assert rune_write["name"] == "[Insight] Champion 22 - ranked-jungle"
    assert rune_write["selectedPerkIds"] == [8112]


def test_champion_config_accepts_leagueakari_rune_v2_and_spell_pages():
    service = LeagueLabService()
    service.phase = "ChampSelect"
    service.settings = LeagueLabSettings.model_validate({
        "automation_enabled": True,
        "auto_champion_config_enabled": True,
        "runesV2": {
            "22": {
                "ranked-jungle": {
                    "primaryStyleId": 8100,
                    "subStyleId": 8300,
                    "selectedPerkIds": [8112, 8126, 8138, 8106, 8347, 8304, 5005, 5008, 5001],
                }
            }
        },
        "summonerSpells": {"22": {"ranked-jungle": {"spell1Id": 11, "spell2Id": 4}}},
    })
    writes = []

    async def fake_request(method, path, *, json_body=None):
        if path == "/lol-champ-select/v1/current-champion":
            return 22
        if path == "/lol-champ-select/v1/session":
            return {"localPlayerCellId": 1, "myTeam": [{"cellId": 1, "assignedPosition": "JUNGLE"}]}
        if path == "/lol-gameflow/v1/session":
            return {"gameData": {"queue": {"gameMode": "CLASSIC", "type": "RANKED_SOLO_5x5"}}}
        if path == "/lol-perks/v1/pages":
            return [{"id": 9, "isEditable": True}]
        writes.append((method, path, json_body))
        return {}

    service.request = fake_request
    asyncio.run(service._run_champion_config())
    assert ("PATCH", "/lol-champ-select/v1/session/my-selection", {"spell1Id": 11, "spell2Id": 4}) in writes
    rune_write = next(body for method, path, body in writes if method == "PUT" and path == "/lol-perks/v1/pages/9")
    assert rune_write["primaryStyleId"] == 8100
    assert rune_write["subStyleId"] == 8300
    assert rune_write["selectedPerkIds"][-3:] == [5005, 5008, 5001]


def test_champion_config_reports_success_in_champion_select_chat():
    service = LeagueLabService()
    service.phase = "ChampSelect"
    service.settings = LeagueLabSettings.model_validate({
        "automation_enabled": True,
        "auto_champion_config_enabled": True,
        "runesV2": {
            "22": {
                "default": {
                    "primaryStyleId": 8000,
                    "subStyleId": 8100,
                    "selectedPerkIds": [8005],
                }
            }
        },
        "summonerSpells": {"22": {"default": {"spell1Id": 4, "spell2Id": 7}}},
    })
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        calls.append((method, path, json_body))
        if path == "/lol-champ-select/v1/current-champion":
            return 22
        if path == "/lol-champ-select/v1/session":
            return {"localPlayerCellId": 1, "myTeam": [{"cellId": 1}]}
        if path == "/lol-gameflow/v1/session":
            return {"gameData": {"queue": {"gameMode": "CLASSIC", "type": "NORMAL_GAME"}}}
        if path == "/lol-perks/v1/pages":
            return [{"id": 9, "isEditable": True}]
        if path == "/lol-chat/v1/conversations":
            return [{"id": "champ-select", "type": "championSelect"}]
        return None

    service.request = request
    asyncio.run(service._run_champion_config())

    chat_posts = [
        body
        for method, path, body in calls
        if method == "POST" and path.endswith("/messages")
    ]
    assert len(chat_posts) == 2
    assert {body["type"] for body in chat_posts} == {"celebration"}
    assert any("召唤师技能已应用" in body["body"] for body in chat_posts)
    assert any("符文页已应用" in body["body"] for body in chat_posts)


def test_champion_config_reports_failure_in_champion_select_chat():
    service = LeagueLabService()
    service.phase = "ChampSelect"
    service.settings = LeagueLabSettings.model_validate({
        "automation_enabled": True,
        "auto_champion_config_enabled": True,
        "summonerSpells": {"22": {"default": {"spell1Id": 4, "spell2Id": 7}}},
    })
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        calls.append((method, path, json_body))
        if path == "/lol-champ-select/v1/current-champion":
            return 22
        if path == "/lol-champ-select/v1/session":
            return {"localPlayerCellId": 1, "myTeam": [{"cellId": 1}]}
        if path == "/lol-gameflow/v1/session":
            return {"gameData": {"queue": {"gameMode": "CLASSIC", "type": "NORMAL_GAME"}}}
        if path == "/lol-champ-select/v1/session/my-selection":
            raise RuntimeError("fixture write failed")
        if path == "/lol-chat/v1/conversations":
            return [{"id": "champ-select", "type": "championSelect"}]
        return None

    service.request = request
    asyncio.run(service._run_champion_config())

    chat_posts = [
        body
        for method, path, body in calls
        if method == "POST" and path.endswith("/messages")
    ]
    assert len(chat_posts) == 1
    assert "召唤师技能应用失败" in chat_posts[0]["body"]


def test_sgp_player_challenges_uses_league_session_service(monkeypatch):
    calls = []

    async def fake_common(method, path, *, json_body=None):
        calls.append((method, path, json_body))
        return {"playerChallenges": [{"id": 505001, "currentValue": 170}]}

    monkeypatch.setattr(league_lab, "_sgp_common_request", fake_common)
    payload = asyncio.run(league_lab._sgp_player_challenges("player-puuid"))
    assert payload["playerChallenges"][0]["currentValue"] == 170
    assert calls == [("POST", "/challenges-client/v2/all-player-data/?puuid=player-puuid", [])]


def test_ready_check_runs_auto_accept_once(monkeypatch):
    service = LeagueLabService()
    service.settings = LeagueLabSettings(
        automation_enabled=True,
        auto_accept_enabled=True,
        auto_accept_delay_seconds=0,
    )
    service.phase = "ReadyCheck"
    calls = []

    async def record(label, method, path):
        calls.append((label, method, path))

    monkeypatch.setattr(service, "_record_action", record)
    asyncio.run(service._run_automation())
    asyncio.run(service._run_automation())

    assert calls == [("已自动接受对局", "POST", "/lol-matchmaking/v1/ready-check/accept")]


def test_ready_check_gameflow_event_accepts_immediately(monkeypatch):
    """A zero-delay setting accepts directly from the LCU phase event."""
    service = LeagueLabService()
    service.settings = LeagueLabSettings(
        automation_enabled=True,
        auto_accept_enabled=True,
        auto_accept_delay_seconds=0,
    )
    calls = []

    async def record(label, method, path):
        calls.append((label, method, path))

    monkeypatch.setattr(service, "_record_action", record)
    asyncio.run(service._handle_lcu_event({
        "uri": "/lol-gameflow/v1/gameflow-phase",
        "data": "ReadyCheck",
    }))

    assert service.phase == "ReadyCheck"
    assert calls == [("已自动接受对局", "POST", "/lol-matchmaking/v1/ready-check/accept")]
    assert service._accept_due_at == float("inf")


def test_ready_check_event_invalidates_status_snapshot_without_changing_acceptance(monkeypatch):
    service = LeagueLabService()
    service.settings = LeagueLabSettings(
        automation_enabled=True,
        auto_accept_enabled=True,
        auto_accept_delay_seconds=0,
    )
    service._snapshot_cache_at = time.monotonic()
    calls = []

    async def record(label, method, path):
        calls.append((label, method, path))

    monkeypatch.setattr(service, "_record_action", record)
    asyncio.run(service._handle_lcu_event({
        "uri": "/lol-gameflow/v1/gameflow-phase",
        "data": "ReadyCheck",
    }))

    assert service._snapshot_cache_at == 0.0
    assert calls == [("已自动接受对局", "POST", "/lol-matchmaking/v1/ready-check/accept")]


def test_snapshot_single_flight_coalesces_concurrent_refreshes(monkeypatch):
    service = LeagueLabService()
    refresh_started = asyncio.Event()
    release_refresh = asyncio.Event()
    calls = []

    async def refresh_connection(*, force=False):
        calls.append(("connection", force))
        refresh_started.set()
        return True

    async def refresh_state():
        calls.append(("state",))
        await release_refresh.wait()

    monkeypatch.setattr(service, "refresh_connection", refresh_connection)
    monkeypatch.setattr(service, "_refresh_state", refresh_state)

    async def exercise():
        first = asyncio.create_task(service.snapshot())
        await refresh_started.wait()
        second = asyncio.create_task(service.snapshot())
        await asyncio.sleep(0)
        release_refresh.set()
        return await asyncio.gather(first, second)

    results = asyncio.run(exercise())
    diagnostics = results[-1]["diagnostics"]

    assert len(results) == 2
    assert calls == [("connection", True), ("state",)]
    assert diagnostics["refresh_count"] == 1
    assert diagnostics["coalesced_count"] == 1
    assert diagnostics["last_duration_ms"] is not None
    assert diagnostics["age_ms"] >= 0


def test_snapshot_ttl_skips_refresh_then_expires(monkeypatch):
    service = LeagueLabService()
    refresh_count = 0

    async def refresh_connection(*, force=False):
        nonlocal refresh_count
        refresh_count += 1
        return False

    monkeypatch.setattr(service, "refresh_connection", refresh_connection)

    async def exercise():
        first = await service.snapshot()
        second = await service.snapshot()
        service._snapshot_cache_at -= service._SNAPSHOT_CACHE_TTL_SECONDS + 0.001
        third = await service.snapshot()
        return first, second, third

    results = asyncio.run(exercise())

    assert all(isinstance(result, dict) for result in results)
    assert refresh_count == 2
    assert results[-1]["diagnostics"]["refresh_count"] == 2
    assert results[-1]["diagnostics"]["coalesced_count"] == 0


def test_snapshot_cache_invalidates_on_connection_replacement_and_settings(monkeypatch):
    service = LeagueLabService()
    service._snapshot_cache_at = time.monotonic()
    service.credentials = league_lab.LcuCredentials(port=54321, token="memory-only")

    service._replace_credentials(None)
    assert service._snapshot_cache_at == 0.0

    service._snapshot_cache_at = time.monotonic()
    monkeypatch.setattr(service, "_write_settings", lambda _settings: None)
    service.update_settings(service.settings.model_copy(update={"mini_enabled": not service.settings.mini_enabled}))

    assert service._snapshot_cache_at == 0.0


def test_successful_non_get_request_invalidates_snapshot_cache(monkeypatch):
    service = LeagueLabService()
    service.credentials = league_lab.LcuCredentials(port=54321, token="memory-only")
    service._last_discovery_at = time.monotonic()
    service._snapshot_cache_at = time.monotonic()

    class FakeResponse:
        status_code = 204
        content = b""

        def raise_for_status(self):
            return None

    class FakeClient:
        def __init__(self, **kwargs):
            pass

        async def __aenter__(self):
            return self

        async def __aexit__(self, *args):
            return False

        async def request(self, method, url, **kwargs):
            return FakeResponse()

    monkeypatch.setattr(league_lab.httpx, "AsyncClient", FakeClient)

    async def exercise():
        await service.request("GET", "/read-only")
        read_cache_at = service._snapshot_cache_at
        await service.request("POST", "/write")
        return read_cache_at

    read_cache_at = asyncio.run(exercise())

    assert read_cache_at > 0
    assert service._snapshot_cache_at == 0.0
    assert service.status()["diagnostics"]["age_ms"] is None


def test_ready_check_event_starts_real_timer_from_event(monkeypatch):
    service = LeagueLabService()
    service.settings = LeagueLabSettings(
        automation_enabled=True,
        auto_accept_enabled=True,
        auto_accept_delay_seconds=0.01,
    )
    calls = []

    async def record(label, method, path):
        calls.append((label, method, path))

    monkeypatch.setattr(service, "_record_action", record)

    async def exercise():
        await service._handle_lcu_event({
            "uri": "/lol-gameflow/v1/gameflow-phase",
            "data": "ReadyCheck",
        })
        # Await the task created by the event handler instead of racing a
        # wall-clock sleep against the test runner.  This verifies the real
        # timer lifecycle (including its cleanup) deterministically while
        # still exercising the configured non-zero delay.
        waiter = service._auto_accept_waiter
        assert waiter is not None
        await waiter

    asyncio.run(exercise())

    assert calls == [("已自动接受对局", "POST", "/lol-matchmaking/v1/ready-check/accept")]


def test_auto_accept_waiter_rechecks_an_early_windows_wake(monkeypatch):
    """An early timer wake must not drop the pending accept action."""
    service = LeagueLabService()
    service.settings = LeagueLabSettings(
        automation_enabled=True,
        auto_accept_enabled=True,
        auto_accept_delay_seconds=1,
    )
    service.phase = "ReadyCheck"
    service._accept_due_at = 10.0
    clock = [9.0]
    sleeps = []
    calls = []

    def monotonic():
        return clock[0]

    async def early_sleep(delay):
        sleeps.append(delay)
        # Model the Windows event loop waking halfway through the first wait.
        # The second wait reaches the authoritative monotonic deadline.
        clock[0] += delay / 2 if len(sleeps) == 1 else delay

    async def record(label, method, path):
        calls.append((label, method, path))

    monkeypatch.setattr(league_lab.time, "monotonic", monotonic)
    monkeypatch.setattr(league_lab.asyncio, "sleep", early_sleep)
    monkeypatch.setattr(service, "_record_action", record)

    asyncio.run(service._wait_and_auto_accept(1.0))

    assert sleeps == [1.0, 0.5]
    assert calls == [("已自动接受对局", "POST", "/lol-matchmaking/v1/ready-check/accept")]


def test_ready_check_auto_accept_requires_master_gate(monkeypatch):
    service = LeagueLabService()
    service.settings = LeagueLabSettings(
        automation_enabled=False,
        auto_accept_enabled=True,
        auto_accept_delay_seconds=0,
    )
    service.phase = "ReadyCheck"
    calls = []

    async def record(*args, **kwargs):
        calls.append((args, kwargs))

    monkeypatch.setattr(service, "_record_action", record)
    asyncio.run(service._run_automation())

    assert calls == []
    assert service._accept_due_at is None


@pytest.mark.parametrize("player_response", ["Accepted", "DECLINED"])
def test_ready_check_response_clears_pending_auto_accept_without_writing(monkeypatch, player_response):
    service = LeagueLabService()
    service.settings = LeagueLabSettings(
        automation_enabled=True,
        auto_accept_enabled=True,
        auto_accept_delay_seconds=0,
    )
    service.phase = "ReadyCheck"
    service.ready_check = {"player_response": player_response}
    service._accept_due_at = time.monotonic() - 1
    calls = []

    async def record(*args, **kwargs):
        calls.append((args, kwargs))

    monkeypatch.setattr(service, "_record_action", record)
    asyncio.run(service._run_automation())

    assert service._accept_due_at is None
    assert calls == []


def test_ready_check_exposes_mini_action_countdown():
    service = LeagueLabService()
    service.settings = LeagueLabSettings(
        automation_enabled=True,
        auto_accept_enabled=True,
        auto_accept_delay_seconds=3,
    )
    service.phase = "ReadyCheck"

    asyncio.run(service._run_automation())
    countdown = service.status()["action_countdown"]

    assert countdown["kind"] == "ready-check"
    assert countdown["label"] == "自动接受对局"
    assert 0 < countdown["remaining_seconds"] <= 3


def test_background_loop_wakes_at_pending_auto_accept_deadline(monkeypatch):
    service = LeagueLabService()
    service.settings = LeagueLabSettings(automation_enabled=True, auto_accept_enabled=True)
    service.phase = "ReadyCheck"
    service._accept_due_at = time.monotonic() + 2
    service._chat_ready_automation_done = True
    timeouts = []

    async def no_lcu_refresh():
        return True

    async def no_lcu_state_refresh():
        return None

    async def stop_after_timeout(awaitable, *, timeout):
        timeouts.append(timeout)
        awaitable.close()
        raise asyncio.CancelledError

    monkeypatch.setattr(service, "refresh_connection", no_lcu_refresh)
    monkeypatch.setattr(service, "_refresh_state", no_lcu_state_refresh)
    monkeypatch.setattr(service, "_refresh_respawn_timer", no_lcu_state_refresh)
    monkeypatch.setattr(service, "_run_automation", no_lcu_state_refresh)
    monkeypatch.setattr(league_lab.asyncio, "wait_for", stop_after_timeout)

    with pytest.raises(asyncio.CancelledError):
        asyncio.run(service._run())

    assert len(timeouts) == 1
    assert 0 < timeouts[0] < 5


def test_optional_lcu_404_preserves_discovered_credentials(monkeypatch):
    service = LeagueLabService()
    service.credentials = league_lab.LcuCredentials(port=54321, token="memory-only")
    service._last_discovery_at = time.monotonic()

    class FakeClient:
        def __init__(self, **kwargs):
            pass

        async def __aenter__(self):
            return self

        async def __aexit__(self, *args):
            return False

        async def request(self, method, url, **kwargs):
            return httpx.Response(404, request=httpx.Request(method, url))

    monkeypatch.setattr(league_lab.httpx, "AsyncClient", FakeClient)

    try:
        asyncio.run(service.request("GET", "/optional-route"))
    except RuntimeError:
        pass
    else:
        raise AssertionError("404 must still surface to the optional-route caller")

    assert service.credentials is not None
    assert service.credentials.token == "memory-only"


def test_play_again_waits_for_phase_buffer(monkeypatch):
    service = LeagueLabService()
    service.settings = LeagueLabSettings(automation_enabled=True, play_again_enabled=True)
    service.phase = "EndOfGame"
    calls = []

    async def record(label, method, path):
        calls.append((label, method, path))

    monkeypatch.setattr(service, "_record_action", record)
    asyncio.run(service._run_automation())
    assert calls == []

    service._phase_action_due_at = 0
    asyncio.run(service._run_automation())
    assert calls == [("已自动返回房间", "POST", "/lol-lobby/v2/play-again")]


def test_auto_select_uses_the_first_available_preference(monkeypatch):
    service = LeagueLabService()
    service.phase = "ChampSelect"
    service.settings = LeagueLabSettings(
        automation_enabled=True,
        auto_select_enabled=True,
        auto_pick_champion_ids=[157, 103],
        champion_action_delay_seconds=0,
    )
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        if path == "/lol-champ-select/v1/session":
            return {"localPlayerCellId": 2, "actions": [[{"id": 8, "actorCellId": 2, "type": "pick", "isInProgress": True}]]}
        if path == "/lol-champ-select/v1/pickable-champion-ids":
            return [103]
        if path == "/lol-gameflow/v1/session":
            return {}
        calls.append((method, path, json_body))

    monkeypatch.setattr(service, "request", request)
    asyncio.run(service._run_auto_select())
    asyncio.run(service._run_auto_select())

    assert calls == [("PATCH", "/lol-champ-select/v1/session/actions/8", {"championId": 103, "type": "pick", "completed": True})]


def test_auto_select_limits_card_mode_pick_to_server_subset(monkeypatch):
    service = LeagueLabService()
    service.phase = "ChampSelect"
    profile = league_lab.AutoSelectProfile(
        pick={"enabled": True, "champions": {"default": [103, 157]}, "delay_seconds": 0, "strategy": "lock-in-immediately"}
    )
    service.settings = LeagueLabSettings(automation_enabled=True, auto_select_enabled=True, auto_select_profiles={"aram": profile})
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        if path == "/lol-champ-select/v1/session":
            return {"localPlayerCellId": 2, "benchEnabled": True, "allowSubsetChampionPicks": True, "actions": [[{"id": 8, "actorCellId": 2, "type": "pick", "isInProgress": True}]]}
        if path == "/lol-gameflow/v1/session": return {"gameData": {"queue": {"id": 450, "gameMode": "ARAM"}}}
        if path == "/lol-lobby-team-builder/champ-select/v1/subset-champion-list": return [157]
        if path == "/lol-champ-select/v1/pickable-champion-ids": return [103, 157]
        calls.append((method, path, json_body))

    monkeypatch.setattr(service, "request", request)
    asyncio.run(service._run_auto_select())

    assert calls == [("PATCH", "/lol-champ-select/v1/session/actions/8", {"championId": 157, "type": "pick", "completed": True})]


def test_auto_select_supports_cherry_bravery_special_action(monkeypatch):
    service = LeagueLabService()
    service.phase = "ChampSelect"
    profile = league_lab.AutoSelectProfile(
        pick={"enabled": True, "champions": {"default": [-3]}, "delay_seconds": 0, "strategy": "lock-in-immediately"}
    )
    service.settings = LeagueLabSettings(automation_enabled=True, auto_select_enabled=True, auto_select_profiles={"cherry": profile})
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        if path == "/lol-champ-select/v1/session": return {"localPlayerCellId": 2, "actions": [[{"id": 9, "actorCellId": 2, "type": "pick", "isInProgress": True}]]}
        if path == "/lol-gameflow/v1/session": return {"gameData": {"queue": {"id": 1700, "gameMode": "CHERRY"}}}
        if path == "/lol-champ-select/v1/pickable-champion-ids": return []
        calls.append((method, path, json_body))

    monkeypatch.setattr(service, "request", request)
    asyncio.run(service._run_auto_select())

    assert calls == [("PATCH", "/lol-champ-select/v1/session/actions/9", {"championId": -3, "type": "pick", "completed": True})]


def test_mode_group_matches_league_queue_families():
    assert LeagueLabService._mode_group({"gameData": {"queue": {"id": 420, "gameMode": "CLASSIC", "type": "RANKED_SOLO_5x5"}}}) == "ranked"
    assert LeagueLabService._mode_group({"gameData": {"queue": {"id": 450, "gameMode": "ARAM"}}}) == "aram"
    assert LeagueLabService._mode_group({"gameData": {"queue": {"id": 1700, "gameMode": "CHERRY"}}}) == "cherry"
    assert LeagueLabService._mode_group({"gameData": {"queue": {"id": 430, "gameMode": "CLASSIC", "type": "NORMAL"}}}) == "normal"
    assert LeagueLabService._mode_group({"gameData": {"queue": {"id": 1020, "gameMode": "ONEFORALL"}}}) == "oneforall"
    assert LeagueLabService._mode_group({"gameData": {"queue": {"id": 1400, "gameMode": "ULTBOOK"}}}) == "ultbook"
    assert LeagueLabService._mode_group({"gameData": {"queue": {"id": 830, "gameMode": "CLASSIC", "type": "BOT"}}}) == "bot"
    assert LeagueLabService._mode_group({"gameData": {"isCustomGame": True}}) == "custom"


def test_auto_select_profiles_migrate_legacy_mode_keys_and_fill_current_groups():
    arena = {"pick": {"enabled": True, "champions": {"default": [-3]}}}
    doom_bots = {"ban": {"enabled": True, "champions": {"default": [17]}}}
    settings = LeagueLabSettings(auto_select_profiles={"arena": arena, "doom-bots": doom_bots})

    assert settings.auto_select_profiles["cherry"].pick.champions["default"] == [-3]
    assert settings.auto_select_profiles["ultbook"].ban.champions["default"] == [17]
    assert {"ranked", "normal", "aram", "cherry", "urf", "oneforall", "ultbook", "bot", "custom", "default"} <= set(settings.auto_select_profiles)


def test_bench_swap_never_downgrades_a_configured_current_champion(monkeypatch):
    service = LeagueLabService()
    service.phase = "ChampSelect"
    service.settings = LeagueLabSettings(automation_enabled=True, auto_select_enabled=True)
    profile = league_lab.PickProfile(
        enabled=True,
        champions={"default": [22, 34]},
        bench_select_first_available_champion=True,
        bench_swap_accumulated_delay_seconds=0,
    )
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        calls.append((method, path))

    monkeypatch.setattr(service, "request", request)
    asyncio.run(service._run_bench_swap(
        {"benchChampions": [{"championId": 34}]},
        profile,
        "default",
        normalized={"current_session_champion_id": 22, "auto_select_move": "bench-swap"},
    ))

    assert calls == []


def test_bench_swap_upgrades_to_a_higher_priority_champion(monkeypatch):
    service = LeagueLabService()
    service.phase = "ChampSelect"
    service.settings = LeagueLabSettings(automation_enabled=True, auto_select_enabled=True)
    profile = league_lab.PickProfile(
        enabled=True,
        champions={"default": [22, 34]},
        bench_select_first_available_champion=True,
        bench_swap_accumulated_delay_seconds=0,
    )
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        calls.append((method, path))

    monkeypatch.setattr(service, "request", request)
    asyncio.run(service._run_bench_swap(
        {"benchChampions": [{"championId": 22}]},
        profile,
        "default",
        normalized={"current_session_champion_id": 34, "auto_select_move": "bench-swap"},
    ))

    assert calls == [("POST", "/lol-champ-select/v1/session/bench/swap/22")]


def test_normalized_champ_select_exposes_mini_bench_state():
    normalized = LeagueLabService._normalize_champ_select({
        "localPlayerCellId": 2,
        "myTeam": [{"cellId": 2, "championId": 22}],
        "benchEnabled": True,
        "benchChampions": [{"championId": 12}, {"championId": 34}],
        "rerollsRemaining": 1,
        "allowRerolling": True,
        "actions": [[{"id": 7, "actorCellId": 2, "type": "pick", "championId": 22, "isInProgress": True}]],
        "timer": {"phase": "FINALIZATION", "adjustedTimeLeftInPhase": 9000},
    })
    assert normalized["current_champion_id"] == 22
    assert normalized["bench_champions"] == [12, 34]
    assert normalized["rerolls_remaining"] == 1
    assert normalized["timer_phase"] == "FINALIZATION"
    assert normalized["timer_deadline_at"] > 0
    assert normalized["my_actions"] == [{"id": 7, "type": "pick", "champion_id": 22, "completed": False, "in_progress": True}]


def test_normalized_champ_select_requires_boolean_reroll_support_evidence():
    normalized = LeagueLabService._normalize_champ_select({
        "benchEnabled": True,
        "rerollsRemaining": 1,
        "allowRerolling": "true",
    })

    assert normalized["rerolls_remaining"] == 1
    assert normalized["allow_rerolling"] is False


def test_auto_select_temporary_disable_blocks_all_client_calls(monkeypatch):
    service = LeagueLabService()
    service.phase = "ChampSelect"
    service.auto_select_temporarily_disabled = True

    async def request(*args, **kwargs):
        raise AssertionError("temporary disable must return before touching LCU")

    monkeypatch.setattr(service, "request", request)
    asyncio.run(service._run_auto_select())


def test_auto_select_delay_is_non_blocking_and_visible(monkeypatch):
    service = LeagueLabService()
    service.phase = "ChampSelect"
    service.settings = LeagueLabSettings(
        automation_enabled=True,
        auto_select_enabled=True,
        auto_pick_champion_ids=[103],
        champion_action_delay_seconds=3,
    )

    async def request(method, path, *, json_body=None, params=None):
        if path == "/lol-champ-select/v1/session":
            return {"localPlayerCellId": 2, "actions": [[{"id": 8, "actorCellId": 2, "type": "pick", "isInProgress": True}]]}
        if path == "/lol-champ-select/v1/pickable-champion-ids":
            return [103]
        if path == "/lol-gameflow/v1/session":
            return {}
        raise AssertionError("the delayed lock-in must not run yet")

    monkeypatch.setattr(service, "request", request)
    asyncio.run(service._run_auto_select())

    countdown = service.status()["action_countdown"]
    assert countdown["kind"] == "champion-action"
    assert 0 < countdown["remaining_seconds"] <= 3


def _auto_select_state_fixture(*, action_type="pick", champion_id=0, timer_phase="BAN_PICK", subset=False, bench=False):
    completed = bool(bench and champion_id)
    return {
        "localPlayerCellId": 2,
        "myTeam": [{"cellId": 2, "championId": 103}],
        "allowSubsetChampionPicks": subset,
        "benchEnabled": bench,
        "benchChampions": [{"championId": 157}] if bench else [],
        "actions": [[{
            "id": 8,
            "actorCellId": 2,
            "type": action_type,
            "championId": champion_id,
            "completed": completed,
            "isInProgress": not completed,
        }]],
        "timer": {"phase": timer_phase, "adjustedTimeLeftInPhase": 5000},
    }


@pytest.mark.parametrize(
    ("kwargs", "expected_move"),
    [
        ({"action_type": "pick", "timer_phase": "PLANNING"}, "pick-intent"),
        ({"action_type": "pick"}, "show-pick"),
        ({"action_type": "pick", "champion_id": 103}, "complete-pick"),
        ({"action_type": "ban"}, "show-ban"),
        ({"action_type": "ban", "champion_id": 103}, "complete-ban"),
        ({"action_type": "vote"}, "vote"),
        ({"action_type": "pick", "subset": True}, "show-subset-pick"),
        ({"action_type": "pick", "subset": True, "champion_id": 103}, "complete-subset-pick"),
        ({"action_type": "pick", "champion_id": 103, "subset": True, "bench": True}, "subset-bench-swap"),
        ({"action_type": "pick", "champion_id": 103, "bench": True}, "bench-swap"),
    ],
)
def test_normalized_auto_select_move_and_current_ids(kwargs, expected_move):
    normalized = LeagueLabService._normalize_champ_select(
        _auto_select_state_fixture(**kwargs),
        pickable_ids=[103, 157],
        bannable_ids=[103, 157],
        subset_ids=[157],
    )
    assert normalized["auto_select_move"] == expected_move
    assert normalized["current_pickable_champion_ids"] == [103, 157]
    assert normalized["current_bannable_champion_ids"] == [103, 157]
    assert normalized["current_pickable_ids_available"] is True
    assert normalized["current_bannable_ids_available"] is True


@pytest.mark.parametrize(
    ("settings", "phase"),
    [
        (LeagueLabSettings(auto_select_enabled=True), "ChampSelect"),
        (LeagueLabSettings(automation_enabled=True), "ChampSelect"),
        (LeagueLabSettings(automation_enabled=True, auto_select_enabled=True), "Lobby"),
    ],
)
def test_auto_select_safety_gates_stop_before_session_request(monkeypatch, settings, phase):
    service = LeagueLabService()
    service.settings = settings
    service.phase = phase
    calls = []

    async def request(*args, **kwargs):
        calls.append((args, kwargs))
        raise AssertionError("auto-select gate/phase must stop before LCU access")

    monkeypatch.setattr(service, "request", request)
    asyncio.run(service._run_auto_select())
    assert calls == []


def test_auto_select_rejects_unavailable_candidate_without_write(monkeypatch):
    service = LeagueLabService()
    service.phase = "ChampSelect"
    service.settings = LeagueLabSettings(
        automation_enabled=True,
        auto_select_enabled=True,
        auto_pick_champion_ids=[103],
        champion_action_delay_seconds=0,
    )
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        if path == "/lol-champ-select/v1/session":
            return _auto_select_state_fixture()
        if path == "/lol-gameflow/v1/session":
            return {}
        if path == "/lol-champ-select/v1/pickable-champion-ids":
            return [157]
        calls.append((method, path, json_body))

    monkeypatch.setattr(service, "request", request)
    asyncio.run(service._run_auto_select())
    assert calls == []


def test_auto_select_pick_intent_uses_intent_payload_and_delayed_plan(monkeypatch):
    service = LeagueLabService()
    service.phase = "ChampSelect"
    profile = league_lab.AutoSelectProfile(
        pick={
            "enabled": True,
            "champions": {"default": [103]},
            "delay_seconds": 2,
            "show_intent": True,
            "strategy": "show-and-lock-in",
        }
    )
    service.settings = LeagueLabSettings(
        automation_enabled=True,
        auto_select_enabled=True,
        auto_select_profiles={"default": profile},
    )
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        if path == "/lol-champ-select/v1/session":
            return _auto_select_state_fixture(timer_phase="PLANNING")
        if path == "/lol-gameflow/v1/session":
            return {}
        if path == "/lol-champ-select/v1/pickable-champion-ids":
            return [103]
        calls.append((method, path, json_body))

    monkeypatch.setattr(service, "request", request)
    asyncio.run(service._run_auto_select())
    assert calls == []
    plan = service.status()["delayed_action_plan"]
    assert plan and plan[0]["move"] == "pick-intent"
    assert plan[0]["completed"] is False
    service._champion_action_due_at["pick:8"] = 0
    asyncio.run(service._run_auto_select())
    assert calls == [("PATCH", "/lol-champ-select/v1/session/actions/8", {"championId": 103})]


def test_auto_select_automatic_trade_uses_champion_swaps_endpoint(monkeypatch):
    service = LeagueLabService()
    service.phase = "ChampSelect"
    service.settings = LeagueLabSettings(automation_enabled=True, auto_select_enabled=True)
    service.ongoing_champion_swap = {
        "id": 7,
        "state": "RECEIVED",
        "requesterChampionId": 103,
    }
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        calls.append((method, path, json_body))
        return None

    monkeypatch.setattr(service, "request", request)
    asyncio.run(service._run_trade_handling(
        {"trades": []}, [103, 157], league_lab.PickProfile(
            enabled=True,
            delay_seconds=0,
            bench_handle_trade_enabled=True,
            bench_select_first_available_champion=True,
        ), {"current_session_champion_id": 157}
    ))
    assert calls == [("POST", "/lol-champ-select/v1/session/champion-swaps/7/accept", None)]


def test_auto_select_trade_delay_subtracts_elapsed_received_time(monkeypatch):
    service = LeagueLabService()
    service.phase = "ChampSelect"
    service.settings = LeagueLabSettings(automation_enabled=True, auto_select_enabled=True)
    service.ongoing_champion_swap = {
        "id": 7,
        "state": "RECEIVED",
        "requesterChampionId": 103,
    }
    service._trade_created_at["7"] = 96.0
    now = {"value": 100.0}
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        calls.append((method, path, json_body))
        return None

    monkeypatch.setattr(service, "request", request)
    monkeypatch.setattr(league_lab.time, "monotonic", lambda: now["value"])
    profile = league_lab.PickProfile(
        enabled=True,
        delay_seconds=5,
        bench_handle_trade_enabled=True,
        bench_select_first_available_champion=True,
    )

    asyncio.run(service._run_trade_handling(
        {"trades": []}, [103, 157], profile, {"current_session_champion_id": 157}
    ))
    plan = service._delayed_action_plan["trade:7:accept"]
    assert plan["delay_seconds"] == pytest.approx(1.0)
    assert calls == []

    now["value"] = 101.1
    asyncio.run(service._run_trade_handling(
        {"trades": []}, [103, 157], profile, {"current_session_champion_id": 157}
    ))
    assert calls == [("POST", "/lol-champ-select/v1/session/champion-swaps/7/accept", None)]


def test_auto_select_expected_candidate_statuses_are_fine_grained(monkeypatch):
    service = LeagueLabService()
    service.phase = "ChampSelect"
    service.game_mode = "CLASSIC"
    profile = league_lab.AutoSelectProfile(
        pick={"enabled": True, "champions": {"default": [1, 2, 3, 4, 5]}},
        ban={"enabled": True, "champions": {"default": [7, 8, 9, 10]}},
    )
    service.settings = LeagueLabSettings(
        auto_select_enabled=True,
        auto_select_profiles={"normal": profile},
    )
    service.champ_select = {
        "local_player_cell_id": 1,
        "my_team": [{"cell_id": 1, "assigned_position": "default"}],
        "current_pickable_champion_ids": [2, 3, 4, 5],
        "current_pickable_ids_available": True,
        "current_bannable_champion_ids": [8, 9, 10],
        "current_bannable_ids_available": True,
        "grid_champions_available": True,
        "grid_champions": {
            1: {"owned": False, "selection_status": {}},
            2: {"owned": True, "selection_status": {"is_banned": True}},
            3: {"owned": True, "selection_status": {"pick_intented": True}},
            4: {"owned": True, "selection_status": {"picked_by_other_or_banned": True}},
            5: {"owned": True, "selection_status": {}},
            7: {"owned": True, "selection_status": {}},
            8: {"owned": True, "selection_status": {"is_banned": True}},
            9: {"owned": True, "selection_status": {"pick_intented": True}},
            10: {"owned": True, "selection_status": {}},
        },
        "bench_enabled": True,
        "bench_champions": [4, 5],
        "allow_duplicate_picks": False,
        "allow_subset_champion_picks": False,
    }
    monkeypatch.setattr(league_lab, "_league_client_window_is_present", lambda: False)

    expected = service.status()["auto_select"]
    assert expected["expected_picks"] == [
        {"id": 1, "status": "not-owned"},
        {"id": 2, "status": "banned"},
        {"id": 3, "status": "pick-intented"},
        {"id": 4, "status": "picked"},
        {"id": 5, "status": "pickable"},
    ]
    assert expected["expected_bans"] == [
        {"id": 7, "status": "unbannable"},
        {"id": 8, "status": "banned"},
        {"id": 9, "status": "pick-intented"},
        {"id": 10, "status": "bannable"},
    ]
    assert expected["expected_swaps"] == [
        {"id": 1, "status": "unswappable"},
        {"id": 2, "status": "unswappable"},
        {"id": 3, "status": "unswappable"},
        {"id": 4, "status": "swappable"},
        {"id": 5, "status": "swappable"},
    ]


def test_invitation_strategy_prefers_accept_and_respects_game_type(monkeypatch):
    service = LeagueLabService()
    service.phase = "Lobby"
    service.settings = LeagueLabSettings(
        automation_enabled=True,
        auto_handle_invitations_enabled=True,
        invitation_handling_strategies={"<DEFAULT>": "decline", "NORMAL_GAME": "accept"},
    )
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        if method == "GET":
            return [
                {"invitationId": "custom", "state": "Pending", "canAcceptInvitation": True, "gameConfig": {"inviteGameType": "CUSTOM_GAME"}},
                {"invitationId": "normal", "state": "Pending", "canAcceptInvitation": True, "gameConfig": {"inviteGameType": "NORMAL_GAME"}},
            ]
        calls.append((method, path))

    monkeypatch.setattr(service, "request", request)
    asyncio.run(service._run_lobby_automation())
    assert calls == [("POST", "/lol-lobby/v2/received-invitations/normal/accept")]


def test_received_invitations_require_explicit_automation_switch(monkeypatch):
    service = LeagueLabService()
    service.phase = "Lobby"
    service.settings = LeagueLabSettings(
        automation_enabled=True,
        auto_handle_invitations_enabled=False,
        invitation_strategy="accept",
    )

    async def request(*args, **kwargs):
        raise AssertionError("received invitations must not be read when automation is disabled")

    monkeypatch.setattr(service, "request", request)
    asyncio.run(service._run_lobby_automation())


def test_auto_honor_submits_votes_and_finishes_ballot(monkeypatch):
    service = LeagueLabService()
    service.settings = LeagueLabSettings(automation_enabled=True, auto_honor_enabled=True)
    service.phase = "EndOfGame"
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        if method == "GET":
            return {"gameId": 77, "votePool": {"votes": 1}, "eligibleAllies": [{"puuid": "ally", "botPlayer": False}], "eligibleOpponents": []}
        calls.append((method, path, json_body))

    monkeypatch.setattr(service, "request", request)
    asyncio.run(service._run_auto_honor())
    asyncio.run(service._run_auto_honor())

    assert calls == [
        ("POST", "/lol-honor/v1/honor", {"honorType": "HEART", "recipientPuuid": "ally"}),
        ("POST", "/lol-honor/v1/ballot", None),
    ]


def test_auto_honor_handles_waiting_for_stats_and_rearms_play_again(monkeypatch):
    service = LeagueLabService()
    service.settings = LeagueLabSettings(
        automation_enabled=True,
        auto_honor_enabled=True,
        play_again_enabled=True,
    )
    service.phase = "WaitingForStats"
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        if (method, path) == ("GET", "/lol-honor-v2/v1/ballot/"):
            return {
                "gameId": 780,
                "votePool": {"votes": 1},
                "eligibleAllies": [{"puuid": "ally", "botPlayer": False}],
                "eligibleOpponents": [],
            }
        if (method, path) == ("GET", "/lol-lobby/v2/eog-status"):
            return {"eogPlayers": [], "leftPlayers": [], "readyPlayers": []}
        calls.append((method, path, json_body))

    monkeypatch.setattr(service, "request", request)
    asyncio.run(service._run_automation())

    assert calls == [
        ("POST", "/lol-honor/v1/honor", {"honorType": "HEART", "recipientPuuid": "ally"}),
        ("POST", "/lol-honor/v1/ballot", None),
    ]
    assert service._phase_action_due_at is not None

    # The ballot acknowledgement must not permanently cancel the independent
    # play-again action.  Simulate the short re-armed deadline expiring.
    service._phase_action_due_at = 0
    asyncio.run(service._run_automation())
    assert ("POST", "/lol-lobby/v2/play-again", None) in calls


def test_missing_honor_ballot_does_not_block_play_again(monkeypatch):
    service = LeagueLabService()
    service.settings = LeagueLabSettings(
        automation_enabled=True,
        auto_honor_enabled=True,
        play_again_enabled=True,
    )
    service.phase = "EndOfGame"
    service._acted_phase = "EndOfGame"
    service._phase_action_due_at = 0
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        if (method, path) == ("GET", "/lol-honor-v2/v1/ballot/"):
            raise RuntimeError("LCU 请求失败: 404")
        calls.append((method, path, json_body))

    monkeypatch.setattr(service, "request", request)
    asyncio.run(service._run_automation())

    assert calls == [("POST", "/lol-lobby/v2/play-again", None)]


def test_auto_honor_finishes_empty_ballot_before_returning(monkeypatch):
    service = LeagueLabService()
    service.settings = LeagueLabSettings(automation_enabled=True, auto_honor_enabled=True)
    service.phase = "EndOfGame"
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        if (method, path) == ("GET", "/lol-honor-v2/v1/ballot/"):
            return {"gameId": 781, "votePool": {"votes": 1}, "eligibleAllies": [], "eligibleOpponents": []}
        if (method, path) == ("GET", "/lol-lobby/v2/eog-status"):
            return {}
        calls.append((method, path, json_body))

    monkeypatch.setattr(service, "request", request)
    asyncio.run(service._run_auto_honor())

    assert calls == [("POST", "/lol-honor/v1/ballot", None)]
    assert service._honored_game_id == "781"


def test_auto_honor_prefers_lobby_allies_then_other_allies_then_opponents(monkeypatch):
    service = LeagueLabService()
    service.settings = LeagueLabSettings(automation_enabled=True, auto_honor_enabled=True)
    service.phase = "EndOfGame"
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        if (method, path) == ("GET", "/lol-honor-v2/v1/ballot/"):
            return {
                "gameId": 79,
                "votePool": {"votes": 3},
                "eligibleAllies": [
                    {"puuid": "other", "botPlayer": False},
                    {"puuid": "lobby", "botPlayer": False},
                ],
                "eligibleOpponents": [{"puuid": "enemy", "botPlayer": False}],
            }
        if (method, path) == ("GET", "/lol-lobby/v2/eog-status"):
            return {"eogPlayers": ["lobby"]}
        calls.append((method, path, json_body))

    monkeypatch.setattr(service, "request", request)
    monkeypatch.setattr(league_lab.random, "sample", lambda population, count: list(population)[:count])

    asyncio.run(service._run_auto_honor())

    assert calls == [
        ("POST", "/lol-honor/v1/honor", {"honorType": "HEART", "recipientPuuid": "lobby"}),
        ("POST", "/lol-honor/v1/honor", {"honorType": "HEART", "recipientPuuid": "other"}),
        ("POST", "/lol-honor/v1/honor", {"honorType": "HEART", "recipientPuuid": "enemy"}),
        ("POST", "/lol-honor/v1/ballot", None),
    ]


def test_auto_matchmaking_waits_for_invitees(monkeypatch):
    service = LeagueLabService()
    service.settings = LeagueLabSettings(
        automation_enabled=True,
        auto_matchmaking_enabled=True,
        auto_matchmaking_delay_seconds=0,
        auto_matchmaking_wait_for_invitees=True,
    )
    service.phase = "Lobby"
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        if path == "/lol-lobby/v2/lobby":
            return {
                "localMember": {"isLeader": True},
                "members": [{}],
                "invitations": [{"state": "Pending"}],
                "canStartActivity": True,
            }
        calls.append((method, path))
        return None

    monkeypatch.setattr(service, "request", request)
    asyncio.run(service._run_auto_matchmaking())

    assert service._matchmaking_status == "waiting-for-invitees"
    assert calls == []


def test_auto_matchmaking_requires_master_gate(monkeypatch):
    service = LeagueLabService()
    service.settings = LeagueLabSettings(
        automation_enabled=False,
        auto_matchmaking_enabled=True,
    )
    service.phase = "Lobby"

    async def request(*args, **kwargs):
        raise AssertionError("the master automation gate must stop before LCU access")

    monkeypatch.setattr(service, "request", request)
    asyncio.run(service._run_auto_matchmaking())

    assert service._matchmaking_status == "idle"
    assert service._matchmaking_due_at is None


def test_auto_matchmaking_exposes_cancel_reason_and_chat_feedback(monkeypatch):
    service = LeagueLabService()
    service.settings = LeagueLabSettings(
        automation_enabled=True,
        auto_matchmaking_enabled=True,
        auto_matchmaking_chat_countdown_enabled=True,
        auto_matchmaking_wait_for_invitees=True,
    )
    service.phase = "Lobby"
    service._matchmaking_due_at = time.monotonic() + 5
    service._matchmaking_status = "countdown"
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        calls.append((method, path, json_body))
        if path == "/lol-lobby/v2/lobby":
            return {
                "localMember": {"isLeader": True},
                "members": [{}],
                "invitations": [{"state": "Pending"}],
                "canStartActivity": True,
            }
        if path == "/lol-chat/v1/conversations":
            return [{"id": "custom-room", "type": "customGame"}]
        return None

    monkeypatch.setattr(service, "request", request)
    asyncio.run(service._run_auto_matchmaking())

    assert service._matchmaking_status_reason == "waiting-for-invitees"
    assert service._matchmaking_due_at is None
    chat_posts = [
        body
        for method, path, body in calls
        if method == "POST" and path.endswith("/messages")
    ]
    assert chat_posts == [{
        "body": "[Insight] 自动匹配已取消，正在等待邀请的好友",
        "type": "celebration",
    }]


def test_auto_matchmaking_starts_when_lobby_is_ready(monkeypatch):
    service = LeagueLabService()
    service.settings = LeagueLabSettings(
        automation_enabled=True,
        auto_matchmaking_enabled=True,
        auto_matchmaking_delay_seconds=0,
    )
    service.phase = "Lobby"
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        if path == "/lol-lobby/v2/lobby":
            return {
                "localMember": {"isLeader": True},
                "members": [{}],
                "invitations": [],
                "canStartActivity": True,
            }
        if path == "/lol-matchmaking/v1/search":
            return None
        calls.append((method, path))

    monkeypatch.setattr(service, "request", request)
    asyncio.run(service._run_auto_matchmaking())

    assert calls == [("POST", "/lol-lobby/v2/lobby/matchmaking/search")]
    assert service._matchmaking_status == "searching"


def test_auto_matchmaking_chat_countdown_is_disabled_by_default(monkeypatch):
    service = LeagueLabService()
    service.settings = LeagueLabSettings(
        automation_enabled=True,
        auto_matchmaking_enabled=True,
        auto_matchmaking_delay_seconds=5,
    )
    service.phase = "Lobby"
    service._matchmaking_due_at = 100.0
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        calls.append((method, path, json_body))
        if path == "/lol-lobby/v2/lobby":
            return {
                "localMember": {"isLeader": True},
                "members": [{}],
                "invitations": [],
                "canStartActivity": True,
            }
        if path == "/lol-matchmaking/v1/search":
            return None
        raise AssertionError(f"chat countdown should be disabled: {method} {path}")

    monkeypatch.setattr(service, "request", request)
    monkeypatch.setattr(league_lab.time, "monotonic", lambda: 97.0)

    asyncio.run(service._run_auto_matchmaking())

    assert [path for _method, path, _body in calls] == [
        "/lol-lobby/v2/lobby",
        "/lol-matchmaking/v1/search",
    ]


def test_auto_matchmaking_chat_countdown_sends_each_remaining_second_once(monkeypatch):
    service = LeagueLabService()
    service.settings = LeagueLabSettings(
        automation_enabled=True,
        auto_matchmaking_enabled=True,
        auto_matchmaking_chat_countdown_enabled=True,
    )
    service.phase = "Lobby"
    service._matchmaking_due_at = 100.0
    now = {"value": 97.0}
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        calls.append((method, path, json_body))
        if path == "/lol-lobby/v2/lobby":
            return {
                "localMember": {"isLeader": True},
                "members": [{}],
                "invitations": [],
                "canStartActivity": True,
            }
        if path == "/lol-matchmaking/v1/search":
            return None
        if path == "/lol-chat/v1/conversations":
            return [{"id": "custom-room", "type": "customgame"}]
        return None

    monkeypatch.setattr(service, "request", request)
    monkeypatch.setattr(league_lab.time, "monotonic", lambda: now["value"])

    # The first call announces 3 seconds remaining.
    asyncio.run(service._run_auto_matchmaking())
    # Polling again during the same second must not repeat the announcement.
    asyncio.run(service._run_auto_matchmaking())
    # A new remaining second gets exactly one new announcement.
    now["value"] = 96.0
    asyncio.run(service._run_auto_matchmaking())

    chat_posts = [
        (path, body)
        for method, path, body in calls
        if method == "POST" and path.endswith("/messages")
    ]
    assert chat_posts == [
        (
            "/lol-chat/v1/conversations/custom-room/messages",
            {"body": "[Insight] 将在 3 秒后自动开始匹配", "type": "celebration"},
        ),
        (
            "/lol-chat/v1/conversations/custom-room/messages",
            {"body": "[Insight] 将在 4 秒后自动开始匹配", "type": "celebration"},
        ),
    ]
    assert calls.count(("GET", "/lol-chat/v1/conversations", None)) == 2


def test_auto_honor_opt_out_finishes_without_voting(monkeypatch):
    service = LeagueLabService()
    service.settings = LeagueLabSettings(
        automation_enabled=True,
        auto_honor_enabled=True,
        auto_honor_strategy="opt-out",
    )
    service.phase = "EndOfGame"
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        if method == "GET":
            return {"gameId": 88, "votePool": {"votes": 1}, "eligibleAllies": [{"puuid": "ally"}]}
        calls.append((method, path, json_body))

    monkeypatch.setattr(service, "request", request)
    asyncio.run(service._run_auto_honor())

    assert calls == [("POST", "/lol-honor/v1/ballot", None)]


def test_auto_honor_rearms_play_again_after_pre_end_actions(monkeypatch):
    service = LeagueLabService()
    service.settings = LeagueLabSettings(
        automation_enabled=True,
        auto_honor_enabled=True,
        play_again_enabled=True,
    )
    service.phase = "PreEndOfGame"
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        if path == "/lol-honor-v2/v1/ballot/":
            return {
                "gameId": 890,
                "votePool": {"votes": 1},
                "eligibleAllies": [{"puuid": "ally", "botPlayer": False}],
                "eligibleOpponents": [],
            }
        if path == "/lol-lobby/v2/eog-status":
            return {}
        calls.append((method, path, json_body))

    monkeypatch.setattr(service, "request", request)
    asyncio.run(service._run_automation())

    assert calls == [
        ("POST", "/lol-honor/v1/honor", {"honorType": "HEART", "recipientPuuid": "ally"}),
        ("POST", "/lol-honor/v1/ballot", None),
    ]
    assert service._phase_action_due_at is not None
    assert service._phase_action_due_at > time.monotonic()


def test_automation_disabled_clears_all_scheduler_deadlines_and_plans():
    service = LeagueLabService()
    service.settings = LeagueLabSettings(automation_enabled=False)
    service.phase = "Lobby"
    service._accept_due_at = 1
    service._matchmaking_due_at = 2
    service._phase_action_due_at = 3
    service._matchmaking_status = "searching"
    service._matchmaking_chat_countdown_second = 4
    service._champion_action_due_at["pick:1"] = 5
    service._delayed_action_plan["pick:1"] = {"action_id": "pick:1", "due_at": 5}

    asyncio.run(service._run_automation())

    assert service._accept_due_at is None
    assert service._matchmaking_due_at is None
    assert service._phase_action_due_at is None
    assert service._matchmaking_status == "idle"
    assert service._matchmaking_chat_countdown_second is None
    assert service._champion_action_due_at == {}
    assert service._delayed_action_plan == {}


def test_auto_reply_uses_event_conversation_and_ignores_history(monkeypatch):
    service = LeagueLabService()
    service.settings = LeagueLabSettings(
        automation_enabled=True,
        auto_reply_enabled=True,
        auto_reply_text="稍后回复",
    )
    service.current_summoner = {"summoner_id": 7}
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        calls.append((method, path, json_body))

    monkeypatch.setattr(service, "request", request)
    asyncio.run(
        service._handle_lcu_event(
            {
                "uri": "/lol-chat/v1/conversations/friend/messages/message-1",
                "eventType": "Create",
                "data": {"type": "chat", "fromSummonerId": 8, "isHistorical": False},
            }
        )
    )

    assert calls == [
        (
            "POST",
            "/lol-chat/v1/conversations/friend/messages",
            {"body": "稍后回复", "type": "chat"},
        )
    ]


def test_aram_team_side_sends_once(monkeypatch):
    service = LeagueLabService()
    service.settings = LeagueLabSettings(
        automation_enabled=True,
        auto_send_aram_team_side_enabled=True,
        auto_send_aram_team_side_visible_to_team=True,
    )
    service.phase = "ChampSelect"
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        if path == "/lol-champ-select/v1/session":
            return {"benchEnabled": True, "localPlayerCellId": 3, "myTeam": [{"cellId": 3, "team": 1}]}
        if path == "/lol-gameflow/v1/session":
            return {"map": {"gameMode": "ARAM"}}
        if path == "/lol-chat/v1/conversations":
            return [{"id": "champ", "type": "championSelect"}]
        calls.append((method, path, json_body))

    monkeypatch.setattr(service, "request", request)
    asyncio.run(service._run_aram_team_side())
    asyncio.run(service._run_aram_team_side())

    assert calls == [
        (
            "POST",
            "/lol-chat/v1/conversations/champ/messages",
            {"body": "本局位于左侧（蓝方）", "type": "chat"},
        )
    ]


def test_auto_invite_online_friend_removes_completed_target(monkeypatch, tmp_path):
    monkeypatch.setattr(LeagueLabService, "_settings_path", staticmethod(lambda: tmp_path / "league-lab.json"))
    service = LeagueLabService()
    service.settings = LeagueLabSettings(
        automation_enabled=True,
        auto_invite_friend_puuids=["friend-puuid"],
    )
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        if method == "GET":
            return {"members": [], "localMember": {"allowedInviteOthers": True}}
        calls.append((method, path, json_body))

    monkeypatch.setattr(service, "request", request)
    asyncio.run(
        service._handle_lcu_event(
            {
                "uri": "/lol-chat/v1/friends/friend-puuid",
                "data": {
                    "puuid": "friend-puuid",
                    "availability": "chat",
                    "summonerId": 42,
                    "gameName": "Friend",
                },
            }
        )
    )

    assert calls == [("POST", "/lol-lobby/v2/lobby/invitations", [{"toSummonerId": 42}])]
    assert service.settings.auto_invite_friend_puuids == []


@pytest.mark.parametrize(
    "lobby",
    [None, {}, {"localMember": {}}, {"localMember": {"allowedInviteOthers": False}}],
)
def test_auto_invite_friend_requires_valid_lobby_and_invite_permission(monkeypatch, tmp_path, lobby):
    monkeypatch.setattr(LeagueLabService, "_settings_path", staticmethod(lambda: tmp_path / "league-lab.json"))
    service = LeagueLabService()
    service.settings = LeagueLabSettings(
        automation_enabled=True,
        auto_invite_friend_puuids=["friend-puuid"],
    )
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        if method == "GET":
            return lobby
        calls.append((method, path, json_body))

    monkeypatch.setattr(service, "request", request)
    asyncio.run(
        service._handle_lcu_event(
            {
                "uri": "/lol-chat/v1/friends/friend-puuid",
                "data": {
                    "puuid": "friend-puuid",
                    "availability": "chat",
                    "summonerId": 42,
                },
            }
        )
    )

    assert calls == []
    assert service.settings.auto_invite_friend_puuids == ["friend-puuid"]


def test_auto_invite_friend_selection_uses_read_only_list_and_local_schedule(monkeypatch):
    service = league_lab.league_lab_service
    monkeypatch.setattr(service, "settings", LeagueLabSettings(
        automation_enabled=False,
        auto_invite_friend_puuids=["offline-puuid"],
    ))
    monkeypatch.setattr(service, "_write_settings", lambda _settings: None)
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        calls.append((method, path, json_body))
        assert method == "GET"
        assert path == "/lol-chat/v1/friends"
        return [
            {
                "puuid": "online-puuid",
                "summonerId": 42,
                "gameName": "Online",
                "gameTag": "CN1",
                "availability": "chat",
            },
            {
                "puuid": "busy-puuid",
                "summonerId": 43,
                "gameName": "Busy",
                "gameTag": "CN2",
                "availability": "dnd",
            },
        ]

    monkeypatch.setattr(service, "request", request)
    listed = asyncio.run(league_lab.league_auto_invite_friends())
    assert listed["scheduled_puuids"] == ["offline-puuid"]
    online = next(row for row in listed["friends"] if row["puuid"] == "online-puuid")
    assert online["inviteable"] is True
    assert online["scheduled"] is False
    assert all("spectatorKey" not in row for row in listed["friends"])

    scheduled = asyncio.run(league_lab.schedule_league_auto_invite_friends(
        league_lab.AutoInviteFriendSchedule(puuids=["online-puuid", "unknown-puuid", "online-puuid"])
    ))
    assert scheduled["scheduled_puuids"] == ["online-puuid"]
    assert service.settings.auto_invite_friend_puuids == ["online-puuid"]
    assert all(method == "GET" for method, _path, _body in calls)


def test_player_search_uses_lcu_riot_id_alias(monkeypatch):
    calls = []

    async def unavailable_riot_aliases(game_name, tag_line):
        raise RuntimeError("Riot Client API unavailable in this fixture")

    async def request(method, path, *, json_body=None, params=None):
        calls.append((method, path, json_body, params))
        if path.endswith("/aliases"):
            return [{"puuid": "player-1", "gameName": "Player", "tagLine": "CN1"}]
        return {}

    async def bundle(summoner, match_limit=20, beg_index=0):
        return {"summoner": summoner, "matches": []}

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    monkeypatch.setattr(league_lab, "_riot_player_account_aliases", unavailable_riot_aliases)
    monkeypatch.setattr(league_lab, "_load_player_bundle", bundle)
    result = asyncio.run(league_lab.search_league_player(" Player ", " CN1 "))

    assert result["summoner"]["puuid"] == "player-1"
    assert calls == [
        (
            "POST",
            "/lol-summoner/v1/summoners/aliases",
            [{"gameName": "Player", "tagLine": "CN1"}],
            None,
        )
    ]


def test_cross_region_player_search_uses_riot_alias_and_target_sgp(monkeypatch):
    league_lab.league_lab_service.credentials = league_lab.LcuCredentials(1, "x", "CN", "HN1", 2, "riot")

    async def aliases(game_name, tag_line):
        assert (game_name, tag_line) == ("Player", "KR1")
        return [{"puuid": "global-puuid", "alias": {"game_name": "Player", "tag_line": "KR1"}}]

    async def summoner(puuid, server_id=None):
        assert (puuid, server_id) == ("global-puuid", "KR")
        return {"puuid": puuid, "displayName": "", "source": "sgp", "summonerLevel": 50}

    async def bundle(summoner_row, match_limit=20, beg_index=0, sgp_server_id=None, prefer_sgp=False):
        assert summoner_row["gameName"] == "Player"
        assert summoner_row["tagLine"] == "KR1"
        assert sgp_server_id == "KR"
        assert prefer_sgp is True
        return {"summoner": summoner_row, "server_id": sgp_server_id}

    monkeypatch.setattr(league_lab, "_riot_player_account_aliases", aliases)
    monkeypatch.setattr(league_lab, "_sgp_summoner_by_puuid", summoner)
    monkeypatch.setattr(league_lab, "_load_player_bundle", bundle)

    result = asyncio.run(league_lab.search_league_player(" Player ", " KR1 ", "kr"))
    assert result["server_id"] == "KR"


def test_player_search_servers_marks_current_region():
    league_lab.league_lab_service.credentials = league_lab.LcuCredentials(1, "x", "CN", "HN10")
    result = asyncio.run(league_lab.league_player_search_servers())
    assert result["current"] == "TENCENT_HN10"
    current = [row for row in result["servers"] if row["current"]]
    assert current == [{"id": "TENCENT_HN10", "label": "黑色玫瑰", "current": True}]


def test_recent_players_are_deduplicated_and_sorted(monkeypatch, tmp_path):
    monkeypatch.setattr(league_lab, "_recent_players_path", lambda: tmp_path / "recent.json")
    monkeypatch.setattr(league_lab.time, "time", lambda: 10)
    league_lab._remember_recent_players(
        [{"puuid": "one", "summoner": {"gameName": "One"}, "champion_id": 1}],
        100,
    )
    monkeypatch.setattr(league_lab.time, "time", lambda: 20)
    league_lab._remember_recent_players(
        [
            {"puuid": "one", "summoner": {"gameName": "One Renamed"}, "champion_id": 2},
            {"puuid": "two", "summoner": {"gameName": "Two"}, "champion_id": 3},
        ],
        200,
    )

    rows = league_lab._read_recent_players()
    assert [row["puuid"] for row in rows] == ["one", "two"]
    assert rows[0]["game_name"] == "One Renamed"
    assert rows[0]["last_game_id"] == 200


def test_premade_groups_require_repeated_same_team_matches():
    def match(game_id, teammates):
        identities = [
            {"participantId": index + 1, "player": {"puuid": puuid}}
            for index, puuid in enumerate(teammates)
        ]
        participants = [
            {"participantId": index + 1, "teamId": 100}
            for index in range(len(teammates))
        ]
        return {"gameId": game_id, "participantIdentities": identities, "participants": participants}

    histories = {
        "a": {"games": {"games": [match(1, ["a", "b"]), match(2, ["a", "b"]), match(3, ["a", "b"])]}},
        "c": {"games": {"games": [match(4, ["b", "c"])]}},
    }
    groups = league_lab._infer_premade_groups(histories, {"a", "b", "c"}, threshold=3)

    assert groups["a"] == groups["b"]
    assert "c" not in groups


def test_toolkit_overview_is_read_only(monkeypatch):
    async def request(method, path, *, json_body=None, params=None):
        assert method == "GET"
        payloads = {
            "/lol-missions/v1/missions": [{"id": "mission"}],
            "/lol-missions/v1/series": [{"id": "series"}],
            "/lol-rewards/v1/grants": [{"id": "reward", "viewed": False}],
            "/lol-loot/v1/player-loot-map": {"loot": {"lootId": "loot"}},
            "/lol-chat/v1/friends": [{"puuid": "friend"}],
            "/lol-chat/v1/friend-groups": [{"id": 1, "name": "General"}],
            "/lol-event-hub/v1/events": [],
            "/lol-chat/v1/me": {"availability": "chat", "statusMessage": ""},
        }
        return payloads[path]

    monkeypatch.setattr(
        league_lab.league_lab_service,
        "settings",
        LeagueLabSettings(toolkit_account_actions_enabled=False),
    )
    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    result = asyncio.run(league_lab.league_toolkit_overview())

    assert result["read_only"] is True
    assert result["account_actions_enabled"] is False
    assert result["counts"] == {"missions": 1, "unclaimed_rewards": 1, "loot": 1, "friends": 1}


def test_friend_metadata_matches_league_akari_dates_without_writing(monkeypatch):
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        calls.append((method, path, params))
        assert method == "GET"
        if path == "/lol-chat/v1/friends":
            return [
                {"puuid": "friend-a", "summonerId": 11},
                {"puuid": "friend-b", "summonerId": 22},
            ]
        if path == "/lol-store/v1/giftablefriends":
            return [{"summonerId": 11, "friendsSince": "2025-01-02T03:04:05Z"}]
        if path.endswith("/friend-a/matches"):
            assert params == {"begIndex": 0, "endIndex": 0}
            return {"games": {"games": [{"gameCreation": 1770000000000}]}}
        if path.endswith("/friend-b/matches"):
            return {"games": {"games": []}}
        raise AssertionError(path)

    monkeypatch.setattr(league_lab.league_lab_service, "credentials", None)
    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    result = asyncio.run(league_lab.league_friend_metadata())

    assert result == {
        "friends": {
            "friend-a": {
                "last_game_at": 1770000000000,
                "friends_since": "2025-01-02T03:04:05Z",
                "source": "lcu",
            },
            "friend-b": {"last_game_at": None, "friends_since": None, "source": "lcu"},
        },
        "count": 2,
        "source": "lcu",
    }
    assert all(method == "GET" for method, _path, _params in calls)


def test_toolkit_account_writes_are_enabled_by_default(monkeypatch):
    assert LeagueLabSettings().toolkit_account_actions_enabled is True
    assert LeagueLabSettings().terminate_game_shortcut_enabled is False
    assert LeagueLabSettings().terminate_game_shortcut == "Ctrl+Alt+End"
    assert LeagueLabSettings().in_game_send_enabled is False
    assert LeagueLabSettings().in_game_fixed_presets == []
    monkeypatch.setattr(
        league_lab.league_lab_service,
        "settings",
        LeagueLabSettings(toolkit_account_actions_enabled=False),
    )
    body = league_lab.EventRewardClaim(event_id="event-1", confirmation="我确认领取")
    with pytest.raises(league_lab.HTTPException) as caught:
        asyncio.run(league_lab.league_claim_event_rewards(body))
    assert caught.value.status_code == 403


def test_champ_select_dodge_is_gated_and_revalidates_live_phase(monkeypatch):
    monkeypatch.setattr(
        league_lab.league_lab_service, "settings", LeagueLabSettings(toolkit_account_actions_enabled=True)
    )
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        calls.append((method, path, json_body, params))
        if path == "/lol-gameflow/v1/gameflow-phase":
            return "ChampSelect"
        return None

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    result = asyncio.run(league_lab.league_champ_select_dodge(
        league_lab.ChampSelectDodgeRequest(confirmation="我确认秒退")
    ))
    assert result["last_action"] == "已执行一次英雄选择秒退"
    assert calls[0][1] == "/lol-gameflow/v1/gameflow-phase"
    assert calls[1][1] == "/lol-login/v1/session/invoke"
    assert calls[1][3]["destination"] == "lcdsServiceProxy"


def test_charity_reroll_only_grabs_back_original_with_fresh_evidence(monkeypatch):
    service = league_lab.league_lab_service
    monkeypatch.setattr(
        service,
        "settings",
        LeagueLabSettings(toolkit_account_actions_enabled=True),
    )
    before = {
        "localPlayerCellId": 2,
        "myTeam": [{"cellId": 2, "championId": 22}],
        "benchEnabled": True,
        "allowRerolling": True,
        "rerollsRemaining": 1,
        "timer": {"phase": "FINALIZATION"},
    }
    after = {
        "localPlayerCellId": 2,
        "myTeam": [{"cellId": 2, "championId": 34}],
        "benchEnabled": True,
        "allowSubsetChampionPicks": False,
        "benchChampions": [{"championId": 22}],
        "timer": {"phase": "FINALIZATION"},
    }
    calls = []
    session_reads = 0

    async def request(method, path, *, json_body=None, params=None):
        nonlocal session_reads
        calls.append((method, path, json_body, params))
        if (method, path) == ("GET", "/lol-gameflow/v1/gameflow-phase"):
            return "ChampSelect"
        if (method, path) == ("GET", "/lol-champ-select/v1/session"):
            session_reads += 1
            return before if session_reads == 1 else after
        if (method, path) == ("GET", "/lol-champ-select/v1/pickable-champion-ids"):
            return [22, 34]
        if (method, path) in {
            ("POST", "/lol-champ-select/v1/session/my-selection/reroll"),
            ("POST", "/lol-champ-select/v1/session/bench/swap/22"),
        }:
            return None
        raise AssertionError(f"unexpected request: {method} {path}")

    async def refresh_state():
        return None

    monkeypatch.setattr(service, "request", request)
    monkeypatch.setattr(service, "_refresh_state", refresh_state)

    result = asyncio.run(league_lab.league_champ_select_charity_reroll(
        league_lab.ChampSelectCharityRerollRequest(confirmation="我确认慈善重随")
    ))

    assert result["charity_reroll"] == {
        "original_champion_id": 22,
        "rerolled": True,
        "grabbed_back": True,
        "swap_reason": None,
    }
    assert [path for _method, path, _body, _params in calls if "session/my-selection/reroll" in path] == [
        "/lol-champ-select/v1/session/my-selection/reroll"
    ]
    assert calls[-1][1] == "/lol-champ-select/v1/session/bench/swap/22"


def test_charity_reroll_never_swaps_without_bench_or_pickable_evidence(monkeypatch):
    service = league_lab.league_lab_service
    monkeypatch.setattr(
        service,
        "settings",
        LeagueLabSettings(toolkit_account_actions_enabled=True),
    )
    before = {
        "localPlayerCellId": 2,
        "myTeam": [{"cellId": 2, "championId": 22}],
        "allowRerolling": True,
        "rerollsRemaining": 1,
    }
    after = {
        "localPlayerCellId": 2,
        "myTeam": [{"cellId": 2, "championId": 34}],
        "benchEnabled": True,
        "timer": {"phase": "FINALIZATION"},
        "benchChampions": [],
    }
    calls = []
    session_reads = 0

    async def request(method, path, *, json_body=None, params=None):
        nonlocal session_reads
        calls.append((method, path))
        if path == "/lol-gameflow/v1/gameflow-phase":
            return "ChampSelect"
        if path == "/lol-champ-select/v1/session":
            session_reads += 1
            return before if session_reads == 1 else after
        if path == "/lol-champ-select/v1/session/my-selection/reroll":
            return None
        raise AssertionError(f"no optional evidence should be queried: {method} {path}")

    async def refresh_state():
        return None

    monkeypatch.setattr(service, "request", request)
    monkeypatch.setattr(service, "_refresh_state", refresh_state)
    result = asyncio.run(league_lab.league_champ_select_charity_reroll(
        league_lab.ChampSelectCharityRerollRequest(confirmation="我确认慈善重随")
    ))

    assert result["charity_reroll"]["rerolled"] is True
    assert result["charity_reroll"]["grabbed_back"] is False
    assert all(path != "/lol-champ-select/v1/session/bench/swap/22" for _method, path in calls)


def test_dodge_loop_requires_gate_and_live_phase_before_start(monkeypatch):
    service = league_lab.league_lab_service
    service._terminate_dodge_loop("test-reset")
    calls = []

    async def request(method, path, **_kwargs):
        calls.append((method, path))
        return "ChampSelect"

    monkeypatch.setattr(service, "request", request)
    monkeypatch.setattr(service, "settings", LeagueLabSettings(toolkit_account_actions_enabled=False))
    with pytest.raises(league_lab.HTTPException) as caught:
        asyncio.run(league_lab.league_champ_select_dodge_loop_start(
            league_lab.ChampSelectDodgeLoopRequest(confirmation="我确认秒退")
        ))
    assert caught.value.status_code == 403
    assert calls == []

    monkeypatch.setattr(service, "settings", LeagueLabSettings(toolkit_account_actions_enabled=True))

    async def wrong_phase(method, path, **_kwargs):
        calls.append((method, path))
        return "Lobby"

    monkeypatch.setattr(service, "request", wrong_phase)
    with pytest.raises(league_lab.HTTPException) as caught:
        asyncio.run(league_lab.league_champ_select_dodge_loop_start(
            league_lab.ChampSelectDodgeLoopRequest(confirmation="我确认秒退")
        ))
    assert caught.value.status_code == 409
    assert service.status()["dodge_loop"]["active"] is False


def test_dodge_loop_revalidates_gate_before_each_write(monkeypatch):
    service = LeagueLabService()
    service.settings = LeagueLabSettings(toolkit_account_actions_enabled=True)
    calls = []

    async def request(method, path, **_kwargs):
        calls.append((method, path))
        if path == "/lol-gameflow/v1/gameflow-phase":
            service.settings.toolkit_account_actions_enabled = False
            return "ChampSelect"
        raise AssertionError("gate must stop the write after the phase check")

    monkeypatch.setattr(service, "request", request)
    ok, reason = asyncio.run(service._dodge_once_with_revalidation())
    assert ok is False
    assert reason == "account-actions-disabled"
    assert calls == [("GET", "/lol-gameflow/v1/gameflow-phase")]


def test_dodge_loop_start_and_cancel_is_local_and_uses_five_workers(monkeypatch):
    service = league_lab.league_lab_service
    service._terminate_dodge_loop("test-reset")
    service.settings = LeagueLabSettings(toolkit_account_actions_enabled=True)
    calls = []

    async def request(method, path, **_kwargs):
        calls.append((method, path))
        if path == "/lol-gameflow/v1/gameflow-phase":
            return "ChampSelect"
        if path == "/lol-login/v1/session/invoke":
            return None
        raise AssertionError(path)

    monkeypatch.setattr(service, "request", request)

    async def exercise():
        started = await league_lab.league_champ_select_dodge_loop_start(
            league_lab.ChampSelectDodgeLoopRequest(confirmation="我确认秒退")
        )
        assert started["dodge_loop"]["active"] is True
        assert started["dodge_loop"]["concurrency"] == 5
        cancelled = await league_lab.league_champ_select_dodge_loop_cancel()
        assert cancelled["dodge_loop"]["active"] is False
        assert cancelled["dodge_loop"]["stop_reason"] == "user-cancelled"
        await asyncio.sleep(0)

    asyncio.run(exercise())
    assert any(path == "/lol-gameflow/v1/gameflow-phase" for _method, path in calls)
    assert any(path == "/lol-login/v1/session/invoke" for _method, path in calls) or service.status()["dodge_loop"]["attempts"] == 0

def test_mission_claim_revalidates_status_selection_and_exact_write(monkeypatch):
    monkeypatch.setattr(
        league_lab.league_lab_service, "settings", LeagueLabSettings(toolkit_account_actions_enabled=True)
    )
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        if method == "GET":
            return [{
                "id": "mission/choice",
                "status": "SELECT_REWARDS",
                "rewardStrategy": {"selectMinGroupCount": 1, "selectMaxGroupCount": 1},
                "rewards": [{"rewardGroup": "group-a"}, {"rewardGroup": "group-b"}],
            }]
        calls.append((method, path, json_body, params))

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    result = asyncio.run(league_lab.league_claim_mission_reward(league_lab.MissionRewardClaim(
        mission_id="mission/choice", reward_group_ids=["group-b"], confirmation="我确认领取"
    )))

    assert result["reward_group_ids"] == ["group-b"]
    assert calls == [("PUT", "/lol-missions/v1/player/mission%2Fchoice", {"rewardGroups": ["group-b"]}, None)]


def test_reward_grant_claim_uses_explicit_user_selection(monkeypatch):
    monkeypatch.setattr(
        league_lab.league_lab_service, "settings", LeagueLabSettings(toolkit_account_actions_enabled=True)
    )
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        if method == "GET":
            assert params == {"status": "PENDING_SELECTION"}
            return [{
                "info": {"id": "grant-1", "status": "PENDING_SELECTION"},
                "rewardGroup": {
                    "id": "reward-group",
                    "selectionStrategyConfig": {"minSelectionsAllowed": 1, "maxSelectionsAllowed": 2},
                    "rewards": [{"id": "skin-a"}, {"id": "skin-b"}, {"id": "skin-c"}],
                },
            }]
        calls.append((method, path, json_body, params))

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    result = asyncio.run(league_lab.league_claim_reward_grant(league_lab.RewardGrantClaim(
        grant_id="grant-1", reward_group_id="reward-group", selection_ids=["skin-a", "skin-c"],
        confirmation="我确认领取",
    )))

    assert result["selections"] == ["skin-a", "skin-c"]
    assert calls == [("POST", "/lol-rewards/v1/grants/grant-1/select", {
        "grantId": "grant-1", "rewardGroupId": "reward-group", "selections": ["skin-a", "skin-c"]
    }, None)]


def test_event_claim_and_friend_delete_revalidate_current_lcu_state(monkeypatch):
    monkeypatch.setattr(
        league_lab.league_lab_service, "settings", LeagueLabSettings(toolkit_account_actions_enabled=True)
    )
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        if (method, path) == ("GET", "/lol-event-hub/v1/events"):
            return [{"eventId": "summer/event", "eventInfo": {"unclaimedRewardCount": 2}}]
        if (method, path) == ("GET", "/lol-chat/v1/friends"):
            return [{"id": "friend/one"}, {"id": "friend-two"}]
        calls.append((method, path, json_body, params))

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    event = asyncio.run(league_lab.league_claim_event_rewards(
        league_lab.EventRewardClaim(event_id="summer/event", confirmation="我确认领取")
    ))
    friends = asyncio.run(league_lab.league_delete_friends(
        league_lab.FriendDeleteRequest(friend_ids=["friend/one", "friend-two"], confirmation="我确认删除")
    ))

    assert event == {"claimed": True, "event_id": "summer/event"}
    assert friends == {"deleted": ["friend/one", "friend-two"], "count": 2}
    assert calls == [
        ("POST", "/lol-event-hub/v1/events/summer%2Fevent/reward-track/claim-all", None, None),
        ("DELETE", "/lol-chat/v1/friends/friend%2Fone", None, None),
        ("DELETE", "/lol-chat/v1/friends/friend-two", None, None),
    ]


def test_lobby_options_preserve_upstream_eligibility_and_strawberry_catalog(monkeypatch):
    async def request(method, path, *, json_body=None, params=None):
        payloads = {
            ("GET", "/lol-game-data/assets/v1/queues.json"): [
                {"id": 420, "name": "单双排"}, {"id": 450, "name": "大乱斗"}
            ],
            ("POST", "/lol-lobby/v2/eligibility/party"): [{"queueId": 420}, {"queueId": 450}],
            ("POST", "/lol-lobby/v2/eligibility/self"): [{"queueId": 420}],
            ("GET", "/lol-lobby/v2/lobby"): {"gameConfig": {"gameMode": "STRAWBERRY"}},
            ("GET", "/lol-game-data/assets/v1/strawberry-hub.json"): [{"MapDisplayInfoList": [{
                "value": {"Name": "仓库区", "Map": {"ContentId": "map/content", "ItemId": 7}}
            }]}],
            ("GET", "/lol-loadouts/v4/loadouts/scope/account"): [{"id": "loadout-1"}],
        }
        return payloads[(method, path)]

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    result = asyncio.run(league_lab.league_lobby_options())

    assert result["queues"][0] == {"id": 420, "name": "单双排", "description": "", "eligible": True}
    assert result["queues"][1]["eligible"] is False
    assert result["strawberry"] == {
        "active": True,
        "maps": [{"name": "仓库区", "content_id": "map/content", "item_id": 7}],
        "difficulties": [1, 2, 3],
        "loadout_available": True,
    }


def test_create_and_leave_lobby_revalidate_live_state(monkeypatch):
    monkeypatch.setattr(
        league_lab.league_lab_service, "settings", LeagueLabSettings(toolkit_account_actions_enabled=True)
    )
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        if path in {"/lol-lobby/v2/eligibility/party", "/lol-lobby/v2/eligibility/self"}:
            return [{"queueId": 420}]
        if (method, path) == ("GET", "/lol-lobby/v2/lobby"):
            return {"gameConfig": {"queueId": 420}}
        calls.append((method, path, json_body))

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    created = asyncio.run(league_lab.league_create_queue_lobby(
        league_lab.QueueLobbyCreate(queue_id=420, confirmation="我确认创建")
    ))
    left = asyncio.run(league_lab.league_leave_lobby(
        league_lab.LeaveLobbyRequest(confirmation="我确认离开")
    ))

    assert created == {"created": True, "queue_id": 420}
    assert left == {"left": True}
    assert calls == [
        ("POST", "/lol-lobby/v2/lobby", {"queueId": 420}),
        ("DELETE", "/lol-lobby/v2/lobby", None),
    ]


def test_strawberry_tools_validate_mode_catalog_and_write_exact_payloads(monkeypatch):
    monkeypatch.setattr(
        league_lab.league_lab_service, "settings", LeagueLabSettings(toolkit_account_actions_enabled=True)
    )
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        if (method, path) == ("GET", "/lol-lobby/v2/lobby"):
            return {"gameConfig": {"gameMode": "STRAWBERRY"}}
        if path.endswith("champion-summary.json"):
            return [{"id": 22}, {"id": 55}]
        if path.endswith("strawberry-hub.json"):
            return [{"MapDisplayInfoList": [{"value": {
                "Name": "仓库区", "Map": {"ContentId": "map/content", "ItemId": 7}
            }}]}]
        if path.endswith("scope/account"):
            return [{"id": "account/loadout"}]
        calls.append((method, path, json_body))

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    asyncio.run(league_lab.league_update_strawberry_player(league_lab.StrawberryPlayerUpdate(
        champion_id=55, map_item_id=7, difficulty=3, confirmation="我确认修改"
    )))
    asyncio.run(league_lab.league_update_strawberry_map(league_lab.StrawberryMapUpdate(
        content_id="map/content", item_id=7, confirmation="我确认修改"
    )))
    asyncio.run(league_lab.league_update_strawberry_difficulty(league_lab.StrawberryDifficultyUpdate(
        difficulty=3, confirmation="我确认修改"
    )))

    assert calls == [
        ("PUT", "/lol-lobby/v1/lobby/members/localMember/player-slots", [{
            "championId": 55, "positionPreference": "UNSELECTED", "spell1": 7, "spell2": 3
        }]),
        ("PUT", "/lol-lobby/v2/lobby/strawberryMapId", {"contentId": "map/content", "itemId": 7}),
        ("PATCH", "/lol-loadouts/v4/loadouts/account%2Floadout", {"loadout": {
            "STRAWBERRY_DIFFICULTY": {"inventoryType": "STRAWBERRY_LOADOUT_ITEM", "itemId": 3}
        }}),
    ]


def test_profile_background_and_utility_actions_match_upstream_lcu_contract(monkeypatch):
    monkeypatch.setattr(
        league_lab.league_lab_service, "settings", LeagueLabSettings(toolkit_account_actions_enabled=True)
    )
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        if path.endswith("/champions/22.json"):
            return {"skins": [{"id": 22001, "name": "默认", "skinAugments": {"augments": [{
                "contentId": "augment-1", "overlays": [{"path": "overlay"}]
            }]}}]}
        if (method, path) == ("GET", "/lol-regalia/v2/current-summoner/regalia"):
            return {"bannerType": "last-season"}
        if (method, path) == ("GET", "/lol-chat/v1/me"):
            return {"lol": {"bannerIdSelected": "9"}}
        if path.endswith("scope/account"):
            return [{"id": "account/loadout"}]
        calls.append((method, path, json_body))

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    result = asyncio.run(league_lab.league_update_profile_background(league_lab.ProfileBackgroundUpdate(
        champion_id=22, skin_id=22001, augment_id="augment-1", confirmation="我确认修改"
    )))
    for action in ("banner-accent", "remove-prestige-crest", "clear-challenge-tokens", "clear-emotes"):
        asyncio.run(league_lab.league_profile_utility_action(
            league_lab.ProfileUtilityAction(action=action, confirmation="我确认修改")
        ))

    assert result == {"applied": True, "skin_id": 22001, "augment_id": "augment-1"}
    assert calls[0:2] == [
        ("POST", "/lol-summoner/v1/current-summoner/summoner-profile", {"key": "backgroundSkinId", "value": 22001}),
        ("POST", "/lol-summoner/v1/current-summoner/summoner-profile", {"key": "backgroundSkinAugments", "value": "augment-1"}),
    ]
    assert ("POST", "/lol-challenges/v1/update-player-preferences/", {"bannerAccent": "2"}) in calls
    assert ("PUT", "/lol-regalia/v2/current-summoner/regalia", {
        "preferredCrestType": "prestige", "preferredBannerType": "last-season", "selectedPrestigeCrest": 22
    }) in calls
    clear_emotes = next(row for row in calls if row[0:2] == ("PATCH", "/lol-loadouts/v4/loadouts/account%2Floadout"))
    assert len(clear_emotes[2]["loadout"]) == 13
    assert all(value == {"inventoryType": "EMOTE", "itemId": -1} for value in clear_emotes[2]["loadout"].values())


def test_league_client_window_status_reads_native_bounds_and_lcu_zoom(monkeypatch):
    async def request(method, path, *, json_body=None, params=None):
        assert (method, path) == ("GET", "/riotclient/zoom-scale")
        return 0.9

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    monkeypatch.setattr(
        league_lab,
        "_league_client_window_info",
        lambda: {"width": 1152, "height": 648, "left": 10, "top": 20, "dpi": 96, "scale_factor": 1.0, "supported": True},
    )

    result = asyncio.run(league_lab.league_client_window_status())

    assert result["zoom"] == 0.9
    assert result["width"] == 1152


def test_league_client_window_resize_uses_zoom_and_validated_base_size(monkeypatch):
    async def request(method, path, *, json_body=None, params=None):
        return 1.25

    calls = []
    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    monkeypatch.setattr(
        league_lab,
        "_resize_league_client_window",
        lambda width, height, zoom: calls.append((width, height, zoom)) or {"width": 1600, "height": 900},
    )

    result = asyncio.run(league_lab.league_client_window_resize(league_lab.LeagueClientWindowResize(base_width=1280, base_height=720)))

    assert calls == [(1280, 720, 1.25)]
    assert result == {"width": 1600, "height": 900, "applied": True}


def test_champion_icon_is_proxied_without_exposing_lcu_credentials(monkeypatch):
    async def request_bytes(path):
        assert path == "/lol-game-data/assets/v1/champion-icons/22.png"
        return b"png-bytes", "image/png"

    monkeypatch.setattr(league_lab.league_lab_service, "request_bytes", request_bytes)
    response = asyncio.run(league_lab.league_champion_icon(22))

    assert response.body == b"png-bytes"
    assert response.media_type == "image/png"
    assert response.headers["cache-control"] == "private, max-age=86400"


def test_summoner_spell_icon_resolves_catalog_icon_path(monkeypatch):
    async def request(method, path, *, json_body=None, params=None):
        if path.endswith("perkstyles.json"):
            return []
        if path.endswith("summoner-spells.json"):
            return [{"id": 14, "name": "点燃", "iconPath": "/lol-game-data/assets/DATA/Spells/Icons2D/SummonerIgnite.png"}]
        if path.endswith("perks.json"):
            return []
        raise AssertionError(path)

    async def request_bytes(path):
        assert path == "/lol-game-data/assets/DATA/Spells/Icons2D/SummonerIgnite.png"
        return b"spell-png", "image/png"

    league_lab._loadout_catalog_cache.update({"expires_at": 0.0, "payload": None})
    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    monkeypatch.setattr(league_lab.league_lab_service, "request_bytes", request_bytes)
    response = asyncio.run(league_lab.league_summoner_spell_icon(14))

    assert response.body == b"spell-png"
    assert response.media_type == "image/png"
    assert response.headers["cache-control"] == "private, max-age=86400"


def test_summoner_spell_icon_keeps_legacy_path_fallback(monkeypatch):
    async def request(method, path, *, json_body=None, params=None):
        if path.endswith("perkstyles.json"):
            return []
        if path.endswith("summoner-spells.json"):
            return [{"id": 14, "name": "点燃"}]
        if path.endswith("perks.json"):
            return []
        raise AssertionError(path)

    calls = []

    async def request_bytes(path):
        calls.append(path)
        if path.endswith("/14.png"):
            return b"legacy-spell-png", "image/png"
        raise AssertionError(path)

    league_lab._loadout_catalog_cache.update({"expires_at": 0.0, "payload": None})
    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    monkeypatch.setattr(league_lab.league_lab_service, "request_bytes", request_bytes)
    response = asyncio.run(league_lab.league_summoner_spell_icon(14))

    assert response.body == b"legacy-spell-png"
    assert calls == ["/lol-game-data/assets/v1/summoner-spells/14.png"]


def test_item_icon_falls_back_to_data_dragon(monkeypatch):
    async def request_bytes(path):
        assert path == "/lol-game-data/assets/v1/items/3006.png"
        raise RuntimeError("LCU unavailable")

    async def ddragon_icon(item_id):
        assert item_id == 3006
        return b"item-png", "image/png"

    monkeypatch.setattr(league_lab.league_lab_service, "request_bytes", request_bytes)
    monkeypatch.setattr(league_lab, "_ddragon_item_icon", ddragon_icon)
    response = asyncio.run(league_lab.league_item_icon(3006))

    assert response.body == b"item-png"
    assert response.media_type == "image/png"
    assert response.headers["cache-control"] == "private, max-age=86400"


def test_champion_catalog_falls_back_to_data_dragon_without_lcu(tmp_path, monkeypatch):
    async def request(method, path, *, json_body=None, params=None):
        raise RuntimeError("LCU unavailable")

    async def ddragon_catalog():
        return "16.16.1", [{"id": 22, "name": "Ashe", "alias": "Ashe", "roles": ["marksman"]}]

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    monkeypatch.setattr(league_lab, "_champion_catalog_path", lambda: tmp_path / "champions.json")
    monkeypatch.setattr(league_lab, "_ddragon_champion_catalog", ddragon_catalog)

    rows = asyncio.run(league_lab._champion_catalog())

    assert rows == [{"id": 22, "name": "Ashe", "alias": "Ashe", "roles": ["marksman"]}]
    assert json.loads((tmp_path / "champions.json").read_text(encoding="utf-8")) == rows


def test_champion_icon_falls_back_to_data_dragon(monkeypatch):
    async def request_bytes(path):
        raise RuntimeError("LCU unavailable")

    async def ddragon_icon(champion_id):
        assert champion_id == 22
        return b"public-png", "image/png"

    monkeypatch.setattr(league_lab.league_lab_service, "request_bytes", request_bytes)
    monkeypatch.setattr(league_lab, "_ddragon_champion_icon", ddragon_icon)
    response = asyncio.run(league_lab.league_champion_icon(22))

    assert response.body == b"public-png"
    assert response.media_type == "image/png"


def test_profile_icon_falls_back_to_png(monkeypatch):
    calls = []

    async def request_bytes(path):
        calls.append(path)
        if path.endswith(".jpg"):
            raise RuntimeError("missing jpg")
        return b"profile-png", "image/png"

    monkeypatch.setattr(league_lab.league_lab_service, "request_bytes", request_bytes)
    response = asyncio.run(league_lab.league_profile_icon(29))

    assert calls == [
        "/lol-game-data/assets/v1/profile-icons/29.jpg",
        "/lol-game-data/assets/v1/profile-icons/29.png",
    ]
    assert response.body == b"profile-png"
    assert response.media_type == "image/png"


def test_loadout_catalog_enriches_perks_and_proxies_validated_icon(monkeypatch):
    async def request(method, path, *, json_body=None, params=None):
        if path.endswith("perkstyles.json"):
            return [{"id": 8000, "name": "精密", "slots": [{"perks": [{"id": 8010, "name": "征服者"}]}]}]
        if path.endswith("summoner-spells.json"):
            return [{"id": 4, "name": "闪现"}]
        if path.endswith("perks.json"):
            return [{"id": 8010, "name": "征服者", "longDesc": "获得适应之力", "iconPath": "/lol-game-data/assets/v1/perks/8010.png"}]
        raise AssertionError(path)

    async def request_bytes(path):
        assert path == "/lol-game-data/assets/v1/perks/8010.png"
        return b"perk-png", "image/png"

    league_lab._loadout_catalog_cache.update({"expires_at": 0.0, "payload": None})
    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    monkeypatch.setattr(league_lab.league_lab_service, "request_bytes", request_bytes)

    catalog = asyncio.run(league_lab.league_loadout_catalog())
    response = asyncio.run(league_lab.league_perk_icon(8010))

    assert catalog["perks"][0]["name"] == "征服者"
    assert catalog["perks"][0]["long_description"] == "获得适应之力"
    assert catalog["styles"][0]["perks"][0]["icon_path"].endswith("/8010.png")
    assert response.body == b"perk-png"


def test_sgp_server_routing_matches_tencent_and_global_clients():
    assert league_lab._sgp_server_id(league_lab.LcuCredentials(1, "x", "CN", "HN1")) == "TENCENT_HN1"
    assert league_lab._sgp_server_id(league_lab.LcuCredentials(1, "x", "TENCENT", "HN10")) == "TENCENT_HN10"
    assert league_lab._sgp_server_id(league_lab.LcuCredentials(1, "x", "NA", "NA1")) == "NA1"
    assert league_lab._sgp_server_id(league_lab.LcuCredentials(1, "x", "EUW1", "EUW1")) == "EUW"
    assert league_lab._sgp_region_path(league_lab.LcuCredentials(1, "x", "CN", "HN1")) == "HN1"
    assert league_lab._sgp_region_path(league_lab.LcuCredentials(1, "x", "EUW1", "EUW1")) == "EUW1"


def test_sgp_match_rows_normalize_wrapped_summary():
    payload = {
        "games": [{
            "metadata": {"match_id": "HN1_9"},
            "json": {
                "gameId": 9,
                "gameCreation": 123456,
                "gameDuration": 1800,
                "gameMode": "CLASSIC",
                "gameType": "MATCHED_GAME",
                "queueId": 420,
                "participants": [{
                    "puuid": "player-1", "riotIdGameName": "Ashe Main", "riotIdTagline": "HN1",
                    "profileIconId": 29, "championId": 22, "championName": "Ashe",
                    "teamPosition": "BOTTOM", "individualPosition": "BOTTOM",
                    "summoner1Id": 4, "summoner2Id": 7, "kills": 10, "deaths": 2,
                    "assists": 8, "win": True, "totalMinionsKilled": 180,
                    "neutralMinionsKilled": 10, "goldEarned": 14000,
                    "totalDamageDealtToChampions": 25000, "item0": 3006,
                    "challenges": {"kda": 9.0},
                }],
            },
        }],
    }
    rows = league_lab._normalize_sgp_match_rows(payload, {22: "寒冰射手"}, "player-1")

    assert len(rows) == 1
    row = rows[0]
    assert {key: row[key] for key in ("game_id", "played_at", "duration_seconds", "game_mode", "game_type", "queue_id")} == {
        "game_id": 9, "played_at": 123456, "duration_seconds": 1800,
        "game_mode": "CLASSIC", "game_type": "MATCHED_GAME", "queue_id": 420,
    }
    assert {key: row[key] for key in ("champion_name", "kills", "deaths", "assists", "cs", "gold", "damage")} == {
        "champion_name": "寒冰射手", "kills": 10, "deaths": 2, "assists": 8,
        "cs": 190, "gold": 14000, "damage": 25000,
    }
    assert row["items"] == [3006]
    assert row["item_slots"] == [3006, 0, 0, 0, 0, 0, 0]
    assert row["participants"][0]["puuid"] == "player-1"
    assert row["participants"][0]["game_name"] == "Ashe Main"
    assert row["participants"][0]["tag_line"] == "HN1"
    assert row["participants"][0]["profile_icon_id"] == 29
    assert row["participants"][0]["champion_name"] == "寒冰射手"
    assert row["spell1_id"] == 4
    assert row["spell2_id"] == 7
    assert row["spells"] == [4, 7]
    assert row["participants"][0]["spells"] == [4, 7]
    assert row["participants"][0]["raw_stats"]["kills"] == 10


def test_match_item_normalization_accepts_nested_stats_and_slot_arrays():
    payload = {
        "games": [{
            "json": {
                "gameId": 10,
                "participants": [{
                    "puuid": "player-1",
                    "championId": 22,
                    "stats": {"item_slots": [
                        {"itemId": "3006"}, None, {"id": 3031}, 0, 0, 0, {"item_id": 2052}
                    ]},
                }],
            },
        }],
    }
    rows = league_lab._normalize_sgp_match_rows(payload, {22: "寒冰射手"}, "player-1")

    assert rows[0]["items"] == [3006, 3031, 2052]
    assert rows[0]["item_slots"] == [3006, 0, 3031, 0, 0, 0, 2052]
    assert rows[0]["participants"][0]["items"] == [3006, 3031, 2052]
    assert rows[0]["participants"][0]["item_slots"] == [3006, 0, 3031, 0, 0, 0, 2052]


def test_scalar_match_stats_flattens_challenges_and_drops_nested_values():
    result = league_lab._scalar_match_stats({
        "kills": 12,
        "win": True,
        "nested": {"drop": 1},
        "items": [1001],
        "challenges": {"killParticipation": 0.71, "nested": {"drop": 1}},
    })

    assert result == {"kills": 12, "win": True, "challenge.killParticipation": 0.71}


def test_single_jungle_analysis_matches_leagueakari_geometry_and_early_ganks():
    frames = [
        {"participantFrames": {}, "events": []},
        {"participantFrames": {"7": {"position": {"x": 3830, "y": 7880}, "level": 1}}, "events": []},
        {"participantFrames": {"7": {"position": {"x": 4200, "y": 9800}, "level": 2}}, "events": []},
        {
            "participantFrames": {"7": {"position": {"x": 4500, "y": 10100}, "level": 3, "minionsKilled": 0, "jungleMinionsKilled": 16, "damageStats": {"totalDamageDoneToChampions": 100}}},
            "events": [{"type": "CHAMPION_KILL", "timestamp": 175000, "killerId": 7, "assistingParticipantIds": [], "position": {"x": 4300, "y": 10200}}],
        },
        {
            "participantFrames": {"7": {"position": {"x": 10800, "y": 4200}, "level": 4, "damageStats": {"totalDamageDoneToChampions": 220}}},
            "events": [{"type": "CHAMPION_KILL", "timestamp": 220000, "killerId": 2, "assistingParticipantIds": [7], "position": {"x": 10600, "y": 4100}}],
        },
    ]

    result = league_lab._compute_single_jungle_analysis(frames, 7)

    assert result["start_camp"] == {"camp": "blue", "side": "blue"}
    assert result["ganks"] == {"top": 1, "mid": 0, "bot": 1}
    assert result["level3_gank_detected"] is True
    assert result["level4_gank_detected"] is True
    assert result["zone_weights"]["top"] >= 8
    assert result["zone_weights"]["bot"] >= 6


def test_single_jungle_analysis_counts_elite_monster_objectives_by_team():
    """Objective counters must only consume ELITE_MONSTER_KILL events.

    The Python surface exposes team-level objective ownership (matching
    LeagueAkari): a teammate kill counts even when the selected player did
    not assist, while an enemy kill and non-elite events do not.
    """
    frames = [{
        "participantFrames": {},
        "events": [
            {
                "type": "ELITE_MONSTER_KILL",
                "monsterType": "DRAGON",
                "timestamp": 360000,
                "killerId": 7,
                "assistingParticipantIds": [],
            },
            {
                "type": "ELITE_MONSTER_KILL",
                "monsterType": "DRAGON",
                "timestamp": 420000,
                "killerId": 8,
                "assistingParticipantIds": [],
            },
            {
                "type": "ELITE_MONSTER_KILL",
                "monsterType": "DRAGON",
                "timestamp": 480000,
                "killerId": 2,
                "assistingParticipantIds": [],
            },
            {
                "type": "ELITE_MONSTER_KILL",
                "monsterType": "HORDE",
                "timestamp": 510000,
                "killerId": 7,
                "assistingParticipantIds": [],
            },
            {
                "type": "ELITE_MONSTER_KILL",
                "monsterType": "HORDE",
                "timestamp": 520000,
                "killerId": 8,
                "assistingParticipantIds": [],
            },
            {
                "type": "ELITE_MONSTER_KILL",
                "monsterType": "RIFTHERALD",
                "timestamp": 600000,
                "killerId": 7,
                "assistingParticipantIds": [],
            },
            {
                "type": "ELITE_MONSTER_KILL",
                "monsterType": "BARON_NASHOR",
                "timestamp": 900000,
                "killerId": 7,
                "assistingParticipantIds": [],
            },
            {
                "type": "BUILDING_KILL",
                "timestamp": 1000000,
                "killerId": 7,
            },
            {
                "type": "ELITE_MONSTER_KILL",
                "monsterType": "DRAGON_SOUL",
                "timestamp": 1100000,
                "killerId": 7,
                "assistingParticipantIds": [],
            },
        ],
    }]

    result = league_lab._compute_single_jungle_analysis(frames, 7)

    assert result["objectives"] == {
        "dragons": 2,
        "voidgrubs": 2,
        "heralds": 1,
        "barons": 1,
        "first_dragon": True,
        "first_dragon_time_ms": 360000,
    }


def test_aggregate_jungle_objectives_averages_counts_and_first_dragon_time():
    samples = [
        {
            "zone_weights": {}, "total_zone_weight": 0, "ganks": {},
            "objectives": {
                "first_dragon": True, "first_dragon_time_ms": 360000,
                "dragons": 2, "voidgrubs": 3, "heralds": 1, "barons": 0,
            },
        },
        {
            "zone_weights": {}, "total_zone_weight": 0, "ganks": {},
            "objectives": {
                "first_dragon": False, "first_dragon_time_ms": 480000,
                "dragons": 0, "voidgrubs": 1, "heralds": 0, "barons": 2,
            },
        },
    ]

    result = league_lab._aggregate_jungle_analyses(samples)

    assert result["objectives"] == {
        "first_dragon_rate": 0.5,
        "avg_first_dragon_time_seconds": 420.0,
        "avg_dragons": 1.0,
        "avg_voidgrubs": 2.0,
        "avg_heralds": 0.5,
        "avg_barons": 1.0,
    }


def test_single_jungle_analysis_marks_first_dragon_unknown_when_match_has_no_dragon():
    frames = [{"participantFrames": {}, "events": []}]

    result = league_lab._compute_single_jungle_analysis(frames, 7)

    assert result["objectives"]["first_dragon"] is None
    assert result["objectives"]["first_dragon_time_ms"] is None


def test_single_jungle_analysis_keeps_our_first_dragon_time_after_enemy_opener():
    frames = [{
        "participantFrames": {},
        "events": [
            {
                "type": "ELITE_MONSTER_KILL",
                "monsterType": "DRAGON",
                "timestamp": 360000,
                "killerId": 2,
                "killerTeamId": 100,
            },
            {
                "type": "ELITE_MONSTER_KILL",
                "monsterType": "DRAGON",
                "timestamp": 480000,
                "killerId": 7,
                "killerTeamId": 200,
            },
        ],
    }]

    result = league_lab._compute_single_jungle_analysis(frames, 7, team_id=200)

    assert result["objectives"]["first_dragon"] is False
    assert result["objectives"]["dragons"] == 1
    assert result["objectives"]["first_dragon_time_ms"] == 480000


def test_jungle_objectives_preserve_no_dragon_as_unknown_for_rate_denominator():
    """A match without a dragon must not count as a lost first-dragon attempt.

    LeagueAkari represents this state as ``gotFirstDragon = null`` and uses
    only matches with a known first dragon as the rate denominator.  Keeping
    this as an executable parity test exposes regressions where a missing
    objective is silently treated as ``False`` and divided by all games.
    """
    samples = [
        {
            "zone_weights": {}, "total_zone_weight": 0, "ganks": {},
            "objectives": {"first_dragon": True, "first_dragon_time_ms": 360000},
        },
        {
            "zone_weights": {}, "total_zone_weight": 0, "ganks": {},
            "objectives": {"first_dragon": False, "first_dragon_time_ms": 480000},
        },
        {
            "zone_weights": {}, "total_zone_weight": 0, "ganks": {},
            "objectives": {"first_dragon": None, "first_dragon_time_ms": None},
        },
    ]

    result = league_lab._aggregate_jungle_analyses(samples)

    assert result["objectives"]["first_dragon_rate"] == pytest.approx(0.5)


def test_aggregate_jungle_analysis_generates_local_unsent_draft():
    samples = [
        {"zone_weights": {"top": 10, "mid": 2, "bot": 1}, "total_zone_weight": 13, "ganks": {"top": 2, "mid": 0, "bot": 0}, "start_camp": {"side": "blue", "camp": "blue"}, "level3_gank_detected": True, "level4_gank_detected": False},
        {"zone_weights": {"top": 7, "mid": 3, "bot": 2}, "total_zone_weight": 12, "ganks": {"top": 1, "mid": 1, "bot": 0}, "start_camp": {"side": "blue", "camp": "blue"}, "level3_gank_detected": False, "level4_gank_detected": True},
    ]

    result = league_lab._aggregate_jungle_analyses(samples)

    assert result["games_analyzed"] == 2
    assert result["preferred_lane"] == "top"
    assert result["preferred_start_camp"] == "blue:blue"
    assert result["early_gank"] == {"level3_rate": 0.5, "level4_rate": 0.5}
    assert "近 2 场打野时间线" in result["draft"]


def test_jungle_game_participant_accepts_position_or_smite():
    game = {
        "participantIdentities": [{"participantId": 2, "player": {"puuid": "p1"}}],
        "participants": [{"participantId": 2, "teamPosition": "TOP", "spell1Id": 11, "spell2Id": 4}],
    }

    assert league_lab._jungle_game_participant(game, "p1")["participantId"] == 2
    assert league_lab._jungle_game_participant(game, "missing") is None


def test_sgp_ranked_rows_normalize_division_and_queue_map():
    result = league_lab._normalize_sgp_ranked({
        "queues": [{
            "queueType": "RANKED_SOLO_5x5",
            "tier": "GOLD",
            "rank": "II",
            "leaguePoints": 55,
            "wins": 20,
            "losses": 10,
        }],
    })

    assert result["source"] == "sgp"
    assert result["queues"][0]["division"] == "II"
    assert result["queueMap"]["RANKED_SOLO_5x5"]["leaguePoints"] == 55


def test_sgp_summoner_normalizes_ledge_payload(monkeypatch):
    async def common(method, path, *, json_body=None):
        assert method == "POST"
        assert path == "/summoner-ledge/v1/regions/HN1/summoners/puuids"
        assert json_body == ["player-1"]
        return [{"id": 7, "puuid": "player-1", "name": "Fallback", "level": 30, "profileIconId": 29}]

    league_lab.league_lab_service.credentials = league_lab.LcuCredentials(1, "x", "CN", "HN1")
    monkeypatch.setattr(league_lab, "_sgp_common_request", common)
    result = asyncio.run(league_lab._sgp_summoner_by_puuid("player-1"))

    assert result["summonerId"] == 7
    assert result["displayName"] == "Fallback"
    assert result["source"] == "sgp"


def test_player_bundle_falls_back_to_sgp_history(monkeypatch):
    async def request(method, path, *, json_body=None, params=None):
        if path.startswith("/lol-ranked/"):
            return {}
        if path.startswith("/lol-champion-mastery/"):
            return []
        if path.startswith("/lol-match-history/"):
            return {"games": {"games": []}}
        raise AssertionError(path)

    async def sgp(puuid, beg_index, count):
        assert (puuid, beg_index, count) == ("player-1", 0, 20)
        return {"games": [{"json": {"gameId": 1, "participants": [{"puuid": "player-1", "championId": 22}]}}]}

    async def names():
        return {22: "Ashe"}

    async def ranked(puuid):
        assert puuid == "player-1"
        return {"queues": [{"queueType": "RANKED_SOLO_5x5", "tier": "GOLD", "division": "II"}]}

    async def challenges(puuid):
        assert puuid == "player-1"
        return {}

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    monkeypatch.setattr(league_lab, "_sgp_match_history", sgp)
    monkeypatch.setattr(league_lab, "_sgp_ranked_stats", ranked)
    monkeypatch.setattr(league_lab, "_sgp_player_challenges", challenges)
    monkeypatch.setattr(league_lab, "_champion_names", names)
    result = asyncio.run(league_lab._load_player_bundle({"puuid": "player-1"}))

    assert result["match_source"] == "sgp"
    assert result["ranked_source"] == "sgp"
    assert result["matches"][0]["game_id"] == 1


def test_lcu_match_history_completion_fetches_game_details_and_keeps_summary_fallback(monkeypatch):
    calls = []
    detailed = {
        "gameId": 101,
        "participants": [{"participantId": 1, "teamId": 100, "stats": {"win": True}}],
    }

    async def request(method, path, *, json_body=None, params=None):
        calls.append((method, path))
        if path.endswith("/101"):
            return detailed
        if path.endswith("/102"):
            raise RuntimeError("optional detail unavailable")
        raise AssertionError((method, path))

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    summary_101 = {"gameId": 101, "queueId": 420}
    summary_102 = {"gameId": 102, "queueId": 450}
    result = asyncio.run(league_lab._complete_lcu_match_history({
        "games": {"games": [summary_101, summary_102]},
    }))

    assert result["games"]["games"][0] == detailed
    assert result["games"]["games"][1] == summary_102
    assert sorted(calls) == [
        ("GET", "/lol-match-history/v1/games/101"),
        ("GET", "/lol-match-history/v1/games/102"),
    ]


def test_lcu_match_rows_normalize_detail_identity_positions_items_perks_and_team_stats():
    def stats(**overrides):
        value = {
            "win": True,
            "kills": 12,
            "deaths": 3,
            "assists": 8,
            "goldEarned": 12000,
            "goldSpent": 11000,
            "champLevel": 18,
            "totalMinionsKilled": 100,
            "neutralMinionsKilled": 5,
            "totalDamageDealtToChampions": 20000,
            "totalDamageTaken": 9000,
            "damageDealtToTurrets": 1000,
            "totalHeal": 500,
            "totalTimeCCDealt": 40,
            "visionScore": 30,
            "perkPrimaryStyle": 8000,
            "perkSubStyle": 8300,
            "perk0": 8100,
            "perk1": 8110,
            "perk2": 8120,
            "perk3": 8130,
            "perk4": 8210,
            "perk5": 8230,
            "perk0Var1": 1,
            "perk0Var2": 2,
            "perk0Var3": 3,
            "item0": 100,
            "item1": 0,
            "item2": 102,
            "item3": 0,
            "item4": 104,
            "item5": 0,
            "item6": 106,
            "playerAugment1": 201,
            "playerAugment2": 0,
            "playerAugment3": 203,
            "playerAugment4": 0,
            "playerAugment5": 0,
            "playerAugment6": 206,
            "gameEndedInEarlySurrender": False,
            "gameEndedInSurrender": False,
            "teamEarlySurrendered": False,
        }
        value.update(overrides)
        return value

    game = {
        "gameId": 101,
        "gameCreationDate": 1720000000000,
        "gameDuration": 1800,
        "gameMode": "CLASSIC",
        "gameType": "MATCHED_GAME",
        "gameVersion": "15.16.1",
        "mapId": 11,
        "queueId": 420,
        "endOfGameResult": "GameComplete",
        "teams": [
            {"teamId": 100, "win": "Win", "bans": [{"championId": 55}]},
            {"teamId": 200, "win": "Fail", "bans": []},
        ],
        "participantIdentities": [
            {"participantId": 1, "player": {
                "puuid": "p1", "gameName": "Alpha#CN1", "tagLine": "CN1", "profileIcon": 9,
            }},
            {"participantId": 2, "player": {
                "puuid": "p2", "gameName": "Beta", "tagLine": "CN2", "profileIcon": 10,
            }},
        ],
        "participants": [
            {"participantId": 1, "teamId": 100, "championId": 11, "spell1Id": 4, "spell2Id": 12,
             "teamPosition": "NONE", "individualPosition": "UNSELECTED", "stats": stats()},
            {"participantId": 2, "teamId": 200, "championId": 22, "spell1Id": 7, "spell2Id": 14,
             "teamPosition": "MID", "individualPosition": "MID", "stats": stats(
                 win=False, kills=3, deaths=10, assists=2, item0=300, item2=0, item4=0, item6=0,
                 playerAugment1=0, playerAugment3=0, playerAugment6=0,
             )},
        ],
    }

    rows = league_lab._normalize_match_rows({"games": {"games": [game]}}, {11: "A", 22: "B"}, "p1")
    assert len(rows) == 1
    match = rows[0]
    assert match["participant_puuid"] == "p1"
    assert match["riot_id"] == "Alpha#CN1"
    assert match["game_name"] == "Alpha"
    assert match["tag_line"] == "CN1"
    assert match["teamIdentifier"] == "TEAM-100"
    assert match["position"] is None
    assert match["role"] is None
    assert match["winResult"] == "win"
    assert match["isSurrender"] is False

    own = match["participants"][0]
    assert own["spells"] == [4, 12]
    assert own["teamIdentifier"] == "TEAM-100"
    assert own["items"] == [100, 102, 104, 106]
    assert own["item_slots"] == [100, 0, 102, 0, 104, 0, 106]
    assert own["augments"] == [201, 203, 206]
    assert own["augment_slots"] == [201, 0, 203, 0, 0, 206]
    assert own["perk_styles"][0]["style"] == 8000
    assert len(own["perk_styles"][0]["selections"]) == 4
    assert len(own["perk_styles"][1]["selections"]) == 2

    opponent = match["participants"][1]
    assert opponent["position"] == "MIDDLE"
    assert opponent["riot_id"] == "Beta#CN2"
    assert match["team_stats"]["TEAM-100"]["total_kills"] == 12
    assert match["team_stats"]["TEAM-200"]["total_kills"] == 3
    assert match["teams"][0]["bans"] == [{"championId": 55}]


def test_match_collection_persists_and_deduplicates(tmp_path, monkeypatch):
    path = tmp_path / "league.db"
    monkeypatch.setattr(league_lab, "_league_collection_db_path", lambda: path)
    first = {"game_id": 7, "played_at": 100, "kills": 1}
    updated = {"game_id": 7, "played_at": 100, "kills": 9}
    second = {"game_id": 8, "played_at": 200, "kills": 2}

    assert asyncio.run(league_lab._store_match_collection("player-1", [first])) == 1
    assert asyncio.run(league_lab._store_match_collection("player-1", [updated, second])) == 2

    rows = asyncio.run(league_lab._read_match_collection("player-1"))
    assert [row["game_id"] for row in rows] == [8, 7]
    assert rows[1]["kills"] == 9
    assert asyncio.run(league_lab._match_collection_count("player-1")) == 2


def test_skin_selector_only_exposes_owned_enabled_skins():
    result = league_lab._normalize_skin_selector(
        {"showSkinSelector": True, "selectedSkinId": 22001, "selectedChampionId": 22},
        [
            {"id": 22000, "name": "Base", "unlocked": True, "disabled": False, "splashPath": "/base", "childSkins": []},
            {"id": 22001, "name": "Owned", "unlocked": True, "disabled": False, "splashPath": "/owned", "childSkins": [
                {"id": 22002, "name": "Chroma", "unlocked": True, "disabled": False, "chromaPreviewPath": "/chroma"},
                {"id": 22003, "name": "Locked chroma", "unlocked": False, "disabled": False},
            ]},
            {"id": 22004, "name": "Locked", "unlocked": False, "disabled": False, "childSkins": []},
        ],
    )

    assert result["available"] is True
    assert result["selected_skin_id"] == 22001
    assert [row["id"] for row in result["skins"]] == [22000, 22001, 22002]
    assert result["skins"][-1]["is_chroma"] is True


def test_cooldown_timer_state_orders_enemies_and_exposes_spell_cooldowns(monkeypatch):
    service = league_lab.league_lab_service
    service.settings = LeagueLabSettings(cooldown_timer_enabled=True, cooldown_timer_type="countdown")
    service.phase = "InProgress"
    service.current_summoner = {"puuid": "self"}

    async def request(method, path, *, json_body=None, params=None):
        if path == "/lol-gameflow/v1/session":
            return {"phase": "InProgress", "gameData": {
                "queue": {"gameMode": "CLASSIC"},
                "teamOne": [{"puuid": "self", "selectedPosition": "MIDDLE"}],
                "teamTwo": [
                    {"puuid": "enemy-support", "selectedPosition": "UTILITY"},
                    {"puuid": "enemy-top", "selectedPosition": "TOP"},
                ],
                "playerChampionSelections": [
                    {"puuid": "enemy-support", "championId": 40, "spell1Id": 4, "spell2Id": 14},
                    {"puuid": "enemy-top", "championId": 24, "spell1Id": 4, "spell2Id": 12},
                ],
            }}
        if path == "/lol-game-data/assets/v1/summoner-spells.json":
            return [{"id": 4, "name": "闪现", "cooldown": 300}, {"id": 14, "name": "点燃", "cooldown": 180}, {"id": 12, "name": "传送", "cooldown": 360}]
        raise AssertionError(path)

    async def names():
        return {24: "贾克斯", 40: "迦娜"}

    class FakeResponse:
        def raise_for_status(self): return None
        def json(self): return {"gameTime": 125.5}

    class FakeClient:
        async def __aenter__(self): return self
        async def __aexit__(self, *_args): return None
        async def get(self, _url): return FakeResponse()

    monkeypatch.setattr(service, "request", request)
    monkeypatch.setattr(league_lab, "_champion_names", names)
    monkeypatch.setattr(league_lab.httpx, "AsyncClient", lambda **_kwargs: FakeClient())
    result = asyncio.run(league_lab.league_cooldown_timer_state())

    assert [row["puuid"] for row in result["players"]] == ["enemy-top", "enemy-support"]
    assert result["spells"][4]["cooldown"] == 300
    assert result["game_time"] == 125.5
    assert result["ability_haste"] == 0


def test_cooldown_timer_send_requires_opt_in_and_foreground_guard(monkeypatch):
    service = league_lab.league_lab_service
    service.settings = LeagueLabSettings(cooldown_timer_enabled=False)
    try:
        asyncio.run(league_lab.league_cooldown_timer_send(league_lab.InGameTextSend(text="闪现 10:00")))
    except league_lab.HTTPException as exc:
        assert exc.status_code == 409
    else:
        raise AssertionError("disabled timer must not inject input")

    service.settings = LeagueLabSettings(cooldown_timer_enabled=True)
    monkeypatch.setattr(league_lab, "_send_text_to_foreground_league_game", lambda text: 777 if text == "闪现 10:00" else 0)
    result = asyncio.run(league_lab.league_cooldown_timer_send(league_lab.InGameTextSend(text="闪现 10:00")))
    assert result == {"sent": True, "pid": 777}


def test_skin_change_rejects_skin_outside_owned_snapshot(monkeypatch):
    monkeypatch.setattr(
        league_lab.league_lab_service,
        "settings",
        LeagueLabSettings(toolkit_account_actions_enabled=True),
    )
    async def request(method, path, **_kwargs):
        assert (method, path) == ("GET", "/lol-gameflow/v1/gameflow-phase")
        return "ChampSelect"
    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    league_lab.league_lab_service.champ_select = {
        "skin_selector": {"skins": [{"id": 22001}], "disabled": False}
    }
    try:
        asyncio.run(league_lab.league_champ_select_skin(99999))
    except league_lab.HTTPException as exc:
        assert exc.status_code == 409
    else:
        raise AssertionError("unowned skin should be rejected")


def test_chat_presence_update_is_explicit_and_uses_lcu(monkeypatch):
    calls = []
    monkeypatch.setattr(
        league_lab.league_lab_service, "settings", LeagueLabSettings(toolkit_account_actions_enabled=True)
    )

    async def request(method, path, *, json_body=None, params=None):
        calls.append((method, path, json_body))
        if method == "GET":
            return {"availability": "away", "statusMessage": "休息中"}
        return None

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    result = asyncio.run(league_lab.league_update_chat_presence(
        league_lab.ChatPresenceUpdate(availability="away", status_message="休息中")
    ))

    assert calls[0] == ("PUT", "/lol-chat/v1/me", {"availability": "away", "statusMessage": "休息中"})
    assert result["chat_presence"]["availability"] == "away"


def test_chat_ready_automation_applies_status_and_apex_rank_once(monkeypatch):
    service = LeagueLabService()
    service.credentials = league_lab.LcuCredentials(port=1, token="fixture")
    service.settings = LeagueLabSettings(
        automation_enabled=True,
        auto_set_status_message_enabled=True,
        status_message="今晚打排位",
        auto_set_ranked_status_enabled=True,
        ranked_status={"queue": "RANKED_SOLO_5x5", "tier": "CHALLENGER", "division": "IV"},
    )
    service._chat_ready_since = time.monotonic() - 3
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        calls.append((method, path, json_body))
        if method == "GET":
            return {"availability": "chat", "statusMessage": ""}
        return None

    monkeypatch.setattr(service, "request", request)
    asyncio.run(service._run_chat_ready_automation())
    asyncio.run(service._run_chat_ready_automation())

    writes = [row for row in calls if row[0] == "PUT"]
    assert writes == [
        ("PUT", "/lol-chat/v1/me", {"statusMessage": "今晚打排位"}),
        ("PUT", "/lol-chat/v1/me", {"lol": {"rankedLeagueQueue": "RANKED_SOLO_5x5", "rankedLeagueTier": "CHALLENGER"}}),
    ]
    assert service._chat_ready_automation_done is True


def test_chat_ready_automation_never_writes_when_master_switch_is_off(monkeypatch):
    service = LeagueLabService()
    service.credentials = league_lab.LcuCredentials(port=1, token="fixture")
    service.settings = LeagueLabSettings(
        automation_enabled=False,
        auto_set_status_message_enabled=True,
        status_message="不会写入",
    )
    service._chat_ready_since = time.monotonic() - 3
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        calls.append((method, path, json_body))
        return {"availability": "chat"}

    monkeypatch.setattr(service, "request", request)
    asyncio.run(service._run_chat_ready_automation())

    assert calls == [("GET", "/lol-chat/v1/me", None)]
    assert service._chat_ready_automation_done is True


def test_manual_ranked_status_applies_division_and_interrupts_login_automation(monkeypatch):
    calls = []
    monkeypatch.setattr(
        league_lab.league_lab_service, "settings", LeagueLabSettings(toolkit_account_actions_enabled=True)
    )

    async def request(method, path, *, json_body=None, params=None):
        calls.append((method, path, json_body))

    service = league_lab.league_lab_service
    service._chat_ready_automation_done = False
    monkeypatch.setattr(service, "request", request)
    result = asyncio.run(league_lab.league_update_ranked_status(
        league_lab.RankedStatusUpdate(queue="RANKED_FLEX_SR", tier="GOLD", division="II")
    ))

    assert calls == [("PUT", "/lol-chat/v1/me", {"lol": {
        "rankedLeagueQueue": "RANKED_FLEX_SR",
        "rankedLeagueTier": "GOLD",
        "rankedLeagueDivision": "II",
    }})]
    assert result["ranked_status"] == {"queue": "RANKED_FLEX_SR", "tier": "GOLD", "division": "II"}
    assert service._chat_ready_automation_done is True


def test_manual_chat_preset_sends_to_champion_select(monkeypatch):
    calls = []
    monkeypatch.setattr(
        league_lab.league_lab_service, "settings", LeagueLabSettings(toolkit_account_actions_enabled=True)
    )

    async def request(method, path, *, json_body=None, params=None):
        calls.append((method, path, json_body))
        if method == "GET":
            return [{"id": "champ", "type": "championSelect"}]
        return None

    league_lab.league_lab_service.phase = "ChampSelect"
    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    result = asyncio.run(league_lab.league_send_chat_message(league_lab.ChatMessageSend(lines=["第一行", "第二行"])))

    assert calls[-1] == ("POST", "/lol-chat/v1/conversations/champ/messages", {"body": "第一行\n第二行", "type": "chat"})
    assert result["line_count"] == 2


def test_terminate_game_client_endpoint_uses_foreground_guard(monkeypatch):
    monkeypatch.setattr(
        league_lab.league_lab_service,
        "settings",
        LeagueLabSettings(toolkit_account_actions_enabled=True),
    )
    monkeypatch.setattr(league_lab, "_terminate_foreground_league_game_client", lambda: 4242)

    result = asyncio.run(league_lab.league_terminate_game_client(
        league_lab.TerminateGameClientRequest(confirmation="我确认结束游戏")
    ))

    assert result == {"terminated": True, "pid": 4242}


def test_terminate_game_client_endpoint_preserves_guard_error(monkeypatch):
    monkeypatch.setattr(
        league_lab.league_lab_service,
        "settings",
        LeagueLabSettings(toolkit_account_actions_enabled=True),
    )
    def blocked():
        raise RuntimeError("当前前台窗口不是 League 游戏进程，未执行任何操作")

    monkeypatch.setattr(league_lab, "_terminate_foreground_league_game_client", blocked)
    try:
        asyncio.run(league_lab.league_terminate_game_client(
            league_lab.TerminateGameClientRequest(confirmation="我确认结束游戏")
        ))
    except league_lab.HTTPException as exc:
        assert exc.status_code == 409
        assert "未执行任何操作" in str(exc.detail)
    else:
        raise AssertionError("foreground guard must block termination")


def test_game_settings_file_mode_uses_tencent_game_config(tmp_path, monkeypatch):
    install_root = tmp_path / "LeagueClient"
    install_root.mkdir()
    settings_path = tmp_path / "Game" / "Config" / "PersistedSettings.json"
    settings_path.parent.mkdir(parents=True)
    settings_path.write_text("{}", encoding="utf-8")

    async def request(method, path, **_kwargs):
        assert (method, path) == ("GET", "/data-store/v1/install-dir")
        return str(install_root)

    original = league_lab.league_lab_service.credentials
    original_settings = league_lab.league_lab_service.settings
    league_lab.league_lab_service.credentials = league_lab.LcuCredentials(1, "token", region="TENCENT")
    league_lab.league_lab_service.settings = league_lab.LeagueLabSettings(toolkit_account_actions_enabled=True)
    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    try:
        readonly = asyncio.run(
            league_lab.league_game_settings_file_update(
                league_lab.GameSettingsFileModeUpdate(mode="readonly")
            )
        )
        assert readonly["mode"] == "readonly"
        writable = asyncio.run(
            league_lab.league_game_settings_file_update(
                league_lab.GameSettingsFileModeUpdate(mode="writable")
            )
        )
        assert writable["mode"] == "writable"
    finally:
        league_lab.league_lab_service.credentials = original
        league_lab.league_lab_service.settings = original_settings
        settings_path.chmod(stat.S_IREAD | stat.S_IWRITE)


def test_removed_opgg_settings_are_not_part_of_current_model():
    settings = LeagueLabSettings()
    assert not any(key.startswith("opgg_") for key in settings.model_dump())
    assert settings.mini_opacity == 1.0
    assert settings.mini_show_skin_selector is True


def test_match_history_and_ongoing_navigation_defaults_are_safe():
    settings = LeagueLabSettings()
    assert settings.match_history_refresh_after_game is True
    assert settings.match_history_load_count == 20
    assert settings.ongoing_auto_route_when_game_starts is False
    assert settings.ongoing_match_history_load_count == 20
    assert settings.ongoing_query_concurrency == 4
    assert settings.ongoing_game_details_load_count == 20
    assert settings.ongoing_match_history_tag_preference == "current"
    assert settings.ongoing_order_player_by == "default"
    assert settings.ongoing_query_in_lobby_phase is True
    assert settings.ongoing_premade_threshold == 5
    assert settings.ongoing_jungle_analysis_count == 4
    assert settings.ongoing_show_champion_usage is True
    assert settings.ongoing_champion_usage_mode == "recent"
    assert settings.ongoing_show_match_history_item_border is False
    assert settings.ongoing_show_jungle_pathing is True
    assert settings.ongoing_show_jungle_pathing_for_all_players is False
    assert settings.ongoing_show_premade_tag is True
    assert settings.ongoing_show_local_tag is True
    assert settings.ongoing_show_streak_tags is True
    assert settings.ongoing_show_performance_tags is True
    assert settings.ongoing_player_card_tag_settings["great-performance"] is True
    assert settings.ongoing_player_card_tag_settings["average-team-damage"] is False


def test_legacy_ongoing_champion_visibility_migrates_to_three_state_mode():
    settings = LeagueLabSettings.model_validate({"ongoing_show_champion_usage": False})
    assert settings.ongoing_champion_usage_mode == "none"


def test_ongoing_performance_tags_cover_streaks_and_recent_form():
    matches = [
        {
            "win": True,
            "kills": 8,
            "deaths": 2,
            "assists": 6,
            "cs": 240,
            "duration_seconds": 1800,
            "vision_score": 40,
            "challenges": {"soloKills": 2},
        }
        for _ in range(5)
    ]
    tags = league_lab._ongoing_performance_tags(matches, tag_settings={"average-cs-per-minute": True})
    ids = {tag["id"] for tag in tags}
    assert {"winning-streak", "high-win-rate", "great-kda", "average-cs-per-minute"}.issubset(ids)
    assert all(tag.get("title") for tag in tags)


def test_ongoing_performance_tags_match_granular_upstream_metrics():
    team = [
        {"team_id": 100, "kills": 4, "damage": 10000, "damage_taken": 9000, "gold": 10000}
        for _ in range(5)
    ]
    matches = [{
        "win": True,
        "team_id": 100,
        "participants": team,
        "kills": 12,
        "deaths": 2,
        "assists": 8,
        "damage": 24000,
        "damage_taken": 9000,
        "gold": 12000,
        "duration_seconds": 1800,
        "spell1_id": 4 if index % 2 else 14,
        "spell2_id": 14 if index % 2 else 4,
    } for index in range(8)]
    tags = league_lab._ongoing_performance_tags(matches, tag_settings={
        "average-team-damage": True,
        "average-team-gold": True,
        "akari-score": True,
    })
    ids = {tag["id"] for tag in tags}
    assert {"average-team-damage", "average-team-gold", "suspicious-flash-position", "akari-score"}.issubset(ids)


def test_ongoing_performance_tags_respect_visibility_settings():
    matches = [{"win": False, "kills": 0, "deaths": 8, "assists": 1} for _ in range(5)]
    assert league_lab._ongoing_performance_tags(matches, show_streaks=False, show_performance=False) == []


def test_ongoing_akari_score_matches_the_upstream_weighting_shape():
    teammates = [
        {
            "team_id": 100,
            "kills": 4,
            "damage": 10000,
            "damage_taken": 9000,
            "healing": 500,
            "cs": 180,
            "gold": 10000,
            "vision_score": 20,
        }
        for _ in range(5)
    ]
    baseline = {
        "team_id": 100,
        "participants": teammates,
        "win": False,
        "kills": 4,
        "deaths": 4,
        "assists": 4,
        "damage": 10000,
        "damage_taken": 9000,
        "healing": 500,
        "cs": 180,
        "gold": 10000,
        "vision_score": 20,
        "duration_seconds": 1800,
    }
    stronger = {
        **baseline,
        "win": True,
        "kills": 12,
        "deaths": 2,
        "assists": 8,
        "damage": 25000,
        "cs": 300,
    }

    assert league_lab._ongoing_akari_score([]) == 0.0
    assert 0 < league_lab._ongoing_akari_score([baseline]) < 18
    assert league_lab._ongoing_akari_score([stronger]) > league_lab._ongoing_akari_score([baseline])


def test_ongoing_rating_summary_matches_upstream_preset_metrics():
    matches = [{
        "win": True, "kills": 8, "deaths": 2, "assists": 6,
        "duration_seconds": 1800, "cs": 210, "vision_score": 30,
        "damage": 20000, "damage_taken": 10000, "gold": 12000,
        "champion_id": 1, "champion_name": "Annie", "position": "MIDDLE",
        "team_id": 100, "challenges": {"soloKills": 2},
        "participants": [
            {"team_id": 100, "kills": 8, "damage": 20000, "damage_taken": 10000, "gold": 12000},
            {"team_id": 100, "kills": 4, "damage": 10000, "damage_taken": 10000, "gold": 8000},
        ],
    }]

    summary = league_lab._ongoing_rating_summary(matches)

    assert summary["sample_count"] == 1
    assert summary["win_rate"] == 1
    assert summary["avg_kda"] == 7
    assert summary["avg_solo_kills"] == 2
    assert summary["avg_vision_score"] == 30
    assert summary["avg_champion_damage_percentage_of_team"] == pytest.approx(2 / 3)
    assert summary["avg_damage_taken_percentage_of_team"] == .5
    assert summary["avg_gold_percentage_of_team"] == .6
    assert summary["avg_cs_per_minute"] == 7
    assert summary["avg_kill_participation"] == pytest.approx(14 / 12)
    assert summary["avg_damage_gold_efficiency"] == pytest.approx(5 / 3)
    assert summary["main_champions"] == [{"champion_id": 1, "champion_name": "Annie", "count": 1}]
    assert summary["main_positions"] == [{"position": "MIDDLE", "count": 1}]


def test_ongoing_rating_summary_keeps_unproven_metrics_null():
    summary = league_lab._ongoing_rating_summary([{
        "kills": 1, "deaths": 1, "assists": 1, "duration_seconds": 0,
        "damage": 100, "gold": 100, "participants": [], "challenges": {},
    }])

    assert summary["avg_solo_kills"] is None
    assert summary["avg_champion_damage_percentage_of_team"] is None
    assert summary["avg_damage_taken_percentage_of_team"] is None
    assert summary["avg_gold_percentage_of_team"] is None
    assert summary["avg_kill_participation"] is None
    assert summary["avg_cs_per_minute"] is None


def test_ongoing_game_reads_lobby_filters_current_queue_and_sorts(monkeypatch):
    def history(puuid, rows):
        games = []
        for index, (queue_id, win) in enumerate(rows, start=1):
            games.append({
                "gameId": f"{puuid}-{index}",
                "queueId": queue_id,
                "gameDuration": 1800,
                "participantIdentities": [{"participantId": 1, "player": {"puuid": puuid}}],
                "participants": [{
                    "participantId": 1,
                    "teamId": 100,
                    "championId": 1,
                    "stats": {
                        "win": win,
                        "kills": 8 if win else 2,
                        "deaths": 2 if win else 8,
                        "assists": 6,
                        "goldEarned": 10000,
                        "totalMinionsKilled": 180,
                        "totalDamageDealtToChampions": 15000,
                        "totalDamageTaken": 10000,
                        "visionScore": 20,
                    },
                }],
            })
        return {"games": {"games": games}}

    histories = {
        "high": history("high", [(420, True)]),
        "low": history("low", [(450, True), (420, False)]),
    }
    calls = []

    async def request(method, path, *, json_body=None, params=None):
        calls.append((method, path, params))
        if path == "/lol-gameflow/v1/session":
            return {"phase": "Lobby", "gameData": {}}
        if path == "/lol-lobby/v2/lobby":
            return {
                "partyId": "party-1",
                "gameConfig": {"queueId": 420},
                "members": [
                    {"puuid": "low", "selectedPosition": "TOP"},
                    {"puuid": "high", "selectedPosition": "JUNGLE"},
                ],
            }
        if path.startswith("/lol-summoner/v2/summoners/puuid/"):
            puuid = path.rsplit("/", 1)[-1]
            return {"puuid": puuid, "gameName": puuid.title(), "profileIconId": 10}
        if path.startswith("/lol-ranked/v1/ranked-stats/"):
            return {}
        if "/matches" in path:
            puuid = path.split("/lol/")[1].split("/matches")[0]
            return histories[puuid]
        raise AssertionError(f"unexpected LCU request: {method} {path}")

    async def champion_names():
        return {1: "Annie"}

    settings = LeagueLabSettings(
        ongoing_order_player_by="win-rate",
        ongoing_match_history_tag_preference="current",
        ongoing_show_jungle_pathing=False,
        ongoing_show_premade_tag=False,
    )
    monkeypatch.setattr(league_lab.league_lab_service, "settings", settings)
    monkeypatch.setattr(league_lab.league_lab_service, "phase", "Lobby")
    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    monkeypatch.setattr(league_lab, "_champion_names", champion_names)
    player_tag_reads = []
    monkeypatch.setattr(league_lab, "_read_player_tags", lambda: player_tag_reads.append(True) or {})
    remembered = []
    monkeypatch.setattr(league_lab, "_remember_recent_players", lambda *args: remembered.append(args))
    monkeypatch.setattr(league_lab, "_ongoing_cache", {"key": "", "expires_at": 0.0, "payload": None})

    result = asyncio.run(league_lab.league_ongoing_game())

    assert result["query_stage"] == "lobby"
    assert result["game_id"] == "party-1"
    assert result["queue"]["queueId"] == 420
    assert [row["puuid"] for row in result["players"]] == ["high", "low"]
    assert result["players"][0]["recent"]["matches"] == 1
    assert result["players"][0]["recent"]["wins"] == 1
    assert result["players"][0]["recent"]["average_kda"] == 7.0
    assert result["players"][0]["recent"]["details_analyzed"] == 1
    assert result["players"][0]["recent_matches"][0]["game_id"] == "high-1"
    assert result["players"][0]["recent_matches"][0]["queue_id"] == 420
    assert result["players"][0]["recent_matches"][0]["win"] is True
    assert result["players"][1]["recent"]["matches"] == 1
    assert result["players"][1]["recent"]["wins"] == 0
    assert all(player["team"] == "LOBBY" for player in result["players"])
    assert all(player["champion_name"] == "" for player in result["players"])
    assert result["teams"]["LOBBY"] == ["high", "low"]
    assert result["analysis"]["players"]["high"]["winLoss"]["all"]["winRate"] == 1.0
    assert result["analysis"]["teams"]["LOBBY"]["games"] == 2
    assert remembered == []
    assert len(player_tag_reads) == 1
    assert any(path == "/lol-lobby/v2/lobby" for _, path, _ in calls)


def test_ongoing_game_preserves_partial_player_data_when_optional_endpoint_fails(monkeypatch):
    async def request(method, path, *, json_body=None, params=None):
        if path == "/lol-gameflow/v1/session":
            return {
                "phase": "InProgress",
                "gameData": {
                    "gameId": 123,
                    "queue": {"id": 2400},
                    "playerChampionSelections": [{"puuid": "partial", "championId": 1, "team": 100}],
                },
            }
        if path == "/lol-summoner/v2/summoners/puuid/partial":
            return {"puuid": "partial", "gameName": "Partial Player", "profileIconId": 10}
        if path == "/lol-ranked/v1/ranked-stats/partial":
            raise RuntimeError("LCU request failed: HTTPStatusError")
        if path == "/lol-match-history/v1/products/lol/partial/matches":
            return {"games": {"games": []}}
        raise AssertionError(f"unexpected LCU request: {method} {path}")

    async def champion_names():
        return {1: "Annie"}

    monkeypatch.setattr(
        league_lab.league_lab_service,
        "settings",
        LeagueLabSettings(ongoing_show_jungle_pathing=False, ongoing_show_premade_tag=False),
    )
    monkeypatch.setattr(league_lab.league_lab_service, "phase", "InProgress")
    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    monkeypatch.setattr(league_lab, "_champion_names", champion_names)
    monkeypatch.setattr(league_lab, "_read_player_tags", lambda: {})
    monkeypatch.setattr(league_lab, "_remember_recent_players", lambda *args: None)
    monkeypatch.setattr(league_lab, "_ongoing_cache", {"key": "", "expires_at": 0.0, "payload": None})

    result = asyncio.run(league_lab.league_ongoing_game())

    player = result["players"][0]
    assert player["summoner"]["gameName"] == "Partial Player"
    assert player["ranked"] == {}
    assert player["data_availability"] == {
        "summoner": True,
        "ranked": False,
        "history": True,
        "mastery": True,
        "unavailable": ["ranked"],
        "history_source": "lcu",
    }


def test_ongoing_game_falls_back_to_sgp_history(monkeypatch):
    async def request(method, path, *, json_body=None, params=None):
        if path == "/lol-gameflow/v1/session":
            return {
                "phase": "InProgress",
                "gameData": {
                    "gameId": 456,
                    "queue": {"id": 420},
                    "playerChampionSelections": [{"puuid": "sgp-player", "championId": 1, "team": 100}],
                },
            }
        if path == "/lol-summoner/v2/summoners/puuid/sgp-player":
            return {"puuid": "sgp-player", "gameName": "SGP Player"}
        if path == "/lol-ranked/v1/ranked-stats/sgp-player":
            return {}
        if path == "/lol-match-history/v1/products/lol/sgp-player/matches":
            raise RuntimeError("local history unavailable")
        raise AssertionError(f"unexpected LCU request: {method} {path}")

    async def sgp_history(puuid, beg_index, count, server_id=None):
        assert (puuid, beg_index, count) == ("sgp-player", 0, 20)
        return {
            "games": {
                "games": [{
                    "gameId": 44,
                    "queueId": 420,
                    "participantIdentities": [{"participantId": 1, "player": {"puuid": puuid}}],
                    "participants": [{
                        "participantId": 1,
                        "championId": 1,
                        "stats": {"win": True, "kills": 5, "deaths": 1, "assists": 5},
                    }],
                }],
            },
        }

    async def champion_names():
        return {1: "Annie"}

    monkeypatch.setattr(league_lab.league_lab_service, "settings", LeagueLabSettings(
        ongoing_show_jungle_pathing=False,
        ongoing_show_premade_tag=False,
    ))
    monkeypatch.setattr(league_lab.league_lab_service, "phase", "InProgress")
    monkeypatch.setattr(league_lab.league_lab_service, "credentials", None)
    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    monkeypatch.setattr(league_lab, "_sgp_match_history", sgp_history)
    monkeypatch.setattr(league_lab, "_champion_names", champion_names)
    monkeypatch.setattr(league_lab, "_read_player_tags", lambda: {})
    monkeypatch.setattr(league_lab, "_remember_recent_players", lambda *args: None)
    monkeypatch.setattr(league_lab, "_ongoing_cache", {"key": "", "expires_at": 0.0, "payload": None})

    result = asyncio.run(league_lab.league_ongoing_game())

    player = result["players"][0]
    assert player["recent"]["matches"] == 1
    assert player["recent"]["wins"] == 1
    assert player["data_availability"]["history_source"] == "sgp"
    assert player["data_availability"]["unavailable"] == []


def test_ongoing_game_returns_empty_model_when_idle_session_is_unavailable(monkeypatch):
    async def request(method, path, *, json_body=None, params=None):
        assert method == "GET"
        assert path == "/lol-gameflow/v1/session"
        raise RuntimeError("LCU request failed: HTTPStatusError")

    settings = LeagueLabSettings(
        ongoing_show_match_history_item_border=True,
        ongoing_order_player_by="win-rate",
    )
    monkeypatch.setattr(league_lab.league_lab_service, "settings", settings)
    monkeypatch.setattr(league_lab.league_lab_service, "phase", "None")
    monkeypatch.setattr(league_lab.league_lab_service, "request", request)

    result = asyncio.run(league_lab.league_ongoing_game())

    assert result == {
        "phase": "None",
        "query_stage": "idle",
        "queue": {},
        "game_id": None,
        "players": [],
        "available": False,
        "show_match_history_item_border": True,
        "order_player_by": "win-rate",
    }


def test_ongoing_game_keeps_active_phase_failures_visible(monkeypatch):
    async def request(method, path, *, json_body=None, params=None):
        raise RuntimeError("LCU request failed: HTTPStatusError")

    monkeypatch.setattr(league_lab.league_lab_service, "settings", LeagueLabSettings())
    monkeypatch.setattr(league_lab.league_lab_service, "phase", "ChampSelect")
    monkeypatch.setattr(league_lab.league_lab_service, "request", request)

    with pytest.raises(league_lab.HTTPException) as exc_info:
        asyncio.run(league_lab.league_ongoing_game())

    assert exc_info.value.status_code == 409


def test_game_settings_file_write_gate_blocks_before_install_dir_request(monkeypatch):
    monkeypatch.setattr(
        league_lab.league_lab_service,
        "settings",
        LeagueLabSettings(toolkit_account_actions_enabled=False),
    )
    calls = []

    async def request(*args, **kwargs):
        calls.append((args, kwargs))
        raise AssertionError("toolkit gate must run before install-dir request")

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    with pytest.raises(league_lab.HTTPException) as caught:
        asyncio.run(league_lab.league_game_settings_file_update(
            league_lab.GameSettingsFileModeUpdate(mode="readonly")
        ))
    assert caught.value.status_code == 403
    assert calls == []


def test_arbitrary_game_preview_normalizes_lcu_scoreboard_and_timeline(monkeypatch):
    game = {
        "gameId": 987654,
        "gameMode": "CLASSIC",
        "queueId": 420,
        "gameDuration": 1800,
        "participantIdentities": [
            {"participantId": 1, "player": {"puuid": "p1", "gameName": "Alpha", "tagLine": "CN1", "profileIcon": 12}},
            {"participantId": 2, "player": {"puuid": "p2", "gameName": "Bravo", "tagLine": "CN1"}},
        ],
        "participants": [
            {"participantId": 1, "teamId": 100, "championId": 86, "stats": {"win": True, "kills": 9, "deaths": 2, "assists": 5, "totalDamageDealtToChampions": 22000}},
            {"participantId": 2, "teamId": 200, "championId": 22, "stats": {"win": False, "kills": 2, "deaths": 9, "assists": 1}},
        ],
    }

    async def request(method, path, *, json_body=None, params=None):
        if path == "/lol-match-history/v1/games/987654":
            return game
        if path == "/lol-match-history/v1/game-timelines/987654":
            return {"frames": [{"events": [{"type": "CHAMPION_KILL"}]}, {"events": []}]}
        raise RuntimeError(path)

    async def champion_names():
        return {86: "盖伦", 22: "艾希"}

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    monkeypatch.setattr(league_lab, "_champion_names", champion_names)
    result = asyncio.run(league_lab.league_game_preview(987654, "auto", True))

    assert result["source"] == "lcu"
    assert result["timeline"] == {"loaded": True, "frame_count": 2, "event_count": 1}
    assert result["teams"][0]["players"][0]["summoner"]["gameName"] == "Alpha"
    assert result["teams"][0]["players"][0]["match_stats"]["kda"] == 7.0
    assert result["ongoing_preview"]["historical_preview"] is True
    assert result["ongoing_preview"]["available"] is True


def test_match_timeline_normalizes_participants_frames_and_build_events():
    game = {
        "gameId": 77,
        "mapId": 11,
        "participantIdentities": [{"participantId": 1, "player": {"puuid": "p1", "gameName": "Alpha", "tagLine": "CN1"}}],
        "participants": [{"participantId": 1, "teamId": 100, "championId": 86}],
    }
    timeline = {
        "frames": [{
            "timestamp": 60000,
            "participantFrames": {"1": {"totalGold": 900, "currentGold": 300, "level": 2, "xp": 500, "minionsKilled": 7, "jungleMinionsKilled": 0, "position": {"x": 100, "y": 200}, "championStats": {"health": 550, "healthMax": 700, "attackDamage": 82, "unknown": 999}, "unknown": "drop"}},
            "events": [
                {"type": "ITEM_PURCHASED", "timestamp": 61000, "participantId": 1, "itemId": 1001, "secret": "drop"},
                {"type": "SKILL_LEVEL_UP", "timestamp": 62000, "participantId": 1, "skillSlot": 1, "levelUpType": "NORMAL"},
            ],
        }],
    }

    result = league_lab._normalize_match_timeline(game, timeline, "lcu")

    assert result["participants"] == [{"participant_id": 1, "puuid": "p1", "game_name": "Alpha", "tag_line": "CN1", "champion_id": 86, "team_id": 100}]
    assert result["frame_count"] == 1
    assert result["map_id"] == 11
    assert result["event_count"] == 2
    assert result["frames"][0]["participant_frames"]["1"]["totalGold"] == 900
    assert result["frames"][0]["participant_frames"]["1"]["championStats"] == {"health": 550, "healthMax": 700, "attackDamage": 82}
    assert "unknown" not in result["frames"][0]["participant_frames"]["1"]
    assert result["events"][0] == {"type": "ITEM_PURCHASED", "timestamp": 61000, "participantId": 1, "itemId": 1001}


def test_arbitrary_game_preview_falls_back_to_sgp(monkeypatch):
    async def request(method, path, *, json_body=None, params=None):
        raise RuntimeError("LCU unavailable")

    async def summary(game_id, server_id=None):
        return {"gameId": game_id, "participants": [{"participantId": 1, "puuid": "sgp-p1", "teamId": 100, "championId": 1, "riotIdGameName": "SGP", "kills": 1, "deaths": 0, "assists": 2, "win": True}]}

    async def details(game_id, server_id=None):
        return {"frames": []}

    async def champion_names():
        return {1: "黑暗之女"}

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    monkeypatch.setattr(league_lab, "_sgp_game_summary", summary)
    monkeypatch.setattr(league_lab, "_sgp_game_details", details)
    monkeypatch.setattr(league_lab, "_champion_names", champion_names)
    result = asyncio.run(league_lab.league_game_preview(123, "auto", True))

    assert result["source"] == "sgp"
    assert result["metadata"]["game_id"] == 123
    assert result["ongoing_preview"]["players"][0]["champion_name"] == "黑暗之女"
    assert result["warnings"] == ["LCU unavailable"]


def test_fixed_text_preset_uses_lcu_chat_in_champion_select(monkeypatch):
    calls = []
    settings = LeagueLabSettings(
        toolkit_account_actions_enabled=True,
        in_game_send_enabled=True,
        in_game_fixed_presets=[{"id": "hello", "title": "问候", "content": "第一行\n第二行"}],
    )

    async def request(method, path, *, json_body=None, params=None):
        calls.append((method, path, json_body))
        if path == "/lol-gameflow/v1/gameflow-phase":
            return "ChampSelect"
        if path == "/lol-chat/v1/conversations":
            return [{"id": "champ-chat", "type": "championSelect"}]
        return None

    monkeypatch.setattr(league_lab.league_lab_service, "settings", settings)
    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    result = asyncio.run(league_lab.league_send_in_game_preset(
        league_lab.InGamePresetSend(preset_id="hello", confirmation="我确认发送")
    ))

    assert result["transport"] == "lcu"
    assert calls[-1] == (
        "POST",
        "/lol-chat/v1/conversations/champ-chat/messages",
        {"body": "第一行\n第二行", "type": "chat"},
    )


def test_fixed_text_preset_shortcut_requires_configured_shortcut(monkeypatch):
    settings = LeagueLabSettings(
        toolkit_account_actions_enabled=True,
        in_game_send_enabled=True,
        in_game_fixed_presets=[{"id": "hello", "title": "问候", "content": "你好"}],
    )
    monkeypatch.setattr(league_lab.league_lab_service, "settings", settings)

    with pytest.raises(league_lab.HTTPException) as caught:
        asyncio.run(league_lab.league_send_in_game_preset(
            league_lab.InGamePresetSend(preset_id="hello", trigger="shortcut")
        ))
    assert caught.value.status_code == 409


def test_fixed_text_preset_uses_guarded_native_input_in_game(monkeypatch):
    settings = LeagueLabSettings(
        toolkit_account_actions_enabled=True,
        in_game_send_enabled=True,
        in_game_fixed_presets=[{"id": "hello", "title": "问候", "shortcut": "Ctrl+Alt+H", "content": "只发这一行"}],
    )

    async def request(method, path, *, json_body=None, params=None):
        if path == "/lol-gameflow/v1/gameflow-phase":
            return "InProgress"
        raise RuntimeError(path)

    sent = []
    monkeypatch.setattr(league_lab.league_lab_service, "settings", settings)
    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    monkeypatch.setattr(league_lab, "_send_text_to_foreground_league_game", lambda text: sent.append(text) or 4321)
    result = asyncio.run(league_lab.league_send_in_game_preset(
        league_lab.InGamePresetSend(preset_id="hello", trigger="shortcut")
    ))

    assert sent == ["只发这一行"]
    assert result["transport"] == "native"
    assert result["pid"] == 4321


def test_ad_hoc_in_game_lines_require_exact_confirmation(monkeypatch):
    settings = LeagueLabSettings(
        toolkit_account_actions_enabled=True,
        in_game_send_enabled=True,
    )
    monkeypatch.setattr(league_lab.league_lab_service, "settings", settings)

    with pytest.raises(league_lab.HTTPException) as caught:
        asyncio.run(league_lab.league_send_in_game_lines(
            league_lab.InGameAdHocSend(lines=["友方近况：状态稳定"], confirmation="确认")
        ))
    assert caught.value.status_code == 422

    sent = []

    async def send_lines(lines):
        sent.extend(lines)
        return {"sent": True, "transport": "lcu", "line_count": len(lines)}

    monkeypatch.setattr(league_lab, "_send_league_preset_lines", send_lines)
    result = asyncio.run(league_lab.league_send_in_game_lines(
        league_lab.InGameAdHocSend(lines=["友方近况：状态稳定"], confirmation="我确认发送")
    ))

    assert sent == ["友方近况：状态稳定"]
    assert result == {"sent": True, "transport": "lcu", "line_count": 1}


def test_cancel_in_game_send_sets_the_current_task_signal():
    league_lab._in_game_send_cancel.clear()
    result = asyncio.run(league_lab.league_cancel_in_game_send())
    assert result == {"cancel_requested": True}
    assert league_lab._in_game_send_cancel.is_set()
    league_lab._in_game_send_cancel.clear()


def test_generated_preset_shortcut_requires_matching_configured_target(monkeypatch):
    settings = LeagueLabSettings(toolkit_account_actions_enabled=True, in_game_send_enabled=True)
    monkeypatch.setattr(league_lab.league_lab_service, "settings", settings)
    body = league_lab.InGameAdHocSend(lines=["敌方近期表现"], trigger="shortcut", kind="rating", target="enemy")
    with pytest.raises(league_lab.HTTPException) as caught:
        asyncio.run(league_lab.league_send_in_game_lines(body))
    assert caught.value.status_code == 409

    settings.in_game_rating_shortcuts.enemy = "Ctrl+Alt+E"
    sent = []

    async def send_lines(lines):
        sent.extend(lines)
        return {"sent": True}

    monkeypatch.setattr(league_lab, "_send_league_preset_lines", send_lines)
    assert asyncio.run(league_lab.league_send_in_game_lines(body)) == {"sent": True}
    assert sent == ["敌方近期表现"]


@pytest.mark.parametrize("action", ["accept", "play-again", "reconnect", "start-matchmaking", "stop-matchmaking"])
def test_manual_flow_actions_require_toolkit_gate_before_any_lcu_request(monkeypatch, action):
    calls = []
    monkeypatch.setattr(
        league_lab.league_lab_service,
        "settings",
        LeagueLabSettings(toolkit_account_actions_enabled=False),
    )

    async def request(method, path, **_kwargs):
        calls.append((method, path))
        raise AssertionError("account gate must run before LCU requests")

    async def record(*_args):
        raise AssertionError("account gate must run before _record_action")

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    monkeypatch.setattr(league_lab.league_lab_service, "_record_action", record)

    with pytest.raises(league_lab.HTTPException) as caught:
        asyncio.run(league_lab.run_league_lab_action(action))

    assert caught.value.status_code == 403
    assert calls == []


@pytest.mark.parametrize(
    ("action", "live_phase"),
    [
        ("accept", "Lobby"),
        ("play-again", "Lobby"),
        ("reconnect", "Lobby"),
        ("start-matchmaking", "Matchmaking"),
        ("stop-matchmaking", "Lobby"),
    ],
)
def test_manual_flow_actions_reject_wrong_live_phase_without_recording(monkeypatch, action, live_phase):
    calls = []
    monkeypatch.setattr(
        league_lab.league_lab_service,
        "settings",
        LeagueLabSettings(toolkit_account_actions_enabled=True),
    )

    async def request(method, path, **_kwargs):
        calls.append((method, path))
        if (method, path) == ("GET", "/lol-gameflow/v1/gameflow-phase"):
            return live_phase
        raise AssertionError(f"unexpected LCU request: {method} {path}")

    async def record(*_args):
        raise AssertionError("wrong phase must block _record_action")

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    monkeypatch.setattr(league_lab.league_lab_service, "_record_action", record)

    with pytest.raises(league_lab.HTTPException) as caught:
        asyncio.run(league_lab.run_league_lab_action(action))

    assert caught.value.status_code == 409
    assert calls == [("GET", "/lol-gameflow/v1/gameflow-phase")]


@pytest.mark.parametrize("operation", ["bench", "reroll"])
def test_manual_champ_select_actions_require_gate_and_live_phase(monkeypatch, operation):
    calls = []
    monkeypatch.setattr(
        league_lab.league_lab_service,
        "settings",
        LeagueLabSettings(toolkit_account_actions_enabled=False),
    )

    async def request(method, path, **_kwargs):
        calls.append((method, path))
        raise AssertionError("account gate must run before LCU requests")

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    with pytest.raises(league_lab.HTTPException) as caught:
        if operation == "bench":
            asyncio.run(league_lab.league_bench_swap(7))
        else:
            asyncio.run(league_lab.league_champ_select_reroll())
    assert caught.value.status_code == 403
    assert calls == []


@pytest.mark.parametrize("operation", ["bench", "reroll"])
def test_manual_champ_select_actions_reject_wrong_phase_without_write(monkeypatch, operation):
    calls = []
    monkeypatch.setattr(
        league_lab.league_lab_service,
        "settings",
        LeagueLabSettings(toolkit_account_actions_enabled=True),
    )

    async def request(method, path, **_kwargs):
        calls.append((method, path))
        if (method, path) == ("GET", "/lol-gameflow/v1/gameflow-phase"):
            return "Lobby"
        raise AssertionError(f"unexpected LCU request: {method} {path}")

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    with pytest.raises(league_lab.HTTPException) as caught:
        if operation == "bench":
            asyncio.run(league_lab.league_bench_swap(7))
        else:
            asyncio.run(league_lab.league_champ_select_reroll())
    assert caught.value.status_code == 409
    assert calls == [("GET", "/lol-gameflow/v1/gameflow-phase")]


def test_mini_select_locks_first_subset_pick_with_fresh_evidence(monkeypatch):
    service = league_lab.league_lab_service
    monkeypatch.setattr(
        service,
        "settings",
        LeagueLabSettings(toolkit_account_actions_enabled=True),
    )
    calls = []
    session = {
        "localPlayerCellId": 2,
        "allowSubsetChampionPicks": True,
        "benchEnabled": True,
        "timer": {"phase": "BAN_PICK"},
        "myTeam": [{"cellId": 2, "championId": 0}],
        "actions": [
            [{"id": 3, "actorCellId": 2, "type": "pick", "completed": True}],
            [{"id": 8, "actorCellId": 2, "type": "pick", "completed": False}],
        ],
        "benchChampions": [{"championId": 34}],
    }

    async def request(method, path, *, json_body=None, **_kwargs):
        calls.append((method, path, json_body))
        if (method, path) == ("GET", "/lol-gameflow/v1/gameflow-phase"):
            return "ChampSelect"
        if (method, path) == ("GET", "/lol-champ-select/v1/session"):
            return session
        if (method, path) == ("GET", "/lol-champ-select/v1/pickable-champion-ids"):
            return [22, 34]
        if (method, path) == ("GET", "/lol-lobby-team-builder/champ-select/v1/subset-champion-list"):
            return [22, 34]
        if (method, path) == ("PATCH", "/lol-champ-select/v1/session/actions/8"):
            return None
        raise AssertionError(f"unexpected request: {method} {path}")

    async def refresh_state():
        return None

    monkeypatch.setattr(service, "request", request)
    monkeypatch.setattr(service, "_refresh_state", refresh_state)

    asyncio.run(league_lab.league_champ_select_select_or_bench(22))

    assert calls == [
        ("GET", "/lol-gameflow/v1/gameflow-phase", None),
        ("GET", "/lol-champ-select/v1/session", None),
        ("GET", "/lol-champ-select/v1/pickable-champion-ids", None),
        ("GET", "/lol-lobby-team-builder/champ-select/v1/subset-champion-list", None),
        ("PATCH", "/lol-champ-select/v1/session/actions/8", {"championId": 22, "completed": True, "type": "pick"}),
    ]


def test_mini_select_uses_bench_swap_after_a_champion_is_held(monkeypatch):
    service = league_lab.league_lab_service
    monkeypatch.setattr(
        service,
        "settings",
        LeagueLabSettings(toolkit_account_actions_enabled=True),
    )
    calls = []
    session = {
        "localPlayerCellId": 2,
        "benchEnabled": True,
        "timer": {"phase": "FINALIZATION"},
        "myTeam": [{"cellId": 2, "championId": 22}],
        "actions": [],
        "benchChampions": [{"championId": 34}],
    }

    async def request(method, path, *, json_body=None, **_kwargs):
        calls.append((method, path, json_body))
        if (method, path) == ("GET", "/lol-gameflow/v1/gameflow-phase"):
            return "ChampSelect"
        if (method, path) == ("GET", "/lol-champ-select/v1/session"):
            return session
        if (method, path) == ("POST", "/lol-champ-select/v1/session/bench/swap/34"):
            return None
        raise AssertionError(f"unexpected request: {method} {path}")

    async def refresh_state():
        return None

    monkeypatch.setattr(service, "request", request)
    monkeypatch.setattr(service, "_refresh_state", refresh_state)

    asyncio.run(league_lab.league_champ_select_select_or_bench(34))

    assert calls == [
        ("GET", "/lol-gameflow/v1/gameflow-phase", None),
        ("GET", "/lol-champ-select/v1/session", None),
        ("POST", "/lol-champ-select/v1/session/bench/swap/34", None),
    ]


def test_mini_select_respects_account_write_gate_before_lcu(monkeypatch):
    service = league_lab.league_lab_service
    monkeypatch.setattr(
        service,
        "settings",
        LeagueLabSettings(toolkit_account_actions_enabled=False),
    )
    calls = []

    async def request(method, path, **_kwargs):
        calls.append((method, path))
        raise AssertionError("account gate must run before a Mini selection request")

    monkeypatch.setattr(service, "request", request)
    with pytest.raises(league_lab.HTTPException) as caught:
        asyncio.run(league_lab.league_champ_select_select_or_bench(22))

    assert caught.value.status_code == 403
    assert calls == []


def test_champ_select_reroll_returns_refreshed_status_after_write(monkeypatch):
    calls = []
    monkeypatch.setattr(
        league_lab.league_lab_service,
        "settings",
        LeagueLabSettings(toolkit_account_actions_enabled=True),
    )

    async def request(method, path, **_kwargs):
        calls.append((method, path))
        if (method, path) == ("GET", "/lol-gameflow/v1/gameflow-phase"):
            return "ChampSelect"
        if (method, path) == ("GET", "/lol-champ-select/v1/session"):
            return {"allowRerolling": True, "rerollsRemaining": 1}
        if (method, path) == ("POST", "/lol-champ-select/v1/session/my-selection/reroll"):
            return None
        raise AssertionError(f"unexpected LCU request: {method} {path}")

    async def refresh_state():
        return None

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    monkeypatch.setattr(league_lab.league_lab_service, "_refresh_state", refresh_state)

    result = asyncio.run(league_lab.league_champ_select_reroll())

    assert isinstance(result, dict)
    assert calls == [
        ("GET", "/lol-gameflow/v1/gameflow-phase"),
        ("GET", "/lol-champ-select/v1/session"),
        ("POST", "/lol-champ-select/v1/session/my-selection/reroll"),
    ]


@pytest.mark.parametrize(
    "session",
    [
        {"allowRerolling": False, "rerollsRemaining": 1},
        {"allowRerolling": True, "rerollsRemaining": 0},
    ],
)
def test_champ_select_reroll_requires_explicit_support_and_remaining(monkeypatch, session):
    calls = []
    monkeypatch.setattr(
        league_lab.league_lab_service,
        "settings",
        LeagueLabSettings(toolkit_account_actions_enabled=True),
    )

    async def request(method, path, **_kwargs):
        calls.append((method, path))
        if (method, path) == ("GET", "/lol-gameflow/v1/gameflow-phase"):
            return "ChampSelect"
        if (method, path) == ("GET", "/lol-champ-select/v1/session"):
            return session
        raise AssertionError(f"reroll evidence must block the write: {method} {path}")

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    with pytest.raises(league_lab.HTTPException) as caught:
        asyncio.run(league_lab.league_champ_select_reroll())

    assert caught.value.status_code == 409
    assert calls == [
        ("GET", "/lol-gameflow/v1/gameflow-phase"),
        ("GET", "/lol-champ-select/v1/session"),
    ]


def test_manual_skin_change_requires_gate_and_live_phase(monkeypatch):
    calls = []
    league_lab.league_lab_service.champ_select = {
        "skin_selector": {"skins": [{"id": 22001}], "disabled": False}
    }
    monkeypatch.setattr(
        league_lab.league_lab_service,
        "settings",
        LeagueLabSettings(toolkit_account_actions_enabled=False),
    )

    async def request(method, path, **_kwargs):
        calls.append((method, path))
        raise AssertionError("account gate must run before LCU requests")

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    with pytest.raises(league_lab.HTTPException) as caught:
        asyncio.run(league_lab.league_champ_select_skin(22001))
    assert caught.value.status_code == 403
    assert calls == []


def test_manual_skin_change_rejects_wrong_phase_without_patch(monkeypatch):
    calls = []
    league_lab.league_lab_service.champ_select = {
        "skin_selector": {"skins": [{"id": 22001}], "disabled": False}
    }
    monkeypatch.setattr(
        league_lab.league_lab_service,
        "settings",
        LeagueLabSettings(toolkit_account_actions_enabled=True),
    )

    async def request(method, path, **_kwargs):
        calls.append((method, path))
        if (method, path) == ("GET", "/lol-gameflow/v1/gameflow-phase"):
            return "Lobby"
        raise AssertionError(f"unexpected LCU request: {method} {path}")

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    with pytest.raises(league_lab.HTTPException) as caught:
        asyncio.run(league_lab.league_champ_select_skin(22001))
    assert caught.value.status_code == 409
    assert calls == [("GET", "/lol-gameflow/v1/gameflow-phase")]
def test_friend_spectate_requires_gate_and_live_phase_before_launch(monkeypatch):
    friend_payload = [{
        "puuid": "friend-1",
        "gameName": "Friend",
        "gameTag": "CN1",
        "availability": "dnd",
        "lol": {"gameStatus": "inGame", "spectatorKey": "private"},
    }]
    calls = []
    monkeypatch.setattr(
        league_lab.league_lab_service,
        "settings",
        LeagueLabSettings(toolkit_account_actions_enabled=False),
    )

    async def request(method, path, **_kwargs):
        calls.append((method, path))
        raise AssertionError("account gate must run before LCU requests")

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    with pytest.raises(league_lab.HTTPException) as caught:
        asyncio.run(league_lab.spectate_league_friend("friend-1"))
    assert caught.value.status_code == 403
    assert calls == []

    calls.clear()
    monkeypatch.setattr(
        league_lab.league_lab_service,
        "settings",
        LeagueLabSettings(toolkit_account_actions_enabled=True),
    )

    async def wrong_phase_request(method, path, **_kwargs):
        calls.append((method, path))
        if (method, path) == ("GET", "/lol-chat/v1/friends"):
            return friend_payload
        if (method, path) == ("GET", "/lol-gameflow/v1/gameflow-phase"):
            return "InProgress"
        raise AssertionError(f"unexpected LCU request: {method} {path}")

    monkeypatch.setattr(league_lab.league_lab_service, "request", wrong_phase_request)
    with pytest.raises(league_lab.HTTPException) as caught:
        asyncio.run(league_lab.spectate_league_friend("friend-1"))
    assert caught.value.status_code == 409
    assert calls == [
        ("GET", "/lol-chat/v1/friends"),
        ("GET", "/lol-gameflow/v1/gameflow-phase"),
    ]


def test_terminate_game_client_requires_toolkit_gate_before_process_guard(monkeypatch):
    monkeypatch.setattr(
        league_lab.league_lab_service,
        "settings",
        LeagueLabSettings(toolkit_account_actions_enabled=False),
    )
    monkeypatch.setattr(league_lab, "_terminate_foreground_league_game_client", lambda: (_ for _ in ()).throw(AssertionError("must not terminate")))

    with pytest.raises(league_lab.HTTPException) as caught:
        asyncio.run(league_lab.league_terminate_game_client(
            league_lab.TerminateGameClientRequest(confirmation="我确认结束游戏")
        ))
    assert caught.value.status_code == 403


def test_decline_ready_check_requires_toolkit_gate_before_any_lcu_request(monkeypatch):
    calls = []
    monkeypatch.setattr(
        league_lab.league_lab_service,
        "settings",
        LeagueLabSettings(toolkit_account_actions_enabled=False),
    )

    async def request(method, path, **_kwargs):
        calls.append((method, path))
        raise AssertionError("account gate must run before LCU requests")

    async def record(*_args, **_kwargs):
        raise AssertionError("account gate must run before _record_action")

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    monkeypatch.setattr(league_lab.league_lab_service, "_record_action", record)
    with pytest.raises(league_lab.HTTPException) as caught:
        asyncio.run(league_lab.run_league_lab_action("decline-ready-check"))
    assert caught.value.status_code == 403
    assert calls == []


def test_cancel_auto_accept_only_changes_local_deadline(monkeypatch):
    calls = []
    monkeypatch.setattr(
        league_lab.league_lab_service,
        "settings",
        LeagueLabSettings(automation_enabled=True, auto_accept_enabled=True),
    )
    league_lab.league_lab_service.phase = "ReadyCheck"
    league_lab.league_lab_service._accept_due_at = time.monotonic() + 5

    async def request(method, path, **_kwargs):
        calls.append((method, path))
        raise AssertionError("cancel-auto-accept must not call LCU")

    async def record(*_args, **_kwargs):
        raise AssertionError("cancel-auto-accept must not record an account action")

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    monkeypatch.setattr(league_lab.league_lab_service, "_record_action", record)
    result = asyncio.run(league_lab.cancel_auto_accept())
    assert league_lab.league_lab_service._accept_due_at == float("inf")
    assert result["action_plan"]["accept_due"] is None
    assert calls == []


def test_cancel_auto_matchmaking_disables_only_the_local_plan(monkeypatch):
    calls = []
    service = league_lab.league_lab_service
    monkeypatch.setattr(service, "settings", LeagueLabSettings(
        automation_enabled=True,
        auto_matchmaking_enabled=True,
    ))
    service._matchmaking_due_at = time.monotonic() + 5

    async def request(method, path, **_kwargs):
        calls.append((method, path))
        raise AssertionError("cancel-auto-matchmaking must not call LCU")

    def write_settings(_settings):
        return None

    monkeypatch.setattr(service, "request", request)
    monkeypatch.setattr(service, "_write_settings", write_settings)
    result = asyncio.run(league_lab.cancel_auto_matchmaking())
    assert service._matchmaking_due_at is None
    assert service.settings.auto_matchmaking_enabled is False
    assert result["settings"]["automation_enabled"] is True
    assert result["settings"]["auto_matchmaking_enabled"] is False
    assert calls == []


def test_champ_select_trade_routes_require_gate_and_fresh_phase(monkeypatch):
    calls = []
    monkeypatch.setattr(
        league_lab.league_lab_service,
        "settings",
        LeagueLabSettings(toolkit_account_actions_enabled=False),
    )

    async def request(method, path, **_kwargs):
        calls.append((method, path))
        raise AssertionError("account gate must run before LCU requests")

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    with pytest.raises(league_lab.HTTPException) as caught:
        asyncio.run(league_lab.league_champ_select_trade_accept("17"))
    assert caught.value.status_code == 403
    assert calls == []

    calls.clear()
    monkeypatch.setattr(
        league_lab.league_lab_service,
        "settings",
        LeagueLabSettings(toolkit_account_actions_enabled=True),
    )

    async def wrong_phase_request(method, path, **_kwargs):
        calls.append((method, path))
        if path == "/lol-gameflow/v1/gameflow-phase":
            return "Lobby"
        raise AssertionError("wrong phase must prevent session/write requests")

    monkeypatch.setattr(league_lab.league_lab_service, "request", wrong_phase_request)
    with pytest.raises(league_lab.HTTPException) as caught:
        asyncio.run(league_lab.league_champ_select_trade_decline("17"))
    assert caught.value.status_code == 409
    assert calls == [("GET", "/lol-gameflow/v1/gameflow-phase")]


def test_champ_select_trade_rejects_stale_or_non_actionable_id_without_write(monkeypatch):
    calls = []
    monkeypatch.setattr(
        league_lab.league_lab_service,
        "settings",
        LeagueLabSettings(toolkit_account_actions_enabled=True),
    )

    async def request(method, path, **_kwargs):
        calls.append((method, path))
        if path == "/lol-gameflow/v1/gameflow-phase":
            return "ChampSelect"
        if path == "/lol-champ-select/v1/session":
            return {
                "localPlayerCellId": 1,
                "myTeam": [{"cellId": 1, "summonerId": 10}],
                "theirTeam": [{"cellId": 2, "summonerId": 20}],
                "trades": [{"id": 7, "cellId": 2, "state": "INVALID"}],
            }
        raise AssertionError(f"unexpected LCU request: {method} {path}")

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    with pytest.raises(league_lab.HTTPException) as caught:
        asyncio.run(league_lab.league_champ_select_trade_accept("8"))
    assert caught.value.status_code == 409
    assert calls == [
        ("GET", "/lol-gameflow/v1/gameflow-phase"),
        ("GET", "/lol-champ-select/v1/session"),
    ]

    calls.clear()
    with pytest.raises(league_lab.HTTPException) as caught:
        asyncio.run(league_lab.league_champ_select_trade_decline("7"))
    assert caught.value.status_code == 409
    assert calls == [
        ("GET", "/lol-gameflow/v1/gameflow-phase"),
        ("GET", "/lol-champ-select/v1/session"),
    ]


def test_champ_select_trade_accept_uses_fresh_actionable_session_and_mock_write(monkeypatch):
    calls = []
    monkeypatch.setattr(
        league_lab.league_lab_service,
        "settings",
        LeagueLabSettings(toolkit_account_actions_enabled=True),
    )

    async def request(method, path, **_kwargs):
        calls.append((method, path))
        if path == "/lol-gameflow/v1/gameflow-phase":
            return "ChampSelect"
        if path == "/lol-champ-select/v1/session":
            return {
                "localPlayerCellId": 1,
                "myTeam": [{"cellId": 1, "summonerId": 10, "gameName": "Local"}],
                "theirTeam": [{"cellId": 2, "summonerId": 20, "gameName": "Other"}],
                "trades": [{"id": 7, "cellId": 2, "state": "AVAILABLE"}],
            }
        if path == "/lol-champ-select/v1/session/champion-swaps/7/accept":
            return None
        raise AssertionError(f"unexpected LCU request: {method} {path}")

    monkeypatch.setattr(league_lab.league_lab_service, "request", request)
    result = asyncio.run(league_lab.league_champ_select_trade_accept("7"))
    assert result["last_action"] == "已接受换英雄请求"
    assert calls == [
        ("GET", "/lol-gameflow/v1/gameflow-phase"),
        ("GET", "/lol-champ-select/v1/session"),
        ("POST", "/lol-champ-select/v1/session/champion-swaps/7/accept"),
    ]


def test_status_exposes_normalized_lifecycle_and_action_plan(monkeypatch):
    service = LeagueLabService()
    service.phase = "ReadyCheck"
    service.ready_check = league_lab._normalize_ready_check({
        "state": "InProgress",
        "playerResponse": "None",
        "declinerIds": [9],
    })
    service.matchmaking_search = league_lab._normalize_matchmaking_search({
        "isCurrentlyInQueue": True,
        "searchState": "Searching",
        "timeInQueue": 4.5,
        "estimatedQueueTime": 8,
        "queueId": 420,
    })
    service._accept_due_at = time.monotonic() + 2
    service._phase_action_due_at = time.monotonic() + 3
    service._champion_action_due_at = {"pick-1": time.monotonic() + 4}
    status = service.status()
    assert status["ready_check"]["state"] == "InProgress"
    assert status["ready_check"]["can_decline"] is True
    assert status["matchmaking_search"]["is_currently_in_queue"] is True
    assert status["action_plan"]["accept_due"]["label"] == "自动接受对局"
    assert status["action_plan"]["accept_due"]["remaining_seconds"] > 0
    assert status["action_plan"]["champion_due"][0]["action_id"] == "pick-1"
    assert status["auto_select"]["schema_version"] == 1
    assert status["auto_select"]["delayed_pick"] is None
    assert "expected_picks" in status["auto_select"]


@pytest.mark.parametrize(
    ("response", "can_accept", "can_decline"),
    [("None", True, True), ("Accepted", False, True), ("Declined", True, False)],
)
def test_ready_check_response_can_be_reversed_while_pending(response, can_accept, can_decline):
    ready = league_lab._normalize_ready_check({
        "state": "InProgress",
        "playerResponse": response,
    })
    assert ready["response_state"] == response.lower()
    assert ready["can_accept"] is can_accept
    assert ready["can_decline"] is can_decline
    assert ready["actionability"] == {"accept": can_accept, "decline": can_decline}


def test_disconnected_status_never_requests_auxiliary_windows():
    service = LeagueLabService()
    service.credentials = None
    service.phase = "ChampSelect"
    service.settings = LeagueLabSettings(
        mini_enabled=True,
        mini_auto_show=True,
    )

    status = service.status()

    assert status["connected"] is False
    assert status["mini_should_show"] is False
    assert "opgg_should_show" not in status


def test_connection_invalidation_clears_stale_gameflow_and_window_state():
    service = LeagueLabService()
    service.credentials = league_lab.LcuCredentials(
        pid=1,
        port=2999,
        token="secret",
        region="TENCENT",
        platform_id="HN1",
    )
    service.phase = "Lobby"
    service.game_mode = "CLASSIC"
    service.summoner_name = "Tester"
    service.current_summoner = {"puuid": "player"}
    service.ready_check = {"state": "InProgress"}
    service.matchmaking_search = {"search_state": "Searching"}
    service.champ_select = {"is_spectating": False}
    service._event_connected = True

    service._invalidate_connection()

    assert service.credentials is None
    assert service.phase == ""
    assert service.game_mode == ""
    assert service.summoner_name == ""
    assert service.current_summoner == {}
    assert service.ready_check is None
    assert service.matchmaking_search is None
    assert service.champ_select == {}
    assert service._event_connected is False


def test_normalized_champ_select_exposes_trade_players_and_actionability():
    normalized = LeagueLabService._normalize_champ_select({
        "localPlayerCellId": 1,
        "myTeam": [{"cellId": 1, "summonerId": 10, "gameName": "Local"}],
        "theirTeam": [{"cellId": 2, "summonerId": 20, "gameName": "Other"}],
        "trades": [{"id": 7, "requesterCellId": 2, "state": "AVAILABLE"}],
    })
    trade = normalized["trades"][0]
    assert trade["id"] == 7
    assert trade["state"] == "AVAILABLE"
    assert trade["actionable"] is True
    assert trade["initiated_by_local_player"] is False
    assert trade["other_player"]["game_name"] == "Local"


def test_in_game_preset_settings_defaults_match_renderer_drafts_and_remain_disabled():
    settings = LeagueLabSettings()

    rating = settings.in_game_rating_preset_options.model_dump(by_alias=True)
    assert rating["targetMode"] == "all"
    assert rating["selectedPuuids"] == []
    assert rating["nameDisplayStrategy"] == "preferChampionName"
    assert rating["showCurrentChampion"] is False
    assert rating["display"] == {
        "winRate": True,
        "kda": True,
        "avgSoloKills": True,
        "avgVisionScore": False,
        "avgChampionDamage": False,
        "avgDamageTaken": False,
        "avgGold": False,
        "avgCsPerMinute": False,
        "avgKillParticipation": False,
        "avgDamageGoldEfficiency": False,
        "mainChampions": True,
        "mainPositions": True,
    }

    jungle = settings.in_game_jungle_preset_options.model_dump(by_alias=True)
    assert jungle["targetMode"] == "all"
    assert jungle["showCurrentChampion"] is True
    assert jungle["display"] == {
        "activityPreference": True,
        "firstClearDistribution": True,
        "earlyGank": True,
        "dragonControl": True,
        "monsterControl": True,
        "mainChampions": True,
    }

    premade = settings.in_game_premade_preset_options.model_dump(by_alias=True)
    assert premade == {
        "targetMode": "all",
        "selectedPuuids": [],
        "nameDisplayStrategy": "preferChampionName",
    }
    assert settings.in_game_send_enabled is False
    assert settings.toolkit_account_actions_enabled is True


def test_in_game_preset_settings_accept_camel_case_drafts_and_round_trip_snake_case():
    settings = LeagueLabSettings.model_validate({
        "in_game_rating_preset": {
            "targetMode": "selected",
            "selectedPuuids": ["puuid-a", "puuid-b"],
            "nameDisplayStrategy": "championNameWithName",
            "showCurrentChampion": True,
            "display": {"winRate": False, "kda": True, "mainPositions": False},
        },
        "in_game_jungle_preset_options": {
            "target_mode": "friendly",
            "selected_puuids": ["puuid-a"],
            "name_display_strategy": "preferName",
            "show_current_champion": False,
            "display": {"dragon_control": False},
        },
        "in_game_premade_preset": {
            "targetMode": "enemy",
            "selectedPuuids": ["puuid-b"],
            "nameDisplayStrategy": "preferName",
        },
    })

    assert settings.in_game_rating_preset.target_mode == "selected"
    assert settings.in_game_rating_preset.display.win_rate is False
    assert settings.in_game_jungle_preset.target_mode == "friendly"
    assert settings.in_game_jungle_preset.display.dragon_control is False
    assert settings.in_game_premade_preset.target_mode == "enemy"

    dumped = settings.model_dump()
    assert dumped["in_game_rating_preset_options"]["target_mode"] == "selected"
    assert dumped["in_game_jungle_preset_options"]["display"]["dragon_control"] is False
    assert LeagueLabSettings.model_validate(dumped).in_game_premade_preset.target_mode == "enemy"


def test_in_game_preset_settings_reject_unknown_nested_fields():
    with pytest.raises(ValidationError):
        LeagueLabSettings.model_validate({
            "in_game_rating_preset_options": {"display": {"unknownMetric": True}},
        })
    with pytest.raises(ValidationError):
        LeagueLabSettings.model_validate({
            "in_game_premade_preset": {"display": {"not_supported": True}},
        })
