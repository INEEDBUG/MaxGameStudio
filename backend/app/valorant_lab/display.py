"""Read-only Windows display and monitor inventory for the VALORANT lab.

This module deliberately has no device-control surface.  It only reads:

* GPUs through the read-only ``Win32_VideoController`` CIM class;
* monitor PnP state through ``Get-PnpDevice`` and
  ``Get-PnpDeviceProperty``; and
* the current primary display mode through ``EnumDisplaySettingsW``.

The PowerShell process is always started with an argument list and
``shell=False``.  The PowerShell program is static and contains no command
or path supplied by a caller.  In particular, this module never invokes
device-control cmdlets and never changes a display mode.

The monitor decision is intentionally conservative.  A monitor is counted as
disabled only when it is explicitly present and its ConfigMgr problem code is
22.  Missing or contradictory evidence yields ``unknown`` rather than a safe
skip decision.  Historical devices (not present / problem code 45) and
obvious virtual displays are retained in the evidence but do not enter the
physical-monitor decision.
"""

from __future__ import annotations

import ctypes
import json
import os
import re
import shutil
import subprocess
import sys
from datetime import datetime, timezone
from typing import Any, Iterable, Literal, Mapping


MonitorDisableStatus = Literal[
    "all_present_physical_monitors_disabled",
    "partial",
    "needs_disable",
    "unknown",
    "not_applicable",
]

ALL_PRESENT_PHYSICAL_MONITORS_DISABLED = "all_present_physical_monitors_disabled"
PARTIAL = "partial"
NEEDS_DISABLE = "needs_disable"
UNKNOWN = "unknown"
NOT_APPLICABLE = "not_applicable"

_UNSET = object()
_ENUM_CURRENT_SETTINGS = -1
_DISPLAY_DEVICE_PRIMARY_DEVICE = 0x00000004
_DISPLAY_DEVICE_ACTIVE = 0x00000001
_MAX_DISPLAY_ADAPTERS = 32


_VIRTUAL_NAME_MARKERS = (
    "virtual",
    "indirect display",
    "indirectdisplay",
    "remote display",
    "microsoft remote display",
    "rdp display",
    "dummy display",
    "headless display",
    "mirage display",
    "spacedesk",
    "parsec",
    "sunshine",
    "moonlight",
    "vmware",
    "virtualbox",
    "hyper-v",
    "hyperv",
    "teamviewer",
    "anydesk",
    "splashtop",
    "citrix",
    "vnc",
)
_VIRTUAL_ID_RE = re.compile(
    r"(?:^|[\\/:])(?:RDP(?:DD|UDD|DISPLAY|ENUM)?|MSRDP|IDD|MIRAGE|SPACEDESK|PARSEC|"
    r"VIRTUAL(?:DISPLAY)?|VMWARE|VIRTUALBOX|HYPER[-_]?V|VNC)(?:$|[\\/:_-]|[A-Z0-9])",
    re.IGNORECASE,
)


def _utc_now_iso() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _is_windows() -> bool:
    return sys.platform == "win32" or os.name == "nt"


def _as_int(value: Any) -> int | None:
    if value is None or isinstance(value, bool):
        return None
    if isinstance(value, int):
        return int(value)
    if isinstance(value, float):
        return int(value) if value.is_integer() else None
    text = str(value).strip()
    if not text:
        return None
    try:
        return int(text, 0)
    except ValueError:
        try:
            return int(text, 10)
        except ValueError:
            return None


def _as_bool(value: Any) -> bool | None:
    if isinstance(value, bool):
        return value
    if isinstance(value, int) and value in (0, 1):
        return bool(value)
    if value is None:
        return None
    text = str(value).strip().casefold()
    if text in {"true", "yes", "1", "present", "connected", "active"}:
        return True
    if text in {"false", "no", "0", "absent", "not present", "disconnected", "inactive"}:
        return False
    return None


def _as_text(value: Any) -> str:
    if value is None:
        return ""
    if isinstance(value, bytes):
        return value.decode("utf-8", errors="replace").strip()
    return str(value).strip()


def _as_list(value: Any) -> list[Any]:
    if value is None:
        return []
    if isinstance(value, (list, tuple, set)):
        return list(value)
    return [value]


def _normalise_hardware_ids(value: Any) -> list[str]:
    result: list[str] = []
    seen: set[str] = set()
    for item in _as_list(value):
        # A registry multi-string can occasionally arrive as one newline-
        # separated value through a PowerShell adapter.
        parts = str(item).replace("\r", "\n").split("\n")
        for part in parts:
            text = part.strip().strip("\x00")
            if text and text.casefold() not in seen:
                result.append(text)
                seen.add(text.casefold())
    return result


def _powershell_executable() -> str | None:
    for candidate in ("powershell.exe", "powershell"):
        resolved = shutil.which(candidate)
        if resolved:
            return resolved
    return None


def _hidden_startupinfo() -> Any:
    """Build a hidden-window STARTUPINFO when the Windows runtime exposes it."""

    startupinfo_type = getattr(subprocess, "STARTUPINFO", None)
    if startupinfo_type is None:
        return None
    try:
        startupinfo = startupinfo_type()
        startupinfo.dwFlags |= getattr(subprocess, "STARTF_USESHOWWINDOW", 0)
        startupinfo.wShowWindow = getattr(subprocess, "SW_HIDE", 0)
        return startupinfo
    except Exception:
        return None


