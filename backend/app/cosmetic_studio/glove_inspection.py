"""Read-only parser qualification for the supported offline glove rewrite."""

from __future__ import annotations

import math
import re
from numbers import Integral
from pathlib import Path
from collections.abc import Sequence
from typing import Any

from .inspection import InspectionError, _parser, _rows, _xuid

_STEAM_BASE = 76561197960265728
_SOURCE_DEFINITION = 5030
_REPLACEMENT_DEFINITION = 5034
_REPLACEMENT_PAINT = 10061
_MAX_TICKS = 2048
_UINT32_MAX = (1 << 32) - 1
_UINT64_MAX = (1 << 64) - 1

_PAWN_DEFINITION = "CCSPlayerPawn.m_iItemDefinitionIndex"
_PAWN_HIGH = "CCSPlayerPawn.m_iItemIDHigh"
_PAWN_LOW = "CCSPlayerPawn.m_iItemIDLow"
_PAWN_ATTRIBUTE = "CCSPlayerPawn.CEconItemAttribute.m_iAttributeDefinitionIndex"
_PAWN_ATTRIBUTE_VALUE = "CCSPlayerPawn.CEconItemAttribute.m_flInitialValue"
_PAWN_NAME = "CCSPlayerPawn.m_szCustomName"
_GLOVE_PAINT = "glove_paint_id"
_GLOVE_SEED = "glove_paint_seed"
_GLOVE_WEAR = "glove_paint_float"

_GLOVE_FIELDS = (
    _PAWN_DEFINITION, _PAWN_HIGH, _PAWN_LOW, _PAWN_ATTRIBUTE,
    _PAWN_ATTRIBUTE_VALUE, _PAWN_NAME, _GLOVE_PAINT, _GLOVE_SEED, _GLOVE_WEAR,
)
_OWNER_FIELDS = (
    "original_owner_account_id", "original_owner_account",
    "m_iAccountID", "Weapon.m_iAccountID",
)
_PLAYER_FIELDS = (
    "tick", "steamid", "health", "team_num", "active_weapon_name",
    "item_id_high", "item_id_low", "item_def_idx", *_OWNER_FIELDS,
    *_GLOVE_FIELDS,
)
_GAMEPLAY_FIELDS = (
    "health", "team_num", "active_weapon_name", "item_id_high",
    "item_id_low", "item_def_idx",
)


def _integer(value: Any, label: str) -> int:
    if isinstance(value, bool) or isinstance(value, float):
        raise InspectionError(f"invalid {label}")
    if isinstance(value, Integral):
        return int(value)
    if isinstance(value, str) and re.fullmatch(r"(?:0|[1-9][0-9]*)", value.strip()):
        return int(value.strip(), 10)
    raise InspectionError(f"invalid {label}")


def _exact_xuid(value: Any) -> str:
    if not isinstance(value, str) or not re.fullmatch(r"[1-9][0-9]{0,19}", value):
        raise InspectionError("XUID must be an exact unsigned decimal string")
    normalized = _xuid(value)
    if normalized != value:
        raise InspectionError("invalid target XUID")
    return normalized


def _exact_item_id(value: Any) -> str:
    if isinstance(value, bool) or isinstance(value, float):
        raise InspectionError("item ID must be an exact decimal value")
    try:
        number = int(value)
    except (TypeError, ValueError, OverflowError) as exc:
        raise InspectionError("invalid item ID") from exc
    if isinstance(value, str) and str(number) != value.strip():
        raise InspectionError("item ID must be canonical decimal")
    if not 0 < number <= _UINT64_MAX:
        raise InspectionError("item ID is outside uint64 range")
    return str(number)


def _compose_item_id(row: dict[str, Any], high_field: str, low_field: str) -> str:
    high = _integer(row.get(high_field), "item ID high")
    low = _integer(row.get(low_field), "item ID low")
    if not (0 <= high <= _UINT32_MAX and 0 <= low <= _UINT32_MAX):
        raise InspectionError("item ID components are outside uint32 range")
    item_id = (high << 32) | low
    if item_id == 0:
        raise InspectionError("empty glove item ID")
    return str(item_id)


