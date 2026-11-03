"""Strict request and response contracts for Demo-bound appearance editing."""

from __future__ import annotations

import math
import json
import re
from pathlib import Path
from functools import lru_cache
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator, model_validator

from .catalog import GLOVE_PRESET_DATA, PRESET_DATA, WEAR_LIMITS, GUN_FINISH_METADATA, source_weapon_id


class AppearanceModel(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True)


@lru_cache(maxsize=1)
def _qualified_wear_metadata() -> dict:
    path = Path(__file__).with_name("appearance_catalog_metadata.json")
    data = json.loads(path.read_text(encoding="utf-8"))
    if (data.get("commit") != "e2644abaf1bb26b871d0e90e7b44b7116feef486"
            or data.get("license") != "MIT; image artwork remains Valve property"):
        raise ValueError("Qualified finish metadata provenance is invalid")
    return {**data["items"], **GUN_FINISH_METADATA}


def _local_demo_path(value: str) -> str:
    if not isinstance(value, str) or len(value) > 2048 or any(char in value for char in '\r\n\x00";'):
        raise ValueError("A local absolute Demo path is required")
    if not re.match(r"^[A-Za-z]:[\\/]", value) or Path(value).suffix.lower() != ".dem":
        raise ValueError("Select a local .dem file using an absolute path")
    return value


def _exact_xuid(value: str) -> str:
    if not isinstance(value, str) or not re.fullmatch(r"[1-9][0-9]{0,19}", value):
        raise ValueError("XUID must be an exact decimal string")
    number = int(value, 10)
    if number > (1 << 64) - 1 or str(number) != value:
        raise ValueError("XUID is outside the supported decimal range")
    return value


class AppearanceInventoryRequest(AppearanceModel):
    demo_path: str
    xuid: str | None = None

    @field_validator("xuid")
    @classmethod
    def exact_player(cls, value):
        return _exact_xuid(value) if value is not None else None

    @field_validator("demo_path")
    @classmethod
    def demo_path_is_local(cls, value: str) -> str:
        return _local_demo_path(value)


class AppearanceSettings(AppearanceModel):
    core_exe: str = ""
    output_dir: str = ""

    @field_validator("core_exe", "output_dir")
    @classmethod
    def local_path_or_empty(cls, value: str) -> str:
        if value == "":
            return value
        if len(value) > 2048 or any(char in value for char in '\r\n\x00";'):
            raise ValueError("A plain local path is required")
        # Tauri canonicalizes local paths with the Win32 extended prefix.
        # Normalize only drive-absolute paths, never UNC/device namespaces.
        if value.startswith("\\\\?\\") and re.match(r"^[A-Za-z]:\\", value[4:]):
            value = value[4:]
        if not re.match(r"^[A-Za-z]:[\\/]", value):
            raise ValueError("Network and relative paths are not allowed")
        return value


class AppearanceReplacement(AppearanceModel):
    slot_id: str = Field(min_length=8, max_length=64, pattern=r"^[a-zA-Z0-9:_-]+$")
    preset_id: str
    wear: float = Field(ge=0, le=1, allow_inf_nan=False)
    seed: int = Field(ge=0, le=1000)

    @field_validator("preset_id")
    @classmethod
    def only_qualified_preset(cls, value: str) -> str:
        if value not in PRESET_DATA and value not in GLOVE_PRESET_DATA:
            raise ValueError("Unsupported appearance preset")
        return value

    @model_validator(mode="after")
    def legal_finish_parameters(self):
        if self.preset_id in GLOVE_PRESET_DATA:
            if self.wear != 0.01 or self.seed != 0:
                raise ValueError("The qualified glove finish uses fixed wear and seed")
            return self
        definition, paint, _ = PRESET_DATA[self.preset_id]
        metadata = _qualified_wear_metadata().get(f"{definition}:{paint}")
        minimum, maximum = WEAR_LIMITS.get((definition, paint), (0.0, 1.0))
        if isinstance(metadata, dict):
            minimum, maximum = float(metadata["wear_min"]), float(metadata["wear_max"])
        if not minimum <= self.wear <= maximum:
            raise ValueError("Wear is outside the selected finish range")
        if not math.isfinite(self.wear):
            raise ValueError("Wear must be finite")
        return self


class AppearanceApplyRequest(AppearanceModel):
    demo_path: str
    expected_demo_sha256: str = Field(pattern=r"^[a-f0-9]{64}$")
    xuid: str
    replacements: list[AppearanceReplacement] = Field(max_length=128)
    scope: Literal['item','match'] = 'item'

    @field_validator("demo_path")
    @classmethod
    def demo_path_is_local(cls, value: str) -> str:
        return _local_demo_path(value)

    @field_validator("xuid")
    @classmethod
    def exact_player_id(cls, value: str) -> str:
        return _exact_xuid(value)

    @model_validator(mode="after")
    def unique_slots(self):
        slot_ids = [item.slot_id for item in self.replacements]
        if len(slot_ids) != len(set(slot_ids)):
            raise ValueError("An appearance slot may be selected only once")
        return self


class AppearancePlayer(AppearanceModel):
    xuid: str
    name: str


class AppearancePreset(AppearanceModel):
    id: str
    label: str
    source_definition: int
    target_definition: int
    paint_index: int
    wear_min: float
    wear_max: float
    image_url: str | None = None


class AppearanceItem(AppearanceModel):
    slot_id: str
    xuid: str
    team: Literal["ct", "t", "both", "unknown"]
    definition: int
    item_id: str
    name: str
    paint_index: int | None
    wear: float | None
    seed: int | None
    editable: bool
    reason: str | None = None
    preset_ids: list[str]
    image_url: str | None = None


class AppearanceInventoryResponse(AppearanceModel):
    demo_sha256: str = Field(pattern=r"^[a-f0-9]{64}$")
    players: list[AppearancePlayer]
    items: list[AppearanceItem]
    presets: list[AppearancePreset]
    settings: AppearanceSettings
    plans: dict[str, list[AppearanceReplacement]]
    artifact: AppearanceArtifact | None = None


class AppearanceSettingsRequest(AppearanceModel):
    core_exe: str
    output_dir: str

    @field_validator("core_exe", "output_dir")
    @classmethod
    def local_path(cls, value: str) -> str:
        return AppearanceSettings.local_path_or_empty(value)


class AppearanceItemResult(AppearanceModel):
    slot_id: str
    preset_id: str
    status: Literal["applied", "failed"]
    error: str | None = None


class AppearanceArtifact(AppearanceModel):
    job_id: str = Field(pattern=r"^[a-f0-9]{32}$")
    original_path: str
    playback_path: str
    original_sha256: str = Field(pattern=r"^[a-f0-9]{64}$")
    playback_sha256: str = Field(pattern=r"^[a-f0-9]{64}$")
    profile_sha256: str = Field(pattern=r"^[a-f0-9]{64}$")
    state: Literal["file_verified"] = "file_verified"
    visual_verified: None = None


class AppearanceJobStatus(AppearanceModel):
    id: str
    state: Literal["pending", "running", "file_verified", "failed", "cancelled"]
    items: list[AppearanceItemResult] = Field(default_factory=list)
    error: str | None = None
    artifact: AppearanceArtifact | None = None
    resolved_plan: list[AppearanceReplacement] | None = None
    skipped_foreign_items: int = 0
