"""Local SQLite persistence for VALORANT crosshair presets.

The database stores the normalized native code as text and keeps descriptive
metadata in JSON, matching the project's existing local-lab storage pattern.
No game files, process state or Riot/VALORANT endpoints are accessed here.
"""

from __future__ import annotations

import json
import sqlite3
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, Mapping

try:
    import aiosqlite
except ModuleNotFoundError:  # pragma: no cover - exercised only in minimal tooling runtimes
    class _CompatCursor:
        def __init__(self, cursor: sqlite3.Cursor) -> None:
            self._cursor = cursor

        @property
        def lastrowid(self) -> int | None:
            return self._cursor.lastrowid

        @property
        def rowcount(self) -> int:
            return self._cursor.rowcount

        async def fetchone(self):
            return self._cursor.fetchone()

        async def fetchall(self):
            return self._cursor.fetchall()

    class _CompatConnection:
        def __init__(self, path: str | Path) -> None:
            self._connection = sqlite3.connect(path)

        @property
        def row_factory(self):
            return self._connection.row_factory

        @row_factory.setter
        def row_factory(self, value) -> None:
            self._connection.row_factory = value

        async def __aenter__(self) -> "_CompatConnection":
            return self

        async def __aexit__(self, exc_type, exc, traceback) -> None:
            if exc_type is not None:
                self._connection.rollback()
            self._connection.close()

        async def execute(self, sql: str, parameters=()) -> _CompatCursor:
            return _CompatCursor(self._connection.execute(sql, parameters))

        async def commit(self) -> None:
            self._connection.commit()

    class _CompatAioSqlite:
        Row = sqlite3.Row

        @staticmethod
        def connect(path: str | Path) -> _CompatConnection:
            return _CompatConnection(path)

    aiosqlite = _CompatAioSqlite()

try:
    from ..demo_db import utc_now_iso
except ModuleNotFoundError:  # pragma: no cover - only for aiosqlite-free tooling runtimes
    def utc_now_iso() -> str:
        return datetime.now(UTC).isoformat(timespec="seconds")
from .models import CrosshairPreset, CrosshairPresetCreate, CrosshairPresetPatch


def _limit(value: int, *, default: int = 100, maximum: int = 500) -> int:
    try:
        return max(1, min(int(value), maximum))
    except (TypeError, ValueError):
        return default


def _json_tags(value: Any) -> list[str]:
    if isinstance(value, list):
        return [str(item) for item in value]
    if isinstance(value, str):
        try:
            parsed = json.loads(value)
        except (TypeError, ValueError):
            return []
        return [str(item) for item in parsed] if isinstance(parsed, list) else []
    return []


