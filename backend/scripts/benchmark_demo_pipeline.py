#!/usr/bin/env python3
"""Minimal benchmark for Demo inspection, full analysis, and cached-result load."""

from __future__ import annotations

import argparse
import asyncio
import ctypes
from ctypes import wintypes
import gc
import hashlib
import json
from pathlib import Path
import sys
import time
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from app.demo_db import DemoDB
from app.parse_worker import _run


class _ProcessMemoryCountersEx(ctypes.Structure):
    _fields_ = [
        ("cb", wintypes.DWORD),
        ("PageFaultCount", wintypes.DWORD),
        ("PeakWorkingSetSize", ctypes.c_size_t),
        ("WorkingSetSize", ctypes.c_size_t),
        ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
        ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
        ("PagefileUsage", ctypes.c_size_t),
        ("PeakPagefileUsage", ctypes.c_size_t),
        ("PrivateUsage", ctypes.c_size_t),
    ]


def _memory() -> dict[str, float]:
    if sys.platform != "win32":
        return {}
    counters = _ProcessMemoryCountersEx()
    counters.cb = ctypes.sizeof(counters)
    get_process = ctypes.windll.kernel32.GetCurrentProcess
    get_process.restype = wintypes.HANDLE
    get_memory = ctypes.windll.psapi.GetProcessMemoryInfo
    get_memory.argtypes = [wintypes.HANDLE, ctypes.c_void_p, wintypes.DWORD]
    get_memory.restype = wintypes.BOOL
    if not get_memory(get_process(), ctypes.byref(counters), counters.cb):
        return {}
    scale = 1024 * 1024
    return {
        "working_set_mib": round(counters.WorkingSetSize / scale, 2),
        "peak_working_set_mib": round(counters.PeakWorkingSetSize / scale, 2),
        "private_mib": round(counters.PrivateUsage / scale, 2),
    }


def _encoded_summary(result: Any) -> tuple[bytes, dict[str, Any]]:
    encoded = json.dumps(
        result,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    workspace = result.get("__analysis_workspace__") if isinstance(result, dict) else None
    if isinstance(result, dict) and isinstance(result.get("players"), list):
        player_count = len(result["players"])
    else:
        player_count = len({
            key: value
            for key, value in result.items()
            if isinstance(result, dict) and key != "__analysis_workspace__" and isinstance(value, dict)
        })
    return encoded, {
        "players": player_count,
        "rounds": len(workspace.get("rounds") or []) if isinstance(workspace, dict) else 0,
        "output_bytes": len(encoded),
        "output_sha256": hashlib.sha256(encoded).hexdigest(),
    }


async def _load_cached(db_path: Path, demo_path: str) -> Any:
    return await DemoDB(db_path).get_result(demo_path)


def main() -> int:
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("--demo", required=True)
    cli.add_argument(
        "--mode",
        choices=("inspect", "inspect-fast", "full", "cached-result"),
        required=True,
    )
    cli.add_argument("--players", nargs="*")
    cli.add_argument("--db")
    args = cli.parse_args()

    demo = Path(args.demo).resolve()
    if not demo.is_file():
        raise FileNotFoundError(demo)
    if args.mode == "full" and not args.players:
        cli.error("--players is required for --mode full")
    if args.mode == "cached-result" and not args.db:
        cli.error("--db is required for --mode cached-result")

    before = _memory()
    started = time.perf_counter()
    if args.mode == "inspect":
        result = _run({"action": "inspect", "dem_path": str(demo)})
    elif args.mode == "inspect-fast":
        result = _run({"action": "inspect_fast", "dem_path": str(demo)})
    elif args.mode == "cached-result":
        result = asyncio.run(_load_cached(Path(args.db), str(demo)))
    else:
        result = _run({
            "action": "analyze_batch",
            "dem_path": str(demo),
            "target_players": list(args.players),
            "freeze_to_death_rounds": None,
        })
    operation_seconds = time.perf_counter() - started

    encode_started = time.perf_counter()
    encoded, summary = _encoded_summary(result)
    encode_seconds = time.perf_counter() - encode_started
    after = _memory()
    del encoded, result
    gc.collect()
    time.sleep(0.25)
    retained = _memory()

    print(json.dumps({
        "mode": args.mode,
        "demo_bytes": demo.stat().st_size,
        "operation_seconds": round(operation_seconds, 6),
        "json_encode_seconds": round(encode_seconds, 6),
        "full_seconds": round(operation_seconds + encode_seconds, 6),
        "memory_before": before,
        "memory_after": after,
        "memory_retained_after_gc": retained,
        **summary,
    }, ensure_ascii=False, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
