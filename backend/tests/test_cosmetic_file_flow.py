"""Pure service/runtime seam tests; no native process or DLL is started."""

from pathlib import Path
import threading

import pytest

from app.cosmetic_studio import service as sm
from app.cosmetic_studio import runtime
from app.cosmetic_studio.models import JobRequest, JobStatus, ToolPaths
from app.cosmetic_studio.provider import CosmeticError


XUID = "76561199059074958"


def make_request(tmp_path, provider_id="skin_core", preset_id="awp-dragon-lore"):
    return JobRequest(
        demo_path=str(tmp_path / "original.dem"), xuid=XUID, start_tick=256, end_tick=384,
        provider_id=provider_id, preset_id=preset_id, wear=.18 if preset_id == 'awp-asiimov' else .01,
        tools=ToolPaths(cs2_exe=r"C:\Games\Steam\cs2.exe", hlae_exe=r"C:\Games\HLAE\hlae.exe",
                        nskinz_dll=r"C:\Tools\nskinz.dll", core_exe=r"D:\Tools\skin-core.exe",
                        obs_exe=r"C:\Tools\obs.exe", ffmpeg_exe=r"D:\Tools\ffmpeg.exe",
                        ffprobe_exe=r"D:\Tools\ffprobe.exe", output_dir=str(tmp_path)),
        acknowledge_offline=True)


class FakeSession:
    instances = []
    def __init__(self, request, root, slot, cancel):
        self.request, self.root, self.demo = request, root, Path(request.demo_path)
        self.baseline = {"profile_sha256": "legacy"}
        self.demo_hash = sm.digest(self.demo) if self.demo.is_file() else "playback-hash"
        self._restored = False
        self.launched = False
        self.file_prepare = None
        FakeSession.instances.append(self)
    def prepare(self): pass
    def check_cancel(self): pass
    def launch(self): self.launched = True
    def prepare_file_playback(self, digest): self.file_prepare = digest
    def validate_context(self, **kwargs): return True
    def stop(self): self._restored = True


class FakeRecorder:
    launches = 0
    def __init__(self, session): self.source = session.root / "capture.mkv"
    def prepare(self): pass
    def launch(self): FakeRecorder.launches += 1
    def record(self): self.source.write_bytes(b"capture"); return self.source
    def close(self): pass


def run_job(tmp_path, monkeypatch, request, provider_cls, *, approve=True):
    FakeSession.instances.clear()
    FakeRecorder.launches = 0
    original = Path(request.demo_path)
    original.write_bytes(b"PBDEMS2\x00original")
    root = tmp_path / "job"
    root.mkdir()
    job = sm.Job(request, JobStatus(id="job", state="preparing", message="start"), root, "claim")
    if approve:
        job.approved.set()
    service = sm.CosmeticService(session_factory=FakeSession, recorder_factory=FakeRecorder)
    monkeypatch.setattr("app.cosmetic_studio.inspection.resolve_slot", lambda *a: 4)
    monkeypatch.setattr("app.cosmetic_studio.core_inspection.resolve_owned_awp", lambda *a: "123456789")
    monkeypatch.setattr(sm, "release_runtime_session", lambda claim: None)
    monkeypatch.setattr(sm, "capture_preview", lambda session, path: (path.parent.mkdir(exist_ok=True), path.write_bytes(b"preview")))
    monkeypatch.setattr(sm, "NskinzProvider", provider_cls)
    monkeypatch.setattr(service, "export", lambda *a: {"output_path": "mock.mp4", "litecut_project_id": 1})
    return service, job


def test_skin_core_uses_copy_for_playback_and_never_nskinz(tmp_path, monkeypatch):
    class Core:
        instances = []
        def __init__(self, original, root, profile, executable, python, cancel, **kw):
            self.original, self.root, self.profile = original, root, profile
            self.artifact = None; self.preview_check = None; self.stop_playback = None; self.state = "new"
            Core.instances.append(self)
        def load(self): self.state = "loaded"
        def apply_profile(self, digest):
            playback = self.root / "playback.dem"; playback.parent.mkdir(parents=True); playback.write_bytes(b"PBDEMS2\\x00copy")
            self.artifact = type("A", (), {"original": self.original, "playback": playback,
                "original_sha256": sm.digest(self.original), "playback_sha256": sm.digest(playback)})()
            self.state = "pending_visual_review"
        def confirm_preview(self): self.state = "human_confirmed"
        def check_ready(self): return self.preview_check is not None and self.preview_check(self.artifact) is True
        def unload(self): self.state = "stopped"
    class ForbiddenNskinz:
        def __init__(self, *a, **k): raise AssertionError("nSkinz must not be selected")
    monkeypatch.setattr("app.cosmetic_studio.skin_core.SkinCoreProvider", Core)
    service, job = run_job(tmp_path, monkeypatch, make_request(tmp_path), ForbiddenNskinz)
    service._run(job)
    assert job.status.state == "complete"
    assert FakeSession.instances[-1].request.demo_path != job.request.demo_path
    assert FakeSession.instances[-1].file_prepare
    assert Core.instances[-1].original == Path(job.request.demo_path)


