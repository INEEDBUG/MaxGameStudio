"""Bounded, parser-backed appearance inventory; no synthesized ownership rows."""

from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
import math
from numbers import Integral
from pathlib import Path
import struct
from functools import lru_cache
from typing import Any

from .catalog import GLOVE_PRESET_DATA, PRESET_DATA, WEAR_LIMITS, WEAPON_NAMES, GUN_FINISH_METADATA, source_weapon_id, weapon_matches
from .inspection import InspectionError, _parser, _rows, _xuid
from .appearance_models import AppearanceInventoryResponse, AppearanceItem, AppearancePlayer, AppearancePreset, AppearanceReplacement, AppearanceSettings
from .runtime import digest


_MAX_SAMPLE_TICKS = 20_000
_SAMPLE_BLOCK_TICKS = 512
_VERIFY_WINDOW_TICKS = 128
_MAX_OBSERVED_ITEMS_PER_PLAYER = 128
_STEAM_BASE = 76561197960265728
_TEAM_BY_NUMBER = {2: "t", 3: "ct"}
_PLAYER_TICK_FIELDS = (
    "tick", "steamid", "health", "team_num", "active_weapon_name", "item_id_high", "item_id_low",
    "item_def_idx", "original_owner_account_id", "original_owner_account", "m_iAccountID", "Weapon.m_iAccountID",
    "weapon_skin_id", "weapon_paint_seed", "weapon_float",
)


class AppearanceInventoryError(ValueError):
    pass


@dataclass(frozen=True)
class InventorySnapshot:
    response: AppearanceInventoryResponse
    verification_ticks: dict[str, tuple[int, ...]]


@lru_cache(maxsize=1)
def _appearance_metadata() -> dict[str, Any]:
    path = Path(__file__).with_name("appearance_catalog_metadata.json")
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as exc:
        raise AppearanceInventoryError("Qualified appearance metadata is unavailable") from exc
    if payload.get("commit") != "e2644abaf1bb26b871d0e90e7b44b7116feef486" or payload.get("license") != "MIT; image artwork remains Valve property":
        raise AppearanceInventoryError("Qualified appearance metadata provenance is invalid")
    return {**payload.get("items", {}), **GUN_FINISH_METADATA}


def _exact_int(value: Any, field: str, *, minimum: int = 0, maximum: int = (1 << 64) - 1) -> int:
    if isinstance(value, bool) or isinstance(value, float) or not isinstance(value, Integral):
        raise AppearanceInventoryError(f"Invalid parser {field}")
    number = int(value)
    if number < minimum or number > maximum:
        raise AppearanceInventoryError(f"Invalid parser {field}")
    return number


def _wear(value: Any) -> float | None:
    if value is None:
        return None
    if isinstance(value, bool):
        return None
    if isinstance(value, Integral):
        bits = int(value)
        if not 0 <= bits <= 0xFFFFFFFF:
            return None
        number = struct.unpack("<f", struct.pack("<I", bits))[0]
    elif isinstance(value, (int, float)):
        number = float(value)
    else:
        return None
    return number if math.isfinite(number) and 0.0 <= number <= 1.0 else None


def _float32_bits_to_int(value: Any) -> int | None:
    try:
        number = _exact_int(value, "paint wear bits", maximum=0xFFFFFFFF)
    except AppearanceInventoryError:
        return None
    return number


def _owner_account(row: dict[str, Any]) -> int | None:
    accounts = set()
    for name in ("original_owner_account_id", "original_owner_account", "m_iAccountID", "Weapon.m_iAccountID"):
        if name in row:
            try:
                accounts.add(_exact_int(row[name], "original owner account", maximum=0xFFFFFFFF))
            except AppearanceInventoryError:
                return None
    return next(iter(accounts)) if len(accounts) == 1 else None


def _row_item_id(row: dict[str, Any]) -> str:
    value = _exact_int(row.get("item_id"), "item ID")
    if value == 0:
        raise AppearanceInventoryError("Owned parser item has an empty item identity")
    return str(value)


