"""Compatibility re-exports for code that imports a dedicated parser module."""

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
)

__all__ = [
    "ValorantCrosshairError",
    "CrosshairCodeSyntaxError",
    "CrosshairFieldError",
    "DuplicateCrosshairFieldError",
    "normalize_field",
    "parse_crosshair_code",
    "parse_valorant_crosshair_code",
    "parse_share_code",
    "deserialize_crosshair_code",
    "normalize_crosshair_code",
]
