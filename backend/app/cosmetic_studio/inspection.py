"""Read-only Demo inspection for the local Cosmetic Studio experiment.

This module deliberately returns only parser evidence.  It does not launch a
game, resolve a native slot by nickname, or persist a roster/cache record.
"""

from __future__ import annotations

import hashlib
from pathlib import Path
from typing import Any

from demoparser2 import DemoParser

from .models import Candidate, Inspection, Player
from .catalog import weapon_matches, weapon_definition, WEAPON_NAMES


TICK_RATE = 64
AWP_WINDOW_BEFORE = 128
AWP_WINDOW_AFTER = 192
MIN_CANDIDATE_TICK = 256
_MAX_UINT64 = (1 << 64) - 1
_ACTIVE_WEAPON_FIELDS = (
    "active_weapon",
    "weapon",
    "m_hActiveWeapon",
    "CCSPlayerPawn.m_pWeaponServices.m_hActiveWeapon",
)
_TICK_SAMPLE_RADIUS = 256
_MAX_SAMPLE_TICKS = 20_000
_MAX_KILL_EVENTS = 256


class InspectionError(ValueError):
    """The parser returned data that is unsafe to interpret."""


def _xuid(value: Any) -> str:
    """Normalize only exact integer-like SteamIDs; never pass through float."""
    if isinstance(value, bool) or isinstance(value, float):
        return ""
    if isinstance(value, int):
        number = value
    elif isinstance(value, str) and value.strip().isdigit():
        number = int(value.strip(), 10)
    else:
        return ""
    if not 0 < number <= _MAX_UINT64:
        return ""
    return str(number)


def _rows(raw: Any, *, required: tuple[str, ...]) -> list[dict[str, Any]]:
    """Accept demoparser dict-of-columns, pandas-like frames, or row dicts."""
    if raw is None:
        raise InspectionError("parser returned no data")
    if hasattr(raw, "to_dict") and not isinstance(raw, dict):
        try:
            raw = raw.to_dict("records")
        except Exception as exc:  # pragma: no cover - parser-specific object
            raise InspectionError("parser table could not be materialized") from exc
    if isinstance(raw, list):
        result = [dict(row) for row in raw if isinstance(row, dict)]
        if len(result) != len(raw):
            raise InspectionError("parser rows contain a non-object value")
    elif isinstance(raw, dict):
        if not raw:
            return []
        if not all(isinstance(value, (list, tuple)) for value in raw.values()):
            raise InspectionError("parser columns are not sequences")
        lengths = {len(value) for value in raw.values()}
        if len(lengths) != 1:
            raise InspectionError("parser columns have different lengths")
        size = next(iter(lengths), 0)
        result = [{key: value[index] for key, value in raw.items()} for index in range(size)]
    else:
        raise InspectionError("unsupported parser result shape")
    missing = [key for key in required if key not in (result[0].keys() if result else required)]
    if missing and result:
        raise InspectionError(f"parser result missing fields: {', '.join(missing)}")
    return result


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _parser(path: Path) -> DemoParser:
    if not path.is_file():
        raise InspectionError(f"Demo file not found: {path}")
    return DemoParser(str(path))


