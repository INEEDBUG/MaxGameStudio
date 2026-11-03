from pathlib import Path

import pytest

from app.cosmetic_studio import glove_inspection
from app.cosmetic_studio.inspection import InspectionError


XUID = "76561199369875913"
ACCOUNT = int(XUID) - 76561197960265728
OTHER = "76561198000000000"
ITEM_ID = "52517827499"


def player_row(tick, *, xuid=XUID, pawn_high=12, pawn_low=978219947,
               pawn_definition=5030, paint=1410, seed=604.5204467773438,
               wear=0.38100576400756836, health=100, team=2,
               active="AK-47", item_high=1, item_low=2,
               item_definition=7, owner=None, custom_name=""):
    row = {
        "tick": tick,
        "steamid": int(xuid),
        "health": health,
        "team_num": team,
        "active_weapon_name": active,
        "item_id_high": item_high,
        "item_id_low": item_low,
        "item_def_idx": item_definition,
        "CCSPlayerPawn.m_iItemDefinitionIndex": pawn_definition,
        "CCSPlayerPawn.m_iItemIDHigh": pawn_high,
        "CCSPlayerPawn.m_iItemIDLow": pawn_low,
        "CCSPlayerPawn.CEconItemAttribute.m_iAttributeDefinitionIndex": 8,
        "CCSPlayerPawn.CEconItemAttribute.m_flInitialValue": wear,
        "CCSPlayerPawn.m_szCustomName": custom_name,
        "glove_paint_id": paint,
        "glove_paint_seed": seed,
        "glove_paint_float": wear,
    }
    if owner is not None:
        row["Weapon.m_iAccountID"] = owner
    return row


def skin_row(*, item_id=ITEM_ID, xuid=XUID, definition=5030):
    return {
        "item_id": int(item_id),
        "steamid": int(xuid),
        "def_index": definition,
        "paint_index": 1410,
        "paint_seed": 604,
        "paint_wear": 1052971824,
    }


class FakeParser:
    def __init__(self, rows, *, skins=None, header=None, events=None, extra_rows=None):
        self.rows = rows
        self.skins = [skin_row()] if skins is None else skins
        self.header = {"map_name": "de_test", "demo_version": 1} if header is None else header
        self.events = events or {
            "player_death": [{"tick": 100, "weapon": "ak47"}],
            "round_start": [{"tick": 64}],
            "round_end": [{"tick": 120}],
        }
        self.extra_rows = extra_rows or []

    def parse_ticks(self, fields, ticks=None):
        wanted = set(ticks or [])
        rows = self.rows + self.extra_rows
        return [row for row in rows if row.get("tick") in wanted or row in self.extra_rows]

    def parse_skins(self):
        return self.skins

    def parse_header(self):
        return self.header

    def parse_event(self, name):
        return self.events.get(name, [])


def parser_map(monkeypatch, parsers):
    monkeypatch.setattr(glove_inspection, "_parser", lambda path: parsers[Path(path)])


def two_tick_rows(*, replacement=False):
    ticks = (100, 101)
    source, output = [], []
    for tick in ticks:
        source.extend([player_row(tick), player_row(tick, xuid=OTHER, pawn_high=2, pawn_low=3,
                                                        pawn_definition=5031, paint=44, custom_name="Other glove")])
        if replacement:
            output.extend([
                player_row(tick, pawn_definition=5034, paint=10061, seed=0,
                           wear=0.01, custom_name="CS2 INSIGHT AGENT"),
                player_row(tick, xuid=OTHER, pawn_high=2, pawn_low=3, pawn_definition=5031,
                           paint=44, custom_name="Other glove"),
            ])
        else:
            output.extend(source[-2:])
    return source, output


def test_resolve_owned_glove_uses_exact_pawn_item_and_parse_skins_owner_fallback(monkeypatch, tmp_path):
    parser = FakeParser([player_row(3306)])
    parser_map(monkeypatch, {tmp_path / "source.dem": parser})

    item_id, definition = glove_inspection.resolve_owned_glove(tmp_path / "source.dem", XUID, 3306)

    assert (item_id, definition) == (ITEM_ID, 5030)


