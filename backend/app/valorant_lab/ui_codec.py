"""Bridge between the lab editor schema and native VALORANT share codes."""

from __future__ import annotations

from copy import deepcopy
import math
from typing import Any, Mapping

from .codec import normalize_field, parse_crosshair_code, serialize_crosshair_code
from .models import CrosshairField, CrosshairSection, ValorantCrosshairAST


COLOR_TO_ID = {
    "white": 0,
    "green": 1,
    "yellowgreen": 2,
    "greenyellow": 3,
    "yellow": 4,
    "cyan": 5,
    "pink": 6,
    "red": 7,
}
ID_TO_COLOR = {value: key for key, value in COLOR_TO_ID.items()}


class UICodecError(ValueError):
    """Raised when an editor payload cannot be represented natively."""

DEFAULT_PROFILE = {
    "color": "green",
    "outlines": True,
    "outlineOpacity": 0.8,
    "outlineThickness": 1,
    "centerDot": False,
    "centerDotOpacity": 1,
    "centerDotThickness": 2,
    "innerLines": True,
    "innerLinesOpacity": 1,
    "innerLinesLength": 4,
    "innerLinesThickness": 2,
    "innerLinesOffset": 2,
    "outerLines": False,
    "outerLinesOpacity": 0.5,
    "outerLinesLength": 2,
    "outerLinesThickness": 2,
    "outerLinesOffset": 3,
    "firingError": False,
    "movementError": False,
}


def _bool(value: Any) -> str:
    """Convert only explicit boolean-like values, never arbitrary truthiness."""

    if isinstance(value, bool):
        return "1" if value else "0"
    if isinstance(value, int) and value in (0, 1):
        return str(value)
    if isinstance(value, str):
        normalized = value.strip().casefold()
        if normalized in {"1", "true", "yes", "on"}:
            return "1"
        if normalized in {"0", "false", "no", "off"}:
            return "0"
    raise UICodecError("boolean editor fields must be true/false or 0/1")


def _number(
    value: Any,
    default: float | int,
    *,
    minimum: float,
    maximum: float,
    integer: bool = False,
) -> str:
    """Validate a numeric editor value before it becomes an AST field."""

    try:
        number = float(value)
    except (OverflowError, TypeError, ValueError) as exc:
        raise UICodecError("numeric editor fields must be finite numbers") from exc
    if not math.isfinite(number) or number < minimum or number > maximum:
        raise UICodecError(
            f"numeric editor field must be finite and within [{minimum:g}, {maximum:g}]"
        )
    if integer and not number.is_integer():
        raise UICodecError("this editor field must be an integer")
    if number.is_integer():
        return str(int(number))
    return f"{number:.3f}".rstrip("0").rstrip(".")


def _profile(value: Any) -> dict[str, Any]:
    if value is None:
        value = {}
    if not isinstance(value, Mapping):
        raise UICodecError("each crosshair profile must be an object")
    source = dict(value)
    result = deepcopy(DEFAULT_PROFILE)
    color = source.get("color", result["color"])
    if not isinstance(color, str) or color not in COLOR_TO_ID:
        raise UICodecError("crosshair color is not a supported preset")
    result["color"] = color
    boolean_fields = ("outlines", "centerDot", "innerLines", "outerLines", "firingError", "movementError")
    for key in boolean_fields:
        if key in source:
            result[key] = _bool(source[key]) == "1"

    numeric_fields = {
        "outlineOpacity": (0.0, 1.0, False),
        "outlineThickness": (0.0, 10.0, True),
        "centerDotOpacity": (0.0, 1.0, False),
        "centerDotThickness": (0.0, 6.0, True),
        "innerLinesOpacity": (0.0, 1.0, False),
        "innerLinesLength": (0.0, 20.0, True),
        "innerLinesThickness": (0.0, 10.0, True),
        "innerLinesOffset": (0.0, 20.0, True),
        "outerLinesOpacity": (0.0, 1.0, False),
        "outerLinesLength": (0.0, 20.0, True),
        "outerLinesThickness": (0.0, 10.0, True),
        "outerLinesOffset": (0.0, 40.0, True),
    }
    for key, (minimum, maximum, integer) in numeric_fields.items():
        if key in source:
            normalized = _number(
                source[key],
                result[key],
                minimum=minimum,
                maximum=maximum,
                integer=integer,
            )
            result[key] = float(normalized) if not integer else int(normalized)
    return result


def _field(section: str, key: str, value: Any) -> CrosshairField:
    try:
        normalized, known = normalize_field(section, key, str(value), strict=True)
    except ValueError as exc:
        raise UICodecError(f"invalid native field {section}.{key}: {exc}") from exc
    if not known:
        raise UICodecError(f"UI codec attempted to emit unknown field {section}.{key}")
    return CrosshairField(
        key=key,
        value=normalized,
        raw_value=str(value),
        known=True,
        section=section,
    )


