from pathlib import Path
import hashlib

import pytest

from app.cosmetic_studio import appearance_inventory


PLAYER = "76561199369875913"
OTHER = "76561198000000000"


class Parser:
    def __init__(self, players, skins, ticks=None):
        self.players = players
        self.skins = skins
        self.ticks = ticks or []

    def parse_player_info(self):
        return self.players

    def parse_skins(self):
        return self.skins

    def parse_event(self, name):
        return [{"tick": 100}] if name == "round_start" else []

    def parse_ticks(self, fields, ticks=None):
        wanted = set(ticks or [])
        return [row for row in self.ticks if row["tick"] in wanted]


def columns(rows):
    return {key: [row.get(key) for row in rows] for key in rows[0]} if rows else {}


def test_held_owned_usp_missing_from_parse_skins_can_be_edited(monkeypatch, tmp_path):
    demo = tmp_path / "owned-usp.dem"
    demo.write_bytes(b"original")
    high, low = divmod(50509010986, 1 << 32)
    rows = [{"tick": tick, "steamid": int(PLAYER), "health": 100, "team_num": 3,
             "active_weapon_name": "USP-S", "item_def_idx": 61, "weapon_skin_id": 705,
             "weapon_paint_seed": 994, "weapon_float": 0.024,
             "item_id_high": high, "item_id_low": low,
             "Weapon.m_iAccountID": int(PLAYER) - 76561197960265728}
            for tick in range(100, 228)]
    monkeypatch.setattr(appearance_inventory, "_parser", lambda _: Parser(
        [{"steamid": int(PLAYER), "name": "Player"}], {}, rows))
    snapshot = appearance_inventory.inspect_inventory(demo)
    item = snapshot.response.items[0]
    assert item.editable and item.reason is None
    assert item.item_id == "50509010986" and item.paint_index == 705
    assert item.name == "USP消音版 | 脑洞大开"
    assert item.image_url.startswith("https://community.")
    assert item.seed == 994 and item.wear == pytest.approx(0.024)
    assert "usp-kill-confirmed" in item.preset_ids
    assert len(item.preset_ids) == 45
    assert snapshot.verification_ticks[item.slot_id] == tuple(range(100, 228))
    assert demo.read_bytes() == b"original"


@pytest.mark.parametrize("change", ["foreign_owner", "short_window", "paint_conflict", "owner_conflict"])
def test_observed_weapon_requires_unambiguous_continuous_owned_evidence(monkeypatch, tmp_path, change):
    demo = tmp_path / "ambiguous.dem"
    demo.write_bytes(b"original")
    high, low = divmod(50509010986, 1 << 32)
    rows = [{"tick": tick, "steamid": int(PLAYER), "health": 100, "team_num": 3,
             "active_weapon_name": "USP-S", "item_def_idx": 61, "weapon_skin_id": 705,
             "item_id_high": high, "item_id_low": low,
             "Weapon.m_iAccountID": int(PLAYER) - 76561197960265728}
            for tick in range(100, 228)]
    if change == "foreign_owner":
        for row in rows:
            row["Weapon.m_iAccountID"] = int(OTHER) - 76561197960265728
    elif change == "short_window":
        rows.pop()
    elif change == "paint_conflict":
        rows[-1]["weapon_skin_id"] = 504
    else:
        for row in rows:
            row["original_owner_account_id"] = int(PLAYER) - 76561197960265728
            row["Weapon.m_iAccountID"] = int(OTHER) - 76561197960265728
    monkeypatch.setattr(appearance_inventory, "_parser", lambda _: Parser(
        [{"steamid": int(PLAYER), "name": "Player"}], {}, rows))
    snapshot = appearance_inventory.inspect_inventory(demo)
    item = snapshot.response.items[0]
    assert not item.editable and not item.preset_ids
    assert item.slot_id not in snapshot.verification_ticks


def test_unknown_positive_finish_is_not_misreported_as_default():
    label, image = appearance_inventory._catalog_name(61, 99999, "USP-S")
    assert label == "USP-S · 皮肤编号 99999"
    assert image is None


