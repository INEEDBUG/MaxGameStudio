"""Parser and serializer for native VALORANT crosshair profile codes.

VALORANT profile codes are not a general-purpose JSON format.  They are an
ordered stream of semicolon-separated tokens, with ``0`` as the version and
``P``, ``A`` and ``S`` as profile section markers.  This module validates the
known fields while retaining fields that a newer game build may add.
"""

from __future__ import annotations

import math
import re
from dataclasses import dataclass
from decimal import Decimal, InvalidOperation, ROUND_HALF_UP
from typing import Any, Iterable

from .models import CrosshairField, CrosshairSection, ValorantCrosshairAST


class ValorantCrosshairError(ValueError):
    """Base error raised for malformed or semantically invalid profile codes."""

    def __init__(
        self,
        message: str,
        *,
        token_index: int | None = None,
        section: str | None = None,
        key: str | None = None,
    ) -> None:
        self.token_index = token_index
        self.section = section
        self.key = key
        details: list[str] = []
        if token_index is not None:
            details.append(f"token {token_index}")
        if section is not None:
            details.append(f"section {section}")
        if key is not None:
            details.append(f"field {key}")
        suffix = f" ({', '.join(details)})" if details else ""
        super().__init__(f"{message}{suffix}")


class CrosshairCodeSyntaxError(ValorantCrosshairError):
    """Raised when the token stream cannot be parsed."""


class CrosshairFieldError(ValorantCrosshairError):
    """Raised when a known field has an invalid value."""


class DuplicateCrosshairFieldError(CrosshairCodeSyntaxError):
    """Raised for duplicate known fields in a strict parse."""


@dataclass(frozen=True)
class _FieldSpec:
    kind: str
    minimum: float | int | None = None
    maximum: float | int | None = None


_BOOL = _FieldSpec("bool")
_COLOR = _FieldSpec("int", 0, 8)
_HEX = _FieldSpec("hex")
_OPACITY = _FieldSpec("float", 0, 1)
_SMALL_INT = _FieldSpec("int", 0, 10)
_DOT_INT = _FieldSpec("int", 0, 6)
_INNER_LENGTH = _FieldSpec("int", 0, 20)
_OUTER_LENGTH = _FieldSpec("int", 0, 20)
_INNER_OFFSET = _FieldSpec("int", 0, 20)
_OUTER_OFFSET = _FieldSpec("int", 0, 40)
_MULTIPLIER = _FieldSpec("float", 0, 3)
_SNIPER_THICKNESS = _FieldSpec("float", 0, 4)


# The leading segment before P/A/S contains global switches.  A small number
# of profile-code producers emit these in a different order; the parser keeps
# input order and the canonical serializer can sort them when requested.
_START_SPECS: dict[str, _FieldSpec] = {"s": _BOOL, "c": _BOOL, "p": _BOOL}

# P is the primary profile and A is the ADS profile.  A uses fewer fields in
# older exports, but accepting the complete common set is forward compatible
# and lets us retain a profile copied from a newer client without data loss.
_PROFILE_SPECS: dict[str, _FieldSpec] = {
    "c": _COLOR,
    "u": _HEX,
    "b": _BOOL,
    "h": _BOOL,
    "o": _OPACITY,
    "t": _SMALL_INT,
    "d": _BOOL,
    "a": _OPACITY,
    "z": _DOT_INT,
    "f": _BOOL,
    "s": _BOOL,
    "m": _BOOL,
    "0b": _BOOL,
    "0a": _OPACITY,
    "0l": _INNER_LENGTH,
    "0v": _INNER_LENGTH,
    "0g": _BOOL,
    "0t": _SMALL_INT,
    "0o": _INNER_OFFSET,
    "0m": _BOOL,
    "0s": _MULTIPLIER,
    "0f": _BOOL,
    "0e": _MULTIPLIER,
    "1b": _BOOL,
    "1a": _OPACITY,
    "1l": _OUTER_LENGTH,
    "1v": _OUTER_LENGTH,
    "1g": _BOOL,
    "1t": _SMALL_INT,
    "1o": _OUTER_OFFSET,
    "1m": _BOOL,
    "1s": _MULTIPLIER,
    "1f": _BOOL,
    "1e": _MULTIPLIER,
}

