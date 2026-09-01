"""Safe VALORANT GameUserSettings.ini discovery and transactions."""

from __future__ import annotations

import ctypes
import hashlib
import json
import os
import re
import stat
import subprocess
import sys
import tempfile
import threading
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable, Mapping


GAME_USER_SETTINGS_FILENAME = "GameUserSettings.ini"
VALORANT_PROCESS_NAME = "VALORANT-Win64-Shipping.exe"
RESOLUTION_KEYS = (
    "ResolutionSizeX", "ResolutionSizeY", "LastUserConfirmedResolutionSizeX",
    "LastUserConfirmedResolutionSizeY", "DesiredScreenWidth", "DesiredScreenHeight",
    "LastUserConfirmedDesiredScreenWidth", "LastUserConfirmedDesiredScreenHeight",
)
RESOLUTION_KEY_SET = frozenset(RESOLUTION_KEYS)
RESOLUTION_X_KEYS = frozenset(RESOLUTION_KEYS[::2])
RESOLUTION_Y_KEYS = frozenset(RESOLUTION_KEYS[1::2])
SUPPORTED_RESOLUTION_KEYS = RESOLUTION_KEYS
_READONLY, _BAD_ATTRS = 1, 0xFFFFFFFF
_BOMS = ((b"\xef\xbb\xbf", "utf-8"), (b"\xff\xfe", "utf-16-le"), (b"\xfe\xff", "utf-16-be"))
_SAFE_NAME = re.compile(r"[^A-Za-z0-9._-]+")
_kernel32_instance = None


class GameUserSettingsError(RuntimeError):
    """Base error for settings discovery and transactions."""


class GameUserSettingsNotFoundError(GameUserSettingsError):
    """No valid settings file or backup is available."""


class GameUserSettingsFormatError(GameUserSettingsError):
    """A settings file or manifest is malformed."""


class GameUserSettingsConflictError(GameUserSettingsError):
    """The selected file changed during detection."""


class GameUserSettingsProcessRunningError(GameUserSettingsError):
    """A mutation was refused while VALORANT is running or uncheckable."""


class GameUserSettingsTransactionError(GameUserSettingsError):
    """A write failed after rollback was attempted."""


@dataclass(frozen=True)
class GameUserSettingsFile:
    path: Path
    profile: str
    windows_dir: str
    mtime: float
    mtime_ns: int
    size: int
    sha256: str
    readonly: bool | None
    encoding: str
    has_bom: bool
    resolution: Mapping[str, str]
    file_id: tuple[int, int, int]

    def to_dict(self) -> dict[str, Any]:
        values = dict(self.resolution)
        return {
            "path": str(self.path), "profile": self.profile, "windows_dir": self.windows_dir,
            "windows_folder": self.windows_dir, "filename": self.path.name,
            "mtime": self.mtime, "mtime_ns": self.mtime_ns, "size": self.size,
            "sha256": self.sha256, "readonly": self.readonly, "locked": self.readonly is True,
            "encoding": self.encoding, "has_bom": self.has_bom, "resolution": values,
            "resolution_keys": [key for key in RESOLUTION_KEYS if key in values],
            "file_id": list(self.file_id),
        }


def _path(value: Path | str) -> Path:
    return Path(value).expanduser()


def _sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _file_id(info: os.stat_result) -> tuple[int, int, int]:
    # POSIX ctime changes on chmod; Windows ctime is the creation timestamp.
    return (int(getattr(info, "st_dev", 0)), int(getattr(info, "st_ino", 0)),
            int(info.st_ctime_ns) if sys.platform == "win32" else 0)


def _timestamp(clock: Callable[[], datetime]) -> str:
    value = clock()
    if value.tzinfo is None:
        value = value.replace(tzinfo=timezone.utc)
    return value.astimezone(timezone.utc).isoformat(timespec="microseconds").replace("+00:00", "Z")