def _run_powershell_json(script: str, *, timeout: float = 8.0) -> tuple[Any | None, str | None]:
    """Run a fixed read-only PowerShell query without going through a shell."""

    if not _is_windows():
        return None, "Windows-only display query is not applicable on this platform"
    executable = _powershell_executable()
    if not executable:
        return None, "powershell.exe was not found"

    args = [
        executable,
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        script,
    ]
    kwargs: dict[str, Any] = {
        "capture_output": True,
        "check": False,
        "encoding": "utf-8",
        "errors": "replace",
        "shell": False,
        "stdin": subprocess.DEVNULL,
        "timeout": timeout,
    }
    if _is_windows():
        kwargs["creationflags"] = getattr(subprocess, "CREATE_NO_WINDOW", 0)
        startupinfo = _hidden_startupinfo()
        if startupinfo is not None:
            kwargs["startupinfo"] = startupinfo
    try:
        completed = subprocess.run(args, **kwargs)
    except (OSError, subprocess.SubprocessError) as exc:
        return None, f"PowerShell invocation failed: {type(exc).__name__}: {exc}"

    returncode = getattr(completed, "returncode", 1)
    stdout = _as_text(getattr(completed, "stdout", ""))
    stderr = _as_text(getattr(completed, "stderr", ""))
    if returncode != 0:
        detail = stderr or stdout or f"exit code {returncode}"
        return None, f"PowerShell query failed: {detail[-500:]}"
    if not stdout:
        return None, "PowerShell query returned no JSON"
    try:
        return json.loads(stdout), None
    except json.JSONDecodeError as exc:
        return None, f"PowerShell query returned invalid JSON: {exc.msg}"


_GPU_QUERY = r'''
$ErrorActionPreference = 'Stop'
$gpus = @(Get-CimInstance -ClassName Win32_VideoController | ForEach-Object {
    [pscustomobject]@{
        name = [string]$_.Name
        adapter_compatibility = [string]$_.AdapterCompatibility
        pnp_device_id = [string]$_.PNPDeviceID
        status = [string]$_.Status
        driver_version = [string]$_.DriverVersion
        adapter_ram = $_.AdapterRAM
        video_mode_description = [string]$_.VideoModeDescription
    }
})
[pscustomobject]@{ gpus = $gpus } | ConvertTo-Json -Compress -Depth 6
'''