_SNIPER_SPECS: dict[str, _FieldSpec] = {
    "c": _COLOR,
    # In the S profile t is the custom user color, unlike P/A where t is
    # outline thickness.  Some clients use u for the same purpose, so both
    # are recognized and normalized as hex colors.
    "t": _HEX,
    "u": _HEX,
    "d": _BOOL,
    "o": _OPACITY,
    "s": _SNIPER_THICKNESS,
}

_SECTION_SPECS = {"start": _START_SPECS, "P": _PROFILE_SPECS, "A": _PROFILE_SPECS, "S": _SNIPER_SPECS}
_SECTIONS = {"P", "A", "S"}
_KEY_PATTERN = re.compile(r"^[A-Za-z0-9_:-]+$")
_INTEGER_PATTERN = re.compile(r"^[+-]?\d+$")
_DECIMAL_PATTERN = re.compile(r"^[+-]?(?:\d+(?:\.\d*)?|\.\d+)$")
_HEX_PATTERN = re.compile(r"^[0-9A-Fa-f]{6}(?:[0-9A-Fa-f]{2})?$")

_START_ORDER = ("s", "c", "p")
_PROFILE_ORDER = (
    "c", "u", "b", "h", "o", "t", "d", "a", "z", "f", "s", "m",
    "0b", "0a", "0l", "0v", "0g", "0t", "0o", "0m", "0s", "0f", "0e",
    "1b", "1a", "1l", "1v", "1g", "1t", "1o", "1m", "1s", "1f", "1e",
)
_SNIPER_ORDER = ("c", "s", "o", "d", "t", "u")


def _spec_for(section: str, key: str) -> _FieldSpec | None:
    return _SECTION_SPECS.get(section, {}).get(key)


def _format_decimal(value: Decimal, *, places: int = 3) -> str:
    quantizer = Decimal(1).scaleb(-places)
    normalized = value.quantize(quantizer, rounding=ROUND_HALF_UP)
    text = format(normalized, "f").rstrip("0").rstrip(".")
    return text or "0"


def _normalize_bool(raw: str, *, strict: bool) -> str:
    value = raw.strip().casefold()
    if value in {"0", "1"}:
        return value
    if not strict and value in {"true", "yes", "on"}:
        return "1"
    if not strict and value in {"false", "no", "off"}:
        return "0"
    raise ValueError("expected 0 or 1")


def _normalize_integer(raw: str, spec: _FieldSpec, *, strict: bool) -> str:
    value = raw.strip()
    if _INTEGER_PATTERN.fullmatch(value):
        integer = int(value, 10)
    elif not strict:
        try:
            decimal = Decimal(value)
            if not decimal.is_finite() or decimal != decimal.to_integral_value():
                raise ValueError
            integer = int(decimal)
        except (InvalidOperation, ValueError, TypeError, OverflowError) as exc:
            raise ValueError("expected an integer") from exc
    else:
        raise ValueError("expected an integer")
    if spec.minimum is not None and integer < spec.minimum:
        raise ValueError(f"must be at least {spec.minimum:g}")
    if spec.maximum is not None and integer > spec.maximum:
        raise ValueError(f"must be at most {spec.maximum:g}")
    return str(integer)


def _normalize_float(raw: str, spec: _FieldSpec, *, strict: bool) -> str:
    value = raw.strip()
    if strict and not _DECIMAL_PATTERN.fullmatch(value):
        raise ValueError("expected a finite decimal number")
    try:
        decimal = Decimal(value)
    except (InvalidOperation, ValueError) as exc:
        raise ValueError("expected a finite decimal number") from exc
    if not decimal.is_finite():
        raise ValueError("NaN and infinity are not valid native values")
    if spec.minimum is not None and decimal < Decimal(str(spec.minimum)):
        raise ValueError(f"must be at least {spec.minimum:g}")
    if spec.maximum is not None and decimal > Decimal(str(spec.maximum)):
        raise ValueError(f"must be at most {spec.maximum:g}")
    return _format_decimal(decimal)


def _normalize_hex(raw: str, *, strict: bool) -> str:
    value = raw.strip()
    if not strict and value.startswith("#"):
        value = value[1:]
    if not _HEX_PATTERN.fullmatch(value):
        raise ValueError("expected a 6- or 8-digit hexadecimal color")
    return value.upper()


