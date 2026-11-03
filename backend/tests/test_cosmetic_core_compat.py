"""Compatibility contract tests for the explicitly licensed core wrapper.

These tests use a transport double only; the third-party executable is never
started and no real core protocol is exercised.
"""
import hashlib
import os
import threading
from pathlib import Path

import pytest

from app.cosmetic_studio import service, skin_core
from app.cosmetic_studio.provider import CosmeticError


def test_unqualified_core_is_rejected_before_transport(tmp_path, monkeypatch):
    executable = tmp_path / "skin-core.exe"
    executable.write_bytes(b"fake core")
    monkeypatch.setattr(skin_core, "QUALIFIED_CORE_SHA256", "0" * 64)
    called = []
    monkeypatch.setattr(skin_core, "run_core", lambda *args: called.append(args))

    with pytest.raises(CosmeticError, match="CORE_VERSION_UNQUALIFIED"):
        skin_core.run_licensed_core(executable, ["rewrite-owned-batch"], b"packet", tmp_path,
                                    {"PATH": "safe"}, threading.Event())
    assert called == []


def test_licensed_wrapper_scopes_dev_flag_and_preserves_call_contract(tmp_path, monkeypatch):
    executable = tmp_path / "skin-core.exe"
    executable.write_bytes(b"qualified fake core")
    monkeypatch.setattr(skin_core, "QUALIFIED_CORE_SHA256",
                        hashlib.sha256(executable.read_bytes()).hexdigest())
    cancel = threading.Event()
    args, packet, root, env = ["--input", "in.dem"], b"authenticated", tmp_path, {"PATH": "safe"}
    captured = {}

    def transport(exe, child_args, child_packet, child_root, child_env, child_cancel):
        captured.update(exe=exe, args=child_args, packet=child_packet, root=child_root,
                        env=child_env, cancel=child_cancel)
        return 0, "ok"

    monkeypatch.setattr(skin_core, "run_core", transport)
    monkeypatch.setenv("CS2_SKIN_CORE_DEV", "parent-must-not-be-used")
    monkeypatch.setenv("OPENAI_API_KEY", "secret-must-not-pass")
    assert skin_core.run_licensed_core(executable, args, packet, root, env, cancel) == (0, "ok")
    assert captured["exe"] == executable
    assert captured["args"] is args
    assert captured["packet"] is packet
    assert captured["root"] is root
    assert captured["cancel"] is cancel
    assert captured["env"]["CS2_SKIN_CORE_DEV"] == "1"
    assert "OPENAI_API_KEY" not in captured["env"]
    assert os.environ["CS2_SKIN_CORE_DEV"] == "parent-must-not-be-used"


def test_service_uses_licensed_wrapper_by_default(tmp_path, monkeypatch):
    original = tmp_path / "original.dem"
    original.write_bytes(b"PBDEMS2\\x00original")
    root = tmp_path / "job"
    root.mkdir()
    request = service.JobRequest.model_validate({
        "demo_path": str(original), "xuid": "76561199059074958", "start_tick": 256,
        "end_tick": 384, "provider_id": "skin_core", "preset_id": "awp-dragon-lore",
            "tools": {"cs2_exe": "C:/tools/cs2.exe", "hlae_exe": "C:/tools/hlae.exe", "core_exe": "D:/tools/core.exe",
                       "obs_exe": "D:/tools/obs.exe", "ffmpeg_exe": "D:/tools/ffmpeg.exe", "ffprobe_exe": "D:/tools/ffprobe.exe",
                   "output_dir": str(tmp_path)}, "acknowledge_offline": True,
    })
    job = service.Job(request, service.JobStatus(id="compat", state="preparing", message="start"), root, "claim")
    captured = {}
    sentinel = object()

    monkeypatch.setattr("app.cosmetic_studio.inspection.resolve_slot", lambda *args: 4)
    monkeypatch.setattr("app.cosmetic_studio.core_inspection.resolve_owned_awp", lambda *args: "123456789")
    monkeypatch.setattr(service, "release_runtime_session", lambda _claim: None)
    monkeypatch.setattr(skin_core, "run_licensed_core", sentinel)

    class CaptureProvider:
        def __init__(self, *args, **kwargs):
            captured["transport"] = kwargs["transport"]
            raise CosmeticError("compatibility probe")

    monkeypatch.setattr(skin_core, "SkinCoreProvider", CaptureProvider)
    service.CosmeticService()._run(job)
    assert captured["transport"] is sentinel
    assert job.status.state == "failed"
