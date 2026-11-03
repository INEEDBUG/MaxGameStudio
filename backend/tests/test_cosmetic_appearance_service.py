from pathlib import Path
from types import SimpleNamespace
import threading
import time

import pytest

from app.cosmetic_studio.appearance import AppearanceError, AppearanceService
from app.cosmetic_studio.appearance_inventory import InventorySnapshot
from app.cosmetic_studio.appearance_models import (
    AppearanceApplyRequest, AppearanceInventoryResponse, AppearanceItem,
    AppearancePlayer, AppearanceReplacement, AppearanceSettings,
)
from app.cosmetic_studio.appearance_store import AppearanceStore
from app.cosmetic_studio.runtime import digest


TARGET = "76561199248524928"
OTHER = "76561199369875913"


def _snapshot(path, *, plans=None):
    demo_hash = digest(path)
    target_slot = "slot-target-0001"
    other_slot = "slot-other-00001"
    items = [
        AppearanceItem(slot_id=target_slot, xuid=TARGET, team="ct", definition=61,
            item_id="1001", name="USP-S", paint_index=0, wear=.2, seed=0,
            editable=True, preset_ids=["usp-kill-confirmed"]),
        AppearanceItem(slot_id=other_slot, xuid=OTHER, team="t", definition=61,
            item_id="2002", name="USP-S", paint_index=0, wear=.2, seed=0,
            editable=True, preset_ids=["usp-kill-confirmed"]),
    ]
    response = SimpleNamespace(
        demo_sha256=demo_hash,
        players=[AppearancePlayer(xuid=TARGET, name="Target"), AppearancePlayer(xuid=OTHER, name="Other")],
        items=items,
    )
    return InventorySnapshot(response, {target_slot: tuple(range(100, 228)), other_slot: tuple(range(300, 428))})


class CopyProvider:
    profiles = []

    def __init__(self, original, root, profile, executable, python, cancel, *, verifier, transport):
        self.original, self.root, self.profile = Path(original), Path(root), profile
        self.cancel, self.verifier = cancel, verifier
        type(self).profiles.append(profile)

    def load(self):
        self.root.mkdir(parents=True)

    def apply_profile(self, profile_digest):
        output = self.root / "playback.dem"
        output.write_bytes(self.original.read_bytes() + profile_digest.encode())
        self.artifact = SimpleNamespace(playback=output)


def _setup(tmp_path, monkeypatch, plans=None):
    demo = tmp_path / "source.dem"
    demo.write_bytes(b"immutable demo")
    core = tmp_path / "skin-core.exe"
    core.write_bytes(b"qualified fixture core placeholder")
    output = tmp_path / "out"
    output.mkdir()
    store = AppearanceStore(tmp_path / "store.json")
    store.save_settings(AppearanceSettings(core_exe=str(core), output_dir=str(output)))
    if plans:
        for xuid, selections in plans.items():
            store.save_player_plan(digest(demo), xuid, selections)
    monkeypatch.setattr("app.cosmetic_studio.appearance._verified_core", lambda _path: True, raising=False)
    CopyProvider.profiles.clear()
    service = AppearanceService(store, provider_factory=CopyProvider,
        inventory_reader=lambda _path, **_kwargs: _snapshot(demo))
    return demo, store, service