def _target_rows(parser: Any, ticks: Sequence[int], target: str) -> dict[int, dict[str, Any]]:
    raw = parser.parse_ticks(list(_PLAYER_FIELDS), ticks=list(ticks))
    rows = _rows(raw, required=())
    if not rows:
        raise InspectionError("parser returned no Pawn/player samples")
    wanted = set(ticks)
    found: dict[int, list[dict[str, Any]]] = {}
    for row in rows:
        if "tick" not in row:
            raise InspectionError("parser row is missing actual tick")
        tick = _integer(row["tick"], "parser tick")
        if tick not in wanted:
            raise InspectionError("parser returned an unrequested tick")
        sid = _xuid(row.get("steamid", row.get("steamid64")))
        if sid == target:
            found.setdefault(tick, []).append(row)
    result: dict[int, dict[str, Any]] = {}
    for tick in ticks:
        matches = found.get(tick, [])
        if not matches:
            raise InspectionError(f"missing target Pawn sample at tick {tick}")
        if len(matches) != 1:
            raise InspectionError(f"duplicate target Pawn samples at tick {tick}")
        result[tick] = matches[0]
    return result


def _parse_skins_owner(parser: Any, item_id: str, target: str) -> None:
    rows = _rows(parser.parse_skins(), required=())
    matches = []
    for row in rows:
        try:
            row_item = _exact_item_id(row.get("item_id"))
        except InspectionError:
            continue
        if row_item == item_id:
            matches.append(row)
    if len(matches) != 1:
        raise InspectionError("parse_skins does not uniquely identify the Pawn glove item")
    row = matches[0]
    owner = _xuid(row.get("steamid", row.get("steamid64")))
    if owner != target:
        raise InspectionError("parse_skins glove owner does not match target XUID")
    if _integer(row.get("def_index"), "parse_skins glove definition") != _SOURCE_DEFINITION:
        raise InspectionError("parse_skins item is not the supported source glove")


def resolve_owned_glove(path: Path | str, xuid: str, tick: int) -> tuple[str, int]:
    """Resolve one live 5030 Pawn glove and corroborate its owner via parse_skins.

    Current demoparser output exposes no documented Pawn glove owner-account
    field. Ownership is therefore established by the exact ``item_id`` and
    ``steamid`` pair in ``parse_skins``; active-weapon account fields are not
    treated as glove ownership.
    """
    target = _exact_xuid(xuid)
    tick_value = _integer(tick, "tick")
    if tick_value < 1:
        raise InspectionError("tick must be positive")
    parser = _parser(Path(path).expanduser().resolve())
    row = _target_rows(parser, [tick_value], target)[tick_value]
    if _integer(row.get("health"), "health") <= 0:
        raise InspectionError("target Pawn is not alive")
    definition = _integer(row.get(_PAWN_DEFINITION), "Pawn glove definition")
    if definition != _SOURCE_DEFINITION:
        raise InspectionError("target Pawn does not hold the supported source glove")
    item_id = _compose_item_id(row, _PAWN_HIGH, _PAWN_LOW)
    _parse_skins_owner(parser, item_id, target)
    return item_id, definition


def _valid_ticks(ticks: Any) -> list[int] | None:
    if isinstance(ticks, (str, bytes)) or not isinstance(ticks, Sequence) or not 1 <= len(ticks) <= _MAX_TICKS:
        return None
    result = []
    for value in ticks:
        try:
            tick = _integer(value, "sample tick")
        except InspectionError:
            return None
        if tick < 1:
            return None
        result.append(tick)
    if len(set(result)) != len(result):
        return None
    return result


def _all_player_rows(parser: Any, ticks: list[int]) -> dict[tuple[str, int], dict[str, Any]]:
    raw = parser.parse_ticks(list(_PLAYER_FIELDS), ticks=list(ticks))
    rows = _rows(raw, required=())
    if not rows:
        raise InspectionError("parser returned no player samples")
    wanted = set(ticks)
    keyed: dict[tuple[str, int], dict[str, Any]] = {}
    for row in rows:
        if "tick" not in row:
            raise InspectionError("parser row is missing actual tick")
        tick = _integer(row["tick"], "parser tick")
        if tick not in wanted:
            raise InspectionError("parser returned an unrequested tick")
        sid = _xuid(row.get("steamid", row.get("steamid64")))
        if not sid:
            raise InspectionError("parser returned a player row without exact XUID")
        key = (sid, tick)
        if key in keyed:
            raise InspectionError("duplicate player/tick sample")
        keyed[key] = row
    if {tick for _, tick in keyed} != wanted:
        raise InspectionError("parser omitted one or more requested ticks")
    return keyed