_MONITOR_QUERY = r'''
$ErrorActionPreference = 'Stop'

# Get-PnpDevice exposes the device/property path used by the normal probe.
# Some Windows builds report disabled monitor rows as Present=False and omit
# the property value, so ConfigMgr's Win32_PnPEntity view is collected as an
# independent read-only cross-check and merged by instance id below.
$pnpRows = @()
$pnpError = $null
try {
    $pnpRows = @(Get-PnpDevice -Class Monitor -PresentOnly:$false -ErrorAction Stop | ForEach-Object {
        $device = $_
        $problemCode = $null
        $hardwareIds = @()
        try {
            $problem = Get-PnpDeviceProperty -InstanceId $device.InstanceId -KeyName 'DEVPKEY_Device_ProblemCode' -ErrorAction Stop
            if ($null -ne $problem.Data) { $problemCode = [int]$problem.Data }
        } catch { }
        try {
            $hardware = Get-PnpDeviceProperty -InstanceId $device.InstanceId -KeyName 'DEVPKEY_Device_HardwareIds' -ErrorAction Stop
            $hardwareIds = @($hardware.Data | ForEach-Object { [string]$_ })
        } catch { }
        $present = $null
        if ($device.PSObject.Properties.Name -contains 'Present') { $present = [bool]$device.Present }
        [pscustomobject]@{
            class = [string]$device.Class
            class_guid = [string]$device.ClassGuid
            friendly_name = [string]$device.FriendlyName
            instance_id = [string]$device.InstanceId
            manufacturer = [string]$device.Manufacturer
            service = [string]$device.Service
            status = [string]$device.Status
            present = $present
            problem_code = $problemCode
            hardware_ids = $hardwareIds
            evidence_sources = @('Get-PnpDevice', 'Get-PnpDeviceProperty')
        }
    })
} catch {
    $pnpError = $_.Exception.Message
}

# ConfigMgr exposes Present and ConfigManagerErrorCode together.  The class
# GUID filter keeps this query scoped to display monitors even if PNPClass is
# missing on an older Windows build.
$configRows = @()
$configError = $null
try {
    $configRows = @(Get-CimInstance -ClassName Win32_PnPEntity -ErrorAction Stop |
        Where-Object { $_.PNPClass -eq 'Monitor' -or $_.ClassGuid -eq '{4d36e96e-e325-11ce-bfc1-08002be10318}' } |
        ForEach-Object {
            $ids = @()
            if ($null -ne $_.HardwareID) { $ids = @($_.HardwareID | ForEach-Object { [string]$_ }) }
            [pscustomobject]@{
                class = [string]$_.PNPClass
                class_guid = [string]$_.ClassGuid
                friendly_name = [string]$_.Name
                instance_id = [string]$_.PNPDeviceID
                manufacturer = ''
                service = ''
                status = [string]$_.Status
                present = $_.Present
                problem_code = $_.ConfigManagerErrorCode
                hardware_ids = $ids
                evidence_sources = @('ConfigMgr:Win32_PnPEntity')
            }
        })
} catch {
    $configError = $_.Exception.Message
}

if ($pnpRows.Count -eq 0 -and $configRows.Count -eq 0 -and $pnpError -and $configError) {
    throw "Get-PnpDevice and ConfigMgr queries failed: $pnpError / $configError"
}

$configById = @{}
foreach ($row in $configRows) {
    $key = [string]$row.instance_id
    if ($key) { $configById[$key.ToUpperInvariant()] = $row }
}
$seen = @{}
$monitors = @()
foreach ($row in $pnpRows) {
    $key = [string]$row.instance_id
    $folded = if ($key) { $key.ToUpperInvariant() } else { '' }
    $config = if ($folded -and $configById.ContainsKey($folded)) { $configById[$folded] } else { $null }
    if ($config) {
        $mergedIds = @($row.hardware_ids + $config.hardware_ids | Where-Object { $_ } | Select-Object -Unique)
        $mergedPresent = if ($null -ne $config.present) { $config.present } else { $row.present }
        $mergedProblem = if ($null -ne $config.problem_code) { [int]$config.problem_code } else { $row.problem_code }
        $mergedStatus = if ($config.status) { $config.status } else { $row.status }
        $monitors += [pscustomobject]@{
            class = if ($row.class) { $row.class } else { $config.class }
            class_guid = if ($row.class_guid) { $row.class_guid } else { $config.class_guid }
            friendly_name = if ($row.friendly_name) { $row.friendly_name } else { $config.friendly_name }
            instance_id = if ($row.instance_id) { $row.instance_id } else { $config.instance_id }
            manufacturer = if ($row.manufacturer) { $row.manufacturer } else { $config.manufacturer }
            service = if ($row.service) { $row.service } else { $config.service }
            status = $mergedStatus
            present = $mergedPresent
            problem_code = $mergedProblem
            hardware_ids = $mergedIds
            evidence_sources = @($row.evidence_sources + $config.evidence_sources | Select-Object -Unique)
        }
    } else {
        $monitors += $row
    }
    if ($folded) { $seen[$folded] = $true }
}
foreach ($row in $configRows) {
    $key = [string]$row.instance_id
    $folded = if ($key) { $key.ToUpperInvariant() } else { '' }
    if (-not $folded -or -not $seen.ContainsKey($folded)) { $monitors += $row }
}

[pscustomobject]@{
    monitors = $monitors
    pnp_error = $pnpError
    configmgr_error = $configError
    query_sources = @('Get-PnpDevice', 'Get-PnpDeviceProperty', 'ConfigMgr:Win32_PnPEntity')
} | ConvertTo-Json -Compress -Depth 8
'''


def _normalise_rows(value: Any, key: str) -> list[Mapping[str, Any]]:
    if isinstance(value, Mapping):
        if key not in value:
            # Accept a direct single-row payload as well as the wrapped form
            # emitted by the fixed PowerShell queries.  This also makes the
            # parser tolerant of mocked/native connector responses.
            return [value]
        value = value.get(key, [])
    if isinstance(value, Mapping):
        return [value]
    if not isinstance(value, list):
        return []
    return [row for row in value if isinstance(row, Mapping)]


def _enumerate_gpus() -> tuple[list[dict[str, Any]], str | None]:
    payload, error = _run_powershell_json(_GPU_QUERY)
    if error:
        return [], error
    rows: list[dict[str, Any]] = []
    for raw in _normalise_rows(payload, "gpus"):
        name = _as_text(raw.get("name") or raw.get("Name"))
        if not name:
            continue
        adapter_ram = _as_int(raw.get("adapter_ram") or raw.get("AdapterRAM"))
        rows.append(
            {
                "name": name,
                "adapter_compatibility": _as_text(
                    raw.get("adapter_compatibility") or raw.get("AdapterCompatibility")
                ),
                "pnp_device_id": _as_text(raw.get("pnp_device_id") or raw.get("PNPDeviceID")),
                "status": _as_text(raw.get("status") or raw.get("Status")),
                "driver_version": _as_text(raw.get("driver_version") or raw.get("DriverVersion")),
                "adapter_ram_bytes": adapter_ram,
                "video_mode_description": _as_text(
                    raw.get("video_mode_description") or raw.get("VideoModeDescription")
                ),
            }
        )
    return rows, None