def test_whole_match_batches_new_owned_identities_without_weakening_core_limit(tmp_path, monkeypatch):
    demo, store, service = _setup(tmp_path, monkeypatch)
    first = _snapshot(demo).response.items[0]
    items = [first.model_copy(update={'slot_id':f'match-slot-{i:04d}', 'item_id':str(1001+i)})
             for i in range(18)]
    response = _snapshot(demo).response
    response.items = items
    snapshot = InventorySnapshot(response, {item.slot_id:tuple(range(100+i*200,228+i*200))
                                           for i,item in enumerate(items)})
    monkeypatch.setattr('app.cosmetic_studio.match_inventory.complete_player_inventory',
                        lambda *args:snapshot)
    proof_calls = []
    monkeypatch.setattr('app.cosmetic_studio.appearance.verify_rewrite',
                        lambda src,dst,profile,ticks:proof_calls.append((profile,ticks)) or True)

    class VerifiedCopy(CopyProvider):
        def apply_profile(self, profile_digest):
            super().apply_profile(profile_digest)
            assert self.verifier(self.original, self.artifact.playback, self.profile)

    service.provider_factory = VerifiedCopy
    result = service.run_job('d'*32, AppearanceApplyRequest(demo_path=str(demo),
        expected_demo_sha256=digest(demo),xuid=TARGET,scope='match',
        replacements=[AppearanceReplacement(slot_id=items[0].slot_id,
            preset_id='usp-kill-confirmed',wear=.2,seed=0)]))
    assert result.state == 'file_verified', result.error
    assert [len(profile.items) for profile,_ in proof_calls] == [16,2]
    assert [len(ticks) for _,ticks in proof_calls] == [2048,256]
    assert [item.item_id64 for profile,_ in proof_calls for item in profile.items] == [str(1001+i) for i in range(18)]
    assert len(result.resolved_plan) == len(store.load_plans(digest(demo))[TARGET]) == 18
    assert demo.read_bytes() == b'immutable demo'


def test_empty_replacements_restore_selected_player_and_rebuild_other_saved_plan(tmp_path, monkeypatch):
    other_plan = [AppearanceReplacement(slot_id="slot-other-00001", preset_id="usp-kill-confirmed", wear=.2, seed=7).model_dump()]
    demo, store, service = _setup(tmp_path, monkeypatch, plans={OTHER: other_plan,
        TARGET: [AppearanceReplacement(slot_id="slot-target-0001", preset_id="usp-kill-confirmed", wear=.2, seed=2).model_dump()]})
    request = AppearanceApplyRequest(demo_path=str(demo), expected_demo_sha256=digest(demo), xuid=TARGET, replacements=[])

    result = service.run_job("a" * 32, request)

    assert result.state == "file_verified"
    assert [profile.steam_id64 for profile in CopyProvider.profiles] == [OTHER]
    assert store.load_plans(digest(demo)) == {OTHER: other_plan}
    assert result.artifact.visual_verified is None
    assert result.artifact.original_sha256 == digest(demo)
    assert result.artifact.playback_sha256 == digest(Path(result.artifact.playback_path))
    assert demo.read_bytes() == b"immutable demo"
    assert AppearanceService(store, provider_factory=CopyProvider).status("a" * 32).artifact.playback_sha256 == result.artifact.playback_sha256


def test_rejects_stale_demo_fingerprint_without_touching_saved_plan_or_source(tmp_path, monkeypatch):
    demo, store, service = _setup(tmp_path, monkeypatch)
    plan = [AppearanceReplacement(slot_id="slot-target-0001", preset_id="usp-kill-confirmed", wear=.2, seed=2).model_dump()]
    request = AppearanceApplyRequest(demo_path=str(demo), expected_demo_sha256="0" * 64, xuid=TARGET, replacements=plan)

    result = service.run_job("b" * 32, request)

    assert result.state == "failed"
    assert "changed" in result.error.lower()
    assert result.items == []
    assert store.load_plans(digest(demo)) == {}
    assert demo.read_bytes() == b"immutable demo"
    assert CopyProvider.profiles == []


def test_rejects_selection_owned_by_another_xuid(tmp_path, monkeypatch):
    demo, store, service = _setup(tmp_path, monkeypatch)
    request = AppearanceApplyRequest(demo_path=str(demo), expected_demo_sha256=digest(demo), xuid=TARGET,
        replacements=[AppearanceReplacement(slot_id="slot-other-00001", preset_id="usp-kill-confirmed", wear=.2, seed=1)])

    result = service.run_job("c" * 32, request)

    assert result.state == "failed"
    assert "slot" in result.error.lower()
    assert store.load_plans(digest(demo)) == {}
    assert CopyProvider.profiles == []