def test_resolve_owned_glove_allows_one_zero_item_component(monkeypatch, tmp_path):
    item_id_value = 978219947
    parser = FakeParser([player_row(3306, pawn_high=0, pawn_low=item_id_value)],
                        skins=[skin_row(item_id=str(item_id_value))])
    parser_map(monkeypatch, {tmp_path / "source.dem": parser})

    assert glove_inspection.resolve_owned_glove(tmp_path / "source.dem", XUID, 3306) == (str(item_id_value), 5030)


@pytest.mark.parametrize("changes", [
    {"health": 0},
    {"pawn_high": 0, "pawn_low": 0},
    {"pawn_definition": 5034},
])
def test_resolve_rejects_dead_empty_or_replacement_glove(monkeypatch, tmp_path, changes):
    parser = FakeParser([player_row(3306, **changes)])
    parser_map(monkeypatch, {tmp_path / "source.dem": parser})
    with pytest.raises(InspectionError):
        glove_inspection.resolve_owned_glove(tmp_path / "source.dem", XUID, 3306)


def test_resolve_rejects_duplicate_pawn_rows_and_wrong_parse_skins_owner(monkeypatch, tmp_path):
    path = tmp_path / "source.dem"
    parser = FakeParser([player_row(3306), player_row(3306)])
    parser_map(monkeypatch, {path: parser})
    with pytest.raises(InspectionError):
        glove_inspection.resolve_owned_glove(path, XUID, 3306)

    parser = FakeParser([player_row(3306)], skins=[skin_row(xuid=OTHER)])
    parser_map(monkeypatch, {path: parser})
    with pytest.raises(InspectionError):
        glove_inspection.resolve_owned_glove(path, XUID, 3306)


def test_verify_glove_rewrite_accepts_exact_target_and_preserves_other_player(monkeypatch, tmp_path):
    source_rows, output_rows = two_tick_rows(replacement=True)
    source, output = tmp_path / "source.dem", tmp_path / "output.dem"
    parser_map(monkeypatch, {
        source: FakeParser(source_rows),
        output: FakeParser(output_rows),
    })

    assert glove_inspection.verify_glove_rewrite(
        source, output, XUID, ITEM_ID, 5030, 5034, 10061, 0, 0.01, [100, 101]
    ) is True


def test_verify_rejects_source_parse_skins_owner_mismatch(monkeypatch, tmp_path):
    source_rows, output_rows = two_tick_rows(replacement=True)
    source, output = tmp_path / "source.dem", tmp_path / "output.dem"
    parser_map(monkeypatch, {
        source: FakeParser(source_rows, skins=[skin_row(xuid=OTHER)]),
        output: FakeParser(output_rows),
    })

    assert glove_inspection.verify_glove_rewrite(
        source, output, XUID, ITEM_ID, 5030, 5034, 10061, 0, 0.01, [100, 101]
    ) is False


@pytest.mark.parametrize("field,xuid", [("team_num", XUID), ("glove_paint_seed", OTHER)])
def test_verify_rejects_missing_gameplay_or_other_glove_field(monkeypatch, tmp_path, field, xuid):
    source_rows, output_rows = two_tick_rows(replacement=True)
    for rows in (source_rows, output_rows):
        for row in rows:
            if row["steamid"] == int(xuid):
                row.pop(field, None)
    source, output = tmp_path / "source.dem", tmp_path / "output.dem"
    parser_map(monkeypatch, {source: FakeParser(source_rows), output: FakeParser(output_rows)})

    assert glove_inspection.verify_glove_rewrite(
        source, output, XUID, ITEM_ID, 5030, 5034, 10061, 0, 0.01, [100, 101]
    ) is False


