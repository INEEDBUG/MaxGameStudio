"""Record synthetic training calculations from the existing Python domain code."""

import copy
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[4] / "backend"))

from pydantic import ValidationError

from app.input_lab import InputLabRequest, analyze_input_session
from app.sensitivity_lab import SensitivityRecommendationRequest, recommend_sensitivity


def record(name, payload, model, calculate):
    case = {"name": name, "input": payload}
    try:
        case["output"] = calculate(model.model_validate(payload)).model_dump(mode="json")
    except ValidationError as err:
        first = err.errors()[0]
        case["error"] = {
            "field": ".".join(map(str, first["loc"])),
            "kind": first["type"],
            "message": first["msg"],
        }
    return case


def trial(kind, multiplier, quality):
    sample = {"kind": kind, "multiplier": multiplier, "duration_ms": 15_000}
    if kind == "flick":
        return {
            **sample, "hits": round(20 * quality), "targets": 20,
            "average_reaction_ms": 900 - 500 * quality, "path_efficiency": quality,
            "overshoots": round(8 * (1 - quality)),
        }
    return {
        **sample, "targets": 60, "on_target_ratio": quality, "path_efficiency": quality,
        "overshoots": round(12 * (1 - quality)),
    }


def aim(**updates):
    return {
        "dpi": 800, "current_sensitivity": 1.0, "game_width": 1024, "game_height": 1080,
        "trials": [trial(kind, multiplier, quality) for multiplier, quality in [
            (0.8, 0.45), (1.0, 0.60), (1.2, 0.94)
        ] for kind in ("flick", "tracking")],
        **updates,
    }


def click_metrics(under, over, off_axis=0):
    payload = aim()
    for target in payload["trials"][::2]:
        target.update(
            clicks=target["hits"] + 10, misses=10, underflicks=under, overflicks=over,
            off_axis_misses=off_axis, average_click_error_ratio=0.18,
        )
    return payload


def amend_first_trial(**changes):
    payload = aim()
    payload["trials"][0].update(changes)
    return payload


def key_input(events, **updates):
    return {
        "mode": "counter_strafe", "actuation_mm": 1.0, "rapid_trigger_press_mm": 0.2,
        "rapid_trigger_release_mm": 0.2, "duration_ms": 10_000,
        "events": [{"code": code, "event_type": event_type, "timestamp_ms": time}
                   for code, event_type, time in events],
        **updates,
    }


BASE_KEYS = [
    ("KeyA", "down", 100), ("KeyA", "up", 200),
    ("KeyD", "down", 220), ("KeyA", "down", 300),
    ("KeyD", "up", 310), ("KeyA", "up", 390),
]


def aim_cases():
    tie = [trial(kind, multiplier, 0.7)
           for multiplier in (1.25, 0.75) for kind in ("flick", "tracking")]
    split = aim(trials=[
        trial("flick", 0.75, 0.95), trial("tracking", 0.75, 0.25),
        trial("flick", 1.25, 0.25), trial("tracking", 1.25, 0.95),
    ])
    low = aim(current_sensitivity=0.003)
    high = aim(dpi=32_000, current_sensitivity=25, m_yaw=1, game_width=16_384, game_height=240)
    low_candidate = aim(trials=[
        trial("flick", 0.5, 0.97), trial("tracking", 0.5, 0.99),
        trial("flick", 0.7, 0.1), trial("tracking", 0.7, 0.1),
    ])
    partial = [trial("flick", 1.05049, 0.9), trial("tracking", 1.05, 0.85),
               trial("flick", 0.9, 0.87), trial("flick", 0.9, 0.84)]
    return [
        ("defaults from the original no-click client", aim()),
        ("early click corroborates a faster recommendation", click_metrics(8, 0)),
        ("late clicks conflict without overriding settings", click_metrics(0, 8)),
        ("off-axis misses excluded from the direction denominator", click_metrics(1, 0, 9)),
        ("split between flick and tracking", split),
        ("tied scores preserve candidate source order", aim(trials=tie)),
        ("rounded candidate groups and partial coverage", aim(trials=partial)),
        ("black bars and custom yaw", aim(m_yaw=0.03333335, display_aspect="4:3", scaling_mode="black_bars")),
        ("native and scientific yaw label", aim(m_yaw=0.0000125, display_aspect="other", scaling_mode="native")),
        ("lower sensitivity clamp", low),
        ("upper DPI and sensitivity clamps", high),
        ("lower multiplier clamp", low_candidate),
        ("minimum DPI", aim(dpi=99)),
        ("maximum DPI", aim(dpi=32_001)),
        ("nonpositive sensitivity", aim(current_sensitivity=0)),
        ("yaw outside permitted range", aim(m_yaw=1.01)),
        ("maximum game resolution", aim(game_width=16_385)),
        ("minimum trial count", aim(trials=aim()["trials"][:3])),
        ("both trial kinds required", aim(trials=[trial("flick", 0.8, 0.8)] * 4)),
        ("two rounded candidate groups required", aim(trials=[
            trial(kind, multiplier, 0.8)
            for multiplier in (0.9001, 0.9002) for kind in ("flick", "tracking")
        ])),
        ("invalid multiplier", amend_first_trial(multiplier=0.49)),
        ("minimum trial duration", amend_first_trial(duration_ms=2_999)),
        ("legacy click consistency", amend_first_trial(misses=1)),
        ("clicks must equal hits plus misses", amend_first_trial(clicks=5, hits=3, misses=1)),
        ("off-axis misses never exceed misses", amend_first_trial(clicks=11, hits=10, misses=1, off_axis_misses=2)),
        ("directional misses cannot exceed miss count", amend_first_trial(clicks=13, hits=10, misses=3, off_axis_misses=2, underflicks=2)),
        ("all click direction counts cannot exceed clicks", amend_first_trial(clicks=13, hits=10, misses=3, underflicks=3, overflicks=11)),
    ]


