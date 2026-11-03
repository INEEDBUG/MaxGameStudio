"""Owned whole-Demo sky preview, using the existing HLAE process/restore guard."""
import json
import os
from pathlib import Path
import sys
import threading
import time
from types import SimpleNamespace
import uuid

from ..runtime_session import claim_runtime_session,release_runtime_session
from .runtime import NativeSession,CURRENT_FILE_BUILD,digest,write_json
from .appearance_playback import launch_verified_appearance
from .sky import available_skies,sky_commands


def default_hlae():
    from ..env_utils import get_bundle_data_dir
    candidates=[get_bundle_data_dir().parent/'tools/hlae/HLAE.exe',
        Path(sys.executable).resolve().parent.parent/'tools/hlae/HLAE.exe']
    return next((str(path) for path in candidates if path.is_file()),'')


class SkySession(NativeSession):
    def __init__(self,artifact,cs2_exe,hlae_exe,root,cancel,sky_id):
        self.artifact,self.sky_id=artifact,sky_id
        tools={'cs2_exe':str(cs2_exe),'hlae_exe':str(hlae_exe),'output_dir':str(root.parent)}
        request=SimpleNamespace(provider_id='skin_core',tools=SimpleNamespace(model_dump=lambda:tools),
            demo_path=artifact['playback_path'],expected_demo_sha256=artifact['playback_sha256'],
            preset_id='awp-dragon-lore',xuid='1',pattern_seed=0,wear=.01,start_tick=2048,end_tick=2176,glove_preset_id=None,sky_id='original')
        super().__init__(request,root,1,cancel)

    def check_pins(self):
        actual={key:digest(self.paths[key]) for key in CURRENT_FILE_BUILD}
        if actual!=CURRENT_FILE_BUILD:
            raise ValueError('CS2/HLAE 版本组合未通过天空验证，不会启动。')
        if digest(self.demo)!=self.demo_hash: raise ValueError('制片副本已变化。')

    def prepare(self):
        if not any(row['id']==self.sky_id and row['available'] for row in available_skies(self.paths['cs2_exe'])):
            raise ValueError('当前 CS2 未安装此天空资源。')
        super().prepare()
        config={'root':self.root.as_posix(),'tag':self.tag,'demo':self.demo.as_posix(),'commands':sky_commands(self.sky_id)}
        source=Path(__file__).with_name('sky_runtime.js').read_text(encoding='utf-8')
        (self.root/'runtime.js').write_text(source.replace('__SKY_CONFIG__',json.dumps(config)),encoding='utf-8')
        self.baseline['script_sha256']=digest(self.root/'runtime.js')
        # The inherited evidence was written before installing this reviewed
        # controller. Replace only our own new evidence, never a user file.
        (self.root/'evidence/baseline.json').write_text(json.dumps(self.baseline,indent=2),encoding='utf-8')

    def operation(self,name):
        if name!='position': raise ValueError('天空预览不接受录制或外观扩展命令。')
        self.wait_for(lambda:self.current().get('sky_submitted') is True,timeout=30)

    def validate_context(self,*,paused):
        self.check_cancel();row=self.current()
        if row.get('paused') is not paused or row.get('sky_submitted') is not True or row.get('phase')!='IDLE':
            raise ValueError('天空预览状态未知，不能确认画面。')

    def launch(self):
        super().launch()
        modules=self.module_paths()
        if any('nskinz' in path.lower() for path in modules):
            raise ValueError('天空模式检测到额外换肤扩展，已停止。')


class SkyPlaybackService:
    def __init__(self,session_factory=SkySession):
        self.factory=session_factory;self.lock=threading.RLock();self.job=None

    def status(self):
        with self.lock:
            return dict(self.job['status']) if self.job else {'state':'idle','message':'未启动天空预览'}

    def start(self,artifact,cs2_exe,hlae_exe,output_dir,sky_id):
        if os.name!='nt': raise ValueError('天空预览仅支持 Windows。')
        # Reuse the complete artifact/hash check without launching the plain
        # player: no unchecked path or stale copy gets into the native session.
        class CheckOnly:
            def launch(self,*args,**kwargs): return {}
        launch_verified_appearance(artifact,None,CheckOnly(),offline_acknowledged=True)
        output=Path(output_dir).resolve(strict=True)
        if output.drive.lower()==os.environ.get('SystemDrive','C:').lower():
            raise ValueError('请选择非系统盘制片目录。')
        with self.lock:
            if self.job and self.job['status']['state'] in {'starting','preview','stopping','cleanup_unknown'}:
                raise ValueError('请先关闭当前天空预览。')
            claim=claim_runtime_session('/api/cosmetic-studio/appearance/sky/preview')
            root=output/('sky-'+uuid.uuid4().hex)
            cancel=threading.Event()
            status={'state':'starting','message':'正在启动专用离线天空预览…','sky_id':sky_id,'visual_verified':None}
            job={'status':status,'cancel':cancel,'claim':claim,'root':root}
            try:
                root.mkdir(exist_ok=False)
                self.job=job
                thread=threading.Thread(target=self._run,args=(job,artifact,cs2_exe,hlae_exe,sky_id),daemon=True,name='sky-offline-preview')
                job['thread']=thread;thread.start()
            except Exception:
                release_runtime_session(claim);raise
            return dict(status)

    def stop(self):
        with self.lock:
            if self.job and self.job['status']['state'] in {'starting','preview'}:
                self.job['cancel'].set();self.job['status'].update(state='stopping',message='正在关闭专用回放并检查配置…')
            return self.status()

    def _run(self,job,artifact,cs2_exe,hlae_exe,sky_id):
        session=None;error=None;clean=False
        try:
            session=self.factory(artifact,cs2_exe,hlae_exe,job['root'],job['cancel'],sky_id)
            session.prepare();session.check_cancel();session.launch()
            with self.lock:job['status'].update(state='preview',message='天空命令已送达，请在原生画面中核对效果。可正常暂停、跳转和切换玩家。')
            while not job['cancel'].wait(.5):
                session.current()  # Stale/unknown/lost Demo terminates the session.
        except Exception as exc:error=str(exc)
        finally:
            try:
                if session: session.stop();clean=session._restored is True
                else: clean=True
                if digest(Path(artifact['original_path']))!=artifact['original_sha256']:
                    clean=False;raise ValueError('原始 Demo 已变化。')
            except Exception as exc:error=(error or '')+'；清理未确认：'+str(exc);clean=False
            if clean:release_runtime_session(job['claim'])
            with self.lock:job['status'].update(state='stopped' if clean and job['cancel'].is_set() else 'failed' if clean else 'cleanup_unknown',
                message='专用回放已关闭，配置检查通过。' if clean and job['cancel'].is_set() else error or '预览已结束。',cleanup_verified=clean)
            write_json(job['root']/'result.json',job['status'])


sky_playback=SkyPlaybackService()