def _decode(data: bytes) -> tuple[str, str, bytes]:
    for bom, codec in _BOMS:
        if data.startswith(bom):
            try:
                return data[len(bom):].decode(codec), codec, bom
            except UnicodeDecodeError as exc:
                raise GameUserSettingsFormatError(f"invalid {codec}: {exc}") from exc
    if len(data) >= 4 and len(data) % 2 == 0:
        odd = sum(value == 0 for value in data[1::2])
        even = sum(value == 0 for value in data[::2])
        threshold = max(2, len(data) // 8)
        for codec, count, other in (("utf-16-le", odd, even), ("utf-16-be", even, odd)):
            if count >= threshold and count > other:
                try:
                    return data.decode(codec), codec, b""
                except UnicodeDecodeError:
                    pass
    try:
        return data.decode("utf-8"), "utf-8", b""
    except UnicodeDecodeError as exc:
        raise GameUserSettingsFormatError(f"unsupported encoding: {exc}") from exc


def _valid_ini(text: str) -> bool:
    if "\x00" in text:
        return False
    return any(
        value and not value.startswith((";", "#")) and ("=" in value or value.startswith("["))
        for value in (line.strip() for line in text.splitlines())
    )


def _line_end(line: str) -> tuple[str, str]:
    if line.endswith("\r\n"):
        return line[:-2], "\r\n"
    return (line[:-1], line[-1]) if line.endswith(("\r", "\n")) else (line, "")


def _comment(value: str) -> int | None:
    positions = [value.find(marker) for marker in (";", "#") if marker in value]
    return min(positions) if positions else None


def _clean(value: str) -> str:
    cut = _comment(value)
    value = value if cut is None else value[:cut]
    value = value.strip()
    return value[1:-1] if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'" else value


def _entries(text: str, width: int | None = None, height: int | None = None) -> tuple[str, dict[str, str], list[str]]:
    output, values, changed = [], {}, []
    for line in text.splitlines(keepends=True):
        body, ending = _line_end(line)
        equal = body.find("=")
        key = body[:equal].strip() if equal > 0 else ""
        if key not in RESOLUTION_KEY_SET or body.lstrip(" \t").startswith((";", "#")):
            output.append(line)
            continue
        raw = body[equal + 1:]
        if width is None or height is None:
            values[key] = _clean(raw)
            output.append(line)
            continue
        cut = _comment(raw)
        value, suffix = (raw, "") if cut is None else (raw[:cut], raw[cut:])
        match = re.match(r"([ \t]*)(.*?)([ \t]*)$", value, re.S)
        assert match is not None
        old = match.group(2)
        replacement = str(width if key in RESOLUTION_X_KEYS else height)
        if len(old) >= 2 and old[0] == old[-1] and old[0] in "\"'":
            replacement = old[0] + replacement + old[-1]
        output.append(body[:equal + 1] + match.group(1) + replacement + match.group(3) + suffix + ending)
        changed.append(key)
    return "".join(output), values, changed


def _kernel32():
    global _kernel32_instance
    if sys.platform != "win32":
        return None
    if _kernel32_instance is None:
        dll = ctypes.WinDLL("kernel32", use_last_error=True)
        dll.GetFileAttributesW.argtypes = [ctypes.c_wchar_p]
        dll.GetFileAttributesW.restype = ctypes.c_uint32
        dll.SetFileAttributesW.argtypes = [ctypes.c_wchar_p, ctypes.c_uint32]
        dll.SetFileAttributesW.restype = ctypes.c_int
        _kernel32_instance = dll
    return _kernel32_instance


def get_file_readonly(path: Path | str) -> bool:
    target, dll = _path(path), _kernel32()
    if dll is not None:
        attrs = int(dll.GetFileAttributesW(str(target)))
        if attrs == _BAD_ATTRS:
            raise OSError(ctypes.get_last_error(), f"cannot read attributes: {target}")
        return bool(attrs & _READONLY)
    return not bool(target.stat().st_mode & stat.S_IWRITE)


def set_file_readonly(path: Path | str, readonly: bool) -> None:
    target, dll = _path(path), _kernel32()
    if dll is not None:
        attrs = int(dll.GetFileAttributesW(str(target)))
        if attrs == _BAD_ATTRS:
            raise OSError(ctypes.get_last_error(), f"cannot read attributes: {target}")
        attrs = attrs | _READONLY if readonly else attrs & ~_READONLY
        if not dll.SetFileAttributesW(str(target), attrs):
            raise OSError(ctypes.get_last_error(), f"cannot set attributes: {target}")
        return
    mode = target.stat().st_mode
    os.chmod(target, mode & ~stat.S_IWRITE if readonly else mode | stat.S_IWRITE)


def _tasklist_process_checker() -> bool:
    if sys.platform != "win32":
        return False
    result = subprocess.run(
        ["tasklist", "/FI", f"IMAGENAME eq {VALORANT_PROCESS_NAME}", "/NH"], shell=False,
        check=False, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, timeout=5,
        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
    )
    expected = VALORANT_PROCESS_NAME.casefold()
    return any((line.strip().split() or [""])[0].casefold() == expected for line in (result.stdout or "").splitlines())


def is_valorant_running(process_checker: Callable[[], bool] | None = None) -> bool:
    try:
        return bool((process_checker or _tasklist_process_checker)())
    except Exception:  # noqa: BLE001
        return False


class ValorantGameUserSettingsService:
    def __init__(
        self, local_app_data: Path | str | None = None, *, config_root: Path | str | None = None,
        backup_root: Path | str | None = None, process_checker: Callable[[], bool] | None = None,
        readonly_getter: Callable[[Path], bool] = get_file_readonly,
        readonly_setter: Callable[[Path, bool], None] = set_file_readonly,
        clock: Callable[[], datetime] | None = None,
    ) -> None:
        local = _path(local_app_data) if local_app_data is not None else None
        self.config_root = _path(config_root) if config_root is not None else (local / "VALORANT" / "Saved" / "Config" if local else None)
        self.backup_root = _path(backup_root) if backup_root is not None else (local or Path.home()) / "MaxGameStudio" / "valorant-game-user-settings-backups"
        self._process_checker, self._get_readonly, self._set_readonly = process_checker, readonly_getter, readonly_setter
        self._clock, self._lock = clock or (lambda: datetime.now(timezone.utc)), threading.RLock()

    def _root(self) -> Path:
        if self.config_root is None:
            raise GameUserSettingsNotFoundError("LOCALAPPDATA is unavailable")
        return self.config_root.resolve()

    @staticmethod
    def _inside(path: Path | str, root: Path) -> Path:
        target = _path(path).resolve()
        try:
            target.relative_to(root.resolve())
        except ValueError as exc:
            raise GameUserSettingsError("path is outside the configured root") from exc
        return target

    def _inspect(self, path: Path) -> GameUserSettingsFile:
        root = self._root()
        path = self._inside(path, root)
        parts = path.relative_to(root).parts
        if len(parts) != 3 or parts[-1].casefold() != GAME_USER_SETTINGS_FILENAME.casefold() or not parts[1].casefold().startswith("windows"):
            raise GameUserSettingsFormatError("not a Windows*/GameUserSettings.ini path")
        info, raw = path.stat(), path.read_bytes()
        text, codec, bom = _decode(raw)
        if not _valid_ini(text):
            raise GameUserSettingsFormatError("settings file has no INI syntax")
        try:
            readonly = self._get_readonly(path)
        except OSError:
            readonly = None
        _unused, resolution, _changed = _entries(text)
        return GameUserSettingsFile(path, parts[0], parts[1], float(info.st_mtime), int(info.st_mtime_ns), len(raw), _sha(raw), readonly, codec, bool(bom), resolution, _file_id(info))

    def _candidates(self) -> list[Path]:
        root = self._root()
        if not root.is_dir():
            return []
        result = []
        try:
            profiles = list(root.iterdir())
        except OSError:
            return []
        for profile in profiles:
            if not profile.is_dir():
                continue
            try:
                result.extend(child / GAME_USER_SETTINGS_FILENAME for child in profile.iterdir() if child.is_dir() and child.name.casefold().startswith("windows") and (child / GAME_USER_SETTINGS_FILENAME).is_file())
            except OSError:
                continue
        return result

    def discover(self) -> GameUserSettingsFile | None:
        found = []
        for path in self._candidates():
            try:
                found.append(self._inspect(path))
            except (OSError, GameUserSettingsError):
                continue
        return max(found, key=lambda value: (value.mtime_ns, str(value.path).casefold())) if found else None

    def discover_info(self) -> dict[str, Any] | None:
        value = self.discover()
        return value.to_dict() if value else None

    def discovery_status(self) -> dict[str, Any]:
        """Return actionable, non-mutating diagnostics for first-run setup."""
        if self.config_root is None:
            return {"state": "unavailable", "reason": "local_appdata_missing", "candidate_count": 0, "invalid_count": 0}
        root = self.config_root.resolve()
        if not root.is_dir():
            return {"state": "not_found", "reason": "config_root_missing", "candidate_count": 0, "invalid_count": 0}
        files: list[Path] = []
        profiles = 0
        windows_dirs = 0
        try:
            for profile in root.iterdir():
                if not profile.is_dir():
                    continue
                profiles += 1
                for windows_dir in profile.iterdir():
                    if windows_dir.is_dir() and windows_dir.name.casefold().startswith("windows"):
                        windows_dirs += 1
                        candidate = windows_dir / GAME_USER_SETTINGS_FILENAME
                        if candidate.is_file():
                            files.append(candidate)
        except OSError:
            return {"state": "unavailable", "reason": "config_root_unreadable", "candidate_count": 0, "invalid_count": 0}
        invalid = 0
        for candidate in files:
            try:
                self._inspect(candidate)
            except (OSError, GameUserSettingsError):
                invalid += 1
        if not files:
            reason = "no_profiles" if profiles == 0 else "settings_file_missing" if windows_dirs else "windows_config_missing"
            return {"state": "not_found", "reason": reason, "candidate_count": 0, "invalid_count": 0}
        if invalid == len(files):
            return {"state": "invalid", "reason": "settings_file_invalid", "candidate_count": len(files), "invalid_count": invalid}
        return {"state": "ready", "reason": None, "candidate_count": len(files), "invalid_count": invalid}

    def _target(self, path: Path | str | None) -> GameUserSettingsFile:
        try:
            value = self.discover() if path is None else self._inspect(_path(path))
        except (OSError, GameUserSettingsError) as exc:
            raise GameUserSettingsNotFoundError(f"invalid settings path: {path}") from exc
        if value is None:
            raise GameUserSettingsNotFoundError("no valid GameUserSettings.ini found")
        return value

    def process_status(self) -> dict[str, Any]:
        if self._process_checker is None and sys.platform != "win32":
            return {"supported": False, "checked": True, "running": False, "process_name": VALORANT_PROCESS_NAME, "error": None}
        try:
            running = bool((self._process_checker or _tasklist_process_checker)())
            return {"supported": True, "checked": True, "running": running, "process_name": VALORANT_PROCESS_NAME, "error": None}
        except Exception as exc:  # noqa: BLE001
            return {"supported": True, "checked": False, "running": None, "process_name": VALORANT_PROCESS_NAME, "error": f"{type(exc).__name__}: {exc}"}

    def _guard(self) -> None:
        state = self.process_status()
        if not state["checked"]:
            raise GameUserSettingsProcessRunningError(f"cannot verify {VALORANT_PROCESS_NAME}: {state['error']}")
        if state["running"]:
            raise GameUserSettingsProcessRunningError(f"{VALORANT_PROCESS_NAME} is running")

    def get(self, path: Path | str | None = None) -> dict[str, Any]:
        with self._lock:
            result = self._target(path).to_dict()
            result.update(found=True, process=self.process_status())
            return result

    read = get

    def get_resolution(self, path: Path | str | None = None) -> dict[str, str]:
        return dict(self.get(path)["resolution"])

    def _state(self, path: Path) -> tuple[bool, bytes | None, bool | None, int | None]:
        if not path.is_file():
            return False, None, None, None
        info = path.stat()
        return True, path.read_bytes(), self._get_readonly(path), stat.S_IMODE(info.st_mode)

    def _atomic(self, path: Path, data: bytes, mode: int | None = None) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        temp: Path | None = None
        try:
            with tempfile.NamedTemporaryFile(prefix=f".{path.name}.", suffix=".tmp", dir=path.parent, delete=False) as handle:
                temp = Path(handle.name)
                if mode is not None:
                    os.chmod(temp, mode)
                handle.write(data)
                handle.flush()
                os.fsync(handle.fileno())
            os.replace(temp, path)
            if os.name != "nt":
                try:
                    directory = os.open(path.parent, os.O_RDONLY)
                except OSError:
                    directory = -1
                if directory >= 0:
                    try:
                        os.fsync(directory)
                    finally:
                        os.close(directory)
        finally:
            if temp is not None:
                try:
                    temp.unlink(missing_ok=True)
                except OSError:
                    pass

    def _json(self, path: Path, value: Mapping[str, Any]) -> None:
        self._atomic(path, json.dumps(value, ensure_ascii=False, indent=2).encode("utf-8"))

    def _backup(self, item: GameUserSettingsFile, data: bytes) -> tuple[dict[str, Any], Path]:
        self.backup_root.mkdir(parents=True, exist_ok=True)
        stamp = _timestamp(self._clock).replace("-", "").replace(":", "").replace(".", "")
        safe_profile = _SAFE_NAME.sub("_", item.profile).strip("._") or "profile"
        backup = self.backup_root / f"{safe_profile}-GameUserSettings.ini.{stamp}.bak"
        index = 0
        while backup.exists() or Path(f"{backup}.json").exists():
            index += 1
            backup = self.backup_root / f"{safe_profile}-GameUserSettings.ini.{stamp}-{index}.bak"
        sidecar = Path(f"{backup}.json")
        manifest = {
            "schema_version": 1, "created_at": _timestamp(self._clock), "original_path": str(item.path),
            "profile": item.profile, "windows_dir": item.windows_dir, "original_mtime": item.mtime,
            "original_mtime_ns": item.mtime_ns, "original_readonly": bool(item.readonly),
            "backup_path": str(backup), "original_sha256": _sha(data), "original_size": len(data),
            "original_file_id": list(item.file_id),
        }
        try:
            self._atomic(backup, data)
            if backup.read_bytes() != data:
                raise OSError("backup content verification failed")
            self._json(sidecar, manifest)
        except Exception:
            for partial in (backup, sidecar):
                try:
                    partial.unlink(missing_ok=True)
                except OSError:
                    pass
            raise
        return manifest, sidecar

    def _manifests(self) -> list[tuple[dict[str, Any], Path]]:
        if not self.backup_root.is_dir():
            return []
        result = []
        for sidecar in self.backup_root.glob("*.bak.json"):
            try:
                value = json.loads(sidecar.read_text(encoding="utf-8"))
                if not isinstance(value, dict) or not value.get("original_path") or not value.get("backup_path"):
                    continue
                backup = self._inside(value["backup_path"], self.backup_root)
                if not backup.is_file():
                    continue
                value["backup_path"] = str(backup)
                result.append((value, sidecar))
            except (OSError, ValueError, TypeError, GameUserSettingsError):
                continue
        result.sort(key=lambda pair: (str(pair[0].get("created_at") or ""), str(pair[1]).casefold()), reverse=True)
        return result

    @staticmethod
    def _same(left: Path | str, right: Path | str) -> bool:
        return os.path.normcase(str(_path(left).resolve())) == os.path.normcase(str(_path(right).resolve()))

    def _latest_manifest(self, path: Path | None = None) -> tuple[dict[str, Any], Path] | None:
        for manifest, sidecar in self._manifests():
            if path is None or self._same(manifest["original_path"], path):
                return manifest, sidecar
        return None

    def _record_current(self, manifest: Mapping[str, Any], data: bytes, path: Path) -> dict[str, Any]:
        info = path.stat()
        value = dict(manifest)
        value.update(last_written_at=_timestamp(self._clock), last_written_sha256=_sha(data), last_written_mtime=float(info.st_mtime), last_written_file_id=list(_file_id(info)))
        return value

    def _rollback(self, path: Path, old: tuple[bool, bytes | None, bool | None, int | None]) -> None:
        exists, data, readonly, mode = old
        if not exists:
            if path.exists():
                if self._get_readonly(path):
                    self._set_readonly(path, False)
                path.unlink()
            return
        error: Exception | None = None
        try:
            if path.exists() and self._get_readonly(path):
                self._set_readonly(path, False)
            assert data is not None
            self._atomic(path, data, mode)
            if path.read_bytes() != data:
                error = OSError("rollback content verification failed")
        except Exception as exc:  # noqa: BLE001
            error = exc
        finally:
            if readonly is not None and path.exists():
                try:
                    self._set_readonly(path, readonly)
                except Exception as exc:  # noqa: BLE001
                    error = error or exc
        if error:
            raise error
        if readonly is not None and self._get_readonly(path) != readonly:
            raise OSError("rollback read-only verification failed")

    def _replace(self, path: Path, data: bytes, readonly: bool, old: tuple[bool, bytes | None, bool | None, int | None]) -> None:
        try:
            if path.exists() and self._get_readonly(path):
                self._set_readonly(path, False)
            self._atomic(path, data, old[3])
            if not path.is_file() or path.read_bytes() != data:
                raise OSError("post-replace content verification failed")
            self._set_readonly(path, readonly)
            if self._get_readonly(path) != readonly:
                raise OSError("post-replace read-only verification failed")
        except Exception as exc:  # noqa: BLE001
            try:
                self._rollback(path, old)
            except Exception as rollback:  # noqa: BLE001
                raise GameUserSettingsTransactionError(f"write failed ({exc}); rollback failed ({rollback})") from exc
            raise GameUserSettingsTransactionError(f"write failed ({exc}); original state restored") from exc

    def set_resolution(self, width: int, height: int, path: Path | str | None = None) -> dict[str, Any]:
        if isinstance(width, bool) or isinstance(height, bool):
            raise ValueError("resolution dimensions must be integers")
        try:
            width, height = int(width), int(height)
        except (TypeError, ValueError) as exc:
            raise ValueError("resolution dimensions must be integers") from exc
        if not (1 <= width <= 16_384 and 1 <= height <= 16_384):
            raise ValueError("resolution dimensions must be between 1 and 16384")
        with self._lock:
            selected = self._target(path)
            self._guard()
            raw = selected.path.read_bytes()
            current = self._inspect(selected.path)
            if (current.mtime_ns, current.sha256) != (selected.mtime_ns, selected.sha256):
                raise GameUserSettingsConflictError("settings changed after discovery; retry detection")
            if current.readonly is None:
                raise GameUserSettingsError("cannot determine read-only state safely")
            text, codec, bom = _decode(raw)
            patched, _values, changed = _entries(text, width, height)
            if not changed:
                raise GameUserSettingsFormatError("no supported resolution keys exist")
            data = bom + patched.encode(codec)
            previous = (True, raw, current.readonly, stat.S_IMODE(selected.path.stat().st_mode))
            manifest, sidecar = self._backup(selected, raw)
            try:
                self._replace(selected.path, data, bool(current.readonly), previous)
                self._json(sidecar, self._record_current(manifest, data, selected.path))
            except Exception as exc:
                try:
                    self._rollback(selected.path, previous)
                except Exception as rollback:  # noqa: BLE001
                    raise GameUserSettingsTransactionError(f"transaction failed ({exc}); rollback failed ({rollback})") from exc
                if isinstance(exc, GameUserSettingsTransactionError):
                    raise
                raise GameUserSettingsTransactionError(f"transaction failed ({exc}); original state restored") from exc
            result = self._inspect(selected.path).to_dict()
            result.update(updated=True, width=width, height=height, changed_keys=changed, skipped_keys=[key for key in RESOLUTION_KEYS if key not in changed], backup_path=manifest["backup_path"], manifest_path=str(sidecar), original_readonly=manifest["original_readonly"])
            return result

    sync_resolution = set_resolution

    def lock(self, path: Path | str | None = None) -> dict[str, Any]:
        with self._lock:
            selected = self._target(path)
            self._guard()
            self._set_readonly(selected.path, True)
            if not self._get_readonly(selected.path):
                raise GameUserSettingsError("settings file did not become read-only")
            return {**self._inspect(selected.path).to_dict(), "locked": True, "updated": True}

    def unlock(self, path: Path | str | None = None) -> dict[str, Any]:
        with self._lock:
            selected = self._target(path)
            self._guard()
            self._set_readonly(selected.path, False)
            if self._get_readonly(selected.path):
                raise GameUserSettingsError("settings file did not become writable")
            return {**self._inspect(selected.path).to_dict(), "locked": False, "updated": True}

    def restore_latest_backup(self, path: Path | str | None = None) -> dict[str, Any]:
        with self._lock:
            self._guard()
            explicit = _path(path).resolve() if path is not None else None
            loaded = self._latest_manifest(explicit)
            if loaded is None and explicit is None:
                selected = self.discover()
                loaded = self._latest_manifest(selected.path if selected else None) or self._latest_manifest()
            if loaded is None:
                raise GameUserSettingsNotFoundError("no settings backup is available")
            manifest, sidecar = loaded
            target = self._inside(manifest["original_path"], self._root())
            backup = self._inside(manifest["backup_path"], self.backup_root)
            data = backup.read_bytes()
            if manifest.get("original_sha256") and _sha(data) != manifest["original_sha256"]:
                raise GameUserSettingsFormatError("backup checksum does not match manifest")
            previous = self._state(target)
            try:
                self._replace(target, data, bool(manifest.get("original_readonly")), previous)
                self._json(sidecar, self._record_current(manifest, data, target))
            except Exception as exc:
                try:
                    self._rollback(target, previous)
                except Exception as rollback:  # noqa: BLE001
                    raise GameUserSettingsTransactionError(f"restore metadata failed ({exc}); rollback failed ({rollback})") from exc
                if isinstance(exc, GameUserSettingsTransactionError):
                    raise
                raise GameUserSettingsTransactionError(f"restore metadata failed ({exc}); original state restored") from exc
            result = self._inspect(target).to_dict()
            result.update(restored=True, backup_path=str(backup), manifest_path=str(sidecar), original_readonly=bool(manifest.get("original_readonly")))
            return result

    restore = restore_latest_backup

    def status(self, path: Path | str | None = None) -> dict[str, Any]:
        with self._lock:
            explicit = _path(path).resolve() if path is not None else None
            selected = None
            selection_error = None
            if explicit is not None:
                try:
                    selected = self._inspect(explicit)
                except (OSError, GameUserSettingsError) as exc:
                    selection_error = f"{type(exc).__name__}: {exc}"
            else:
                selected = self.discover()
            loaded = self._latest_manifest(selected.path if selected else explicit)
            if loaded is None and selected is None and explicit is None:
                loaded = self._latest_manifest()
            manifest, sidecar = loaded if loaded else (None, None)
            target = selected.path if selected else (_path(manifest["original_path"]) if manifest else explicit)
            if target is not None:
                try:
                    target = self._inside(target, self._root())
                except GameUserSettingsError:
                    target = None
            exists, info, error = bool(target and target.is_file()), None, None
            if exists and target is not None:
                try:
                    value, data = target.stat(), target.read_bytes()
                    info = (float(value.st_mtime), _sha(data), _file_id(value), self._get_readonly(target))
                except OSError as exc:
                    exists, error = False, f"{type(exc).__name__}: {exc}"
            baseline = (manifest.get("last_written_sha256") or manifest.get("original_sha256")) if manifest else None
            expected_id = (manifest.get("last_written_file_id") or manifest.get("original_file_id")) if manifest else None
            drifted = bool(info and baseline and info[1] != baseline)
            recreated = bool(info and expected_id and list(info[2]) != list(expected_id))
            missing, readonly = bool(target is not None and not exists), (info[3] if info else None)
            state = "error" if selection_error else ("missing" if missing else "recreated" if recreated else "unlocked" if readonly is False else "drift" if drifted else "locked" if readonly is True else "not_found")
            return {
                "found": exists, "tracked": manifest is not None, "path": str(target) if target else None,
                "profile": selected.profile if selected else (manifest.get("profile") if manifest else None),
                "windows_dir": selected.windows_dir if selected else (manifest.get("windows_dir") if manifest else None),
                "missing": missing, "recreated": recreated, "locked": readonly is True,
                "unlocked": readonly is False, "readonly": readonly, "drifted": drifted, "state": state,
                "mtime": info[0] if info else None, "sha256": info[1] if info else None,
                "resolution": dict(selected.resolution) if selected else None,
                "resolution_keys": [key for key in RESOLUTION_KEYS if selected and key in selected.resolution],
                "backup_available": bool(manifest and Path(manifest["backup_path"]).is_file()),
                "backup_path": manifest.get("backup_path") if manifest else None,
                "manifest_path": str(sidecar) if sidecar else None,
                "original_mtime": manifest.get("original_mtime") if manifest else None,
                "original_readonly": manifest.get("original_readonly") if manifest else None,
                "original_sha256": manifest.get("original_sha256") if manifest else None,
                "process": self.process_status(), "error": selection_error or error,
            }


GameUserSettingsService = ValorantGameUserSettingsService


def discover_game_user_settings_file(local_app_data: Path | str | None = None, *, config_root: Path | str | None = None) -> GameUserSettingsFile | None:
    return ValorantGameUserSettingsService(local_app_data, config_root=config_root).discover()


def discover_game_user_settings(local_app_data: Path | str | None = None, *, config_root: Path | str | None = None) -> dict[str, Any] | None:
    value = discover_game_user_settings_file(local_app_data, config_root=config_root)
    return value.to_dict() if value else None


__all__ = [
    "GAME_USER_SETTINGS_FILENAME", "VALORANT_PROCESS_NAME", "RESOLUTION_KEYS", "RESOLUTION_KEY_SET",
    "RESOLUTION_X_KEYS", "RESOLUTION_Y_KEYS", "SUPPORTED_RESOLUTION_KEYS", "GameUserSettingsError",
    "GameUserSettingsNotFoundError", "GameUserSettingsFormatError", "GameUserSettingsConflictError",
    "GameUserSettingsProcessRunningError", "GameUserSettingsTransactionError", "GameUserSettingsFile",
    "ValorantGameUserSettingsService", "GameUserSettingsService", "get_file_readonly", "set_file_readonly",
    "is_valorant_running", "discover_game_user_settings_file", "discover_game_user_settings",
]
