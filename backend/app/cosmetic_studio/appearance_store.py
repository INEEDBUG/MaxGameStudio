"""Small, atomic local settings and Demo/player plan store."""

from __future__ import annotations

import json
import os
from pathlib import Path
import re
import sys
import tempfile
import threading
from typing import Any

from .appearance_models import AppearanceReplacement, AppearanceSettings
from .runtime import digest
from .skin_core import QUALIFIED_CORE_SHA256


_MAX_STORE_BYTES = 2 * 1024 * 1024
_MAX_RETAINED_JOBS = 20
_EMPTY: dict[str, Any] = {
    "schema_version": 1,
    "settings": {"core_exe": "", "output_dir": ""},
    "plans": {},
    "artifacts": {},
    "jobs": {},
}


class AppearanceStoreError(RuntimeError):
    pass


def default_store_path() -> Path:
    from ..env_utils import resolve_config_path
    return resolve_config_path().parent / "cosmetic-appearance.json"


def default_output_dir() -> Path:
    from ..env_utils import get_data_dir
    return get_data_dir() / "cosmetic_playbacks"


def _verified_core(path: Path) -> bool:
    try:
        return path.is_file() and not path.is_symlink() and digest(path) == QUALIFIED_CORE_SHA256
    except OSError:
        return False


def discovered_core_path() -> str:
    candidates = [Path(sys.executable).resolve().parent.parent / "tools" / "skin-core.exe"]
    if os.name == "nt":
        candidates.append(Path(r"E:\Tools\CS2 Insight Agent\tools\skin-core.exe"))
    for path in candidates:
        if _verified_core(path):
            return str(path.resolve())
    return ""


def default_settings() -> AppearanceSettings:
    return AppearanceSettings(core_exe=discovered_core_path(), output_dir=str(default_output_dir()))


def _validate_xuid(value: str) -> str:
    if not isinstance(value, str) or not re.fullmatch(r"[1-9][0-9]{0,19}", value) or int(value) > (1 << 64) - 1:
        raise AppearanceStoreError("Invalid player binding in appearance plans")
    return value


