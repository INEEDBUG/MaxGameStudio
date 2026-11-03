"""Opt-in file-rewrite provider, separate from the runtime-DLL provider.

The strict transport remains available. The local experimental service uses
the explicitly authorized, fixed-hash compatibility transport below; it is
not a claim of vendor-certified production SDK support.
"""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import threading
from typing import Literal

from pydantic import ConfigDict, Field, field_validator, model_validator

from .core_wire import MAX_FRAME, decode_response, encode_request, stdin_packet
from .models import JobRequest, StrictModel
from .catalog import PRESET_DATA, WEAR_LIMITS, source_weapon_id
from .owned_process import core_environment, run_core
from .provider import CosmeticError, ProviderStatus
from .runtime import digest, write_json

QUALIFIED_CORE_SHA256 = "4bbccff3ae262cb174dec4599b0d42ae9b12ba608dfd97d1d6f8359449710be5"


def run_licensed_core(executable, args, packet, root, env, cancel):
    """Explicit licensed third-party invocation of the qualified file tool.

    The reference client's documented DEV switch skips its parent-PE allowlist.
    This is a compatibility mode, not a newly supported vendor SDK. The owner
    has authorized third-party invocation; keep the switch local to this one
    pinned child. Never inherit it into CS2/HLAE/OBS or the host environment.
    The unmodified default runner remains available for strict caller testing.
    """
    if digest(Path(executable)) != QUALIFIED_CORE_SHA256:
        raise CosmeticError("CORE_VERSION_UNQUALIFIED")
    child_env = {**env, "CS2_SKIN_CORE_DEV": "1"}
    return run_core(executable, args, packet, root, child_env, cancel)


class CoreItem(StrictModel):
    model_config = ConfigDict(extra="forbid", strict=True, frozen=True)
    item_id64: str
    definition_index: int = 9
    replacement_definition_index: int | None = None
    paint_kit: int
    pattern_seed: int = Field(ge=0, le=1000, default=0)
    wear: float = Field(ge=0, le=1, allow_inf_nan=False, default=.01)

    @field_validator("item_id64")
    @classmethod
    def item_id(cls, value):
        return JobRequest.exact_xuid(value)

    @model_validator(mode='after')
    def compatible_finish(self):
        target_definition = self.replacement_definition_index or self.definition_index
        same_model_pairs = {
            (value[0], value[1]) for preset_id, value in PRESET_DATA.items()
            if source_weapon_id(preset_id) == value[0]
        }
        cross_model_pairs = {
            (source_weapon_id(preset_id), value[0], value[1])
            for preset_id, value in PRESET_DATA.items()
            if source_weapon_id(preset_id) != value[0]
        }
        if self.replacement_definition_index is None:
            if (self.definition_index, self.paint_kit) not in same_model_pairs:
                raise ValueError('Unsupported weapon/paint combination')
        elif (self.definition_index, target_definition, self.paint_kit) not in cross_model_pairs:
            raise ValueError('Unsupported source/replacement/paint combination')
        if (target_definition, self.paint_kit) not in {(v[0], v[1]) for v in PRESET_DATA.values()}:
            raise ValueError('Unsupported replacement weapon/paint combination')
        minimum, maximum = WEAR_LIMITS.get((target_definition, self.paint_kit), (0.0, 1.0))
        if not minimum <= self.wear <= maximum:
            raise ValueError('Wear is outside the qualified finish range')
        return self


class CoreMaterializedUspItem(StrictModel):
    """Separate zero-ID CT USP rule, qualified by an immutable file-only probe.

    Never reuse the positive-owned-ID contract for missing inventory. The core
    derives the playback-only ID from target Pawn provenance; we verify it.
    """
    model_config = ConfigDict(extra='forbid', strict=True, frozen=True)
    item_id64: Literal['0'] = '0'
    definition_index: Literal[61] = 61
    paint_kit: Literal[504] = 504
    pattern_seed: int = Field(ge=0, le=1000, default=0)
    wear: float = Field(ge=0, le=1, allow_inf_nan=False, default=.01)
    team: Literal['CT'] = 'CT'
    replacement_definition_index: None = None

    @field_validator('definition_index', 'paint_kit', mode='before')
    @classmethod
    def exact_integer(cls, value):
        if type(value) is not int:
            raise ValueError('Materialized identifiers must be exact integers')
        return value


