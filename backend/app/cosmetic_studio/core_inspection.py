"""Independent, parser-backed checks for SkinCore Demo rewrites."""

from __future__ import annotations

import math
from pathlib import Path
from typing import Any

from .inspection import InspectionError, _parser, _rows, _xuid, _active_weapon_is_awp
from .skin_core import CoreProfile, CoreGloveItem
from .catalog import weapon_matches

_BASE = 76561197960265728
_MAX_TICKS = 2048
_COSMETIC_KEYS = {"paint", "paint_kit", "item_paint_kit", "m_nFallbackPaintKit",
                  "seed", "pattern_seed", "wear", "float", "custom_name",
                  "m_flFallbackWear", "m_nFallbackSeed", "weapon_skin_id",
                  "weapon_paint_seed", "weapon_float"}


def _int(value: Any, label: str) -> int:
    if isinstance(value, bool) or isinstance(value, float):
        raise InspectionError(f"invalid {label}")
    try:
        result = int(value)
    except (TypeError, ValueError, OverflowError) as exc:
        raise InspectionError(f"invalid {label}") from exc
    if isinstance(value, str) and str(result) != value.strip():
        raise InspectionError(f"invalid {label}")
    return result


def _field(row: dict[str, Any], *names: str) -> Any:
    for name in names:
        if name in row:
            return row[name]
    raise InspectionError(f"parser row missing {'/'.join(names)}")


def _item_id(row: dict[str, Any]) -> int:
    high = _int(_field(row, "item_id_high", "m_iItemIDHigh"), "item_id_high")
    low = _int(_field(row, "item_id_low", "m_iItemIDLow"), "item_id_low")
    if not 0 <= high <= 0xFFFFFFFF or not 0 <= low <= 0xFFFFFFFF:
        raise InspectionError("invalid item id component")
    value = (high << 32) | low
    if value == 0:
        raise InspectionError("empty item id")
    return value


def _maybe_item_id(row: dict[str, Any]) -> int | None:
    high = row.get("item_id_high", row.get("m_iItemIDHigh"))
    low = row.get("item_id_low", row.get("m_iItemIDLow"))
    if (high is None and low is None) or (high == 0 and low == 0):
        return None
    return _item_id(row)


def _steam(row: dict[str, Any]) -> str:
    value = _xuid(row.get("steamid", row.get("steamid64")))
    if not value:
        raise InspectionError("invalid player SteamID")
    return value


def _ticks(parser: Any, ticks: list[int]) -> list[dict[str, Any]]:
    if not isinstance(ticks, list) or not ticks or len(ticks) > _MAX_TICKS:
        raise InspectionError("tick sample is empty or exceeds limit")
    normalized = []
    for tick in ticks:
        value = _int(tick, "tick")
        if value < 1:
            raise InspectionError("invalid tick")
        normalized.append(value)
    raw = parser.parse_ticks([
        "steamid", "health", "team_num", "active_weapon_name", "item_id_high", "item_id_low",
        "m_iAccountID", "Weapon.m_iAccountID", "m_nFallbackPaintKit", "m_nFallbackSeed", "m_flFallbackWear",
        "weapon_skin_id", "weapon_paint_seed", "weapon_float",
        "item_def_idx",
    ], ticks=sorted(set(normalized)))
    rows = _rows(raw, required=())
    if not rows:
        raise InspectionError("parser returned no player samples")
    return rows


