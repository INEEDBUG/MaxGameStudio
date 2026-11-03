"""Independent parser verification for the qualified zero-ID CT USP rewrite."""

from __future__ import annotations

import math
from pathlib import Path
from typing import Any

from . import core_inspection as inspection
from .catalog import weapon_matches
from .inspection import InspectionError
from .skin_core import CoreMaterializedUspItem, CoreProfile

_BASE = 76561197960265728
_MAX_TICKS = 2048
_OWNER_FIELDS = ("original_owner_account_id", "original_owner_account", "m_iAccountID", "Weapon.m_iAccountID")
_HIGH_FIELDS = ("item_id_high", "m_iItemIDHigh")
_LOW_FIELDS = ("item_id_low", "m_iItemIDLow")
_ID_FIELDS = set(_HIGH_FIELDS + _LOW_FIELDS)
_COSMETIC_FIELDS = {
    "paint", "paint_kit", "item_paint_kit", "m_nFallbackPaintKit",
    "seed", "pattern_seed", "m_nFallbackSeed",
    "wear", "float", "m_flFallbackWear",
    "weapon_skin_id", "weapon_paint_seed", "weapon_float",
}
_PAINT_FIELDS = ("weapon_skin_id", "paint", "paint_kit", "item_paint_kit", "m_nFallbackPaintKit")
_SEED_FIELDS = ("weapon_paint_seed", "seed", "pattern_seed", "m_nFallbackSeed")
_WEAR_FIELDS = ("weapon_float", "wear", "float", "m_flFallbackWear")


def _owner(row: dict[str, Any]) -> int:
    values = [inspection._int(row[name], "owner account") for name in _OWNER_FIELDS if name in row]
    if not values or any(value != values[0] for value in values[1:]):
        raise InspectionError("owner aliases are missing or contradictory")
    return values[0]


def _item_components(row: dict[str, Any], *, required: bool = True) -> tuple[int, int] | None:
    present = any(name in row for name in _ID_FIELDS)
    if not present and not required:
        return None
    highs = [inspection._int(row[name], "item_id_high") for name in _HIGH_FIELDS if name in row]
    lows = [inspection._int(row[name], "item_id_low") for name in _LOW_FIELDS if name in row]
    if (not highs or not lows or any(value != highs[0] for value in highs[1:])
            or any(value != lows[0] for value in lows[1:])):
        raise InspectionError("item ID aliases are missing or contradictory")
    if not 0 <= highs[0] <= 0xFFFFFFFF or not 0 <= lows[0] <= 0xFFFFFFFF:
        raise InspectionError("invalid item id component")
    return highs[0], lows[0]


def _is_zero_id(row: dict[str, Any]) -> bool:
    high, low = _item_components(row)
    return high == 0 and low == 0


def _is_target_usp_slot(row: dict[str, Any], xuid: str) -> bool:
    return (
        inspection._steam(row) == xuid
        and inspection._int(inspection._field(row, "item_def_idx", "item_definition_index", "definition_index"), "definition") == 61
        and inspection._int(inspection._field(row, "team_num", "team"), "team") == 3
        and weapon_matches(inspection._field(row, "active_weapon_name"), 61)
    )


def _exact_ticks(ticks: list[int]) -> list[int]:
    if not isinstance(ticks, list) or not ticks or len(ticks) > _MAX_TICKS:
        raise InspectionError("tick sample is empty or exceeds limit")
    values = [inspection._int(tick, "tick") for tick in ticks]
    if any(tick < 1 for tick in values) or len(set(values)) != len(values):
        raise InspectionError("tick sample contains invalid or duplicate ticks")
    return values


def _keyed(rows: list[dict[str, Any]]) -> dict[tuple[str, int], dict[str, Any]]:
    result: dict[tuple[str, int], dict[str, Any]] = {}
    for row in rows:
        key = (inspection._steam(row), inspection._int(inspection._field(row, "tick"), "tick"))
        if key in result:
            raise InspectionError("duplicate player sample")
        result[key] = row
    return result


def resolve_materialized_usp(path: Path, xuid: str, tick: int) -> bool:
    """Confirm one live target CT USP row with an observed vanilla zero ID."""
    target = inspection._xuid(xuid)
    if not target:
        raise InspectionError("invalid target XUID")
    target_account = int(target) - _BASE
    if target_account <= 0:
        raise InspectionError("invalid target account")
    parser = inspection._parser(Path(path).expanduser().resolve())
    rows = inspection._ticks(parser, [inspection._int(tick, "tick")])
    target_rows = [row for row in rows if inspection._steam(row) == target]
    if len(target_rows) != 1:
        raise InspectionError("target row is missing or duplicated at tick")
    row = target_rows[0]
    if (inspection._int(inspection._field(row, "health"), "health") <= 0
            or not _is_target_usp_slot(row, target)):
        raise InspectionError("target is not alive with the exact CT USP-S")
    if not _is_zero_id(row):
        raise InspectionError("target USP item ID is not exactly zero")
    if _owner(row) not in (0, 1, target_account):
        raise InspectionError("source owner account is not an allowed vanilla value")
    return True