def test_inventory_uses_exact_parse_skins_owner_and_does_not_fabricate_or_guess_team(monkeypatch, tmp_path):
    demo = tmp_path / "match.dem"
    demo.write_bytes(b"immutable fixture")
    item_id = 48742431476
    high, low = divmod(item_id, 1 << 32)
    active_rows = [{
        "tick": tick, "steamid": int(PLAYER), "health": 100, "team_num": 3,
        "active_weapon_name": "karambit", "item_id_high": high, "item_id_low": low,
        "item_def_idx": 507, "original_owner_account_id": int(PLAYER) - 76561197960265728,
    } for tick in range(100, 228)]
    parser = Parser(
        players=[{"steamid": int(PLAYER), "name": "Player"}, {"steamid": int(OTHER), "name": "Other"}],
        skins=columns([
            {"steamid": int(PLAYER), "item_id": 48742431476, "def_index": 507,
             "paint_index": 38, "paint_seed": 13, "paint_wear": 0x3DCCCCCD, "custom_name": None},
            {"steamid": int(PLAYER), "item_id": 52517827499, "def_index": 5030,
             "paint_index": 1410, "paint_seed": 1, "paint_wear": 0x3E000000, "custom_name": None},
            {"steamid": int(PLAYER), "item_id": 800, "def_index": 9999,
             "paint_index": 1, "paint_seed": 0, "paint_wear": 0x3DCCCCCD, "custom_name": None},
            {"steamid": int(OTHER), "item_id": 900, "def_index": 9,
             "paint_index": 344, "paint_seed": 0, "paint_wear": 0x3DCCCCCD, "custom_name": None},
        ]), ticks=active_rows,
    )
    monkeypatch.setattr(appearance_inventory, "_parser", lambda path: parser)
    monkeypatch.setattr(appearance_inventory, "_glove_identity_matches",
        lambda path, xuid, seen_item, definition, tick: (seen_item, definition) == ("52517827499", 5030))

    result = appearance_inventory.inspect_inventory(demo)
    target = [row for row in result.response.items if row.xuid == PLAYER]
    knife = next(row for row in target if row.item_id == "48742431476")
    glove = next(row for row in target if row.item_id == "52517827499")
    unsupported = next(row for row in target if row.item_id == "800")

    assert result.response.demo_sha256 == hashlib.sha256(b"immutable fixture").hexdigest()
    assert {row.xuid for row in result.response.players} == {PLAYER, OTHER}
    assert len(target) == 3
    assert knife.team == "ct"
    assert knife.preset_ids == ["karambit-fade", "karambit-to-butterfly-fade"]
    assert knife.editable is True
    assert knife.wear == pytest.approx(0.1, abs=1e-6)
    assert result.verification_ticks[knife.slot_id] == tuple(range(100, 228))
    assert glove.editable is True
    assert glove.preset_ids == ["specialist-10061"]
    assert unsupported.editable is False
    assert unsupported.reason == "unsupported_source_definition"
    # Inventory is complete across the Demo roster; another player's owned
    # item remains visible but is never mixed into the selected player's list.
    assert any(row.xuid == OTHER and row.item_id == "900" for row in result.response.items)
    assert all(row.xuid == PLAYER for row in target)

    # The same parse_skins row is read-only if the Pawn verifier reports a
    # different exact glove identity, even though a Karambit window exists.
    monkeypatch.setattr(appearance_inventory, "_glove_identity_matches", lambda *args: False)
    unverified = appearance_inventory.inspect_inventory(demo)
    glove = next(row for row in unverified.response.items if row.xuid == PLAYER and row.item_id == "52517827499")
    assert glove.editable is False
    assert glove.reason == "owned_glove_identity_unverified"


def test_inventory_rejects_ambiguous_duplicate_owned_item_ids(monkeypatch, tmp_path):
    demo = tmp_path / "match.dem"
    demo.write_bytes(b"immutable fixture")
    row = {"steamid": int(PLAYER), "item_id": 42, "def_index": 9,
           "paint_index": 344, "paint_seed": 0, "paint_wear": 0x3DCCCCCD, "custom_name": None}
    parser = Parser([{"steamid": int(PLAYER), "name": "Player"}], columns([row, {**row, "def_index": 7}]))
    monkeypatch.setattr(appearance_inventory, "_parser", lambda path: parser)

    with pytest.raises(ValueError, match="ambiguous item identity"):
        appearance_inventory.inspect_inventory(demo)


