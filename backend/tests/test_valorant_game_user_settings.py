from __future__ import annotations

import json
import os
import stat
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
from types import SimpleNamespace

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from app.valorant_lab import game_user_settings as settings


def _settings_path(local_app_data: Path, profile: str = "profile-a", windows_dir: str = "WindowsClient") -> Path:
    return (
        local_app_data
        / "VALORANT"
        / "Saved"
        / "Config"
        / profile
        / windows_dir
        / settings.GAME_USER_SETTINGS_FILENAME
    )


def _write_settings(
    local_app_data: Path,
    body: str | bytes,
    *,
    profile: str = "profile-a",
    windows_dir: str = "WindowsClient",
    encoding: str = "utf-8",
) -> Path:
    path = _settings_path(local_app_data, profile, windows_dir)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(body if isinstance(body, bytes) else body.encode(encoding))
    return path


def _service(
    tmp_path: Path,
    *,
    process_checker=None,
    initial_readonly: dict[Path, bool] | None = None,
    clock=None,
) -> tuple[settings.ValorantGameUserSettingsService, Path, dict[Path, bool]]:
    local_app_data = tmp_path / "local-app-data"
    flags = {key.resolve(): value for key, value in (initial_readonly or {}).items()}

    def readonly_getter(path: Path) -> bool:
        return flags.get(path.resolve(), False)

    def readonly_setter(path: Path, value: bool) -> None:
        flags[path.resolve()] = bool(value)

    service = settings.ValorantGameUserSettingsService(
        local_app_data,
        backup_root=tmp_path / "backups",
        process_checker=process_checker if process_checker is not None else (lambda: False),
        readonly_getter=readonly_getter,
        readonly_setter=readonly_setter,
        clock=clock,
    )
    path = _write_settings(
        local_app_data,
        "[/Script/Engine.GameUserSettings]\nResolutionSizeX=1920\nResolutionSizeY=1080\n",
    )
    return service, path, flags


def test_discovery_uses_dynamic_profile_and_windows_directory_and_returns_one(tmp_path: Path):
    local = tmp_path / "local"
    older = _write_settings(local, "[Section]\nResolutionSizeX=1280\n", profile="profile-old", windows_dir="WindowsA")
    newer = _write_settings(local, "[Section]\nResolutionSizeX=1920\n", profile="arbitrary-profile", windows_dir="WindowsClient")
    os.utime(older, ns=(1_000_000_000, 1_000_000_000))
    os.utime(newer, ns=(2_000_000_000, 2_000_000_000))

    service = settings.ValorantGameUserSettingsService(local, process_checker=lambda: False)

    selected = service.discover()

    assert selected is not None
    assert selected.path == newer.resolve()
    assert selected.profile == "arbitrary-profile"
    assert selected.windows_dir == "WindowsClient"
    assert not isinstance(selected, list)


def test_discovery_skips_newer_invalid_file(tmp_path: Path):
    local = tmp_path / "local"
    valid = _write_settings(local, "[Section]\nResolutionSizeX=1280\n", profile="valid", windows_dir="WindowsA")
    invalid = _write_settings(local, b"\xff\xfe\x01", profile="invalid", windows_dir="WindowsB")
    os.utime(valid, ns=(1_000_000_000, 1_000_000_000))
    os.utime(invalid, ns=(3_000_000_000, 3_000_000_000))

    service = settings.ValorantGameUserSettingsService(local, process_checker=lambda: False)

    assert service.discover().path == valid.resolve()


def test_discovery_rejects_non_windows_directory_and_garbage_ini(tmp_path: Path):
    local = tmp_path / "local"
    _write_settings(local, "not an ini file", profile="garbage", windows_dir="WindowsA")
    _write_settings(local, "[Section]\nResolutionSizeX=1\n", profile="wrong-folder", windows_dir="LinuxClient")

    service = settings.ValorantGameUserSettingsService(local, process_checker=lambda: False)

    assert service.discover() is None


def test_get_snapshot_reports_only_actual_supported_keys(tmp_path: Path):
    service, path, _flags = _service(tmp_path)
    path.write_text(
        "[Section]\nResolutionSizeX=1920\nDesiredScreenHeight=1080\n"
        "SomeResolutionSizeX=999\nFullscreenMode=1\n",
        encoding="utf-8",
    )

    result = service.get()

    assert result["resolution"] == {"ResolutionSizeX": "1920", "DesiredScreenHeight": "1080"}
    assert result["resolution_keys"] == ["ResolutionSizeX", "DesiredScreenHeight"]
    assert "SomeResolutionSizeX" not in result["resolution"]
    assert result["path"] == str(path.resolve())


