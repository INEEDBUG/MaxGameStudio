"""Real validation and parser-boundary behavior for same-model gun profiles."""
from pathlib import Path
import threading

import pytest

from app.cosmetic_studio import inspection, core_inspection, runtime
from app.cosmetic_studio.models import Capabilities, JobRequest
from app.cosmetic_studio.skin_core import CoreItem, CoreProfile

TARGET = '76561199059074958'
ACCOUNT = 1098809230


def request(preset='ak47-fire-serpent'):
    return JobRequest(demo_path=r'D:\Demos\match.dem', xuid=TARGET,
        start_tick=1000, end_tick=1128, provider_id='skin_core', preset_id=preset,
        acknowledge_offline=True, wear=.06 if preset == 'ak47-fire-serpent' else .01, tools={
            'cs2_exe': r'D:\CS2\game\bin\win64\cs2.exe', 'hlae_exe': r'D:\HLAE\hlae.exe',
            'core_exe': r'D:\Tools\skin-core.exe', 'obs_exe': r'D:\OBS\obs64.exe',
            'ffmpeg_exe': r'D:\Tools\ffmpeg.exe', 'ffprobe_exe': r'D:\Tools\ffprobe.exe',
            'output_dir': r'D:\Movies'})


def test_ak_profile_and_capabilities_bind_exact_definition_and_paint():
    assert request().preset_id == 'ak47-fire-serpent'
    item = CoreItem(item_id64='52256425842', definition_index=7, paint_kit=180, wear=.06)
    assert CoreProfile(steam_id64=TARGET, items=(item,)).payload()['items'][0]['definition_index'] == 7
    preset = next(p for p in Capabilities(enabled=True, reason='', defaults={}).presets if p.id == 'ak47-fire-serpent')
    assert (preset.weapon_id, preset.paint_kit, preset.provider_ids) == (7, 180, ['skin_core'])
    with pytest.raises(ValueError):
        CoreItem(item_id64='52256425842', definition_index=7, paint_kit=344)


class Parser:
    def parse_player_info(self):
        return [{'steamid': TARGET, 'name': 'Player'}]

    def parse_event(self, name):
        return [{'tick': 1000, 'weapon': 'ak47', 'attacker_steamid': TARGET}] if name == 'player_death' else []

    def parse_ticks(self, fields, ticks):
        return [{'tick': tick, 'steamid': TARGET, 'user_id': 3, 'health': 100,
            'team_num': 2, 'active_weapon_name': 'AK-47',
            'item_id_high': 12, 'item_id_low': 717885,
            'Weapon.m_iAccountID': ACCOUNT, 'weapon_skin_id': 180,
            'weapon_paint_seed': 0, 'weapon_float': .06} for tick in ticks]

    def parse_header(self):
        return {'map_name': 'de_dust2'}


def test_parser_emits_weapon_bound_ak_candidates_and_resolves_player(monkeypatch, tmp_path):
    demo = tmp_path / 'match.dem'
    demo.write_bytes(b'fixture')
    monkeypatch.setattr(inspection, '_parser', lambda _: Parser())
    found = inspection.inspect_demo(demo)
    assert len(found.candidates) == 1
    assert found.candidates[0].weapon_id == 7
    assert inspection.resolve_slot(demo, TARGET, 1000, weapon_id=7) == 4
    with pytest.raises(inspection.InspectionError):
        inspection.resolve_slot(demo, TARGET, 1000, weapon_id=9)


def test_independent_rewrite_verification_accepts_ak_only_for_owned_identity(monkeypatch, tmp_path):
    source, output = tmp_path / 'original.dem', tmp_path / 'playback.dem'
    monkeypatch.setattr(core_inspection, '_parser', lambda _: Parser())
    item_id = core_inspection.resolve_owned_weapon(source, TARGET, 1000, 7)
    assert item_id == str((12 << 32) | 717885)
    profile = CoreProfile(steam_id64=TARGET, items=(CoreItem(item_id64=item_id, definition_index=7, paint_kit=180, wear=.06),))
    assert core_inspection.verify_rewrite(source, output, profile, [1000, 1001]) is True
    class WrongOwner(Parser):
        def parse_ticks(self, fields, ticks):
            rows = super().parse_ticks(fields, ticks)
            for row in rows:
                row['Weapon.m_iAccountID'] += 1
            return rows
    monkeypatch.setattr(core_inspection, '_parser', lambda _: WrongOwner())
    with pytest.raises(inspection.InspectionError):
        core_inspection.resolve_owned_weapon(source, TARGET, 1000, 7)


def test_independent_verifier_rejects_other_players_item_definition_drift(monkeypatch, tmp_path):
    class WithOther(Parser):
        def __init__(self, modified): self.modified=modified
        def parse_ticks(self, fields, ticks):
            rows=super().parse_ticks(fields,ticks)
            for row in list(rows):
                row['item_def_idx']=7
                other={**row,'steamid':'76561199034001385','item_id_low':717886,
                    'item_def_idx':507 if self.modified else 7}
                rows.append(other)
            return rows
    source,output=tmp_path/'original.dem',tmp_path/'playback.dem'
    monkeypatch.setattr(core_inspection,'_parser',lambda path: WithOther(path==output))
    profile=CoreProfile(steam_id64=TARGET,items=(CoreItem(item_id64=str((12<<32)|717885),
        definition_index=7,paint_kit=180,wear=.06),))
    assert core_inspection.verify_rewrite(source,output,profile,[1000,1001],require_target_holding=True) is False


@pytest.mark.parametrize('definition,paint,first,second',[(60,445,'M4A1-S','M4A4'),
                                                       (61,504,'USP-S','P2000')])