def test_identical_repeated_skin_record_does_not_block_the_demo(monkeypatch, tmp_path):
    demo = tmp_path / "repeated-knife.dem"
    demo.write_bytes(b"unchanged demo")
    knife = {"steamid": int(PLAYER), "item_id": 52938219867, "def_index": 521,
             "paint_index": 42, "paint_seed": 686, "paint_wear": 1052286828,
             "custom_name": "斩龙"}
    other = {"steamid": int(OTHER), "item_id": 900, "def_index": 9,
             "paint_index": 344, "paint_seed": 0, "paint_wear": 0x3DCCCCCD,
             "custom_name": None}
    monkeypatch.setattr(appearance_inventory, "_parser", lambda _: Parser(
        [{"steamid": int(PLAYER), "name": "Player"}, {"steamid": int(OTHER), "name": "Other"}],
        columns([knife, dict(knife), other, dict(knife)])))
    snapshot = appearance_inventory.inspect_inventory(demo)
    assert len(snapshot.response.items) == 2
    item = next(item for item in snapshot.response.items if item.xuid == PLAYER)
    assert item.item_id == "52938219867" and item.paint_index == 42
    assert item.name == "流浪者匕首（★） | 蓝钢"
    assert not item.editable and item.reason == "unsupported_source_definition"
    assert demo.read_bytes() == b"unchanged demo"


@pytest.mark.parametrize("field,value", [("paint_index", 38), ("paint_seed", 0),
                                        ("paint_wear", 0x3DCCCCCD)])
def test_repeated_item_with_conflicting_appearance_is_not_silently_merged(monkeypatch, tmp_path, field, value):
    demo = tmp_path / "conflicting-record.dem"
    demo.write_bytes(b"original")
    skin = {"steamid": int(PLAYER), "item_id": 52938219867, "def_index": 521,
            "paint_index": 42, "paint_seed": 686, "paint_wear": 1052286828,
            "custom_name": "斩龙"}
    monkeypatch.setattr(appearance_inventory, "_parser", lambda _: Parser(
        [{"steamid": int(PLAYER), "name": "Player"}], columns([skin, {**skin, field: value}])))
    with pytest.raises(ValueError, match="duplicate owned item identity"):
        appearance_inventory.inspect_inventory(demo)


def test_inventory_rejects_parser_float_xuids_instead_of_rounding(monkeypatch, tmp_path):
    demo = tmp_path / "match.dem"
    demo.write_bytes(b"immutable fixture")
    parser = Parser([{"steamid": float(PLAYER), "name": "Player"}], columns([]))
    monkeypatch.setattr(appearance_inventory, "_parser", lambda path: parser)

    with pytest.raises(ValueError, match="valid player roster"):
        appearance_inventory.inspect_inventory(demo)


def test_inventory_exposes_observed_equipment_when_player_has_no_inventory_rows(monkeypatch, tmp_path):
    demo = tmp_path / "missing-inventory.dem"
    demo.write_bytes(b"original unchanged")
    rows = [
        {"tick": 110, "steamid": int(PLAYER), "health": 100, "team_num": 3,
         "active_weapon_name": "USP-S", "item_def_idx": 61, "weapon_skin_id": 1142,
         "item_id_high": 0, "item_id_low": 0, "Weapon.m_iAccountID": 1},
        {"tick": 200, "steamid": int(PLAYER), "health": 100, "team_num": 2,
         "active_weapon_name": "AWP", "item_def_idx": 9, "weapon_skin_id": 137,
         "item_id_high": 0, "item_id_low": 0, "Weapon.m_iAccountID": 1},
        {"tick": 201, "steamid": int(OTHER), "health": 100, "team_num": 2,
         "active_weapon_name": "AK-47", "item_def_idx": 7, "weapon_skin_id": None,
         "item_id_high": 0, "item_id_low": 0, "Weapon.m_iAccountID": 1},
    ]
    monkeypatch.setattr(appearance_inventory, "_parser", lambda _: Parser(
        [{"steamid": int(PLAYER), "name": "Player"}, {"steamid": int(OTHER), "name": "Other"}], {}, rows))

    result = appearance_inventory.inspect_inventory(demo)
    target = [item for item in result.response.items if item.xuid == PLAYER]

    assert {(item.definition, item.paint_index, item.team) for item in target} == {(61, 1142, "ct"), (9, 137, "t")}
    assert all(item.item_id == "0" and not item.editable and item.reason == "missing_item_identity" for item in target)
    assert all(item.slot_id not in result.verification_ticks for item in target)
    assert all(item.definition != 7 for item in target)
    assert demo.read_bytes() == b"original unchanged"


