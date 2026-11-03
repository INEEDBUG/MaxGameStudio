"""Process-wide ownership guard for CS2/OBS mutating operations."""

from __future__ import annotations

from datetime import datetime, timezone
import threading
from typing import AsyncIterator
import uuid

from fastapi import HTTPException, Request


_claim_lock = threading.Lock()
_owner: dict[str, str] | None = None


def claim_runtime_session(operation: str) -> str:
    """Acquire across a background job lifetime, not just its starting request."""
    global _owner
    claim = {
        "id": uuid.uuid4().hex,
        "operation": operation,
        "started_at": datetime.now(timezone.utc).isoformat(),
    }
    with _claim_lock:
        if _owner is not None:
            raise HTTPException(
                status_code=409,
                detail={
                    "code": "RUNTIME_SESSION_BUSY",
                    "message": "Another CS2/OBS operation is already running.",
                    "owner": dict(_owner),
                },
            )
        _owner = claim
    return claim["id"]


def release_runtime_session(claim_id: str) -> None:
    global _owner
    with _claim_lock:
        if _owner is not None and _owner.get("id") == claim_id:
            _owner = None


async def runtime_session_dependency(request: Request) -> AsyncIterator[None]:
    claim_id = claim_runtime_session(request.url.path)
    try:
        yield
    finally:
        release_runtime_session(claim_id)


def runtime_session_state() -> dict[str, object]:
    with _claim_lock:
        return {"busy": _owner is not None, "owner": dict(_owner) if _owner else None}
