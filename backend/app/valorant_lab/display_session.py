"""Bounded Windows display-mode testing with an automatic rollback timer.

This module only switches to display modes already accepted by the installed
driver.  Creating vendor-specific custom modes remains a separate capability.
"""

from __future__ import annotations

import ctypes
import json
import os
import sys
import threading
import time
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any


ENUM_CURRENT_SETTINGS = -1
CDS_TEST = 0x00000002
DISP_CHANGE_SUCCESSFUL = 0
DM_PELSWIDTH = 0x00080000
DM_PELSHEIGHT = 0x00100000
DM_DISPLAYFREQUENCY = 0x00400000

_RESULT_LABELS = {
    0: "successful",
    1: "restart_required",
    -1: "failed",
    -2: "bad_mode",
    -3: "not_updated",
    -4: "bad_flags",
    -5: "bad_parameter",
    -6: "bad_dual_view",
}


class _POINTL(ctypes.Structure):
    _fields_ = [("x", ctypes.c_long), ("y", ctypes.c_long)]


class _DISPLAY_FIELDS(ctypes.Structure):
    _fields_ = [
        ("dmPosition", _POINTL),
        ("dmDisplayOrientation", ctypes.c_uint32),
        ("dmDisplayFixedOutput", ctypes.c_uint32),
    ]


class _PRINTER_FIELDS(ctypes.Structure):
    _fields_ = [
        ("dmOrientation", ctypes.c_int16),
        ("dmPaperSize", ctypes.c_int16),
        ("dmPaperLength", ctypes.c_int16),
        ("dmPaperWidth", ctypes.c_int16),
        ("dmScale", ctypes.c_int16),
        ("dmCopies", ctypes.c_int16),
        ("dmDefaultSource", ctypes.c_int16),
        ("dmPrintQuality", ctypes.c_int16),
    ]


class _DEVMODE_UNION(ctypes.Union):
    _fields_ = [("display", _DISPLAY_FIELDS), ("printer", _PRINTER_FIELDS)]


class DEVMODEW(ctypes.Structure):
    _anonymous_ = ("u",)
    _fields_ = [
        ("dmDeviceName", ctypes.c_wchar * 32),
        ("dmSpecVersion", ctypes.c_uint16),
        ("dmDriverVersion", ctypes.c_uint16),
        ("dmSize", ctypes.c_uint16),
        ("dmDriverExtra", ctypes.c_uint16),
        ("dmFields", ctypes.c_uint32),
        ("u", _DEVMODE_UNION),
        ("dmColor", ctypes.c_int16),
        ("dmDuplex", ctypes.c_int16),
        ("dmYResolution", ctypes.c_int16),
        ("dmTTOption", ctypes.c_int16),
        ("dmCollate", ctypes.c_int16),
        ("dmFormName", ctypes.c_wchar * 32),
        ("dmLogPixels", ctypes.c_uint16),
        ("dmBitsPerPel", ctypes.c_uint32),
        ("dmPelsWidth", ctypes.c_uint32),
        ("dmPelsHeight", ctypes.c_uint32),
        ("dmDisplayFlags", ctypes.c_uint32),
        ("dmDisplayFrequency", ctypes.c_uint32),
        ("dmICMMethod", ctypes.c_uint32),
        ("dmICMIntent", ctypes.c_uint32),
        ("dmMediaType", ctypes.c_uint32),
        ("dmDitherType", ctypes.c_uint32),
        ("dmReserved1", ctypes.c_uint32),
        ("dmReserved2", ctypes.c_uint32),
        ("dmPanningWidth", ctypes.c_uint32),
        ("dmPanningHeight", ctypes.c_uint32),
    ]


@dataclass(frozen=True)
class DisplayMode:
    width: int
    height: int
    refresh_hz: int
    bits_per_pixel: int


def _unsupported() -> dict[str, Any]:
    return {
        "supported": False,
        "reason": "windows_only",
        "current": None,
        "modes": [],
    }


def _user32():
    if sys.platform != "win32":
        return None
    dll = ctypes.WinDLL("user32", use_last_error=True)
    dll.EnumDisplaySettingsW.argtypes = [ctypes.c_wchar_p, ctypes.c_uint32, ctypes.POINTER(DEVMODEW)]
    dll.EnumDisplaySettingsW.restype = ctypes.c_bool
    dll.ChangeDisplaySettingsExW.argtypes = [
        ctypes.c_wchar_p,
        ctypes.POINTER(DEVMODEW),
        ctypes.c_void_p,
        ctypes.c_uint32,
        ctypes.c_void_p,
    ]
    dll.ChangeDisplaySettingsExW.restype = ctypes.c_long
    return dll