class ValorantCrosshairDB:
    """Async SQLite store for named crosshair presets."""

    def __init__(self, db_path: str | Path) -> None:
        self.db_path = Path(db_path)

    async def init_tables(self) -> None:
        self.db_path.parent.mkdir(parents=True, exist_ok=True)
        async with aiosqlite.connect(self.db_path) as conn:
            await conn.execute(
                """
                CREATE TABLE IF NOT EXISTS valorant_crosshair_presets (
                    id          INTEGER PRIMARY KEY AUTOINCREMENT,
                    name        TEXT NOT NULL,
                    code        TEXT NOT NULL,
                    description TEXT NOT NULL DEFAULT '',
                    tags_json   TEXT NOT NULL DEFAULT '[]',
                    source      TEXT,
                    created_at  TEXT NOT NULL,
                    updated_at  TEXT NOT NULL
                )
                """,
            )
            await conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_valorant_crosshair_updated "
                "ON valorant_crosshair_presets(updated_at DESC, id DESC)",
            )
            await conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_valorant_crosshair_name "
                "ON valorant_crosshair_presets(name COLLATE NOCASE)",
            )
            await conn.commit()

    async def create_preset(
        self,
        preset: CrosshairPreset | CrosshairPresetCreate | Mapping[str, Any] | None = None,
        *,
        name: str | None = None,
        code: str | None = None,
        description: str = "",
        tags: list[str] | None = None,
        source: str | None = None,
    ) -> CrosshairPreset:
        """Validate and insert a preset, returning its assigned integer id."""

        value = self._coerce_create(
            preset,
            name=name,
            code=code,
            description=description,
            tags=tags,
            source=source,
        )
        now = utc_now_iso()
        async with aiosqlite.connect(self.db_path) as conn:
            cursor = await conn.execute(
                """
                INSERT INTO valorant_crosshair_presets(
                    name, code, description, tags_json, source, created_at, updated_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    value.name,
                    value.code,
                    value.description,
                    json.dumps(value.tags, ensure_ascii=False, separators=(",", ":")),
                    value.source,
                    now,
                    now,
                ),
            )
            await conn.commit()
            preset_id = int(cursor.lastrowid)
        return value.model_copy(update={"id": preset_id, "created_at": now, "updated_at": now})

    async def save_preset(self, preset: CrosshairPreset | CrosshairPresetCreate | Mapping[str, Any]) -> CrosshairPreset:
        """Insert a preset or update it when an existing id is provided."""

        value = self._coerce_preset(preset)
        if value.id is None:
            return await self.create_preset(value)
        updated = await self.update_preset(value.id, value)
        if updated is None:
            return await self.create_preset(value.model_copy(update={"id": None}))
        return updated

    async def get_preset(self, preset_id: int) -> CrosshairPreset | None:
        async with aiosqlite.connect(self.db_path) as conn:
            conn.row_factory = aiosqlite.Row
            row = await (
                await conn.execute(
                    """
                    SELECT id, name, code, description, tags_json, source, created_at, updated_at
                    FROM valorant_crosshair_presets WHERE id = ?
                    """,
                    (int(preset_id),),
                )
            ).fetchone()
        return None if row is None else self._row_to_preset(row)

    async def get_by_name(self, name: str) -> CrosshairPreset | None:
        async with aiosqlite.connect(self.db_path) as conn:
            conn.row_factory = aiosqlite.Row
            row = await (
                await conn.execute(
                    """
                    SELECT id, name, code, description, tags_json, source, created_at, updated_at
                    FROM valorant_crosshair_presets
                    WHERE name = ? COLLATE NOCASE
                    ORDER BY id ASC LIMIT 1
                    """,
                    (str(name).strip(),),
                )
            ).fetchone()
        return None if row is None else self._row_to_preset(row)

    async def list_presets(
        self,
        *,
        limit: int = 100,
        offset: int = 0,
        search: str | None = None,
        tag: str | None = None,
    ) -> list[CrosshairPreset]:
        clauses: list[str] = []
        params: list[Any] = []
        if search and search.strip():
            clauses.append("(name LIKE ? OR description LIKE ? OR code LIKE ?)")
            needle = f"%{search.strip()}%"
            params.extend([needle, needle, needle])
        if tag and tag.strip():
            # JSON arrays are stored as text; the separators used above make a
            # quoted token boundary safe for this local filter.
            clauses.append("tags_json LIKE ?")
            params.append(f'%"{tag.strip()}"%')
        where = f"WHERE {' AND '.join(clauses)}" if clauses else ""
        params.extend([_limit(limit), max(0, int(offset))])
        async with aiosqlite.connect(self.db_path) as conn:
            conn.row_factory = aiosqlite.Row
            rows = await (
                await conn.execute(
                    f"""
                    SELECT id, name, code, description, tags_json, source, created_at, updated_at
                    FROM valorant_crosshair_presets
                    {where}
                    ORDER BY updated_at DESC, id DESC
                    LIMIT ? OFFSET ?
                    """,
                    params,
                )
            ).fetchall()
        return [self._row_to_preset(row) for row in rows]

    async def update_preset(
        self,
        preset_id: int,
        patch: CrosshairPreset | CrosshairPresetPatch | Mapping[str, Any] | None = None,
        **changes: Any,
    ) -> CrosshairPreset | None:
        """Patch a preset and return the updated row, or ``None`` if absent."""

        existing = await self.get_preset(preset_id)
        if existing is None:
            return None
        if patch is None:
            patch = changes
        elif changes:
            if not isinstance(patch, Mapping):
                raise TypeError("keyword changes can only be combined with a mapping patch")
            patch = {**patch, **changes}
        if isinstance(patch, CrosshairPreset):
            value = patch
        else:
            patch_model = patch if isinstance(patch, CrosshairPresetPatch) else CrosshairPresetPatch.model_validate(patch)
            values = existing.model_dump()
            values.update(patch_model.model_dump(exclude_unset=True))
            values["id"] = int(preset_id)
            value = CrosshairPreset.model_validate(values)
        now = utc_now_iso()
        async with aiosqlite.connect(self.db_path) as conn:
            cursor = await conn.execute(
                """
                UPDATE valorant_crosshair_presets
                SET name = ?, code = ?, description = ?, tags_json = ?, source = ?, updated_at = ?
                WHERE id = ?
                """,
                (
                    value.name,
                    value.code,
                    value.description,
                    json.dumps(value.tags, ensure_ascii=False, separators=(",", ":")),
                    value.source,
                    now,
                    int(preset_id),
                ),
            )
            await conn.commit()
        if cursor.rowcount <= 0:
            return None
        return value.model_copy(
            update={
                "id": int(preset_id),
                "created_at": existing.created_at or now,
                "updated_at": now,
            },
        )

    async def delete_preset(self, preset_id: int) -> bool:
        async with aiosqlite.connect(self.db_path) as conn:
            cursor = await conn.execute(
                "DELETE FROM valorant_crosshair_presets WHERE id = ?",
                (int(preset_id),),
            )
            await conn.commit()
        return cursor.rowcount > 0

    # Common storage naming aliases.
    list = list_presets
    get = get_preset
    delete = delete_preset

    @staticmethod
    def _coerce_create(
        preset: CrosshairPreset | CrosshairPresetCreate | Mapping[str, Any] | None,
        *,
        name: str | None,
        code: str | None,
        description: str,
        tags: list[str] | None,
        source: str | None,
    ) -> CrosshairPresetCreate:
        if preset is not None:
            if isinstance(preset, CrosshairPreset):
                return CrosshairPresetCreate.model_validate(preset.model_dump(exclude={"id", "created_at", "updated_at"}))
            if isinstance(preset, CrosshairPresetCreate):
                return preset
            return CrosshairPresetCreate.model_validate(preset)
        if name is None or code is None:
            raise ValueError("create_preset requires name and code")
        return CrosshairPresetCreate(
            name=name,
            code=code,
            description=description,
            tags=tags or [],
            source=source,
        )

    @staticmethod
    def _coerce_preset(value: CrosshairPreset | CrosshairPresetCreate | Mapping[str, Any]) -> CrosshairPreset:
        if isinstance(value, CrosshairPreset):
            return value
        if isinstance(value, CrosshairPresetCreate):
            return CrosshairPreset.model_validate(value.model_dump())
        return CrosshairPreset.model_validate(value)

    @staticmethod
    def _row_to_preset(row: aiosqlite.Row) -> CrosshairPreset:
        return CrosshairPreset(
            id=int(row["id"]),
            name=str(row["name"]),
            code=str(row["code"]),
            description=str(row["description"] or ""),
            tags=_json_tags(row["tags_json"]),
            source=None if row["source"] is None else str(row["source"]),
            created_at=str(row["created_at"]),
            updated_at=str(row["updated_at"]),
        )


# Aliases matching the names used by the other local-lab stores.
ValorantPresetStore = ValorantCrosshairDB
CrosshairPresetDB = ValorantCrosshairDB
ValorantCrosshairStore = ValorantCrosshairDB
CrosshairStore = ValorantCrosshairDB


__all__ = [
    "ValorantCrosshairDB",
    "ValorantPresetStore",
    "CrosshairPresetDB",
    "ValorantCrosshairStore",
    "CrosshairStore",
]
