import asyncio
from types import SimpleNamespace

from app.recording import api


def test_existing_ai_preview_exports_actual_segments_without_second_model_call(monkeypatch):
    calls=[]
    req=SimpleNamespace(demo_path='D:/demo.dem',tick_rate=64,events=[])
    outline=SimpleNamespace(blocks=[],rationale='existing plan')
    async def suggest(value):
        calls.append(value)
        return outline,'heuristic',None
    monkeypatch.setattr(api,'normalize',lambda dto:req)
    monkeypatch.setattr(api,'suggest_recording_outline',suggest)
    monkeypatch.setattr(api,'outline_to_preview_lines',lambda *args:[])
    monkeypatch.setattr(api,'victim_pov_omitted_kills',lambda *args:[])
    monkeypatch.setattr(api,'count_available_victim_pov',lambda *args:0)
    monkeypatch.setattr(api,'plan_from_ai_outline',lambda *args:[SimpleNamespace(
        perspective='killer',target_steamid64='76561199034001385',start_tick=1900,end_tick=2300,disabled=False)])
    result=asyncio.run(api.ai_director_preview(None))
    assert result['cosmetic_plan']=={'demo_path':'D:/demo.dem','tick_rate':64,
        'active_segments':[{'target_steamid64':'76561199034001385','start_tick':1900,'end_tick':2300,'disabled':False}]}
    assert calls==[req]
