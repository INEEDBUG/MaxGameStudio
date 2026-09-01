"""Local VALORANT lab APIs.

System mutations are deliberately narrow: only a driver-validated display mode
can be tested, it is non-persistent, and an automatic rollback timer is always
armed.  This router never injects into or reads the VALORANT process.
"""

from __future__ import annotations

import json
import os
import subprocess
import threading
from pathlib import Path
from typing import Any

from fastapi import APIRouter, HTTPException, Request
from pydantic import BaseModel, Field

from ..env_utils import resolve_config_path
from ..session_auth import request_session_token, session_token_matches
from .codec import parse_crosshair_code, serialize_crosshair_code
from .display import ALL_PRESENT_PHYSICAL_MONITORS_DISABLED, collect_display_status
from .display_session import display_mode_session, enumerate_display_modes, test_display_mode
from .game_user_settings import (
    GameUserSettingsConflictError,
    GameUserSettingsError,
    GameUserSettingsFormatError,
    GameUserSettingsNotFoundError,
    GameUserSettingsProcessRunningError,
    GameUserSettingsTransactionError,
    ValorantGameUserSettingsService,
)
from .presets import list_display_presets
from .ui_codec import UICodecError, code_to_ui_profiles, ui_profiles_to_code


router = APIRouter(prefix="/api/valorant-lab", tags=["valorant-lab"])
_profile_lock = threading.RLock()
_game_user_settings = ValorantGameUserSettingsService(
    os.environ.get("LOCALAPPDATA"),
    backup_root=resolve_config_path().parent / "valorant-game-user-settings-backups",
)


class StretchRequest(BaseModel):
    width: int = Field(ge=320, le=7680)
    height: int = Field(ge=240, le=4320)
    refresh_hz: int | None = Field(default=None, ge=24, le=1000)
    preset: str = Field(default="custom", max_length=100)
    mode: str = Field(default="real-stretched", max_length=50)
    confirmed: bool = False
    timeout_seconds: int = Field(default=20, ge=10, le=60)
    lock_cfg: bool = True


class CrosshairProfilesPayload(BaseModel):
    profiles: dict[str, dict[str, Any]]


class CrosshairCodePayload(BaseModel):
    code: str = Field(min_length=1, max_length=16_384)


def _require_session(request: Request) -> None:
    if not session_token_matches(request_session_token(request)):
        raise HTTPException(status_code=401, detail="desktop session required")


def _profiles_path() -> Path:
    return resolve_config_path().parent / "valorant-crosshair-profiles.json"


def _read_profiles() -> dict[str, Any]:
    path = _profiles_path()
    if not path.is_file():
        return {}
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {}
    return value if isinstance(value, dict) else {}


def _write_profiles(value: dict[str, Any]) -> None:
    path = _profiles_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_suffix(path.suffix + ".tmp")
    try:
        with temp.open("w", encoding="utf-8", newline="\n") as handle:
            json.dump(value, handle, ensure_ascii=False, indent=2)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temp, path)
    except Exception:
        # A failed write must not leave a stale staging file that could be
        # mistaken for the current profile store on the next launch.
        try:
            temp.unlink(missing_ok=True)
        except OSError:
            pass
        raise


def _vendor(gpus: list[dict[str, Any]]) -> str:
    names = " ".join(str(item.get("name") or "") for item in gpus).casefold()
    if "nvidia" in names:
        return "nvidia"
    if "amd" in names or "radeon" in names:
        return "amd"
    if "intel" in names:
        return "intel"
    return "unknown"


def _encode_profiles(profiles: dict[str, dict[str, Any]]) -> str:
    try:
        return ui_profiles_to_code(profiles)
    except (UICodecError, ValueError, TypeError) as exc:
        raise HTTPException(status_code=422, detail=f"invalid crosshair profile: {exc}") from exc


def _decode_code(code: str) -> tuple[str, dict[str, dict[str, Any]]]:
    try:
        normalized = serialize_crosshair_code(
            # Native share codes use strict 0/1 and numeric tokens.  Unknown
            # fields remain forward-compatible, but malformed known fields or
            # duplicate known fields must not be silently persisted/exported.
            parse_crosshair_code(code, strict=True, allow_unknown=True),
        )
        return normalized, code_to_ui_profiles(normalized)
    except (ValueError, TypeError) as exc:
        raise HTTPException(status_code=422, detail=f"invalid VALORANT crosshair code: {exc}") from exc


def _frontend_display_status() -> dict[str, Any]:
    recovery = display_mode_session.recover_if_needed()
    snapshot = collect_display_status()
    modes = enumerate_display_modes()
    ready = bool(
        snapshot.get("safe_to_skip_disable")
        and snapshot.get("gpu", {}).get("status") == "ready"
        and snapshot.get("resolution", {}).get("status") == "ready"
        and snapshot.get("refresh_rate", {}).get("status") == "ready"
    )
    snapshot["raw_monitor_status"] = snapshot.get("monitor_disable_status")
    snapshot["overall"] = "ready" if ready else ("warning" if snapshot.get("errors") == [] else "unknown")
    snapshot["gpu"]["vendor"] = _vendor(snapshot.get("gpus", []))
    snapshot["display_modes"] = modes.get("modes", [])
    snapshot["display_mode_session"] = display_mode_session.status()
    snapshot["display_recovery"] = recovery
    snapshot["monitor"]["status"] = "ready" if snapshot.get("safe_to_skip_disable") else snapshot["monitor"].get("status", "unknown")
    snapshot["cfg_status"] = _cfg_status()
    return snapshot