class CoreGloveItem(StrictModel):
    """One independently qualified Pawn-glove transformation, not a gun swap."""
    model_config = ConfigDict(extra='forbid', strict=True, frozen=True)
    item_id64: str
    definition_index: Literal[5030] = 5030
    replacement_definition_index: Literal[5034] = 5034
    paint_kit: Literal[10061] = 10061
    pattern_seed: Literal[0] = 0
    wear: Literal[0.01] = 0.01

    @field_validator('definition_index', 'replacement_definition_index', 'paint_kit', 'pattern_seed', mode='before')
    @classmethod
    def exact_integer(cls, value):
        if type(value) is not int:
            raise ValueError('Glove identifiers must be explicit integers')
        return value

    @field_validator('item_id64')
    @classmethod
    def item_id(cls, value):
        return JobRequest.exact_xuid(value)


class CoreProfile(StrictModel):
    model_config = ConfigDict(extra="forbid", strict=True, frozen=True)
    steam_id64: str
    items: tuple[CoreItem | CoreGloveItem | CoreMaterializedUspItem, ...] = Field(min_length=1, max_length=16)

    @field_validator("steam_id64")
    @classmethod
    def xuid(cls, value):
        return JobRequest.exact_xuid(value)

    @model_validator(mode="after")
    def unique(self):
        if any(isinstance(item, CoreMaterializedUspItem) for item in self.items) and len(self.items) != 1:
            raise ValueError('Materialized USP rules require a separate single-item rewrite step')
        if len({item.item_id64 for item in self.items}) != len(self.items):
            raise ValueError("Duplicate owned item")
        gloves = [item for item in self.items if isinstance(item, CoreGloveItem)]
        if len(gloves) > 1:
            raise ValueError('Only one qualified glove override is supported')
        if gloves and any(isinstance(item, CoreItem) and item.replacement_definition_index is not None
                          for item in self.items):
            raise ValueError('The cross-model knife/glove combination is not qualified')
        return self

    def payload(self):
        return {"schema_version": 1, **self.model_dump(mode="json", exclude_none=True)}

    def sha256(self):
        return hashlib.sha256(json.dumps(self.payload(), ensure_ascii=False, sort_keys=True,
                                         separators=(",", ":"), allow_nan=False).encode()).hexdigest()


@dataclass(frozen=True)
class FileArtifact:
    original: Path
    playback: Path
    original_sha256: str
    playback_sha256: str
    profile_sha256: str
    items_rewritten: int
    # A core-authenticated output is never an automatic visual ACK.
    visual_verified: bool | None = None
    name_policy: str = "core_defined"


def validate_reply(reply, profile, output):
    if type(reply.get("schema_version")) is not int or reply["schema_version"] != 1 or reply.get("ok") is not True:
        raise CosmeticError("CORE_REWRITE_REJECTED")
    if type(reply.get("items_rewritten")) is not int or reply["items_rewritten"] != len(profile.items) or reply.get("failed"):
        raise CosmeticError("CORE_PARTIAL_REWRITE")
    succeeded = reply.get("succeeded")
    expected = {(item.item_id64, item.definition_index) for item in profile.items}
    if not isinstance(succeeded, list) or len(succeeded) != len(expected):
        raise CosmeticError("CORE_TARGET_RESULT_MISSING")
    actual = []
    for item in succeeded:
        if (not isinstance(item, dict) or type(item.get("item_id64")) is not str
                or type(item.get("definition_index")) is not int):
            raise CosmeticError("CORE_TARGET_RESULT_INVALID")
        actual.append((item["item_id64"], item["definition_index"]))
    if set(actual) != expected:
        raise CosmeticError("CORE_TARGET_RESULT_MISMATCH")
    if (any(isinstance(item, CoreMaterializedUspItem) for item in profile.items)
            and any(item.get('team') != 'CT' for item in succeeded)):
        raise CosmeticError('CORE_TARGET_TEAM_MISMATCH')
    expected_hash = reply.get("sha256")
    if not isinstance(expected_hash, str) or not re.fullmatch(r"[a-f0-9]{64}", expected_hash):
        raise CosmeticError("CORE_OUTPUT_DIGEST_MISSING")
    if not output.is_file() or output.is_symlink() or output.stat().st_size < 16:
        raise CosmeticError("CORE_OUTPUT_MISSING")
    with output.open("rb") as stream:
        if stream.read(8) != b"PBDEMS2\x00":
            raise CosmeticError("CORE_OUTPUT_NOT_DEMO")
    if digest(output) != expected_hash:
        raise CosmeticError("CORE_OUTPUT_DIGEST_MISMATCH")
    return expected_hash