def _standard_section(name: str, value: Mapping[str, Any]) -> CrosshairSection:
    data = _profile(value)
    fields = [
        _field(name, "c", COLOR_TO_ID[data["color"]]),
        _field(name, "h", _bool(data["outlines"])),
        _field(name, "o", _number(data["outlineOpacity"], 0.8, minimum=0, maximum=1)),
        _field(name, "t", _number(data["outlineThickness"], 1, minimum=0, maximum=10, integer=True)),
        _field(name, "d", _bool(data["centerDot"])),
        _field(name, "a", _number(data["centerDotOpacity"], 1, minimum=0, maximum=1)),
        _field(name, "z", _number(data["centerDotThickness"], 2, minimum=0, maximum=6, integer=True)),
        _field(name, "f", "0"),
        _field(name, "0b", _bool(data["innerLines"])),
        _field(name, "0a", _number(data["innerLinesOpacity"], 1, minimum=0, maximum=1)),
        _field(name, "0l", _number(data["innerLinesLength"], 4, minimum=0, maximum=20, integer=True)),
        _field(name, "0t", _number(data["innerLinesThickness"], 2, minimum=0, maximum=10, integer=True)),
        _field(name, "0o", _number(data["innerLinesOffset"], 2, minimum=0, maximum=20, integer=True)),
        _field(name, "0m", _bool(data["movementError"])),
        _field(name, "0f", _bool(data["firingError"])),
        _field(name, "1b", _bool(data["outerLines"])),
        _field(name, "1a", _number(data["outerLinesOpacity"], 0.5, minimum=0, maximum=1)),
        _field(name, "1l", _number(data["outerLinesLength"], 2, minimum=0, maximum=20, integer=True)),
        _field(name, "1t", _number(data["outerLinesThickness"], 2, minimum=0, maximum=10, integer=True)),
        _field(name, "1o", _number(data["outerLinesOffset"], 3, minimum=0, maximum=40, integer=True)),
        _field(name, "1m", _bool(data["movementError"])),
        _field(name, "1f", _bool(data["firingError"])),
    ]
    return CrosshairSection(name=name, fields=fields)  # type: ignore[arg-type]


def _sniper_section(value: Mapping[str, Any]) -> CrosshairSection:
    data = _profile(value)
    fields = [
        _field("S", "c", COLOR_TO_ID[data["color"]]),
        _field("S", "d", _bool(data["centerDot"])),
        _field("S", "o", _number(data["centerDotOpacity"], 1, minimum=0, maximum=1)),
        _field("S", "s", _number(data["centerDotThickness"], 2, minimum=0, maximum=4)),
    ]
    return CrosshairSection(name="S", fields=fields)


def ui_profiles_to_code(profiles: Mapping[str, Any]) -> str:
    if not isinstance(profiles, Mapping):
        raise UICodecError("crosshair profiles must be an object")
    allowed_profiles = {"P", "A", "S"}
    unknown_profiles = sorted(str(key) for key in profiles.keys() if key not in allowed_profiles)
    if unknown_profiles:
        raise UICodecError(
            "unsupported crosshair profile(s): " + ", ".join(unknown_profiles)
        )
    ast = ValorantCrosshairAST(
        start_fields=[_field("start", "s", "1")],
        sections=[
            _standard_section("P", profiles.get("P", {})),
            _standard_section("A", profiles.get("A", {})),
            _sniper_section(profiles.get("S", {})),
        ],
    )
    # Re-parse the generated code through the native validator as a final
    # boundary check.  The editor has its own ranges, but API callers can
    # bypass the UI and must never be able to persist an invalid share code.
    generated = serialize_crosshair_code(ast, canonical=True)
    validated = parse_crosshair_code(generated, strict=True, allow_unknown=False)
    return serialize_crosshair_code(validated, canonical=True)


def _section_to_profile(section: CrosshairSection | None, *, sniper: bool = False) -> dict[str, Any]:
    data = deepcopy(DEFAULT_PROFILE)
    if section is None:
        return data

    def get(key: str, default: str) -> str:
        return section.get(key, default) or default

    try:
        color_id = int(get("c", "1"))
    except ValueError:
        color_id = 1
    data["color"] = ID_TO_COLOR.get(color_id, "white")
    if sniper:
        data["centerDot"] = get("d", "1") == "1"
        data["centerDotOpacity"] = float(get("o", "1"))
        data["centerDotThickness"] = float(get("s", "2"))
        data["innerLines"] = False
        data["outerLines"] = False
        return data
    data.update(
        {
            "outlines": get("h", "1") == "1",
            "outlineOpacity": float(get("o", "0.5")),
            "outlineThickness": float(get("t", "1")),
            "centerDot": get("d", "0") == "1",
            "centerDotOpacity": float(get("a", "1")),
            "centerDotThickness": float(get("z", "2")),
            "innerLines": get("0b", "1") == "1",
            "innerLinesOpacity": float(get("0a", "0.8")),
            "innerLinesLength": float(get("0l", "6")),
            "innerLinesThickness": float(get("0t", "2")),
            "innerLinesOffset": float(get("0o", "3")),
            "outerLines": get("1b", "1") == "1",
            "outerLinesOpacity": float(get("1a", "0.35")),
            "outerLinesLength": float(get("1l", "2")),
            "outerLinesThickness": float(get("1t", "2")),
            "outerLinesOffset": float(get("1o", "10")),
            "movementError": get("0m", get("1m", "0")) == "1",
            "firingError": get("0f", get("1f", "0")) == "1",
        }
    )
    return data


def code_to_ui_profiles(code: str) -> dict[str, dict[str, Any]]:
    ast = parse_crosshair_code(code, strict=False, allow_unknown=True)
    return {
        "P": _section_to_profile(ast.section("P")),
        "A": _section_to_profile(ast.section("A")),
        "S": _section_to_profile(ast.section("S"), sniper=True),
    }
