"""Fail-closed tests for the qualified zero-ID USP materialization path."""

from pathlib import Path

import pytest

from app.cosmetic_studio import materialized_inspection
from app.cosmetic_studio.inspection import InspectionError
from app.cosmetic_studio.skin_core import CoreMaterializedUspItem, CoreProfile

TARGET = "76561199849752808"
ACCOUNT = int(TARGET) - 76561197960265728
GENERATED_ID = 14204634964567075421
TICKS = [100, 101]


def profile():
    return CoreProfile(steam_id64=TARGET, items=(CoreMaterializedUspItem(
        pattern_seed=17, wear=0.12,
    ),))


def source_row(tick, steamid=TARGET, **changes):
    value = {
        "tick": tick,
        "steamid": steamid,
        "health": 100,
        "team_num": 3,
        "active_weapon_name": "usp_silencer",
        "item_id_high": 0,
        "item_id_low": 0,
        "item_def_idx": 61,
        "m_iAccountID": 0,
        "m_nFallbackPaintKit": 1142,
        "m_nFallbackSeed": 0,
        "m_flFallbackWear": 0.058,
        "position_x": 10.0,
    }
    value.update(changes)
    return value


def output_row(tick, steamid=TARGET, **changes):
    value = source_row(
        tick,
        steamid,
        item_id_high=3,
        item_id_low=GENERATED_ID & 0xFFFFFFFF,
        m_iAccountID=ACCOUNT,
        m_nFallbackPaintKit=504,
        m_nFallbackSeed=17,
        m_flFallbackWear=0.12,
    )
    value["item_id_high"] = GENERATED_ID >> 32
    value.update(changes)
    return value


class FakeParser:
    def __init__(self, rows, *, header=None, events=None):
        self.rows = rows
        self.header = header or {"map_name": "de_dust2", "demo_version": 4}
        self.events = events or {}

    def parse_ticks(self, fields, ticks=None):
        wanted = set(ticks or [])
        return [row for row in self.rows if row.get("tick") in wanted]

    def parse_header(self):
        return self.header

    def parse_event(self, name):
        return self.events.get(name, [])


def event_map():
    return {
        name: [{"tick": 100, "event_marker": name}]
        for name in ("player_death", "round_start", "round_end")
    }


def parser_map(monkeypatch, mapping):
    monkeypatch.setattr(materialized_inspection.inspection, "_parser", lambda path: mapping[Path(path)])


def fixture_rows():
    source = [source_row(tick) for tick in TICKS]
    source.extend(source_row(tick, steamid="76561199000000001", health=100,
                             team_num=2, active_weapon_name="ak47", item_id_high=1,
                             item_id_low=2, item_def_idx=7, m_iAccountID=123)
                  for tick in TICKS)
    output = [output_row(tick) for tick in TICKS]
    output.extend(source_row(tick, steamid="76561199000000001", health=100,
                             team_num=2, active_weapon_name="ak47", item_id_high=1,
                             item_id_low=2, item_def_idx=7, m_iAccountID=123)
                  for tick in TICKS)
    return source, output


def test_resolve_materialized_usp_accepts_only_exact_live_ct_usp_zero_id(monkeypatch, tmp_path):
    path = tmp_path / "source.dem"
    parser_map(monkeypatch, {path: FakeParser([source_row(100)])})

    assert materialized_inspection.resolve_materialized_usp(path, TARGET, 100) is True


@pytest.mark.parametrize("changes", [
    {"health": 0},
    {"steamid": "76561199000000001"},
    {"team_num": 2},
    {"active_weapon_name": "usp"},
    {"item_def_idx": 9},
    {"item_id_high": 1},
    {"item_id_low": -1},
    {"m_iAccountID": 2},
])
def test_resolve_materialized_usp_rejects_wrong_or_malformed_slot(monkeypatch, tmp_path, changes):
    path = tmp_path / "source.dem"
    parser_map(monkeypatch, {path: FakeParser([source_row(100, **changes)])})

    with pytest.raises(InspectionError):
        materialized_inspection.resolve_materialized_usp(path, TARGET, 100)