def _normalise_monitor_row(raw: Mapping[str, Any]) -> dict[str, Any]:
    problem_code = _as_int(
        raw.get("problem_code")
        if "problem_code" in raw
        else raw.get("ProblemCode", raw.get("config_manager_error_code"))
    )
    present_value = raw.get("present") if "present" in raw else raw.get("Present")
    present = _as_bool(present_value)
    instance_id = _as_text(raw.get("instance_id") or raw.get("InstanceId") or raw.get("pnp_device_id"))
    hardware_ids = _normalise_hardware_ids(raw.get("hardware_ids", raw.get("HardwareIds")))
    evidence_sources = [
        _as_text(item)
        for item in _as_list(raw.get("evidence_sources", raw.get("EvidenceSources")))
        if _as_text(item)
    ]
    friendly_name = _as_text(
        raw.get("friendly_name")
        or raw.get("FriendlyName")
        or raw.get("name")
        or raw.get("Name")
        or raw.get("description")
    )
    return {
        "class": _as_text(raw.get("class") or raw.get("Class") or "Monitor"),
        "class_guid": _as_text(raw.get("class_guid") or raw.get("ClassGuid")),
        "friendly_name": friendly_name,
        "instance_id": instance_id,
        "manufacturer": _as_text(raw.get("manufacturer") or raw.get("Manufacturer")),
        "service": _as_text(raw.get("service") or raw.get("Service")),
        "status": _as_text(raw.get("status") or raw.get("Status")),
        "present": present,
        "problem_code": problem_code,
        "hardware_ids": hardware_ids,
        "evidence_sources": evidence_sources,
    }


def _enumerate_pnp_monitors() -> tuple[list[dict[str, Any]], str | None]:
    payload, error = _run_powershell_json(_MONITOR_QUERY)
    if error:
        return [], error
    rows = [_normalise_monitor_row(row) for row in _normalise_rows(payload, "monitors")]
    if isinstance(payload, Mapping):
        pnp_error = _as_text(payload.get("pnp_error"))
        config_error = _as_text(payload.get("configmgr_error"))
        if not rows and pnp_error and config_error:
            return [], f"Get-PnpDevice and ConfigMgr queries failed: {pnp_error} / {config_error}"
    return rows, None


def _virtual_monitor_reason(row: Mapping[str, Any]) -> str | None:
    name = _as_text(row.get("friendly_name"))
    instance_id = _as_text(row.get("instance_id"))
    hardware_ids = _normalise_hardware_ids(row.get("hardware_ids"))
    name_folded = name.casefold()
    for marker in _VIRTUAL_NAME_MARKERS:
        if marker in name_folded:
            return f"obvious_virtual_name:{marker}"
    for value in [instance_id, *hardware_ids]:
        if _VIRTUAL_ID_RE.search(value):
            return f"obvious_virtual_id:{value}"
    return None


def _monitor_disabled_state(row: Mapping[str, Any]) -> tuple[bool | None, str]:
    if row.get("present") is not True:
        return None, "not_present"
    problem_code = row.get("problem_code")
    if problem_code == 22:
        return True, "problem_code_22"
    if problem_code == 45:
        return None, "problem_code_45_conflicts_with_present"
    if problem_code is not None:
        return False, f"problem_code_{problem_code}"
    status = _as_text(row.get("status")).casefold()
    if status in {"ok", "started", "running", "active"}:
        return False, f"status_{status}"
    return None, "problem_code_missing"


def _classify_monitor_row(raw: Mapping[str, Any]) -> dict[str, Any]:
    row = _normalise_monitor_row(raw)
    row["considered"] = False
    row["excluded"] = False
    row["exclude_reason"] = None
    row["physical"] = None
    row["disabled"] = None
    row["state_evidence"] = ""

    virtual_reason = _virtual_monitor_reason(row)
    if virtual_reason:
        row["excluded"] = True
        row["exclude_reason"] = virtual_reason
        row["physical"] = False
        return row

    # Present=False is the authoritative historical-device signal.  Code 45
    # is also treated as historical when Present is unavailable, but a
    # contradictory Present=True + Code45 remains unknown and is retained.
    if row["present"] is False:
        row["excluded"] = True
        row["exclude_reason"] = "not_present_historical"
        row["physical"] = False
        return row
    if row["present"] is not True and row["problem_code"] == 45:
        row["excluded"] = True
        row["exclude_reason"] = "problem_code_45_historical"
        row["physical"] = False
        return row

    device_class = _as_text(row.get("class")).casefold()
    if device_class and device_class not in {"monitor", "display"}:
        row["considered"] = True
        row["physical"] = None
        row["state_evidence"] = f"unexpected_device_class:{device_class}"
        return row
    if row["present"] is not True:
        row["considered"] = True
        row["physical"] = None
        row["state_evidence"] = "present_state_missing"
        return row

    # Get-PnpDevice was queried with -Class Monitor.  A present row in that
    # class is physical unless it matched one of the explicit virtual-device
    # rules above.  We retain all identifiers as evidence.
    row["considered"] = True
    row["physical"] = True
    row["disabled"], row["state_evidence"] = _monitor_disabled_state(row)
    if row["disabled"] is None:
        row["state"] = "unknown"
    elif row["disabled"]:
        row["state"] = "disabled"
    else:
        row["state"] = "enabled_or_not_disabled"
    return row


