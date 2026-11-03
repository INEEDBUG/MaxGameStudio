import json

import pytest

from app.cosmetic_studio.appearance_store import AppearanceStore, AppearanceStoreError


def test_store_round_trips_settings_and_per_player_plans_without_overwriting_other_players(tmp_path):
    store = AppearanceStore(tmp_path / "appearance.json")
    store.save_settings({"core_exe": "D:/Tools/skin-core.exe", "output_dir": "D:/Data/playbacks"})
    store.save_player_plan("a" * 64, "76561199369875913", [{"slot_id": "slot-0001", "preset_id": "karambit-fade", "wear": 0.03, "seed": 5}])
    store.save_player_plan("a" * 64, "76561198000000000", [{"slot_id": "slot-0002", "preset_id": "awp-dragon-lore", "wear": 0.01, "seed": 0}])
    store.save_player_plan("b" * 64, "76561199369875913", [{"slot_id": "slot-0003", "preset_id": "ak47-fire-serpent", "wear": 0.1, "seed": 1}])

    reopened = AppearanceStore(store.path)
    assert reopened.load_settings().model_dump() == {"core_exe": "D:/Tools/skin-core.exe", "output_dir": "D:/Data/playbacks"}
    assert reopened.load_plans("a" * 64) == {
        "76561199369875913": [{"slot_id": "slot-0001", "preset_id": "karambit-fade", "wear": 0.03, "seed": 5}],
        "76561198000000000": [{"slot_id": "slot-0002", "preset_id": "awp-dragon-lore", "wear": 0.01, "seed": 0}],
    }
    assert reopened.load_plans("b" * 64)["76561199369875913"][0]["preset_id"] == "ak47-fire-serpent"


def test_store_refuses_corrupt_file_instead_of_replacing_it(tmp_path):
    path = tmp_path / "appearance.json"
    original = b"{not valid json"
    path.write_bytes(original)
    store = AppearanceStore(path)

    with pytest.raises(AppearanceStoreError, match="corrupt"):
        store.save_settings({"core_exe": "D:/Tools/skin-core.exe", "output_dir": "D:/Data"})
    assert path.read_bytes() == original


def test_store_atomic_replace_failure_preserves_previous_configuration(tmp_path, monkeypatch):
    import app.cosmetic_studio.appearance_store as module

    path = tmp_path / "appearance.json"
    store = AppearanceStore(path)
    store.save_settings({"core_exe": "D:/Tools/old.exe", "output_dir": "D:/Data/old"})
    original = path.read_bytes()
    monkeypatch.setattr(module.os, "replace", lambda *args: (_ for _ in ()).throw(OSError("replace failed")))

    with pytest.raises(AppearanceStoreError, match="save"):
        store.save_settings({"core_exe": "D:/Tools/new.exe", "output_dir": "D:/Data/new"})
    assert path.read_bytes() == original


def test_store_rejects_unknown_fields_and_oversized_or_invalid_data(tmp_path):
    path = tmp_path / "appearance.json"
    path.write_text(json.dumps({"schema_version": 1, "settings": {}, "plans": {}, "extra": True}), encoding="utf-8")
    with pytest.raises(AppearanceStoreError, match="corrupt"):
        AppearanceStore(path).load_plans("a" * 64)


def test_store_does_not_create_or_write_default_file_at_import(monkeypatch, tmp_path):
    import app.cosmetic_studio.appearance_store as module

    monkeypatch.setattr(module, "default_store_path", lambda: tmp_path / "not-created.json")
    assert not (tmp_path / "not-created.json").exists()


def test_store_preserves_explicitly_cleared_settings_instead_of_reapplying_defaults(tmp_path, monkeypatch):
    import app.cosmetic_studio.appearance_store as module
    from app.cosmetic_studio.appearance_models import AppearanceSettings

    monkeypatch.setattr(module, "default_settings", lambda: AppearanceSettings(core_exe="D:/Tools/discovered.exe", output_dir="D:/Data/default"))
    store = AppearanceStore(tmp_path / "appearance.json")
    store.save_settings({"core_exe": "D:/Tools/skin-core.exe", "output_dir": "D:/Data/playbacks"})
    monkeypatch.setattr(module, "default_settings", lambda: pytest.fail("saved blank settings must not trigger discovery"))
    store.save_settings({"core_exe": "", "output_dir": ""})

    assert store.load_settings().model_dump() == {"core_exe": "", "output_dir": ""}

