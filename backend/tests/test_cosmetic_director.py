"""Director plan bindings reuse proven parser candidates, never execute a plan."""
import pytest
from app.cosmetic_studio.models import Candidate, Inspection
from app.cosmetic_studio.director import select_director_candidates

DEMO=r'D:\Demos\fixture.dem'
XUID='76561199034001385'

def inspection():
    return Inspection(players=[],demo_sha256='a'*64,candidates=[
        Candidate(xuid=XUID,start_tick=1000,end_tick=1128,label='AK',weapon_id=7),
        Candidate(xuid='76561199059074958',start_tick=1000,end_tick=1128,label='AWP')])

def plan():
    return {'demo_path':DEMO,'tick_rate':64,'active_segments':[
        {'target_steamid64':XUID,'start_tick':900,'end_tick':1200,'disabled':False}]}

def test_import_retains_exact_player_contained_candidates():
    result=select_director_candidates(inspection(),plan(),DEMO,'a'*64)
    assert len(result.candidates)==1
    assert result.candidates[0].weapon_id==7
    assert result.demo_sha256=='a'*64

@pytest.mark.parametrize('mutation', ['source','hash','float_xuid','disabled','partial','rate','too_many'])
def test_wrong_or_unknown_director_bindings_rejected(mutation):
    value=plan(); expected='a'*64
    if mutation=='source':value['demo_path']=r'D:\Demos\another.dem'
    elif mutation=='hash':expected='b'*64
    elif mutation=='float_xuid':value['active_segments'][0]['target_steamid64']=float(XUID)
    elif mutation=='disabled':value['active_segments'][0]['disabled']=True
    elif mutation=='partial':value['active_segments'][0]['end_tick']=1100
    elif mutation=='rate':value['tick_rate']=128
    elif mutation=='too_many':value['active_segments']*=129
    with pytest.raises(ValueError):select_director_candidates(inspection(),value,DEMO,expected)