def test_core_failure_prevents_game_launch(tmp_path, monkeypatch):
    class Failing:
        def __init__(self, *a, **k): pass
        def load(self): raise CosmeticError("core rejected")
    monkeypatch.setattr("app.cosmetic_studio.skin_core.SkinCoreProvider", Failing)
    service, job = run_job(tmp_path, monkeypatch, make_request(tmp_path), object)
    service._run(job)
    assert job.status.state == "failed"
    assert not FakeSession.instances or not FakeSession.instances[-1].launched
    assert "core rejected" in job.status.message


def test_visual_confirmation_is_mandatory_for_file_provider(tmp_path, monkeypatch):
    class Unconfirmed:
        def __init__(self, original, root, profile, executable, python, cancel, **kw):
            self.original, self.root, self.preview_check, self.stop_playback = original, root, None, None
            playback = root / "playback.dem"
            playback.parent.mkdir(parents=True, exist_ok=True)
            playback.write_bytes(b"PBDEMS2\x00copy")
            self.artifact = type("A", (), {"original": original, "playback": playback,
                "original_sha256": sm.digest(original), "playback_sha256": sm.digest(playback)})()
        def load(self): pass
        def apply_profile(self, digest): pass
        def confirm_preview(self): raise CosmeticError("visual review rejected")
        def check_ready(self): return False
        def unload(self): pass
    monkeypatch.setattr("app.cosmetic_studio.skin_core.SkinCoreProvider", Unconfirmed)
    service, job = run_job(tmp_path, monkeypatch, make_request(tmp_path), object)
    service._run(job)
    assert job.status.state == "failed"
    assert FakeRecorder.launches == 0
    assert "visual review rejected" in job.status.message


def test_nskinz_request_keeps_original_demo_and_legacy_provider(tmp_path, monkeypatch):
    class Legacy:
        def __init__(self, session): self.session = session
        def load(self): pass
        def apply_profile(self, digest): pass
        def confirm_preview(self): pass
        def check_ready(self): return True
        def unload(self): pass
    service, job = run_job(tmp_path, monkeypatch, make_request(tmp_path, "nskinz"), Legacy)
    service._run(job)
    assert job.status.state == "complete"
    assert FakeSession.instances[-1].request.demo_path == job.request.demo_path


def test_file_mode_has_no_dll_and_rejects_extension_load(tmp_path):
    req = make_request(tmp_path, "skin_core")
    session = runtime.NativeSession(req, tmp_path / "job", 4, threading.Event())
    assert session.file_mode is True
    assert session.dll is None
    assert "nskinz_dll" not in session.paths
    with pytest.raises(CosmeticError, match="禁止加载换肤 DLL"):
        session.load_extension()


def test_provider_specific_presets_and_tools_are_strict():
    core = make_request(Path("D:/MgsCsTests/test"), "skin_core", "awp-asiimov")
    assert core.provider_id == "skin_core" and core.preset_id == "awp-asiimov"
    with pytest.raises(ValueError):
        JobRequest.model_validate({**core.model_dump(), "tools": {}})


def test_core_unknown_cleanup_retains_claim_before_any_game_launch(tmp_path, monkeypatch):
    from app.cosmetic_studio.owned_process import CoreCleanupError
    class Unknown:
        def __init__(self, *a, **k): pass
        def load(self): raise CoreCleanupError("CORE_CHILDREN_EXIT_UNCONFIRMED")
    monkeypatch.setattr("app.cosmetic_studio.skin_core.SkinCoreProvider", Unknown)
    service, job = run_job(tmp_path, monkeypatch, make_request(tmp_path), object)
    released = []
    monkeypatch.setattr(sm, "release_runtime_session", released.append)
    service._run(job)
    assert job.status.state == "failed" and job.cleanup_verified is False
    assert released == [] and FakeSession.instances == []