@pytest.mark.parametrize("item_id,reason", [
    ("0", "missing_item_identity"),
    ("1001", "unverified_original_ownership"),
])
def test_observed_equipment_cannot_be_applied_by_forging_a_selection(tmp_path, monkeypatch, item_id, reason):
    demo, store, service = _setup(tmp_path, monkeypatch)
    snapshot = _snapshot(demo)
    snapshot.response.items[0] = snapshot.response.items[0].model_copy(update={
        "item_id": item_id, "editable": False, "preset_ids": [], "reason": reason,
    })
    snapshot.verification_ticks.pop("slot-target-0001")
    service.inventory_reader = lambda *_args, **_kwargs: snapshot
    request = AppearanceApplyRequest(demo_path=str(demo), expected_demo_sha256=digest(demo), xuid=TARGET,
        replacements=[AppearanceReplacement(slot_id="slot-target-0001", preset_id="usp-kill-confirmed", wear=.2, seed=1)])

    result = service.run_job("9" * 32, request)

    assert result.state == "failed"
    assert store.load_plans(digest(demo)) == {}
    assert CopyProvider.profiles == []
    assert demo.read_bytes() == b"immutable demo"


def test_qualified_materialized_usp_is_a_separate_verified_rewrite_step(tmp_path, monkeypatch):
    from app.cosmetic_studio.skin_core import CoreMaterializedUspItem
    demo, store, service = _setup(tmp_path, monkeypatch)
    snapshot = _snapshot(demo)
    snapshot.response.items[0] = snapshot.response.items[0].model_copy(update={"item_id": "0"})
    snapshot.response.items.append(AppearanceItem(slot_id="slot-target-awp", xuid=TARGET, team="ct",
        definition=9, item_id="3003", name="AWP", paint_index=0, wear=.2, seed=0,
        editable=True, preset_ids=["awp-dragon-lore"]))
    snapshot.verification_ticks["slot-target-awp"] = tuple(range(300, 428))
    service.inventory_reader = lambda *_args, **_kwargs: snapshot
    checked = []
    monkeypatch.setattr("app.cosmetic_studio.appearance._resolve_materialized_usp",
        lambda path, xuid, tick: checked.append((path, xuid, tick)) or True, raising=False)
    request = AppearanceApplyRequest(demo_path=str(demo), expected_demo_sha256=digest(demo), xuid=TARGET,
        replacements=[AppearanceReplacement(slot_id="slot-target-0001", preset_id="usp-kill-confirmed", wear=.01, seed=0),
                      AppearanceReplacement(slot_id="slot-target-awp", preset_id="awp-dragon-lore", wear=.01, seed=0)])
    result = service.run_job("7" * 32, request)
    assert result.state == "file_verified"
    assert checked == [(demo, TARGET, 100)]
    assert len(CopyProvider.profiles) == 2
    assert [profile.items[0].item_id64 for profile in CopyProvider.profiles] == ["3003", "0"]
    assert isinstance(CopyProvider.profiles[1].items[0], CoreMaterializedUspItem)
    assert len(store.load_plans(digest(demo))[TARGET]) == 2
    assert result.artifact.visual_verified is None
    assert demo.read_bytes() == b"immutable demo"


def test_materialized_usp_source_must_be_rechecked_before_running_any_provider(tmp_path, monkeypatch):
    demo, store, service = _setup(tmp_path, monkeypatch)
    snapshot = _snapshot(demo)
    snapshot.response.items[0] = snapshot.response.items[0].model_copy(update={"item_id": "0"})
    service.inventory_reader = lambda *_args, **_kwargs: snapshot
    monkeypatch.setattr("app.cosmetic_studio.appearance._resolve_materialized_usp", lambda *_args: False)
    request = AppearanceApplyRequest(demo_path=str(demo), expected_demo_sha256=digest(demo), xuid=TARGET,
        replacements=[AppearanceReplacement(slot_id="slot-target-0001", preset_id="usp-kill-confirmed", wear=.01, seed=0)])
    result = service.run_job("8" * 32, request)
    assert result.state == "failed"
    assert CopyProvider.profiles == []
    assert store.load_plans(digest(demo)) == {}
    assert demo.read_bytes() == b"immutable demo"