def test_set_patches_existing_keys_preserves_comments_order_newlines_and_fullscreen(tmp_path: Path):
    service, path, _flags = _service(tmp_path)
    original = (
        "; keep this comment\r\n"
        "[Section]\r\n"
        "ResolutionSizeX = 1920 ; width comment\r\n"
        "Unknown=keep\r\n"
        "FullscreenMode=1\r\n"
        "ResolutionSizeY=1080\r\n"
        "ResolutionSizeX=1900\r\n"
        "# keep this comment\r\n"
    )
    path.write_bytes(original.encode("utf-8"))

    result = service.set_resolution(1568, 1080)
    updated = path.read_bytes().decode("utf-8")

    assert result["changed_keys"] == ["ResolutionSizeX", "ResolutionSizeY", "ResolutionSizeX"]
    assert updated == (
        "; keep this comment\r\n"
        "[Section]\r\n"
        "ResolutionSizeX = 1568 ; width comment\r\n"
        "Unknown=keep\r\n"
        "FullscreenMode=1\r\n"
        "ResolutionSizeY=1080\r\n"
        "ResolutionSizeX=1568\r\n"
        "# keep this comment\r\n"
    )
    assert updated.count("\r\n") == original.count("\r\n")


@pytest.mark.parametrize(
    ("encoding", "prefix"),
    [("utf-8", b"\xef\xbb\xbf"), ("utf-16-le", b"\xff\xfe"), ("utf-16-be", b"\xfe\xff")],
)
def test_set_preserves_utf_bom_and_encoding(tmp_path: Path, encoding: str, prefix: bytes):
    service, path, _flags = _service(tmp_path)
    body = "[Section]\r\nResolutionSizeX=1920\r\nResolutionSizeY=1080\r\n"
    path.write_bytes(prefix + body.encode(encoding))

    service.set_resolution(1280, 720)
    raw = path.read_bytes()

    assert raw.startswith(prefix)
    decoded = raw[len(prefix) :].decode(encoding)
    assert "ResolutionSizeX=1280\r\n" in decoded
    assert "ResolutionSizeY=720\r\n" in decoded


def test_bomless_utf16_is_read_and_rewritten(tmp_path: Path):
    service, path, _flags = _service(tmp_path)
    body = "[Section]\nResolutionSizeX=1920\nResolutionSizeY=1080\n"
    path.write_bytes(body.encode("utf-16-le"))

    service.set_resolution(1024, 768)

    raw = path.read_bytes()
    assert not raw.startswith((b"\xef\xbb\xbf", b"\xff\xfe", b"\xfe\xff"))
    assert "ResolutionSizeX=1024" in raw.decode("utf-16-le")


def test_set_updates_only_existing_x_and_y_keys(tmp_path: Path):
    service, path, _flags = _service(tmp_path)
    path.write_text("[Section]\nResolutionSizeX=1920\nOther=keep\n", encoding="utf-8")

    result = service.set_resolution(1600, 900)

    assert "ResolutionSizeX=1600" in path.read_text(encoding="utf-8")
    assert "ResolutionSizeY" not in path.read_text(encoding="utf-8")
    assert result["changed_keys"] == ["ResolutionSizeX"]
    assert set(result["skipped_keys"]) == set(settings.RESOLUTION_KEYS) - {"ResolutionSizeX"}


def test_set_requires_at_least_one_supported_key(tmp_path: Path):
    service, path, flags = _service(tmp_path)
    path.write_text("[Section]\nFullscreenMode=1\n", encoding="utf-8")
    before = path.read_bytes()

    with pytest.raises(settings.GameUserSettingsFormatError):
        service.set_resolution(1280, 720)

    assert path.read_bytes() == before
    assert flags.get(path.resolve(), False) is False
    assert list((tmp_path / "backups").glob("*")) == []


def test_set_validates_dimensions(tmp_path: Path):
    service, path, _flags = _service(tmp_path)
    before = path.read_bytes()

    for width, height in ((0, 720), (-1, 720), (16_385, 720), (1280, 0), (True, 720), ("bad", 720)):
        with pytest.raises(ValueError):
            service.set_resolution(width, height)

    assert path.read_bytes() == before


def test_readonly_get_set_use_injected_python_api(tmp_path: Path):
    service, path, flags = _service(tmp_path)

    assert service.get()["readonly"] is False
    locked = service.lock()
    assert locked["readonly"] is True
    assert flags[path.resolve()] is True
    unlocked = service.unlock()
    assert unlocked["readonly"] is False
    assert flags[path.resolve()] is False


