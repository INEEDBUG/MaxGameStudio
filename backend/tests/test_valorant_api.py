from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest
from fastapi import FastAPI
from fastapi.testclient import TestClient

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from app.valorant_lab import api
from app.valorant_lab.game_user_settings import GameUserSettingsProcessRunningError


def _client(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> TestClient:
    profile_path = tmp_path / "valorant-crosshair-profiles.json"
    monkeypatch.setattr(api, "_profiles_path", lambda: profile_path)
    application = FastAPI()
    application.include_router(api.router)
    return TestClient(application)


def _valid_profiles() -> dict[str, dict[str, object]]:
    return {
        "P": {"color": "cyan", "innerLinesLength": 6, "centerDot": True},
        "A": {"color": "yellow"},
        "S": {"color": "red", "centerDot": True},
    }


def test_encode_endpoint_returns_native_code_and_422_for_invalid_profiles(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
):
    client = _client(monkeypatch, tmp_path)

    response = client.post(
        "/api/valorant-lab/crosshair/encode",
        json={"profiles": _valid_profiles()},
    )
    assert response.status_code == 200
    assert response.json()["code"].startswith("0;s;1;P;")
    assert response.json()["format"] == "valorant-native-v0"

    for profiles in (
        {"Q": {}},
        {"P": {"outlineOpacity": 2}},
        {"P": {"color": "invalid"}},
    ):
        invalid = client.post(
            "/api/valorant-lab/crosshair/encode",
            json={"profiles": profiles},
        )
        assert invalid.status_code == 422


def test_decode_endpoint_preserves_unknown_fields_and_rejects_invalid_known_fields(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
):
    client = _client(monkeypatch, tmp_path)

    valid = client.post(
        "/api/valorant-lab/crosshair/decode",
        json={"code": "0;P;c;5;future;keep-me"},
    )
    assert valid.status_code == 200
    assert "future;keep-me" in valid.json()["code"]
    assert valid.json()["profiles"]["P"]["color"] == "cyan"

    for code in ("", "1;P;c;5", "0;P;c;99", "0;P;c;5;c;4", "0;P;c"):
        invalid = client.post(
            "/api/valorant-lab/crosshair/decode",
            json={"code": code},
        )
        assert invalid.status_code == 422


def test_put_crosshair_writes_atomically_and_get_reencodes_saved_profiles(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
):
    client = _client(monkeypatch, tmp_path)
    profile_path = api._profiles_path()

    response = client.put(
        "/api/valorant-lab/crosshair",
        json={"profiles": _valid_profiles()},
    )
    assert response.status_code == 200
    assert response.json()["saved"] is True
    assert profile_path.is_file()
    assert not profile_path.with_suffix(profile_path.suffix + ".tmp").exists()
    stored = json.loads(profile_path.read_text(encoding="utf-8"))
    assert stored["code"].startswith("0;s;1;P;")

    loaded = client.get("/api/valorant-lab/crosshair")
    assert loaded.status_code == 200
    assert loaded.json()["code"] == stored["code"]


def test_atomic_profile_write_removes_failed_staging_file_and_keeps_original(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
):
    _client(monkeypatch, tmp_path)
    profile_path = api._profiles_path()
    original = '{"profiles":{"P":{}},"code":"old"}'
    profile_path.write_text(original, encoding="utf-8")

    with pytest.raises(TypeError):
        api._write_profiles({"not_json": object()})

    assert profile_path.read_text(encoding="utf-8") == original
    assert not profile_path.with_suffix(profile_path.suffix + ".tmp").exists()


class _FakeCfgService:
    def __init__(self) -> None:
        self.calls: list[tuple] = []
        self.readonly = False
        self.backup_available = False
        self.resolution = {"ResolutionSizeX": "1920", "ResolutionSizeY": "1080"}
        self.set_error: Exception | None = None

    def status(self):
        return {
            "found": True,
            "state": "locked" if self.readonly else "unlocked",
            "readonly": self.readonly,
            "locked": self.readonly,
            "backup_available": self.backup_available,
            "drifted": False,
            "recreated": False,
            "profile": "profile-a",
            "windows_dir": "WindowsClient",
        }

    def get(self):
        return {**self.status(), "resolution": dict(self.resolution), "path": "C:/fixture/GameUserSettings.ini"}

    def set_resolution(self, width, height):
        self.calls.append(("set", width, height))
        if self.set_error:
            raise self.set_error
        self.backup_available = True
        for key in tuple(self.resolution):
            self.resolution[key] = str(width if key.endswith("X") or key.endswith("Width") else height)
        return self.get()

    def lock(self):
        self.calls.append(("lock",))
        self.readonly = True
        return self.get()

    def unlock(self):
        self.calls.append(("unlock",))
        self.readonly = False
        return self.get()

    def restore_latest_backup(self):
        self.calls.append(("restore",))
        self.resolution = {"ResolutionSizeX": "1920", "ResolutionSizeY": "1080"}
        self.readonly = False
        return {**self.get(), "backup_path": "C:/fixture/backup.ini"}


def _ready_snapshot():
    return {
        "safe_to_skip_disable": True,
        "monitor_disable_status": api.ALL_PRESENT_PHYSICAL_MONITORS_DISABLED,
        "errors": [],
        "gpu": {"status": "ready", "name": "GPU"},
        "monitor": {"status": "ready", "name": "Monitor"},
        "resolution": {"status": "ready"},
        "refresh_rate": {"status": "ready", "value": 240},
        "gpus": [{"name": "NVIDIA"}],
    }


def _patch_ready_display(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(api, "collect_display_status", _ready_snapshot)
    monkeypatch.setattr(api, "enumerate_display_modes", lambda: {"modes": []})
    monkeypatch.setattr(api.display_mode_session, "recover_if_needed", lambda: None)
    monkeypatch.setattr(api.display_mode_session, "status", lambda: {})


def test_display_status_includes_cfg_snapshot(monkeypatch: pytest.MonkeyPatch, tmp_path: Path):
    service = _FakeCfgService()
    monkeypatch.setattr(api, "_game_user_settings", service)
    _patch_ready_display(monkeypatch)
    client = _client(monkeypatch, tmp_path)

    response = client.get("/api/valorant-lab/display/status")

    assert response.status_code == 200
    assert response.json()["cfg_status"]["current_resolution"] == {"width": 1920, "height": 1080}
    assert response.json()["cfg_status"]["can_unlock"] is False


def test_apply_syncs_cfg_then_locks_before_display_change(monkeypatch: pytest.MonkeyPatch, tmp_path: Path):
    service = _FakeCfgService()
    monkeypatch.setattr(api, "_game_user_settings", service)
    _patch_ready_display(monkeypatch)
    monkeypatch.setattr(
        api.display_mode_session,
        "apply",
        lambda width, height, refresh, timeout: {"applied": True, "width": width, "height": height},
    )
    client = _client(monkeypatch, tmp_path)

    response = client.post(
        "/api/valorant-lab/stretch/apply",
        json={"width": 1568, "height": 1080, "confirmed": True, "lock_cfg": True},
    )

    assert response.status_code == 200
    assert service.calls == [("set", 1568, 1080), ("lock",)]
    assert response.json()["cfg_status"]["state"] == "locked"
    assert response.json()["cfg_status"]["in_sync"] is True


def test_apply_refuses_running_valorant_before_display_change(monkeypatch: pytest.MonkeyPatch, tmp_path: Path):
    service = _FakeCfgService()
    service.set_error = GameUserSettingsProcessRunningError("VALORANT-Win64-Shipping.exe is running")
    display_apply = lambda *args: pytest.fail("display mode must not change while VALORANT is running")
    monkeypatch.setattr(api, "_game_user_settings", service)
    _patch_ready_display(monkeypatch)
    monkeypatch.setattr(api.display_mode_session, "apply", display_apply)
    client = _client(monkeypatch, tmp_path)

    response = client.post(
        "/api/valorant-lab/stretch/apply",
        json={"width": 1568, "height": 1080, "confirmed": True},
    )

    assert response.status_code == 409
    assert response.json()["detail"]["code"] == "valorant_running"


def test_display_apply_failure_restores_cfg_backup(monkeypatch: pytest.MonkeyPatch, tmp_path: Path):
    service = _FakeCfgService()
    monkeypatch.setattr(api, "_game_user_settings", service)
    _patch_ready_display(monkeypatch)
    monkeypatch.setattr(api.display_mode_session, "apply", lambda *args: (_ for _ in ()).throw(RuntimeError("display failed")))
    client = _client(monkeypatch, tmp_path)

    response = client.post(
        "/api/valorant-lab/stretch/apply",
        json={"width": 1568, "height": 1080, "confirmed": True},
    )

    assert response.status_code == 409
    assert service.calls == [("set", 1568, 1080), ("lock",), ("restore",)]


def test_cfg_unlock_and_restore_endpoints(monkeypatch: pytest.MonkeyPatch, tmp_path: Path):
    service = _FakeCfgService()
    service.readonly = True
    service.backup_available = True
    monkeypatch.setattr(api, "_game_user_settings", service)
    client = _client(monkeypatch, tmp_path)

    unlocked = client.post("/api/valorant-lab/stretch/cfg/unlock")
    restored = client.post("/api/valorant-lab/stretch/cfg/restore")

    assert unlocked.status_code == 200
    assert unlocked.json()["cfg_status"]["readonly"] is False
    assert restored.status_code == 200
    assert service.calls == [("unlock",), ("restore",)]
