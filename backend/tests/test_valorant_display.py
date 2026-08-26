from __future__ import annotations

from types import SimpleNamespace
from unittest.mock import patch

from app.valorant_lab import display


def _monitor(
    name: str,
    instance_id: str,
    *,
    present: bool | None,
    problem_code: int | None,
    status: str = "",
    hardware_ids: list[str] | None = None,
) -> dict:
    return {
        "class": "Monitor",
        "friendly_name": name,
        "instance_id": instance_id,
        "present": present,
        "problem_code": problem_code,
        "status": status,
        "hardware_ids": hardware_ids or [],
    }


def test_expected_present_code22_monitors_and_historical_rows_are_separated():
    result = display.collect_display_status(
        gpu_rows=[{"name": "NVIDIA GeForce RTX 5070", "status": "OK"}],
        monitor_rows=[
            _monitor("A", r"DISPLAY\DEL0001", present=True, problem_code=22),
            _monitor("B", r"DISPLAY\AOC0002", present=True, problem_code=22),
            _monitor("Old A", r"DISPLAY\OLD0001", present=False, problem_code=45),
            _monitor("Old B", r"DISPLAY\OLD0002", present=False, problem_code=45),
        ],
        primary_display={"width": 2560, "height": 1440, "refresh_rate_hz": 240},
    )

    assert result["status"] == "all_present_physical_monitors_disabled"
    assert result["monitor_disable_status"] == "all_present_physical_monitors_disabled"
    assert result["safe_to_skip_disable"] is True
    assert len(result["monitors"]) == 2
    assert len(result["excluded_monitors"]) == 2
    assert result["monitor"]["historical_count"] == 2
    assert {row["exclude_reason"] for row in result["excluded_monitors"]} == {
        "not_present_historical"
    }
    assert result["resolution"] == {
        "status": "ready",
        "width": 2560,
        "height": 1440,
        "source": "injected",
    }
    assert result["refresh_rate"]["value"] == 240


def test_partial_and_needs_disable_are_distinct():
    partial = display.evaluate_monitor_disable_status(
        [
            _monitor("A", r"DISPLAY\A", present=True, problem_code=22),
            _monitor("B", r"DISPLAY\B", present=True, problem_code=0),
        ]
    )
    needs = display.evaluate_monitor_disable_status(
        [
            _monitor("A", r"DISPLAY\A", present=True, problem_code=0),
            _monitor("B", r"DISPLAY\B", present=True, problem_code=0),
        ]
    )

    assert partial["status"] == "partial"
    assert partial["evidence"]["disabled_count"] == 1
    assert needs["status"] == "needs_disable"
    assert needs["safe_to_skip_disable"] is False


def test_any_missing_problem_code_or_query_failure_is_unknown_not_safe():
    missing = display.evaluate_monitor_disable_status(
        [_monitor("A", r"DISPLAY\A", present=True, problem_code=None, status="Unknown")]
    )
    failed = display.evaluate_monitor_disable_status([], query_ok=False, query_error="query failed")

    assert missing["status"] == "unknown"
    assert missing["safe_to_skip_disable"] is False
    assert failed["status"] == "unknown"
    assert failed["evidence"]["query_error"] == "query failed"
    assert failed["safe_to_skip_disable"] is False


def test_obvious_virtual_and_historical_devices_remain_in_evidence_but_are_excluded():
    result = display.evaluate_monitor_disable_status(
        [
            _monitor("Microsoft Indirect Display", r"ROOT\IDD0001", present=True, problem_code=22),
            _monitor("Past physical monitor", r"DISPLAY\OLD", present=None, problem_code=45),
            _monitor("Physical", r"DISPLAY\REAL", present=True, problem_code=22),
        ]
    )

    assert result["status"] == "all_present_physical_monitors_disabled"
    assert len(result["monitors"]) == 1
    assert len(result["excluded_monitors"]) == 2
    reasons = [row["exclude_reason"] for row in result["excluded_monitors"]]
    assert any(str(reason).startswith("obvious_virtual_") for reason in reasons)
    assert "problem_code_45_historical" in reasons


def test_contradictory_present_code45_is_unknown_and_not_history_skipped():
    result = display.evaluate_monitor_disable_status(
        [_monitor("Contradictory", r"DISPLAY\WEIRD", present=True, problem_code=45)]
    )

    assert result["status"] == "unknown"
    assert result["evidence"]["uncertain_count"] == 1
    assert not result["excluded_monitors"]


def test_successful_empty_inventory_is_not_applicable_but_query_failure_is_unknown():
    empty = display.evaluate_monitor_disable_status([])
    failed = display.evaluate_monitor_disable_status([], query_ok=False, query_error="unavailable")

    assert empty["status"] == "not_applicable"
    assert failed["status"] == "unknown"


def test_powershell_is_invoked_with_argument_list_and_shell_disabled():
    completed = SimpleNamespace(returncode=0, stdout='{"gpus": []}', stderr="")
    with (
        patch.object(display, "_is_windows", return_value=True),
        patch.object(display, "_powershell_executable", return_value="C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe"),
        patch.object(display.subprocess, "run", return_value=completed) as run,
    ):
        payload, error = display._run_powershell_json("[pscustomobject]@{gpus=@()} | ConvertTo-Json")

    assert error is None
    assert payload == {"gpus": []}
    args, kwargs = run.call_args
    assert args[0][0].lower().endswith("powershell.exe")
    assert args[0][1:5] == ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"]
    assert kwargs["shell"] is False
    assert kwargs["check"] is False


def test_display_payload_exposes_primary_resolution_and_refresh_without_mutation():
    result = display.collect_display_status(
        gpu_rows=[{"name": "GPU", "status": "OK"}],
        monitor_rows=[_monitor("A", r"DISPLAY\A", present=True, problem_code=22)],
        primary_display={
            "device_name": r"\\.\DISPLAY1",
            "width": 1920,
            "height": 1080,
            "refresh_rate_hz": 165,
            "is_primary": True,
        },
    )

    assert result["primary_display"]["is_primary"] is True
    assert result["primary_display"]["width"] == 1920
    assert result["primary_display"]["height"] == 1080
    assert result["primary_display"]["refresh_rate_hz"] == 165
    assert result["evidence"]["mutation_attempted"] is False
    assert result["evidence"]["mutation_apis"] == []
