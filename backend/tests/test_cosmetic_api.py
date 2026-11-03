"""Router-only API tests; no main lifespan, native session, or subprocesses."""

import time
from pathlib import Path

import pytest
from fastapi import FastAPI
from fastapi.testclient import TestClient

from app.cosmetic_studio import api, service as service_module
from app.cosmetic_studio.models import JobRequest, JobStatus, ToolPaths
from app.cosmetic_studio.provider import CosmeticError
from app import session_auth
from app import runtime_session


TOKEN = "unit-test-desktop-token"


@pytest.fixture()
def client(monkeypatch):
    monkeypatch.setattr(session_auth, "_SESSION_TOKEN", TOKEN)
    app = FastAPI()
    app.include_router(api.router)
    with TestClient(app) as value:
        yield value


def payload(tmp_path: Path, *, ack=True):
    return {
        "demo_path": r"C:\Demos\fixture.dem", "xuid": "76561199059074958",
        "start_tick": 256, "end_tick": 384, "preset_id": "awp-dragon-lore",
        "acknowledge_offline": ack,
        "expected_demo_sha256": "a" * 64,
        "tools": {
            "cs2_exe": r"C:\Games\Steam\cs2.exe", "hlae_exe": r"C:\Games\HLAE\hlae.exe",
            "nskinz_dll": r"C:\Games\Tools\nskinz.dll", "obs_exe": r"C:\Games\Tools\obs.exe",
            "ffmpeg_exe": r"C:\Games\Tools\ffmpeg.exe", "ffprobe_exe": r"C:\Games\Tools\ffprobe.exe",
            "output_dir": str(tmp_path),
        },
    }


def headers():
    return {session_auth.SESSION_TOKEN_HEADER: TOKEN}


def test_manual_knife_validation_is_authenticated_fingerprint_bound_and_read_only(client, monkeypatch):
    from app.cosmetic_studio.models import Candidate
    from app.cosmetic_studio import intervals
    body = {'demo_path': r'D:\Demos\match.dem', 'expected_demo_sha256': 'a'*64,
            'xuid': '76561199369875913', 'start_tick': 67431, 'end_tick': 67687, 'weapon_id': 507}
    monkeypatch.setattr(api.service, 'start', lambda _: pytest.fail('validation must not launch'))
    monkeypatch.setattr(api, 'digest', lambda _: 'a'*64, raising=False)
    monkeypatch.setattr(intervals, 'validate_interval', lambda *args: Candidate(
        xuid=body['xuid'], start_tick=67431, end_tick=67687, weapon_id=507, label='Knife'))
    assert client.post('/api/cosmetic-studio/interval', json=body).status_code == 401
    response = client.post('/api/cosmetic-studio/interval', json=body, headers=headers())
    assert response.status_code == 200
    assert response.json()['weapon_id'] == 507
    monkeypatch.setattr(api, 'digest', lambda _: 'b'*64)
    assert client.post('/api/cosmetic-studio/interval', json=body, headers=headers()).status_code == 400


def test_manual_knife_validation_rejects_demo_change_during_parse(client, monkeypatch):
    from app.cosmetic_studio.models import Candidate
    from app.cosmetic_studio import intervals
    body = {'demo_path': r'D:\Demos\match.dem', 'expected_demo_sha256': 'a'*64,
            'xuid': '76561199369875913', 'start_tick': 67431, 'end_tick': 67687, 'weapon_id': 507}
    values=iter(['a'*64, 'b'*64])
    monkeypatch.setattr(api, 'digest', lambda _: next(values), raising=False)
    monkeypatch.setattr(intervals, 'validate_interval', lambda *args: Candidate(
        xuid=body['xuid'], start_tick=67431, end_tick=67687, weapon_id=507, label='Knife'))
    assert client.post('/api/cosmetic-studio/interval', json=body, headers=headers()).status_code == 400


def test_authenticated_job_cannot_omit_inspected_source_hash(client, monkeypatch, tmp_path):
    body=payload(tmp_path);body.pop('expected_demo_sha256')
    monkeypatch.setattr(api.service,'start',lambda request: pytest.fail('Missing fingerprint must fail before job start'))
    response=client.post('/api/cosmetic-studio/jobs',json=body,headers=headers())
    assert response.status_code==400


def test_api_rejects_glove_request_when_server_qualification_gate_is_closed(client, monkeypatch, tmp_path):
    body=payload(tmp_path)
    body.update(provider_id='skin_core',preset_id='karambit-fade',glove_preset_id='specialist-10061')
    body['tools']['core_exe']=r'D:\Tools\skin-core.exe'
    monkeypatch.setattr(api,'GLOVE_PRESET_DATA',{},raising=False)
    monkeypatch.setattr(api.service,'start',lambda _: pytest.fail('Unqualified combination must not launch'))
    assert client.post('/api/cosmetic-studio/jobs',json=body,headers=headers()).status_code==400


