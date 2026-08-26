from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest
from fastapi import FastAPI
from fastapi.testclient import TestClient

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from app.valorant_lab import api


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
