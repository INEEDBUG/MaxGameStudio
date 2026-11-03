"""Manual, parser-verified Demo intervals for supported cosmetic presets."""

from __future__ import annotations

import re
from numbers import Integral
from pathlib import Path
from typing import Any

from .catalog import PRESET_DATA, weapon_matches
from .inspection import InspectionError, _parser, _rows, _xuid
from .models import Candidate

_STEAM_BASE = 76561197960265728
_TICK_RATE = 64
_MIN_START = 256
_MIN_DURATION = 2 * _TICK_RATE
_MAX_DURATION = 30 * _TICK_RATE
_LOOKBEHIND = _TICK_RATE
_MAX_SAMPLE_TICKS = 2048
_UINT32_MAX = (1 << 32) - 1
_OWNER_FIELDS = (
    "original_owner_account_id",
    "original_owner_account",
    "m_iAccountID",
    "Weapon.m_iAccountID",
)


def _strict_int(value: Any, field: str) -> int:
    if isinstance(value, bool) or isinstance(value, float):
        raise InspectionError(f"invalid {field}")
    if isinstance(value, Integral):
        return int(value)
    if isinstance(value, str) and re.fullmatch(r"-?(0|[1-9][0-9]*)", value.strip()):
        return int(value.strip(), 10)
    raise InspectionError(f"invalid {field}")


def _exact_xuid(value: Any) -> str:
    if not isinstance(value, str) or not re.fullmatch(r"[1-9][0-9]{0,19}", value):
        raise InspectionError("XUID must be an exact unsigned 64-bit decimal string")
    normalized = _xuid(value)
    if not normalized or normalized != value:
        raise InspectionError("invalid target XUID")
    return normalized


def _item_identity(row: dict[str, Any]) -> tuple[int, int]:
    high = _strict_int(row.get("item_id_high", row.get("m_iItemIDHigh")), "item id high")
    low = _strict_int(row.get("item_id_low", row.get("m_iItemIDLow")), "item id low")
    if not (0 < high <= _UINT32_MAX and 0 < low <= _UINT32_MAX):
        raise InspectionError("weapon item id components must both be non-zero uint32 values")
    return high, low


def _owner_account(row: dict[str, Any]) -> int:
    present = [row[name] for name in _OWNER_FIELDS if name in row]
    if not present:
        raise InspectionError("parser row is missing original owner account")
    owners = {_strict_int(value, "original owner account") for value in present}
    if len(owners) != 1:
        raise InspectionError("parser returned conflicting original owner accounts")
    return owners.pop()


def validate_interval(
    path: Path | str,
    xuid: str,
    start_tick: int,
    end_tick: int,
    weapon_id: int,
) -> Candidate:
    """Validate an explicitly selected continuous, owned-weapon Demo segment.

    The parser is read-only. The returned candidate is configuration for a
    later user-confirmed workflow; this function never launches a core/game or
    writes to the source Demo.
    """
    target = _exact_xuid(xuid)
    start = _strict_int(start_tick, "start tick")
    end = _strict_int(end_tick, "end tick")
    definition = _strict_int(weapon_id, "weapon definition")
    supported_definitions = {entry[0] for entry in PRESET_DATA.values()}
    if definition not in supported_definitions:
        raise InspectionError("unsupported weapon definition")
    if start < _MIN_START:
        raise InspectionError("interval start must be at least tick 256")
    duration = end - start
    if not _MIN_DURATION <= duration <= _MAX_DURATION:
        raise InspectionError("manual interval must span 2–30 seconds")

    ticks = list(range(start - _LOOKBEHIND, end + 1))
    if len(ticks) > _MAX_SAMPLE_TICKS:
        raise InspectionError("interval sample exceeds parser tick limit")

    parser = _parser(Path(path).expanduser().resolve())
    fields = [
        "tick", "steamid", "health", "active_weapon_name", "item_id_high",
        "item_id_low", "item_def_idx", *_OWNER_FIELDS,
    ]
    raw = parser.parse_ticks(fields, ticks=ticks)
    rows = _rows(raw, required=())
    if not rows:
        raise InspectionError("parser returned no player samples")

    requested = set(ticks)
    by_tick: dict[int, list[dict[str, Any]]] = {}
    for row in rows:
        if "tick" not in row:
            raise InspectionError("parser row is missing its actual tick")
        actual_tick = _strict_int(row["tick"], "parser tick")
        if actual_tick not in requested:
            raise InspectionError("parser returned a tick outside the requested sample")
        row_xuid = _xuid(row.get("steamid", row.get("steamid64")))
        if row_xuid == target:
            by_tick.setdefault(actual_tick, []).append(row)

    stable_item: tuple[int, int] | None = None
    expected_owner = int(target) - _STEAM_BASE
    for tick in ticks:
        target_rows = by_tick.get(tick, [])
        if not target_rows:
            raise InspectionError(f"missing target sample at tick {tick}")
        if len(target_rows) != 1:
            raise InspectionError(f"duplicate target rows at tick {tick}")
        row = target_rows[0]
        health = _strict_int(row.get("health"), "health")
        if health <= 0:
            raise InspectionError(f"target is not alive at tick {tick}")
        if not weapon_matches(row.get("active_weapon_name"), definition):
            raise InspectionError(f"target is not actively holding the selected definition at tick {tick}")
        row_definition = _strict_int(row.get("item_def_idx"), "item definition")
        if row_definition != definition:
            raise InspectionError(f"active item definition mismatch at tick {tick}")
        item = _item_identity(row)
        if stable_item is None:
            stable_item = item
        elif item != stable_item:
            raise InspectionError(f"active weapon item changed at tick {tick}")
        if _owner_account(row) != expected_owner:
            raise InspectionError(f"weapon original owner does not match target at tick {tick}")

    return Candidate(
        xuid=target,
        start_tick=start,
        end_tick=end,
        weapon_id=definition,
        label=f"{target} · 手动片段 · weapon {definition} · tick {start}–{end}",
    )
