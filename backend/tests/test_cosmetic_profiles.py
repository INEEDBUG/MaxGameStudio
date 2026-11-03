import re

import pytest

from app.cosmetic_studio.profiles import ProfileStore, ProfileValidationError, ProfileStoreError


def _profile(**overrides):
    value = {
        "name": "Offline AWP",
        "xuid": "76561199059074958",
        "preset_id": "awp-dragon-lore",
        "pattern_seed": 0,
        "wear": 0.01,
    }
    value.update(overrides)
    return value


def test_create_and_list_round_trip_a_named_profile(tmp_path):
    store = ProfileStore(tmp_path / "profiles.json")

    created = store.create(_profile(demo_sha256="a" * 64, start_tick=100, end_tick=300))
    loaded = ProfileStore(tmp_path / "profiles.json").list()

    assert len(loaded) == 1
    assert loaded[0] == created
    assert re.fullmatch(r"[0-9a-f]{32}", created["id"])
    assert created["name"] == "Offline AWP"
    assert created["xuid"] == "76561199059074958"
    assert created["demo_sha256"] == "a" * 64
    assert (created["start_tick"], created["end_tick"]) == (100, 300)


@pytest.mark.parametrize(
    "overrides",
    [
        {"xuid": 76561199059074958},
        {"xuid": "0"},
        {"preset_id": "not-in-catalog"},
        {"start_tick": 100},
        {"start_tick": 300, "end_tick": 200},
        {"demo_sha256": "not-a-sha256"},
        {"wear": float("nan")},
        {"pattern_seed": 1001},
        {"tool_path": "C:/should-not-be-stored.exe"},
    ],
)
def test_create_rejects_invalid_or_mismatched_binding_without_saving(tmp_path, overrides):
    path = tmp_path / "profiles.json"
    store = ProfileStore(path)

    with pytest.raises(ProfileValidationError):
        store.create(_profile(**overrides))

    assert store.list() == []
    assert not path.exists()


def test_corrupt_profile_file_is_preserved_and_never_overwritten(tmp_path):
    path = tmp_path / "profiles.json"
    original = b'{"schema_version":1,"profiles":[broken'
    path.write_bytes(original)
    store = ProfileStore(path)

    with pytest.raises(ProfileStoreError):
        store.list()
    with pytest.raises(ProfileStoreError):
        store.create(_profile())

    assert path.read_bytes() == original


def test_delete_removes_only_the_requested_profile(tmp_path):
    store = ProfileStore(tmp_path / "profiles.json")
    first = store.create(_profile(name="First"))
    second = store.create(_profile(name="Second"))

    assert store.delete(first["id"]) is True
    assert store.delete(first["id"]) is False
    assert store.list() == [second]


def test_profile_count_is_bounded_at_fifty(tmp_path):
    store = ProfileStore(tmp_path / "profiles.json")
    for index in range(50):
        store.create(_profile(name=f"Profile {index}"))
    before = (tmp_path / "profiles.json").read_bytes()

    with pytest.raises(ProfileStoreError, match="maximum"):
        store.create(_profile(name="Too many"))

    assert len(store.list()) == 50
    assert (tmp_path / "profiles.json").read_bytes() == before


def test_atomic_replace_failure_preserves_previous_file(tmp_path, monkeypatch):
    path = tmp_path / "profiles.json"
    store = ProfileStore(path)
    first = store.create(_profile())
    before = path.read_bytes()

    def fail_replace(_source, _destination):
        raise OSError("simulated replace failure")

    monkeypatch.setattr("app.cosmetic_studio.profiles.os.replace", fail_replace)
    with pytest.raises(ProfileStoreError, match="save"):
        store.create(_profile(name="Second"))

    assert path.read_bytes() == before
    assert ProfileStore(path).list() == [first]
    assert not list(tmp_path.glob("*.tmp"))


def test_oversized_file_is_rejected_without_replacement(tmp_path):
    path = tmp_path / "profiles.json"
    original = b"x" * (1024 * 1024 + 1)
    path.write_bytes(original)
    store = ProfileStore(path)

    with pytest.raises(ProfileStoreError, match="size"):
        store.list()
    assert path.read_bytes() == original
