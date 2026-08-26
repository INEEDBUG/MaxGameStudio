"""Pure backend/domain support for VALORANT crosshair profiles.

This package only parses, normalizes, serializes and locally stores native
profile codes.  It deliberately has no game-process, memory, injection or
game-configuration integration.
"""

from .codec import (
    CrosshairCodeSyntaxError,
    CrosshairFieldError,
    DuplicateCrosshairFieldError,
    ValorantCrosshairError,
    deserialize_crosshair_code,
    normalize_crosshair_code,
    normalize_field,
    parse_crosshair_code,
    parse_share_code,
    parse_valorant_crosshair_code,
    serialize_crosshair_code,
    serialize_share_code,
    serialize_valorant_crosshair_code,
)
from .models import (
    CrosshairField,
    CrosshairPreset,
    CrosshairPresetCreate,
    CrosshairPresetPatch,
    CrosshairSection,
    ValorantCrosshairAST,
    ValorantCrosshairCode,
    ValorantCrosshairField,
    ValorantCrosshairPreset,
    ValorantCrosshairSection,
)
__all__ = [
    "ValorantCrosshairError",
    "CrosshairCodeSyntaxError",
    "CrosshairFieldError",
    "DuplicateCrosshairFieldError",
    "CrosshairField",
    "CrosshairSection",
    "ValorantCrosshairAST",
    "ValorantCrosshairCode",
    "ValorantCrosshairField",
    "ValorantCrosshairSection",
    "CrosshairPreset",
    "CrosshairPresetCreate",
    "CrosshairPresetPatch",
    "ValorantCrosshairPreset",
    "normalize_field",
    "parse_crosshair_code",
    "parse_valorant_crosshair_code",
    "parse_share_code",
    "deserialize_crosshair_code",
    "serialize_crosshair_code",
    "serialize_valorant_crosshair_code",
    "serialize_share_code",
    "normalize_crosshair_code",
    "ValorantCrosshairDB",
    "ValorantPresetStore",
    "CrosshairPresetDB",
    "ValorantCrosshairStore",
    "CrosshairStore",
]


def __getattr__(name: str):
    """Load SQLite support only when a caller requests the storage classes.

    Parsing and serialization are useful in lightweight tooling that does not
    need the optional database runtime.  Keeping this import lazy also mirrors
    the domain/application boundary: the codec itself has no persistence side
    effects.
    """

    if name in {
        "ValorantCrosshairDB",
        "ValorantPresetStore",
        "CrosshairPresetDB",
        "ValorantCrosshairStore",
        "CrosshairStore",
    }:
        from .storage import (
            CrosshairPresetDB,
            CrosshairStore,
            ValorantCrosshairDB,
            ValorantCrosshairStore,
            ValorantPresetStore,
        )

        return {
            "ValorantCrosshairDB": ValorantCrosshairDB,
            "ValorantPresetStore": ValorantPresetStore,
            "CrosshairPresetDB": CrosshairPresetDB,
            "ValorantCrosshairStore": ValorantCrosshairStore,
            "CrosshairStore": CrosshairStore,
        }[name]
    raise AttributeError(name)
