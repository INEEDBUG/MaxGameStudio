from __future__ import annotations

import json
from dataclasses import asdict
from types import SimpleNamespace
from unittest.mock import patch

from app.valorant_lab import display_session


def _mode(width: int = 1920, height: int = 1080, refresh_hz: int = 240) -> display_session.DisplayMode:
    return display_session.DisplayMode(
        width=width,
        height=height,
        refresh_hz=refresh_hz,
        bits_per_pixel=32,
    )


class _FakeTimer:
    instances: list["_FakeTimer"] = []

    def __init__(self, interval, function):
        self.interval = interval
        self.function = function
        self.daemon = False
        self.cancelled = False
        self.started = False
        self.instances.append(self)

    def start(self):
        self.started = True

    def cancel(self):
        self.cancelled = True

    def fire(self):
        self.function()


def _fake_devmode() -> SimpleNamespace:
    # _requested_devmode only needs these fields; a SimpleNamespace keeps
    # tests independent of the host Win32 ABI and never calls user32.
    return SimpleNamespace(
        dmFields=0,
        dmPelsWidth=1920,
        dmPelsHeight=1080,
        dmDisplayFrequency=240,
        dmBitsPerPel=32,
    )


def test_test_display_mode_only_uses_cds_test_and_never_applies_mode():
    calls: list[int] = []
    with (
        patch.object(display_session, "_read_mode", return_value=_fake_devmode()),
        patch.object(display_session, "_change_mode", side_effect=lambda mode, flags: calls.append(flags) or 0),
    ):
        result = display_session.test_display_mode(1568, 1080, 240)

    assert result["accepted"] is True
    assert calls == [display_session.CDS_TEST]


def test_rejected_cds_test_does_not_fall_through_to_a_real_change():
    calls: list[int] = []
    with (
        patch.object(display_session, "_read_mode", return_value=_fake_devmode()),
        patch.object(display_session, "_change_mode", side_effect=lambda mode, flags: calls.append(flags) or -2),
    ):
        result = display_session.test_display_mode(1568, 1080, 239)

    assert result["accepted"] is False
    assert result["result"] == "bad_mode"
    assert calls == [display_session.CDS_TEST]


def test_native_test_exception_is_reported_without_propagating():
    with (
        patch.object(display_session, "_read_mode", return_value=_fake_devmode()),
        patch.object(display_session, "_change_mode", side_effect=OSError("native test unavailable")),
    ):
        result = display_session.test_display_mode(1568, 1080, 240)

    assert result["accepted"] is False
    assert result["result"] == "display_test_failed"
    assert "native test unavailable" in result["error"]


def test_apply_requires_driver_test_then_arms_automatic_rollback():
    _FakeTimer.instances = []
    previous = _mode()
    applied = _mode(1568, 1080, 240)
    changes: list[int] = []
    session = display_session.DisplayModeSession()
    with (
        patch.object(display_session, "test_display_mode", return_value={"accepted": True, "code": 0}),
        patch.object(display_session, "current_display_mode", side_effect=[previous, applied]),
        patch.object(display_session, "_read_mode", return_value=_fake_devmode()),
        patch.object(display_session, "_change_mode", side_effect=lambda mode, flags: changes.append(flags) or 0),
        patch.object(display_session.threading, "Timer", _FakeTimer),
    ):
        result = session.apply(1568, 1080, 240, 20)

        assert result["applied"] is True
        assert result["persistent"] is False
        assert session.status()["active"] is True
        assert len(_FakeTimer.instances) == 1
        assert _FakeTimer.instances[0].started is True

        # The timer callback is the only path that changes back to the
        # captured mode.  It must also clear the active session.
        _FakeTimer.instances[0].fire()

    assert changes == [0, 0]
    assert session.status()["active"] is False
    assert session.status()["previous"] is None


def test_apply_rejection_does_not_change_mode_or_create_rollback_timer():
    _FakeTimer.instances = []
    changes: list[int] = []
    session = display_session.DisplayModeSession()
    with (
        patch.object(display_session, "test_display_mode", return_value={"accepted": False, "code": -2}),
        patch.object(display_session, "_change_mode", side_effect=lambda mode, flags: changes.append(flags) or 0),
        patch.object(display_session.threading, "Timer", _FakeTimer),
    ):
        result = session.apply(1568, 1080, 239, 20)

    assert result["applied"] is False
    assert changes == []
    assert _FakeTimer.instances == []
    assert session.status()["active"] is False