def evaluate_monitor_disable_status(
    monitor_rows: Iterable[Mapping[str, Any]],
    *,
    query_ok: bool = True,
    query_error: str | None = None,
) -> dict[str, Any]:
    """Classify monitor rows and return a conservative disable decision."""

    if monitor_rows is None:  # type: ignore[comparison-overlap]
        query_ok = False
        query_error = query_error or "monitor inventory was not provided"
        monitor_rows = []
    classified = [_classify_monitor_row(row) for row in monitor_rows]
    considered = [row for row in classified if row.get("considered")]
    excluded = [row for row in classified if row.get("excluded")]
    uncertain_rows = [
        row
        for row in considered
        if row.get("physical") is not True or row.get("disabled") is None
    ]
    historical_rows = [
        row
        for row in excluded
        if str(row.get("exclude_reason") or "").startswith(("not_present", "problem_code_45"))
    ]
    virtual_rows = [
        row
        for row in excluded
        if str(row.get("exclude_reason") or "").startswith("obvious_virtual_")
    ]
    query_sources = sorted(
        {
            _as_text(source)
            for row in classified
            for source in _as_list(row.get("evidence_sources"))
            if _as_text(source)
        }
    )

    if not query_ok:
        status: MonitorDisableStatus = UNKNOWN
    elif uncertain_rows:
        status = UNKNOWN
    elif not considered:
        status = NOT_APPLICABLE
    else:
        states = [bool(row.get("disabled")) for row in considered]
        if all(states):
            status = ALL_PRESENT_PHYSICAL_MONITORS_DISABLED
        elif any(states):
            status = PARTIAL
        else:
            status = NEEDS_DISABLE

    evidence = {
        "query_ok": bool(query_ok),
        "query_error": query_error,
        "query_sources": query_sources,
        "decision_rule": "only Present=True and ProblemCode=22 counts as disabled",
        "considered_count": len(considered),
        "disabled_count": sum(1 for row in considered if row.get("disabled") is True),
        "uncertain_count": len(uncertain_rows),
        "excluded_count": len(excluded),
        "historical_count": len(historical_rows),
        "virtual_count": len(virtual_rows),
        "excluded_reasons": [row.get("exclude_reason") for row in excluded],
        "uncertainty_reasons": [
            {
                "instance_id": row.get("instance_id"),
                "reason": row.get("state_evidence") or "physical_or_problem_state_unknown",
            }
            for row in uncertain_rows
        ],
        "read_only": True,
        "mutation_attempted": False,
    }
    return {
        "status": status,
        "monitor_disable_status": status,
        "monitors": considered,
        "present_physical_monitors": considered,
        "excluded_monitors": excluded,
        "historical_monitors": historical_rows,
        "virtual_monitors": virtual_rows,
        "evidence": evidence,
        "safe_to_skip_disable": status == ALL_PRESENT_PHYSICAL_MONITORS_DISABLED,
    }


# Concise aliases for callers/tests that use the term inventory rather than
# the longer decision name.
classify_monitor_inventory = evaluate_monitor_disable_status
classify_monitor_status = evaluate_monitor_disable_status


class _POINTL(ctypes.Structure):
    _fields_ = [("x", ctypes.c_int32), ("y", ctypes.c_int32)]


class _DEVMODE_DISPLAY(ctypes.Structure):
    _fields_ = [
        ("dmPosition", _POINTL),
        ("dmDisplayOrientation", ctypes.c_uint32),
        ("dmDisplayFixedOutput", ctypes.c_uint32),
    ]


class _DEVMODE_PRINTER(ctypes.Structure):
    _fields_ = [
        ("dmOrientation", ctypes.c_int16),
        ("dmPaperSize", ctypes.c_int16),
        ("dmPaperLength", ctypes.c_int16),
        ("dmPaperWidth", ctypes.c_int16),
        ("dmScale", ctypes.c_int16),
        ("dmCopies", ctypes.c_int16),
        ("dmDefaultSource", ctypes.c_int16),
        ("dmPrintQuality", ctypes.c_int16),
    ]


class _DEVMODE_UNION(ctypes.Union):
    _fields_ = [("display", _DEVMODE_DISPLAY), ("printer", _DEVMODE_PRINTER)]


class _DEVMODE_FLAGS_UNION(ctypes.Union):
    _fields_ = [("dmDisplayFlags", ctypes.c_uint32), ("dmNup", ctypes.c_uint32)]


class _DEVMODEW(ctypes.Structure):
    # WCHAR is represented as uint16 here so the structure has the Windows
    # layout even when a test imports this module on a non-Windows host.
    _fields_ = [
        ("dmDeviceName", ctypes.c_uint16 * 32),
        ("dmSpecVersion", ctypes.c_uint16),
        ("dmDriverVersion", ctypes.c_uint16),
        ("dmSize", ctypes.c_uint16),
        ("dmDriverExtra", ctypes.c_uint16),
        ("dmFields", ctypes.c_uint32),
        ("u1", _DEVMODE_UNION),
        ("dmColor", ctypes.c_int16),
        ("dmDuplex", ctypes.c_int16),
        ("dmYResolution", ctypes.c_int16),
        ("dmTTOption", ctypes.c_int16),
        ("dmCollate", ctypes.c_int16),
        ("dmFormName", ctypes.c_uint16 * 32),
        ("dmLogPixels", ctypes.c_uint16),
        ("dmBitsPerPel", ctypes.c_uint32),
        ("dmPelsWidth", ctypes.c_uint32),
        ("dmPelsHeight", ctypes.c_uint32),
        ("u2", _DEVMODE_FLAGS_UNION),
        ("dmDisplayFrequency", ctypes.c_uint32),
        ("dmICMMethod", ctypes.c_uint32),
        ("dmICMIntent", ctypes.c_uint32),
        ("dmMediaType", ctypes.c_uint32),
        ("dmDitherType", ctypes.c_uint32),
        ("dmReserved1", ctypes.c_uint32),
        ("dmReserved2", ctypes.c_uint32),
        ("dmPanningWidth", ctypes.c_uint32),
        ("dmPanningHeight", ctypes.c_uint32),
    ]