def normalize_field(section: str, key: str, raw_value: str, *, strict: bool = True) -> tuple[str, bool]:
    """Normalize one field and return ``(canonical_value, is_known)``.

    Unknown fields are intentionally not interpreted: their token text is
    retained so future game fields survive an import/export cycle.
    """

    if not isinstance(section, str) or section not in {"start", "P", "A", "S"}:
        raise CrosshairFieldError("unsupported VALORANT section", section=section)
    if not isinstance(key, str) or not _KEY_PATTERN.fullmatch(key):
        raise CrosshairFieldError("invalid field key", section=section, key=key)
    if not isinstance(raw_value, str):
        raise CrosshairFieldError("field value must be text", section=section, key=key)
    value = raw_value.strip()
    if not value:
        raise CrosshairFieldError("field value must not be blank", section=section, key=key)
    spec = _spec_for(section, key)
    if spec is None:
        if not strict and any(char.isspace() for char in value):
            # Tolerant mode only removes surrounding whitespace; a space in
            # an unknown value is valid text and remains untouched otherwise.
            value = value
        return value, False
    try:
        if spec.kind == "bool":
            return _normalize_bool(value, strict=strict), True
        if spec.kind == "int":
            return _normalize_integer(value, spec, strict=strict), True
        if spec.kind == "float":
            return _normalize_float(value, spec, strict=strict), True
        if spec.kind == "hex":
            return _normalize_hex(value, strict=strict), True
    except ValueError as exc:
        raise CrosshairFieldError(str(exc), section=section, key=key) from exc
    raise CrosshairFieldError("unsupported field specification", section=section, key=key)


def _new_field(section: str, key: str, raw_value: str, *, strict: bool, allow_unknown: bool, token_index: int) -> CrosshairField:
    try:
        normalized, known = normalize_field(section, key, raw_value, strict=strict)
    except ValorantCrosshairError as exc:
        if exc.token_index is None:
            raise type(exc)(str(exc), token_index=token_index, section=section, key=key) from exc
        raise
    if not allow_unknown and not known:
        raise CrosshairFieldError("unknown field is not allowed", token_index=token_index, section=section, key=key)
    return CrosshairField(
        key=key,
        value=normalized,
        raw_value=raw_value.strip(),
        known=known,
        section=section,
    )


def parse_crosshair_code(
    code: str,
    *,
    strict: bool = True,
    allow_unknown: bool = True,
    require_primary: bool = True,
) -> ValorantCrosshairAST:
    """Parse a native VALORANT profile code into an ordered AST.

    Strict mode enforces version ``0``, known-field ranges, finite numbers,
    complete key/value pairs, unique known keys and unique profile sections.
    Tolerant mode still rejects structural corruption but accepts common human
    edits such as ``true``/``false`` booleans, a leading ``#`` on colors and a
    trailing semicolon.
    """

    if not isinstance(code, str):
        raise CrosshairCodeSyntaxError("crosshair code must be text")
    text = code.strip().lstrip("\ufeff")
    if not text:
        raise CrosshairCodeSyntaxError("crosshair code must not be blank")
    tokens = text.split(";")
    if not strict:
        while tokens and not tokens[-1].strip():
            tokens.pop()
    if not tokens or tokens[0].strip() != "0":
        raise CrosshairCodeSyntaxError("VALORANT crosshair code must start with version 0", token_index=0)

    start_fields: list[CrosshairField] = []
    sections: list[CrosshairSection] = []
    current: CrosshairSection | None = None
    seen_sections: set[str] = set()
    seen_keys: dict[str, set[str]] = {"start": set()}
    index = 1
    while index < len(tokens):
        token = tokens[index].strip()
        if not token:
            raise CrosshairCodeSyntaxError("empty token is not valid", token_index=index)
        if token in _SECTIONS:
            if token in seen_sections:
                raise CrosshairCodeSyntaxError("profile section is repeated", token_index=index, section=token)
            seen_sections.add(token)
            current = CrosshairSection(name=token)  # type: ignore[arg-type]
            sections.append(current)
            seen_keys[token] = set()
            index += 1
            continue
        if token == "0":
            raise CrosshairCodeSyntaxError("version token may only appear at the beginning", token_index=index)
        if not _KEY_PATTERN.fullmatch(token):
            raise CrosshairCodeSyntaxError("invalid field key", token_index=index, key=token)
        if index + 1 >= len(tokens):
            raise CrosshairCodeSyntaxError("field key is missing a value", token_index=index, key=token)
        raw_value = tokens[index + 1].strip()
        if not raw_value:
            raise CrosshairCodeSyntaxError("field value must not be blank", token_index=index + 1, key=token)
        section_name = current.name if current is not None else "start"
        # A reserved section marker cannot be a key/value value boundary.  It
        # is still allowed as the value of an unknown field, since the parser
        # only treats markers when they occur where a key is expected.
        field = _new_field(
            section_name,
            token,
            raw_value,
            strict=strict,
            allow_unknown=allow_unknown,
            token_index=index,
        )
        if field.known and token in seen_keys.setdefault(section_name, set()):
            if strict:
                raise DuplicateCrosshairFieldError(
                    "known field is repeated",
                    token_index=index,
                    section=section_name,
                    key=token,
                )
        seen_keys.setdefault(section_name, set()).add(token)
        if current is None:
            start_fields.append(field)
        else:
            current.fields.append(field)
        index += 2

    if require_primary and "P" not in seen_sections:
        raise CrosshairCodeSyntaxError("crosshair code must contain a P primary section")
    try:
        return ValorantCrosshairAST(version=0, start_fields=start_fields, sections=sections)
    except ValueError as exc:
        raise CrosshairCodeSyntaxError(str(exc)) from exc


