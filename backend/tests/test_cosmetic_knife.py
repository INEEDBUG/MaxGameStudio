"""Native-qualified same-model knife profile, never an online skin changer."""
import pytest

from app.cosmetic_studio import catalog
from app.cosmetic_studio.catalog import native_weapon_matches, weapon_matches
from app.cosmetic_studio.models import Capabilities
from app.cosmetic_studio.skin_core import CoreItem


def test_qualified_karambit_finish_is_explicit_and_file_provider_only():
    preset = next(p for p in Capabilities(enabled=True, reason='', defaults={}).presets
                  if p.id == 'karambit-fade')
    assert (preset.weapon_id, preset.paint_kit, preset.provider_ids) == (507, 38, ['skin_core'])
    assert CoreItem(item_id64='48742431476', definition_index=507, paint_kit=38).definition_index == 507
    assert weapon_matches('Karambit', 507)
    assert not weapon_matches('weapon_knife', 507)
    assert not weapon_matches('Bayonet', 507)
    assert native_weapon_matches('weapon_knife', 507)


def test_knife_profile_refuses_unqualified_models_and_out_of_schema_wear():
    with pytest.raises(ValueError):
        CoreItem(item_id64='48742431476', definition_index=500, paint_kit=38)
    with pytest.raises(ValueError):
        CoreItem(item_id64='48742431476', definition_index=507, paint_kit=38, wear=.09)


def test_saved_knife_profile_refuses_unusable_wear(tmp_path):
    from app.cosmetic_studio.profiles import ProfileStore, ProfileValidationError
    store=ProfileStore(tmp_path/'profiles.json')
    body={'name':'Knife', 'xuid':'76561199369875913', 'preset_id':'karambit-fade', 'wear':.09}
    with pytest.raises(ProfileValidationError):
        store.create(body)
    saved=store.create({**body, 'wear':.08})
    assert saved['wear'] == .08


def test_qualified_butterfly_finish_is_explicit_cross_model_mapping():
    preset = next(p for p in Capabilities(enabled=True, reason='', defaults={}).presets
                  if p.id == 'karambit-to-butterfly-fade')
    assert (preset.weapon_id, preset.source_weapon_id, preset.paint_kit) == (515, 507, 38)
    assert catalog.source_weapon_id('karambit-to-butterfly-fade') == 507
    assert weapon_matches('Karambit', 507)
    for name in ('Butterfly', 'Butterfly Knife', 'knife_butterfly', 'weapon_knife_butterfly'):
        assert weapon_matches(name, 515)
    for name in ('weapon_knife', 'c_weaponknife', 'weapon_knife_butterfly'):
        assert native_weapon_matches(name, 515)
    assert not native_weapon_matches('weapon_knife_butterfly', 507)


def test_core_item_only_allows_qualified_507_to_515_and_omits_absent_override():
    from app.cosmetic_studio.skin_core import CoreGloveItem, CoreProfile

    item = CoreItem(item_id64='48742431476', definition_index=507,
                    replacement_definition_index=515, paint_kit=38, wear=.08)
    assert item.model_dump(mode='json')['replacement_definition_index'] == 515
    profile = CoreProfile(steam_id64='76561199369875913', items=(item,))
    assert profile.payload()['items'][0]['definition_index'] == 507
    assert profile.payload()['items'][0]['replacement_definition_index'] == 515

    same_model = CoreItem(item_id64='48742431476', definition_index=507, paint_kit=38)
    assert 'replacement_definition_index' not in CoreProfile(
        steam_id64='76561199369875913', items=(same_model,)).payload()['items'][0]
    for values in (
        {'definition_index': 500, 'replacement_definition_index': 515, 'paint_kit': 38},
        {'definition_index': 507, 'replacement_definition_index': 500, 'paint_kit': 38},
        {'definition_index': 507, 'replacement_definition_index': 515, 'paint_kit': 344},
    ):
        with pytest.raises(ValueError):
            CoreItem(item_id64='48742431476', **values)
    glove = CoreGloveItem(item_id64='52517827499')
    with pytest.raises(ValueError, match='not qualified'):
        CoreProfile(steam_id64='76561199369875913', items=(item, glove))
