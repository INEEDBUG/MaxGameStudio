"""Pinned-version diagnostic contract, not a rendered-appearance assertion."""
import pytest
from app.cosmetic_studio.runtime import refresh_fields_ready, refresh_armed
from app.cosmetic_studio.provider import CosmeticError

XUID = "76561199059074958"
SYNC = f"Standard-weapon in-place attribute synchronization: xuid={XUID}, definition=9, paint=344, changed_slots=3, verified=1, vector_replacement=0."
MATERIAL = "Standard-weapon native material initialization: entity=255, paint=344, before=1/0, after=0/2, direct=1, material_created=1, visual_result_not_verified=1."


def test_submission_is_not_paused_update_ready():
    text = "Runtime reloaded nSkinzCS2.json and queued Update"
    assert not refresh_armed(text)
    text += "\nFrameStageNotify hook became active\nManual refresh rebuilding runtime entities"
    assert not refresh_armed(text)
    assert refresh_armed(text + "\nProfiles: enabled=1")


def test_field_readback_is_not_enough_without_material_initialization():
    assert refresh_fields_ready(SYNC, XUID) is False
    assert refresh_fields_ready(SYNC + "\n" + MATERIAL, XUID) is True


def test_wrong_profile_and_failed_material_are_not_ready():
    assert refresh_fields_ready(SYNC.replace(XUID, "76561199184844942") + "\n" + MATERIAL, XUID) is False
    assert refresh_fields_ready(SYNC + "\n" + MATERIAL.replace("material_created=1", "material_created=0"), XUID) is False
    assert refresh_fields_ready("", XUID) is False


@pytest.mark.parametrize("failure", [
    "Weapon appearance apply failed before verification: carrier_entity=255",
    "Rolled back weapon appearance: carrier_entity=255, rolled_back=1, failed_stage=Verify",
    "Weapon appearance stage failed: Verify",
    SYNC.replace("changed_slots=3, verified=1", "changed_slots=0, verified=0"),
])
def test_fresh_failure_blocks_even_if_positive_lines_also_exist(failure):
    with pytest.raises(CosmeticError, match="扩展"):
        refresh_fields_ready(SYNC + "\n" + MATERIAL + "\n" + failure, XUID)
