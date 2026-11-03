"""One explicitly requested experimental job; normal replay queues stay unchanged."""
from __future__ import annotations

import asyncio
from dataclasses import dataclass, field
import json
import math
import os
import re
from pathlib import Path
import shutil
import subprocess
import sys
import threading
import time
import uuid

from ..runtime_session import claim_runtime_session, release_runtime_session
from . import native
from .capture import capture_preview
from .models import JobRequest, JobStatus, PRESETS
from .catalog import PRESET_DATA, source_weapon_id
from .provider import CosmeticError, NskinzProvider
from .recorder import IsolatedRecorder
from .runtime import NativeSession, PINS, digest, write_json

TERMINAL = {"complete", "failed", "cancelled"}


def _build_core_profile(request):
    from .core_inspection import resolve_owned_awp, resolve_owned_weapon
    from .skin_core import CoreItem, CoreGloveItem, CoreProfile

    target_definition = PRESET_DATA[request.preset_id][0]
    source_definition = source_weapon_id(request.preset_id)
    item_id = (resolve_owned_awp(Path(request.demo_path), request.xuid, request.start_tick - 64)
        if source_definition == 9 else resolve_owned_weapon(Path(request.demo_path), request.xuid,
            request.start_tick - 64, source_definition))
    weapon = CoreItem(item_id64=item_id, definition_index=source_definition,
        replacement_definition_index=(target_definition if target_definition != source_definition else None),
        paint_kit=PRESETS[request.preset_id], pattern_seed=request.pattern_seed, wear=request.wear)
    items = [weapon]
    if request.glove_preset_id:
        from .glove_inspection import resolve_owned_glove
        glove_id, glove_definition = resolve_owned_glove(Path(request.demo_path),
            request.xuid, request.start_tick - 64)
        if glove_definition != 5030:
            raise CosmeticError('原始手套型号尚未验证，不会尝试替换')
        items.append(CoreGloveItem(item_id64=glove_id))
    return CoreProfile(steam_id64=request.xuid, items=tuple(items))


def validate_capture_video(metadata: dict) -> float:
    try:
        duration = float(metadata['format']['duration'])
        videos = [stream for stream in metadata['streams'] if stream.get('codec_type') == 'video']
        valid = (math.isfinite(duration) and 1 <= duration <= 40 and len(videos) == 1
                 and videos[0].get('width') == 1280 and videos[0].get('height') == 720)
    except (KeyError, TypeError, ValueError, OverflowError):
        valid = False
    if not valid:
        raise CosmeticError('录制视频尺寸、视频流或时长不符合实验范围')
    return duration


def validate_capture_audio(metadata: dict, diagnostics: bytes):
    if not any(stream.get('codec_type') == 'audio' for stream in metadata.get('streams', [])):
        raise CosmeticError('录制缺少游戏音轨，不会标记为成功')
    matches = re.findall(rb'max_volume:\s*(-?\d+(?:\.\d+)?)\s*dB', diagnostics)
    means = re.findall(rb'mean_volume:\s*(-?\d+(?:\.\d+)?)\s*dB', diagnostics)
    # Deliberately conservative acceptance for 2–30s kill-window experiments,
    # not a universal audio/mastering standard. Quiet clips remain as raw
    # evidence rather than silently entering the ready library. Qualified
    # AK/USP fixture recordings are substantially above these floors.
    if (not matches or not means or not math.isfinite(float(matches[-1]))
            or not math.isfinite(float(means[-1])) or float(matches[-1]) <= -60
            or float(means[-1]) <= -50):
        raise CosmeticError('游戏音轨静音或状态未知，不会标记为成功')


@dataclass
class Job:
    request: JobRequest
    status: JobStatus
    root: Path
    claim: str
    cancel: threading.Event = field(default_factory=threading.Event)
    approved: threading.Event = field(default_factory=threading.Event)
    lock: threading.RLock = field(default_factory=threading.RLock)
    preview_at: float = 0
    preview_sha256: str | None = None
    thread: threading.Thread | None = None
    cleanup_verified: bool = False
    session: object | None = None
    recorder: object | None = None
    file_artifact: object | None = None