def test_confirm_cancels_rollback_and_keeps_the_tested_mode():
    _FakeTimer.instances = []
    previous = _mode()
    applied = _mode(1568, 1080, 240)
    changes: list[int] = []
    session = display_session.DisplayModeSession()
    with (
        patch.object(display_session, "test_display_mode", return_value={"accepted": True, "code": 0}),
        patch.object(display_session, "current_display_mode", side_effect=[previous, applied, applied]),
        patch.object(display_session, "_read_mode", return_value=_fake_devmode()),
        patch.object(display_session, "_change_mode", side_effect=lambda mode, flags: changes.append(flags) or 0),
        patch.object(display_session.threading, "Timer", _FakeTimer),
    ):
        session.apply(1568, 1080, 240, 20)
        result = session.confirm()

    assert result["confirmed"] is True
    assert result["current"] == asdict(applied)
    assert _FakeTimer.instances[0].cancelled is True
    assert changes == [0]
    assert session.status()["active"] is False


def test_restore_failure_keeps_previous_mode_for_a_user_retry():
    _FakeTimer.instances = []
    previous = _mode()
    applied = _mode(1568, 1080, 240)
    changes = iter([0, -3, 0])
    session = display_session.DisplayModeSession()
    with (
        patch.object(display_session, "test_display_mode", return_value={"accepted": True, "code": 0}),
        patch.object(display_session, "current_display_mode", side_effect=[previous, applied]),
        patch.object(display_session, "_read_mode", return_value=_fake_devmode()),
        patch.object(display_session, "_change_mode", side_effect=lambda mode, flags: next(changes)),
        patch.object(display_session.threading, "Timer", _FakeTimer),
    ):
        session.apply(1568, 1080, 240, 20)
        failed = session.restore()
        active_after_failed = session.status()["active"]
        retried = session.restore()

    assert failed["restored"] is False
    assert failed["retry_available"] is True
    assert active_after_failed is True
    assert retried["restored"] is True
    assert session.status()["active"] is False


def test_manifest_survives_native_change_exception_for_startup_recovery(tmp_path):
    manifest = tmp_path / "display-recovery.json"
    previous = _mode()
    session = display_session.DisplayModeSession(manifest)
    with (
        patch.object(display_session, "test_display_mode", return_value={"accepted": True, "code": 0}),
        patch.object(display_session, "current_display_mode", return_value=previous),
        patch.object(display_session, "_read_mode", return_value=_fake_devmode()),
        patch.object(display_session, "_change_mode", side_effect=OSError("change failed")),
    ):
        result = session.apply(1568, 1080, 240, 20)

    assert result["applied"] is False
    assert result["result"] == "display_change_failed"
    assert manifest.exists()
    assert session.status()["active"] is False
    assert session.status()["recovery_pending"] is True


def test_manifest_recovery_clears_only_after_successful_recovery(tmp_path):
    manifest = tmp_path / "display-recovery.json"
    previous = _mode()
    requested = _mode(1568, 1080, 240)
    manifest.write_text(
        json.dumps({"schema_version": 1, "previous": asdict(previous), "requested": asdict(requested)}),
        encoding="utf-8",
    )
    session = display_session.DisplayModeSession(manifest)
    with (
        patch.object(display_session, "current_display_mode", return_value=requested),
        patch.object(display_session, "_read_mode", return_value=_fake_devmode()),
        patch.object(display_session, "_change_mode", return_value=0),
    ):
        result = session.recover_if_needed()

    assert result["recovered"] is True
    assert result["reason"] == "successful"
    assert not manifest.exists()
    assert session.status()["recovery_pending"] is False


def test_already_restored_manifest_cleanup_failure_is_not_reported_as_recovered(tmp_path):
    manifest = tmp_path / "display-recovery.json"
    previous = _mode()
    manifest.write_text(
        json.dumps({"schema_version": 1, "previous": asdict(previous)}),
        encoding="utf-8",
    )
    session = display_session.DisplayModeSession(manifest)
    with (
        patch.object(display_session, "current_display_mode", return_value=previous),
        patch.object(session, "_clear_manifest", return_value=False),
    ):
        result = session.recover_if_needed()

    assert result["recovered"] is False
    assert result["reason"] == "recovery_manifest_cleanup_failed"
    assert session.status()["recovery_pending"] is True