def test_resolve_materialized_usp_rejects_duplicate_and_missing_target_rows(monkeypatch, tmp_path):
    path = tmp_path / "source.dem"
    parser_map(monkeypatch, {
        path: FakeParser([source_row(100), source_row(100)]),
    })
    with pytest.raises(InspectionError):
        materialized_inspection.resolve_materialized_usp(path, TARGET, 100)

    parser_map(monkeypatch, {path: FakeParser([])})
    with pytest.raises(InspectionError):
        materialized_inspection.resolve_materialized_usp(path, TARGET, 100)


def test_verify_materialized_usp_accepts_valid_rewrite_and_optional_holding(monkeypatch, tmp_path):
    source_path, output_path = tmp_path / "source.dem", tmp_path / "output.dem"
    source, output = fixture_rows()
    events = event_map()
    parser_map(monkeypatch, {
        source_path: FakeParser(source, events=events),
        output_path: FakeParser(output, events=events),
    })

    assert materialized_inspection.verify_materialized_usp_rewrite(
        source_path, output_path, profile(), TICKS,
        require_target_holding=True,
    ) is True


def test_verify_materialized_usp_accepts_actual_weapon_alias_fields(monkeypatch, tmp_path):
    source_path, output_path = tmp_path / "source.dem", tmp_path / "output.dem"
    source, output = fixture_rows()
    for rows, output_side in ((source, False), (output, True)):
        for row in rows:
            is_target = row["steamid"] == TARGET
            row.pop("m_nFallbackPaintKit", None)
            row.pop("m_nFallbackSeed", None)
            row.pop("m_flFallbackWear", None)
            row.pop("m_iAccountID", None)
            if is_target:
                row["weapon_skin_id"] = 504 if output_side else 1142
                row["weapon_paint_seed"] = 17 if output_side else 0
                row["weapon_float"] = 0.12 if output_side else 0.058
                row["Weapon.m_iAccountID"] = ACCOUNT if output_side else 0
            else:
                row["weapon_skin_id"] = 77
                row["weapon_paint_seed"] = 3
                row["weapon_float"] = 0.3
                row["Weapon.m_iAccountID"] = 123
    parser_map(monkeypatch, {
        source_path: FakeParser(source, events=event_map()),
        output_path: FakeParser(output, events=event_map()),
    })

    assert materialized_inspection.verify_materialized_usp_rewrite(
        source_path, output_path, profile(), TICKS,
        require_target_holding=True,
    ) is True


def test_verify_materialized_usp_leaves_preowned_target_usp_unchanged(monkeypatch, tmp_path):
    source_path, output_path = tmp_path / "source.dem", tmp_path / "output.dem"
    source, output = fixture_rows()
    source[0]["item_id_high"] = 1
    source[0]["item_id_low"] = 8
    output[0] = dict(source[0])
    parser_map(monkeypatch, {
        source_path: FakeParser(source, events=event_map()),
        output_path: FakeParser(output, events=event_map()),
    })

    assert materialized_inspection.verify_materialized_usp_rewrite(
        source_path, output_path, profile(), TICKS,
        require_target_holding=False,
    ) is True


