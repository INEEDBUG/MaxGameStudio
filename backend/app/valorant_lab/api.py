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
from .presets import list_display_presets
from .ui_codec import UICodecError, code_to_ui_profiles, ui_profiles_to_code


router = APIRouter(prefix="/api/valorant-lab", tags=["valorant-lab"])
_profile_lock = threading.RLock()


class StretchRequest(BaseModel):
    width: int = Field(ge=320, le=7680)
    height: int = Field(ge=240, le=4320)
    refresh_hz: int | None = Field(default=None, ge=24, le=1000)
    preset: str = Field(default="custom", max_length=100)
    mode: str = Field(default="real-stretched", max_length=50)
    confirmed: bool = False
    timeout_seconds: int = Field(default=20, ge=10, le=60)


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
    return snapshot


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
    try:
        result = display_mode_session.apply(body.width, body.height, refresh, body.timeout_seconds)
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    if not result.get("applied"):
        raise HTTPException(status_code=409, detail=result)
    return result


@router.post("/stretch/confirm")
async def confirm_stretch(request: Request):
    _require_session(request)
    return display_mode_session.confirm()


@router.post("/stretch/restore")
async def restore_stretch(request: Request):
    _require_session(request)
    return display_mode_session.restore()


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
