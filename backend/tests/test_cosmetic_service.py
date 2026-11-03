"""Service and router tests with no native session or subprocess execution."""

import threading
import time
from pathlib import Path

import pytest

from app.cosmetic_studio import api
from app.cosmetic_studio import service as service_module
from app.cosmetic_studio.models import JobRequest, JobStatus, ToolPaths
from app.cosmetic_studio.provider import CosmeticError


def request(tmp_path: Path) -> JobRequest:
    return JobRequest(
        demo_path=r"C:\Demos\fixture.dem", xuid="76561199059074958",
        start_tick=256, end_tick=384, preset_id="awp-dragon-lore",
        tools=ToolPaths(
            cs2_exe=r"C:\Games\Steam\cs2.exe", hlae_exe=r"C:\Games\HLAE\hlae.exe",
            nskinz_dll=r"C:\Games\Tools\nskinz.dll", obs_exe=r"C:\Games\Tools\obs.exe",
            ffmpeg_exe=r"C:\Games\Tools\ffmpeg.exe", ffprobe_exe=r"C:\Games\Tools\ffprobe.exe",
            output_dir=str(tmp_path),
        ),
        acknowledge_offline=True,
    )


def preview_job(tmp_path, service):
    root = tmp_path / "job"
    (root / "evidence").mkdir(parents=True)
    preview = root / "evidence/preview.png"
    preview.write_bytes(b"mock-preview")
    job = service_module.Job(
        request(tmp_path),
        JobStatus(id="job1", state="preview", message="preview", preview_id="p" * 32),
        root, "claim-1",
    )
    job.preview_at = time.monotonic()
    job.preview_sha256 = service_module.digest(preview)
    service._jobs[job.status.id] = job
    return job


def test_preview_confirmation_rejects_wrong_id_replay_expiry_and_hash_change(tmp_path):
    service = service_module.CosmeticService()
    job = preview_job(tmp_path, service)
    with pytest.raises(CosmeticError):
        service.confirm(job.status.id, "wrong")
    with pytest.raises(CosmeticError):
        service.confirm(job.status.id, "p" * 31)
    service.confirm(job.status.id, "p" * 32)
    with pytest.raises(CosmeticError):
        service.confirm(job.status.id, "p" * 32)

    expired = preview_job(tmp_path / "expired", service)
    expired.preview_at = time.monotonic() - 181
    with pytest.raises(CosmeticError):
        service.confirm(expired.status.id, "p" * 32)

    changed = preview_job(tmp_path / "changed", service)
    (changed.root / "evidence/preview.png").write_bytes(b"changed")
    with pytest.raises(CosmeticError):
        service.confirm(changed.status.id, "p" * 32)


def test_cancel_marks_only_nonterminal_job(tmp_path):
    service = service_module.CosmeticService()
    job = preview_job(tmp_path, service)
    result = service.cancel_job(job.status.id)
    assert job.cancel.is_set()
    assert result.message.startswith("正在停止")
    job.status = JobStatus(id="job1", state="failed", message="done")
    service.cancel_job(job.status.id)
    assert job.cancel.is_set()


def test_cross_model_service_profile_resolves_original_definition_and_explicit_target(monkeypatch, tmp_path):
    from types import SimpleNamespace
    from app.cosmetic_studio import core_inspection

    calls = []
    monkeypatch.setattr(core_inspection, 'resolve_owned_weapon',
        lambda path, xuid, tick, definition: calls.append((path, xuid, tick, definition)) or '48742431476')
    request = SimpleNamespace(demo_path=tmp_path / 'source.dem', xuid='76561199369875913',
        start_tick=67431, preset_id='karambit-to-butterfly-fade', pattern_seed=0,
        wear=.01, glove_preset_id=None)

    profile = service_module._build_core_profile(request)

    assert calls == [(request.demo_path, request.xuid, 67367, 507)]
    item = profile.items[0]
    assert (item.item_id64, item.definition_index, item.replacement_definition_index,
            item.paint_kit, item.pattern_seed, item.wear) == ('48742431476', 507, 515, 38, 0, .01)


def test_cleanup_failure_keeps_runtime_claim_and_failed_state(tmp_path, monkeypatch):
    service = service_module.CosmeticService()
    root = tmp_path / "job"
    root.mkdir()
    job = service_module.Job(
        request(tmp_path), JobStatus(id="job2", state="preparing", message="start"),
        root, "claim-keep",
    )
    service._jobs[job.status.id] = job
    released = []
    monkeypatch.setattr(service_module, "release_runtime_session", lambda claim: released.append(claim))
    monkeypatch.setattr("app.cosmetic_studio.inspection.resolve_slot", lambda *args: 4)

    class FakeSession:
        baseline = {"profile_sha256": "profile"}
        def __init__(self): self.stopped = False
        def prepare(self): pass
        def check_cancel(self): pass
        def launch(self): raise CosmeticError("launch failed")
        def stop(self): self.stopped = True; raise CosmeticError("cleanup unknown")

    class FakeRecorder:
        def __init__(self, session): pass
        def prepare(self): pass
        def close(self): pass

    monkeypatch.setattr(service, "session_factory", lambda *args: FakeSession())
    monkeypatch.setattr(service, "recorder_factory", FakeRecorder)
    service._run(job)
    assert job.status.state == "failed"
    assert job.cleanup_verified is False
    assert released == []
    assert "清理未确认" in job.status.message


