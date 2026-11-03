import pytest


def test_vpk_reader_handles_preload_and_rejects_truncated_entries(tmp_path):
    import struct
    from app.cosmetic_studio.sky import vpk_materials
    tree=(b'vmat_c\0materials/skybox\0sky_test\0'+
          struct.pack('<IHHIIH',0,3,0,0,0,0xffff)+b'xyz'+b'\0\0\0')
    path=tmp_path/'pak01_dir.vpk'
    path.write_bytes(struct.pack('<III',0x55aa1234,1,len(tree))+tree)
    assert vpk_materials(path)=={'materials/skybox/sky_test.vmat_c'}
    path.write_bytes(struct.pack('<III',0x55aa1234,2,len(tree))+b'\0'*16+tree)
    assert vpk_materials(path)=={'materials/skybox/sky_test.vmat_c'}
    path.write_bytes(struct.pack('<III',0x55aa1234,1,len(tree))+tree[:-2])
    with pytest.raises(ValueError):vpk_materials(path)


def test_recording_sky_is_optional_finite_and_file_provider_only():
    from app.cosmetic_studio.models import JobRequest
    base=dict(demo_path='D:/demo.dem',expected_demo_sha256='a'*64,xuid='76561198282934892',
              preset_id='awp-dragon-lore',start_tick=2048,end_tick=2176,
              acknowledge_offline=True,tools=dict(cs2_exe='D:/cs2.exe',hlae_exe='D:/HLAE.exe',
                 output_dir='D:/output',core_exe='D:/skin-core.exe',obs_exe='D:/obs.exe',
                 ffmpeg_exe='D:/ffmpeg.exe',ffprobe_exe='D:/ffprobe.exe'))
    assert JobRequest.model_validate({**base,'provider_id':'skin_core','sky_id':'italy'}).sky_id=='italy'
    with pytest.raises(ValueError):JobRequest.model_validate({**base,'provider_id':'nskinz','sky_id':'italy'})
    with pytest.raises(ValueError):JobRequest.model_validate({**base,'provider_id':'skin_core','sky_id':'italy;connect'})

def test_sky_presets_are_finite_safe_game_local_materials():
    from app.cosmetic_studio.sky import sky_commands
    assert sky_commands('original')==['mirv_sky material default','mirv_sky color default','mirv_sky clouds color default','mirv_sky clouds draw 1']
    commands=sky_commands('italy')
    assert 'mirv_sky material "materials/skybox/cs_italy_s2_skybox_2.vmat_c"' in commands
    for unknown in ['../../x','";connect server','unknown','']:
        with pytest.raises(ValueError): sky_commands(unknown)

def test_sky_contract_does_not_accept_console_commands_or_online_ack():
    from app.cosmetic_studio.sky import SkyPreviewRequest
    base={'job_id':'a'*32,'sky_id':'italy','offline_acknowledged':True}
    assert SkyPreviewRequest.model_validate(base).sky_id=='italy'
    for changes in [{'sky_id':'x;exec autoexec'},{'offline_acknowledged':False},{'command':'connect server'}]:
        with pytest.raises(ValueError): SkyPreviewRequest.model_validate({**base,**changes})

def test_sky_routes_require_desktop_auth(monkeypatch,tmp_path):
    from fastapi import FastAPI
    from fastapi.testclient import TestClient
    from app.cosmetic_studio.api import router
    from app import session_auth
    monkeypatch.setattr(session_auth,'_SESSION_TOKEN','sky-test-session')
    app=FastAPI();app.include_router(router);client=TestClient(app)
    for path in ('options','status'):
        assert client.get('/api/cosmetic-studio/appearance/sky/'+path).status_code==401
    assert client.post('/api/cosmetic-studio/appearance/sky/stop').status_code==401
    assert client.post('/api/cosmetic-studio/appearance/sky/preview',json={}).status_code==401

def test_sky_failure_releases_only_confirmed_cleanup(monkeypatch,tmp_path):
    import threading
    from app.cosmetic_studio import sky_playback as module
    calls=[]
    monkeypatch.setattr(module,'claim_runtime_session',lambda name:'owned-claim')
    monkeypatch.setattr(module,'release_runtime_session',lambda claim:calls.append(claim))
    monkeypatch.setattr(module,'launch_verified_appearance',lambda *args,**kwargs:{})
    monkeypatch.setattr(module,'digest',lambda path:'a'*64)
    class Session:
        _restored=False
        def __init__(self,*args):pass
        def prepare(self):raise ValueError('Steam closed')
        def stop(self):self._restored=True
    service=module.SkyPlaybackService(Session)
    artifact={'original_path':str(tmp_path/'original.dem'),'original_sha256':'a'*64}
    service.start(artifact,'D:/game/cs2.exe','D:/hlae/HLAE.exe',str(tmp_path),'italy')
    service.job['thread'].join(timeout=3)
    assert service.status()['state']=='failed'
    assert service.status()['cleanup_verified'] is True and calls==['owned-claim']

def test_sky_unknown_cleanup_keeps_process_claim(monkeypatch,tmp_path):
    from app.cosmetic_studio import sky_playback as module
    calls=[]
    monkeypatch.setattr(module,'claim_runtime_session',lambda name:'owned-claim')
    monkeypatch.setattr(module,'release_runtime_session',lambda claim:calls.append(claim))
    monkeypatch.setattr(module,'launch_verified_appearance',lambda *args,**kwargs:{})
    monkeypatch.setattr(module,'digest',lambda path:'a'*64)
    class Session:
        _restored=False
        def __init__(self,*args):pass
        def prepare(self):raise ValueError('failed')
        def stop(self):raise ValueError('unknown owner')
    service=module.SkyPlaybackService(Session)
    service.start({'original_path':'D:/original.dem','original_sha256':'a'*64},'D:/game/cs2.exe','D:/hlae/HLAE.exe',str(tmp_path),'italy')
    service.job['thread'].join(timeout=3)
    assert service.status()['state']=='cleanup_unknown' and calls==[]
    with pytest.raises(ValueError,match='关闭'):service.start({},'','',str(tmp_path),'italy')
