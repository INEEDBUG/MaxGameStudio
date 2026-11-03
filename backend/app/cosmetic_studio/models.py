"""Opt-in local Demo production contract; no game state is changed on import."""
from pathlib import Path
import re
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator, model_validator
from .catalog import PRESET_DATA, WEAR_LIMITS, GLOVE_SUPPORTED_WEAPONS, source_weapon_id


class StrictModel(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True)

    @field_validator("*", mode="before")
    @classmethod
    def explicit_acknowledgement(cls, value, info):
        if info.field_name in ("acknowledge_offline", "appearance_confirmed") and value is not True:
            raise ValueError("An explicit true acknowledgement is required")
        return value


class ToolPaths(StrictModel):
    cs2_exe: str
    hlae_exe: str
    nskinz_dll: str | None = None
    core_exe: str | None = None
    obs_exe: str
    ffmpeg_exe: str
    ffprobe_exe: str
    output_dir: str

    @field_validator("*")
    @classmethod
    def local_path(cls, value: str) -> str:
        if value is None:
            return value
        if not value or len(value) > 1000 or any(c in value for c in '\r\n\x00";'):
            raise ValueError("A plain absolute local path is required")
        if not re.match(r"^[A-Za-z]:[\\/]", value):
            raise ValueError("Network and relative paths are not allowed")
        return value


class InspectRequest(StrictModel):
    demo_path: str

    @field_validator("demo_path")
    @classmethod
    def demo_local(cls, value: str) -> str:
        value = ToolPaths.local_path(value)
        if Path(value).suffix.lower() != ".dem":
            raise ValueError("Select a .dem file")
        return value


class JobRequest(InspectRequest):
    xuid: str
    start_tick: int = Field(strict=True, ge=256, le=100_000_000)
    end_tick: int = Field(strict=True, ge=257, le=100_000_000)
    provider_id: Literal["nskinz", "skin_core"] = "nskinz"
    preset_id: str
    pattern_seed: int = Field(strict=True, ge=0, le=1000, default=0)
    wear: float = Field(ge=0, le=1, allow_inf_nan=False, default=.01)
    tools: ToolPaths
    acknowledge_offline: Literal[True]
    expected_demo_sha256: str | None = Field(default=None, pattern=r'^[a-f0-9]{64}$')
    glove_preset_id: Literal['specialist-10061'] | None = None
    sky_id: Literal['original','italy','dust2','night','overcast','sunset'] = 'original'

    @field_validator('preset_id')
    @classmethod
    def supported_preset(cls, value):
        if value not in PRESET_DATA:
            raise ValueError('Unsupported qualified skin preset')
        return value

    @field_validator("xuid")
    @classmethod
    def exact_xuid(cls, value: str) -> str:
        if not isinstance(value, str) or not re.fullmatch(r"[1-9][0-9]{0,19}", value) or int(value) > 2**64 - 1:
            raise ValueError("XUID must be an exact unsigned 64-bit decimal string")
        return value

    @model_validator(mode="after")
    def interval(self):
        if self.sky_id!='original' and self.provider_id!='skin_core':
            raise ValueError('天空仅支持专用文件制片模式，不加载额外换肤扩展。')
        # Conservative initial scope; no lengthy unattended sessions.
        if not 128 <= self.end_tick - self.start_tick <= 1920:
            raise ValueError("Initial experiment supports 2–30 seconds at 64 ticks/s")
        definition, paint, _ = PRESET_DATA[self.preset_id]
        low, high = WEAR_LIMITS.get((definition, paint), (0.0, 1.0))
        if not low <= self.wear <= high:
            raise ValueError('Wear is outside the qualified finish range')
        if self.glove_preset_id and definition not in GLOVE_SUPPORTED_WEAPONS:
            raise ValueError('This weapon/glove combination is not qualified')
        if self.provider_id == "nskinz":
            if self.glove_preset_id is not None:
                raise ValueError('Glove attachment requires the qualified file provider')
            if not self.tools.nskinz_dll or self.preset_id != "awp-dragon-lore" or self.pattern_seed != 0 or self.wear != .01:
                raise ValueError("Runtime provider currently requires its qualified fixed AWP preset and DLL")
        elif not self.tools.core_exe:
            raise ValueError("Select the qualified Skin Core executable")
        return self


class ConfirmRequest(StrictModel):
    preview_id: str = Field(min_length=32, max_length=64, pattern=r"^[a-f0-9]+$")
    appearance_confirmed: Literal[True]


class DirectorImportRequest(InspectRequest):
    expected_demo_sha256: str = Field(pattern=r'^[a-f0-9]{64}$')
    plan: dict


class IntervalRequest(InspectRequest):
    """Read-only manual knife fragment validation, not a game command."""
    expected_demo_sha256: str = Field(pattern=r'^[a-f0-9]{64}$')
    xuid: str
    start_tick: int = Field(ge=256, le=100_000_000)
    end_tick: int = Field(ge=257, le=100_000_000)
    weapon_id: Literal[507]

    @field_validator('xuid')
    @classmethod
    def exact_xuid(cls, value):
        return JobRequest.exact_xuid(value)

    @model_validator(mode='after')
    def interval(self):
        if not 128 <= self.end_tick - self.start_tick <= 1920:
            raise ValueError('Select a 2–30 second fragment at 64 ticks/s')
        return self


class JobStatus(StrictModel):
    id: str
    state: Literal["preparing", "preview", "recording", "exporting", "complete", "failed", "cancelled"]
    message: str
    preview_id: str | None = None
    preview_url: str | None = None
    output_path: str | None = None
    litecut_project_id: int | None = None


class Player(StrictModel):
    xuid: str
    name: str


class Candidate(StrictModel):
    xuid: str
    start_tick: int
    end_tick: int
    label: str
    weapon_id: int = 9


class Inspection(StrictModel):
    players: list[Player]
    candidates: list[Candidate]
    demo_sha256: str
    candidates_limited: bool = False


class Preset(StrictModel):
    id: str = "awp-dragon-lore"
    label: str = "AWP · 巨龙传说（离线实验）"
    weapon_id: int = 9
    source_weapon_id: int = 9
    paint_kit: int = 344
    provider_ids: list[str] = Field(default_factory=lambda: ["nskinz", "skin_core"])


PRESETS = {key: value[1] for key, value in PRESET_DATA.items()}


class ProviderOption(StrictModel):
    id: str
    label: str
    notice: str


class GlovePreset(StrictModel):
    id: str
    label: str
    weapon_ids: list[int]


class Capabilities(StrictModel):
    enabled: bool
    reason: str
    defaults: dict[str, str]
    # Populate only after combined native/OBS/LiteCut qualification; a core
    # reply or a glove-only screenshot alone is not a public capability.
    glove_presets: list[GlovePreset] = Field(default_factory=list)
    presets: list[Preset] = Field(default_factory=lambda: [Preset(id=key, weapon_id=value[0],
        source_weapon_id=source_weapon_id(key), paint_kit=value[1],
        label=value[2] + '（需核对预览）', provider_ids=['nskinz', 'skin_core'] if key == 'awp-dragon-lore' else ['skin_core'])
        for key, value in PRESET_DATA.items()])
    providers: list[ProviderOption] = Field(default_factory=lambda: [
        ProviderOption(id="skin_core", label="Demo 制片副本", notice="通过已授权核心的兼容入口生成副本，不改原始 Demo。核心会写入自己的武器名称；录制沿用纯净 HUD，仅保留击杀信息。")])