def test_cancelled_worker_blocks_a_second_rewrite_until_worker_exits(tmp_path, monkeypatch):
    entered, release = threading.Event(), threading.Event()

    class BlockingProvider(CopyProvider):
        def apply_profile(self, profile_digest):
            entered.set()
            assert release.wait(5)
            super().apply_profile(profile_digest)

    demo, store, service = _setup(tmp_path, monkeypatch)
    service.provider_factory = BlockingProvider
    request = AppearanceApplyRequest(demo_path=str(demo), expected_demo_sha256=digest(demo), xuid=TARGET,
        replacements=[AppearanceReplacement(slot_id="slot-target-0001", preset_id="usp-kill-confirmed", wear=.2, seed=2)])
    first = service.start(request)
    assert entered.wait(5)
    assert service.cancel(first.id).state == "cancelled"

    with pytest.raises(AppearanceError, match="already being built"):
        service.start(request)

    release.set()
    deadline = time.monotonic() + 5
    while service._jobs[first.id]["thread"].is_alive() and time.monotonic() < deadline:
        time.sleep(.01)
    assert not service._jobs[first.id]["thread"].is_alive()
    assert store.load_plans(digest(demo)) == {}


def test_atomic_commit_failure_keeps_prior_plan_and_reports_failed(tmp_path, monkeypatch):
    old_plan = [AppearanceReplacement(slot_id="slot-target-0001", preset_id="usp-kill-confirmed", wear=.25, seed=4).model_dump()]
    demo, store, service = _setup(tmp_path, monkeypatch, plans={TARGET: old_plan})
    original_write = store._write_unlocked
    calls = 0

    def fail_commit(state):
        nonlocal calls
        calls += 1
        if calls == 2:
            raise OSError("simulated atomic commit failure")
        return original_write(state)

    monkeypatch.setattr(store, "_write_unlocked", fail_commit)
    request = AppearanceApplyRequest(demo_path=str(demo), expected_demo_sha256=digest(demo), xuid=TARGET,
        replacements=[AppearanceReplacement(slot_id="slot-target-0001", preset_id="usp-kill-confirmed", wear=.2, seed=1)])

    result = service.run_job("e" * 32, request)

    assert result.state == "failed"
    assert "atomic" in result.error.lower()
    assert store.load_plans(digest(demo)) == {TARGET: old_plan}


def test_other_players_saved_plan_failure_is_not_attributed_to_submitted_player(tmp_path, monkeypatch):
    old_target = [AppearanceReplacement(slot_id="slot-target-0001", preset_id="usp-kill-confirmed", wear=.25, seed=4).model_dump()]
    old_other = [AppearanceReplacement(slot_id="slot-other-00001", preset_id="usp-kill-confirmed", wear=.3, seed=6).model_dump()]
    demo, store, service = _setup(tmp_path, monkeypatch, plans={TARGET: old_target, OTHER: old_other})

    class FailingOtherProvider(CopyProvider):
        def apply_profile(self, profile_digest):
            if self.profile.steam_id64 == OTHER:
                raise RuntimeError("saved other-player rewrite failed")
            super().apply_profile(profile_digest)

    service.provider_factory = FailingOtherProvider
    request = AppearanceApplyRequest(demo_path=str(demo), expected_demo_sha256=digest(demo), xuid=TARGET,
        replacements=[AppearanceReplacement(slot_id="slot-target-0001", preset_id="usp-kill-confirmed", wear=.2, seed=1)])
    result = service.run_job("1" * 32, request)

    assert result.state == "failed"
    assert "other-player" in result.error
    assert result.items == []
    assert store.load_plans(digest(demo)) == {TARGET: old_target, OTHER: old_other}