def _read_mode(index: int, device_name: str | None = None) -> DEVMODEW | None:
    dll = _user32()
    if dll is None:
        return None
    mode = DEVMODEW()
    mode.dmSize = ctypes.sizeof(DEVMODEW)
    if not dll.EnumDisplaySettingsW(device_name, index, ctypes.byref(mode)):
        return None
    return mode


def current_display_mode(device_name: str | None = None) -> DisplayMode | None:
    mode = _read_mode(ENUM_CURRENT_SETTINGS, device_name)
    if mode is None:
        return None
    return DisplayMode(
        width=int(mode.dmPelsWidth),
        height=int(mode.dmPelsHeight),
        refresh_hz=int(mode.dmDisplayFrequency),
        bits_per_pixel=int(mode.dmBitsPerPel),
    )


def enumerate_display_modes(device_name: str | None = None) -> dict[str, Any]:
    if sys.platform != "win32":
        return _unsupported()
    unique: dict[tuple[int, int, int], DisplayMode] = {}
    index = 0
    while True:
        raw = _read_mode(index, device_name)
        if raw is None:
            break
        mode = DisplayMode(
            width=int(raw.dmPelsWidth),
            height=int(raw.dmPelsHeight),
            refresh_hz=int(raw.dmDisplayFrequency),
            bits_per_pixel=int(raw.dmBitsPerPel),
        )
        if mode.width > 0 and mode.height > 0:
            unique[(mode.width, mode.height, mode.refresh_hz)] = mode
        index += 1
        if index > 4096:
            break
    current = current_display_mode(device_name)
    return {
        "supported": current is not None,
        "reason": None if current is not None else "display_settings_unavailable",
        "current": asdict(current) if current else None,
        "modes": [
            asdict(mode)
            for mode in sorted(unique.values(), key=lambda item: (item.width, item.height, item.refresh_hz))
        ],
    }


def _requested_devmode(width: int, height: int, refresh_hz: int | None) -> DEVMODEW | None:
    current = _read_mode(ENUM_CURRENT_SETTINGS)
    if current is None:
        return None
    current.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT
    current.dmPelsWidth = int(width)
    current.dmPelsHeight = int(height)
    if refresh_hz:
        current.dmFields |= DM_DISPLAYFREQUENCY
        current.dmDisplayFrequency = int(refresh_hz)
    return current


def _change_mode(mode: DEVMODEW, flags: int) -> int:
    dll = _user32()
    if dll is None:
        return -1
    return int(dll.ChangeDisplaySettingsExW(None, ctypes.byref(mode), None, flags, None))


def test_display_mode(width: int, height: int, refresh_hz: int | None = None) -> dict[str, Any]:
    try:
        mode = _requested_devmode(width, height, refresh_hz)
    except Exception as exc:
        return {
            "supported": False,
            "accepted": False,
            "code": -1,
            "result": "display_query_failed",
            "error": f"{type(exc).__name__}: {exc}",
        }
    if mode is None:
        return {"supported": False, "accepted": False, "code": -1, "result": "windows_only"}
    try:
        code = _change_mode(mode, CDS_TEST)
    except Exception as exc:
        return {
            "supported": True,
            "accepted": False,
            "code": -1,
            "result": "display_test_failed",
            "error": f"{type(exc).__name__}: {exc}",
            "requested": {
                "width": int(width),
                "height": int(height),
                "refresh_hz": int(refresh_hz or mode.dmDisplayFrequency or 0),
            },
        }
    return {
        "supported": True,
        "accepted": code == DISP_CHANGE_SUCCESSFUL,
        "code": code,
        "result": _RESULT_LABELS.get(code, "unknown"),
        "requested": {
            "width": int(width),
            "height": int(height),
            "refresh_hz": int(refresh_hz or mode.dmDisplayFrequency or 0),
        },
    }