def _item_identity(row: dict[str, Any]) -> str | None:
    try:
        high = _exact_int(row.get("item_id_high"), "item ID high", maximum=0xFFFFFFFF)
        low = _exact_int(row.get("item_id_low"), "item ID low", maximum=0xFFFFFFFF)
    except AppearanceInventoryError:
        return None
    value = (high << 32) | low
    return str(value) if value else None


def _catalog_name(definition: int, paint: int, observed_name: str | None = None) -> tuple[str, str | None]:
    metadata = _display_metadata().get(f"{definition}:{paint}") or _appearance_metadata().get(f"{definition}:{paint}")
    if isinstance(metadata, dict):
        return str(metadata.get("name") or f"Item {definition} · Paint {paint}"), metadata.get("image_url")
    names = WEAPON_NAMES.get(definition, ("",))
    label = observed_name or (names[0].upper() if names and names[0] else f"未识别装备（编号 {definition}）")
    return (f"{label} · 皮肤编号 {paint}" if paint else f"{label} · 原始外观"), None


@lru_cache(maxsize=1)
def _display_metadata() -> dict[str, Any]:
    """Presentation only; PRESET_DATA remains the separate rewrite allowlist."""
    path = Path(__file__).with_name("appearance_display_catalog.json")
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as exc:
        raise AppearanceInventoryError("Appearance display metadata is unavailable") from exc
    if (payload.get("commit") != "e2644abaf1bb26b871d0e90e7b44b7116feef486"
            or payload.get("license") != "MIT; image artwork remains Valve property"):
        raise AppearanceInventoryError("Appearance display metadata provenance is invalid")
    return payload.get("items", {})


def preset_catalog() -> list[AppearancePreset]:
    metadata = _appearance_metadata()
    result = []
    for preset_id, (target, paint, label) in PRESET_DATA.items():
        minimum, maximum = WEAR_LIMITS.get((target, paint), (0.0, 1.0))
        item = metadata.get(f"{target}:{paint}")
        if isinstance(item, dict):
            minimum = float(item.get("wear_min", minimum))
            maximum = float(item.get("wear_max", maximum))
        result.append(AppearancePreset(
            id=preset_id,
            label=str(item.get("name")) if isinstance(item, dict) and item.get("name") else label,
            source_definition=source_weapon_id(preset_id),
            target_definition=target,
            paint_index=paint,
            wear_min=minimum,
            wear_max=maximum,
            image_url=item.get("image_url") if isinstance(item, dict) else None,
        ))
    glove_metadata = metadata.get("5034:10061", {})
    result.append(AppearancePreset(
        id="specialist-10061", label=glove_metadata.get("name") or GLOVE_PRESET_DATA["specialist-10061"],
        source_definition=5030, target_definition=5034, paint_index=10061,
        wear_min=0.01, wear_max=0.01, image_url=glove_metadata.get("image_url"),
    ))
    return result


def _round_start_ticks(parser: Any) -> list[int]:
    try:
        rows = _rows(parser.parse_event("round_start"), required=())
    except (AttributeError, InspectionError, ValueError, TypeError):
        return []
    ticks = set()
    for row in rows:
        try:
            tick = _exact_int(row.get("tick"), "round_start tick", minimum=1, maximum=100_000_000)
        except AppearanceInventoryError:
            continue
        ticks.add(tick)
    return sorted(ticks)