class AppearanceStore:
    def __init__(self, path: Path | str | None = None):
        self.path = Path(path) if path is not None else default_store_path()
        self._lock = threading.RLock()

    def _empty_state(self) -> dict[str, Any]:
        state = json.loads(json.dumps(_EMPTY))
        # Defaults are materialized only when a store is first read/written.
        # This prevents the first pending-job record from committing blank
        # settings before the worker resolves discovered core/output paths.
        state["settings"] = default_settings().model_dump()
        return state

    def _read_unlocked(self) -> dict[str, Any]:
        if not self.path.exists():
            return self._empty_state()
        if self.path.is_symlink() or not self.path.is_file():
            raise AppearanceStoreError("Appearance settings file is not a regular file")
        try:
            if self.path.stat().st_size > _MAX_STORE_BYTES:
                raise AppearanceStoreError("Appearance settings file exceeds the size limit")
            state = json.loads(self.path.read_text(encoding="utf-8"))
            if not isinstance(state, dict) or set(state) != set(_EMPTY) or state.get("schema_version") != 1:
                raise ValueError("unexpected schema")
            AppearanceSettings.model_validate(state["settings"])
            plans = state["plans"]
            if not isinstance(plans, dict) or len(plans) > 256:
                raise ValueError("invalid plans")
            clean_plans: dict[str, dict[str, list[dict[str, Any]]]] = {}
            for source_hash, player_plans in plans.items():
                if not isinstance(source_hash, str) or not re.fullmatch(r"[a-f0-9]{64}", source_hash) or not isinstance(player_plans, dict):
                    raise ValueError("invalid source plan binding")
                clean_plans[source_hash] = {}
                for xuid, selections in player_plans.items():
                    _validate_xuid(xuid)
                    if not isinstance(selections, list) or len(selections) > 128:
                        raise ValueError("invalid plan selections")
                    clean_plans[source_hash][xuid] = [AppearanceReplacement.model_validate(row).model_dump() for row in selections]
            if not isinstance(state["artifacts"], dict) or not isinstance(state["jobs"], dict):
                raise ValueError("invalid artifact state")
            for source_hash, artifact in state["artifacts"].items():
                if not re.fullmatch(r"[a-f0-9]{64}", str(source_hash)) or not isinstance(artifact, dict):
                    raise ValueError("invalid artifact binding")
            for job_id, job in state["jobs"].items():
                if not re.fullmatch(r"[a-f0-9]{32}", str(job_id)) or not isinstance(job, dict):
                    raise ValueError("invalid job record")
            state["plans"] = clean_plans
            return state
        except AppearanceStoreError:
            raise
        except (OSError, ValueError, TypeError, KeyError) as exc:
            raise AppearanceStoreError("Appearance settings file is corrupt; it was preserved") from exc

    def _write_unlocked(self, state: dict[str, Any]) -> None:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        if self.path.is_symlink():
            raise AppearanceStoreError("Appearance settings file may not be a symlink")
        encoded = json.dumps(state, ensure_ascii=False, separators=(",", ":"), allow_nan=False).encode("utf-8")
        if len(encoded) > _MAX_STORE_BYTES:
            raise AppearanceStoreError("Appearance settings exceed the size limit")
        temp_path: Path | None = None
        try:
            with tempfile.NamedTemporaryFile(mode="wb", dir=self.path.parent, prefix=".cosmetic-appearance-", suffix=".tmp", delete=False) as stream:
                temp_path = Path(stream.name)
                stream.write(encoded)
                stream.flush()
                os.fsync(stream.fileno())
            os.replace(temp_path, self.path)
        except OSError as exc:
            if temp_path is not None:
                try:
                    temp_path.unlink(missing_ok=True)
                except OSError:
                    pass
            raise AppearanceStoreError("Could not atomically save appearance settings") from exc

    def load_settings(self) -> AppearanceSettings:
        with self._lock:
            existed = self.path.exists()
            state = self._read_unlocked()
            if not existed:
                return default_settings()
            return AppearanceSettings.model_validate(state["settings"])

    def save_settings(self, settings: dict[str, Any] | AppearanceSettings) -> AppearanceSettings:
        validated = settings if isinstance(settings, AppearanceSettings) else AppearanceSettings.model_validate(settings)
        with self._lock:
            state = self._read_unlocked()
            state["settings"] = validated.model_dump()
            self._write_unlocked(state)
        return validated

    def load_plans(self, demo_sha256: str) -> dict[str, list[dict[str, Any]]]:
        with self._lock:
            return self._read_unlocked()["plans"].get(demo_sha256, {})

    def load_all_plans(self) -> dict[str, dict[str, list[dict[str, Any]]]]:
        with self._lock:
            return self._read_unlocked()["plans"]

    def save_player_plan(self, demo_sha256: str, xuid: str, selections: list[dict[str, Any]]) -> None:
        _validate_xuid(xuid)
        if not re.fullmatch(r"[a-f0-9]{64}", demo_sha256):
            raise AppearanceStoreError("Invalid source Demo fingerprint")
        clean = [AppearanceReplacement.model_validate(row).model_dump() for row in selections]
        if len(clean) > 128:
            raise AppearanceStoreError("A player plan exceeds the qualified item limit")
        with self._lock:
            state = self._read_unlocked()
            source_plans = state["plans"].setdefault(demo_sha256, {})
            if clean:
                source_plans[xuid] = clean
            else:
                source_plans.pop(xuid, None)
                if not source_plans:
                    state["plans"].pop(demo_sha256, None)
            self._write_unlocked(state)

    def load_artifact(self, demo_sha256: str) -> dict[str, Any] | None:
        with self._lock:
            return self._read_unlocked()["artifacts"].get(demo_sha256)

    def load_job(self, job_id: str) -> dict[str, Any] | None:
        with self._lock:
            return self._read_unlocked()["jobs"].get(job_id)

    def save_job(self, job: dict[str, Any]) -> None:
        job_id = job.get("id") if isinstance(job, dict) else None
        if not isinstance(job_id, str) or not re.fullmatch(r"[a-f0-9]{32}", job_id):
            raise AppearanceStoreError("Invalid appearance job identifier")
        if job.get("state") not in {"pending", "running", "file_verified", "failed", "cancelled"}:
            raise AppearanceStoreError("Invalid appearance job state")
        encoded = json.dumps(job, ensure_ascii=False, separators=(",", ":"), allow_nan=False)
        if len(encoded.encode("utf-8")) > 128 * 1024:
            raise AppearanceStoreError("Appearance job record exceeds the size limit")
        with self._lock:
            state = self._read_unlocked()
            state["jobs"][job_id] = json.loads(encoded)
            while len(state["jobs"]) > _MAX_RETAINED_JOBS:
                oldest = next(iter(state["jobs"]))
                del state["jobs"][oldest]
            self._write_unlocked(state)

    def commit_verified_apply(
        self,
        demo_sha256: str,
        plans: dict[str, list[dict[str, Any]]],
        artifact: dict[str, Any],
        job_status: dict[str, Any],
    ) -> None:
        """Commit plans and their verified playback proof in one atomic write."""
        clean_plans = {
            _validate_xuid(xuid): [AppearanceReplacement.model_validate(row).model_dump() for row in rows]
            for xuid, rows in plans.items() if rows
        }
        if len(clean_plans) > 16 or any(len(rows) > 128 for rows in clean_plans.values()):
            raise AppearanceStoreError("Verified plan set exceeds its qualified size limit")
        with self._lock:
            state = self._read_unlocked()
            if clean_plans:
                state["plans"][demo_sha256] = clean_plans
            else:
                state["plans"].pop(demo_sha256, None)
            state["artifacts"][demo_sha256] = artifact
            state["jobs"][job_status["id"]] = job_status
            while len(state["jobs"]) > _MAX_RETAINED_JOBS:
                oldest = next(iter(state["jobs"]))
                del state["jobs"][oldest]
            if len(state["artifacts"]) > _MAX_RETAINED_JOBS:
                oldest = next(iter(state["artifacts"]))
                del state["artifacts"][oldest]
            self._write_unlocked(state)
