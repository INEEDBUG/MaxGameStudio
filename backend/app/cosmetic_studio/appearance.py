"""File-only, Demo-bound appearance plan orchestration.

This module never starts CS2, OBS or a recorder. A successful job means only
that a distinct playback copy passed the existing parser/core verification.
"""
from __future__ import annotations

import shutil
import threading
import uuid
from pathlib import Path
from typing import Any

from .appearance_inventory import inspect_inventory
from .appearance_models import (
    AppearanceApplyRequest, AppearanceArtifact, AppearanceInventoryResponse,
    AppearanceJobStatus, AppearanceItemResult, AppearanceSettings,
)
from .appearance_store import AppearanceStore, AppearanceStoreError
from .catalog import GLOVE_PRESET_DATA, PRESET_DATA, source_weapon_id
from .core_inspection import verify_rewrite
from .runtime import digest
from .skin_core import CoreGloveItem, CoreItem, CoreMaterializedUspItem, CoreProfile, SkinCoreProvider, run_licensed_core


def _resolve_materialized_usp(path, xuid, tick):
    from .materialized_inspection import resolve_materialized_usp
    return resolve_materialized_usp(path, xuid, tick)


class AppearanceError(ValueError):
    pass


class AppearanceService:
    _MAX_LIVE_JOBS = 20
    def __init__(self, store: AppearanceStore | None = None, *, provider_factory=SkinCoreProvider,
                 core_transport=run_licensed_core, inventory_reader=inspect_inventory):
        self.store = store or AppearanceStore()
        self.provider_factory = provider_factory
        self.core_transport = core_transport
        self.inventory_reader = inventory_reader
        self._lock = threading.RLock()
        self._jobs: dict[str, dict[str, Any]] = {}

    def settings(self) -> AppearanceSettings:
        return self.store.load_settings()

    def update_settings(self, core_exe: str, output_dir: str) -> AppearanceSettings:
        return self.store.save_settings({"core_exe": core_exe, "output_dir": output_dir})

    def inventory(self, demo_path: str, *, xuid: str | None = None) -> AppearanceInventoryResponse:
        path = Path(demo_path).expanduser().resolve(strict=True)
        if path.suffix.lower() != ".dem" or not path.is_file():
            raise AppearanceError("Select an existing local Demo")
        source_hash = digest(path)
        stored_artifact = self.store.load_artifact(source_hash)
        plans = self.store.load_plans(source_hash)
        snapshot = self.inventory_reader(path, settings=self.settings(), plans=plans)
        if xuid is not None:
            from .match_inventory import complete_player_inventory
            if not any(player.xuid == xuid for player in snapshot.response.players):
                raise AppearanceError("目标玩家不在这份 Demo 中。")
            snapshot = complete_player_inventory(path, source_hash, xuid, snapshot)
        artifact = None
        if stored_artifact:
            job_id = stored_artifact.get("job_id")
            job = self.store.load_job(job_id) if isinstance(job_id, str) else None
            try:
                original = Path(stored_artifact["original_path"]).resolve(strict=True)
                playback = Path(stored_artifact["playback_path"]).resolve(strict=True)
                hashes_match = (stored_artifact["original_sha256"] == source_hash
                    and digest(original) == source_hash
                    and digest(playback) == stored_artifact["playback_sha256"])
            except (KeyError, OSError, TypeError):
                hashes_match = False
            if job and job.get("state") == "file_verified" and hashes_match:
                artifact = AppearanceArtifact.model_validate(stored_artifact)
        return snapshot.response.model_copy(update={"artifact": artifact})

    def status(self, job_id: str) -> AppearanceJobStatus:
        with self._lock:
            live = self._jobs.get(job_id)
            if live:
                return AppearanceJobStatus.model_validate(live["status"])
        stored = self.store.load_job(job_id)
        if not stored:
            raise KeyError(job_id)
        if stored.get("state") in {"pending", "running"}:
            stored.update(state="failed", error="Desktop service restarted before this file job completed")
            self.store.save_job(stored)
        return AppearanceJobStatus.model_validate(stored)

    def cancel(self, job_id: str) -> AppearanceJobStatus:
        with self._lock:
            live = self._jobs.get(job_id)
            if live:
                if live["status"]["state"] not in {"file_verified", "failed", "cancelled"}:
                    live["cancel"].set()
                    live["status"]["state"] = "cancelled"
                    live["status"]["error"] = "Cancelled by user"
                    self.store.save_job(live["status"])
                return AppearanceJobStatus.model_validate(live["status"])
        stored = self.store.load_job(job_id)
        if not stored:
            raise KeyError(job_id)
        if stored["state"] in {"pending", "running"}:
            stored.update(state="cancelled", error="Cancelled after service restart")
            self.store.save_job(stored)
        return AppearanceJobStatus.model_validate(stored)

    def start(self, request: AppearanceApplyRequest) -> AppearanceJobStatus:
        job_id = uuid.uuid4().hex
        state = {"id": job_id, "state": "pending", "items": [], "error": None, "artifact": None}
        cancel = threading.Event()
        with self._lock:
            for old_id, old_job in list(self._jobs.items()):
                worker = old_job.get("thread")
                if old_job["status"]["state"] in {"file_verified", "failed", "cancelled"} and not (worker and worker.is_alive()):
                    self._jobs.pop(old_id, None)
            if any(job["status"]["state"] in {"pending", "running"}
                   or (job.get("thread") is not None and job["thread"].is_alive())
                   for job in self._jobs.values()):
                raise AppearanceError("An appearance plan is already being built")
            if len(self._jobs) >= self._MAX_LIVE_JOBS:
                raise AppearanceError("Appearance job history is full; restart the desktop service")
            self._jobs[job_id] = {"status": state, "cancel": cancel}
            try:
                self.store.save_job(state)
            except Exception:
                self._jobs.pop(job_id, None)
                raise
            thread = threading.Thread(target=self._run, args=(job_id, request, cancel),
                                      name="appearance-file-job", daemon=True)
            self._jobs[job_id]["thread"] = thread
        thread.start()
        return AppearanceJobStatus.model_validate(state)

    def run_job(self, job_id: str, request: AppearanceApplyRequest, cancel: threading.Event | None = None) -> AppearanceJobStatus:
        """Synchronous worker entry, exposed for deterministic tests."""
        cancel = cancel or threading.Event()
        with self._lock:
            live = self._jobs.setdefault(job_id, {"status": {"id": job_id, "state": "pending", "items": [], "error": None, "artifact": None}, "cancel": cancel})
            live["cancel"] = cancel
            status = live["status"]
            if status["state"] == "cancelled":
                return AppearanceJobStatus.model_validate(status)
            status["state"] = "running"
            self.store.save_job(status)
        try:
            result = self._apply(request, job_id, cancel)
            with self._lock:
                if cancel.is_set():
                    status.update(state="cancelled", error="Cancelled by user")
                else:
                    status.update(state="file_verified", items=[item.model_dump() for item in result["items"]],
                                  artifact=result["artifact"], error=None)
                    if request.scope == 'match':
                        status.update(resolved_plan=result['plans'].get(request.xuid, []),
                                      skipped_foreign_items=result['skipped_foreign_items'])
                    self.store.commit_verified_apply(result["demo_sha256"], result["plans"],
                        result["artifact"], status)
        except Exception as exc:
            with self._lock:
                if cancel.is_set():
                    status.update(state="cancelled", error="Cancelled by user")
                else:
                    failure = str(exc)[:400]
                    # A batch rebuild can fail while applying a previously
                    # saved plan for another player; do not falsely attribute
                    # that failure to only the player submitted this request.
                    status.update(state="failed", error=failure, items=[])
                try:
                    self.store.save_job(status)
                except AppearanceStoreError:
                    pass
        return AppearanceJobStatus.model_validate(status)

    def _apply(self, request: AppearanceApplyRequest, job_id: str, cancel: threading.Event) -> dict[str, Any]:
        path = Path(request.demo_path).expanduser().resolve(strict=True)
        source_hash = digest(path)
        if source_hash != request.expected_demo_sha256:
            raise AppearanceError("Demo changed; inspect it again before applying")
        settings = self.settings()
        core_path, output_path = Path(settings.core_exe), Path(settings.output_dir)
        if not output_path.exists():
            from .appearance_store import default_output_dir
            if output_path.resolve() == default_output_dir().resolve():
                # First use may create only the app's configured, controlled
                # output directory, and only after the user explicitly saves.
                output_path.mkdir(parents=True, exist_ok=True)
        if (not core_path.is_file() or core_path.is_symlink() or not output_path.is_dir()
                or output_path.is_symlink()):
            raise AppearanceError("Configure the pinned local skin core and an existing output directory")

        snapshot = self.inventory_reader(path, settings=settings, plans=self.store.load_plans(source_hash))
        skipped_foreign_items = 0
        if request.scope == 'match':
            from .match_inventory import complete_player_inventory
            from .match_rules import expand_match_plan
            snapshot = complete_player_inventory(path, source_hash, request.xuid, snapshot)
            replacements, skipped_foreign_items = expand_match_plan(snapshot.response.items,
                request.xuid, request.replacements)
            request = request.model_copy(update={'replacements':replacements})
        items = {row.slot_id: row for row in snapshot.response.items}
        roster = {row.xuid for row in snapshot.response.players}
        if request.xuid not in roster:
            raise AppearanceError("Selected player is not in this Demo")

        all_plans = self.store.load_plans(source_hash)
        if request.replacements:
            if len(request.replacements) > 128:
                raise AppearanceError("A player plan exceeds the verified core item limit")
            all_plans[request.xuid] = [row.model_dump() for row in request.replacements]
        else:
            all_plans.pop(request.xuid, None)

        for player, plan in all_plans.items():
            if any(row['slot_id'] not in items for row in plan):
                from .match_inventory import complete_player_inventory
                snapshot=complete_player_inventory(path,source_hash,player,snapshot)
                items={row.slot_id:row for row in snapshot.response.items}

        selections_by_player: dict[str, list[dict[str, Any]]] = {}
        per_player_items: dict[str, list[CoreItem | CoreGloveItem | CoreMaterializedUspItem]] = {}
        per_player_ticks: dict[str, set[int]] = {}
        response_items: list[AppearanceItemResult] = []
        for xuid, selections in all_plans.items():
            if xuid not in roster:
                raise AppearanceError("A saved player plan is not bound to this Demo roster")
            weapon_items: list[CoreItem | CoreGloveItem | CoreMaterializedUspItem] = []
            ticks: set[int] = set()
            has_glove = False
            has_same_model_knife = False
            for raw in selections:
                selection = request_replacement(raw)
                item = items.get(selection.slot_id)
                if item is None or item.xuid != xuid or not item.editable:
                    raise AppearanceError("A selected owned-item slot is missing or not verified for editing")
                if selection.preset_id not in item.preset_ids:
                    raise AppearanceError("Selected finish does not match the observed source item")
                if selection.preset_id in GLOVE_PRESET_DATA:
                    has_glove = True
                    continue
                target, paint, _ = PRESET_DATA[selection.preset_id]
                source_definition = source_weapon_id(selection.preset_id)
                if item.definition != source_definition:
                    raise AppearanceError("Selected preset is not compatible with the actual source item")
                replacement = target if target != source_definition else None
                if item.item_id == "0":
                    window = snapshot.verification_ticks.get(item.slot_id, ())
                    if (source_definition != 61 or target != 61 or paint != 504 or item.team != "ct"
                            or not window or not _resolve_materialized_usp(path, xuid, min(window))):
                        raise AppearanceError("The zero-ID USP source is not independently verified")
                    weapon_items.append(CoreMaterializedUspItem(paint_kit=paint,
                        pattern_seed=selection.seed, wear=selection.wear))
                else:
                    weapon_items.append(CoreItem(item_id64=item.item_id, definition_index=source_definition,
                        replacement_definition_index=replacement, paint_kit=paint,
                        pattern_seed=selection.seed, wear=selection.wear))
                ticks.update(snapshot.verification_ticks.get(item.slot_id, ()))
                if item.definition == 507 and selection.preset_id == "karambit-fade":
                    has_same_model_knife = True
                response_items.append(AppearanceItemResult(slot_id=item.slot_id, preset_id=selection.preset_id, status="applied"))
            if has_glove:
                if not has_same_model_knife or any(item.replacement_definition_index is not None for item in weapon_items):
                    raise AppearanceError("The glove finish is qualified only with a same-model Karambit Fade plan")
                glove_item = next((items[row.slot_id] for row in map(request_replacement, selections)
                                   if row.preset_id in GLOVE_PRESET_DATA), None)
                if glove_item is None or glove_item.definition != 5030 or not glove_item.editable:
                    raise AppearanceError("The source glove is not parser-verified")
                knife = next((items[row.slot_id] for row in map(request_replacement, selections)
                              if row.preset_id == "karambit-fade"), None)
                if knife is None:
                    raise AppearanceError("The qualified same-model knife slot is missing")
                glove_id, source_definition = _resolve_glove(path, xuid, min(snapshot.verification_ticks[knife.slot_id]))
                if glove_id != glove_item.item_id or source_definition != glove_item.definition:
                    raise AppearanceError("The Pawn glove identity changed since inventory inspection")
                weapon_items.append(CoreGloveItem(item_id64=glove_id))
                ticks.update(snapshot.verification_ticks[knife.slot_id])
                response_items.append(AppearanceItemResult(slot_id=glove_item.slot_id,
                    preset_id="specialist-10061", status="applied"))
            if weapon_items:
                if not ticks or len(ticks) > 16_384:
                    raise AppearanceError("Selected item windows exceed the parser verification bound")
                per_player_items[xuid] = weapon_items
                per_player_ticks[xuid] = ticks
                selections_by_player[xuid] = selections

        if digest(path) != source_hash or cancel.is_set():
            raise AppearanceError("Cancelled or source Demo changed before rewrite")
        job_root = output_path / ("appearance-" + job_id)
        job_root.mkdir(exist_ok=False)
        current_source = path
        current_hash = source_hash
        final_path = None
        steps = []
        for xuid in sorted(per_player_items):
            ordinary = tuple(item for item in per_player_items[xuid] if not isinstance(item, CoreMaterializedUspItem))
            if ordinary:
                steps.extend((xuid, ordinary[offset:offset+16]) for offset in range(0,len(ordinary),16))
            steps.extend((xuid, (item,)) for item in per_player_items[xuid] if isinstance(item, CoreMaterializedUspItem))
        for index, (xuid, step_items) in enumerate(steps):
            if cancel.is_set():
                raise AppearanceError("Cancelled by user")
            root = job_root / f"step-{index:02d}"
            profile = CoreProfile(steam_id64=xuid, items=step_items)
            step_ids = {item.item_id64 for item in step_items}
            tick_list = sorted({tick for slot,window in snapshot.verification_ticks.items()
                if slot in items and items[slot].xuid==xuid and items[slot].item_id in step_ids
                for tick in window})
            def verifier(src, dst, spec, ticks=tick_list):
                if any(isinstance(item, CoreMaterializedUspItem) for item in spec.items):
                    from .materialized_inspection import verify_materialized_usp_rewrite
                    return verify_materialized_usp_rewrite(src, dst, spec, ticks)
                weapons = tuple(item for item in spec.items if not isinstance(item, CoreGloveItem))
                gloves = tuple(item for item in spec.items if isinstance(item, CoreGloveItem))
                if not weapons or not verify_rewrite(src, dst, CoreProfile(steam_id64=spec.steam_id64, items=weapons), ticks):
                    return False
                if gloves:
                    from .glove_inspection import verify_glove_rewrite
                    glove = gloves[0]
                    return verify_glove_rewrite(src, dst, spec.steam_id64, glove.item_id64,
                        glove.definition_index, glove.replacement_definition_index, glove.paint_kit,
                        glove.pattern_seed, glove.wear, ticks)
                return True
            provider = self.provider_factory(current_source, root, profile, core_path, Path(__import__("sys").executable),
                cancel, verifier=verifier, transport=self.core_transport)
            provider.load()
            provider.apply_profile(profile.sha256())
            final_path = provider.artifact.playback
            if digest(current_source) != current_hash or digest(path) != source_hash:
                raise AppearanceError("Source or staged Demo changed during appearance rewrite")
            current_source = final_path
            current_hash = digest(final_path)

        if final_path is None:
            final_path = job_root / "playback.dem"
            shutil.copyfile(path, final_path)
            if digest(path) != source_hash or digest(final_path) != source_hash:
                raise AppearanceError("Unmodified playback copy failed its hash check")
        if digest(path) != source_hash:
            raise AppearanceError("Original Demo changed; result rejected")
        playback_hash = digest(final_path)
        profile_hash = _plan_digest(all_plans)
        artifact = {
            "job_id": job_id, "original_path": str(path), "playback_path": str(final_path),
            "original_sha256": source_hash, "playback_sha256": playback_hash,
            "profile_sha256": profile_hash, "state": "file_verified", "visual_verified": None,
        }
        return {"demo_sha256": source_hash, "plans": all_plans, "artifact": artifact,
                "items": response_items, "skipped_foreign_items":skipped_foreign_items}

    def _run(self, job_id: str, request: AppearanceApplyRequest, cancel: threading.Event) -> None:
        self.run_job(job_id, request, cancel)


def request_replacement(value: Any):
    from .appearance_models import AppearanceReplacement
    return AppearanceReplacement.model_validate(value)


def _resolve_glove(path: Path, xuid: str, tick: int) -> tuple[str, int]:
    from .glove_inspection import resolve_owned_glove
    return resolve_owned_glove(path, xuid, tick)


def _plan_digest(plans: dict[str, list[dict[str, Any]]]) -> str:
    import hashlib, json
    encoded = json.dumps(plans, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()
    return hashlib.sha256(encoded).hexdigest()


service = AppearanceService()
