from pathlib import Path

import pytest

from app.cosmetic_studio import inspection


class FakeParser:
    player_info = {"steamid": [76561198000000001], "name": ["Target"]}
    deaths = {
        "tick": [100, 380, 500],
        "weapon": ["ak47", "awp", "awp"],
        "attacker_steamid": ["76561198000000001"] * 3,
    }
    ticks = {"user_id": [2], "steamid": [76561198000000001], "health": [100], "active_weapon_name": ["awp"]}

    def __init__(self, _path):
        pass

    def parse_player_info(self):
        return self.player_info

    def parse_event(self, name):
        assert name == "player_death"
        return self.deaths

    def parse_ticks(self, _props, *, ticks):
        values = self.ticks
        return {
            "tick": list(ticks),
            "user_id": [values["user_id"][0]] * len(ticks),
            "steamid": [values["steamid"][0]] * len(ticks),
            "health": [values["health"][0]] * len(ticks),
            "active_weapon_name": [values["active_weapon_name"][0]] * len(ticks),
        }


def _patch_parser(monkeypatch, parser=FakeParser):
    monkeypatch.setattr(inspection, "DemoParser", parser)


def test_inspection_builds_awp_windows_and_preserves_repeated_player_clips(monkeypatch, tmp_path):
    demo = tmp_path / "demo.dem"
    demo.write_bytes(b"fixture")
    _patch_parser(monkeypatch)

    result = inspection.inspect_demo(demo)

    assert [item.model_dump() for item in result.players] == [{"xuid": "76561198000000001", "name": "Target"}]
    assert [(item.start_tick, item.end_tick) for item in result.candidates] == [(256, 572), (372, 692)]
    assert result.demo_sha256


def test_malformed_columns_are_rejected(monkeypatch, tmp_path):
    demo = tmp_path / "demo.dem"
    demo.write_bytes(b"fixture")

    class Broken(FakeParser):
        player_info = {"steamid": [1, 2], "name": ["only-one"]}

    _patch_parser(monkeypatch, Broken)
    with pytest.raises(inspection.InspectionError):
        inspection.inspect_demo(demo)


def test_float_or_unknown_xuid_never_becomes_a_candidate(monkeypatch, tmp_path):
    demo = tmp_path / "demo.dem"
    demo.write_bytes(b"fixture")

    class BadIds(FakeParser):
        player_info = {"steamid": [76561198000000001.0], "name": ["Target"]}

    _patch_parser(monkeypatch, BadIds)
    with pytest.raises(inspection.InspectionError):
        inspection.inspect_demo(demo)


def test_resolve_slot_requires_identity_live_state_and_event(monkeypatch, tmp_path):
    demo = tmp_path / "demo.dem"
    demo.write_bytes(b"fixture")
    _patch_parser(monkeypatch)

    assert inspection.resolve_slot(demo, "76561198000000001", 380) == 3
    with pytest.raises(inspection.InspectionError):
        inspection.resolve_slot(demo, "76561198000000002", 380)


def test_resolve_slot_rejects_dead_or_non_awp_active_state(monkeypatch, tmp_path):
    demo = tmp_path / "demo.dem"
    demo.write_bytes(b"fixture")

    class Dead(FakeParser):
        ticks = {"user_id": [2], "steamid": [76561198000000001], "health": [0], "active_weapon_name": ["awp"]}

    _patch_parser(monkeypatch, Dead)
    with pytest.raises(inspection.InspectionError):
        inspection.resolve_slot(demo, "76561198000000001", 380)


def test_resolve_slot_rejects_missing_weapon_name(monkeypatch, tmp_path):
    demo = tmp_path / "demo.dem"
    demo.write_bytes(b"fixture")

    class MissingWeaponName(FakeParser):
        def parse_ticks(self, _props, *, ticks):
            return {"tick": list(ticks), "user_id": [2], "steamid": [76561198000000001], "health": [100]}

    _patch_parser(monkeypatch, MissingWeaponName)
    with pytest.raises(inspection.InspectionError):
        inspection.resolve_slot(demo, "76561198000000001", 380)



def test_resolve_slot_rejects_non_awp_active_state(monkeypatch, tmp_path):
    demo = tmp_path / "demo.dem"
    demo.write_bytes(b"fixture")

    class WrongWeapon(FakeParser):
        ticks = {"user_id": [2], "steamid": [76561198000000001], "health": [100], "active_weapon_name": ["ak47"]}

    _patch_parser(monkeypatch, WrongWeapon)
    with pytest.raises(inspection.InspectionError):
        inspection.resolve_slot(demo, "76561198000000001", 380)


def test_inspection_crops_to_contiguous_awp_run(monkeypatch, tmp_path):
    demo = tmp_path / "demo.dem"
    demo.write_bytes(b"fixture")

    class Run(FakeParser):
        deaths = {"tick": [38346], "weapon": ["awp"], "attacker_steamid": ["76561198000000001"]}

        def parse_ticks(self, _props, *, ticks):
            run = set(range(38055, 38360))
            selected = [tick for tick in ticks if tick in run]
            return {"tick": selected, "user_id": [2] * len(selected), "steamid": [76561198000000001] * len(selected), "health": [100] * len(selected), "active_weapon_name": ["awp"] * len(selected)}

    _patch_parser(monkeypatch, Run)
    result = inspection.inspect_demo(demo)
    assert [(item.start_tick, item.end_tick) for item in result.candidates] == [(38218, 38355)]


def test_inspection_rejects_switch_or_gap_inside_candidate_run(monkeypatch, tmp_path):
    demo = tmp_path / "demo.dem"
    demo.write_bytes(b"fixture")

    class BrokenRun(FakeParser):
        deaths = {"tick": [38346], "weapon": ["awp"], "attacker_steamid": ["76561198000000001"]}

        def parse_ticks(self, _props, *, ticks):
            selected = [tick for tick in ticks if 38055 <= tick <= 38150 or 38160 <= tick <= 38359]
            return {"tick": selected, "user_id": [2] * len(selected), "steamid": [76561198000000001] * len(selected), "health": [100] * len(selected), "active_weapon_name": ["awp"] * len(selected)}

    _patch_parser(monkeypatch, BrokenRun)
    assert inspection.inspect_demo(demo).candidates == []


def test_inspection_rejects_conflicting_duplicate_snapshot(monkeypatch, tmp_path):
    demo = tmp_path / "demo.dem"
    demo.write_bytes(b"fixture")

    class Conflicting(FakeParser):
        deaths = {"tick": [38346], "weapon": ["awp"], "attacker_steamid": ["76561198000000001"]}

        def parse_ticks(self, _props, *, ticks):
            selected = list(ticks)
            return {"tick": selected + [38346], "user_id": [2] * len(selected) + [2], "steamid": [76561198000000001] * len(selected) + [76561198000000001], "health": [100] * len(selected) + [0], "active_weapon_name": ["awp"] * len(selected) + ["awp"]}

    _patch_parser(monkeypatch, Conflicting)
    assert inspection.inspect_demo(demo).candidates == []


def test_inspection_enforces_sample_limit(monkeypatch, tmp_path):
    demo = tmp_path / "demo.dem"
    demo.write_bytes(b"fixture")

    class ManyEvents(FakeParser):
        deaths = {"tick": list(range(256, 256 + 50_000)), "weapon": ["awp"] * 50_000, "attacker_steamid": ["76561198000000001"] * 50_000}

    _patch_parser(monkeypatch, ManyEvents)
    result = inspection.inspect_demo(demo)
    assert result.candidates_limited is True
    assert result.candidates