def test_inventory_exposes_only_persisted_artifact_whose_hashes_still_match(tmp_path):
    demo = tmp_path / "source.dem"
    playback = tmp_path / "playback.dem"
    demo.write_bytes(b"original")
    playback.write_bytes(b"verified copy")
    source_hash, playback_hash = digest(demo), digest(playback)
    artifact = {"job_id": "f" * 32, "original_path": str(demo), "playback_path": str(playback),
        "original_sha256": source_hash, "playback_sha256": playback_hash, "profile_sha256": "a" * 64,
        "state": "file_verified", "visual_verified": None}
    status = {"id": "f" * 32, "state": "file_verified", "items": [], "error": None, "artifact": artifact}
    store = AppearanceStore(tmp_path / "store.json")
    store.commit_verified_apply(source_hash, {}, artifact, status)
    response = AppearanceInventoryResponse(demo_sha256=source_hash, players=[], items=[], presets=[],
        settings=AppearanceSettings(), plans={})
    service = AppearanceService(store, inventory_reader=lambda *_args, **_kwargs: InventorySnapshot(response, {}))

    assert service.inventory(str(demo)).artifact.job_id == "f" * 32
    playback.write_bytes(b"tampered copy")
    assert service.inventory(str(demo)).artifact is None


def test_status_after_restart_recovers_verified_job_and_fails_incomplete_work(tmp_path):
    store = AppearanceStore(tmp_path / "store.json")
    store.save_job({"id": "2" * 32, "state": "running", "items": [], "error": None, "artifact": None})

    result = AppearanceService(store).status("2" * 32)

    assert result.state == "failed"
    assert "restarted" in result.error.lower()
    assert store.load_job("2" * 32)["state"] == "failed"


def test_first_apply_creates_only_the_configured_default_output_directory(tmp_path, monkeypatch):
    import app.cosmetic_studio.appearance_store as store_module

    demo, store, service = _setup(tmp_path, monkeypatch)
    default_output = tmp_path / "app-data" / "cosmetic_playbacks"
    monkeypatch.setattr(store_module, "default_output_dir", lambda: default_output)
    store.save_settings({"core_exe": str(tmp_path / "skin-core.exe"), "output_dir": str(default_output)})
    request = AppearanceApplyRequest(demo_path=str(demo), expected_demo_sha256=digest(demo), xuid=TARGET,
        replacements=[AppearanceReplacement(slot_id="slot-target-0001", preset_id="usp-kill-confirmed", wear=.2, seed=0)])

    result = service.run_job("3" * 32, request)

    assert result.state == "file_verified"
    assert default_output.is_dir()


def test_first_start_job_keeps_discovered_defaults_when_pending_job_creates_the_store(tmp_path, monkeypatch):
    import app.cosmetic_studio.appearance_store as store_module

    demo = tmp_path / "source.dem"
    demo.write_bytes(b"immutable first-run demo")
    core = tmp_path / "discovered-skin-core.exe"
    core.write_bytes(b"test core path")
    default_output = tmp_path / "new-app-data" / "cosmetic_playbacks"
    expected_settings = AppearanceSettings(core_exe=str(core), output_dir=str(default_output))
    monkeypatch.setattr(store_module, "default_output_dir", lambda: default_output)
    monkeypatch.setattr(store_module, "default_settings", lambda: expected_settings)
    store = AppearanceStore(tmp_path / "first-run-store.json")
    service = AppearanceService(store, provider_factory=CopyProvider,
        inventory_reader=lambda _path, **_kwargs: _snapshot(demo))
    request = AppearanceApplyRequest(demo_path=str(demo), expected_demo_sha256=digest(demo), xuid=TARGET,
        replacements=[AppearanceReplacement(slot_id="slot-target-0001", preset_id="usp-kill-confirmed", wear=.2, seed=0)])

    pending = service.start(request)
    deadline = time.monotonic() + 10
    result = service.status(pending.id)
    while result.state in {"pending", "running"} and time.monotonic() < deadline:
        time.sleep(.01)
        result = service.status(pending.id)

    assert result.state == "file_verified"
    assert store.load_settings() == expected_settings
    assert default_output.is_dir()
    assert store.load_plans(digest(demo))[TARGET][0]["preset_id"] == "usp-kill-confirmed"
    assert demo.read_bytes() == b"immutable first-run demo"