def key_cases():
    slow = [("KeyA", "down", 0), ("KeyA", "up", 100),
            ("KeyD", "down", 201), ("KeyD", "up", 301)]
    rapid = [("KeyW", "down", 100), ("KeyW", "up", 115),
             ("Space", "down", 120), ("Space", "up", 135)]
    jitter = [("KeyS", "down", 50), ("KeyS", "up", 50),
              ("KeyS", "down", 75), ("KeyS", "up", 147)]
    reverse = copy.deepcopy(BASE_KEYS)
    reverse[2] = ("KeyD", "down", 190)
    return [
        ("real key gaps and overlaps from browser events", key_input(BASE_KEYS)),
        ("overlap has no diagnosis in gameplay", key_input(BASE_KEYS, mode="gameplay")),
        ("slow reversal with minimum millimeter clamps", key_input(slow, actuation_mm=0.1, rapid_trigger_press_mm=0.05)),
        ("rapid tap balanced", key_input(rapid, mode="rapid_tap")),
        ("inconsistent holds take priority", key_input(jitter)),
        ("equal times and exact jitter threshold", key_input([
            ("KeyS", "down", 0), ("KeyS", "up", 0), ("KeyW", "down", 0), ("KeyW", "up", 70)
        ])),
        ("missing and repeated edges with upper bounds", key_input([
            ("KeyD", "up", 0), ("KeyA", "down", 10), ("KeyA", "down", 12), ("KeyA", "up", 100)
        ], actuation_mm=4.0, rapid_trigger_press_mm=4.0, rapid_trigger_release_mm=3.98)),
        ("end-held keys close without chatter", key_input([
            ("KeyW", "down", 2998.625), ("Space", "down", 3001.5)
        ], duration_ms=3000)),
        ("missing downs leave no hold data", key_input([
            ("KeyS", "up", 0), ("KeyW", "up", 10),
        ])),
        ("long manual session", key_input(rapid, duration_ms=86_400_000)),
        ("short duration rejected", key_input(rapid, duration_ms=2999)),
        ("empty keyboard name rejected", key_input(rapid, keyboard_name="")),
        ("unsupported keyboard code rejected", key_input([("KeyQ", "down", 0), ("KeyQ", "up", 5)])),
        ("chronology violation", key_input(reverse)),
        ("timestamp range", key_input([("KeyW", "down", -1), ("KeyW", "up", 0)])),
        ("actuation lower bound", key_input(rapid, actuation_mm=0.09)),
        ("trigger upper bound", key_input(rapid, rapid_trigger_release_mm=4.01)),
        ("minimum event sample", key_input(rapid[:1])),
    ]


if __name__ == "__main__":
    print(json.dumps({
        "sensitivity": [record(name, data, SensitivityRecommendationRequest, recommend_sensitivity)
                        for name, data in aim_cases()],
        "keyboard": [record(name, data, InputLabRequest, analyze_input_session)
                     for name, data in key_cases()],
    }, indent=2, allow_nan=False))
