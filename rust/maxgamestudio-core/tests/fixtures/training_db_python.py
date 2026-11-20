"""Read and write only the synthetic profile supplied by the Rust interoperability test."""

import asyncio
import json
import sqlite3
import sys
from datetime import UTC, datetime
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(Path(__file__).resolve().parents[4] / "backend"))

from app.input_lab import InputLabRequest, analyze_input_session
from app.sensitivity_lab import SensitivityRecommendationRequest, recommend_sensitivity
from app.training_db import TrainingDB


def layout(conn):
    output = {}
    for table, index in (
        ("sensitivity_sessions", "idx_sensitivity_sessions_created"),
        ("input_lab_sessions", "idx_input_lab_sessions_created"),
    ):
        sql, = conn.execute("SELECT sql FROM sqlite_schema WHERE name=?", (table,)).fetchone()
        output[table] = {
            "columns": conn.execute(f"PRAGMA table_info({table})").fetchall(),
            "autoincrement": "INTEGER PRIMARY KEY AUTOINCREMENT" in " ".join(sql.split()),
            "index": conn.execute(f"PRAGMA index_xinfo({index})").fetchall(),
        }
    return output


async def run(action, path):
    if action == "seed":
        if path.exists():
            raise FileExistsError("the synthetic profile path must be new")
    elif action in {"read", "layout"}:
        if not path.is_file():
            raise FileNotFoundError("the synthetic profile does not exist")
    else:
        raise ValueError(f"unknown test action: {action}")
    db = TrainingDB(path)
    if action == "layout":
        with sqlite3.connect(path) as conn:
            return layout(conn)
    if action == "seed":
        fixtures = json.loads((HERE / "training_input.json").read_text(encoding="utf-8"))
        request = SensitivityRecommendationRequest.model_validate(fixtures["sensitivity"][0]["input"])
        events = InputLabRequest.model_validate(fixtures["keyboard"][0]["input"])
        await db.init_tables()
        with sqlite3.connect(path) as conn:
            assert conn.execute("PRAGMA journal_mode=WAL").fetchone() == ("wal",)
            conn.execute("CREATE TABLE profile_canary(note TEXT)")
            conn.execute("INSERT INTO profile_canary VALUES ('keep profile')")
        await db.save_sensitivity_session(request.model_dump(mode="json"), recommend_sensitivity(request).model_dump(mode="json"))
        keyboard = analyze_input_session(events).model_dump(mode="json")
        keyboard["legacy_note"] = "旧记录"
        await db.save_input_session(events.model_dump(mode="json"), keyboard)
        with sqlite3.connect(path) as conn:
            stamp = "2024-02-03T04:05:06.123456+00:00"
            conn.execute("UPDATE sensitivity_sessions SET created_at=?, result_json='{}'", (stamp,))
            conn.execute("UPDATE input_lab_sessions SET created_at=?", (stamp,))

    output = {
        "sensitivity": await db.list_sensitivity_sessions(),
        "input": await db.list_input_sessions(),
    }
    with sqlite3.connect(path) as conn:
        output["layout"] = layout(conn)
        output["journal"], = conn.execute("PRAGMA journal_mode").fetchone()
        output["canary"], = conn.execute("SELECT note FROM profile_canary").fetchone()
        if action == "read":
            for table, prefix in (("sensitivity_sessions", "sensitivity"), ("input_lab_sessions", "input")):
                stamp, request, result = conn.execute(
                    f"SELECT created_at, request_json, result_json FROM {table} WHERE id=2"
                ).fetchone()
                assert datetime.fromisoformat(stamp).tzinfo == UTC
                assert stamp.endswith("+00:00")
                assert ("." not in stamp) or (len(stamp.split(".")[1].split("+")[0]) == 6)
                output[f"{prefix}_request"] = json.loads(request)
                output[f"{prefix}_result"] = json.loads(result)
    return output


if __name__ == "__main__":
    print(json.dumps(asyncio.run(run(sys.argv[1], Path(sys.argv[2]))), allow_nan=False))