def _header(parser: Any) -> dict[str, Any]:
    value = parser.parse_header()
    if not isinstance(value, dict):
        raise InspectionError("parser header is unavailable")
    return value


def _event_rows(parser: Any, event_name: str) -> list[dict[str, Any]]:
    return _rows(parser.parse_event(event_name), required=())


def _owner_fingerprint(row: dict[str, Any]) -> tuple[tuple[str, Any], ...]:
    return tuple((name, row.get(name)) for name in _OWNER_FIELDS if name in row)


def _wear_matches(value: Any, wanted: float) -> bool:
    if isinstance(value, bool):
        return False
    try:
        number = float(value)
    except (TypeError, ValueError, OverflowError):
        return False
    return math.isfinite(number) and abs(number - wanted) <= 1e-6


def verify_glove_rewrite(
    source: Path | str,
    output: Path | str,
    xuid: str,
    item_id: str,
    source_definition: int,
    replacement_definition: int,
    paint: int,
    seed: int,
    wear: float,
    ticks: Sequence[int],
) -> bool:
    """Compare only authorized glove cosmetics plus required gameplay facts."""
    sampled = _valid_ticks(ticks)
    if sampled is None:
        return False
    try:
        target = _exact_xuid(xuid)
        expected_item = _exact_item_id(item_id)
        if (
            _integer(source_definition, "source definition") != _SOURCE_DEFINITION
            or _integer(replacement_definition, "replacement definition") != _REPLACEMENT_DEFINITION
            or _integer(paint, "paint") != _REPLACEMENT_PAINT
            or _integer(seed, "seed") != 0
            or isinstance(wear, bool)
            or not _wear_matches(wear, 0.01)
        ):
            return False
        source_parser = _parser(Path(source).expanduser().resolve())
        output_parser = _parser(Path(output).expanduser().resolve())
        _parse_skins_owner(source_parser, expected_item, target)
        if _header(source_parser) != _header(output_parser):
            return False
        before = _all_player_rows(source_parser, sampled)
        after = _all_player_rows(output_parser, sampled)
        if set(before) != set(after):
            return False
        for event in ("player_death", "round_start", "round_end"):
            if _event_rows(source_parser, event) != _event_rows(output_parser, event):
                return False

        for key, old in before.items():
            new = after[key]
            sid, tick = key
            required_fields = (*_GAMEPLAY_FIELDS, *_GLOVE_FIELDS)
            if any(field not in old or field not in new for field in required_fields):
                return False
            for field in _GAMEPLAY_FIELDS:
                if old.get(field) != new.get(field):
                    return False
            if _owner_fingerprint(old) != _owner_fingerprint(new):
                return False
            if sid != target:
                if any(old.get(field) != new.get(field) for field in _GLOVE_FIELDS):
                    return False
                continue

            if _integer(old.get("health"), "health") <= 0 or _integer(new.get("health"), "health") <= 0:
                return False
            if _compose_item_id(old, _PAWN_HIGH, _PAWN_LOW) != expected_item:
                return False
            if _compose_item_id(new, _PAWN_HIGH, _PAWN_LOW) != expected_item:
                return False
            if _integer(old.get(_PAWN_DEFINITION), "source Pawn definition") != _SOURCE_DEFINITION:
                return False
            if _integer(new.get(_PAWN_DEFINITION), "replacement Pawn definition") != _REPLACEMENT_DEFINITION:
                return False
            if _integer(new.get(_GLOVE_PAINT), "replacement glove paint") != _REPLACEMENT_PAINT:
                return False
            if not _wear_matches(new.get(_GLOVE_SEED), 0.0):
                return False
            if not _wear_matches(new.get(_GLOVE_WEAR), 0.01):
                return False
            if not _wear_matches(new.get(_PAWN_ATTRIBUTE_VALUE), 0.01):
                return False
            if old.get(_PAWN_ATTRIBUTE) != new.get(_PAWN_ATTRIBUTE):
                return False

        target_keys = {key for key in before if key[0] == target}
        if {tick for _, tick in target_keys} != set(sampled):
            return False
        return True
    except (InspectionError, TypeError, ValueError, OverflowError, KeyError):
        return False


__all__ = ["resolve_owned_glove", "verify_glove_rewrite"]
