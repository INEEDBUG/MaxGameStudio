"""Return a real replay packet from the pinned wheel for a disposable fixture."""

from importlib import metadata
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "backend"))

from app.parser.replay_match_cache import load_match_replay_round_binary


if __name__ == "__main__":
    profile, demo = map(Path, sys.argv[1:])
    os.environ["CS2_INSIGHT_DATA_DIR"] = str(profile)
    assert metadata.version("demoparser2") == "0.41.4+cs2insight7"
    packet = load_match_replay_round_binary(
        str(demo), start_tick=100, end_tick=112, fps=32.0, tick_rate=64.0,
    )
    assert packet is not None, "synthetic cache key or format differs from the shipped backend"
    sys.stdout.buffer.write(packet)