class DisplayModeSession:
    """One active, non-persistent display switch with an automatic rollback."""

    def __init__(self, manifest_path: Path | None = None) -> None:
        self._lock = threading.RLock()
        self._timer: threading.Timer | None = None
        self._previous: DisplayMode | None = None
        self._deadline: float | None = None
        self._manifest_path = manifest_path
        self._recovery_pending = bool(manifest_path and manifest_path.is_file())

    def _write_manifest(self, previous: DisplayMode, requested: DisplayMode) -> None:
        path = self._manifest_path
        if path is None:
            return
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary = path.with_suffix(path.suffix + ".tmp")
        payload = {
            "schema_version": 1,
            "previous": asdict(previous),
            "requested": asdict(requested),
            "created_at": time.time(),
        }
        with temporary.open("w", encoding="utf-8", newline="\n") as handle:
            json.dump(payload, handle, ensure_ascii=False, indent=2)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
        self._recovery_pending = True

    def _clear_manifest(self) -> bool:
        path = self._manifest_path
        if path is not None:
            try:
                path.unlink(missing_ok=True)
            except OSError:
                return False
        self._recovery_pending = False
        return True

    def _read_manifest_previous(self) -> DisplayMode | None:
        path = self._manifest_path
        if path is None or not path.is_file():
            return None
        try:
            value = json.loads(path.read_text(encoding="utf-8"))
            previous = value.get("previous") if isinstance(value, dict) else None
            if not isinstance(previous, dict):
                return None
            return DisplayMode(
                width=int(previous["width"]),
                height=int(previous["height"]),
                refresh_hz=int(previous["refresh_hz"]),
                bits_per_pixel=int(previous.get("bits_per_pixel") or 32),
            )
        except (OSError, ValueError, TypeError, KeyError, json.JSONDecodeError):
            return None

    def recover_if_needed(self) -> dict[str, Any]:
        """Restore a mode left behind by an interrupted timed test."""

        with self._lock:
            if self._previous is not None:
                return {"recovered": False, "reason": "active_session"}
            previous = self._read_manifest_previous()
            if previous is None:
                if self._manifest_path and self._manifest_path.is_file():
                    return {"recovered": False, "reason": "invalid_recovery_manifest"}
                self._recovery_pending = False
                return {"recovered": False, "reason": "no_recovery_manifest"}
            current = current_display_mode()
            if current == previous:
                manifest_cleared = self._clear_manifest()
                return {
                    "recovered": manifest_cleared,
                    "reason": "already_restored" if manifest_cleared else "recovery_manifest_cleanup_failed",
                    "target": asdict(previous),
                }
            try:
                mode = _requested_devmode(previous.width, previous.height, previous.refresh_hz)
                code = _change_mode(mode, 0) if mode is not None else -1
                recovery_error = None
            except Exception as exc:
                code = -1
                recovery_error = f"{type(exc).__name__}: {exc}"
            if code == DISP_CHANGE_SUCCESSFUL:
                self._clear_manifest()
            return {
                "recovered": code == DISP_CHANGE_SUCCESSFUL,
                "reason": _RESULT_LABELS.get(code, "unknown"),
                "code": code,
                "target": asdict(previous),
                "error": recovery_error,
            }

    def status(self) -> dict[str, Any]:
        with self._lock:
            return {
                "active": self._previous is not None,
                "previous": asdict(self._previous) if self._previous else None,
                "rollback_deadline": self._deadline,
                "remaining_seconds": max(0.0, self._deadline - time.time()) if self._deadline else None,
                "recovery_pending": self._recovery_pending,
            }

    def apply(self, width: int, height: int, refresh_hz: int | None, timeout_seconds: int) -> dict[str, Any]:
        timeout = max(10, min(int(timeout_seconds), 60))
        with self._lock:
            if self._previous is not None:
                raise RuntimeError("display_test_already_active")
            tested = test_display_mode(width, height, refresh_hz)
            if not tested.get("accepted"):
                return {**tested, "applied": False}
            try:
                previous = current_display_mode()
                mode = _requested_devmode(width, height, refresh_hz)
            except Exception as exc:
                return {
                    **tested,
                    "applied": False,
                    "result": "current_mode_unavailable",
                    "error": f"{type(exc).__name__}: {exc}",
                }
            if previous is None or mode is None:
                return {**tested, "applied": False, "result": "current_mode_unavailable"}
            requested = DisplayMode(
                width=int(width),
                height=int(height),
                refresh_hz=int(refresh_hz or mode.dmDisplayFrequency or previous.refresh_hz),
                bits_per_pixel=int(mode.dmBitsPerPel or previous.bits_per_pixel),
            )
            try:
                self._write_manifest(previous, requested)
            except OSError as exc:
                return {
                    **tested,
                    "applied": False,
                    "result": "recovery_manifest_unavailable",
                    "error": str(exc),
                }
            try:
                code = _change_mode(mode, 0)
            except Exception as exc:
                # Keep the manifest: a native call can fail after changing
                # part of the display state, so startup recovery must remain
                # possible even when the API call itself raises.
                return {
                    **tested,
                    "applied": False,
                    "code": -1,
                    "result": "display_change_failed",
                    "error": f"{type(exc).__name__}: {exc}",
                }
            if code != DISP_CHANGE_SUCCESSFUL:
                self._clear_manifest()
                return {
                    **tested,
                    "applied": False,
                    "code": code,
                    "result": _RESULT_LABELS.get(code, "unknown"),
                }
            try:
                applied = current_display_mode()
            except Exception as exc:
                applied = None
                readback_error = f"{type(exc).__name__}: {exc}"
            else:
                readback_error = None
            if (
                applied is None
                or applied.width != int(width)
                or applied.height != int(height)
                or (refresh_hz and applied.refresh_hz != int(refresh_hz))
            ):
                try:
                    rollback_mode = _requested_devmode(
                        previous.width,
                        previous.height,
                        previous.refresh_hz,
                    )
                    rollback_code = _change_mode(rollback_mode, 0) if rollback_mode is not None else -1
                except Exception as exc:
                    rollback_code = -1
                    rollback_error = f"{type(exc).__name__}: {exc}"
                else:
                    rollback_error = None
                if rollback_code == DISP_CHANGE_SUCCESSFUL:
                    self._clear_manifest()
                return {
                    **tested,
                    "applied": False,
                    "result": "display_mode_readback_mismatch",
                    "observed": asdict(applied) if applied else None,
                    "rollback_code": rollback_code,
                    "rollback_result": _RESULT_LABELS.get(rollback_code, "unknown"),
                    "readback_error": readback_error,
                    "rollback_error": rollback_error,
                }
            self._previous = previous
            self._deadline = time.time() + timeout
            try:
                timer = threading.Timer(timeout, self.restore)
                timer.daemon = True
                self._timer = timer
                timer.start()
            except Exception as exc:
                # No unguarded active mode may be left without a rollback
                # timer.  Attempt the same restore path used by the timer;
                # if that fails the manifest and active state remain retryable.
                rollback = self.restore()
                return {
                    **tested,
                    "applied": False,
                    "result": "rollback_after_timer_failure",
                    "error": f"{type(exc).__name__}: {exc}",
                    "rollback": rollback,
                }
            return {
                **tested,
                "applied": True,
                "previous": asdict(previous),
                "rollback_deadline": self._deadline,
                "timeout_seconds": timeout,
                "persistent": False,
            }

    def confirm(self) -> dict[str, Any]:
        with self._lock:
            if self._previous is None:
                return {"confirmed": False, "reason": "no_active_display_test"}
            if not self._clear_manifest():
                return {"confirmed": False, "reason": "recovery_manifest_cleanup_failed"}
            if self._timer:
                self._timer.cancel()
            try:
                current = current_display_mode()
            except Exception as exc:
                current = None
                current_error = f"{type(exc).__name__}: {exc}"
            else:
                current_error = None
            self._timer = None
            self._previous = None
            self._deadline = None
            return {
                "confirmed": True,
                "current": asdict(current) if current else None,
                "current_error": current_error,
                "persistent": False,
            }

    def restore(self) -> dict[str, Any]:
        with self._lock:
            previous = self._previous
            if previous is None:
                return {"restored": False, "reason": "no_active_display_test"}
            try:
                mode = _requested_devmode(previous.width, previous.height, previous.refresh_hz)
                code = _change_mode(mode, 0) if mode is not None else -1
                restore_error = None
            except Exception as exc:
                code = -1
                restore_error = f"{type(exc).__name__}: {exc}"
            if self._timer:
                self._timer.cancel()
            self._timer = None
            self._deadline = None
            if code == DISP_CHANGE_SUCCESSFUL:
                self._previous = None
                self._clear_manifest()
            return {
                "restored": code == DISP_CHANGE_SUCCESSFUL,
                "code": code,
                "result": _RESULT_LABELS.get(code, "unknown"),
                "target": asdict(previous),
                "retry_available": code != DISP_CHANGE_SUCCESSFUL,
                "error": restore_error,
            }


try:
    from ..env_utils import resolve_config_path

    _DEFAULT_MANIFEST = resolve_config_path().parent / "valorant-display-recovery.json"
except Exception:
    _DEFAULT_MANIFEST = None

display_mode_session = DisplayModeSession(_DEFAULT_MANIFEST)
