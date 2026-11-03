"""On-demand selected-player full timeline scan; retain only bounded item proofs.

No persistent frame cache. Complete timeline coverage is distinct from the
Sampled inventory retains its 128-tick threshold. A complete player scan can
retain a shorter positive-ID ordinary-gun proof without changing ownership,
identity or paint checks. The output is independently checked at those ticks.
"""
import copy
from .inspection import _parser, _xuid
from .appearance_inventory import (InventorySnapshot, _PLAYER_TICK_FIELDS, _STEAM_BASE,
    _VERIFY_WINDOW_TICKS, _observed_equipment, _qualify_observed_owned_weapons,
    _qualify_materialized_usp, _owner_account, _item_identity, _exact_int, _verified_window)
from .catalog import WEAPON_NAMES, weapon_matches

_FULL_SCAN_MIN_GUN_TICKS = 32


def _iter_rows(raw):
    if isinstance(raw,dict):
        if not raw:return
        if not all(isinstance(values,(list,tuple)) for values in raw.values()) or len({len(values) for values in raw.values()})!=1:
            raise ValueError('整局解析返回的字段长度不一致。')
        for index in range(len(next(iter(raw.values())))):
            yield {key:values[index] for key,values in raw.items()}
    elif hasattr(raw,'itertuples'):
        columns=list(raw.columns)
        for values in raw.itertuples(index=False,name=None):
            yield dict(zip(columns,values))
    elif isinstance(raw,list):
        yield from raw
    else:
        raise ValueError('无法读取完整对局的装备记录。')


def complete_player_inventory(path, source_hash, xuid, snapshot):
    raw = _parser(path).parse_ticks(list(_PLAYER_TICK_FIELDS), players=[int(xuid)])
    proofs, keyed, identities, unknown, teams = {}, {}, {}, set(), {}
    run, previous_key, previous_tick = [], None, -1
    saw_row=False
    for count,row in enumerate(_iter_rows(raw)):
        saw_row=True
        if count>=1_000_000:
            raise ValueError('对局超出整局扫描上限，未生成部分结果。')
        if _xuid(row.get('steamid'))!=xuid:
            raise ValueError('整局扫描返回了其他玩家，停止应用。')
        try:
            tick=_exact_int(row.get('tick'),'tick',minimum=1,maximum=100_000_000)
            definition=_exact_int(row.get('item_def_idx'),'definition',maximum=100_000)
            health=_exact_int(row.get('health'),'health',maximum=1000)
        except ValueError:
            if row.get('active_weapon_name'):
                raise ValueError('整局中有无法确认的武器时间线，停止应用。')
            run=[]; previous_key=None
            continue
        if tick<=previous_tick:
            raise ValueError('整局时间线重复或乱序，停止应用。')
        previous_tick_before=previous_tick; previous_tick=tick
        if definition not in WEAPON_NAMES or definition>=500 or health<=0 or not weapon_matches(row.get('active_weapon_name'),definition):
            run=[]; previous_key=None
            continue
        identity=_item_identity(row) or '0'
        owner=_owner_account(row)
        key=(identity,definition, row.get('weapon_skin_id') if identity=='0' else None)
        teams.setdefault(key,set()).add(row.get('team_num'))
        if len(identities)>=128 and key not in identities:
            raise ValueError('整局装备记录超出验证上限。')
        # Missing ownership is not silently treated as a foreign pickup.
        if owner is None:
            unknown.add(key)
        if key not in identities:
            identities[key]=row
            keyed[(xuid,tick)]=row
        else:
            before=identities[key]
            if identity!='0' and (before.get('weapon_skin_id')!=row.get('weapon_skin_id') or _owner_account(before)!=owner):
                raise ValueError('整局装备外观或归属存在冲突，停止应用。')
        contiguous=key==previous_key and tick==previous_tick_before+1
        run = run+[row] if contiguous and len(run)<_VERIFY_WINDOW_TICKS else run if contiguous else [row]
        previous_key=key
        if len(run) in {_FULL_SCAN_MIN_GUN_TICKS, _VERIFY_WINDOW_TICKS} and len(run)>len(proofs.get(key,())):
            ticks=tuple(r['tick'] for r in run)
            proofs[key]=ticks
            for proof_row in run: keyed[(xuid,proof_row['tick'])]=proof_row
    del raw
    if not saw_row:raise ValueError('未读到完整玩家时间线，未复用旧的抽样结果。')
    blocks=list(proofs.values())
    observed=_observed_equipment(source_hash,{xuid:[]},keyed)
    windows=_qualify_observed_owned_weapons(observed,blocks,keyed,minimum_ticks=_FULL_SCAN_MIN_GUN_TICKS)
    windows.update(_qualify_materialized_usp(observed,blocks,keyed))
    existing={(i.item_id,i.definition,i.paint_index if i.item_id=='0' else None):i for i in snapshot.response.items if i.xuid==xuid}
    all_windows={slot:ticks for slot,ticks in snapshot.verification_ticks.items()
                 if any(i.slot_id==slot and i.xuid!=xuid for i in snapshot.response.items)}
    merged=[]
    for item in observed:
        key=(item.item_id,item.definition,item.paint_index if item.item_id=='0' else None)
        old=existing.pop(key,None)
        ticks=windows.get(item.slot_id)
        # Preserve stable stored slot identities, not the old sampled eligibility.
        if old:
            item=item.model_copy(update={'slot_id':old.slot_id})
        if key in unknown:
            item=item.model_copy(update={'editable':False,'preset_ids':[],'reason':'match_ownership_unknown'})
        elif _owner_account(identities[key]) not in {0,1,int(xuid)-_STEAM_BASE}:
            item=item.model_copy(update={'editable':False,'preset_ids':[],'reason':'foreign_pickup'})
        elif not item.editable and _owner_account(identities[key])==int(xuid)-_STEAM_BASE:
            item=item.model_copy(update={'reason':'no_bounded_holding_evidence'})
        if ticks and item.editable: all_windows[item.slot_id]=ticks
        sides=teams.get(key,set())&{2,3}
        if item.editable and sides=={2,3}:item=item.model_copy(update={'team':'both'})
        merged.append(item)
    # Non-active inventory items (e.g. gloves) retain the existing separate proof.
    for item in existing.values():
        if item.editable and item.definition<500:
            raise ValueError('整局解析缺少此前确认的装备，停止应用。')
        merged.append(item)
        if item.slot_id in snapshot.verification_ticks: all_windows[item.slot_id]=snapshot.verification_ticks[item.slot_id]
    response=copy.copy(snapshot.response)
    response.items=[i for i in snapshot.response.items if i.xuid!=xuid]+merged
    return InventorySnapshot(response=response,verification_ticks=all_windows)
