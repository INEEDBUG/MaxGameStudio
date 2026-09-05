"""Crash-safe, byte-for-byte desktop storage migration.

This module is intentionally independent from the historical data migration:
it copies the complete storage layout to a new destination and never removes
or modifies a source.
"""
from __future__ import annotations

import argparse
import csv
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import re
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Callable, Iterable

try:
    from .desktop_data_migration import DesktopDataMigrationError, ensure_backend_stopped, _canonical_data_has_payload
    from .electron_ui_state_migration import discover_legacy_profiles
except ImportError:  # direct ``python -I desktop_storage_migration.py`` entry
    import importlib.util
    _module_path = Path(__file__).with_name("desktop_data_migration.py")
    _spec = importlib.util.spec_from_file_location("desktop_data_migration", _module_path)
    if _spec is None or _spec.loader is None:
        raise ImportError(f"cannot load {_module_path}")
    _module = importlib.util.module_from_spec(_spec)
    sys.modules[_spec.name] = _module
    _spec.loader.exec_module(_module)
    DesktopDataMigrationError = _module.DesktopDataMigrationError
    ensure_backend_stopped = _module.ensure_backend_stopped
    _canonical_data_has_payload = _module._canonical_data_has_payload
    _ui_spec = importlib.util.spec_from_file_location("electron_ui_state_migration", Path(__file__).with_name("electron_ui_state_migration.py"))
    _ui_module = importlib.util.module_from_spec(_ui_spec)
    sys.modules[_ui_spec.name] = _ui_module
    _ui_spec.loader.exec_module(_ui_module)
    discover_legacy_profiles = _ui_module.discover_legacy_profiles

MARKER_NAME = ".storage-migration-v1.json"
PROCESS_NAMES = {"maxgamestudioleague.exe", "cs2-insight-agent-desktop.exe",
                 "maxgamestudio.exe", "cs2 insight agent.exe", "cs2 ultimate insight studio.exe"}
SID_PATTERN = re.compile(r"^S-\d-\d+(?:-\d+)+$")
LAYOUT_NAMES = ("data", "webview", "league-runtime")


class StorageMigrationError(DesktopDataMigrationError):
    """Raised when a complete, safe migration cannot be proven."""


@dataclass(frozen=True)
class StorageMigrationResult:
    mode: str
    destination: str
    file_count: int
    byte_count: int
    marker: str


def _is_reparse(path: Path) -> bool:
    if path.is_symlink():
        return True
    try:
        attrs = getattr(os.stat(path, follow_symlinks=False), "st_file_attributes", 0)
    except OSError as exc:
        raise StorageMigrationError(f"cannot inspect {path}: {exc}") from exc
    return bool(attrs & getattr(__import__("stat"), "FILE_ATTRIBUTE_REPARSE_POINT", 0x400))


def _reject_reparse_tree(root: Path) -> None:
    if _is_reparse(root):
        raise StorageMigrationError(f"symlinks and junctions are not supported: {root}")
    if root.is_dir():
        for item in root.rglob("*"):
            if _is_reparse(item):
                raise StorageMigrationError(f"symlinks and junctions are not supported: {item}")


def _safe_resolve(path: Path) -> Path:
    absolute = Path(os.path.abspath(path.expanduser()))
    current = absolute
    while True:
        try:
            os.lstat(current)
        except FileNotFoundError:
            pass
        except OSError as exc:
            raise StorageMigrationError(f"cannot inspect path ancestor {current}: {exc}") from exc
        else:
            if _is_reparse(current):
                raise StorageMigrationError(f"symlinks and junctions are not supported: {current}")
        if current.parent == current:
            break
        current = current.parent
    return absolute.resolve()


def _files(root: Path) -> Iterable[Path]:
    for item in root.rglob("*"):
        if item.is_file():
            yield item