def test_locked_set_temporarily_unlocks_then_relocks_and_records_original(tmp_path: Path):
    service, path, flags = _service(tmp_path)
    flags[path.resolve()] = True
    original = path.read_bytes()

    result = service.set_resolution(1568, 1080)

    assert result["updated"] is True
    assert result["original_readonly"] is True
    assert flags[path.resolve()] is True
    assert path.read_bytes() != original
    manifest = json.loads(Path(result["manifest_path"]).read_text(encoding="utf-8"))
    assert manifest["original_readonly"] is True
    assert manifest["original_path"] == str(path.resolve())
    assert manifest["profile"] == "profile-a"
    assert manifest["backup_path"] == result["backup_path"]


def test_unlocked_set_remains_unlocked(tmp_path: Path):
    service, path, flags = _service(tmp_path)

    result = service.set_resolution(1600, 900)

    assert result["readonly"] is False
    assert flags[path.resolve()] is False


def test_backup_is_full_byte_exact_and_timestamped(tmp_path: Path):
    fixed_clock = lambda: datetime(2026, 9, 1, 1, 2, 3, 456789, tzinfo=timezone.utc)
    service, path, _flags = _service(tmp_path, clock=fixed_clock)
    original = path.read_bytes()

    result = service.set_resolution(1280, 720)
    backup = Path(result["backup_path"])

    assert backup.is_file()
    assert backup.read_bytes() == original
    assert "20260901T010203456789Z" in backup.name
    assert Path(result["manifest_path"]).is_file()