@pytest.mark.parametrize("mutate", [
    "wrong_generated_owner", "wrong_paint", "wrong_seed", "wrong_wear",
    "wrong_output_definition", "wrong_output_team", "wrong_active_weapon",
    "source_not_zero_id", "source_wrong_owner", "unrelated_zero_id_cosmetic",
    "generated_id_collision", "gameplay_change", "missing_tick",
    "header_change", "event_change", "contradictory_source_owner",
    "contradictory_output_owner", "contradictory_source_id", "contradictory_output_id",
])
def test_verify_materialized_usp_rejects_unqualified_or_changed_output(
        monkeypatch, tmp_path, mutate):
    source_path, output_path = tmp_path / "source.dem", tmp_path / "output.dem"
    source, output = fixture_rows()
    events = event_map()
    if mutate == "wrong_generated_owner":
        output[0]["m_iAccountID"] += 1
    elif mutate == "wrong_paint":
        output[0]["m_nFallbackPaintKit"] = 61
    elif mutate == "wrong_seed":
        output[0]["m_nFallbackSeed"] = 18
    elif mutate == "wrong_wear":
        output[0]["m_flFallbackWear"] = 0.2
    elif mutate == "wrong_output_definition":
        output[0]["item_def_idx"] = 9
    elif mutate == "wrong_output_team":
        output[0]["team_num"] = 2
    elif mutate == "wrong_active_weapon":
        output[0]["active_weapon_name"] = "glock"
    elif mutate == "source_not_zero_id":
        source[0]["item_id_low"] = 8
    elif mutate == "source_wrong_owner":
        source[0]["m_iAccountID"] = ACCOUNT + 1
    elif mutate == "unrelated_zero_id_cosmetic":
        output[-1]["m_nFallbackPaintKit"] = 999
    elif mutate == "generated_id_collision":
        source[-1]["item_id_high"] = GENERATED_ID >> 32
        source[-1]["item_id_low"] = GENERATED_ID & 0xFFFFFFFF
    elif mutate == "gameplay_change":
        output[0]["position_x"] += 1
    elif mutate == "missing_tick":
        source = [row for row in source if row["tick"] != 101]
        output = [row for row in output if row["tick"] != 101]
    elif mutate == "header_change":
        out_header = {"map_name": "de_mirage", "demo_version": 4}
    elif mutate == "event_change":
        events = {**events, "round_start": [{"tick": 101, "event_marker": "changed"}]}
    elif mutate == "contradictory_source_owner":
        source[0]["Weapon.m_iAccountID"] = ACCOUNT + 1
    elif mutate == "contradictory_output_owner":
        output[0]["Weapon.m_iAccountID"] = ACCOUNT + 1
    elif mutate == "contradictory_source_id":
        source[0]["m_iItemIDLow"] = 1
    elif mutate == "contradictory_output_id":
        output[0]["m_iItemIDLow"] = output[0]["item_id_low"] + 1
    else:
        raise AssertionError(mutate)

    parser_map(monkeypatch, {
        source_path: FakeParser(source, events=event_map()),
        output_path: FakeParser(output, header=locals().get("out_header"), events=events),
    })

    assert materialized_inspection.verify_materialized_usp_rewrite(
        source_path, output_path, profile(), TICKS,
        require_target_holding=True,
    ) is False


def test_verify_materialized_usp_rejects_duplicate_player_samples(monkeypatch, tmp_path):
    source_path, output_path = tmp_path / "source.dem", tmp_path / "output.dem"
    source, output = fixture_rows()
    source.append(dict(source[0]))
    output.append(dict(output[0]))
    parser_map(monkeypatch, {
        source_path: FakeParser(source, events=event_map()),
        output_path: FakeParser(output, events=event_map()),
    })
    assert materialized_inspection.verify_materialized_usp_rewrite(
        source_path, output_path, profile(), TICKS,
    ) is False


def test_verify_materialized_usp_enforces_tick_limit_and_target_holding(monkeypatch, tmp_path):
    source_path, output_path = tmp_path / "source.dem", tmp_path / "output.dem"
    source, output = fixture_rows()
    output[0]["health"] = 0
    parser_map(monkeypatch, {
        source_path: FakeParser(source, events=event_map()),
        output_path: FakeParser(output, events=event_map()),
    })

    assert materialized_inspection.verify_materialized_usp_rewrite(
        source_path, output_path, profile(), TICKS,
        require_target_holding=True,
    ) is False
    assert materialized_inspection.verify_materialized_usp_rewrite(
        source_path, output_path, profile(), list(range(1, 2050)),
    ) is False


def test_verify_materialized_usp_rejects_profile_without_positive_steam_account(monkeypatch, tmp_path):
    source_path, output_path = tmp_path / "source.dem", tmp_path / "output.dem"
    source, output = fixture_rows()
    parser_map(monkeypatch, {
        source_path: FakeParser(source, events=event_map()),
        output_path: FakeParser(output, events=event_map()),
    })
    bad_profile = CoreProfile(steam_id64="1", items=(CoreMaterializedUspItem(),))

    assert materialized_inspection.verify_materialized_usp_rewrite(
        source_path, output_path, bad_profile, TICKS,
    ) is False
