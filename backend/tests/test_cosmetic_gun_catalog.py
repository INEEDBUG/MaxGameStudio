"""All-gun selection contracts, not permission to alter ownership or models."""
import pytest
from pydantic import ValidationError

from app.cosmetic_studio.catalog import PRESET_DATA, weapon_definition
from app.cosmetic_studio.appearance_inventory import preset_catalog
from app.cosmetic_studio.appearance_models import AppearanceReplacement
from app.cosmetic_studio.skin_core import CoreItem, CoreMaterializedUspItem

GUNS = [(1, 'deagle'), (2, 'elite'), (3, 'fiveseven'), (4, 'glock'),
        (7, 'ak47'), (8, 'aug'), (9, 'awp'), (10, 'famas'), (11, 'g3sg1'),
        (13, 'galilar'), (14, 'm249'), (16, 'm4a1'), (17, 'mac10'), (19, 'p90'),
        (23, 'mp5sd'), (24, 'ump45'), (25, 'xm1014'), (26, 'bizon'), (27, 'mag7'),
        (28, 'negev'), (29, 'sawedoff'), (30, 'tec9'), (32, 'hkp2000'),
        (33, 'mp7'), (34, 'mp9'), (35, 'nova'), (36, 'p250'), (38, 'scar20'),
        (39, 'sg556'), (40, 'ssg08'), (60, 'm4a1_silencer'), (61, 'usp_silencer'),
        (63, 'cz75a'), (64, 'revolver')]


@pytest.mark.parametrize('definition,alias', GUNS)
def test_every_firearm_has_compatible_picker_entries_and_parser_aliases(definition, alias):
    assert weapon_definition('weapon_' + alias) == definition
    entries = [p for p in preset_catalog() if p.source_definition == definition]
    assert len(entries) >= 20
    assert all(p.target_definition == definition and p.image_url for p in entries)


def test_catalog_contains_every_snapshot_gun_finish_once_and_retains_old_ids():
    entries = [p for p in preset_catalog() if p.target_definition < 100]
    assert len(entries) == 1449
    assert len({(p.target_definition, p.paint_index) for p in entries}) == 1449
    assert PRESET_DATA['awp-dragon-lore'][:2] == (9, 344)
    assert PRESET_DATA['usp-kill-confirmed'][:2] == (61, 504)
    assert PRESET_DATA['karambit-to-butterfly-fade'][:2] == (515, 38)


@pytest.mark.parametrize('definition,paint,low,high', [
    (10, 1066, 0., .5), (3, 1002, 0., .1), (40, 253, 0., .03),
    (23, 781, 0., .5), (9, 1026, 0., .08),
])
def test_new_finish_bounds_reach_picker_selection_and_core(definition, paint, low, high):
    preset_id = f'gun-{definition}-{paint}'
    option = next((p for p in preset_catalog() if p.id == preset_id), None)
    assert option is not None
    assert (option.wear_min, option.wear_max) == (low, high)
    selection = AppearanceReplacement(slot_id='slot-0000', preset_id=preset_id, wear=high, seed=0)
    assert selection.preset_id == preset_id
    assert CoreItem(item_id64='123', definition_index=definition, paint_kit=paint, wear=high).paint_kit == paint
    with pytest.raises(ValidationError):
        AppearanceReplacement(slot_id='slot-0000', preset_id=preset_id, wear=high + .001, seed=0)
    with pytest.raises(ValidationError):
        CoreItem(item_id64='123', definition_index=definition, paint_kit=paint, wear=high + .001)


@pytest.mark.parametrize('definition,paint', [(34, 344), (9, 99999), (10, 504), (999, 1066)])
def test_full_catalog_still_rejects_nonexistent_or_wrong_gun_finishes(definition, paint):
    with pytest.raises(ValidationError):
        CoreItem(item_id64='123', definition_index=definition, paint_kit=paint)


def test_full_catalog_does_not_expand_zero_id_or_cross_model_rules():
    with pytest.raises(ValidationError):
        CoreMaterializedUspItem(paint_kit=818)
    with pytest.raises(ValidationError):
        CoreItem(item_id64='123', definition_index=34, replacement_definition_index=9, paint_kit=344)
    with pytest.raises(ValidationError):
        AppearanceReplacement(slot_id='slot-0000', preset_id='gun-34-630', wear=.01, seed=0, stickers=[])


@pytest.mark.parametrize('alias,definition', [
    ('Five-SeveN', 3), ('Dual Berettas', 2), ('Galil AR', 13), ('MP5-SD', 23),
    ('MAG-7', 27), ('PP-Bizon', 26), ('SG 553', 39), ('SSG 08', 40),
    ('CZ75-Auto', 63), ('R8 Revolver', 64), ('P2000', 32),
])
def test_parser_display_names_recognize_the_exact_gun_without_model_alias_collisions(alias, definition):
    assert weapon_definition(alias) == definition
    assert weapon_definition('weapon_hkp2000') == 32
    assert weapon_definition('weapon_m4a1') == 16


def test_profile_without_wear_chooses_a_legal_default_but_explicit_invalid_wear_is_rejected(tmp_path):
    from app.cosmetic_studio.profiles import ProfileStore, ProfileValidationError
    store = ProfileStore(tmp_path / 'profiles.json')
    payload = {'name': 'Fire Serpent', 'xuid': '76561199369875913', 'preset_id': 'ak47-fire-serpent'}
    assert store.create(payload)['wear'] == .06
    with pytest.raises(ProfileValidationError):
        store.create({**payload, 'wear': .01})


def test_newly_supported_gun_requires_real_original_ownership_and_continuous_holding(monkeypatch, tmp_path):
    from app.cosmetic_studio import appearance_inventory
    from test_cosmetic_appearance_inventory import Parser
    xuid = '76561199369875913'
    demo = tmp_path / 'mp9.dem'
    demo.write_bytes(b'original')
    item_id = 50509010986
    high, low = divmod(item_id, 1 << 32)
    rows = [{'tick': tick, 'steamid': int(xuid), 'health': 100, 'team_num': 3,
             'active_weapon_name': 'MP9', 'item_def_idx': 34, 'weapon_skin_id': 630,
             'weapon_paint_seed': 0, 'weapon_float': .02,
             'item_id_high': high, 'item_id_low': low,
             'Weapon.m_iAccountID': int(xuid) - 76561197960265728}
            for tick in range(100, 228)]
    monkeypatch.setattr(appearance_inventory, '_parser', lambda _: Parser(
        [{'steamid': int(xuid), 'name': 'Player'}], {}, rows))
    snapshot = appearance_inventory.inspect_inventory(demo)
    item = snapshot.response.items[0]
    assert item.editable and item.definition == 34
    assert len(item.preset_ids) == 47 and 'gun-34-630' in item.preset_ids
    assert snapshot.verification_ticks[item.slot_id] == tuple(range(100, 228))
    for row in rows:
        row['Weapon.m_iAccountID'] += 1
    item = appearance_inventory.inspect_inventory(demo).response.items[0]
    assert not item.editable and item.preset_ids == []