def test_shutdown_reports_false_when_cleanup_is_unknown(tmp_path):
    service = service_module.CosmeticService()
    job = preview_job(tmp_path, service)
    job.thread = threading.Thread(target=lambda: None)
    job.thread.start()
    job.cleanup_verified = False
    assert service.shutdown(timeout=0) is False


def test_capabilities_router_response_shape_without_startup(monkeypatch, tmp_path):
    config_path = tmp_path / "missing-config.json"
    monkeypatch.setattr("app.env_utils.resolve_config_path", lambda: config_path)
    result = api.capabilities()
    payload = result.model_dump()
    assert set(payload) == {"enabled", "reason", "defaults", "presets", "providers", "glove_presets"}
    assert payload["enabled"] is True
    assert payload["presets"][0]["id"] == "awp-dragon-lore"
    assert {p["id"] for p in payload["providers"]} == {"skin_core"}
    assert payload["presets"][1]["provider_ids"] == ["skin_core"]
    assert payload["defaults"]["cs2_exe"] == ""
    assert len(payload["glove_presets"]) == 1
    assert payload["glove_presets"][0]["id"] == "specialist-10061"
    assert payload["glove_presets"][0]["weapon_ids"] == [507]


def test_export_rejects_blackdetect_before_litecut_or_ready_material(tmp_path, monkeypatch):
    service = service_module.CosmeticService(db_path=tmp_path / "cosmetic.db")
    job = preview_job(tmp_path, service)
    source = job.root / "evidence/cosmetic.mkv"
    source.write_bytes(b"mock-video")
    calls = []

    class FakeSession:
        env = {}
        demo_hash = "d" * 64
        paths = {"ffmpeg_exe": Path(r"C:\Tools\ffmpeg.exe"),
                 "ffprobe_exe": Path(r"C:\Tools\ffprobe.exe")}

        def check_cancel(self):
            calls.append("check_cancel")

    def copy_tool(src, dst):
        Path(dst).write_bytes(b"pinned-tool")
        return dst

    monkeypatch.setattr(service_module.shutil, "copy2", copy_tool)
    monkeypatch.setattr(service_module, "digest", lambda path: service_module.PINS[
        "ffmpeg_exe" if Path(path).name == "ffmpeg.exe" else "ffprobe_exe"
    ])

    class Completed:
        stdout = b'{"format":{"duration":"2.5"},"streams":[{"codec_type":"video","width":1280,"height":720}]}'
        stderr = b"[blackdetect] black_start:0.021 black_end:0.821"

    def run(args, **kwargs):
        calls.append(tuple(args))
        return Completed()

    monkeypatch.setattr(service_module.subprocess, "run", run)
    monkeypatch.setattr("app.lite_cut.composer.export_lite_cut_project",
                        lambda **kwargs: (_ for _ in ()).throw(AssertionError("LiteCut must not run")))

    async def store_material(*args, **kwargs):
        raise AssertionError("ready material must not be stored")

    monkeypatch.setattr(service, "store_material", store_material)
    with pytest.raises(CosmeticError, match="黑屏帧"):
        service.export(job, FakeSession(), source)
    assert len(calls) == 3
    assert not (job.root / "cosmetic-litecut.mp4").exists()


def test_final_litecut_missing_audio_is_rejected_before_material_commit(tmp_path, monkeypatch):
    import json
    from types import SimpleNamespace
    service=service_module.CosmeticService(db_path=tmp_path/'cosmetic.db')
    job=preview_job(tmp_path,service)
    source=job.root/'evidence/cosmetic.mkv';source.write_bytes(b'mock')
    class FakeSession:
        env={};demo_hash='d'*64
        paths={'ffmpeg_exe':Path(r'C:\Tools\ffmpeg.exe'),'ffprobe_exe':Path(r'C:\Tools\ffprobe.exe')}
        def check_cancel(self):pass
    def copy_tool(src,dst):Path(dst).write_bytes(b'tool')
    monkeypatch.setattr(service_module.shutil,'copy2',copy_tool)
    monkeypatch.setattr(service_module,'digest',lambda path:service_module.PINS[
        'ffmpeg_exe' if Path(path).name=='ffmpeg.exe' else 'ffprobe_exe'])
    probes=[]
    def run(args,**kwargs):
        if '-show_streams' in args:
            probes.append(args[-1])
            streams=[{'codec_type':'video','width':1280,'height':720}]+([{'codec_type':'audio'}] if len(probes)==1 else [])
            return SimpleNamespace(stdout=json.dumps({'format':{'duration':'2.5'},'streams':streams}).encode(),stderr=b'')
        return SimpleNamespace(stdout=b'',stderr=b'mean_volume: -19.4 dB\nmax_volume: -0.6 dB')
    monkeypatch.setattr(service_module.subprocess,'run',run)
    def compose(**kwargs):
        target=Path(kwargs['output_path_str']);target.write_bytes(b'mock');return target
    monkeypatch.setattr('app.lite_cut.composer.export_lite_cut_project',compose)
    async def store(*args):pytest.fail('Invalid final video must never enter materials')
    monkeypatch.setattr(service,'store_material',store)
    with pytest.raises(CosmeticError,match='缺少游戏音轨'):
        service.export(job,FakeSession(),source)
    assert len(probes)==2