class _DISPLAY_DEVICEW(ctypes.Structure):
    _fields_ = [
        ("cb", ctypes.c_uint32),
        ("DeviceName", ctypes.c_uint16 * 32),
        ("DeviceString", ctypes.c_uint16 * 128),
        ("StateFlags", ctypes.c_uint32),
        ("DeviceID", ctypes.c_uint16 * 128),
        ("DeviceKey", ctypes.c_uint16 * 128),
    ]


def _utf16_array_text(value: Any) -> str:
    if isinstance(value, str):
        return value.rstrip("\x00")
    try:
        chars = [int(item) for item in value]
        raw = b"".join(item.to_bytes(2, "little", signed=False) for item in chars)
        return raw.decode("utf-16-le", errors="replace").rstrip("\x00")
    except Exception:
        return _as_text(value)


def _load_user32() -> Any:
    loader = getattr(ctypes, "WinDLL", None)
    if loader is not None:
        return loader("user32", use_last_error=True)
    return ctypes.windll.user32  # type: ignore[attr-defined]


def _set_winapi_signature(function: Any, *, argtypes: list[Any], restype: Any) -> None:
    try:
        function.argtypes = argtypes
        function.restype = restype
    except Exception:
        # A fake function supplied by a unit test may not support ctypes
        # metadata.  Calling it is still safe; real WinDLL functions do.
        pass


def _read_primary_display() -> tuple[dict[str, Any] | None, str | None]:
    """Read the active primary display mode using user32, without mutation."""

    if not _is_windows():
        return None, "Windows-only display mode query is not applicable on this platform"
    try:
        user32 = _load_user32()
        enum_devices = getattr(user32, "EnumDisplayDevicesW", None)
        enum_settings = getattr(user32, "EnumDisplaySettingsW", None)
        if enum_devices is None or enum_settings is None:
            return None, "user32 display enumeration APIs are unavailable"
        _set_winapi_signature(
            enum_devices,
            argtypes=[ctypes.c_wchar_p, ctypes.c_uint32, ctypes.POINTER(_DISPLAY_DEVICEW), ctypes.c_uint32],
            restype=ctypes.c_int,
        )
        _set_winapi_signature(
            enum_settings,
            argtypes=[ctypes.c_wchar_p, ctypes.c_uint32, ctypes.POINTER(_DEVMODEW)],
            restype=ctypes.c_int,
        )

        primary: _DISPLAY_DEVICEW | None = None
        first_active: _DISPLAY_DEVICEW | None = None
        for index in range(_MAX_DISPLAY_ADAPTERS):
            device = _DISPLAY_DEVICEW()
            device.cb = ctypes.sizeof(_DISPLAY_DEVICEW)
            try:
                ok = bool(enum_devices(None, index, ctypes.byref(device), 0))
            except Exception as exc:
                return None, f"EnumDisplayDevicesW failed: {type(exc).__name__}: {exc}"
            if not ok:
                break
            state_flags = int(device.StateFlags)
            if not (state_flags & _DISPLAY_DEVICE_ACTIVE):
                continue
            if first_active is None:
                first_active = device
            if state_flags & _DISPLAY_DEVICE_PRIMARY_DEVICE:
                primary = device
                break
        selected = primary or first_active
        if selected is None:
            return None, "no active primary display adapter was enumerated"

        device_name = _utf16_array_text(selected.DeviceName)
        device_string = _utf16_array_text(selected.DeviceString)
        mode = _DEVMODEW()
        mode.dmSize = ctypes.sizeof(_DEVMODEW)
        try:
            ok = bool(enum_settings(device_name or None, _ENUM_CURRENT_SETTINGS, ctypes.byref(mode)))
        except Exception as exc:
            return None, f"EnumDisplaySettingsW failed: {type(exc).__name__}: {exc}"
        if not ok:
            return None, "EnumDisplaySettingsW returned no current mode"

        width = int(mode.dmPelsWidth)
        height = int(mode.dmPelsHeight)
        refresh = int(mode.dmDisplayFrequency)
        if width <= 0 or height <= 0:
            return None, "EnumDisplaySettingsW returned an invalid resolution"
        payload = {
            "device_name": device_name,
            "device_string": device_string,
            "state_flags": int(selected.StateFlags),
            "is_primary": bool(int(selected.StateFlags) & _DISPLAY_DEVICE_PRIMARY_DEVICE),
            "width": width,
            "height": height,
            "refresh_rate_hz": refresh if refresh > 0 else None,
            "source": "ctypes:user32.EnumDisplaySettingsW",
        }
        return payload, None if refresh > 0 else "EnumDisplaySettingsW returned no refresh rate"
    except Exception as exc:
        return None, f"ctypes display query failed: {type(exc).__name__}: {exc}"