def _digest(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def _manifest(root: Path, *, exclude_marker: bool = False) -> dict[str, dict[str, int | str]]:
    result: dict[str, dict[str, int | str]] = {}
    for item in sorted(_files(root), key=lambda p: p.relative_to(root).as_posix()):
        rel = item.relative_to(root).as_posix()
        stat = item.stat()
        if not (exclude_marker and rel == MARKER_NAME):
            result[rel] = {"size": stat.st_size, "sha256": _digest(item)}
    return result


def _space_required(roots: Iterable[Path]) -> int:
    return sum(item.stat().st_size for root in roots for item in _files(root))


def _snapshot_path(path: Path) -> dict[str, dict[str, int | str]]:
    if path.is_dir():
        return _manifest(path)
    stat = path.stat()
    return {path.name: {"size": stat.st_size, "sha256": _digest(path)}}


def _same_content(left: Path, right: Path) -> bool:
    if left.is_dir() and right.is_dir():
        return _manifest(left) == _manifest(right)
    if left.is_file() and right.is_file():
        return left.stat().st_size == right.stat().st_size and _digest(left) == _digest(right)
    return False


def _process_snapshot() -> list[tuple[int, str]]:
    if os.name != "nt":
        return []
    try:
        completed = subprocess.run(
            [str(Path(os.environ.get("SystemRoot", "C:\\Windows")) / "System32" / "tasklist.exe"), "/FO", "CSV", "/NH"],
            check=True, capture_output=True, text=True,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        )
    except (OSError, subprocess.SubprocessError) as exc:
        raise StorageMigrationError(f"cannot inspect desktop processes: {exc}") from exc
    rows: list[tuple[int, str]] = []
    for line in completed.stdout.splitlines():
        try:
            fields = next(csv.reader([line]))
        except csv.Error as exc:
            raise StorageMigrationError("cannot parse desktop process list") from exc
        if len(fields) >= 2 and fields[0].lower() in PROCESS_NAMES:
            try:
                rows.append((int(fields[1]), fields[0]))
            except ValueError:
                raise StorageMigrationError("cannot parse desktop process list")
    return rows


def ensure_desktop_stopped(*, host_pid: int | None = None) -> None:
    if host_pid is not None and (isinstance(host_pid, bool) or host_pid <= 0):
        raise StorageMigrationError("--host-pid must be a positive integer")
    ensure_backend_stopped()
    running = _process_snapshot()
    blocked = [(pid, name) for pid, name in running if pid != host_pid]
    # A duplicate bootstrap now exits immediately when the native startup mutex
    # is owned. Confirm short-lived snapshots before rejecting; persistent live
    # hosts/League still fail closed, and no PID is silently exempted.
    for _ in range(2):
        if not blocked:
            break
        time.sleep(0.1)
        ensure_backend_stopped()
        blocked = [(pid, name) for pid, name in _process_snapshot() if pid != host_pid]
    if blocked:
        details = ", ".join(f"{name} (PID {pid})" for pid, name in blocked)
        raise StorageMigrationError(f"desktop process is still running: {details}")


def _secure_staging(path: Path) -> None:
    """Restrict a freshly-created staging directory before copying data."""
    if os.name != "nt":
        return
    system32 = Path(os.environ.get("SystemRoot", "C:\\Windows")) / "System32"
    whoami = system32 / "whoami.exe"
    icacls = system32 / "icacls.exe"
    try:
        result = subprocess.run([str(whoami), "/user", "/fo", "csv", "/nh"], check=True,
                                capture_output=True, text=True, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        rows = list(csv.reader(result.stdout.splitlines()))
        sid = rows[0][1] if rows and len(rows[0]) > 1 else ""
        if not SID_PATTERN.fullmatch(sid):
            raise StorageMigrationError("cannot validate current user SID")
        subprocess.run([str(icacls), str(path), "/inheritance:r", "/grant:r",
                        f"*{sid}:(OI)(CI)F", "*S-1-5-18:(OI)(CI)F",
                        "*S-1-5-32-544:(OI)(CI)F", "/q"], check=True,
                       capture_output=True, text=True, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    except (OSError, subprocess.SubprocessError) as exc:
        raise StorageMigrationError(f"cannot secure migration staging directory: {exc}") from exc


def _validate_target(source: Path, destination: Path) -> None:
    source = _safe_resolve(source)
    destination = _safe_resolve(destination)
    if source == destination or source in destination.parents or destination in source.parents:
        raise StorageMigrationError("source and destination must be distinct, non-nested paths")
    if not source.exists() or (not source.is_dir() and not source.is_file()):
        raise StorageMigrationError(f"source path does not exist: {source}")
    if destination.exists() and (not destination.is_dir() or any(destination.iterdir())):
        raise StorageMigrationError(f"destination must be absent or empty: {destination}")
    _reject_reparse_tree(source)


def _copy_tree(source: Path, destination: Path) -> None:
    destination.mkdir(parents=True, exist_ok=True)
    for item in sorted(source.rglob("*"), key=lambda p: p.relative_to(source).as_posix()):
        rel = item.relative_to(source)
        if item.parent == source and item.name == MARKER_NAME:
            continue
        target = destination / rel
        if _is_reparse(item):
            raise StorageMigrationError(f"symlinks and junctions are not supported: {item}")
        if item.is_dir():
            target.mkdir()
        elif item.is_file():
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(item, target)


def _build_sources(source: Path | None, appdata: Path | None, localappdata: Path | None) -> list[tuple[Path, Path]]:
    if source is not None:
        return [(source, Path("."))] if source.exists() else []
    assert appdata is not None and localappdata is not None
    candidates = [
        (appdata / "CS2 Insight Agent" / "data", Path("data")),
        (appdata / "com.cs2insightagent.app" / "data", Path("data")),
        (appdata / "cs2-insight-agent" / "data", Path("data")),
        (localappdata / "com.cs2insightagent.app", Path("webview")),
        (appdata / "MaxGameStudio" / "league-runtime", Path("league-runtime")),
    ]
    state = appdata / "com.cs2insightagent.app" / ".window-state.json"
    if state.is_file():
        candidates.append((state, Path("window-state.json")))
    # An empty canonical directory must not mask a populated older layout.
    existing = [(path, rel) for path, rel in candidates
                if path.exists() and (path.is_file() or any(item.is_file() for item in path.rglob("*")))]
    # Prefer recognized configuration/database payload over log-only/metadata
    # directories. Retain original order among valid versions; if no recognized
    # layout exists, preserve the nonempty unknown data instead of discarding it.
    existing.sort(key=lambda pair: 0 if pair[1] == Path("data") and _canonical_data_has_payload(pair[0]) else 1)
    result = []
    data_selected = False
    for path, rel in existing:
        if rel == Path("data"):
            if data_selected:
                continue
            data_selected = True
        result.append((path, rel))
    if not any(rel == Path("data") for _, rel in result):
        for root in (appdata / "com.cs2insightagent.app", appdata / "cs2-insight-agent"):
            if root.is_dir() and any((root / name).exists() for name in ("cs2-insight.config.json", "cs2-insight.db")):
                result.insert(0, (root, Path("data")))
                break
    return result


def _legacy_cache_target(link: Path, destination: Path) -> Path:
    """Read-only exception for historical cache/trash directory junctions.

    Never carry a junction into the new root, follow a nested link, or touch
    the old link/target. Ordinary source roots retain the strict no-link rule.
    """
    info = link.lstat()
    if os.name != "nt" or link.name.lower() not in {"cache", "trash"} or getattr(info, "st_reparse_tag", 0) != 0xA0000003:
        raise StorageMigrationError(f"unsupported legacy junction: {link}")
    raw = os.readlink(link)
    if raw.startswith("\\\\?\\"):
        raw = raw[4:]
    target = Path(raw)
    system = os.environ.get("SystemDrive", "C:").lower()
    if (not re.match(r"^[A-Za-z]:\\", raw) or target.drive.lower() == system
            or target.drive.lower() != destination.drive.lower()):
        raise StorageMigrationError("legacy junction must target the selected local non-system volume")
    target = _safe_resolve(target)
    if not target.is_dir():
        raise StorageMigrationError(f"legacy junction target is unavailable: {link}")
    return target


def _expand_legacy_cache_sources(pairs: list[tuple[Path, Path]], destination: Path):
    expanded = []
    layouts: dict[Path, list[str]] = {}
    aliases: dict[Path, Path] = {}
    for path, rel in pairs:
        if rel != Path("data"):
            expanded.append((path, rel))
            continue
        path = _safe_resolve(path)
        children = list(path.iterdir())
        if not any(_is_reparse(child) for child in children):
            expanded.append((path, rel))
            continue
        layouts[path] = sorted(child.name for child in children)
        for child in children:
            source = child
            if _is_reparse(child):
                source = _legacy_cache_target(child, destination)
                if source == path or source in path.parents or path in source.parents:
                    raise StorageMigrationError("legacy junction creates an overlapping source tree")
                aliases[child] = source
            expanded.append((source, rel / child.name))
    return expanded, layouts, aliases


def _verify_legacy_cache_sources(layouts, aliases, destination: Path) -> None:
    for path, expected in layouts.items():
        _safe_resolve(path)
        if sorted(child.name for child in path.iterdir()) != expected:
            raise StorageMigrationError("legacy source layout changed during migration")
    for link, expected in aliases.items():
        if _legacy_cache_target(link, destination) != expected:
            raise StorageMigrationError("legacy junction target changed during migration")


def migrate_storage(*, destination: Path, source: Path | None = None,
                    legacy_appdata: Path | None = None, legacy_localappdata: Path | None = None,
                    host_pid: int | None = None,
                    process_check: bool = True,
                    copy_impl: Callable[[Path, Path], None] = _copy_tree) -> StorageMigrationResult:
    if source is not None and (legacy_appdata is not None or legacy_localappdata is not None):
        raise StorageMigrationError("source mode cannot include legacy appdata paths")
    if (source is None) == (legacy_appdata is None or legacy_localappdata is None):
        raise StorageMigrationError("provide either --source or both legacy appdata paths")
    source_root = _safe_resolve(Path(source)) if source is not None else None
    appdata_root = _safe_resolve(Path(legacy_appdata)) if legacy_appdata is not None else None
    local_root = _safe_resolve(Path(legacy_localappdata)) if legacy_localappdata is not None else None
    destination = _safe_resolve(Path(destination))
    pairs = _build_sources(source_root, appdata_root, local_root)
    invocation = {"mode": "source" if source is not None else "legacy",
                  "source": str(source_root) if source_root else None,
                  "legacy_appdata": str(appdata_root) if appdata_root else None,
                  "legacy_localappdata": str(local_root) if local_root else None,
                  "destination": str(destination)}
    if source is not None and not pairs:
        raise StorageMigrationError("source directory does not exist")
    if source_root is not None and not source_root.is_dir():
        raise StorageMigrationError("source must be a directory")
    layouts, aliases = {}, {}
    if appdata_root is not None:
        pairs, layouts, aliases = _expand_legacy_cache_sources(pairs, destination)
    if appdata_root is not None:
        # Never launch an old Electron executable to export settings: it can
        # write to its live APPDATA profile. Preserve an offline recovery copy.
        for profile in discover_legacy_profiles(appdata_root):
            pairs.append((profile, Path("legacy-electron-profiles") / profile.name))
    # A source-root invocation must reject nesting even when its children are
    # individually siblings of the destination.
    if source is not None:
        source_root = _safe_resolve(Path(source))
        if source_root == destination or source_root in destination.parents or destination in source_root.parents:
            raise StorageMigrationError("source and destination must be distinct, non-nested paths")
    marker = destination / MARKER_NAME
    if process_check:
        ensure_desktop_stopped(host_pid=host_pid)
    if destination.is_dir() and marker.is_file():
        try:
            body = json.loads(marker.read_text(encoding="utf-8"))
            if body.get("version") == 1 and body.get("invocation") == invocation and body.get("manifest") == _manifest(destination, exclude_marker=True):
                return StorageMigrationResult("existing", str(destination), body["file_count"], body["byte_count"], str(marker))
        except (OSError, ValueError, KeyError, TypeError):
            pass
    if destination.exists() and (not destination.is_dir() or any(destination.iterdir())):
        raise StorageMigrationError(f"destination must be absent or empty: {destination}")
    for path, _ in pairs:
        _validate_target(path, destination)
    destination.parent.mkdir(parents=True, exist_ok=True)
    source_manifests = {str(path): _snapshot_path(path) for path, _ in pairs}
    required = _space_required(path for path, _ in pairs if path.is_dir()) + sum(
        path.stat().st_size for path, _ in pairs if path.is_file()
    )
    if shutil.disk_usage(destination.parent).free < required + max(64 * 1024 * 1024, required // 10):
        raise StorageMigrationError("insufficient disk space for storage migration")
    staging = Path(tempfile.mkdtemp(prefix=".storage-migration-", dir=destination.parent))
    try:
        _secure_staging(staging)
        for path, rel in pairs:
            target = staging / rel
            if path.is_dir():
                copy_impl(path, target)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(path, target)
            copied_ok = (_manifest(path, exclude_marker=True) == _manifest(target)) if path.is_dir() else _same_content(path, target)
            if not copied_ok:
                raise StorageMigrationError(f"copied content hash mismatch: {path}")
        for name in LAYOUT_NAMES:
            (staging / name).mkdir(exist_ok=True)
        if any(source_manifests[str(path)] != _snapshot_path(path) for path, _ in pairs):
            raise StorageMigrationError("source changed during migration")
        manifest = _manifest(staging, exclude_marker=True)
        expected_count = len(manifest)
        expected_bytes = sum(int(item["size"]) for item in manifest.values())
        body = {"version": 1, "completed_at": datetime.now(timezone.utc).isoformat(),
                "invocation": invocation,
                "legacy_aliases": {str(link): str(target) for link, target in aliases.items()},
                "rollback_sources": [str(path) for path, _ in pairs], "file_count": expected_count,
                "byte_count": expected_bytes, "manifest": manifest}
        (staging / MARKER_NAME).write_text(json.dumps(body, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        # Close the time-of-check window: promotion is allowed only if all
        # writers remain stopped and every source still matches its snapshot.
        if process_check:
            ensure_desktop_stopped(host_pid=host_pid)
        _verify_legacy_cache_sources(layouts, aliases, destination)
        for path, _ in pairs:
            _reject_reparse_tree(path)
        if any(source_manifests[str(path)] != _snapshot_path(path) for path, _ in pairs):
            raise StorageMigrationError("source changed before migration promotion")
        if destination.exists():
            destination.rmdir()
        os.replace(staging, destination)
    except Exception:
        shutil.rmtree(staging, ignore_errors=True)
        raise
    return StorageMigrationResult("migrated", str(destination), expected_count, expected_bytes, str(destination / MARKER_NAME))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Safely copy MaxGameStudio desktop storage")
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--source", type=Path)
    group.add_argument("--legacy-appdata", type=Path)
    parser.add_argument("--legacy-localappdata", type=Path)
    parser.add_argument("--destination", required=True, type=Path)
    parser.add_argument("--host-pid", type=int)
    args = parser.parse_args(argv)
    try:
        result = migrate_storage(destination=args.destination, source=args.source,
                                 legacy_appdata=args.legacy_appdata,
                                 legacy_localappdata=args.legacy_localappdata,
                                 host_pid=args.host_pid)
    except (StorageMigrationError, OSError) as exc:
        print(f"storage migration failed: {exc}", file=sys.stderr)
        return 2
    print(json.dumps(result.__dict__, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
