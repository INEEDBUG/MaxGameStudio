from __future__ import annotations

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from app.valorant_lab import (  # noqa: E402
    CrosshairCodeSyntaxError,
    CrosshairFieldError,
    CrosshairPreset,
    DuplicateCrosshairFieldError,
    normalize_crosshair_code,
    parse_crosshair_code,
    serialize_crosshair_code,
)


FULL_CODE = (
    "0;s;1;c;0;p;1;P;c;05;u;00ff00ff;h;0;o;0.5000;t;01;d;1;z;03;"
    "0b;1;0a;1.000;0l;004;0v;4;0g;0;0t;1;0o;2;0m;0;0s;1.000;0f;0;0e;1.003;"
    "1b;0;1a;0.35;1l;2;1v;2;1g;0;1t;1;1o;3;1m;0;1s;1;1f;0;1e;1;future-key;keep-me;"
    "A;c;6;h;0;0l;4;0o;2;0a;1;0f;0;1b;0;ads-future;7;"
    "S;c;4;s;1.157;o;1;d;1;t;000000"
)


def test_parser_supports_all_native_sections_and_normalizes_known_values():
    ast = parse_crosshair_code(FULL_CODE)

    assert ast.version == 0
    assert [section.name for section in ast.sections] == ["P", "A", "S"]
    assert ast.values("start") == {"s": "1", "c": "0", "p": "1"}
    assert ast.get("P", "c") == "5"
    assert ast.get("P", "o") == "0.5"
    assert ast.get("P", "0l") == "4"
    assert ast.get("P", "future-key") == "keep-me"
    assert ast.get("S", "s") == "1.157"
    assert ast.section("P").get_field("future-key").known is False


def test_serializer_preserves_order_and_unknown_fields_for_round_trip():
    ast = parse_crosshair_code(FULL_CODE)
    exported = serialize_crosshair_code(ast)
    reparsed = parse_crosshair_code(exported)

    assert exported == normalize_crosshair_code(FULL_CODE)
    assert "future-key;keep-me" in exported
    assert "ads-future;7" in exported
    assert reparsed.values("start") == ast.values("start")
    assert reparsed.values("P") == ast.values("P")
    assert reparsed.values("A") == ast.values("A")
    assert reparsed.values("S") == ast.values("S")


def test_canonical_serializer_orders_known_fields_but_keeps_unknown_fields():
    ast = parse_crosshair_code("0;P;future;raw;0l;4;c;5;0t;1")

    assert serialize_crosshair_code(ast, canonical=True) == "0;P;c;5;0l;4;0t;1;future;raw"


def test_tolerant_mode_accepts_common_human_edits_and_still_normalizes():
    ast = parse_crosshair_code(" 0;P;c;05;h;true;u;#00ff00; ", strict=False)

    assert ast.get("P", "c") == "5"
    assert ast.get("P", "h") == "1"
    assert ast.get("P", "u") == "00FF00"
    assert ast.to_code() == "0;P;c;5;h;1;u;00FF00"


@pytest.mark.parametrize(
    "value",
    [
        "",
        "1;P;c;5",
        "0;P;c",
        "0;P;c;5;h;2",
        "0;P;c;5;o;NaN",
        "0;P;c;5;P;c;1",
        "0;A;c;5",
    ],
)
def test_strict_parser_rejects_structural_and_known_field_errors(value: str):
    with pytest.raises((CrosshairCodeSyntaxError, CrosshairFieldError)):
        parse_crosshair_code(value)


def test_strict_parser_rejects_duplicate_known_field_but_retains_duplicate_unknown_field():
    with pytest.raises(DuplicateCrosshairFieldError):
        parse_crosshair_code("0;P;c;5;c;4")

    ast = parse_crosshair_code("0;P;c;5;future;one;future;two")
    assert [field.value for field in ast.section("P").fields if field.key == "future"] == ["one", "two"]
    assert ast.to_code() == "0;P;c;5;future;one;future;two"


def test_ast_mutation_uses_the_same_known_field_normalization():
    ast = parse_crosshair_code("0;P;c;5")
    ast.set_field("P", "0o", "007")
    ast.set_field("P", "unknown-new-key", "value")

    assert ast.get("P", "0o") == "7"
    assert ast.get("P", "unknown-new-key") == "value"
    assert ast.to_code() == "0;P;c;5;0o;7;unknown-new-key;value"


def test_preset_model_validates_and_exposes_an_ast():
    preset = CrosshairPreset(name="  Small cyan  ", code="0;P;c;05;h;0", tags=["aim", " aim "])

    assert preset.name == "Small cyan"
    assert preset.code == "0;P;c;5;h;0"
    assert preset.tags == ["aim"]
    assert preset.ast.get("P", "c") == "5"
