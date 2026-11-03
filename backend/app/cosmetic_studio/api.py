"""Requires the existing desktop session token on every route; no new listener."""
import asyncio
import json
import os
import sys
from functools import lru_cache
from pathlib import Path

from fastapi import APIRouter, Depends, HTTPException, Request
from fastapi.responses import FileResponse, Response

from .models import Capabilities, Candidate, ConfirmRequest, DirectorImportRequest, Inspection, InspectRequest, IntervalRequest, JobRequest, JobStatus
from .provider import CosmeticError
from .service import service
from .runtime import digest
from .catalog import GLOVE_PRESET_DATA, GLOVE_SUPPORTED_WEAPONS

def require_desktop(request: Request):
    from ..session_auth import SESSION_TOKEN_HEADER, session_auth_enabled, session_token_matches
    # No unauthenticated localhost fallback for game/DLL control, even in dev.
    if not session_auth_enabled() or not session_token_matches(request.headers.get(SESSION_TOKEN_HEADER)):
        raise HTTPException(401, detail="此实验仅接受当前桌面端会话的授权请求")


router = APIRouter(prefix="/api/cosmetic-studio", tags=["Cosmetic Studio (local experiment)"],
                   dependencies=[Depends(require_desktop)])

from .appearance_api import router as appearance_router
router.include_router(appearance_router)


@lru_cache(maxsize=1)
def profile_store():
    from .profiles import ProfileStore
    return ProfileStore()


@router.get('/profiles')
def list_profiles():
    from .profiles import ProfileStoreError
    try:
        return profile_store().list()
    except ProfileStoreError as exc:
        raise HTTPException(409, detail=str(exc)) from exc


@router.post('/profiles', status_code=201)
def create_profile(body: dict):
    from .profiles import ProfileStoreError, ProfileValidationError
    try:
        return profile_store().create(body)
    except ProfileValidationError as exc:
        raise HTTPException(400, detail=str(exc)) from exc
    except ProfileStoreError as exc:
        raise HTTPException(409, detail=str(exc)) from exc


@router.delete('/profiles/{profile_id}', status_code=204)
def delete_profile(profile_id: str):
    from .profiles import ProfileStoreError, ProfileValidationError
    try:
        if not profile_store().delete(profile_id):
            raise HTTPException(404, detail='预设不存在')
        return Response(status_code=204)
    except ProfileValidationError as exc:
        raise HTTPException(400, detail=str(exc)) from exc
    except ProfileStoreError as exc:
        raise HTTPException(409, detail=str(exc)) from exc


def _capability_path(value: str | Path) -> str:
    """Reduce only local extended-length paths; preserve UNC/device paths."""
    text = str(value)
    if (text.startswith("\\\\?\\") and len(text) >= 7
            and text[4].isalpha() and text[5] == ":" and text[6] in ("\\", "/")):
        return text[4:]
    return text


@router.get("/capabilities", response_model=Capabilities)
def capabilities():
    from ..env_utils import AppConfig, resolve_config_path
    # Do not migrate/normalize/write application config just to display this page.
    config_path = resolve_config_path()
    try:
        cfg = AppConfig.model_validate(json.loads(config_path.read_text(encoding="utf-8-sig"))) if config_path.is_file() else AppConfig()
    except (ValueError, OSError):
        cfg = AppConfig()
    bundled_core = Path(sys.executable).parent.parent / "tools" / "skin-core.exe"
    return Capabilities(
        enabled=os.name == "nt",
        reason="独立离线实验，需固定版本工具与人工预览确认；支持已验证枪械、爪子刀渐变、爪子刀替换蝴蝶刀渐变与限定手套。跨刀型预设不能同时附加手套；不支持在线游戏或任意型号替换。" if os.name == "nt" else "此实验仅支持 Windows。",
        glove_presets=[{'id':key,'label':label,'weapon_ids':sorted(GLOVE_SUPPORTED_WEAPONS)} for key,label in GLOVE_PRESET_DATA.items()],
        defaults={"cs2_exe": cfg.cs2_path or "", "obs_exe": cfg.obs.obs_path or "",
                  "ffmpeg_exe": cfg.ffmpeg_path or "", "ffprobe_exe": "", "hlae_exe": "", "nskinz_dll": "",
                  "core_exe": _capability_path(bundled_core) if bundled_core.is_file() else "", "output_dir": ""},
    )


@router.post("/inspect", response_model=Inspection)
async def inspect(request: InspectRequest):
    from .inspection import InspectionError, inspect_demo
    try:
        return await asyncio.to_thread(inspect_demo, Path(request.demo_path))
    except (InspectionError, OSError, ValueError) as exc:
        raise HTTPException(400, detail=str(exc)) from exc


@router.post("/jobs", response_model=JobStatus, status_code=202)
def create(request: JobRequest):
    # Internal qualification fixtures may bind their source independently;
    # every desktop/API job must carry the inspection fingerprint.
    if not request.expected_demo_sha256:
        raise HTTPException(400, detail='请先检查 Demo 并提交检查时的 SHA256')
    if request.glove_preset_id and request.glove_preset_id not in GLOVE_PRESET_DATA:
        raise HTTPException(400, detail='此组合外观尚未完成运行验证，不会启动')
    try:
        return service.start(request)
    except (CosmeticError, OSError, ValueError) as exc:
        raise HTTPException(400, detail=str(exc)) from exc


@router.post('/interval', response_model=Candidate)
async def validate_manual_interval(request: IntervalRequest):
    from .intervals import validate_interval
    path = Path(request.demo_path)
    try:
        if await asyncio.to_thread(digest, path) != request.expected_demo_sha256:
            raise ValueError('Demo 已在检查后变化，请重新检查')
        result = await asyncio.to_thread(validate_interval, path, request.xuid,
            request.start_tick, request.end_tick, request.weapon_id)
        if await asyncio.to_thread(digest, path) != request.expected_demo_sha256:
            raise ValueError('Demo 已在片段校验期间变化，请重新检查')
        return result
    except (OSError, ValueError) as exc:
        raise HTTPException(400, detail=str(exc)) from exc


@router.post('/director/import', response_model=Inspection)
async def import_director(request: DirectorImportRequest):
    from .inspection import inspect_demo
    from .director import select_director_candidates
    try:
        inspection = await asyncio.to_thread(inspect_demo, Path(request.demo_path))
        return select_director_candidates(inspection, request.plan, request.demo_path, request.expected_demo_sha256)
    except (OSError, ValueError) as exc:
        raise HTTPException(400, detail=str(exc)) from exc


@router.get("/active", response_model=JobStatus | None)
def active():
    return service.active()


def call(method, *args):
    try:
        return method(*args)
    except KeyError as exc:
        raise HTTPException(404, detail="此实验任务不存在") from exc
    except (CosmeticError, OSError) as exc:
        raise HTTPException(409, detail=str(exc)) from exc


@router.get("/jobs/{job_id}", response_model=JobStatus)
def status(job_id: str):
    return call(service.status, job_id)


@router.post("/jobs/{job_id}/confirm", response_model=JobStatus)
def confirm(job_id: str, request: ConfirmRequest):
    return call(service.confirm, job_id, request.preview_id)


@router.post("/jobs/{job_id}/cancel", response_model=JobStatus)
def cancel(job_id: str):
    return call(service.cancel_job, job_id)


@router.get("/jobs/{job_id}/preview/{preview_id}")
def preview(job_id: str, preview_id: str):
    path = call(service.preview_path, job_id, preview_id)
    return FileResponse(path, media_type="image/png", headers={"Cache-Control": "no-store"})
