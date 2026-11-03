"""Recording API integration with real contracts; mock only external boundaries."""
import asyncio
from pathlib import Path
from types import SimpleNamespace
import pytest
from fastapi import HTTPException
from app.recording import api,queue_appearance
from app.env_utils import AppConfig
from app import obs_director,cs2_config_backup
from app.cosmetic_studio.runtime import digest
from test_recording_appearance import request,profile

@pytest.mark.parametrize('case',['success','prepare_failure','cancel','tampered'])
def test_queue_skins_reach_director_only_after_verification(monkeypatch,tmp_path,case):
 source=tmp_path/'original.dem';source.write_bytes(b'original')
 copy=tmp_path/'playback.dem';copy.write_bytes(b'copy')
 selected=profile();selected['expected_demo_sha256']=digest(source)
 dto=request(selected);dto.demo.demo_path=str(source)
 plain=request(request_id='plain');plain.demo.demo_path=str(source)
 receipt={'state':'file_verified','original_path':str(source),'original_sha256':digest(source),
  'playback_path':str(copy),'playback_sha256':digest(copy),'visual_verified':None}
 events=[];persisted=[]
 monkeypatch.setattr(api,'_queue_abort_event',None)
 monkeypatch.setattr(api,'load_config',lambda:AppConfig(cs2_path='D:/cs2.exe'))
 monkeypatch.setattr(api,'ensure_cs2_path',lambda cfg:cfg)
 monkeypatch.setattr(cs2_config_backup,'is_cs2_running',lambda:False)
 monkeypatch.setattr(cs2_config_backup,'is_restore_required',lambda:False)
 async def prepare(rows,abort):
  events.append('prepare')
  assert api._queue_abort_event is abort
  if case=='prepare_failure':raise ValueError('外观失败')
  if case=='cancel':abort.set()
  return [rows[0].model_copy(update={'demo':rows[0].demo.model_copy(update={'demo_path':str(copy)})}),rows[1]],{dto.request_id:receipt}
 monkeypatch.setattr(queue_appearance,'prepare_queue_appearances',prepare)
 class OBS:
  def __init__(self,*a):pass
  def connect(self):events.append('obs')
  def disconnect(self):pass
 monkeypatch.setattr(api,'OBSClient',OBS)
 class Fade:
  def __init__(self,*a):pass
  async def setup(self):return False
 monkeypatch.setattr(api,'OBSFadeController',Fade)
 class Director:
  def __init__(self,*a,**kw):self.abort=kw['abort_event']
  async def execute_plan_queue(self,rows,**kw):
   events.append('director')
   assert self.abort is api._queue_abort_event and not self.abort.is_set()
   assert rows[0].demo.demo_path==str(copy) and rows[1].demo.demo_path==str(source)
   return [{'request_id':'one','success':True},{'request_id':'plain','success':False}]
 monkeypatch.setattr(obs_director,'OBSDirector',Director)
 def compat(path):
  events.append('compat')
  assert path==str(copy),'Compatibility must not edit skin-bound original'
  if case=='tampered':copy.write_bytes(b'tampered')
  return SimpleNamespace(cached=True,report=SimpleNamespace(outcome='unchanged',removed_messages=0,removed_win_panel_events=0))
 monkeypatch.setattr(api,'ensure_demo_compatible',compat)
 async def persist(rows,results):persisted.extend(rows)
 monkeypatch.setattr(api,'_persist_v3_results',persist)
 async def run():return await api.execute_recording_queue(api.QueueRecordingRequest(requests=[dto,plain]))
 if case=='success':
  results=asyncio.run(run())
  assert events==['prepare','obs','compat','director']
  assert results[0]['recording_appearance']==receipt and 'recording_appearance' not in results[1]
  assert persisted[0].demo.demo_path==str(source)
 else:
  with pytest.raises(HTTPException):asyncio.run(run())
  assert 'director' not in events
  if case in {'prepare_failure','cancel'}:assert 'obs' not in events
 assert source.read_bytes()==b'original'
 assert api._queue_abort_event is None

def test_single_execution_does_not_silently_ignore_skin_profile():
 with pytest.raises(HTTPException) as error:asyncio.run(api.execute_recording(request(profile())))
 assert error.value.status_code==422
