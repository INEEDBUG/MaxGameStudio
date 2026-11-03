"""Fake-parser tests for independent SkinCore rewrite verification."""

from pathlib import Path

import pytest

from app.cosmetic_studio import core_inspection
from app.cosmetic_studio.inspection import InspectionError
from app.cosmetic_studio.skin_core import CoreProfile

TARGET = "76561199059074958"
ACCOUNT = int(TARGET) - 76561197960265728
ITEM = (1 << 32) | 2


def row(tick=100, **changes):
    value = {"tick": tick, "steamid": TARGET, "health": 100, "team": 2,
             "active_weapon_name": "awp", "item_id_high": 1, "item_id_low": 2,
             "item_definition_index": 9, "original_owner_account_id": ACCOUNT,
             "paint_kit": 0, "pattern_seed": 0, "wear": .5}
    value.update(changes)
    return value


class FakeParser:
    def __init__(self, rows, events=None, header=None):
        self.rows, self.events = rows, events or {}
        self.header = header or {"map_name": "de_test", "demo_version": 1}

    def parse_ticks(self, fields, ticks=None):
        wanted = set(ticks or [])
        return [r for r in self.rows if r.get("tick") in wanted]

    def parse_event(self, name):
        return self.events.get(name, [])

    def parse_header(self):
        return self.header


def profile():
    return CoreProfile(steam_id64=TARGET, items=({
        "item_id64": str(ITEM), "definition_index": 9, "paint_kit": 344,
        "pattern_seed": 7, "wear": .01,
    },))


def cross_model_profile():
    return CoreProfile(steam_id64=TARGET, items=({
        "item_id64": str(ITEM), "definition_index": 507,
        "replacement_definition_index": 515, "paint_kit": 38,
        "pattern_seed": 0, "wear": .01,
    },))


def parser_map(monkeypatch, mapping):
    monkeypatch.setattr(core_inspection, "_parser", lambda path: mapping[Path(path)])


def test_resolve_owned_awp_composes_exact_item_id(monkeypatch, tmp_path):
    path = tmp_path / "source.dem"
    parser_map(monkeypatch, {path: FakeParser([row()])})
    assert core_inspection.resolve_owned_awp(path, TARGET, 100) == str(ITEM)


@pytest.mark.parametrize("changes", [
    {"health": 0},
    {"active_weapon_name": "ak47"},
    {"item_id_high": 0, "item_id_low": 0},
    {"original_owner_account_id": ACCOUNT + 1},
])
def test_resolve_owned_awp_rejects_unsafe_or_unowned_rows(monkeypatch, tmp_path, changes):
    path = tmp_path / "source.dem"
    parser_map(monkeypatch, {path: FakeParser([row(**changes)])})
    with pytest.raises(InspectionError):
        core_inspection.resolve_owned_awp(path, TARGET, 100)


def test_verify_rewrite_requires_exact_gameplay_and_requested_cosmetics(monkeypatch, tmp_path):
    source, output = tmp_path / "source.dem", tmp_path / "output.dem"
    events = {name: [{"tick": 100, "weapon": "awp"}] for name in ("player_death", "round_start", "round_end")}
    parser_map(monkeypatch, {
        source: FakeParser([row()], events=events),
        output: FakeParser([row(paint_kit=344, pattern_seed=7, wear=.01)], events=events),
    })
    assert core_inspection.verify_rewrite(source, output, profile(), [100]) is True


def test_verify_rewrite_rejects_empty_output_and_gameplay_drift(monkeypatch, tmp_path):
    source, output = tmp_path / "source.dem", tmp_path / "output.dem"
    parser_map(monkeypatch, {
        source: FakeParser([row()]),
        output: FakeParser([]),
    })
    with pytest.raises(InspectionError):
        core_inspection.verify_rewrite(source, output, profile(), [100])

    parser_map(monkeypatch, {
        source: FakeParser([row()]),
        output: FakeParser([row(team=3, paint_kit=344, pattern_seed=7, wear=.01)]),
    })
    assert core_inspection.verify_rewrite(source, output, profile(), [100]) is False


def test_verify_rewrite_rejects_header_or_event_changes_and_tick_overflow(monkeypatch, tmp_path):
    source, output = tmp_path / "source.dem", tmp_path / "output.dem"
    parser_map(monkeypatch, {
        source: FakeParser([row()], header={"map_name": "de_test", "demo_version": 1}),
        output: FakeParser([row(paint_kit=344, pattern_seed=7, wear=.01)], header={"map_name": "de_other", "demo_version": 1}),
    })
    assert core_inspection.verify_rewrite(source, output, profile(), [100]) is False
    parser_map(monkeypatch, {
        source: FakeParser([row()]),
        output: FakeParser([row(paint_kit=344, pattern_seed=7, wear=.01)]),
    })
    with pytest.raises(InspectionError):
        core_inspection.verify_rewrite(source, output, profile(), list(range(1, 2050)))


def test_verify_rewrite_allows_idempotent_target_and_different_acting_player(monkeypatch, tmp_path):
    source, output = tmp_path / "source.dem", tmp_path / "output.dem"
    acting = "76561198000000000"
    before = row(steamid=acting, paint_kit=344, pattern_seed=7, wear=.01)
    after = row(steamid=acting, paint_kit=344, pattern_seed=7, wear=.01)
    parser_map(monkeypatch, {source: FakeParser([before]), output: FakeParser([after])})
    assert core_inspection.verify_rewrite(source, output, profile(), [100]) is True


