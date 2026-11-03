"""File-only queue preflight. No game/OBS launch and no shared-plan mutation."""
import asyncio
import hashlib
import json
import threading
import uuid
from pathlib import Path

from ..cosmetic_studio.appearance import AppearanceService
from ..cosmetic_studio.appearance_models import AppearanceApplyRequest, AppearanceSettings
from ..cosmetic_studio.appearance_store import AppearanceStore, default_output_dir
from ..cosmetic_studio.runtime import digest


def _check_receipt(receipt, source, source_hash, root):
    copy = Path(receipt['playback_path']).resolve(strict=True)
    if (receipt.get('state') != 'file_verified'
            or Path(receipt['original_path']).resolve() != source
            or receipt.get('original_sha256') != source_hash
            or digest(source) != source_hash
            or copy == source or not copy.is_relative_to(root)
            or copy.suffix.lower() != '.dem'
            or digest(copy) != receipt.get('playback_sha256')):
        raise ValueError('录制外观副本校验失败，未使用原皮肤代替。')
    return copy


def prepare_appearances(requests, cancel, *, settings=None, service_factory=AppearanceService):
    if len({dto.request_id for dto in requests}) != len(requests):
        raise ValueError('队列片段编号重复，请重新添加片段。')
    prepared, receipts, cache = [], {}, {}
    for dto in requests:
        if cancel.is_set(): raise ValueError('录制准备已取消。')
        profile = dto.demo.appearance
        if profile is None:
            prepared.append(dto); continue
        source = Path(dto.demo.demo_path).resolve(strict=True)
        source_hash = digest(source)
        if source_hash != profile.expected_demo_sha256:
            raise ValueError('Demo 已变化，请重新设置录制外观。未开始录制。')
        apply = AppearanceApplyRequest(demo_path=str(source), expected_demo_sha256=source_hash,
            xuid=dto.target_player.steamid64, replacements=profile.replacements, scope=profile.scope)
        key = hashlib.sha256(json.dumps(apply.model_dump(), sort_keys=True).encode()).hexdigest()
        if key not in cache:
            if len(cache) >= 32: raise ValueError('一次最多准备 32 种不同录制外观，请分批录制。')
            config = AppearanceSettings.model_validate(settings) if settings else AppearanceStore().load_settings()
            base = Path(config.output_dir).resolve()
            if not base.exists() and base == default_output_dir().resolve(): base.mkdir(parents=True)
            if not base.is_dir(): raise ValueError('录制外观保存目录不可用。')
            root = base / ('queue-appearance-' + uuid.uuid4().hex)
            root.mkdir()
            store = AppearanceStore(root / 'appearance.json')
            store.save_settings(config.model_copy(update={'output_dir':str(root)}))
            result = service_factory(store).run_job(uuid.uuid4().hex, apply, cancel)
            if cancel.is_set(): raise ValueError('录制准备已取消。')
            if result.state != 'file_verified' or not result.artifact:
                raise ValueError('录制外观未应用成功：' + (result.error or '状态未确认'))
            receipt = result.artifact.model_dump()
            copy = _check_receipt(receipt, source, source_hash, root)
            cache[key] = (copy, receipt, root)
        copy, receipt, root = cache[key]
        _check_receipt(receipt, source, source_hash, root)
        prepared.append(dto.model_copy(update={'demo':dto.demo.model_copy(update={'demo_path':str(copy)})}))
        receipts[dto.request_id] = {**receipt, 'target_xuid':apply.xuid,
            'selection': profile.model_dump(), 'visual_verified':None}
    if cancel.is_set(): raise ValueError('录制准备已取消。')
    return prepared, receipts


async def prepare_queue_appearances(requests, abort):
    cancel = threading.Event()
    if abort.is_set(): raise ValueError('录制准备已取消。')
    task = asyncio.create_task(asyncio.to_thread(prepare_appearances, requests, cancel))
    try:
        while not task.done():
            if abort.is_set(): cancel.set()
            await asyncio.wait({task}, timeout=.2)
        result = await task
        if abort.is_set(): raise ValueError('录制准备已取消。')
        return result
    except BaseException:
        cancel.set()
        # Cancellation must not orphan a rewriting child in the background.
        try: await asyncio.shield(task)
        except BaseException: pass
        raise