class CosmeticService:
    def __init__(self, *, session_factory=NativeSession, recorder_factory=IsolatedRecorder, db_path=None,
                 core_transport=None):
        self._jobs: dict[str, Job] = {}
        self._lock = threading.Lock()
        self.session_factory, self.recorder_factory = session_factory, recorder_factory
        self.db_path = db_path
        # Dependency injection for isolated qualification/tests, never an HTTP
        # or inherited environment option. The file path defaults to the pinned
        # licensed compatibility transport; tests may replace it explicitly.
        self.core_transport = core_transport

    def start(self, request: JobRequest) -> JobStatus:
        if os.name != "nt":
            raise CosmeticError("离线制片实验仅支持 Windows")
        output = Path(request.tools.output_dir)
        if not output.is_dir():
            raise CosmeticError("请选择已存在的输出文件夹")
        resolved = native.final_path(str(output))
        if native.comparable_path(resolved).startswith("//"):
            raise CosmeticError("输出不支持网络目录")
        resolved = Path(resolved.removeprefix("\\\\?\\"))
        if resolved.drive.lower() == os.environ.get("SystemDrive", "C:").lower():
            raise CosmeticError("请将实验文件保存到非系统盘")
        if shutil.disk_usage(resolved).free < 2 * 1024**3:
            raise CosmeticError("实验副本和视频需要至少 2 GB 可用空间")
        claim = claim_runtime_session("/api/cosmetic-studio/jobs")
        job_id = uuid.uuid4().hex
        root = resolved / ("cosmetic-" + job_id)
        job = Job(request, JobStatus(id=job_id, state="preparing", message="正在校验离线工具、Demo 和目标片段…"), root, claim)
        try:
            root.mkdir(exist_ok=False)
            write_json(root / "job.json", {"id": job_id, "request": request.model_dump(), "experimental": True, "visual_layer_only": True})
            with self._lock:
                # Retain bounded in-memory history; evidence stays under chosen output.
                for key in list(self._jobs):
                    if len(self._jobs) >= 20 and self._jobs[key].status.state in TERMINAL and self._jobs[key].cleanup_verified:
                        self._jobs.pop(key)
                self._jobs[job_id] = job
            job.thread = threading.Thread(target=self._run, args=(job,), name="cosmetic-offline-job", daemon=True)
            job.thread.start()
        except Exception:
            release_runtime_session(claim)
            raise
        return self.status(job_id)

    def get(self, job_id: str) -> Job:
        with self._lock:
            if job_id not in self._jobs:
                raise KeyError(job_id)
            return self._jobs[job_id]

    def active(self):
        with self._lock:
            jobs = list(self._jobs.values())
        for job in reversed(jobs):
            with job.lock:
                if job.status.state not in TERMINAL or not job.cleanup_verified:
                    return job.status.model_copy(deep=True)
        return None

    def status(self, job_id):
        job = self.get(job_id)
        with job.lock:
            return job.status.model_copy(deep=True)

    def confirm(self, job_id, preview_id):
        job = self.get(job_id)
        with job.lock:
            if (job.status.state != "preview" or job.status.preview_id != preview_id
                    or job.approved.is_set() or job.cancel.is_set() or time.monotonic()-job.preview_at > 180):
                raise CosmeticError("此预览已失效或已确认，请勿重复提交")
            if digest(job.root / "evidence/preview.png") != job.preview_sha256:
                raise CosmeticError("预览文件已变化")
            job.approved.set()
            job.status.message = "已收到画面确认，正在复核当前会话…"
            return job.status.model_copy(deep=True)

    def cancel_job(self, job_id):
        job = self.get(job_id)
        with job.lock:
            if job.status.state not in TERMINAL:
                job.cancel.set()
                job.status.message = "正在停止专用制片会话并核对清理结果…"
            return job.status.model_copy(deep=True)

    def preview_path(self, job_id, preview_id):
        job = self.get(job_id)
        with job.lock:
            if job.status.state != "preview" or job.status.preview_id != preview_id:
                raise CosmeticError("预览已失效")
            path = job.root / "evidence/preview.png"
            if digest(path) != job.preview_sha256:
                raise CosmeticError("预览文件已变化")
            return path

    def update(self, job, state, message, **fields):
        with job.lock:
            values = job.status.model_dump()
            values.update(state=state, message=message, **fields)
            job.status = JobStatus.model_validate(values)

    def _run(self, job):
        session = provider = recorder = None
        result = error = None
        clean = True
        try:
            if job.request.expected_demo_sha256 and digest(Path(job.request.demo_path)) != job.request.expected_demo_sha256:
                raise CosmeticError('Demo 已在检查后变化，请重新检查；不会继续启动')
            from .inspection import resolve_slot
            weapon_id = PRESET_DATA[job.request.preset_id][0]
            source_definition = source_weapon_id(job.request.preset_id)
            if source_definition == 507:
                # A knife showcase need not contain a knife kill. Independently
                # bind every requested tick plus preroll to the same owned,
                # alive, exact-definition item before calling any native tool.
                from .intervals import validate_interval
                validate_interval(Path(job.request.demo_path), job.request.xuid,
                    job.request.start_tick, job.request.end_tick, source_definition)
            slot = (resolve_slot(Path(job.request.demo_path), job.request.xuid,
                        job.request.start_tick - 64)
                if source_definition == 9 else resolve_slot(Path(job.request.demo_path),
                    job.request.xuid, job.request.start_tick - 64, weapon_id=source_definition))
            playback_request = job.request
            if job.request.provider_id == "skin_core":
                from .core_inspection import verify_profile_rewrite
                from .skin_core import SkinCoreProvider, run_licensed_core
                self.update(job, "preparing", "正在生成独立制片副本并检查数据；原始 Demo 保持不变…")
                profile = _build_core_profile(job.request)
                ticks = list(range(job.request.start_tick - 64, job.request.end_tick + 1))
                provider = SkinCoreProvider(Path(job.request.demo_path), job.root / "appearance", profile,
                    Path(job.request.tools.core_exe), Path(sys.executable), job.cancel,
                    transport=self.core_transport or run_licensed_core,
                    verifier=lambda src, dst, spec: verify_profile_rewrite(src, dst, spec, ticks))
                provider.load()
                if (job.request.expected_demo_sha256
                        and provider.source_hash != job.request.expected_demo_sha256):
                    raise CosmeticError('原始 Demo 已在任务准备期间变化，停止制片')
                provider.apply_profile(profile.sha256())
                job.file_artifact = provider.artifact
                playback_request = job.request.model_copy(update={"demo_path": str(provider.artifact.playback),
                    'expected_demo_sha256': provider.artifact.playback_sha256})
            session = self.session_factory(playback_request, job.root, slot, job.cancel)
            job.session = session
            session.prepare()
            session.check_cancel()
            recorder = self.recorder_factory(session)
            job.recorder = recorder
            recorder.prepare()  # Slow copy happens before any game/DLL launch.
            session.check_cancel()
            self.update(job, "preparing", "正在启动专用离线 Demo，普通游戏不会被接管…")
            session.launch()
            if job.file_artifact:
                def fresh_preview(artifact):
                    session.validate_context(paused=True)
                    return (artifact.playback == session.demo and artifact.playback_sha256 == session.demo_hash
                            and job.approved.is_set() and not job.cancel.is_set()
                            and 0 <= time.monotonic() - job.preview_at <= 180
                            and digest(job.root / "evidence/preview.png") == job.preview_sha256)
                def stop_playback():
                    session.stop()
                    return session._restored is True
                provider.preview_check, provider.stop_playback = fresh_preview, stop_playback
                session.prepare_file_playback(job.file_artifact.playback_sha256)
            else:
                provider = NskinzProvider(session)
                provider.load()
                provider.apply_profile(session.baseline["profile_sha256"])
            capture_preview(session, job.root / "evidence/preview.png")
            with job.lock:
                job.preview_at = time.monotonic()
                job.preview_sha256 = digest(job.root / "evidence/preview.png")
                preview_id = uuid.uuid4().hex
                self.update(job, "preview", "请核对原生画面的玩家和所选武器外观；确认后才会录制。三分钟未确认将自动退出。",
                            preview_id=preview_id, preview_url=f"/api/cosmetic-studio/jobs/{job.status.id}/preview/{preview_id}")
            while not job.approved.wait(.25):
                session.validate_context(paused=True)
                if time.monotonic()-job.preview_at > 180:
                    raise CosmeticError("预览确认超时，已停止制片")
            session.validate_context(paused=True)
            if digest(job.root / "evidence/preview.png") != job.preview_sha256:
                raise CosmeticError("已确认的预览文件发生变化")
            provider.confirm_preview()
            if not provider.check_ready():
                raise CosmeticError("录制前状态复核失败")
            self.update(job, "recording", "正在录制已确认的原生 Demo 画面…", preview_url=None, preview_id=None)
            recorder.launch()
            source = recorder.record()
            recorder.close()
            recorder = None
            job.recorder = None
            provider.unload()
            provider = None
            job.cleanup_verified = True
            self.update(job, "exporting", "原生会话已退出，正在校验视频并生成 LiteCut 工程…")
            result = self.export(job, session, source)
        except Exception as exc:
            error = str(exc) or type(exc).__name__
            from .owned_process import CoreCleanupError
            if isinstance(exc, CoreCleanupError):
                clean = False
        finally:
            if recorder:
                try:
                    recorder.close()
                except Exception as exc:
                    clean = False
                    error = (error or "录制失败") + "；OBS 清理未确认：" + str(exc)
            if session:
                try:
                    session.stop()
                except Exception as exc:
                    clean = False
                    error = (error or "制片失败") + "；游戏清理未确认：" + str(exc)
            if job.file_artifact:
                try:
                    if digest(job.file_artifact.original) != job.file_artifact.original_sha256:
                        raise CosmeticError("原始 Demo 已变化，不能确认本次制片结果")
                except Exception as exc:
                    error = (error or "制片失败") + "；" + str(exc)
            job.cleanup_verified = clean
            if clean:
                release_runtime_session(job.claim)
            # Unknown cleanup deliberately keeps ordinary CS2/OBS ownership blocked.
            if error:
                state = "cancelled" if job.cancel.is_set() and clean else "failed"
                self.update(job, state, error, preview_id=None, preview_url=None)
            else:
                self.update(job, "complete", "录制与 LiteCut 工程已生成；仅本次人工确认的离线外观生效。", **result)
            write_json(job.root / "result.json", {**job.status.model_dump(), "cleanup_verified": clean,
                                                  "automated_visual_ack": None, "experimental": True})

    def export(self, job, session, source):
        from ..lite_cut.composer import export_lite_cut_project
        from ..lite_cut.models import LiteCutProjectBody
        session.check_cancel()
        tools = job.root / "media-tools"
        tools.mkdir(exist_ok=False)
        for name in ("ffmpeg", "ffprobe"):
            shutil.copy2(session.paths[name + "_exe"], tools / (name + ".exe"))
            if digest(tools / (name + ".exe")) != PINS[name + "_exe"]:
                raise CosmeticError("媒体工具副本校验失败；停止导出")
        ffmpeg, ffprobe = tools / "ffmpeg.exe", tools / "ffprobe.exe"
        metadata = json.loads(subprocess.run([str(ffprobe), "-v", "error", "-show_streams", "-show_format", "-of", "json", str(source)],
                                             capture_output=True, check=True, timeout=30, env=session.env,
                                             creationflags=subprocess.CREATE_NO_WINDOW).stdout)
        duration = validate_capture_video(metadata)
        decoded = subprocess.run([str(ffmpeg), "-v", "info", "-xerror", "-i", str(source),
                                  "-vf", "blackdetect=d=0.05:pic_th=0.995:pix_th=0.02", '-af', 'volumedetect', "-f", "null", "-"],
                                 capture_output=True, check=True, timeout=90, env=session.env,
                                 creationflags=subprocess.CREATE_NO_WINDOW)
        # The qualified daylight Demo fixture must not contain blank capture
        # intervals. Do not publish a decodable-but-black recording as ready.
        if b"black_start:" in decoded.stderr:
            raise CosmeticError("录制出现黑屏帧，已保留原始素材但不会标记成功或加入 LiteCut")
        validate_capture_audio(metadata, decoded.stderr)
        cosmetic = {"provider": job.request.provider_id, "target_xuid": job.request.xuid,
                     "weapon_id": PRESET_DATA[job.request.preset_id][0],
                     "source_weapon_id": source_weapon_id(job.request.preset_id),
                     "paint_kit": PRESETS[job.request.preset_id],
                    "pattern_seed": job.request.pattern_seed,
                    "glove_preset_id": job.request.glove_preset_id,
                    "wear": job.request.wear if job.file_artifact else .0000001,
                    "demo_sha256": job.file_artifact.original_sha256 if job.file_artifact else session.demo_hash,
                    "playback_sha256": session.demo_hash, "name_policy": "core_defined" if job.file_artifact else "preserve",
                    "invocation_mode": "licensed_compatibility" if job.file_artifact and self.core_transport is None else "local_qualification",
                    "hud_mode": "death_notices_only" if job.file_artifact else "unchanged",
                    "visual_layer_only": True, "verification": "human_preview", "preview_sha256": job.preview_sha256,
                    "automated_visual_ack": None, "job_id": job.status.id,
                    "demo_ticks": [job.request.start_tick, job.request.end_tick], "source_sha256": digest(source)}
        body = LiteCutProjectBody.model_validate({"schema_version": 2,
            "output": {"dir": str(job.root), "filename": "cosmetic-litecut.mp4", "width": 1280, "height": 720, "fps": 30, "encoder": "libx264"},
            "tracks": [{"id": "native-demo", "type": "video", "label": "Demo 外观制片", "clips": [
                {"id": job.status.id, "source_type": "file", "file_path": str(source), "trim_in": 0.1,
                 "trim_out": max(.2, duration-.1), "meta": {"cosmetic_studio": cosmetic}}]}]}).model_dump(mode="json")
        output = export_lite_cut_project(ffmpeg_bin=ffmpeg, project_body=body, clip_path_by_id={},
                                        output_path_str=str(job.root / "cosmetic-litecut.mp4"),
                                        montage_encoder="libx264", cancel_event=job.cancel)
        session.check_cancel()
        final_metadata = json.loads(subprocess.run([str(ffprobe), '-v', 'error', '-show_streams',
            '-show_format', '-of', 'json', str(output)], capture_output=True, check=True,
            timeout=30, env=session.env, creationflags=subprocess.CREATE_NO_WINDOW).stdout)
        validate_capture_video(final_metadata)
        final_decode = subprocess.run([str(ffmpeg), "-v", "info", "-xerror", "-i", str(output),
            '-vf', 'blackdetect=d=0.05:pic_th=0.995:pix_th=0.02', '-af', 'volumedetect', "-f", "null", "-"],
            capture_output=True, check=True, timeout=90, env=session.env, creationflags=subprocess.CREATE_NO_WINDOW)
        if b'black_start:' in final_decode.stderr:
            raise CosmeticError('最终 LiteCut 视频出现黑屏帧，不会标记为成功')
        validate_capture_audio(final_metadata, final_decode.stderr)
        project_id = asyncio.run(self.store_material(job, source, body, duration, cosmetic))
        write_json(job.root / "litecut-project.json", body)
        return {"output_path": str(output), "litecut_project_id": project_id}

    async def store_material(self, job, source, body, duration, cosmetic):
        from ..montage_db import MontageDB
        from ..lite_cut.db import LiteCutDB
        from ..env_utils import resolve_config_path
        database = self.db_path or resolve_config_path().parent / "cs2-insight.db"
        media, editor = MontageDB(database), LiteCutDB(database)
        await media.init_tables()
        await editor.init_tables()
        row_id = await media.insert_recorded_clip(clip_id="cosmetic-" + job.status.id,
            demo_path=job.request.demo_path, demo_filename=Path(job.request.demo_path).name,
            player_name=job.request.xuid, output_path=str(source), duration_sec=duration,
            status="ready", clip_meta={"cosmetic_studio": cosmetic})
        clip = body["tracks"][0]["clips"][0]
        clip.update(source_type="recorded_clip", source_id=row_id, file_path=None)
        # A material row remains useful if project creation fails; never delete its video.
        return await editor.create_project(name="Demo 外观制片 " + job.status.id[:8], body=body)

    def shutdown(self, timeout=25):
        with self._lock:
            jobs = list(self._jobs.values())
        for job in jobs:
            if job.status.state not in TERMINAL:
                job.cancel.set()
        deadline = time.monotonic() + timeout
        for job in jobs:
            if job.thread:
                job.thread.join(timeout=max(0, deadline-time.monotonic()))
        return all((not job.thread or not job.thread.is_alive()) and job.cleanup_verified for job in jobs)


service = CosmeticService()
