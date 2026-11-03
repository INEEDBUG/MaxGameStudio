import pytest

from app.cosmetic_studio import intervals
from app.cosmetic_studio.catalog import PRESET_DATA, WEAPON_NAMES
from app.cosmetic_studio.inspection import InspectionError


XUID = "76561198000000000"
ACCOUNT = int(XUID) - 76561197960265728
OTHER = "76561198000000001"


def player_row(tick, *, xuid=XUID, definition=7, name="AK-47", high=1, low=2,
               owner=ACCOUNT, health=100):
    return {
        "tick": tick,
        "steamid": int(xuid),
        "health": health,
        "active_weapon_name": name,
        "item_id_high": high,
        "item_id_low": low,
        "item_def_idx": definition,
        "m_iAccountID": owner,
        "team_num": 2,
    }


class FakeParser:
    def __init__(self, rows):
        self.rows = rows
        self.requested_ticks = None
        self.requested_fields = None
        self.include_unrequested = False

    def parse_ticks(self, fields, ticks=None):
        self.requested_fields = list(fields)
        self.requested_ticks = list(ticks or [])
        wanted = set(self.requested_ticks)
        return [row for row in self.rows if self.include_unrequested or row.get("tick") in wanted]


@pytest.fixture(autouse=True)
def supported_directory(monkeypatch):
    # Karambit is added to the production directory by the integrating agent;
    # keep this test self-contained while exercising that exact-definition path.
    monkeypatch.setitem(PRESET_DATA, "test-ak", (7, 180, "AK test"))
    monkeypatch.setitem(WEAPON_NAMES, 7, ("ak47", "weapon_ak47", "c_weaponak47", "ak-47"))


def valid_rows(start=256, end=384):
    rows = []
    for tick in range(start - 64, end + 1):
        # Multiple players at every tick ensure rows are keyed by actual parser
        # tick and XUID, not zipped positionally against requested ticks.
        rows.append(player_row(tick, xuid=OTHER, owner=ACCOUNT + 1, low=22))
        rows.append(player_row(tick))
    return rows


def install_parser(monkeypatch, rows):
    parser = FakeParser(rows)
    monkeypatch.setattr(intervals, "_parser", lambda path: parser)
    return parser


def test_valid_manual_segment_uses_actual_ticks_and_exact_player(monkeypatch, tmp_path):
    parser = install_parser(monkeypatch, valid_rows())

    candidate = intervals.validate_interval(tmp_path / "clip.dem", XUID, 256, 384, 7)

    assert (candidate.xuid, candidate.start_tick, candidate.end_tick, candidate.weapon_id) == (XUID, 256, 384, 7)
    assert "manual" in candidate.label.lower() or "片段" in candidate.label
    assert parser.requested_ticks[0] == 192
    assert parser.requested_ticks[-1] == 384
    assert len(parser.requested_ticks) == 193
    assert {"tick", "steamid", "health", "active_weapon_name", "item_id_high", "item_id_low", "item_def_idx"}.issubset(parser.requested_fields)


@pytest.mark.parametrize("bad_xuid", [76561198000000000, 76561198000000000.0, "76561198000000000.0", "0", ""])
def test_rejects_non_exact_xuid_types_and_values(monkeypatch, tmp_path, bad_xuid):
    install_parser(monkeypatch, valid_rows())
    with pytest.raises(InspectionError):
        intervals.validate_interval(tmp_path / "clip.dem", bad_xuid, 256, 384, 7)


@pytest.mark.parametrize("start,end", [(255, 383), (256, 383), (256, 2177), (256.0, 384), (True, 384)])
def test_rejects_out_of_range_or_non_integer_interval(monkeypatch, tmp_path, start, end):
    install_parser(monkeypatch, valid_rows())
    with pytest.raises(InspectionError):
        intervals.validate_interval(tmp_path / "clip.dem", XUID, start, end, 7)


def test_rejects_unsupported_weapon_definition(monkeypatch, tmp_path):
    install_parser(monkeypatch, valid_rows())
    with pytest.raises(InspectionError):
        intervals.validate_interval(tmp_path / "clip.dem", XUID, 256, 384, 9999)


def test_rejects_missing_or_extra_target_tick(monkeypatch, tmp_path):
    rows = valid_rows()
    rows = [r for r in rows if not (r["tick"] == 256 and r["steamid"] == int(XUID))]
    install_parser(monkeypatch, rows)
    with pytest.raises(InspectionError, match="missing|unique|sample"):
        intervals.validate_interval(tmp_path / "clip.dem", XUID, 256, 384, 7)

    rows = valid_rows()
    rows.append(player_row(9999))
    parser = install_parser(monkeypatch, rows)
    parser.include_unrequested = True
    with pytest.raises(InspectionError, match="extra|outside|tick"):
        intervals.validate_interval(tmp_path / "clip.dem", XUID, 256, 384, 7)


def test_rejects_duplicate_target_row_at_same_actual_tick(monkeypatch, tmp_path):
    rows = valid_rows()
    rows.append(player_row(256))
    install_parser(monkeypatch, rows)
    with pytest.raises(InspectionError, match="duplicate|unique"):
        intervals.validate_interval(tmp_path / "clip.dem", XUID, 256, 384, 7)


@pytest.mark.parametrize("changes", [
    {"health": 0},
    {"active_weapon_name": "M4A1"},
    {"item_id_high": 0},
    {"item_id_low": 0},
    {"item_id_low": 3},
    {"m_iAccountID": ACCOUNT + 1},
])
def test_rejects_dead_switched_dropped_replaced_or_unowned_weapon(monkeypatch, tmp_path, changes):
    rows = valid_rows()
    for row in rows:
        if row["steamid"] == int(XUID) and row["tick"] == 300:
            row.update(changes)
            break
    install_parser(monkeypatch, rows)
    with pytest.raises(InspectionError):
        intervals.validate_interval(tmp_path / "clip.dem", XUID, 256, 384, 7)


def test_same_native_model_alias_does_not_accept_another_definition(monkeypatch, tmp_path):
    monkeypatch.setitem(PRESET_DATA, "test-m4", (16, 309, "M4 test"))
    monkeypatch.setitem(WEAPON_NAMES, 16, ("m4a1", "weapon_m4a1"))
    rows = valid_rows()
    for row in rows:
        if row["steamid"] == int(XUID) and row["tick"] == 300:
            row.update(item_def_idx=16, active_weapon_name="M4A1")
            break
    install_parser(monkeypatch, rows)
    with pytest.raises(InspectionError):
        intervals.validate_interval(tmp_path / "clip.dem", XUID, 256, 384, 7)


def test_rejects_float_identity_tick_or_missing_original_owner(monkeypatch, tmp_path):
    rows = valid_rows()
    target = next(r for r in rows if r["steamid"] == int(XUID))
    target["steamid"] = float(target["steamid"])
    install_parser(monkeypatch, rows)
    with pytest.raises(InspectionError):
        intervals.validate_interval(tmp_path / "clip.dem", XUID, 256, 384, 7)

    rows = valid_rows()
    target = next(r for r in rows if r["steamid"] == int(XUID))
    del target["m_iAccountID"]
    install_parser(monkeypatch, rows)
    with pytest.raises(InspectionError):
        intervals.validate_interval(tmp_path / "clip.dem", XUID, 256, 384, 7)