def test_independent_verifier_rejects_same_engine_class_midspan_switch(definition,paint,first,second,monkeypatch,tmp_path):
    class Switching(Parser):
        def parse_ticks(self,fields,ticks):
            rows=super().parse_ticks(fields,ticks)
            for row in rows:
                row['active_weapon_name']=first if row['tick']==1000 else second
                if row['tick']==1000:row['weapon_skin_id']=paint
                else:row['item_id_low']=717886
            return rows
    monkeypatch.setattr(core_inspection,'_parser',lambda _:Switching())
    profile=CoreProfile(steam_id64=TARGET,items=(CoreItem(item_id64=str((12<<32)|717885),
        definition_index=definition,paint_kit=paint),))
    assert core_inspection.verify_rewrite(tmp_path/'original.dem',tmp_path/'playback.dem',profile,[1000,1001],require_target_holding=True) is False


def test_native_context_checks_selected_ak_not_awp_substring(monkeypatch, tmp_path):
    session = runtime.NativeSession(request(), tmp_path, 4, threading.Event())
    monkeypatch.setattr(session, 'current', lambda: {'paused': True, 'phase': 'IDLE', 'tick': 936,
        'pov': {'mode': 2, 'xuid': TARGET, 'weapon': 'C_WeaponAK47', 'weapon_debug': 'weapon_ak47'}})
    session.validate_context(paused=True)
    monkeypatch.setattr(session, 'current', lambda: {'paused': True, 'phase': 'IDLE', 'tick': 936,
        'pov': {'mode': 2, 'xuid': TARGET, 'weapon': 'C_WeaponAWP', 'weapon_debug': 'weapon_awp'}})
    with pytest.raises(runtime.CosmeticError):
        session.validate_context(paused=True)


@pytest.mark.parametrize('preset,engine_class', [('usp-kill-confirmed','weapon_hkp2000'),
                                               ('m4a1s-hot-rod','weapon_m4a1')])
def test_engine_shared_classes_do_not_reclassify_parser_inventory(preset, engine_class, monkeypatch, tmp_path):
    from app.cosmetic_studio.catalog import weapon_matches
    definition = 61 if preset.startswith('usp') else 60
    assert not weapon_matches(engine_class, definition)
    session = runtime.NativeSession(request(preset), tmp_path, 4, threading.Event())
    monkeypatch.setattr(session, 'current', lambda: {'paused':True,'phase':'IDLE','tick':936,
        'pov': {'mode':2,'xuid':TARGET,'weapon':engine_class,'weapon_debug':engine_class}})
    session.validate_context(paused=True)


def test_qualified_native_build_pair_rejects_mixed_hook(monkeypatch, tmp_path):
    session = runtime.NativeSession(request('awp-dragon-lore'), tmp_path, 4, threading.Event())
    session.demo_hash = 'demo'
    actual = {key: runtime.PINS[key] for key in session.paths}
    actual.update(cs2_exe='bb25389a10b381b0b50c984087f34e6b30f2919108751a0fdd97cf0dfc142c7b',
        client='d7db25d48f1d10c5e0b0296e20ed803426eb9509da41760daeda39dd35ba89b9',
        hlae_exe='c62d80943382537a6fa3daf0198f83a9d3f8125e555ccbe14bd82e1c83d031cb',
        hook='27d3298cd1063c7bd2fd1c43e23471816488dbbf668129ae7563fc459f708a08')
    def digest(path):
        return 'demo' if path == session.demo else actual[next(k for k, p in session.paths.items() if p == path)]
    monkeypatch.setattr(runtime, 'digest', digest)
    session.check_pins()
    actual['hook'] = 'adce361d0fa994d2e04627e70db8d8ddc6e8b3409237384fd7584a157ef0bd49'
    with pytest.raises(runtime.CosmeticError):
        session.check_pins()


def test_butterfly_requires_current_complete_file_build_and_accepts_shared_native_alias(monkeypatch, tmp_path):
    session = runtime.NativeSession(request('karambit-to-butterfly-fade'), tmp_path, 4, threading.Event())
    session.demo_hash = 'demo'
    baseline = {key: runtime.PINS[key] for key in session.paths}
    def digest(path):
        if path == session.demo:
            return 'demo'
        key = next(name for name, candidate in session.paths.items() if candidate == path)
        return baseline[key]
    monkeypatch.setattr(runtime, 'digest', digest)
    with pytest.raises(runtime.CosmeticError):
        session.check_pins()

    baseline.update(runtime.CURRENT_FILE_BUILD)
    session.check_pins()
    monkeypatch.setattr(session, 'current', lambda: {'paused': True, 'phase': 'IDLE', 'tick': 936,
        'pov': {'mode': 2, 'xuid': TARGET, 'weapon': 'weapon_knife', 'weapon_debug': 'weapon_knife'}})
    session.validate_context(paused=True)


@pytest.mark.parametrize('preset,glove',[('karambit-fade',None),('awp-dragon-lore','specialist-10061')])
def test_knife_and_glove_require_the_newly_qualified_native_build(preset,glove,monkeypatch,tmp_path):
    req=request(preset).model_copy(update={'glove_preset_id':glove})
    session=runtime.NativeSession(req,tmp_path,4,threading.Event())
    session.demo_hash='demo'
    def digest(path):
        return 'demo' if path==session.demo else runtime.PINS[next(k for k,p in session.paths.items() if p==path)]
    monkeypatch.setattr(runtime,'digest',digest)
    with pytest.raises(runtime.CosmeticError):session.check_pins()