def test_atomic_writer_fsyncs_and_replaces_through_temp_file(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    service, path, _flags = _service(tmp_path)
    fsync_calls: list[int] = []
    replace_calls: list[tuple[Path, Path]] = []
    real_fsync = settings.os.fsync
    real_replace = settings.os.replace

    def fsync(fd: int):
        fsync_calls.append(fd)
        return real_fsync(fd)

    def replace(source, target):
        replace_calls.append((Path(source), Path(target)))
        return real_replace(source, target)

    monkeypatch.setattr(settings.os, "fsync", fsync)
    monkeypatch.setattr(settings.os, "replace", replace)

    service.set_resolution(1280, 720)

    assert fsync_calls
    assert any(target == path.resolve() for _source, target in replace_calls)
    assert not list(path.parent.glob(f".{path.name}.*.tmp"))


def test_replace_failure_rolls_back_content_and_readonly(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    service, path, flags = _service(tmp_path)
    flags[path.resolve()] = True
    original = path.read_bytes()
    real_replace = settings.os.replace

    def fail_target(source, target):
        if Path(target).resolve() == path.resolve():
            raise OSError("simulated replace failure")
        return real_replace(source, target)

    monkeypatch.setattr(settings.os, "replace", fail_target)

    with pytest.raises(settings.GameUserSettingsTransactionError):
        service.set_resolution(1280, 720)

    assert path.read_bytes() == original
    assert flags[path.resolve()] is True


def test_post_replace_verification_failure_rolls_back(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    service, path, flags = _service(tmp_path)
    flags[path.resolve()] = True
    original = path.read_bytes()
    real_transaction = service._replace

    def fail_after_replace(target, data, readonly, previous):
        real_transaction(target, data, readonly, previous)
        target.write_bytes(b"corrupted-after-verify")
        raise OSError("simulated post-write verification failure")

    monkeypatch.setattr(service, "_replace", fail_after_replace)

    with pytest.raises(settings.GameUserSettingsTransactionError):
        service.set_resolution(1280, 720)

    assert path.read_bytes() == original
    assert flags[path.resolve()] is True


def test_manifest_failure_rolls_back_target_and_attribute(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    service, path, flags = _service(tmp_path)
    flags[path.resolve()] = True
    original = path.read_bytes()
    calls = 0
    real_writer = service._json

    def fail_second_manifest(path_arg, value):
        nonlocal calls
        calls += 1
        if calls == 2:
            raise OSError("simulated manifest failure")
        return real_writer(path_arg, value)

    monkeypatch.setattr(service, "_json", fail_second_manifest)

    with pytest.raises(settings.GameUserSettingsTransactionError):
        service.set_resolution(1280, 720)

    assert path.read_bytes() == original
    assert flags[path.resolve()] is True


def test_restore_latest_backup_restores_bytes_and_original_readonly(tmp_path: Path):
    service, path, flags = _service(tmp_path)
    flags[path.resolve()] = True
    original = path.read_bytes()
    service.set_resolution(1280, 720)
    assert path.read_bytes() != original
    flags[path.resolve()] = False

    result = service.restore_latest_backup()

    assert result["restored"] is True
    assert path.read_bytes() == original
    assert flags[path.resolve()] is True


def test_restore_recreates_missing_target_from_latest_backup(tmp_path: Path):
    service, path, flags = _service(tmp_path)
    original = path.read_bytes()
    service.set_resolution(1280, 720)
    path.unlink()
    flags.pop(path.resolve(), None)

    result = service.restore_latest_backup()

    assert result["restored"] is True
    assert path.read_bytes() == original
    assert flags[path.resolve()] is False


def test_status_detects_missing_recreated_unlocked_and_drift(tmp_path: Path):
    service, path, flags = _service(tmp_path)
    service.set_resolution(1280, 720)

    locked_status = service.status()
    assert locked_status["missing"] is False
    assert locked_status["locked"] is False  # the fixture starts writable
    assert locked_status["drifted"] is False

    path.unlink()
    flags.pop(path.resolve(), None)
    missing = service.status()
    assert missing["missing"] is True
    assert missing["state"] == "missing"

    _write_settings(service.config_root.parent.parent.parent, "[Section]\nResolutionSizeX=1\n", profile="profile-a", windows_dir="WindowsClient")
    recreated = service.status()
    assert recreated["recreated"] is True

    flags[path.resolve()] = False
    unlocked = service.status()
    assert unlocked["unlocked"] is True
    assert unlocked["state"] == "recreated"  # recreation evidence has priority

    flags[path.resolve()] = True
    path.write_text("[Section]\nResolutionSizeX=999\n", encoding="utf-8")
    drift = service.status()
    assert drift["drifted"] is True
    assert drift["recreated"] is True


def test_status_without_backup_still_reports_readonly_state(tmp_path: Path):
    local = tmp_path / "local"
    path = _write_settings(local, "[Section]\nResolutionSizeX=1\n")
    flags = {path.resolve(): True}
    service = settings.ValorantGameUserSettingsService(
        local,
        backup_root=tmp_path / "backups",
        process_checker=lambda: False,
        readonly_getter=lambda candidate: flags.get(candidate.resolve(), False),
        readonly_setter=lambda candidate, value: flags.__setitem__(candidate.resolve(), bool(value)),
    )

    result = service.status()

    assert result["tracked"] is False
    assert result["locked"] is True
    assert result["drifted"] is False


def test_process_check_blocks_mutations_without_kill_or_vanguard_access(tmp_path: Path):
    service, path, flags = _service(tmp_path, process_checker=lambda: True)
    original = path.read_bytes()

    for action in (lambda: service.set_resolution(1280, 720), service.lock, service.unlock, service.restore_latest_backup):
        with pytest.raises(settings.GameUserSettingsProcessRunningError):
            action()

    assert path.read_bytes() == original
    assert flags.get(path.resolve(), False) is False


def test_process_status_reports_injected_checker(tmp_path: Path):
    service, _path, _flags = _service(tmp_path, process_checker=lambda: True)

    result = service.process_status()

    assert result == {
        "supported": True,
        "checked": True,
        "running": True,
        "process_name": settings.VALORANT_PROCESS_NAME,
        "error": None,
    }


def test_tasklist_process_probe_is_exact_and_shell_free(monkeypatch: pytest.MonkeyPatch):
    calls: list[dict] = []
    monkeypatch.setattr(settings.sys, "platform", "win32")

    def fake_run(command, **kwargs):
        calls.append({"command": command, **kwargs})
        return SimpleNamespace(stdout="VALORANT-Win64-Shipping.exe  1234 Console 1 100,000 K\r\n", returncode=0)

    monkeypatch.setattr(settings.subprocess, "run", fake_run)

    assert settings.is_valorant_running() is True
    assert calls[0]["command"] == [
        "tasklist",
        "/FI",
        "IMAGENAME eq VALORANT-Win64-Shipping.exe",
        "/NH",
    ]
    assert calls[0]["shell"] is False
    assert "cmd.exe" not in calls[0]["command"]


def test_tasklist_probe_does_not_match_similar_process_name(monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr(settings.sys, "platform", "win32")
    monkeypatch.setattr(
        settings.subprocess,
        "run",
        lambda *args, **kwargs: SimpleNamespace(
            stdout="VALORANT-Win64-Shipping-helper.exe  1234 Console 1 100,000 K\r\n", returncode=0
        ),
    )

    assert settings.is_valorant_running() is False


def test_backup_path_outside_configured_root_is_rejected(tmp_path: Path):
    service, path, _flags = _service(tmp_path)
    outside = tmp_path / "outside.bak"
    outside.write_bytes(path.read_bytes())
    sidecar = tmp_path / "backups" / "tampered.bak.json"
    sidecar.parent.mkdir(parents=True, exist_ok=True)
    sidecar.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "created_at": "2026-09-01T00:00:00Z",
                "original_path": str(path),
                "profile": "profile-a",
                "original_mtime": 1,
                "original_readonly": False,
                "backup_path": str(outside),
                "original_sha256": "",
                "original_size": 0,
            }
        ),
        encoding="utf-8",
    )

    assert service._manifests() == []