def test_unknown_header_fields_cannot_silently_change(monkeypatch, tmp_path):
    source, output = tmp_path / "source.dem", tmp_path / "output.dem"
    parser_map(monkeypatch, {
        source: FakeParser([row()], header={"map_name": "de_test", "future_field": 1}),
        output: FakeParser([row(paint_kit=344, pattern_seed=7, wear=.01)], header={"map_name": "de_test", "future_field": 2}),
    })
    assert core_inspection.verify_rewrite(source, output, profile(), [100]) is False


def test_verify_rewrite_rejects_item_drift_non_target_cosmetic_drift_and_missing_tick(monkeypatch, tmp_path):
    source, output = tmp_path / "source.dem", tmp_path / "output.dem"
    other = row(steamid="76561198000000000", item_id_high=2, item_id_low=3, active_weapon_name="ak47", paint_kit=12)
    parser_map(monkeypatch, {
        source: FakeParser([row(), other]),
        output: FakeParser([row(paint_kit=344, pattern_seed=7, wear=.01),
                            row(steamid="76561198000000000", item_id_high=2, item_id_low=4, active_weapon_name="ak47", paint_kit=12)]),
    })
    assert core_inspection.verify_rewrite(source, output, profile(), [100]) is False

    parser_map(monkeypatch, {
        source: FakeParser([row(), other]),
        output: FakeParser([row(paint_kit=344, pattern_seed=7, wear=.01),
                            row(steamid="76561198000000000", item_id_high=2, item_id_low=3, active_weapon_name="ak47", paint_kit=13)]),
    })
    assert core_inspection.verify_rewrite(source, output, profile(), [100]) is False

    parser_map(monkeypatch, {
        source: FakeParser([row(100), row(101)]),
        output: FakeParser([row(100, paint_kit=344, pattern_seed=7, wear=.01)]),
    })
    assert core_inspection.verify_rewrite(source, output, profile(), [100, 101]) is False


def test_cross_model_knife_verifies_source_to_target_only_for_owned_item_each_tick(monkeypatch, tmp_path):
    source, output = tmp_path / "source.dem", tmp_path / "output.dem"
    source_rows = [row(tick, active_weapon_name='Karambit', item_def_idx=507,
                       item_definition_index=507) for tick in (100, 101)]
    output_rows = [row(tick, active_weapon_name='Butterfly Knife', item_def_idx=515,
                       item_definition_index=515, paint_kit=38, pattern_seed=0, wear=.01)
                   for tick in (100, 101)]
    events = {name: [{"tick": 100, "weapon": "weapon_knife"}]
              for name in ("player_death", "round_start", "round_end")}
    parser_map(monkeypatch, {source: FakeParser(source_rows, events=events),
                             output: FakeParser(output_rows, events=events)})
    assert core_inspection.verify_rewrite(source, output, cross_model_profile(), [100, 101],
                                          require_target_holding=True) is True


@pytest.mark.parametrize('source_changes,output_changes', [
    ({'active_weapon_name': 'Bayonet'}, {'active_weapon_name': 'Butterfly Knife'}),
    ({'active_weapon_name': 'Karambit'}, {'active_weapon_name': 'Falchion Knife'}),
    ({'item_def_idx': 500}, {'item_def_idx': 515}),
    ({'item_id_low': 99}, {'item_id_low': 99}),
])
def test_cross_model_knife_rejects_unqualified_identity_or_name_drift(
        monkeypatch, tmp_path, source_changes, output_changes):
    source, output = tmp_path / "source.dem", tmp_path / "output.dem"
    before = row(**{'active_weapon_name': 'Karambit', 'item_def_idx': 507,
                    'item_definition_index': 507, **source_changes})
    after = row(**{'active_weapon_name': 'Butterfly Knife', 'item_def_idx': 515,
                   'item_definition_index': 515, 'paint_kit': 38, 'pattern_seed': 0,
                   'wear': .01, **output_changes})
    parser_map(monkeypatch, {source: FakeParser([before]), output: FakeParser([after])})
    assert core_inspection.verify_rewrite(source, output, cross_model_profile(), [100],
                                          require_target_holding=True) is False


def test_cross_model_exception_does_not_allow_another_players_active_weapon_to_change(
        monkeypatch, tmp_path):
    source, output = tmp_path / "source.dem", tmp_path / "output.dem"
    other = row(steamid='76561198000000000', item_id_high=2, item_id_low=3,
                active_weapon_name='ak47', item_def_idx=7, item_definition_index=7,
                paint_kit=12)
    changed_other = {**other, 'active_weapon_name': 'm4a1'}
    before = row(active_weapon_name='Karambit', item_def_idx=507, item_definition_index=507)
    after = row(active_weapon_name='Butterfly Knife', item_def_idx=515,
                item_definition_index=515, paint_kit=38, pattern_seed=0, wear=.01)
    parser_map(monkeypatch, {source: FakeParser([before, other]),
                             output: FakeParser([after, changed_other])})
    assert core_inspection.verify_rewrite(source, output, cross_model_profile(), [100],
                                          require_target_holding=True) is False