def resolve_owned_weapon(path: Path, xuid: str, tick: int, definition_index: int) -> str:
    """Resolve one non-zero owned gun item ID from an exact live player row."""
    target = _xuid(xuid)
    if not target:
        raise InspectionError("invalid target XUID")
    parser = _parser(Path(path).expanduser().resolve())
    rows = _ticks(parser, [_int(tick, "tick")])
    matches = []
    target_account = _int(target, "target XUID") - _BASE
    if target_account <= 0:
        raise InspectionError("invalid target account")
    for row in rows:
        if _steam(row) != target:
            continue
        if _int(_field(row, "health"), "health") <= 0:
            continue
        if any(name in row for name in ("item_definition_index", "definition_index", "item_def_idx")):
            if _int(_field(row, "item_definition_index", "definition_index", "item_def_idx"), "definition") != definition_index:
                continue
        if not weapon_matches(_field(row, 'active_weapon_name'), definition_index):
            continue
        owner = _int(_field(row, "original_owner_account_id", "original_owner_account", "m_iAccountID", "Weapon.m_iAccountID"), "owner account")
        if owner != target_account:
            raise InspectionError("original owner does not match target account")
        matches.append(_item_id(row))
    if len(matches) != 1:
        raise InspectionError("target is not uniquely live with the selected owned weapon")
    return str(matches[0])


def resolve_owned_awp(path: Path, xuid: str, tick: int) -> str:
    return resolve_owned_weapon(path, xuid, tick, 9)


def _header(parser: Any) -> dict[str, Any]:
    raw = parser.parse_header()
    if not isinstance(raw, dict):
        raise InspectionError("parser header is not an object")
    return raw


def _gameplay(row: dict[str, Any]) -> dict[str, Any]:
    return {key: value for key, value in row.items()
            if key.lower() not in {name.lower() for name in _COSMETIC_KEYS}}


def _events(parser: Any, name: str) -> list[dict[str, Any]]:
    rows = _rows(parser.parse_event(name), required=())
    return [_gameplay(row) for row in rows]


def _number(row: dict[str, Any], *names: str) -> float:
    value = _field(row, *names)
    if isinstance(value, bool):
        raise InspectionError("invalid cosmetic number")
    try:
        result = float(value)
    except (TypeError, ValueError, OverflowError) as exc:
        raise InspectionError("invalid cosmetic number") from exc
    if not math.isfinite(result):
        raise InspectionError("invalid cosmetic number")
    return result


