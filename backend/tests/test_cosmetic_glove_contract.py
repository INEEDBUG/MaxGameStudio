"""Bounded, offline file-provider glove attachment contract."""
import pytest

from app.cosmetic_studio import skin_core
from app.cosmetic_studio.models import JobRequest


def test_core_profile_serializes_only_the_qualified_glove_replacement():
    glove=skin_core.CoreGloveItem(item_id64='52517827499')
    weapon=skin_core.CoreItem(item_id64='48742431476',definition_index=507,paint_kit=38)
    profile=skin_core.CoreProfile(steam_id64='76561199369875913',items=(weapon,glove))
    payload=profile.payload()
    assert 'replacement_definition_index' not in payload['items'][0]
    assert payload['items'][1]=={'item_id64':'52517827499','definition_index':5030,
        'replacement_definition_index':5034,'paint_kit':10061,'pattern_seed':0,'wear':.01}
    for changes in ({'definition_index':5031},{'replacement_definition_index':5035},
                    {'paint_kit':10062},{'pattern_seed':1},{'wear':.5}):
        with pytest.raises(ValueError): skin_core.CoreGloveItem(item_id64='52517827499',**changes)


def test_ordinary_weapon_contract_does_not_gain_model_replacement():
    with pytest.raises(ValueError):
        skin_core.CoreItem(item_id64='48742431476',definition_index=507,paint_kit=38,
            replacement_definition_index=500)


@pytest.mark.parametrize('change',[{'definition_index':5030.0},{'paint_kit':10061.0},
                                  {'pattern_seed':False}])
def test_glove_protocol_never_coerces_float_or_boolean_identifiers(change):
    with pytest.raises(ValueError):
        skin_core.CoreGloveItem.model_validate({'item_id64':'52517827499',**change})


def test_combined_profile_never_accepts_a_failed_glove_verifier(monkeypatch, tmp_path):
    from app.cosmetic_studio import core_inspection, glove_inspection
    glove=skin_core.CoreGloveItem(item_id64='52517827499')
    weapon=skin_core.CoreItem(item_id64='48742431476',definition_index=507,paint_kit=38)
    profile=skin_core.CoreProfile(steam_id64='76561199369875913',items=(weapon,glove))
    def gun_check(source, output, spec, ticks, **kwargs):
        assert spec.items == (weapon,)
        assert kwargs['require_target_holding'] is True
        return True
    monkeypatch.setattr(core_inspection, 'verify_rewrite', gun_check)
    monkeypatch.setattr(glove_inspection, 'verify_glove_rewrite', lambda *args: False)
    assert not core_inspection.verify_profile_rewrite(tmp_path/'source.dem',tmp_path/'copy.dem',profile,[1000])
    monkeypatch.setattr(glove_inspection, 'verify_glove_rewrite', lambda *args: True)
    assert core_inspection.verify_profile_rewrite(tmp_path/'source.dem',tmp_path/'copy.dem',profile,[1000])


def test_runtime_dll_provider_cannot_request_the_file_only_glove_attachment():
    body=dict(demo_path=r'D:\Demo\match.dem',xuid='76561199369875913',start_tick=1000,end_tick=1128,
        provider_id='nskinz',preset_id='awp-dragon-lore',glove_preset_id='specialist-10061',
        acknowledge_offline=True,tools={'cs2_exe':r'D:\Tools\cs2.exe','hlae_exe':r'D:\Tools\hlae.exe',
        'nskinz_dll':r'D:\Tools\nskinz.dll','obs_exe':r'D:\Tools\obs.exe',
        'ffmpeg_exe':r'D:\Tools\ffmpeg.exe','ffprobe_exe':r'D:\Tools\ffprobe.exe','output_dir':r'D:\Movies'})
    with pytest.raises(ValueError): JobRequest.model_validate(body)
    body.update(provider_id='skin_core',preset_id='karambit-fade');body['tools']['core_exe']=r'D:\Tools\skin-core.exe'
    assert JobRequest.model_validate(body).glove_preset_id=='specialist-10061'
    with pytest.raises(ValueError):JobRequest.model_validate({**body,'preset_id':'ak47-fire-serpent'})


def test_glove_attachment_is_preserved_in_player_profile_without_starting(tmp_path):
    from app.cosmetic_studio.profiles import ProfileStore, ProfileValidationError
    store=ProfileStore(tmp_path/'profiles.json')
    body={'name':'Knife and gloves','xuid':'76561199369875913','preset_id':'karambit-fade',
          'glove_preset_id':'specialist-10061'}
    saved=store.create(body)
    assert store.list()[0]['glove_preset_id']==saved['glove_preset_id']=='specialist-10061'
    with pytest.raises(ProfileValidationError):
        store.create({**body,'glove_preset_id':'arbitrary-replacement'})
