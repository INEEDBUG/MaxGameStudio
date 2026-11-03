from fastapi import FastAPI
from fastapi.testclient import TestClient

from app import session_auth
from app.cosmetic_studio import api, appearance_api
from app.cosmetic_studio.appearance import AppearanceService
from app.cosmetic_studio.appearance_store import AppearanceStore


TOKEN = "appearance-api-test-token"


def _client(monkeypatch, tmp_path):
    monkeypatch.setattr(session_auth, "_SESSION_TOKEN", TOKEN)
    appearance_api.service = AppearanceService(AppearanceStore(tmp_path / "appearance.json"))
    app = FastAPI()
    app.include_router(api.router)
    return TestClient(app)


def test_appearance_routes_inherit_desktop_auth_and_settings_persist_without_launch(monkeypatch, tmp_path):
    client = _client(monkeypatch, tmp_path)
    settings = {"core_exe": "D:/Tools/skin-core.exe", "output_dir": "D:/Data/playbacks"}

    assert client.post("/api/cosmetic-studio/appearance/inventory", json={"demo_path": "D:/demo.dem"}).status_code == 401
    assert client.put("/api/cosmetic-studio/appearance/settings", json=settings).status_code == 401
    response = client.put("/api/cosmetic-studio/appearance/settings", json=settings,
        headers={session_auth.SESSION_TOKEN_HEADER: TOKEN})

    assert response.status_code == 200
    assert response.json() == settings
    assert (tmp_path / "appearance.json").is_file()


def test_appearance_settings_reject_extra_fields_and_nonlocal_paths(monkeypatch, tmp_path):
    client = _client(monkeypatch, tmp_path)
    headers = {session_auth.SESSION_TOKEN_HEADER: TOKEN}

    assert client.put("/api/cosmetic-studio/appearance/settings",
        json={"core_exe": "relative.exe", "output_dir": "D:/Data"}, headers=headers).status_code == 422
    assert client.put("/api/cosmetic-studio/appearance/settings",
        json={"core_exe": "D:/Tools/core.exe", "output_dir": "D:/Data", "token": "x"}, headers=headers).status_code == 422


def test_verified_playback_is_separate_authenticated_acknowledged_action(monkeypatch, tmp_path):
    client = _client(monkeypatch, tmp_path)
    headers = {session_auth.SESSION_TOKEN_HEADER: TOKEN}
    called = []
    monkeypatch.setattr(appearance_api.service, 'status', lambda job: type('Status', (), {
        'state': 'file_verified', 'artifact': type('Artifact', (), {'model_dump': lambda self: {'job_id': job}})()
    })())
    monkeypatch.setattr(appearance_api, 'load_config', lambda: object(), raising=False)
    monkeypatch.setattr(appearance_api, 'ensure_cs2_path', lambda config: config, raising=False)
    monkeypatch.setattr(appearance_api, 'launch_verified_appearance', lambda artifact, config, playback, **kwargs:
        called.append((artifact, kwargs)) or {'session_id': 'offline', 'visual_verified': None}, raising=False)
    route = '/api/cosmetic-studio/appearance/jobs/test-job/playback'
    assert client.post(route, json={'offline_acknowledged': True}).status_code == 401
    assert client.post(route, json={'offline_acknowledged': False}, headers=headers).status_code == 400
    assert client.post(route, json={'offline_acknowledged': 'true'}, headers=headers).status_code == 422
    assert client.post(route, json={'offline_acknowledged': True, 'playback_path': 'D:/untrusted.dem'}, headers=headers).status_code == 422
    assert called == []
    response = client.post(route, json={'offline_acknowledged': True}, headers=headers)
    assert response.status_code == 200
    assert response.json()['visual_verified'] is None
    assert called == [({'job_id': 'test-job'}, {'offline_acknowledged': True})]


def test_playback_rejects_unverified_job_without_launch(monkeypatch, tmp_path):
    client = _client(monkeypatch, tmp_path)
    monkeypatch.setattr(appearance_api.service, 'status', lambda job: type('Status', (), {'state': 'failed', 'artifact': None})())
    headers = {session_auth.SESSION_TOKEN_HEADER: TOKEN}
    response = client.post('/api/cosmetic-studio/appearance/jobs/test-job/playback', json={'offline_acknowledged': True}, headers=headers)
    assert response.status_code == 409