def inspect_demo(path: Path) -> Inspection:
    """Build roster and bounded supported-gun death windows from parser output."""
    path = Path(path).expanduser().resolve()
    parser = _parser(path)
    roster_raw = parser.parse_player_info()
    if isinstance(roster_raw, dict) and {"steamid", "name"}.issubset(roster_raw):
        roster_rows = _rows(roster_raw, required=("steamid", "name"))
    else:
        roster_rows = _rows(roster_raw, required=("steamid", "name"))

    by_xuid: dict[str, Player] = {}
    for row in roster_rows:
        sid = _xuid(row.get("steamid", row.get("steamid64")))
        name = row.get("name")
        if not sid or not isinstance(name, str) or not name.strip():
            continue
        by_xuid.setdefault(sid, Player(xuid=sid, name=name.strip()))
    players = list(by_xuid.values())
    if not players:
        raise InspectionError("parser returned no valid SteamID roster")

    death_rows = _rows(parser.parse_event("player_death"), required=("tick", "weapon", "attacker_steamid"))
    awp_events: list[tuple[str, int, int]] = []
    for row in death_rows:
        weapon = str(row.get("weapon") or "").strip().lower()
        definition = weapon_definition(weapon)
        if definition is None:
            continue
        sid = _xuid(row.get("attacker_steamid", row.get("attacker_steamid64")))
        try:
            kill_tick = int(row.get("tick"))
        except (TypeError, ValueError, OverflowError):
            continue
        if not sid or sid not in by_xuid:
            continue
        if kill_tick >= MIN_CANDIDATE_TICK:
            awp_events.append((sid, kill_tick, definition))

    if not awp_events:
        return Inspection(demo_sha256=_sha256(path), players=players, candidates=[])

    sample_ticks = set()
    sampled_events = []
    for sid, kill_tick, definition in awp_events:
        if len(sampled_events) >= _MAX_KILL_EVENTS:
            break
        wanted = set(range(max(1, kill_tick - _TICK_SAMPLE_RADIUS), kill_tick + _TICK_SAMPLE_RADIUS + 1))
        if len(sample_ticks) + len(wanted - sample_ticks) > _MAX_SAMPLE_TICKS:
            continue
        sample_ticks.update(wanted)
        sampled_events.append((sid, kill_tick, definition))
    tick_rows = _rows(
        parser.parse_ticks(["steamid", "health", "active_weapon_name", "user_id"], ticks=sorted(sample_ticks)),
        required=(),
    )
    snapshot_rows: dict[tuple[str, int], list[dict[str, Any]]] = {}
    for row in tick_rows:
        try:
            tick_value = int(row.get("tick"))
        except (TypeError, ValueError, OverflowError):
            continue
        sid = _xuid(row.get("steamid", row.get("steamid64")))
        if not sid or sid not in by_xuid:
            continue
        snapshot_rows.setdefault((sid, tick_value), []).append(row)

    snapshots: dict[tuple[str, int], set[int]] = {}
    for (sid, tick_value), rows in snapshot_rows.items():
        if len(rows) != 1:
            continue
        row = rows[0]
        try:
            health = int(row.get("health"))
        except (TypeError, ValueError, OverflowError):
            continue
        definition = weapon_definition(row.get('active_weapon_name'))
        if health <= 0 or definition is None:
            continue
        snapshots.setdefault((sid, definition), set()).add(tick_value)

    candidates: list[Candidate] = []
    seen: set[tuple[str, int, int]] = set()
    for sid, kill_tick, definition in sampled_events:
        known = snapshots.get((sid, definition), set())
        if kill_tick not in known:
            continue
        run_start = kill_tick
        while run_start - 1 in known:
            run_start -= 1
        run_end = kill_tick
        while run_end + 1 in known:
            run_end += 1
        start = max(MIN_CANDIDATE_TICK, kill_tick - AWP_WINDOW_BEFORE, run_start + TICK_RATE + 4)
        end = min(kill_tick + AWP_WINDOW_AFTER, run_end - 4)
        if kill_tick < start or kill_tick > end or end - start < TICK_RATE * 2:
            continue
        key = (sid, start, end)
        if key in seen:
            continue
        seen.add(key)
        name = by_xuid[sid].name
        candidates.append(Candidate(xuid=sid, start_tick=start, end_tick=end, weapon_id=definition,
            label=f"{name} · {WEAPON_NAMES[definition][0]} · tick {start}–{end}"))

    return Inspection(demo_sha256=_sha256(path), players=players, candidates=candidates,
        candidates_limited=len(sampled_events) != len(awp_events))


def _active_weapon_is_awp(value: Any) -> bool:
    text = str(value or "").strip().lower()
    return text in {"awp", "weapon_awp"} or text.endswith("/awp")


def resolve_slot(path: Path, xuid: str, tick: int, weapon_id: int = 9) -> int:
    """Return ``user_id + 1`` only with exact identity and live-state evidence.

    Guns require a nearby matching death event. A same-model knife fragment
    uses snapshot evidence here and requires full interval/ownership checks
    in the service before native playback. This legacy slot is a hint only:
    the runtime independently resolves its controller from the exact XUID.
    """
    target = _xuid(xuid)
    if not target:
        raise InspectionError("invalid target XUID")
    try:
        tick_i = int(tick)
    except (TypeError, ValueError, OverflowError):
        raise InspectionError("invalid snapshot tick")
    if tick_i < 1:
        raise InspectionError("target is not an AWP event participant at the requested snapshot")
    parser = _parser(Path(path).expanduser().resolve())
    events = ([] if weapon_id == 507 else
        _rows(parser.parse_event("player_death"), required=("tick", "weapon", "attacker_steamid")))
    event_verified = weapon_id == 507
    for row in events:
        if _xuid(row.get("attacker_steamid", row.get("attacker_steamid64"))) != target:
            continue
        if not weapon_matches(row.get('weapon'), weapon_id):
            continue
        try:
            if abs(int(row.get("tick")) - tick_i) <= AWP_WINDOW_AFTER:
                event_verified = True
                break
        except (TypeError, ValueError, OverflowError):
            continue
    if not event_verified:
        raise InspectionError("target is not an AWP event participant at the requested snapshot")

    snapshot_tick = tick_i
    raw = parser.parse_ticks(["user_id", "steamid", "health", "active_weapon_name"], ticks=[snapshot_tick])
    rows = _rows(raw, required=("user_id", "steamid", "health"))
    matches = []
    active_fields = [field for field in ("active_weapon_name",) if rows and field in rows[0]]
    if not active_fields:
        raise InspectionError("parser did not expose active_weapon_name")
    for row in rows:
        if _xuid(row.get("steamid", row.get("steamid64"))) != target:
            continue
        try:
            user_id = int(row.get("user_id"))
            health = int(row.get("health"))
        except (TypeError, ValueError, OverflowError):
            continue
        if user_id < 0 or health <= 0:
            continue
        if active_fields and not any(weapon_matches(row.get(field), weapon_id) for field in active_fields):
            continue
        matches.append(user_id + 1)
    if len(set(matches)) != 1:
        raise InspectionError("target is not uniquely live in the requested snapshot")
    return matches[0]


__all__ = ["Inspection", "InspectionError", "inspect_demo", "resolve_slot"]
