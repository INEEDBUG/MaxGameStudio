import pytest
import hashlib
import threading
from pathlib import Path
from types import SimpleNamespace
from pydantic import ValidationError
from app.recording.models import RecordingRequestDTO


def request(profile=None, request_id='one'):
    return RecordingRequestDTO.model_validate({
        'request_id': request_id, 'request_type': 'highlight', 'source_type': 'kill',
        'demo': {'demo_path': 'D:/source.dem', 'demo_filename': 'source.dem',
                 'map_name': 'de_dust2', 'tick_rate': 64, 'appearance': profile,
                 'first_tick': 0, 'demo_end_tick': 10000, 'final_round': 1,
                 'final_round_start_tick': 0, 'final_round_end_tick': 10000},
        'target_player': {'name': 'Alice', 'steamid64': '76561198282934892'},
    })


def profile():
    return {'expected_demo_sha256': 'a' * 64, 'scope': 'match',
            'replacements': [{'slot_id': 'owned:usp', 'preset_id': 'usp-kill-confirmed', 'wear': .01, 'seed': 0}]}


def test_recording_request_preserves_selected_skin_parameters():
    dto = request(profile())
    assert dto.demo.model_dump()['appearance'] == profile()


@pytest.mark.parametrize('change', ['bad_hash', 'unknown_skin', 'duplicate_slot'])
def test_recording_rejects_invalid_appearance_before_execution(change):
    data = profile()
    if change == 'bad_hash': data['expected_demo_sha256'] = 'stale'
    if change == 'unknown_skin': data['replacements'][0]['preset_id'] = 'untrusted-skin'
    if change == 'duplicate_slot': data['replacements'] *= 2
    with pytest.raises(ValidationError):
        request(data)


def test_queue_prepares_distinct_variants_and_keeps_original_provenance(tmp_path):
    from app.recording.queue_appearance import prepare_appearances
    source = tmp_path / 'source.dem'; source.write_bytes(b'original')
    data = profile(); data['expected_demo_sha256'] = hashlib.sha256(b'original').hexdigest()
    one = request(data); one.demo.demo_path = str(source)
    two = one.model_copy(deep=True); two.request_id = 'two'; two.demo.appearance.replacements[0].seed = 42
    three = one.model_copy(deep=True); three.request_id = 'three'
    plain = request(request_id='plain'); plain.demo.demo_path = str(source)
    class FileOnlyService:
        def __init__(self, store): self.store = store
        def run_job(self, job_id, apply, cancel):
            assert not self.store.load_plans(apply.expected_demo_sha256)
            output = Path(self.store.load_settings().output_dir) / (job_id + '.dem')
            output.write_bytes(b'rewritten:' + str(apply.replacements[0].seed).encode())
            return SimpleNamespace(state='file_verified', error=None, artifact=SimpleNamespace(model_dump=lambda: {
                'original_path': str(source), 'original_sha256': data['expected_demo_sha256'],
                'playback_path': str(output), 'playback_sha256': hashlib.sha256(output.read_bytes()).hexdigest(),
                'state': 'file_verified', 'visual_verified': None}))
    output = tmp_path / 'outputs'; output.mkdir()
    prepared, receipts = prepare_appearances([one,two,three,plain], threading.Event(),
        settings={'core_exe': 'D:/core.exe', 'output_dir': str(output)}, service_factory=FileOnlyService)
    assert prepared[0].demo.demo_path != prepared[1].demo.demo_path
    assert prepared[0].demo.demo_path == prepared[2].demo.demo_path
    assert prepared[3] is plain
    assert one.demo.demo_path == str(source) and source.read_bytes() == b'original'
    assert receipts['two']['original_path'] == str(source)
    assert receipts['two']['visual_verified'] is None