def _current_resolution(values: dict[str, Any]) -> dict[str, int] | None:
    resolution = values.get("resolution") if isinstance(values.get("resolution"), dict) else {}
    pairs = (
        ("ResolutionSizeX", "ResolutionSizeY"),
        ("LastUserConfirmedResolutionSizeX", "LastUserConfirmedResolutionSizeY"),
        ("DesiredScreenWidth", "DesiredScreenHeight"),
        ("LastUserConfirmedDesiredScreenWidth", "LastUserConfirmedDesiredScreenHeight"),
    )
    for x_key, y_key in pairs:
        try:
            return {"width": int(resolution[x_key]), "height": int(resolution[y_key])}
        except (KeyError, TypeError, ValueError):
            continue
    return None


def _cfg_status(width: int | None = None, height: int | None = None) -> dict[str, Any]:
    try:
        status = _game_user_settings.status()
    except GameUserSettingsNotFoundError:
        return {
            "found": False,
            "state": "not_found",
            "current_resolution": None,
            "can_unlock": False,
            "can_restore": False,
        }
    except GameUserSettingsError as exc:
        return {
            "found": False,
            "state": "error",
            "error": str(exc),
            "current_resolution": None,
            "can_unlock": False,
            "can_restore": False,
        }
    if status.get("found"):
        try:
            status.update(_game_user_settings.get())
        except GameUserSettingsError as exc:
            status.update(state="error", error=str(exc))
    current = _current_resolution(status)
    status["current_resolution"] = current
    status["can_unlock"] = status.get("readonly") is True
    status["can_restore"] = status.get("backup_available") is True
    if width is not None and height is not None and isinstance(status.get("resolution"), dict):
        resolution = status["resolution"]
        x_values = [int(value) for key, value in resolution.items() if key in {
            "ResolutionSizeX", "LastUserConfirmedResolutionSizeX", "DesiredScreenWidth", "LastUserConfirmedDesiredScreenWidth"
        } and str(value).strip().isdigit()]
        y_values = [int(value) for key, value in resolution.items() if key in {
            "ResolutionSizeY", "LastUserConfirmedResolutionSizeY", "DesiredScreenHeight", "LastUserConfirmedDesiredScreenHeight"
        } and str(value).strip().isdigit()]
        status["in_sync"] = bool(x_values or y_values) and all(value == width for value in x_values) and all(value == height for value in y_values)
        if not status["in_sync"]:
            status["state"] = "out_of_sync"
        elif status.get("readonly"):
            status["state"] = "locked"
        else:
            status["state"] = "synced"
    elif status.get("drifted") or status.get("recreated"):
        status["state"] = "out_of_sync"
    return status


def _cfg_http_error(exc: GameUserSettingsError) -> HTTPException:
    if isinstance(exc, GameUserSettingsProcessRunningError):
        return HTTPException(
            status_code=409,
            detail={"code": "valorant_running", "message": "检测到 VALORANT 正在运行，请关闭游戏后再应用真拉伸配置。"},
        )
    if isinstance(exc, GameUserSettingsNotFoundError):
        return HTTPException(status_code=409, detail={"code": "cfg_not_found", "message": "未找到有效的 GameUserSettings.ini。"})
    if isinstance(exc, (GameUserSettingsFormatError, GameUserSettingsConflictError)):
        return HTTPException(status_code=409, detail={"code": "cfg_changed_or_invalid", "message": str(exc)})
    status_code = 500 if isinstance(exc, GameUserSettingsTransactionError) else 409
    return HTTPException(status_code=status_code, detail={"code": "cfg_operation_failed", "message": str(exc)})


@router.get("/display/status")
async def display_status(request: Request):
    _require_session(request)
    return _frontend_display_status()


@router.get("/presets")
async def display_presets(request: Request):
    _require_session(request)
    return list_display_presets()


@router.post("/stretch/prepare")
async def prepare_stretch(body: StretchRequest, request: Request):
    _require_session(request)
    snapshot = _frontend_display_status()
    if snapshot.get("raw_monitor_status") != ALL_PRESENT_PHYSICAL_MONITORS_DISABLED:
        raise HTTPException(
            status_code=409,
            detail={
                "code": "monitor_prerequisite_not_satisfied",
                "status": snapshot.get("raw_monitor_status"),
            },
        )
    refresh = body.refresh_hz or snapshot.get("refresh_rate", {}).get("value")
    result = test_display_mode(body.width, body.height, refresh)
    if not result.get("accepted"):
        raise HTTPException(status_code=409, detail={"code": "display_mode_rejected", **result})
    return {
        "prepared": True,
        "preset": body.preset,
        "display_test": result,
        "monitor_step": "skipped_already_disabled",
        "safe_to_apply": True,
        "persistent": False,
        "warning": "The switch is temporary and will auto-restore unless confirmed.",
    }


