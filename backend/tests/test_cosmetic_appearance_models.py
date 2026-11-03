from pydantic import ValidationError
import pytest

from app.cosmetic_studio.appearance_models import (
    AppearanceApplyRequest,
    AppearanceInventoryRequest,
    AppearanceReplacement,
)


XUID = "76561199369875913"


def test_replacement_accepts_only_a_qualified_finish_with_legal_wear_and_seed():
    selection = AppearanceReplacement(
        slot_id="slot-0000",
        preset_id="karambit-to-butterfly-fade",
        wear=0.08,
        seed=1000,
    )

    assert selection.model_dump() == {
        "slot_id": "slot-0000",
        "preset_id": "karambit-to-butterfly-fade",
        "wear": 0.08,
        "seed": 1000,
    }


@pytest.mark.parametrize("payload", [
    {"slot_id": "slot-0000", "preset_id": "unknown", "wear": 0.01, "seed": 0},
    {"slot_id": "slot-0000", "preset_id": "karambit-fade", "wear": 0.0801, "seed": 0},
    {"slot_id": "slot-0000", "preset_id": "awp-asiimov", "wear": 0.17, "seed": 0},
    {"slot_id": "slot-0000", "preset_id": "awp-dragon-lore", "wear": 0.01, "seed": 1001},
    {"slot_id": "slot-0000", "preset_id": "awp-dragon-lore", "wear": float("nan"), "seed": 0},
    {"slot_id": "slot-0000", "preset_id": "awp-dragon-lore", "wear": 0.01, "seed": 0, "item_id": "99"},
])
def test_replacement_rejects_unknown_finish_illegal_parameters_and_client_item_identity(payload):
    with pytest.raises(ValidationError):
        AppearanceReplacement.model_validate(payload)


def test_apply_request_binds_to_demo_hash_and_exact_decimal_xuid():
    request = AppearanceApplyRequest.model_validate({
        "demo_path": "D:/Demos/match.dem",
        "expected_demo_sha256": "a" * 64,
        "xuid": XUID,
        "replacements": [],
    })

    assert request.xuid == XUID
    assert request.expected_demo_sha256 == "a" * 64


@pytest.mark.parametrize("changes", [
    {"expected_demo_sha256": "not-a-hash"},
    {"xuid": 76561199369875913},
    {"xuid": "7.656e16"},
    {"replacements": [{"slot_id": "same-slot", "preset_id": "awp-dragon-lore", "wear": 0.01, "seed": 0},
                       {"slot_id": "same-slot", "preset_id": "awp-asiimov", "wear": 0.01, "seed": 0}]},
    {"replacements": [{"slot_id": "slot-0000", "preset_id": "awp-dragon-lore", "wear": 0.01, "seed": 0, "item_id": "99"}]},
])
def test_apply_request_rejects_invalid_fingerprint_identity_duplicate_or_trusted_item_id(changes):
    payload = {"demo_path": "D:/Demos/match.dem", "expected_demo_sha256": "a" * 64,
               "xuid": XUID, "replacements": []}
    payload.update(changes)

    with pytest.raises(ValidationError):
        AppearanceApplyRequest.model_validate(payload)


def test_inventory_request_requires_absolute_demo_path():
    with pytest.raises(ValidationError):
        AppearanceInventoryRequest(demo_path="match.dem")


def test_settings_normalize_extended_local_drive_paths_from_desktop_runtime():
    from app.cosmetic_studio.appearance_models import AppearanceSettings

    settings = AppearanceSettings(
        core_exe=r"\\?\D:\Tools\skin-core.exe",
        output_dir=r"\\?\D:\Data\cosmetic_playbacks",
    )
    assert settings.core_exe == r"D:\Tools\skin-core.exe"
    assert settings.output_dir == r"D:\Data\cosmetic_playbacks"


@pytest.mark.parametrize("path", [
    r"\\?\UNC\server\share\core.exe", r"\\server\share\core.exe",
    r"\\.\D:\Tools\core.exe", r"\\?\GLOBALROOT\Device\core.exe",
    r"\\?\D:relative.exe",
])
def test_settings_still_reject_network_device_and_relative_extended_paths(path):
    from app.cosmetic_studio.appearance_models import AppearanceSettings

    with pytest.raises(ValidationError):
        AppearanceSettings(core_exe=path)
