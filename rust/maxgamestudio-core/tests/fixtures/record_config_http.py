"""Record the deployed config handlers against a disposable profile."""

import ast
import json
from pathlib import Path
import shutil
import sys
import tempfile
from typing import Any, Literal, Optional

ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "backend"))

from fastapi import FastAPI
from fastapi.testclient import TestClient
from pydantic import BaseModel, Field

from app import env_utils
from app.env_utils import (
    LLMConfig, OBSConfig, llm_api_key_configured, llm_base_url_is_local_host, load_config, save_config,
)


LEGACY = {
    "obs": {
        "host": "127.0.0.1", "port": "4456", "password": "fixture-only-OBS",
        "obs_config_verified": True, "unrecognized": {"remember": "OBS field"},
    },
    "llm": {
        "model": "fixture-model", "base_url": " http://localhost:9412/v1/chat/completions/ ",
        "api_key": "fixture-only-LLM", "unrecognized": ["LLM field"],
    },
    "cs2_path": "fixture-missing-cs2.exe",
    "ffmpeg_path": "fixture-missing-ffmpeg.exe",
    "locale": "zh-HK",
    "close_to_tray": False,
    "cs2_extra_launch_args": "-novid",
    "record_inject_console_lines": "",
    "kb_overlay_tick_offset": 8,
    "kill_fx_tick_offset": -3,
    "steam_api_key": "fixture-only-STEAM",
    "steam_known_share_code": "CSGO-fixtu-reonl-yfict-ional-AAAAA",
    "steam_match_share_codes": ["CSGO-fixtu-reonl-yfict-ional-BBBBB"],
    "future_setting": {"nested": ["fixture", 1, None]},
}

CASES = [
    ("get", "/api/config", None),
    ("get", "/api/config/quick-check", None),
    ("put", "/api/config", {
        "obs": {"port": "4460", "password": " ****match ", "obs_path": " fixture-obs.exe "},
        "llm": {"model": "updated", "api_key": "****match", "base_url": "http://host/v1/completions"},
        "demo_watch_paths": ["Archive/./Demos/", "  "], "demo_watch_scan_depth": "3.0",
        "close_to_tray": True, "close_action": "exit", "ai_mode": "on",
        "expected_parse_players": ["  Alice", "Bob ", "Alice", " "],
        "recording_global_pacing": {"startDelaySeconds": 4.5},
        "cs2_extra_launch_args": "", "record_inject_console_lines": "",
        "obs_transition_name": "  ", "obs_transition_duration_ms": -1,
        "experimental": {"pov_enabled": True},
        "steam_api_key": "****match", "steam_known_share_code": "CSGO-****-****-****-****-AAAAA",
        "match_mode": "bad-mode", "match_count": 50,
        "update_check_frequency": " MONTHLY ", "last_update_check_at": " 2026-01-01T00:00:00Z ",
        "not_a_saved_field": True,
    }),
    ("get", "/api/config", None),
    ("put", "/api/config", {
        "obs": {"host": None, "port": "invalid"},
        "demo_watch_paths": ["valid", 3], "demo_watch_scan_depth": 33,
        "ai_mode": "invalid", "close_action": "invalid", "kb_overlay_tick_offset": 1.5,
    }),
    ("put", "/api/config", {"steam_known_share_code": " CSGO-fixtu-reonl-yfict-ional-CCCCC "}),
    ("get", "/api/config", None),
]


def record():
    source = ROOT / "backend/app/main.py"
    tree = ast.parse(source.read_text(encoding="utf-8"))
    selected = {"ConfigPayload", "ExperimentalPayload", "get_config", "update_config", "config_quick_check"}
    definitions = [node for node in tree.body if getattr(node, "name", None) in selected]
    fixture = {"old": LEGACY, "cases": []}
    with tempfile.TemporaryDirectory(prefix="mgs-config-fixture-") as scratch:
        scratch = Path(scratch)
        path = scratch / "cs2-insight.config.json"
        path.write_text(json.dumps(LEGACY), encoding="utf-8-sig")
        example = scratch / "cs2-insight.config.example.json"
        shutil.copyfile(ROOT / "data" / example.name, example)
        env_utils.resolve_config_path = lambda: path
        env_utils.resolve_example_config_path = lambda: example
        env_utils._LEGACY_CONFIG_PATH = scratch / "old-backend.json"
        app = FastAPI()
        scope = {**globals(), "app": app, "demo_watcher": None, "ensure_cs2_path": lambda cfg: cfg}
        exec(compile(ast.Module(body=definitions, type_ignores=[]), str(source), "exec"), scope)
        with TestClient(app) as client:
            for method, route, body in CASES:
                args = {"json": body} if body is not None else {}
                response = client.request(method, route, **args)
                fixture["cases"].append({
                    "method": method.upper(), "path": route, "body": body,
                    "status": response.status_code, "response": response.json(),
                })
            fixture["saved"] = json.loads(path.read_text(encoding="utf-8"))
            path.unlink()
            fixture["first_load"] = client.get("/api/config").json()
    return fixture


if __name__ == "__main__":
    print(json.dumps(record(), ensure_ascii=True, indent=2))
