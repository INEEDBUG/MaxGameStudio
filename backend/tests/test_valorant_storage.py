from __future__ import annotations

import asyncio
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from app.valorant_lab.models import CrosshairPresetPatch  # noqa: E402
from app.valorant_lab.storage import ValorantCrosshairDB  # noqa: E402


def test_crosshair_presets_round_trip_through_sqlite(tmp_path: Path):
    async def scenario():
        database = ValorantCrosshairDB(tmp_path / "training.db")
        await database.init_tables()
        created = await database.create_preset(
            name="Cyan one-tap",
            code="0;s;1;P;c;05;h;0;0l;4;0o;2;future;preserve;S;c;4;o;1",
            tags=["aim", "pro"],
            source="manual",
        )
        loaded = await database.get_preset(created.id)
        rows = await database.list_presets(tag="aim")
        return created, loaded, rows

    created, loaded, rows = asyncio.run(scenario())

    assert created.id == 1
    assert loaded is not None
    assert loaded.code == "0;s;1;P;c;5;h;0;0l;4;0o;2;future;preserve;S;c;4;o;1"
    assert loaded.tags == ["aim", "pro"]
    assert len(rows) == 1
    assert rows[0].name == "Cyan one-tap"


def test_crosshair_preset_patch_revalidates_code_and_delete(tmp_path: Path):
    async def scenario():
        database = ValorantCrosshairDB(tmp_path / "training.db")
        await database.init_tables()
        created = await database.create_preset(name="Original", code="0;P;c;1")
        updated = await database.update_preset(
            created.id,
            CrosshairPresetPatch(code="0;P;c;8;u;00ff00", name="Updated"),
        )
        deleted = await database.delete_preset(created.id)
        return updated, deleted, await database.get_preset(created.id)

    updated, deleted, missing = asyncio.run(scenario())

    assert updated is not None
    assert updated.name == "Updated"
    assert updated.code == "0;P;c;8;u;00FF00"
    assert deleted is True
    assert missing is None
