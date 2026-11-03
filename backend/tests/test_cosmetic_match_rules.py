from types import SimpleNamespace
import pytest
from app.cosmetic_studio.appearance_models import AppearanceItem, AppearanceReplacement

XUID = '76561198282934892'
def item(slot, identity, **changes):
    return AppearanceItem.model_validate(dict(slot_id=slot, xuid=XUID, team='ct', definition=61,
        item_id=identity, name='USP-S', paint_index=705, wear=.01, seed=0, editable=True,
        preset_ids=['gun-61-818'], **changes))

def choice(slot='original01', preset='gun-61-818'):
    return AppearanceReplacement(slot_id=slot, preset_id=preset, wear=.01, seed=0)

def test_whole_match_expands_rebuy_and_halftime_without_tick_scope():
    from app.cosmetic_studio.match_rules import expand_match_plan
    first = item('original01', '101')
    second = first.model_copy(update={'slot_id':'repurchase02','item_id':'102','team':'t'})
    third = first.model_copy(update={'slot_id':'foreign03','item_id':'103','editable':False,
        'reason':'foreign_pickup','preset_ids':[]})
    plan, skipped = expand_match_plan([first,second,third], XUID, [choice()])
    assert [row.slot_id for row in plan] == ['original01','repurchase02']
    assert skipped == 1

def test_unknown_ownership_is_not_silently_classified_as_a_foreign_pickup():
    from app.cosmetic_studio.match_rules import expand_match_plan
    first=item('original01','101')
    unknown=first.model_copy(update={'slot_id':'unknown02','item_id':'102','editable':False,
        'reason':'unverified_original_ownership','preset_ids':[]})
    with pytest.raises(ValueError,match='未确认'):
        expand_match_plan([first,unknown],XUID,[choice()])

def test_match_never_partially_applies_an_unsupported_owned_slot():
    from app.cosmetic_studio.match_rules import expand_match_plan
    first = item('original01', '101')
    zero = first.model_copy(update={'slot_id':'zeroitem02','item_id':'0','editable':True,
        'preset_ids':['usp-kill-confirmed']})
    with pytest.raises(ValueError, match='整局'):
        expand_match_plan([first, zero], XUID, [choice()])

def test_match_rejects_different_choices_for_one_weapon():
    from app.cosmetic_studio.match_rules import expand_match_plan
    first = item('original01', '101')
    second = first.model_copy(update={'slot_id':'repurchase02','item_id':'102'})
    with pytest.raises(ValueError, match='不同'):
        expand_match_plan([first, second], XUID, [choice(), choice('repurchase02','usp-kill-confirmed')])

def test_full_timeline_discovers_late_purchase_and_preserves_foreign_owner(monkeypatch, tmp_path):
    from app.cosmetic_studio import match_inventory
    from app.cosmetic_studio.appearance_inventory import InventorySnapshot
    first=item('original01','101')
    snapshot=InventorySnapshot(response=SimpleNamespace(items=[first]),verification_ticks={'original01':tuple(range(100,228))})
    rows=[]
    for identity, begin, owner in [(101,100,322669164),(102,20000,322669164),(103,30000,99)]:
        for tick in range(begin,begin+128):
            rows.append(dict(tick=tick,steamid=int(XUID),health=100,team_num=3 if begin==100 else 2,
                active_weapon_name='USP-S', item_def_idx=61,item_id_high=0,item_id_low=identity,
                weapon_skin_id=705,weapon_paint_seed=0,weapon_float=.01,**{'Weapon.m_iAccountID':owner}))
    class Parser:
        def parse_ticks(self, fields, *, players):
            assert players==[int(XUID)]
            return rows
    monkeypatch.setattr(match_inventory,'_parser',lambda path:Parser())
    result=match_inventory.complete_player_inventory(tmp_path/'fixture.dem','a'*64,XUID,snapshot)
    ids={row.item_id for row in result.response.items if row.editable}
    assert ids=={'101','102'}
    assert len(result.verification_ticks)==2

@pytest.mark.parametrize('raw',[{},[],{'tick':[1],'steamid':[]}])
def test_incomplete_full_timeline_never_reuses_old_eligibility(monkeypatch,tmp_path,raw):
    from app.cosmetic_studio import match_inventory
    from app.cosmetic_studio.appearance_inventory import InventorySnapshot
    snapshot=InventorySnapshot(response=SimpleNamespace(items=[item('original01','101')]),verification_ticks={'original01':tuple(range(128))})
    monkeypatch.setattr(match_inventory,'_parser',lambda path:SimpleNamespace(parse_ticks=lambda *args,**kwargs:raw))
    with pytest.raises(ValueError):match_inventory.complete_player_inventory(tmp_path/'fixture.dem','a'*64,XUID,snapshot)

