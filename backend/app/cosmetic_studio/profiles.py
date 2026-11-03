"""Persistent cosmetic preset bindings; this module never starts a runtime."""

from __future__ import annotations

import json
import math
import os
import re
import tempfile
import threading
import uuid
from pathlib import Path
from typing import Any

from .catalog import PRESET_DATA, WEAR_LIMITS, GLOVE_SUPPORTED_WEAPONS

MAX_PROFILES = 50
MAX_FILE_BYTES = 1024 * 1024
_PROFILE_FIELDS = frozenset(
    {
        "id",
        "name",
        "xuid",
        "demo_sha256",
        "start_tick",
        "end_tick",
        "preset_id",
        "pattern_seed",
        "wear",
        "glove_preset_id",
    }
)
_CREATE_FIELDS = _PROFILE_FIELDS - {"id"}
_XUID_RE = re.compile(r"[1-9][0-9]{0,19}\Z")
_SHA256_RE = re.compile(r"[0-9a-fA-F]{64}\Z")
_ID_RE = re.compile(r"[0-9a-f]{32}\Z")


class ProfileStoreError(RuntimeError):
    """Profile persistence failed or the stored document is invalid."""


class ProfileValidationError(ValueError):
    """A profile has invalid fields or a mismatched binding."""


def default_profiles_path() -> Path:
    """Resolve the user-data path only when the store is constructed."""
    from ..env_utils import resolve_config_path

    return resolve_config_path().parent / "cosmetic-studio-profiles.json"


def _validate_profile(value: Any, *, persisted: bool = False) -> dict[str, Any]:
    allowed = _PROFILE_FIELDS if persisted else _CREATE_FIELDS
    if not isinstance(value, dict):
        raise ProfileValidationError("profile must be an object")
    extra = set(value) - allowed
    missing = allowed - set(value)
    optional = {"demo_sha256", "start_tick", "end_tick", "pattern_seed", "wear", "glove_preset_id"}
    missing -= optional
    if extra:
        raise ProfileValidationError("unknown profile fields are forbidden")
    if missing:
        raise ProfileValidationError("required profile fields are missing")

    name = value.get("name")
    if not isinstance(name, str) or not name.strip() or len(name.strip()) > 80:
        raise ProfileValidationError("name must contain 1 to 80 characters")

    xuid = value.get("xuid")
    if not isinstance(xuid, str) or not _XUID_RE.fullmatch(xuid) or int(xuid) > 2**64 - 1:
        raise ProfileValidationError("xuid must be an exact unsigned 64-bit decimal string")

    preset_id = value.get("preset_id")
    if not isinstance(preset_id, str) or preset_id not in PRESET_DATA:
        raise ProfileValidationError("preset_id is not present in the cosmetic catalog")
    glove_preset_id = value.get('glove_preset_id')
    if glove_preset_id not in (None, 'specialist-10061'):
        raise ProfileValidationError('glove preset is not qualified')
    if glove_preset_id and PRESET_DATA[preset_id][0] not in GLOVE_SUPPORTED_WEAPONS:
        raise ProfileValidationError('weapon/glove combination is not qualified')

    demo_sha256 = value.get("demo_sha256")
    if demo_sha256 is not None:
        if not isinstance(demo_sha256, str) or not _SHA256_RE.fullmatch(demo_sha256):
            raise ProfileValidationError("demo_sha256 must be a 64-character hexadecimal digest")
        demo_sha256 = demo_sha256.lower()

    start_tick = value.get("start_tick")
    end_tick = value.get("end_tick")
    if (start_tick is None) != (end_tick is None):
        raise ProfileValidationError("start_tick and end_tick must be supplied together")
    if start_tick is not None:
        if (
            isinstance(start_tick, bool)
            or not isinstance(start_tick, int)
            or isinstance(end_tick, bool)
            or not isinstance(end_tick, int)
            or start_tick < 0
            or end_tick <= start_tick
        ):
            raise ProfileValidationError("tick binding must be an increasing integer interval")

    pattern_seed = value.get("pattern_seed", 0)
    if isinstance(pattern_seed, bool) or not isinstance(pattern_seed, int) or not 0 <= pattern_seed <= 1000:
        raise ProfileValidationError("pattern_seed must be an integer from 0 through 1000")

    definition, paint, _ = PRESET_DATA[preset_id]
    low, high = WEAR_LIMITS.get((definition, paint), (0.0, 1.0))
    wear = value.get("wear", min(high, max(low, 0.01)))
    if isinstance(wear, bool) or not isinstance(wear, (int, float)) or not math.isfinite(wear) or not 0 <= wear <= 1:
        raise ProfileValidationError("wear must be finite and between 0 and 1")
    if not low <= wear <= high:
        raise ProfileValidationError('wear is outside the qualified finish range')

    profile_id = value.get("id") if persisted else uuid.uuid4().hex
    if not isinstance(profile_id, str) or not _ID_RE.fullmatch(profile_id):
        raise ProfileValidationError("id must be a lowercase UUID hex string")

    return {
        "id": profile_id,
        "name": name.strip(),
        "xuid": xuid,
        "demo_sha256": demo_sha256,
        "start_tick": start_tick,
        "end_tick": end_tick,
        "preset_id": preset_id,
        "pattern_seed": pattern_seed,
        "wear": float(wear),
        "glove_preset_id": glove_preset_id,
    }


