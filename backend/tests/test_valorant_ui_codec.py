from __future__ import annotations

import math
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from app.valorant_lab.codec import parse_crosshair_code
from app.valorant_lab.ui_codec import (
    DEFAULT_PROFILE,
    UICodecError,
    code_to_ui_profiles,
    ui_profiles_to_code,
)


def _profiles() -> dict[str, dict[str, object]]:
    return {
        "P": {**DEFAULT_PROFILE, "color": "cyan", "centerDot": True, "innerLinesLength": 6},
        "A": {**DEFAULT_PROFILE, "color": "yellow", "outerLines": True},
        "S": {**DEFAULT_PROFILE, "color": "red", "centerDot": True},
    }


def test_ui_profiles_emit_native_v0_code_and_round_trip_known_values():
    code = ui_profiles_to_code(_profiles())

    assert code.startswith("0;s;1;P;")
    ast = parse_crosshair_code(code, strict=True, allow_unknown=False)
    assert ast.get("P", "c") == "5"  # cyan
    assert ast.get("P", "0l") == "6"
    assert ast.get("A", "c") == "4"  # yellow
    assert ast.get("S", "c") == "7"  # red

    restored = code_to_ui_profiles(code)
    assert restored["P"]["color"] == "cyan"
    assert restored["P"]["centerDot"] is True
    assert restored["P"]["innerLinesLength"] == 6.0
    assert restored["A"]["outerLines"] is True
    assert restored["S"]["centerDot"] is True


@pytest.mark.parametrize(
    "profiles",
    [
        {"Q": {}},
        {"P": {"color": "not-a-valorant-color"}},
        {"P": {"outlineOpacity": math.nan}},
        {"P": {"outlineOpacity": math.inf}},
        {"P": {"innerLinesLength": 1.5}},
        {"P": {"centerDot": "maybe"}},
    ],
)
def test_illegal_ui_profiles_raise_a_validation_error(profiles):
    with pytest.raises(UICodecError):
        ui_profiles_to_code(profiles)


def test_ui_codec_uses_defaults_only_for_missing_profiles():
    code = ui_profiles_to_code({})
    restored = code_to_ui_profiles(code)

    assert restored["P"]["color"] == DEFAULT_PROFILE["color"]
    assert restored["A"]["innerLines"] is True
    assert restored["S"]["outerLines"] is False


def test_code_to_ui_profiles_rejects_malformed_native_code():
    with pytest.raises(ValueError):
        code_to_ui_profiles("0;P;c;9")
