"""Finite HLAE sky presentation presets; never accept arbitrary console text."""
import struct
from pathlib import Path
from typing import Literal
from pydantic import Field, field_validator
from .appearance_models import AppearanceModel, AppearanceSettings

SKIES = {
    'original':('原始天空',None),
    'italy':('意大利 · 晴朗天空','materials/skybox/cs_italy_s2_skybox_2.vmat_c'),
    'dust2':('沙漠 · 晴空','materials/skybox/sky_de_dust2.vmat_c'),
    'night':('雨夜','materials/skybox/sky_rain_night_01.vmat_c'),
    'overcast':('阴天','materials/skybox/sky_overcast_01.vmat_c'),
    'sunset':('日落','materials/skybox/light_test_sky_sunset_light.vmat_c'),
}

def sky_commands(sky_id):
    if sky_id not in SKIES: raise ValueError('不支持的天空预设。')
    material=SKIES[sky_id][1]
    return ['mirv_sky material '+('"'+material+'"' if material else 'default'),
        'mirv_sky color default','mirv_sky clouds color default','mirv_sky clouds draw 1']

class SkyPreviewRequest(AppearanceModel):
    job_id: str=Field(pattern=r'^[a-f0-9]{32}$')
    sky_id: Literal['original','italy','dust2','night','overcast','sunset']
    offline_acknowledged: Literal[True]
    hlae_exe: str=''

    @field_validator('offline_acknowledged',mode='before')
    @classmethod
    def explicit_true(cls,value):
        if value is not True: raise ValueError('请确认专用离线 Demo 回放。')
        return value

    @field_validator('hlae_exe')
    @classmethod
    def local_tool(cls,value): return AppearanceSettings.local_path_or_empty(value)

def vpk_materials(path: Path):
    """Read only the bounded VPK directory tree; no extraction/execution."""
    with path.open('rb') as stream:
        header=stream.read(12)
        if len(header)!=12: raise ValueError('VPK 索引不完整。')
        magic,version,size=struct.unpack('<III',header)
        if magic!=0x55AA1234 or version not in (1,2) or not 0<size<=32*1024*1024:
            raise ValueError('VPK 索引格式不支持。')
        if version==2: stream.read(16)
        tree=stream.read(size)
        if len(tree)!=size: raise ValueError('VPK 索引不完整。')
    pos=0; result=set()
    def text():
        nonlocal pos
        end=tree.find(b'\0',pos)
        if end<0: raise ValueError('VPK 条目损坏。')
        value=tree[pos:end].decode('utf-8'); pos=end+1
        return value
    while True:
        extension=text()
        if not extension: break
        while True:
            directory=text()
            if not directory: break
            while True:
                name=text()
                if not name: break
                if pos+18>len(tree): raise ValueError('VPK 条目损坏。')
                _,preload,_,_,_,terminator=struct.unpack_from('<IHHIIH',tree,pos)
                pos+=18+preload
                if terminator!=0xFFFF or pos>len(tree): raise ValueError('VPK 条目损坏。')
                if extension=='vmat_c' and directory=='materials/skybox':
                    result.add(directory+'/'+name+'.'+extension)
    return result

def available_skies(cs2_exe):
    try: materials=vpk_materials(Path(cs2_exe).parents[2]/'csgo/pak01_dir.vpk')
    except (OSError,ValueError,IndexError): materials=set()
    return [dict(id=key,label=label,available=material is None or material in materials)
        for key,(label,material) in SKIES.items()]
