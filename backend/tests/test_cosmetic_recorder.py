"""Mock-only recorder status guards; no OBS process is started."""

import json
from pathlib import Path

import pytest

from app.cosmetic_studio import recorder


class FakeSession:
    def __init__(self, root: Path):
        self.root = root


def test_recording_audio_is_bound_to_cs2_window_not_desktop_or_microphone(tmp_path):
    scene = recorder.scene_config(tmp_path / "control.lua")
    captures = [source for source in scene["sources"] if source["id"] != "scene"]
    assert len(captures) == 1
    source = captures[0]
    assert source["id"] == "window_capture"
    assert source["settings"]["window"] == "Counter-Strike 2:SDL_app:cs2.exe"
    assert source["settings"]["capture_audio"] is True
    assert source["volume"] > 0 and source["mixers"] & 1
    assert not any(source["id"] in {"wasapi_input_capture", "wasapi_output_capture"}
                   for source in scene["sources"])


def make_recorder(tmp_path, monkeypatch, now=100.0):
    root = tmp_path / "session"
    (root / "evidence").mkdir(parents=True)
    (root / "requests").mkdir()
    monkeypatch.setattr(recorder.time, "time", lambda: now)
    return recorder.IsolatedRecorder(FakeSession(root))


def write_status_log(rec, *lines, trailing_newline=True):
    payload = "\n".join(lines)
    if trailing_newline:
        payload += "\n"
    (rec.root / "evidence/obs-status.jsonl").write_text(payload, encoding="utf-8")


def status_line(**overrides):
    value = {"time": 100.0, "active": True, "requested": True, "stopped": False}
    value.update(overrides)
    return json.dumps(value, separators=(",", ":"))


def test_status_does_not_fall_back_to_legacy_single_json_path(tmp_path, monkeypatch):
    rec = make_recorder(tmp_path, monkeypatch)
    (rec.root / "evidence/obs-status.json").write_text(status_line(), encoding="utf-8")
    assert rec.status() is None


def test_status_uses_fresh_complete_line_and_ignores_truncated_tail(tmp_path, monkeypatch):
    rec = make_recorder(tmp_path, monkeypatch)
    write_status_log(rec, status_line(), '{"time":100,"active":true', trailing_newline=False)
    assert rec.status()["active"] is True


@pytest.mark.parametrize(
    "tail",
    [
        "not-json",
        status_line(time=97.0),
        status_line(time=101.0),
    ],
)
def test_status_never_falls_back_from_malformed_stale_or_future_complete_line(tmp_path, monkeypatch, tail):
    rec = make_recorder(tmp_path, monkeypatch)
    write_status_log(rec, status_line(), tail)
    assert rec.status() is None


@pytest.mark.parametrize(
    "field,value",
    [
        ("active", 1),
        ("requested", "true"),
        ("stopped", 0),
        ("time", float("nan")),
        ("time", float("inf")),
    ],
)
def test_status_requires_strict_booleans_and_finite_time(tmp_path, monkeypatch, field, value):
    rec = make_recorder(tmp_path, monkeypatch)
    write_status_log(rec, status_line(**{field: value}))
    assert rec.status() is None


def test_status_rejects_valid_record_when_native_owner_is_dead(tmp_path, monkeypatch):
    rec = make_recorder(tmp_path, monkeypatch)
    rec.handle = object()
    monkeypatch.setattr(recorder.native, "is_alive", lambda handle: False)
    write_status_log(rec, status_line())
    with pytest.raises(recorder.CosmeticError, match="录制进程意外退出"):
        rec.status()


def test_capture_ready_waits_for_known_source_size_and_fresh_settle(tmp_path, monkeypatch):
    rec = make_recorder(tmp_path, monkeypatch)
    samples = iter([
        {"time": 100, "width": 0, "height": 0},
        {"time": 100, "width": 1280, "height": 720},
        {"time": 101, "width": 1280, "height": 720},
        {"time": 102, "width": 1280, "height": 720},
    ])
    monkeypatch.setattr(rec, "status", lambda: next(samples))
    checks = []
    rec.session.validate_context = lambda **kwargs: checks.append(kwargs)
    def wait_for(predicate, timeout):
        assert predicate() is False
        assert predicate() is False
        assert predicate() is False
        assert predicate() is True
    rec.session.wait_for = wait_for
    rec.wait_capture_ready()
    assert len(checks) == 4