def _cosmetic_values_match(row: dict[str, Any], item: CoreMaterializedUspItem) -> bool:
    paint = inspection._int(inspection._field(row, *_PAINT_FIELDS), "paint kit")
    seed = inspection._int(inspection._field(row, *_SEED_FIELDS), "pattern seed")
    wear = inspection._number(row, *_WEAR_FIELDS)
    return (paint == item.paint_kit and seed == item.pattern_seed
            and math.isclose(wear, item.wear, rel_tol=0.0, abs_tol=1e-6))


def _same_noncosmetic_fields(before: dict[str, Any], after: dict[str, Any]) -> bool:
    identity_names = _ID_FIELDS | set(_OWNER_FIELDS)
    def without_allowed(row):
        return {key: value for key, value in row.items()
                if key not in identity_names and key not in _COSMETIC_FIELDS}
    return without_allowed(before) == without_allowed(after)


def _verify_materialized_usp_rewrite(
    source: Path,
    output: Path,
    profile: CoreProfile,
    ticks: list[int],
    *,
    require_target_holding: bool = False,
) -> bool:
    """Verify the dedicated zero-ID USP output without trusting core reply data."""
    if not isinstance(profile, CoreProfile) or len(profile.items) != 1:
        return False
    item = profile.items[0]
    if not isinstance(item, CoreMaterializedUspItem):
        return False
    requested = _exact_ticks(ticks)
    source_parser = inspection._parser(Path(source).expanduser().resolve())
    output_parser = inspection._parser(Path(output).expanduser().resolve())
    if inspection._header(source_parser) != inspection._header(output_parser):
        return False

    before = _keyed(inspection._ticks(source_parser, requested))
    after = _keyed(inspection._ticks(output_parser, requested))
    wanted_ticks = set(requested)
    if set(before) != set(after):
        return False
    if {tick for _, tick in before} != wanted_ticks:
        return False

    xuid = profile.steam_id64
    account = int(xuid) - _BASE
    if account <= 0:
        return False
    selected: list[tuple[tuple[str, int], dict[str, Any], dict[str, Any]]] = []
    selected_keys: set[tuple[str, int]] = set()
    for key, old in before.items():
        if key[0] != xuid:
            continue
        if _is_target_usp_slot(old, xuid):
            if not _is_zero_id(old):
                if old != after[key]:
                    return False
                continue
            if _owner(old) not in (0, 1, account):
                return False
            new = after[key]
            if (_int_health(new) <= 0 and require_target_holding) or not _is_target_usp_slot(new, xuid):
                return False
            if _is_zero_id(new):
                return False
            generated = inspection._item_id(new)
            high, low = _item_components(new)
            if (high, low) != (generated >> 32, generated & 0xFFFFFFFF):
                return False
            if _owner(new) != account or not _cosmetic_values_match(new, item):
                return False
            if not _same_noncosmetic_fields(old, new):
                return False
            selected.append((key, old, new))
            selected_keys.add(key)
        elif old != after[key]:
            # All non-selected rows belonging to the target stay byte-for-byte
            # parser-equivalent, including its other zero-ID cosmetics.
            return False

    if not selected:
        return False
    if require_target_holding and {key[1] for key, _, _ in selected} != wanted_ticks:
        return False
    generated_ids = {inspection._item_id(new) for _, _, new in selected}
    if len(generated_ids) != 1:
        return False
    generated_id = next(iter(generated_ids))
    if generated_id <= 0:
        return False
    for row in before.values():
        components = _item_components(row, required=False)
        if components is not None and ((components[0] << 32) | components[1]) == generated_id:
            return False

    for key, old in before.items():
        if key not in selected_keys and old != after[key]:
            return False
    for event_name in ("player_death", "round_start", "round_end"):
        if inspection._events(source_parser, event_name) != inspection._events(output_parser, event_name):
            return False
    return True


def verify_materialized_usp_rewrite(
    source: Path,
    output: Path,
    profile: CoreProfile,
    ticks: list[int],
    *,
    require_target_holding: bool = False,
) -> bool:
    """Fail closed on malformed parser evidence and semantic mismatches."""
    try:
        return _verify_materialized_usp_rewrite(
            source, output, profile, ticks,
            require_target_holding=require_target_holding,
        )
    except InspectionError:
        return False


def _int_health(row: dict[str, Any]) -> int:
    return inspection._int(inspection._field(row, "health"), "health")