def _normalise_display_payload(raw: Mapping[str, Any] | None) -> dict[str, Any]:
    value = dict(raw or {})
    width = _as_int(value.get("width") or value.get("dmPelsWidth"))
    height = _as_int(value.get("height") or value.get("dmPelsHeight"))
    refresh = _as_int(
        value.get("refresh_rate_hz")
        if "refresh_rate_hz" in value
        else value.get("refreshRateHz", value.get("refresh_rate"))
    )
    return {
        "device_name": _as_text(value.get("device_name") or value.get("deviceName")),
        "device_string": _as_text(value.get("device_string") or value.get("deviceString")),
        "state_flags": _as_int(value.get("state_flags")),
        "is_primary": _as_bool(value.get("is_primary")) if "is_primary" in value else True,
        "width": width,
        "height": height,
        "refresh_rate_hz": refresh,
        "refresh_rate": refresh,
        "refresh_hz": refresh,
        "source": _as_text(value.get("source")) or ("injected" if raw is not None else "unknown"),
    }


def _ui_status(status: str) -> str:
    if status == ALL_PRESENT_PHYSICAL_MONITORS_DISABLED:
        return "ready"
    if status in {PARTIAL, NEEDS_DISABLE}:
        return "warning"
    return "unknown"


def _preferred_gpu(gpus: list[dict[str, Any]]) -> dict[str, Any] | None:
    """Prefer a physical vendor GPU over root/remote display adapters."""

    if not gpus:
        return None

    def score(item: Mapping[str, Any]) -> tuple[int, int]:
        name = _as_text(item.get("name")).casefold()
        pnp_id = _as_text(item.get("pnp_device_id")).casefold()
        compatibility = _as_text(item.get("adapter_compatibility")).casefold()
        virtual = any(
            marker in f"{name} {compatibility}"
            for marker in ("virtual", "gameviewer", "todesk", "mumu", "remote", "indirect")
        ) or pnp_id.startswith("root\\display")
        physical_vendor = any(marker in f"{name} {compatibility}" for marker in ("nvidia", "amd", "radeon", "intel"))
        pci = pnp_id.startswith("pci\\")
        return (0 if virtual else 1, (2 if physical_vendor else 0) + (1 if pci else 0))

    return max(gpus, key=score)