@pytest.mark.parametrize('failure', ['source_changed', 'cancelled', 'core_failed', 'copy_tampered'])
def test_queue_never_falls_back_to_original_on_appearance_failure(tmp_path, failure):
    from app.recording.queue_appearance import prepare_appearances
    source = tmp_path / 'source.dem'; source.write_bytes(b'original')
    data = profile(); data['expected_demo_sha256'] = hashlib.sha256(b'original').hexdigest()
    dto = request(data); dto.demo.demo_path = str(source)
    cancel = threading.Event()
    if failure == 'source_changed': source.write_bytes(b'changed')
    if failure == 'cancelled': cancel.set()
    class BadService:
        def __init__(self, store): pass
        def run_job(self, *args):
            if failure == 'copy_tampered':
                return SimpleNamespace(state='file_verified', artifact=SimpleNamespace(model_dump=lambda: {
                    'original_path':str(source),'original_sha256':data['expected_demo_sha256'],
                    'playback_path':str(source),'playback_sha256':'0'*64,'state':'file_verified'}))
            return SimpleNamespace(state='failed', artifact=None, error='替换失败')
    output = tmp_path / 'outputs'; output.mkdir()
    with pytest.raises(ValueError):
        prepare_appearances([dto], cancel, settings={'core_exe':'D:/core.exe','output_dir':str(output)}, service_factory=BadService)


def test_inventory_selected_player_finds_owned_gun_outside_round_start_sample(monkeypatch,tmp_path):
    from app.cosmetic_studio import appearance_inventory, match_inventory
    from app.cosmetic_studio.appearance import AppearanceService
    from app.cosmetic_studio.appearance_store import AppearanceStore
    xuid='76561198282934892'
    source=tmp_path/'late.dem'; source.write_bytes(b'original')
    rows=[{'tick':tick,'steamid':int(xuid),'health':100,'team_num':3,
        'active_weapon_name':'SSG 08','item_def_idx':40,'weapon_skin_id':26,
        'item_id_high':0,'item_id_low':12345,'weapon_paint_seed':0,'weapon_float':.01,
        'Weapon.m_iAccountID':int(xuid)-76561197960265728} for tick in range(1000,1128)]
    class Parser:
        def parse_player_info(self):return [{'steamid':int(xuid),'name':'Alice'}]
        def parse_skins(self):return [{'steamid':int(xuid),'item_id':12345,'def_index':40,'paint_index':26}]
        def parse_event(self,name):return [{'tick':100}]
        def parse_ticks(self,fields,ticks=None,players=None):return rows if players else [row for row in rows if row['tick'] in set(ticks or [])]
    monkeypatch.setattr(appearance_inventory,'_parser',lambda _:Parser())
    monkeypatch.setattr(match_inventory,'_parser',lambda _:Parser())
    service=AppearanceService(AppearanceStore(tmp_path/'settings.json'))
    assert not service.inventory(str(source)).items[0].editable
    result=service.inventory(str(source),xuid=xuid)
    assert result.items[0].editable and result.items[0].definition==40
    assert result.items[0].item_id=='12345' and source.read_bytes()==b'original'

@pytest.mark.parametrize('duration,owner,expected',[(48,322669164,True),(31,322669164,False),(48,417137238,False),(48,None,False)])
def test_full_inventory_short_owned_gun_does_not_require_two_seconds(monkeypatch,tmp_path,duration,owner,expected):
    from app.cosmetic_studio import appearance_inventory,match_inventory
    from app.cosmetic_studio.appearance import AppearanceService
    from app.cosmetic_studio.appearance_store import AppearanceStore
    xuid='76561198282934892'; source=tmp_path/'short.dem';source.write_bytes(b'original')
    rows=[{'tick':tick,'steamid':int(xuid),'health':100,'team_num':3,'active_weapon_name':'M4A1-S',
        'item_def_idx':60,'weapon_skin_id':1433,'item_id_high':12,'item_id_low':816352350,
        'weapon_paint_seed':282,'weapon_float':.3438,'Weapon.m_iAccountID':owner} for tick in range(3693,3693+duration)]
    class Parser:
        def parse_player_info(self):return [{'steamid':int(xuid),'name':'Alice'}]
        def parse_skins(self):return []
        def parse_event(self,name):return [{'tick':100}]
        def parse_ticks(self,fields,ticks=None,players=None):return rows if players else []
    monkeypatch.setattr(appearance_inventory,'_parser',lambda _:Parser())
    monkeypatch.setattr(match_inventory,'_parser',lambda _:Parser())
    result=AppearanceService(AppearanceStore(tmp_path/'settings.json')).inventory(str(source),xuid=xuid)
    assert result.items[0].editable is expected
    assert result.items[0].item_id=='52355959902'