class ProfileStore:
    """Small atomic JSON store with strict validation and bounded storage."""

    def __init__(self, path: str | Path | None = None):
        self.path = Path(path) if path is not None else default_profiles_path()
        self._lock = threading.RLock()

    def list(self) -> list[dict[str, Any]]:
        with self._lock:
            return self._load()

    def create(self, profile: dict[str, Any]) -> dict[str, Any]:
        candidate = _validate_profile(profile)
        with self._lock:
            profiles = self._load()
            if len(profiles) >= MAX_PROFILES:
                raise ProfileStoreError(f"maximum of {MAX_PROFILES} profiles reached")
            if any(item["id"] == candidate["id"] for item in profiles):
                raise ProfileStoreError("profile ID collision")
            profiles.append(candidate)
            self._save(profiles)
        return dict(candidate)

    def delete(self, profile_id: str) -> bool:
        if not isinstance(profile_id, str) or not _ID_RE.fullmatch(profile_id):
            raise ProfileValidationError("profile id must be a lowercase UUID hex string")
        with self._lock:
            profiles = self._load()
            remaining = [item for item in profiles if item["id"] != profile_id]
            if len(remaining) == len(profiles):
                return False
            self._save(remaining)
            return True

    def _load(self) -> list[dict[str, Any]]:
        try:
            size = self.path.stat().st_size
        except FileNotFoundError:
            return []
        except OSError as exc:
            raise ProfileStoreError("unable to inspect profile file") from exc
        if size > MAX_FILE_BYTES:
            raise ProfileStoreError("profile file exceeds size limit")
        try:
            raw = self.path.read_bytes()
            if len(raw) > MAX_FILE_BYTES:
                raise ProfileStoreError("profile file exceeds size limit")
            document = json.loads(raw.decode("utf-8"))
            if (
                not isinstance(document, dict)
                or set(document) != {"schema_version", "profiles"}
                or document["schema_version"] != 1
                or isinstance(document["schema_version"], bool)
                or not isinstance(document["profiles"], list)
                or len(document["profiles"]) > MAX_PROFILES
            ):
                raise ValueError("invalid profile document shape")
            profiles = [_validate_profile(item, persisted=True) for item in document["profiles"]]
            ids = [item["id"] for item in profiles]
            if len(ids) != len(set(ids)):
                raise ValueError("duplicate profile IDs")
            return profiles
        except ProfileStoreError:
            raise
        except (OSError, UnicodeDecodeError, json.JSONDecodeError, ValueError, TypeError) as exc:
            raise ProfileStoreError("profile file is corrupt or invalid; original was preserved") from exc

    def _save(self, profiles: list[dict[str, Any]]) -> None:
        document = json.dumps(
            {"schema_version": 1, "profiles": profiles},
            ensure_ascii=False,
            separators=(",", ":"),
            allow_nan=False,
        ).encode("utf-8")
        if len(document) > MAX_FILE_BYTES:
            raise ProfileStoreError("profile file exceeds size limit")
        temporary: str | None = None
        try:
            self.path.parent.mkdir(parents=True, exist_ok=True)
            with tempfile.NamedTemporaryFile(
                mode="wb", dir=self.path.parent, prefix=self.path.name + ".", suffix=".tmp", delete=False
            ) as stream:
                temporary = stream.name
                stream.write(document)
                stream.flush()
                os.fsync(stream.fileno())
            os.replace(temporary, self.path)
            temporary = None
        except OSError as exc:
            raise ProfileStoreError("unable to save profiles atomically") from exc
        finally:
            if temporary is not None:
                try:
                    os.unlink(temporary)
                except OSError:
                    pass