def test_profiles_round_trip_requires_desktop_and_never_starts_job(client, monkeypatch, tmp_path):
    from app.cosmetic_studio.profiles import ProfileStore
    monkeypatch.setattr(api, 'profile_store', lambda: ProfileStore(tmp_path / 'profiles.json'), raising=False)
    monkeypatch.setattr(api.service, 'start', lambda request: pytest.fail('profile config must not launch a job'))
    body = {'name': 'My AK', 'xuid': '76561199034001385', 'preset_id': 'ak47-fire-serpent',
            'demo_sha256': 'a'*64, 'start_tick': 1000, 'end_tick': 1128}
    assert client.post('/api/cosmetic-studio/profiles', json=body).status_code == 401
    saved = client.post('/api/cosmetic-studio/profiles', json=body, headers=headers())
    assert saved.status_code == 201
    profile_id = saved.json()['id']
    assert client.get('/api/cosmetic-studio/profiles', headers=headers()).json()[0]['xuid'] == body['xuid']
    assert client.post('/api/cosmetic-studio/profiles', json={**body, 'tools': {'secret':'no'}}, headers=headers()).status_code == 400
    assert client.delete('/api/cosmetic-studio/profiles/'+profile_id, headers=headers()).status_code == 204
    assert client.get('/api/cosmetic-studio/profiles', headers=headers()).json() == []


def test_auth_requires_enabled_session_header_and_rejects_query_token(client, monkeypatch):
    monkeypatch.setattr(session_auth, "_SESSION_TOKEN", "")
    assert client.get("/api/cosmetic-studio/capabilities").status_code == 401
    monkeypatch.setattr(session_auth, "_SESSION_TOKEN", TOKEN)
    assert client.get("/api/cosmetic-studio/capabilities").status_code == 401
    assert client.get("/api/cosmetic-studio/capabilities", params={"_session": TOKEN}).status_code == 401
    assert client.get("/api/cosmetic-studio/capabilities", headers=headers()).status_code == 200


@pytest.mark.parametrize(("raw", "expected"), [
    (r"\\?\D:\MgsCsTestApp\tools\skin-core.exe", r"D:\MgsCsTestApp\tools\skin-core.exe"),
    (r"\\server\share\skin-core.exe", r"\\server\share\skin-core.exe"),
    (r"\\?\UNC\server\share\skin-core.exe", r"\\?\UNC\server\share\skin-core.exe"),
])
def test_capability_path_normalizes_only_local_extended_path(raw, expected):
    assert api._capability_path(raw) == expected


def test_capabilities_exposes_normalized_bundled_core_path(monkeypatch):
    monkeypatch.setattr(api.sys, "executable", r"\\?\D:\MgsCsTestApp\python\python.exe")
    monkeypatch.setattr(Path, "is_file", lambda value: str(value).lower().endswith("skin-core.exe"))
    result = api.capabilities().model_dump()
    assert result["defaults"]["core_exe"] == r"D:\MgsCsTestApp\tools\skin-core.exe"


@pytest.mark.parametrize("ack", [1, 1.0, "true", "True", False])
def test_create_rejects_non_boolean_true_acknowledgement(client, tmp_path, ack):
    response = client.post("/api/cosmetic-studio/jobs", json=payload(tmp_path, ack=ack), headers=headers())
    assert response.status_code == 422


def test_create_rejects_path_injection(client, tmp_path):
    body = payload(tmp_path)
    body["tools"]["cs2_exe"] = 'C:\\Games\\x\";Start-Process evil.exe'
    assert client.post("/api/cosmetic-studio/jobs", json=body, headers=headers()).status_code == 422


def test_active_null_and_preview_wrong_id_404_or_409(client, tmp_path):
    service_module.service._jobs.clear()
    assert client.get("/api/cosmetic-studio/active", headers=headers()).json() is None
    assert client.get("/api/cosmetic-studio/jobs/missing/preview/" + "a" * 32, headers=headers()).status_code == 404
    root = tmp_path / "job"
    (root / "evidence").mkdir(parents=True)
    (root / "evidence/preview.png").write_bytes(b"preview")
    job = service_module.Job(JobRequest.model_validate(payload(tmp_path)), JobStatus(id="known", state="preview", message="preview", preview_id="b" * 32), root, "claim")
    job.preview_at = time.monotonic()
    job.preview_sha256 = service_module.digest(root / "evidence/preview.png")
    service_module.service._jobs["known"] = job
    assert client.get("/api/cosmetic-studio/jobs/known/preview/" + "c" * 32, headers=headers()).status_code == 409


def test_inspection_response_serializes_mock_parser(client, monkeypatch, tmp_path):
    monkeypatch.setattr("app.cosmetic_studio.inspection.inspect_demo", lambda path: {
        "players": [{"xuid": "76561199059074958", "name": "Player"}],
        "candidates": [{"xuid": "76561199059074958", "start_tick": 256, "end_tick": 384, "label": "clip"}],
        "demo_sha256": "a" * 64,
    })
    response = client.post("/api/cosmetic-studio/inspect", json={"demo_path": r"C:\Demos\fixture.dem"}, headers=headers())
    assert response.status_code == 200
    assert response.json()["players"][0]["xuid"] == "76561199059074958"
    assert response.json()["candidates"][0]["end_tick"] == 384


def test_create_busy_claim_is_conflict_not_bad_request(client, monkeypatch, tmp_path):
    monkeypatch.setattr(service_module.native, "final_path", lambda path: path)
    monkeypatch.setattr(service_module.shutil, "disk_usage", lambda path: type("Usage", (), {"free": 3 * 2**30})())
    monkeypatch.setenv("SystemDrive", "Z:")
    claim = runtime_session.claim_runtime_session("/api/other-operation")
    try:
        response = client.post("/api/cosmetic-studio/jobs", json=payload(tmp_path), headers=headers())
        assert response.status_code == 409
        assert response.json()["detail"]["code"] == "RUNTIME_SESSION_BUSY"
    finally:
        runtime_session.release_runtime_session(claim)
