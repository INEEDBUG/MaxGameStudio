"""Record real deployed library handlers and their SQLite schema in a scratch directory."""

import ast
import asyncio
from contextlib import closing
import json
from pathlib import Path
import sqlite3
import tempfile
from typing import Optional
from urllib.parse import urlencode

from fastapi import FastAPI, HTTPException, Query
from fastapi.testclient import TestClient
from pydantic import BaseModel, Field


ROOT = Path(__file__).resolve().parents[4]
DEMO_TABLES = ("demo_files", "match_results", "demo_result_summaries", "demo_player_stats")
SEED = Path(__file__).with_name("demo_library_seed.sql").read_text(encoding="utf-8")

COMPACT = "/api/demos/compact"
IDS = "/api/demos/ids"
FULL = "/api/demos"
CASES = [
    (COMPACT, {}),
    (COMPACT, {"limit": 1, "offset": 1}),
    (COMPACT, {"limit": 3, "offset": 200, "q": "  "}),
    (COMPACT, {"q": "  50% Q'uote  "}),
    (COMPACT, {"q": "北区", "map_names": ", de_荒漠 ,,de_mirage ", "map_name": "de_dust2"}),
    (COMPACT, {"q": "%_", "player_query": "q'uote%_玩家"}),
    (COMPACT, {"q": "' OR 1=1 --"}),
    (COMPACT, {"statuses": "pending,other", "status": "done"}),
    (COMPACT, {"statuses": "parsing,loaded,error", "map_name": "de_mirage"}),
    (COMPACT, {"min_kills": 17, "max_deaths": 5, "min_assists": 7, "min_kd": 3.25}),
    (COMPACT, {"min_kills": 17, "max_deaths": 5, "min_assists": 7, "steam_query": "1:40", "player_query": "q'UOTE"}),
    (COMPACT, {"min_kills": 17, "max_deaths": 5, "steam_query": "7656119800", "player_query": "No Steam"}),
    (COMPACT, {"steam_query": "7777"}),
    (COMPACT, {"rounds_min": 15, "rounds_max": 21, "duration_min": "22.0", "duration_max": 25}),
    (COMPACT, {"date_from": "2025-03-18", "date_to": "2025-03-18"}),
    (COMPACT, {"date_from": "2025-03-18", "date_to": "2025-03-18T23:59:58Z"}),
    (COMPACT, {"date_to": "2025-02-30"}),
    (COMPACT, {"sort_key": "match_time", "sort_dir": "asc"}),
    (COMPACT, {"sort_key": "duration", "sort_dir": "desc", "limit": 2}),
    (COMPACT, {"sort_key": "size", "sort_dir": "asc", "limit": 2}),
    (COMPACT, {"sort_key": "date", "sort_dir": "desc", "limit": 1}),
    (COMPACT, {"sort_key": "id;DROP TABLE demo_files", "sort_dir": "asc", "limit": 1}),
    (COMPACT, [("limit", "2"), ("limit", "1.0"), ("offset", " 1 "), ("q", " NoDate ")]),
    (COMPACT, {"limit": 0, "offset": -1, "min_kills": "five", "min_kd": "abc", "sort_dir": "DESC"}),
    (COMPACT, {"limit": 1001, "rounds_max": -10, "sort_key": "x" * 33}),
    (COMPACT, {"steam_query": "界" * 65}),
    (IDS, {}),
    (IDS, {"min_kills": 17, "sort_key": "date", "sort_dir": "asc", "limit": 2, "offset": 1}),
    (IDS, {"steam_query": "7777", "q": "  alpha  "}),
    (FULL, {"statuses": "loaded,done"}),
    (FULL, {}),
    ("/api/demos/2", {}),
    ("/api/demos/5", {}),
    ("/api/demos/6", {}),
    ("/api/demos/999", {}),
    ("/api/demos/%32", {}),
    ("/api/demos/unknown-fixture", {}),
    ("/api/demos/1/player-stats", {}),
    ("/api/demos/3/player-stats", {}),
    ("/api/demos/99/player-stats", {}),
    ("/api/demos/discovered", {}),
    ("/api/demos/discovered", {"limit": 2, "offset": 1}),
    ("/api/demos/discovered", {"q": "  HUNT_  "}),
    ("/api/demos/discovered", {"q": "' OR 1=1 --"}),
]