def test_observed_equipment_deduplicates_samples_and_merges_confirmed_teams(monkeypatch, tmp_path):
    demo = tmp_path / "observed.dem"
    demo.write_bytes(b"original")
    rows = [{"tick": tick, "steamid": int(PLAYER), "health": 100, "team_num": team,
             "active_weapon_name": "AWP", "item_def_idx": 9, "weapon_skin_id": 137,
             "item_id_high": 0, "item_id_low": 0} for tick, team in [(101, 3), (102, 3), (150, 2)]]
    monkeypatch.setattr(appearance_inventory, "_parser", lambda _: Parser(
        [{"steamid": int(PLAYER), "name": "Player"}], {}, rows))

    result = appearance_inventory.inspect_inventory(demo)
    assert len(result.response.items) == 1
    assert result.response.items[0].team == "both"


def test_observed_equipment_never_converts_pickup_into_original_ownership(monkeypatch, tmp_path):
    demo = tmp_path / "pickup.dem"
    demo.write_bytes(b"original")
    real_id = 48742431476
    high, low = divmod(real_id, 1 << 32)
    rows = [{"tick": 120, "steamid": int(PLAYER), "health": 100, "team_num": 3,
             "active_weapon_name": "karambit", "item_def_idx": 507, "weapon_skin_id": 38,
             "item_id_high": high, "item_id_low": low,
             "Weapon.m_iAccountID": int(OTHER) - 76561197960265728}]
    skins = columns([{"steamid": int(OTHER), "item_id": real_id, "def_index": 507,
                      "paint_index": 38, "paint_seed": 0, "paint_wear": 0x3C23D70A}])
    monkeypatch.setattr(appearance_inventory, "_parser", lambda _: Parser(
        [{"steamid": int(PLAYER), "name": "Player"}, {"steamid": int(OTHER), "name": "Other"}], skins, rows))

    result = appearance_inventory.inspect_inventory(demo)
    observed = next(item for item in result.response.items if item.xuid == PLAYER)
    assert observed.item_id == str(real_id)
    assert not observed.editable and observed.reason == "unverified_original_ownership"
    assert observed.slot_id not in result.verification_ticks


def test_observed_equipment_does_not_duplicate_owned_inventory(monkeypatch, tmp_path):
    demo = tmp_path / "owned.dem"
    demo.write_bytes(b"original")
    high, low = divmod(48742431476, 1 << 32)
    rows = [{"tick": tick, "steamid": int(PLAYER), "health": 100, "team_num": 3,
             "active_weapon_name": "karambit", "item_def_idx": 507, "weapon_skin_id": 38,
             "item_id_high": high, "item_id_low": low,
             "Weapon.m_iAccountID": int(PLAYER) - 76561197960265728} for tick in range(100, 228)]
    skins = columns([{"steamid": int(PLAYER), "item_id": 48742431476, "def_index": 507,
                      "paint_index": 38, "paint_seed": 0, "paint_wear": 0x3C23D70A}])
    monkeypatch.setattr(appearance_inventory, "_parser", lambda _: Parser(
        [{"steamid": int(PLAYER), "name": "Player"}], skins, rows))
    result = appearance_inventory.inspect_inventory(demo)
    assert len(result.response.items) == 1 and result.response.items[0].editable


def test_observed_equipment_ignores_dead_invalid_and_non_cosmetic_utility_rows(monkeypatch, tmp_path):
    demo = tmp_path / "bad-observations.dem"
    demo.write_bytes(b"original")
    base = {"tick": 101, "steamid": int(PLAYER), "health": 100, "team_num": 3,
            "active_weapon_name": "AWP", "item_def_idx": 9, "weapon_skin_id": 137,
            "item_id_high": 0, "item_id_low": 0}
    rows = [dict(base, tick=101, health=0), dict(base, tick=102, item_def_idx=9.0),
            dict(base, tick=103, active_weapon_name=""),
            dict(base, tick=104, item_def_idx=49, active_weapon_name="C4 Explosive"),
            dict(base, tick=105, item_def_idx=46, active_weapon_name="Molotov")]
    monkeypatch.setattr(appearance_inventory, "_parser", lambda _: Parser(
        [{"steamid": int(PLAYER), "name": "Player"}], {}, rows))
    assert appearance_inventory.inspect_inventory(demo).response.items == []