@router.post("/stretch/apply")
async def apply_stretch(body: StretchRequest, request: Request):
    _require_session(request)
    if not body.confirmed:
        raise HTTPException(status_code=400, detail="explicit confirmation required")
    snapshot = _frontend_display_status()
    if snapshot.get("raw_monitor_status") != ALL_PRESENT_PHYSICAL_MONITORS_DISABLED:
        raise HTTPException(status_code=409, detail="monitor prerequisite changed; detect again")
    refresh = body.refresh_hz or snapshot.get("refresh_rate", {}).get("value")
    cfg_changed = False
    try:
        _game_user_settings.set_resolution(body.width, body.height)
        cfg_changed = True
        if body.lock_cfg:
            _game_user_settings.lock()
        else:
            _game_user_settings.unlock()
        result = display_mode_session.apply(body.width, body.height, refresh, body.timeout_seconds)
    except GameUserSettingsError as exc:
        if cfg_changed:
            try:
                _game_user_settings.restore_latest_backup()
            except GameUserSettingsError as rollback_exc:
                raise HTTPException(
                    status_code=500,
                    detail={"code": "cfg_rollback_failed", "message": f"{exc}; rollback failed: {rollback_exc}"},
                ) from exc
        raise _cfg_http_error(exc) from exc
    except RuntimeError as exc:
        if cfg_changed:
            try:
                _game_user_settings.restore_latest_backup()
            except GameUserSettingsError as rollback_exc:
                raise HTTPException(
                    status_code=500,
                    detail={"code": "cfg_rollback_failed", "message": f"{exc}; rollback failed: {rollback_exc}"},
                ) from exc
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    if not result.get("applied"):
        if cfg_changed:
            try:
                _game_user_settings.restore_latest_backup()
            except GameUserSettingsError as rollback_exc:
                raise HTTPException(status_code=500, detail={"code": "cfg_rollback_failed", "message": str(rollback_exc)}) from rollback_exc
        raise HTTPException(status_code=409, detail=result)
    result["cfg_status"] = _cfg_status(body.width, body.height)
    return result


@router.post("/stretch/confirm")
async def confirm_stretch(request: Request):
    _require_session(request)
    return display_mode_session.confirm()


@router.post("/stretch/restore")
async def restore_stretch(request: Request):
    _require_session(request)
    return display_mode_session.restore()


@router.post("/stretch/cfg/unlock")
async def unlock_stretch_cfg(request: Request):
    _require_session(request)
    try:
        _game_user_settings.unlock()
    except GameUserSettingsError as exc:
        raise _cfg_http_error(exc) from exc
    return {"cfg_status": _cfg_status()}


@router.post("/stretch/cfg/restore")
async def restore_stretch_cfg(request: Request):
    _require_session(request)
    try:
        restored = _game_user_settings.restore_latest_backup()
    except GameUserSettingsError as exc:
        raise _cfg_http_error(exc) from exc
    return {"restored": True, "backup_path": restored.get("backup_path"), "cfg_status": _cfg_status()}


@router.post("/display/open-device-manager")
async def open_device_manager(request: Request):
    _require_session(request)
    if os.name != "nt":
        raise HTTPException(status_code=501, detail="windows only")
    try:
        subprocess.Popen(
            ["mmc.exe", "devmgmt.msc"],
            shell=False,
            close_fds=True,
        )
    except OSError as exc:
        raise HTTPException(status_code=500, detail=f"unable to open Device Manager: {exc}") from exc
    return {"opened": True}


@router.get("/crosshair")
async def get_crosshair_profiles(request: Request):
    _require_session(request)
    with _profile_lock:
        stored = _read_profiles()
    profiles = stored.get("profiles") if isinstance(stored.get("profiles"), dict) else {}
    result: dict[str, Any] = {"profiles": profiles}
    if profiles:
        result["code"] = _encode_profiles(profiles)
    return result


@router.put("/crosshair")
async def save_crosshair_profiles(body: CrosshairProfilesPayload, request: Request):
    _require_session(request)
    code = _encode_profiles(body.profiles)
    payload = {"profiles": body.profiles, "code": code, "format": "valorant-native-v0"}
    with _profile_lock:
        _write_profiles(payload)
    return {**payload, "saved": True}


@router.post("/crosshair/encode")
async def encode_crosshair(body: CrosshairProfilesPayload, request: Request):
    _require_session(request)
    return {"code": _encode_profiles(body.profiles), "format": "valorant-native-v0"}


@router.post("/crosshair/decode")
async def decode_crosshair(body: CrosshairCodePayload, request: Request):
    _require_session(request)
    normalized, profiles = _decode_code(body.code)
    return {
        "code": normalized,
        "profiles": profiles,
        "format": "valorant-native-v0",
    }