CATALOG_EDITS = [
    ("PATCH", "/api/demos/1", {"display_name": "  新名称 ' %_  ", "ignored": True}),
    ("PATCH", "/api/demos/1/remark", {"remark": "  原样保留空白  \n"}),
    ("GET", "/api/demos/1", None),
    ("PATCH", "/api/demos/1", {"display_name": "  \t  "}),
    ("PATCH", "/api/demos/1/remark", {}),
    ("GET", "/api/demos/1", None),
    ("PATCH", "/api/demos/5", {"display_name": "Updated damaged result"}),
    ("PATCH", "/api/demos/999", {"display_name": "Missing"}),
    ("PATCH", "/api/demos/999/remark", {"remark": "Missing"}),
    ("PATCH", "/api/demos/1", {"display_name": None}),
    ("PATCH", "/api/demos/1/remark", {"remark": 2}),
    ("PATCH", "/api/demos/1", {"display_name": "界" * 513}),
    ("PATCH", "/api/demos/1/remark", []),
    ("POST", "/api/demos/watch-path/inspect", {"path": "", "max_depth": 33}),
    ("POST", "/api/demos/watch-path/inspect", {"max_depth": None}),
]


class CapturedHub:
    def __init__(self):
        self.reasons = []

    async def notify(self, reason):
        self.reasons.append(reason)


def record(db_source: str, routes_source: str):
    scope = {"__name__": "demo_db_fixture"}
    exec(compile(db_source, "demo_db.py", "exec"), scope)
    module = ast.parse(routes_source)
    names = {
        "_split_csv_query_param", "_demo_library_filters_from_query", "list_demos",
        "list_demos_compact_api", "list_demo_ids", "get_demo_library_item",
        "get_demo_player_stats_library", "list_discovered_demos", "DemoDisplayNamePatch",
        "DemoRemarkPatch", "DemoWatchPathInspectBody", "patch_demo_display_name",
        "patch_demo_remark", "inspect_demo_watch_path",
    }
    definitions = [
        node for node in module.body
        if getattr(node, "name", None) in names
        or isinstance(node, ast.Assign)
        and any(getattr(target, "id", None) == "_DEMO_LIBRARY_ALLOWED_STATUSES" for target in node.targets)
    ]
    fixture = {"cases": [], "catalog_edits": []}
    with tempfile.TemporaryDirectory(prefix="mgs-demo-library-fixture-") as scratch:
        path = Path(scratch) / "cs2-insight.db"
        demo = scope["DemoDB"](path)
        asyncio.run(demo.init_db())
        with closing(sqlite3.connect(path)) as conn:
            slots = ",".join("?" for _ in DEMO_TABLES)
            schema = conn.execute(
                f"SELECT sql FROM sqlite_schema WHERE tbl_name IN ({slots}) AND sql IS NOT NULL ORDER BY type DESC, rowid",
                DEMO_TABLES,
            )
            fixture["schema"] = [row[0] for row in schema]
            conn.executescript(SEED)
        app = FastAPI()
        hub = CapturedHub()
        routes = {**globals(), "app": app, "DemoListFilters": dict, "demo_db": demo, "demo_library_hub": hub}
        exec(compile(ast.Module(body=definitions, type_ignores=[]), "main.py", "exec"), routes)
        with TestClient(app, raise_server_exceptions=False) as client:
            for route, query in CASES:
                path = f"{route}?{urlencode(query)}" if query else route
                response = client.get(path)
                case = {"path": path, "status": response.status_code}
                if response.headers["content-type"].startswith("application/json"):
                    case["response"] = response.json()
                else:
                    case["text"] = response.text
                fixture["cases"].append(case)
            for method, path, body in CATALOG_EDITS:
                args = {"json": body} if body is not None else {}
                response = client.request(method, path, **args)
                fixture["catalog_edits"].append({
                    "method": method, "path": path, "body": body, "status": response.status_code,
                    "response": response.json(),
                })
        assert hub.reasons == ["display_name", "remark", "display_name", "remark", "display_name"]
    return fixture


if __name__ == "__main__":
    fixture = record(
        (ROOT / "backend/app/demo_db.py").read_text(encoding="utf-8"),
        (ROOT / "backend/app/main.py").read_text(encoding="utf-8"),
    )
    print(json.dumps(fixture, indent=2, ensure_ascii=True))