def test_observed_equipment_limits_untrusted_unique_variants(monkeypatch, tmp_path):
    demo = tmp_path / "too-many.dem"
    demo.write_bytes(b"original")
    rows = [{"tick": tick, "steamid": int(PLAYER), "health": 100, "team_num": 3,
             "active_weapon_name": "AWP", "item_def_idx": 9, "weapon_skin_id": tick,
             "item_id_high": 0, "item_id_low": 0} for tick in range(100, 229)]
    monkeypatch.setattr(appearance_inventory, "_parser", lambda _: Parser(
        [{"steamid": int(PLAYER), "name": "Player"}], {}, rows))
    with pytest.raises(ValueError, match="per-player inventory limit"):
        appearance_inventory.inspect_inventory(demo)


def test_observed_nonzero_identity_deduplicates_missing_or_conflicting_paint(monkeypatch, tmp_path):
    demo = tmp_path / "observed-conflict.dem"
    demo.write_bytes(b"original")
    high, low = divmod(48742431476, 1 << 32)
    rows = [{"tick": tick, "steamid": int(PLAYER), "health": 100, "team_num": 3,
             "active_weapon_name": "AWP", "item_def_idx": 9, "weapon_skin_id": paint,
             "item_id_high": high, "item_id_low": low}
            for tick, paint in [(100, None), (101, 137), (102, 344)]]
    monkeypatch.setattr(appearance_inventory, "_parser", lambda _: Parser(
        [{"steamid": int(PLAYER), "name": "Player"}], {}, rows))
    result = appearance_inventory.inspect_inventory(demo)
    assert len(result.response.items) == 1
    item = result.response.items[0]
    assert item.item_id == "48742431476"
    assert item.paint_index is None and item.image_url is None
    assert not item.editable and not item.preset_ids
    assert item.reason == "unverified_original_ownership"


def test_zero_id_usp_requires_an_exact_continuous_ct_holding_window(monkeypatch, tmp_path):
    demo = tmp_path / "materializable.dem"
    demo.write_bytes(b"original")
    rows = [{"tick": tick, "steamid": int(PLAYER), "health": 100, "team_num": 3,
             "active_weapon_name": "USP-S", "item_def_idx": 61, "weapon_skin_id": 1142,
             "item_id_high": 0, "item_id_low": 0, "Weapon.m_iAccountID": 1}
            for tick in range(100, 228)]
    monkeypatch.setattr(appearance_inventory, "_parser", lambda _: Parser(
        [{"steamid": int(PLAYER), "name": "Player"}], {}, rows))
    result = appearance_inventory.inspect_inventory(demo)
    assert len(result.response.items) == 1
    item = result.response.items[0]
    assert item.editable and item.item_id == "0" and item.reason is None
    assert item.preset_ids == ["usp-kill-confirmed"]
    assert result.verification_ticks[item.slot_id] == tuple(range(100, 228))


@pytest.mark.parametrize("change", [{"item_id_high": None}, {"item_id_low": 0.0},
    {"Weapon.m_iAccountID": 999}, {"team_num": 2}, {"active_weapon_name": "AWP"},
    {"tick": 200, "weapon_skin_id": 137}])
def test_zero_id_usp_does_not_enable_unknown_or_ambiguous_evidence(monkeypatch, tmp_path, change):
    demo = tmp_path / "unqualified.dem"
    demo.write_bytes(b"original")
    rows = [{"tick": tick, "steamid": int(PLAYER), "health": 100, "team_num": 3,
             "active_weapon_name": "USP-S", "item_def_idx": 61, "weapon_skin_id": 1142,
             "item_id_high": 0, "item_id_low": 0, "Weapon.m_iAccountID": 1}
            for tick in range(100, 228)]
    if "tick" in change:
        rows[100] = {**rows[100], **change}
    else:
        rows = [{**row, **change} for row in rows]
    monkeypatch.setattr(appearance_inventory, "_parser", lambda _: Parser(
        [{"steamid": int(PLAYER), "name": "Player"}], {}, rows))
    result = appearance_inventory.inspect_inventory(demo)
    assert all(not item.editable for item in result.response.items)
    assert result.verification_ticks == {}