def _sample_blocks(parser: Any) -> tuple[list[tuple[int, ...]], dict[tuple[str, int], dict[str, Any]]]:
    round_ticks = _round_start_ticks(parser)
    if not round_ticks:
        return [], {}
    block_count = max(1, _MAX_SAMPLE_TICKS // _SAMPLE_BLOCK_TICKS)
    if len(round_ticks) > block_count:
        indexes = {round((len(round_ticks) - 1) * index / (block_count - 1)) for index in range(block_count)} if block_count > 1 else {0}
        round_ticks = [round_ticks[index] for index in sorted(indexes)]
    blocks = []
    for start in round_ticks:
        ticks = tuple(range(start, start + _SAMPLE_BLOCK_TICKS))
        blocks.append(ticks)
    unique_ticks = sorted({tick for block in blocks for tick in block})
    raw = parser.parse_ticks(list(_PLAYER_TICK_FIELDS), ticks=unique_ticks)
    rows = _rows(raw, required=())
    requested = set(unique_ticks)
    keyed: dict[tuple[str, int], dict[str, Any]] = {}
    duplicates: set[tuple[str, int]] = set()
    for row in rows:
        if "tick" not in row:
            continue
        try:
            tick = _exact_int(row["tick"], "actual tick", minimum=1, maximum=100_000_000)
        except AppearanceInventoryError:
            continue
        if tick not in requested:
            raise AppearanceInventoryError("Parser returned a tick outside the bounded inventory sample")
        xuid = _xuid(row.get("steamid", row.get("steamid64")))
        if not xuid:
            continue
        key = (xuid, tick)
        if key in keyed:
            duplicates.add(key)
        else:
            keyed[key] = row
    for key in duplicates:
        keyed.pop(key, None)
    return blocks, keyed


def _verified_window(
    blocks: list[tuple[int, ...]],
    keyed: dict[tuple[str, int], dict[str, Any]],
    *,
    xuid: str,
    item_id: str,
    definition: int,
    minimum_ticks: int = _VERIFY_WINDOW_TICKS,
) -> tuple[tuple[int, ...], str] | None:
    wanted_owner = int(xuid) - _STEAM_BASE
    for block in blocks:
        window_size = min(_VERIFY_WINDOW_TICKS, len(block))
        if window_size < minimum_ticks:
            continue
        for offset in range(len(block) - window_size + 1):
            ticks = block[offset:offset + window_size]
            teams: set[str] = set()
            valid = True
            for tick in ticks:
                row = keyed.get((xuid, tick))
                if row is None:
                    valid = False
                    break
                try:
                    health = _exact_int(row.get("health"), "health", maximum=1000)
                    row_definition = _exact_int(row.get("item_def_idx"), "active definition", maximum=100_000)
                    team_number = _exact_int(row.get("team_num"), "team", maximum=16)
                except AppearanceInventoryError:
                    valid = False
                    break
                if (health <= 0 or row_definition != definition
                        or not weapon_matches(row.get("active_weapon_name"), definition)
                        or _item_identity(row) != item_id or _owner_account(row) != wanted_owner):
                    valid = False
                    break
                team = _TEAM_BY_NUMBER.get(team_number)
                if team:
                    teams.add(team)
            if valid:
                return ticks, ("both" if len(teams) > 1 else next(iter(teams), "unknown"))
    return None


def _all_owned_item_windows(
    player_items: list[dict[str, Any]],
    blocks: list[tuple[int, ...]],
    keyed: dict[tuple[str, int], dict[str, Any]],
    xuid: str,
) -> tuple[dict[str, tuple[int, ...]], dict[str, str]]:
    windows: dict[str, tuple[int, ...]] = {}
    teams: dict[str, str] = {}
    for item in player_items:
        definition = item["definition"]
        if definition not in WEAPON_NAMES:
            continue
        identity = _verified_window(blocks, keyed, xuid=xuid, item_id=item["item_id"], definition=definition)
        if identity:
            windows[item["slot_id"]], teams[item["slot_id"]] = identity
    return windows, teams


def _glove_identity_matches(path: Path, xuid: str, item_id: str, definition: int, tick: int) -> bool:
    """Require exact live Pawn glove identity, corroborated by parse_skins."""
    try:
        from .glove_inspection import resolve_owned_glove
        actual_id, actual_definition = resolve_owned_glove(path, xuid, tick)
    except (InspectionError, OSError, ValueError, TypeError):
        return False
    return actual_id == item_id and actual_definition == definition


def _observed_equipment(source_hash: str, owned: dict[str, list[dict[str, Any]]],
                        keyed: dict[tuple[str, int], dict[str, Any]]) -> list[AppearanceItem]:
    """Expose actual held equipment without inventing inventory ownership.

    parse_skins does not enumerate every player's weapons in every Demo. Tick
    observations are display evidence. A separate continuous owned-window
    qualification is required before any observation permits a rewrite.
    """
    inventory_ids = {xuid: {item["item_id"] for item in items} for xuid, items in owned.items()}
    observed: dict[tuple[str, str, int, int], dict[str, Any]] = {}
    counts: dict[str, int] = {}
    for (xuid, _tick), row in keyed.items():
        if xuid not in owned:
            continue
        name = row.get("active_weapon_name")
        if not isinstance(name, str) or not name.strip() or len(name) > 128:
            continue
        try:
            definition = _exact_int(row.get("item_def_idx"), "observed definition", minimum=1, maximum=100_000)
            health = _exact_int(row.get("health"), "observed health", minimum=1, maximum=1000)
            team_num = _exact_int(row.get("team_num"), "observed team", maximum=16)
            paint = _exact_int(row["weapon_skin_id"], "observed paint", maximum=100_000) if row.get("weapon_skin_id") is not None else 0
        except AppearanceInventoryError:
            continue
        # Grenades, C4 and healthshots are not cosmetic equipment slots.
        if definition in {43, 44, 45, 46, 47, 48, 49, 57}:
            continue
        item_id = _item_identity(row) or "0"
        if item_id in inventory_ids[xuid]:
            continue
        # A real item ID identifies the same weapon even when a tick omits
        # paint. Zero-ID records need the paint as a variant discriminator.
        key = (xuid, item_id, definition, paint if item_id == "0" else -1)
        observed_paint = paint if row.get("weapon_skin_id") is not None else None
        if key not in observed:
            if counts.get(xuid, 0) >= _MAX_OBSERVED_ITEMS_PER_PLAYER:
                raise AppearanceInventoryError("Observed equipment exceeds the per-player inventory limit")
            counts[xuid] = counts.get(xuid, 0) + 1
            label, image = _catalog_name(definition, paint, name.strip())
            observed[key] = {"name": label, "image": image, "teams": set(),
                             "paint": observed_paint, "paint_conflict": False,
                             "wear": _wear(row.get("weapon_float")), "seed": None}
        elif observed_paint is not None and not observed[key]["paint_conflict"]:
            data = observed[key]
            if data["paint"] is None:
                data["paint"] = observed_paint
                data["name"], data["image"] = _catalog_name(definition, observed_paint, name.strip())
            elif data["paint"] != observed_paint:
                # Conflicting observations must not advertise a precise finish.
                data.update(paint=None, paint_conflict=True, wear=None,
                            name=name.strip(), image=None)
        if not observed[key]["paint_conflict"] and observed[key]["seed"] is None:
            try:
                observed[key]["seed"] = _exact_int(row.get("weapon_paint_seed"), "observed seed", maximum=1000)
            except AppearanceInventoryError:
                pass
        team = _TEAM_BY_NUMBER.get(team_num)
        if team:
            observed[key]["teams"].add(team)
    result = []
    for (xuid, item_id, definition, paint), data in observed.items():
        teams = data["teams"]
        slot_id = hashlib.sha256(f"{source_hash}:observed:{xuid}:{item_id}:{definition}:{paint}".encode()).hexdigest()[:32]
        result.append(AppearanceItem(slot_id=slot_id, xuid=xuid,
            team="both" if len(teams) > 1 else next(iter(teams), "unknown"),
            definition=definition, item_id=item_id, name=data["name"], paint_index=data["paint"],
            wear=data["wear"], seed=data["seed"], image_url=data["image"],
            editable=False, preset_ids=[],
            reason="missing_item_identity" if item_id == "0" else "unverified_original_ownership"))
    return result


def _qualify_observed_owned_weapons(items: list[AppearanceItem], blocks: list[tuple[int, ...]],
                                   keyed: dict[tuple[str, int], dict[str, Any]], *,
                                   minimum_ticks: int = _VERIFY_WINDOW_TICKS) -> dict[str, tuple[int, ...]]:
    """Missing parse_skins entries are not missing ownership: require exact proof.

    Only already-supported ordinary guns qualify here. Knives/gloves retain
    their separate inventory/Pawn checks. Foreign pickups and ambiguous paint
    or owner fields never become editable.
    """
    windows = {}
    for item in items:
        if (item.reason != "unverified_original_ownership" or item.item_id == "0"
                or item.paint_index is None or item.definition >= 500):
            continue
        presets = [preset for preset in PRESET_DATA if source_weapon_id(preset) == item.definition]
        if not presets:
            continue
        identity = _verified_window(blocks, keyed, xuid=item.xuid,
                                    item_id=item.item_id, definition=item.definition, minimum_ticks=minimum_ticks)
        if not identity:
            continue
        ticks, team = identity
        # The same exact item must also expose this finish throughout the
        # proof window. A label or a missing paint value cannot grant support.
        try:
            paints = {_exact_int(keyed[(item.xuid, tick)].get("weapon_skin_id"), "paint", maximum=100_000)
                      for tick in ticks}
        except AppearanceInventoryError:
            continue
        if paints != {item.paint_index}:
            continue
        item.editable, item.reason, item.preset_ids, item.team = True, None, presets, team
        windows[item.slot_id] = ticks
    return windows


def _qualify_materialized_usp(items: list[AppearanceItem], blocks: list[tuple[int, ...]],
                             keyed: dict[tuple[str, int], dict[str, Any]]) -> dict[str, tuple[int, ...]]:
    """One separately qualified zero-ID source, never generic missing ownership."""
    windows = {}
    candidates = [item for item in items if item.item_id == "0" and item.definition == 61]
    for item in candidates:
        if (item.team != "ct" or item.paint_index is None
                or sum(other.xuid == item.xuid for other in candidates) != 1):
            continue
        found = None
        for block in blocks:
            for offset in range(len(block) - _VERIFY_WINDOW_TICKS + 1):
                ticks = block[offset:offset + _VERIFY_WINDOW_TICKS]
                valid = True
                for tick in ticks:
                    row = keyed.get((item.xuid, tick))
                    if row is None:
                        valid = False
                        break
                    try:
                        high = _exact_int(row.get("item_id_high"), "source ID high", maximum=0xFFFFFFFF)
                        low = _exact_int(row.get("item_id_low"), "source ID low", maximum=0xFFFFFFFF)
                        health = _exact_int(row.get("health"), "health", maximum=1000)
                        side = _exact_int(row.get("team_num"), "team", maximum=16)
                        definition = _exact_int(row.get("item_def_idx"), "definition", maximum=100_000)
                        paint = _exact_int(row.get("weapon_skin_id"), "paint", maximum=100_000)
                    except AppearanceInventoryError:
                        valid = False
                        break
                    if (high != 0 or low != 0 or health <= 0 or side != 3 or definition != 61
                            or paint != item.paint_index or not weapon_matches(row.get("active_weapon_name"), 61)
                            or _owner_account(row) not in {0, 1, int(item.xuid) - _STEAM_BASE}):
                        valid = False
                        break
                if valid:
                    found = ticks
                    break
            if found:
                break
        if found:
            item.editable = True
            item.reason = None
            item.preset_ids = ["usp-kill-confirmed"]
            windows[item.slot_id] = found
    return windows


def inspect_inventory(path: Path | str, *, settings: AppearanceSettings | None = None,
                      plans: dict[str, list[dict[str, Any]]] | None = None) -> InventorySnapshot:
    source = Path(path).expanduser().resolve(strict=True)
    if source.suffix.lower() != ".dem" or not source.is_file():
        raise AppearanceInventoryError("Select an existing .dem file")
    source_hash = digest(source)
    parser = _parser(source)
    roster_rows = _rows(parser.parse_player_info(), required=("steamid", "name"))
    players_by_xuid: dict[str, AppearancePlayer] = {}
    for row in roster_rows:
        xuid = _xuid(row.get("steamid", row.get("steamid64")))
        name = row.get("name")
        if not xuid or not isinstance(name, str) or not name.strip():
            continue
        players_by_xuid.setdefault(xuid, AppearancePlayer(xuid=xuid, name=name.strip()[:128]))
    if not players_by_xuid:
        raise AppearanceInventoryError("Demo does not expose a valid player roster")

    skins = _rows(parser.parse_skins(), required=("steamid", "item_id", "def_index", "paint_index"))
    owned: dict[str, list[dict[str, Any]]] = {xuid: [] for xuid in players_by_xuid}
    seen_ids: dict[str, tuple[str, int]] = {}
    seen_rows: dict[str, dict[str, Any]] = {}
    for row in skins:
        xuid = _xuid(row.get("steamid", row.get("steamid64")))
        if xuid not in players_by_xuid:
            continue
        item_id = _row_item_id(row)
        definition = _exact_int(row.get("def_index"), "definition", maximum=100_000)
        paint = _exact_int(row.get("paint_index"), "paint index", maximum=100_000)
        identity = (xuid, definition)
        previous = seen_ids.get(item_id)
        if previous is not None:
            if previous != identity:
                raise AppearanceInventoryError("Demo contains an ambiguous item identity")
            # parse_skins can repeat the same unchanged economy record. This
            # is one item, not ambiguous ownership; never merge differing
            # appearance/owner/definition fields under the same identity.
            if seen_rows[item_id] == row:
                continue
            raise AppearanceInventoryError("Demo contains a duplicate owned item identity")
        seen_ids[item_id] = identity
        seen_rows[item_id] = row
        seed_value = row.get("paint_seed")
        try:
            seed = _exact_int(seed_value, "paint seed", maximum=1000) if seed_value is not None else None
        except AppearanceInventoryError:
            seed = None
        label, image_url = _catalog_name(definition, paint)
        slot_id = hashlib.sha256(f"{source_hash}:{xuid}:{item_id}:{definition}".encode()).hexdigest()[:32]
        source_presets = [preset_id for preset_id in PRESET_DATA if source_weapon_id(preset_id) == definition]
        if definition == 5030:
            source_presets.extend(GLOVE_PRESET_DATA)
        owned[xuid].append({
            "slot_id": slot_id,
            "xuid": xuid,
            "definition": definition,
            "item_id": item_id,
            "name": label,
            "paint_index": paint,
            "wear": _wear(row.get("paint_wear")),
            "seed": seed,
            "preset_ids": source_presets,
            "image_url": image_url,
        })

    blocks, keyed = _sample_blocks(parser)
    verified_windows: dict[str, tuple[int, ...]] = {}
    items: list[AppearanceItem] = []
    for xuid, player_items in owned.items():
        player_windows, player_teams = _all_owned_item_windows(player_items, blocks, keyed, xuid)
        verified_windows.update(player_windows)
        observed_507 = any(item["definition"] == 507 and item["slot_id"] in player_windows for item in player_items)
        for item in player_items:
            definition = item["definition"]
            has_window = item["slot_id"] in player_windows
            if definition == 5030:
                knife_slot = next((owned_item["slot_id"] for owned_item in player_items
                                   if owned_item["definition"] == 507 and owned_item["slot_id"] in player_windows), None)
                glove_window = player_windows.get(knife_slot, ()) if knife_slot else ()
                verified_glove = bool(glove_window and _glove_identity_matches(
                    source, xuid, item["item_id"], definition, glove_window[0]))
                supported = bool(item["preset_ids"]) and observed_507 and verified_glove
                reason = (None if supported else
                    "unsupported_source_definition" if not item["preset_ids"] else
                    "qualified_combination_requires_karambit" if not observed_507 else
                    "owned_glove_identity_unverified")
                team = "unknown"
            else:
                supported = bool(item["preset_ids"]) and has_window
                reason = None if supported else ("no_bounded_holding_evidence" if item["preset_ids"] else "unsupported_source_definition")
                team = player_teams.get(item["slot_id"], "unknown")
            items.append(AppearanceItem(
                slot_id=item["slot_id"], xuid=xuid, team=team,
                definition=definition, item_id=item["item_id"], name=item["name"],
                paint_index=item["paint_index"], wear=item["wear"], seed=item["seed"],
                editable=supported, reason=reason, preset_ids=item["preset_ids"], image_url=item["image_url"],
            ))

    items.extend(_observed_equipment(source_hash, owned, keyed))
    verified_windows.update(_qualify_observed_owned_weapons(items, blocks, keyed))
    verified_windows.update(_qualify_materialized_usp(items, blocks, keyed))

    response = AppearanceInventoryResponse(
        demo_sha256=source_hash,
        players=list(players_by_xuid.values()),
        items=items,
        presets=preset_catalog(),
        settings=settings or AppearanceSettings(),
        plans={xuid: [AppearanceReplacement.model_validate(value) for value in selections]
               for xuid, selections in (plans or {}).items()},
    )
    return InventorySnapshot(response=response, verification_ticks=verified_windows)