def verify_rewrite(source: Path, output: Path, profile: CoreProfile, ticks: list[int], *, require_target_holding: bool = False) -> bool:
    """Independently prove bounded gameplay preservation and requested gun changes."""
    source_parser, output_parser = _parser(Path(source)), _parser(Path(output))
    if _header(source_parser) != _header(output_parser):
        return False
    source_rows, output_rows = _ticks(source_parser, ticks), _ticks(output_parser, ticks)
    def keyed(rows):
        result = {}
        for row in rows:
            key = (_steam(row), _int(_field(row, "tick"), "tick"))
            if key in result:
                raise InspectionError("duplicate player sample")
            result[key] = row
        return result
    before, after = keyed(source_rows), keyed(output_rows)
    if set(before) != set(after):
        return False
    requested_ticks = {_int(tick, "tick") for tick in ticks}
    if ({key[1] for key in before} | {key[1] for key in after}) != requested_ticks:
        return False
    targets = {item.item_id64: item for item in profile.items}
    if not targets:
        raise InspectionError("profile has no supported owned item")
    # A target must hold a requested owned item at EVERY required tick, not
    # merely once in the interval. Cross-model knife identity is source-bound.
    for tick in requested_ticks if require_target_holding else ():
        key = (profile.steam_id64, tick)
        if key not in before or key not in after:
            return False
        item = targets.get(str(_maybe_item_id(before[key])))
        if item is None or str(_maybe_item_id(after[key])) != item.item_id64:
            return False
        for row, expected_definition in (
            (before[key], item.definition_index),
            (after[key], item.replacement_definition_index or item.definition_index),
        ):
            if (_int(_field(row, 'health'), 'health') <= 0
                    or not weapon_matches(_field(row, 'active_weapon_name'), expected_definition)):
                return False
            if (item.replacement_definition_index is not None
                    and _int(_field(row, 'item_def_idx'), 'definition') != expected_definition):
                return False
    changed = set()
    for key, old in before.items():
        new = after[key]
        item_value = _maybe_item_id(new)
        item = str(item_value) if item_value is not None else None
        wanted = targets.get(item)
        cross_model_target = bool(wanted and wanted.replacement_definition_index is not None)
        if cross_model_target:
            try:
                if (_int(_field(old, 'item_def_idx'), 'source definition') != wanted.definition_index
                        or _int(_field(new, 'item_def_idx'), 'replacement definition')
                        != wanted.replacement_definition_index):
                    return False
            except InspectionError:
                return False
        elif old.get('item_def_idx') != new.get('item_def_idx'):
            return False
        for fields in (("health",), ("team", "team_num"),
                       ("item_id_high",), ("item_id_low",),
                       ("original_owner_account_id", "original_owner_account", "m_iAccountID", "Weapon.m_iAccountID")):
            if _field(old, *fields) != _field(new, *fields):
                return False
        if not cross_model_target and _field(old, 'active_weapon_name') != _field(new, 'active_weapon_name'):
            return False
        if wanted is not None:
            owner = _int(_field(new, "original_owner_account_id", "original_owner_account", "m_iAccountID", "Weapon.m_iAccountID"), "owner account")
            target_definition = wanted.replacement_definition_index or wanted.definition_index
            if (owner != _int(profile.steam_id64, "profile XUID") - _BASE
                    or not weapon_matches(_field(old, 'active_weapon_name'), wanted.definition_index)
                    or not weapon_matches(_field(new, 'active_weapon_name'), target_definition)):
                return False
            if (any(name in new for name in ("item_definition_index", "definition_index", "item_def_idx"))
                    and _int(_field(new, "item_definition_index", "definition_index", "item_def_idx"), "definition") != target_definition
                    or abs(_number(new, "paint_kit", "item_paint_kit", "weapon_skin_id", "m_nFallbackPaintKit") - wanted.paint_kit) > 1e-6
                    or abs(_number(new, "pattern_seed", "seed", "weapon_paint_seed", "m_nFallbackSeed") - wanted.pattern_seed) > 1e-6
                    or abs(_number(new, "wear", "weapon_float", "m_flFallbackWear") - wanted.wear) > 1e-6):
                return False
            changed.add(item)
        elif item_value is not None:
            for aliases in (("paint_kit", "item_paint_kit", "weapon_skin_id", "m_nFallbackPaintKit"),
                            ("pattern_seed", "seed", "weapon_paint_seed", "m_nFallbackSeed"),
                            ("wear", "weapon_float", "m_flFallbackWear")):
                if _field(old, *aliases) != _field(new, *aliases):
                    return False
    if changed != set(targets):
        return False
    for event in ("player_death", "round_start", "round_end"):
        if _events(source_parser, event) != _events(output_parser, event):
            return False
    return True


def verify_profile_rewrite(source: Path, output: Path, profile: CoreProfile, ticks: list[int]) -> bool:
    """Compose independently checked weapon and optional Pawn-glove changes."""
    weapons = tuple(item for item in profile.items if not isinstance(item, CoreGloveItem))
    gloves = tuple(item for item in profile.items if isinstance(item, CoreGloveItem))
    if not weapons or len(gloves) > 1:
        return False
    weapon_profile = CoreProfile(steam_id64=profile.steam_id64, items=weapons)
    if not verify_rewrite(source, output, weapon_profile, ticks, require_target_holding=True):
        return False
    if gloves:
        from .glove_inspection import verify_glove_rewrite
        glove = gloves[0]
        if not verify_glove_rewrite(source, output, profile.steam_id64, glove.item_id64,
                glove.definition_index, glove.replacement_definition_index,
                glove.paint_kit, glove.pattern_seed, glove.wear, ticks):
            return False
    return True


__all__ = ["resolve_owned_awp", "resolve_owned_weapon", "verify_rewrite", "verify_profile_rewrite"]
