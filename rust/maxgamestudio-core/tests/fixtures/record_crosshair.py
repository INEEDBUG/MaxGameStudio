"""Record isolated crosshair behavior using the Python app's pinned interpreter.

Run with `python -B` and the app's dependencies on PYTHONPATH. The recorder
uses synthetic inputs, imports only the crosshair router and domain, and
writes the golden JSON to standard output.
"""

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
sys.path[:0] = [str(ROOT / "backend"), str(ROOT / "backend" / "tests")]

from fastapi import HTTPException

from app.valorant_lab.api import _decode_code, _encode_profiles
from app.valorant_lab.codec import parse_crosshair_code, serialize_crosshair_code
from test_valorant_codec import FULL_CODE
from test_valorant_ui_codec import _profiles


def record(name, inputs, call):
    result = {"name": name, "input": inputs}
    try:
        result["output"] = call(inputs)
    except HTTPException as exc:
        result["error"] = exc.detail
    except ValueError as exc:
        result["error"] = str(exc)
    return result


def parse(inputs):
    ast = parse_crosshair_code(**inputs)
    return {
        "normal": serialize_crosshair_code(ast),
        "canonical": serialize_crosshair_code(ast, canonical=True),
        "known": serialize_crosshair_code(ast, preserve_unknown=False),
    }


def encode(profiles):
    return {"code": _encode_profiles(profiles), "format": "valorant-native-v0"}


def decode(code):
    normal, profiles = _decode_code(code)
    return {"code": normal, "profiles": profiles, "format": "valorant-native-v0"}


PARSE = [
    ("all native sections and fields", {"code": FULL_CODE}),
    ("order of sections and unknown duplicates", {"code": "0;future;P;p;1;A;0l;4;P;z;01;a;.3335;c;+05;future;one;future;two;S;t;abcdEf;o;1."}),
    ("case-sensitive field keys", {"code": "0;P;C;7;c;5"}),
    ("tolerant input and duplicate last value", {"code": " 0;P;c;05;h;true;u;#00ff00;h;off; ;", "strict": False}),
    ("tolerant decimal syntax", {"code": "0;P;c;+5.0e0;o;5_005e-4;0l;20e0;0s;2_5e-1", "strict": False}),
    ("unicode digits and a leading BOM", {"code": "\ufeff\ufeff0;P;c;+０５;o;.٣٣٣٥"}),
    ("unicode tolerant boolean aliases", {"code": "0;P;h;yeſ;d;oﬀ", "strict": False}),
    ("negative zero and decimal tie rounding", {"code": "0;P;o;-0.000;a;.0005;0s;1.2345;S;s;3.9995"}),
    ("blank", {"code": ""}),
    ("wrong version", {"code": "1;P;c;5"}),
    ("missing value", {"code": "0;P;c"}),
    ("blank value", {"code": "0;P;c; "}),
    ("invalid boolean", {"code": "0;P;c;5;h;2"}),
    ("strict nonfinite", {"code": "0;P;c;5;o;NaN"}),
    ("tolerant nonfinite", {"code": "0;P;c;5;o;+Inf", "strict": False}),
    ("strict exponent", {"code": "0;P;o;1e-1"}),
    ("strict hexadecimal", {"code": "0;P;u;#ff00FF"}),
    ("invalid field key", {"code": "0;P;custom key;v"}),
    ("duplicate section", {"code": "0;P;c;5;P;c;1"}),
    ("duplicate known field", {"code": "0;P;c;5;c;4"}),
    ("empty key token", {"code": "0;P;"}),
    ("version in key slot", {"code": "0;P;0;v"}),
    ("missing primary", {"code": "0;A;c;5"}),
    ("primary required by AST", {"code": "0;A;c;5", "require_primary": False}),
    ("unknown rejected if opted out", {"code": "0;P;future;one", "allow_unknown": False}),
    ("below minimum before rounding", {"code": "0;P;o;-.00001"}),
    ("above maximum before rounding", {"code": "0;P;o;1.00001"}),
    ("editor-supported Unicode whitespace", {"code": "\u001c0;\u001fP;future;\u001e  keeps inner spaces  \u001d"}),
    ("key length model limit", {"code": f"0;P;{'k' * 65};value"}),
    ("raw value length model limit", {"code": f"0;P;c;{'0' * 1024}5"}),
    ("unknown value line break validation", {"code": "0;P;future;open\nclose"}),
    ("start field count model limit", {"code": "0;" + "k;v;" * 129 + "P"}),
    ("Python integer conversion digit limit", {"code": f"0;P;c;{'0' * 4300}5"}),
    ("Python decimal exponent limit", {"code": "0;P;o;0e1000000000000000000", "strict": False}),
]

ENCODE = [
    ("existing editor fixture", _profiles()),
    ("missing profiles use defaults", {}),
    ("JSON booleans and numeric strings", {"P": {"outlines": 1, "innerLines": "off", "outlineThickness": "１_０", "centerDot": "yeſ", "centerDotThickness": True, "outlineOpacity": " 5.125e-1 ", "unused": []}}),
    ("Python float rounding on editor values", {"P": {"outlineOpacity": 0.0625, "centerDotOpacity": 0.0005, "innerLinesOpacity": 0.5125, "outerLinesOpacity": 0.0001}}),
    ("unknown profiles sorted", {"Z": {}, "Q": {}, "a": {}}),
    ("unknown editor color", {"P": {"color": "not-a-valorant-color"}}),
    ("nonfinite numeric string", {"P": {"outlineOpacity": "inf"}}),
    ("nonfinite numeric sign", {"P": {"outlineOpacity": "-NaN"}}),
    ("nonnumeric string", {"P": {"outlineOpacity": "thin"}}),
    ("invalid underscore in numeric string", {"P": {"outlineOpacity": "0._5"}}),
    ("nonintegral numeric field", {"P": {"innerLinesLength": 1.5}}),
    ("boolean string validation precedes numeric validation", {"P": {"centerDot": "maybe", "outlineOpacity": 11}}),
    ("floating point is not a boolean alias", {"P": {"firingError": 1.0}}),
    ("sniper has narrower emitted thickness range", {"S": {"centerDotThickness": 5}}),
]

DECODE = [
    ("editor payload with all three present", FULL_CODE),
    ("empty primary differs from absent profiles", "0;P"),
    ("empty ADS and sniper profiles", "0;S;P;A"),
    ("unknown duplicate fields and custom color", "0;P;c;8;future;P;future;two;1f;1;1m;1"),
    ("editor uses primary firing and movement switches first", "0;P;c;5;0f;0;1f;1;0m;0;1m;1"),
    ("strict decoder rejects tolerant boolean", "0;P;h;false"),
    ("duplicate known field", "0;P;c;5;c;4"),
    ("invalid known field", "0;P;c;9"),
    ("blank input", "  "),
    ("field metadata errors keep original JSON detail", "0;P;future;open\nclose"),
]


if __name__ == "__main__":
    print(json.dumps({
        "parse": [record(name, value, parse) for name, value in PARSE],
        "encode": [record(name, value, encode) for name, value in ENCODE],
        "decode": [record(name, value, decode) for name, value in DECODE],
    }, indent=2, allow_nan=False))