def collect_display_status(
    *,
    gpu_rows: Iterable[Mapping[str, Any]] | object = _UNSET,
    monitor_rows: Iterable[Mapping[str, Any]] | object = _UNSET,
    primary_display: Mapping[str, Any] | None | object = _UNSET,
    gpu_error: str | None = None,
    monitor_error: str | None = None,
    display_error: str | None = None,
    checked_at: str | None = None,
) -> dict[str, Any]:
    """Collect a read-only display snapshot.

    The optional row arguments are intended for deterministic tests and for a
    future connector that already has a Windows inventory.  Omitting them
    performs the native Windows queries.  Passing an empty list is different
    from omitting the argument and represents a successful empty inventory.
    """

    windows = _is_windows()
    injected = any(value is not _UNSET for value in (gpu_rows, monitor_rows, primary_display))
    errors: list[str] = []

    if gpu_rows is _UNSET:
        if windows:
            detected_gpus, detected_error = _enumerate_gpus()
            gpus = detected_gpus
            gpu_error = gpu_error or detected_error
        else:
            gpus = []
    else:
        gpus = []
        if gpu_rows is None:
            gpu_error = gpu_error or "GPU inventory was explicitly unavailable"
        for row in _as_list(gpu_rows):
            if not isinstance(row, Mapping):
                continue
            name = _as_text(row.get("name") or row.get("Name"))
            if not name:
                continue
            gpus.append(
                {
                    "name": name,
                    "adapter_compatibility": _as_text(row.get("adapter_compatibility")),
                    "pnp_device_id": _as_text(row.get("pnp_device_id")),
                    "status": _as_text(row.get("status")),
                    "driver_version": _as_text(row.get("driver_version")),
                    "adapter_ram_bytes": _as_int(row.get("adapter_ram_bytes")),
                    "video_mode_description": _as_text(row.get("video_mode_description")),
                }
            )

    if monitor_rows is _UNSET:
        if windows:
            detected_monitors, detected_error = _enumerate_pnp_monitors()
            raw_monitors: list[Mapping[str, Any]] = detected_monitors
            monitor_error = monitor_error or detected_error
        else:
            raw_monitors = []
    else:
        if monitor_rows is None:
            monitor_error = monitor_error or "monitor inventory was explicitly unavailable"
        raw_monitors = [row for row in _as_list(monitor_rows) if isinstance(row, Mapping)]

    if monitor_error:
        errors.append(f"monitor: {monitor_error}")
    monitor_query_ok = not bool(monitor_error)
    # Non-Windows is a deliberate not-applicable result, not a failed query.
    if not windows and monitor_rows is _UNSET:
        monitor_query_ok = True
    monitor_result = evaluate_monitor_disable_status(
        raw_monitors,
        query_ok=monitor_query_ok,
        query_error=monitor_error,
    )
    monitor_status: MonitorDisableStatus = monitor_result["status"]

    if primary_display is _UNSET:
        if windows:
            display_payload, detected_error = _read_primary_display()
            display_error = display_error or detected_error
        else:
            display_payload = None
    else:
        display_payload = primary_display if isinstance(primary_display, Mapping) else None
    display = _normalise_display_payload(display_payload)
    if display_error:
        errors.append(f"display: {display_error}")

    if gpu_error:
        errors.append(f"gpu: {gpu_error}")

    if not windows and gpu_rows is _UNSET:
        gpu_status = NOT_APPLICABLE
    elif gpu_error:
        gpu_status = UNKNOWN
    elif gpus:
        gpu_status = "ready"
    else:
        gpu_status = UNKNOWN

    if not windows and primary_display is _UNSET:
        display_status = NOT_APPLICABLE
        refresh_status = NOT_APPLICABLE
    elif display_error:
        display_status = UNKNOWN
        refresh_status = UNKNOWN
    elif display["width"] and display["height"]:
        display_status = "ready"
        refresh_status = "ready" if display["refresh_rate_hz"] else UNKNOWN
    else:
        display_status = UNKNOWN
        refresh_status = UNKNOWN

    if not windows and monitor_rows is _UNSET:
        monitor_ui_status = "unknown"
    else:
        monitor_ui_status = _ui_status(monitor_status)

    monitor_names = [
        _as_text(row.get("friendly_name"))
        for row in monitor_result["present_physical_monitors"]
        if _as_text(row.get("friendly_name"))
    ]
    monitor_block = {
        "status": monitor_ui_status,
        "decision": monitor_status,
        "status_code": monitor_status,
        "disable_status": monitor_status,
        "monitor_disable_status": monitor_status,
        "name": monitor_names[0] if monitor_names else "",
        "present_physical_count": len(monitor_result["present_physical_monitors"]),
        "disabled_count": monitor_result["evidence"]["disabled_count"],
        "historical_count": monitor_result["evidence"]["historical_count"],
        "virtual_count": monitor_result["evidence"]["virtual_count"],
        "monitors": monitor_result["present_physical_monitors"],
        "excluded_monitors": monitor_result["excluded_monitors"],
        "evidence": monitor_result["evidence"],
    }
    preferred_gpu = _preferred_gpu(gpus)
    gpu_block = {
        "status": gpu_status,
        "name": preferred_gpu["name"] if preferred_gpu else "",
        "primary_device": preferred_gpu,
        "devices": gpus,
        "evidence": {
            "source": "PowerShell:Get-CimInstance Win32_VideoController" if windows else "not_applicable",
            "query_ok": not bool(gpu_error),
            "query_error": gpu_error,
            "count": len(gpus),
            "read_only": True,
        },
    }
    refresh_block = {
        "status": refresh_status,
        "value": display["refresh_rate_hz"],
        "value_hz": display["refresh_rate_hz"],
        "source": display["source"],
    }

    checked = checked_at or _utc_now_iso()
    # ``status`` is the machine-readable monitor decision requested by the
    # lab.  ``overall`` is intentionally the same value for compatibility;
    # ``ui_status`` gives frontends a small ready/warning/unknown vocabulary.
    component_unknown = bool(errors) or "unknown" in {gpu_status, display_status, refresh_status}
    overall = UNKNOWN if component_unknown and monitor_status == ALL_PRESENT_PHYSICAL_MONITORS_DISABLED else monitor_status
    result = {
        "ok": not component_unknown and monitor_status not in {UNKNOWN, NOT_APPLICABLE},
        "status": monitor_status,
        "monitor_status": monitor_status,
        "monitor_disable_status": monitor_status,
        "overall": overall,
        "ui_status": _ui_status(overall),
        # A safe skip is only valid when the complete read-only snapshot is
        # healthy.  A monitor-only success must not hide a failed GPU/display
        # probe from a caller deciding whether it may skip a disable step.
        "safe_to_skip_disable": monitor_result["safe_to_skip_disable"] and not component_unknown,
        "gpus": gpus,
        "gpu": gpu_block,
        "monitors": monitor_result["present_physical_monitors"],
        "excluded_monitors": monitor_result["excluded_monitors"],
        "monitor": monitor_block,
        "primary_display": display,
        "display": display,
        "resolution": {
            "status": display_status,
            "width": display["width"],
            "height": display["height"],
            "source": display["source"],
        },
        "refresh_rate": refresh_block,
        "refreshRate": refresh_block,
        "checked_at": checked,
        "checkedAt": checked,
        "source": "injected-test" if injected else "windows-read-only",
        "errors": errors,
        "evidence": {
            "monitor": monitor_result["evidence"],
            "gpu": gpu_block["evidence"],
            "display": {
                "source": display["source"],
                "query_ok": not bool(display_error),
                "query_error": display_error,
                "read_only": True,
            },
            "read_only": True,
            "mutation_attempted": False,
            "mutation_apis": [],
        },
    }
    return result


def get_display_status() -> dict[str, Any]:
    """Public endpoint-friendly alias for :func:`collect_display_status`."""

    return collect_display_status()


detect_display_status = collect_display_status
inspect_display = collect_display_status


__all__ = [
    "MonitorDisableStatus",
    "ALL_PRESENT_PHYSICAL_MONITORS_DISABLED",
    "PARTIAL",
    "NEEDS_DISABLE",
    "UNKNOWN",
    "NOT_APPLICABLE",
    "evaluate_monitor_disable_status",
    "classify_monitor_inventory",
    "classify_monitor_status",
    "collect_display_status",
    "detect_display_status",
    "inspect_display",
    "get_display_status",
]
