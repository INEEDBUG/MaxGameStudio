"""Appearance endpoints mounted beneath the existing desktop-authenticated router."""
from fastapi import APIRouter, Depends, HTTPException
from pydantic import StrictBool

from ..demo_playback_service import demo_playback_service, DemoPlaybackBusyError, DemoPlaybackCs2RunningError
from ..env_utils import ensure_cs2_path, load_config
from ..runtime_session import runtime_session_dependency
from .appearance_playback import launch_verified_appearance

from .appearance import AppearanceError, service
from .appearance_inventory import AppearanceInventoryError
from .appearance_models import (
    AppearanceApplyRequest, AppearanceInventoryRequest, AppearanceInventoryResponse,
    AppearanceJobStatus, AppearanceModel, AppearanceSettings, AppearanceSettingsRequest,
)
from .appearance_store import AppearanceStoreError

router = APIRouter(prefix="/appearance", tags=["Demo appearance plans"])

from .sky import SkyPreviewRequest, available_skies
from .sky_playback import sky_playback, default_hlae


@router.get('/sky/options')
def sky_options():
    # Read only; viewing this panel must not normalize/migrate app settings.
    import json
    from ..env_utils import AppConfig, resolve_config_path, detect_cs2_path
    path=resolve_config_path()
    try:
        cfg=AppConfig.model_validate(json.loads(path.read_text(encoding='utf-8-sig'))) if path.is_file() else AppConfig()
    except (ValueError,OSError): cfg=AppConfig()
    cs2=cfg.cs2_path or detect_cs2_path() or ''
    return {'presets':available_skies(cs2),'hlae_exe':default_hlae()}


@router.post('/sky/preview',status_code=202)
def sky_preview(request: SkyPreviewRequest):
    try:
        job=service.status(request.job_id)
        if job.state!='file_verified' or not job.artifact:
            raise ValueError('请先保存并校验 Demo 副本。')
        cfg=ensure_cs2_path(load_config())
        return sky_playback.start(job.artifact.model_dump(),cfg.cs2_path,
            request.hlae_exe or default_hlae(),service.settings().output_dir,request.sky_id)
    except KeyError as exc: raise HTTPException(404,detail='制片副本不存在。') from exc
    except (ValueError,OSError,AppearanceStoreError) as exc: raise HTTPException(400,detail=str(exc)) from exc


@router.get('/sky/status')
def sky_status(): return sky_playback.status()


@router.post('/sky/stop')
def stop_sky(): return sky_playback.stop()


class AppearancePlaybackRequest(AppearanceModel):
    offline_acknowledged: StrictBool


@router.post("/inventory", response_model=AppearanceInventoryResponse)
def inventory(request: AppearanceInventoryRequest):
    try:
        return service.inventory(request.demo_path, xuid=request.xuid) if request.xuid else service.inventory(request.demo_path)
    except (AppearanceError, AppearanceInventoryError, AppearanceStoreError, OSError, ValueError) as exc:
        raise HTTPException(400, detail=str(exc)) from exc


@router.put("/settings", response_model=AppearanceSettings)
def save_settings(request: AppearanceSettingsRequest):
    try:
        return service.update_settings(request.core_exe, request.output_dir)
    except (AppearanceStoreError, OSError, ValueError) as exc:
        raise HTTPException(400, detail=str(exc)) from exc


@router.post("/apply", response_model=AppearanceJobStatus, status_code=202)
def apply(request: AppearanceApplyRequest):
    try:
        return service.start(request)
    except (AppearanceError, AppearanceStoreError, OSError, ValueError) as exc:
        raise HTTPException(409, detail=str(exc)) from exc


@router.get("/jobs/{job_id}", response_model=AppearanceJobStatus)
def status(job_id: str):
    try:
        return service.status(job_id)
    except KeyError as exc:
        raise HTTPException(404, detail="Appearance job was not found") from exc
    except (AppearanceStoreError, ValueError) as exc:
        raise HTTPException(409, detail=str(exc)) from exc


@router.post("/jobs/{job_id}/cancel", response_model=AppearanceJobStatus)
def cancel(job_id: str):
    try:
        return service.cancel(job_id)
    except KeyError as exc:
        raise HTTPException(404, detail="Appearance job was not found") from exc
    except (AppearanceStoreError, ValueError) as exc:
        raise HTTPException(409, detail=str(exc)) from exc


@router.post("/jobs/{job_id}/playback", dependencies=[Depends(runtime_session_dependency)])
def playback(job_id: str, request: AppearancePlaybackRequest):
    if request.offline_acknowledged is not True:
        raise HTTPException(400, detail="请明确确认只启动专用离线 Demo 回放。")
    try:
        job = service.status(job_id)
        if job.state != 'file_verified' or job.artifact is None:
            raise HTTPException(409, detail="此任务尚无校验通过的制片副本。")
        return launch_verified_appearance(job.artifact.model_dump(),
            ensure_cs2_path(load_config()), demo_playback_service,
            offline_acknowledged=True)
    except KeyError as exc:
        raise HTTPException(404, detail="Appearance job was not found") from exc
    except (DemoPlaybackBusyError, DemoPlaybackCs2RunningError) as exc:
        raise HTTPException(409, detail="请先结束已有 CS2 会话，再启动专用离线回放。") from exc
    except (AppearanceStoreError, ValueError, OSError) as exc:
        raise HTTPException(400, detail=str(exc)) from exc