@pytest.mark.parametrize("changes", [
    {"pawn_high": 13},
    {"pawn_definition": 5030},
    {"paint": 10062},
    {"seed": 1},
    {"wear": 0.02},
    {"custom_name": "Changed other glove"},
])
def test_verify_rejects_target_identity_or_other_players_glove_drift(monkeypatch, tmp_path, changes):
    source_rows, output_rows = two_tick_rows(replacement=True)
    target_change = "custom_name" not in changes
    row = next(r for r in output_rows if r["steamid"] == int(XUID)) if target_change else next(
        r for r in output_rows if r["steamid"] == int(OTHER)
    )
    field_map = {
        "pawn_high": "CCSPlayerPawn.m_iItemIDHigh",
        "pawn_definition": "CCSPlayerPawn.m_iItemDefinitionIndex",
        "paint": "glove_paint_id",
        "seed": "glove_paint_seed",
        "wear": "glove_paint_float",
        "custom_name": "CCSPlayerPawn.m_szCustomName",
    }
    row.update({field_map.get(key, key): value for key, value in changes.items()})
    source, output = tmp_path / "source.dem", tmp_path / "output.dem"
    parser_map(monkeypatch, {source: FakeParser(source_rows), output: FakeParser(output_rows)})

    assert glove_inspection.verify_glove_rewrite(
        source, output, XUID, ITEM_ID, 5030, 5034, 10061, 0, 0.01, [100, 101]
    ) is False


@pytest.mark.parametrize("ticks", [[100, 100], [100, 1025], [], list(range(1, 2050))])
def test_verify_rejects_duplicate_unknown_missing_or_overflow_ticks(monkeypatch, tmp_path, ticks):
    source_rows, output_rows = two_tick_rows(replacement=True)
    source, output = tmp_path / "source.dem", tmp_path / "output.dem"
    parser_map(monkeypatch, {source: FakeParser(source_rows), output: FakeParser(output_rows)})

    assert glove_inspection.verify_glove_rewrite(
        source, output, XUID, ITEM_ID, 5030, 5034, 10061, 0, 0.01, ticks
    ) is False


@pytest.mark.parametrize("mutation", ["missing", "duplicate", "extra", "gameplay", "header", "event"])
def test_verify_rejects_incomplete_duplicate_or_changed_facts(monkeypatch, tmp_path, mutation):
    source_rows, output_rows = two_tick_rows(replacement=True)
    source = FakeParser(source_rows)
    output = FakeParser(output_rows)
    if mutation == "missing":
        output.rows = [r for r in output.rows if not (r["tick"] == 101 and r["steamid"] == int(XUID))]
    elif mutation == "duplicate":
        output.rows.append(player_row(100, pawn_definition=5034, paint=10061, seed=0, wear=.01))
    elif mutation == "extra":
        output.extra_rows.append(player_row(999))
    elif mutation == "gameplay":
        next(r for r in output.rows if r["steamid"] == int(OTHER))["team_num"] = 3
    elif mutation == "header":
        output.header = {"map_name": "de_other", "demo_version": 1}
    else:
        output.events["round_start"] = [{"tick": 65}]
    paths = {tmp_path / "source.dem": source, tmp_path / "output.dem": output}
    parser_map(monkeypatch, paths)

    assert glove_inspection.verify_glove_rewrite(
        tmp_path / "source.dem", tmp_path / "output.dem", XUID, ITEM_ID,
        5030, 5034, 10061, 0, 0.01, [100, 101]
    ) is False


def test_verify_rejects_unqualified_rewrite_parameters(monkeypatch, tmp_path):
    source_rows, output_rows = two_tick_rows(replacement=True)
    source, output = tmp_path / "source.dem", tmp_path / "output.dem"
    parser_map(monkeypatch, {source: FakeParser(source_rows), output: FakeParser(output_rows)})
    for source_def, replacement_def, paint in ((5031, 5034, 10061), (5030, 5035, 10061), (5030, 5034, 1)):
        assert glove_inspection.verify_glove_rewrite(
            source, output, XUID, ITEM_ID, source_def, replacement_def, paint, 0, .01, [100, 101]
        ) is False


def test_verify_does_not_compare_unrequested_gun_cosmetics(monkeypatch, tmp_path):
    source_rows, output_rows = two_tick_rows(replacement=True)
    source_rows[0]["weapon_skin_id"] = 10
    output_rows[0]["weapon_skin_id"] = 999
    source, output = tmp_path / "source.dem", tmp_path / "output.dem"
    parser_map(monkeypatch, {source: FakeParser(source_rows), output: FakeParser(output_rows)})

    assert glove_inspection.verify_glove_rewrite(
        source, output, XUID, ITEM_ID, 5030, 5034, 10061, 0, 0.01, [100, 101]
    ) is True