class SkinCoreProvider:
    """One profile and one immutable source per invocation.

    `verifier` must independently parse the actual copied Demo and establish
    expected cosmetics and selected gameplay invariants. Returning False or
    raising rejects the artifact. An authenticated response alone is not enough.
    The host controls native playback and supplies a fresh `preview_check`.
    """
    def __init__(self, original: Path, root: Path, profile: CoreProfile, executable: Path,
                 python: Path, cancel: threading.Event, *, verifier, transport=run_core,
                 preview_check=None, stop_playback=None):
        self.original, self.root, self.profile = Path(original), Path(root), profile
        self.executable, self.python = Path(executable), Path(python)
        self.cancel, self.verifier, self.transport = cancel, verifier, transport
        self.preview_check, self.stop_playback = preview_check, stop_playback
        self.state, self.artifact = "new", None
        self._confirmed = False

    def _check_cancel(self):
        if self.cancel.is_set():
            raise CosmeticError("CORE_CANCELLED")

    def load(self):
        if self.state != "new":
            raise CosmeticError("PROVIDER_ALREADY_USED")
        self._check_cancel()
        self.original = self.original.resolve(strict=True)
        self.executable, self.python = self.executable.resolve(strict=True), self.python.resolve(strict=True)
        if self.original.suffix.lower() != ".dem" or not self.original.is_file():
            raise CosmeticError("CORE_SOURCE_NOT_DEMO")
        if digest(self.executable) != QUALIFIED_CORE_SHA256:
            raise CosmeticError("CORE_VERSION_UNQUALIFIED")
        if not self.python.is_file() or self.python.name.lower() != "python.exe":
            raise CosmeticError("CORE_PARSER_RUNTIME_INVALID")
        parent = self.root.parent.resolve(strict=True)
        if (self.root.name in ("", ".", "..") or str(parent).startswith(("\\\\", "//"))
                or (os.name == "nt" and parent.drive.lower() == os.environ.get("SystemDrive", "C:").lower())):
            raise CosmeticError("CORE_OUTPUT_LOCATION_INVALID")
        self.root = parent / self.root.name
        if shutil.disk_usage(parent).free < self.original.stat().st_size * 3 + 64 * 1024**2:
            raise CosmeticError("CORE_INSUFFICIENT_SPACE")
        self.source_hash = digest(self.original)
        self.root.mkdir(exist_ok=False)
        for name in ("temp", "profile"):
            (self.root / name).mkdir()
        self.input, self.output = self.root / "input.dem", self.root / "playback.dem"
        with self.original.open("rb") as source, self.input.open("xb") as destination:
            shutil.copyfileobj(source, destination)
        if digest(self.input) != self.source_hash or digest(self.original) != self.source_hash:
            self.state = "failed"
            raise CosmeticError("CORE_SOURCE_CHANGED_DURING_COPY")
        write_json(self.root / "profile.json", self.profile.payload())
        self.state = "loaded"

    def apply_profile(self, profile_digest: str):
        if self.state != "loaded" or profile_digest != self.profile.sha256():
            raise CosmeticError("CORE_PROFILE_NOT_BOUND")
        self._check_cancel()
        key = packet = None
        try:
            self.state = "rewriting"
            if digest(self.executable) != QUALIFIED_CORE_SHA256 or digest(self.original) != self.source_hash or digest(self.input) != self.source_hash:
                raise CosmeticError("CORE_INPUT_OR_TOOL_CHANGED")
            request, response = self.root / "request.bin", self.root / "response.bin"
            key, envelope = encode_request(self.profile.payload(), str(self.input), str(self.output))
            with request.open("xb") as stream:
                stream.write(envelope)
            packet = stdin_packet(key)
            argv = ["rewrite-owned-batch", "--input", str(self.input), "--output", str(self.output),
                    "--request", str(request), "--response", str(response), "--demoparser2-python", str(self.python)]
            code, category = self.transport(self.executable, argv, packet, self.root,
                core_environment(self.root, self.python, self.executable), self.cancel)
            self._check_cancel()
            if code != 0:
                raise CosmeticError(category)
            if not response.is_file() or response.is_symlink() or response.stat().st_size > MAX_FRAME:
                raise CosmeticError("CORE_RESPONSE_MISSING")
            reply = decode_response(response.read_bytes(), key, str(self.input), str(self.output))
            output_hash = validate_reply(reply, self.profile, self.output)
            # Validate source/working input before the parser receives the output.
            if digest(self.original) != self.source_hash or digest(self.input) != self.source_hash:
                raise CosmeticError("CORE_SOURCE_OR_INPUT_CHANGED")
            if self.verifier(self.input, self.output, self.profile) is not True:
                raise CosmeticError("CORE_INDEPENDENT_VERIFICATION_FAILED")
            self._check_cancel()
            if digest(self.output) != output_hash:
                raise CosmeticError("CORE_OUTPUT_CHANGED_DURING_VERIFICATION")
            self.artifact = FileArtifact(self.original, self.output, self.source_hash, output_hash,
                                         self.profile.sha256(), len(self.profile.items))
            write_json(self.root / "artifact.json", {"source_sha256": self.source_hash, "playback_sha256": output_hash,
                "profile_sha256": self.profile.sha256(), "items_rewritten": len(self.profile.items),
                "visual_verified": None, "name_policy": "core_defined", "analysis_uses_original": True,
                "invocation_mode": "licensed_compatibility" if self.transport is run_licensed_core else "strict_or_test_transport"})
            self.state = "pending_visual_review"
        except Exception:
            self.state, self.artifact = "failed", None
            raise
        finally:
            key = packet = None  # No disk, environment, log, or exception serialization of key material.
            try:
                unchanged = digest(self.original) == self.source_hash and digest(self.input) == self.source_hash
            except OSError:
                unchanged = False
            if not unchanged:
                self.state, self.artifact = "failed", None
                raise CosmeticError("CORE_SOURCE_OR_INPUT_CHANGED")

    def confirm_preview(self):
        if self.state != "pending_visual_review" or not self.artifact or not self.preview_check:
            raise CosmeticError("CORE_VISUAL_REVIEW_NOT_BOUND")
        self._check_cancel()
        if self.preview_check(self.artifact) is not True or digest(self.artifact.playback) != self.artifact.playback_sha256:
            raise CosmeticError("CORE_VISUAL_REVIEW_STALE")
        self._confirmed, self.state = True, "human_confirmed"

    def check_ready(self):
        if self.cancel.is_set() or self.state != "human_confirmed" or not self._confirmed or not self.artifact or not self.preview_check:
            return False
        try:
            return self.preview_check(self.artifact) is True and digest(self.artifact.playback) == self.artifact.playback_sha256
        except (CosmeticError, OSError):
            return False

    def get_status(self):
        ready = self.check_ready()
        return ProviderStatus(self.state, True if ready else None, "human_preview" if ready else None)

    def unload(self):
        self._confirmed, self.state = False, "stopping"
        # Core has already exited; only a host-owned playback lifecycle can
        # prove native cleanup. This is not a DLL hot-unload implementation.
        if self.stop_playback:
            if self.stop_playback() is not True:
                self.state = "cleanup_unknown"
                raise CosmeticError("CORE_PLAYBACK_STOP_UNCONFIRMED")
        self.state = "stopped"
