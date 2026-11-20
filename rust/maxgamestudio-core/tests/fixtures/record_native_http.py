"""Record the existing HTTP handlers without starting the application or opening a profile."""

import ast
import json
from pathlib import Path
import sys
from typing import Optional

ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "backend"))

from fastapi import Body, FastAPI
from fastapi.middleware.cors import CORSMiddleware
from fastapi.testclient import TestClient

from app.gsi_ready import gsi_status, notify_gsi_payload, reset_gsi_ready


VERSION = "3.2.0-parity"
TRUSTED_ORIGIN = "http://tauri.localhost"
OTHER_ORIGIN = "https://example.invalid"


def original_client():
    source = ROOT / "backend/app/main.py"
    tree = ast.parse(source.read_text(encoding="utf-8"))
    handlers = {"health", "cs2_gsi", "cs2_gsi_status"}
    definitions = [
        node for node in tree.body if (
            isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name in handlers
        ) or (
            isinstance(node, ast.Expr)
            and isinstance(node.value, ast.Call)
            and ast.unparse(node.value.func) == "app.add_middleware"
            and ast.unparse(node.value.args[0]) == "CORSMiddleware"
        )
    ]
    assert len(definitions) == 4
    app = FastAPI()
    exec(compile(ast.Module(body=definitions, type_ignores=[]), source, "exec"), {
        "app": app, "APP_VERSION": VERSION, "Optional": Optional, "Body": Body,
        "CORSMiddleware": CORSMiddleware,
        "gsi_status": gsi_status, "notify_gsi_payload": notify_gsi_payload,
    })
    return TestClient(app, follow_redirects=False)


def capture(client, name, method, path, headers=None, body=None):
    response = client.request(method, path, headers=headers or {}, content=body)
    result = {
        "name": name, "method": method, "path": path, "headers": headers or {},
        "body": body, "status": response.status_code,
        "response_headers": {
            key: value for key, value in response.headers.items()
            if key in {"allow", "vary", "content-type"} or key.startswith("access-control-")
        },
    }
    if response.content and response.headers.get("content-type") == "application/json":
        value = response.json()
        if isinstance(value, dict):
            for key in ("last_payload_at", "last_ready_at"):
                if value.get(key, 0) > 0:
                    value[key] = "<positive monotonic time>"
        result["response_json"] = value
    else:
        result["response_text"] = response.text
    return result


JSON = {"Content-Type": "application/json"}
PREFLIGHT = {"Origin": TRUSTED_ORIGIN, "Access-Control-Request-Method": "POST"}
HEALTH = "/api/health"
SINK = "/api/gsi/cs2"
STATUS = "/api/gsi/status"

CASES = [
    ("health is public", "GET", HEALTH),
    ("a desktop token does not gate health", "GET", HEALTH, {"X-CS2-Insight-Token": "synthetic-token"}),
    ("empty readiness", "GET", STATUS),
    ("empty guest GSI", "POST", SINK),
    ("explicit null is allowed", "POST", SINK, JSON, "null"),
    ("ready guest GSI", "POST", SINK, JSON, '{"map":{"name":"fixture_map"},"player":{"name":"玩家"}}'),
    ("guest status observes ready payload", "GET", STATUS),
    ("loading payload does not reset overall readiness", "POST", SINK, JSON, '{"player":{"activity":"loading"}}'),
    ("status keeps the previous ready timestamp", "GET", STATUS),
    ("invalid object payload", "POST", SINK, JSON, "[]"),
    ("invalid boolean payload", "POST", SINK, JSON, "true"),
    ("invalid string payload", "POST", SINK, JSON, '"text"'),
    ("malformed JSON", "POST", SINK, JSON, '{"map":'),
    ("JSON followed by junk", "POST", SINK, JSON, '{}more'),
    ("body without content type", "POST", SINK, {}, '{}'),
    ("plain text body", "POST", SINK, {"Content-Type": "text/plain"}, '{"map": {"round": 0}}'),
    ("invalid requests leave stored readiness unchanged", "GET", STATUS),
    ("vendor JSON type", "POST", SINK, {"Content-Type": "application/vnd.fixture+json; charset=UTF-8"}, "{}"),
    ("health HEAD", "HEAD", HEALTH),
    ("incorrect method has Allow", "PUT", SINK),
    ("unknown route does not claim success", "GET", "/api/demos"),
    ("desktop response allows its own origin", "GET", HEALTH, {"Origin": TRUSTED_ORIGIN}),
    ("https desktop origin", "GET", HEALTH, {"Origin": "https://tauri.localhost"}),
    ("tauri protocol origin", "GET", HEALTH, {"Origin": "tauri://localhost"}),
    ("Vite localhost origin", "GET", HEALTH, {"Origin": "http://localhost:5173"}),
    ("Vite numeric loopback origin", "GET", HEALTH, {"Origin": "http://127.0.0.1:5173"}),
    ("unknown origin is not readable by the browser", "GET", HEALTH, {"Origin": OTHER_ORIGIN}),
    ("allowed preflight mirrors requested headers", "OPTIONS", SINK, {**PREFLIGHT, "Access-Control-Request-Headers": "Content-Type, X-CS2-Insight-Token"}),
    ("disallowed preflight origin", "OPTIONS", SINK, {**PREFLIGHT, "Origin": OTHER_ORIGIN}),
    ("disallowed preflight method", "OPTIONS", SINK, {**PREFLIGHT, "Access-Control-Request-Method": "TRACE"}),
    ("disallowed private network preflight", "OPTIONS", SINK, {**PREFLIGHT, "Access-Control-Request-Private-Network": "true"}),
    ("ordinary options has no preflight", "OPTIONS", SINK, {"Origin": TRUSTED_ORIGIN}),
]


if __name__ == "__main__":
    reset_gsi_ready()
    client = original_client()
    print(json.dumps({"version": VERSION, "cases": [capture(client, *case) for case in CASES]}, indent=2))