def _ordered_fields(fields: Iterable[CrosshairField], order: tuple[str, ...], *, preserve_unknown: bool) -> list[CrosshairField]:
    source = list(fields)
    known = {field.key: field for field in source if field.known}
    ordered = [known[key] for key in order if key in known]
    unknown = [field for field in source if not field.known] if preserve_unknown else []
    return ordered + unknown


def serialize_crosshair_code(
    ast: ValorantCrosshairAST | str,
    *,
    preserve_unknown: bool = True,
    canonical: bool = False,
) -> str:
    """Serialize an AST, normalizing known values and retaining unknown ones.

    ``canonical=False`` preserves input field and section order, which is the
    safest import/export behavior.  ``canonical=True`` orders recognized keys
    using the documented native order and appends unknown keys afterward.
    """

    if isinstance(ast, str):
        ast = parse_crosshair_code(ast)
    if not isinstance(ast, ValorantCrosshairAST):
        raise TypeError("serialize_crosshair_code expects a ValorantCrosshairAST or code string")

    def field_value(field: CrosshairField, section: str) -> str:
        if not field.known:
            return field.raw_value if field.raw_value is not None else field.value
        # ASTs are also a public domain-layer input, so do not trust a caller
        # that manually constructed a ``known=True`` field.  Re-validate it at
        # the serialization boundary just as the parser does.
        try:
            normalized, known = normalize_field(section, field.key, field.value, strict=True)
        except ValorantCrosshairError:
            raise
        if not known:
            raise CrosshairFieldError(
                "known field is not recognized for this section",
                section=section,
                key=field.key,
            )
        return normalized

    parts = ["0"]
    start_fields = _ordered_fields(ast.start_fields, _START_ORDER, preserve_unknown=preserve_unknown) if canonical else list(ast.start_fields)
    for field in start_fields:
        if not preserve_unknown and not field.known:
            continue
        parts.extend([field.key, field_value(field, "start")])
    for section in ast.sections:
        parts.append(section.name)
        if canonical:
            if section.name == "S":
                fields = _ordered_fields(section.fields, _SNIPER_ORDER, preserve_unknown=preserve_unknown)
            else:
                fields = _ordered_fields(section.fields, _PROFILE_ORDER, preserve_unknown=preserve_unknown)
        else:
            fields = list(section.fields)
        for field in fields:
            if not preserve_unknown and not field.known:
                continue
            parts.extend([field.key, field_value(field, section.name)])
    return ";".join(parts)


def normalize_crosshair_code(code: str, *, strict: bool = True) -> str:
    """Parse and export a code so all known values use canonical spelling."""

    return serialize_crosshair_code(parse_crosshair_code(code, strict=strict))


# Naming aliases make the domain layer convenient for API callers and tests.
parse_valorant_crosshair_code = parse_crosshair_code
serialize_valorant_crosshair_code = serialize_crosshair_code
parse_share_code = parse_crosshair_code
serialize_share_code = serialize_crosshair_code
deserialize_crosshair_code = parse_crosshair_code


__all__ = [
    "ValorantCrosshairError",
    "CrosshairCodeSyntaxError",
    "CrosshairFieldError",
    "DuplicateCrosshairFieldError",
    "normalize_field",
    "parse_crosshair_code",
    "serialize_crosshair_code",
    "normalize_crosshair_code",
    "parse_valorant_crosshair_code",
    "serialize_valorant_crosshair_code",
    "parse_share_code",
    "serialize_share_code",
    "deserialize_crosshair_code",
]
