"""Mock-only guards for the local Cosmetic Studio host lifecycle."""

import threading
from pathlib import Path

import pytest

from app.cosmetic_studio import runtime
from app.cosmetic_studio.models import JobRequest, ToolPaths


def job(tmp_path: Path) -> JobRequest:
    tools = ToolPaths(
        cs2_exe=r"C:\Games\Steam\cs2.exe", hlae_exe=r"C:\Games\HLAE\hlae.exe",
        nskinz_dll=r"C:\Games\Tools\nskinz.dll", obs_exe=r"C:\Games\Tools\obs.exe",
        ffmpeg_exe=r"C:\Games\Tools\ffmpeg.exe", ffprobe_exe=r"C:\Games\Tools\ffprobe.exe",
        output_dir=str(tmp_path),
    )
    return JobRequest(
        demo_path=r"C:\Demos\fixture.dem", xuid="76561199059074958",
        start_tick=256, end_tick=384, preset_id="awp-dragon-lore",
        tools=tools, acknowledge_offline=True,
    )


def test_console_parser_accepts_only_matching_session_records():
    text = (
        '09/12 20:00:00 noise {"session":"other","kind":"DEMO_SAMPLE"}\n'
        'MGS_TAG {"session":"MGS_TAG","kind":"DEMO_SAMPLE","valid":true}\n'
        'MGS_TAG {"session":"MGS_TAG","kind":"STOPPING","reason":"DEMO_LOST"}\n'
    )
    rows = runtime.parse_console(text, "MGS_TAG")
    assert [row["kind"] for row in rows] == ["DEMO_SAMPLE", "STOPPING"]


def test_profile_is_fixed_awp_and_exact_xuid():
    profile = runtime.profile_for("76561199059074958")
    assert profile["version"] == 5
    assert profile["profiles"][0]["base_item"] == 9
    assert profile["profiles"][0]["paint_kit"] == 344
    assert profile["profiles"][0]["xuid"] == 76561199059074958
    assert profile["knife_profiles"][0]["enabled"] is False


def test_native_stop_reports_verified_reason_without_accepting_old_sample(tmp_path, monkeypatch):
    session = runtime.NativeSession(job(tmp_path), tmp_path / "session", 4, threading.Event())
    monkeypatch.setattr(session, "assert_owner", lambda: None)
    monkeypatch.setattr(session, "records", lambda: [
        {"kind": "DEMO_SAMPLE", "valid": True},
        {"kind": "STOPPING", "reason": "RECORD_TARGET_CHANGED"},
    ])
    with pytest.raises(runtime.CosmeticError, match="录制期间观察目标或武器发生变化"):
        session.current(required=False)


def test_initial_loaded_flag_is_not_completed_demo_initialization():
    initial = {"phase": "IDLE", "paused": True, "tick": 1, "script_time_ms": 1000}
    assert not runtime.initial_demo_progressed(None, initial)
    assert not runtime.initial_demo_progressed(initial, initial)
    a = {"phase": "IDLE", "paused": False, "tick": 1040, "script_time_ms": 2000}
    assert not runtime.initial_demo_progressed(a, a)
    assert not runtime.initial_demo_progressed(a, {**a, "tick": 1056, "script_time_ms": 2100})
    assert runtime.initial_demo_progressed(a, {**a, "tick": 1056, "script_time_ms": 2300})


def test_unqualified_long_extension_path_is_rejected_without_migration():
    short = Path("D:/" + "a" * 94 + "/nsk.dll")
    assert len(str(short).encode("utf-16-le")) // 2 == 105
    runtime.qualify_extension_path(short)
    with pytest.raises(runtime.CosmeticError, match="长目录"):
        runtime.qualify_extension_path(Path(str(short) + "x"))


def test_prepare_refuses_non_windows_without_touching_processes(tmp_path, monkeypatch):
    session = runtime.NativeSession(job(tmp_path), tmp_path / "session", 4, threading.Event())
    monkeypatch.setattr(runtime.os, "name", "posix")
    with pytest.raises(runtime.CosmeticError, match="不支持的制片环境"):
        session.prepare()


def test_source_fingerprint_change_stops_before_tool_checks_or_files(tmp_path, monkeypatch):
    request=job(tmp_path).model_copy(update={'expected_demo_sha256':'a'*64})
    session=runtime.NativeSession(request,tmp_path/'session',4,threading.Event())
    monkeypatch.setattr(runtime,'qualify_extension_path',lambda path:None)
    monkeypatch.setattr(runtime,'processes',lambda:[{'Name':'steam.exe'}])
    monkeypatch.setattr(runtime,'digest',lambda path:'b'*64)
    monkeypatch.setattr(session,'check_pins',lambda:pytest.fail('Changed source must stop first'))
    with pytest.raises(runtime.CosmeticError,match='指纹不一致'):
        session.prepare()
    assert not session.root.exists()


def test_launch_pin_failure_happens_before_popen(tmp_path, monkeypatch):
    root = tmp_path / "session"
    (root / "requests").mkdir(parents=True)
    (root / "evidence").mkdir()
    session = runtime.NativeSession(job(tmp_path), root, 4, threading.Event())
    session.baseline = {"script_sha256": "script"}
    monkeypatch.setattr(runtime, "digest", lambda path: "script")
    monkeypatch.setattr(session, "check_pins", lambda: (_ for _ in ()).throw(runtime.CosmeticError("pin mismatch")))
    spawned = []
    monkeypatch.setattr(runtime.subprocess, "Popen", lambda *args, **kwargs: spawned.append(True))
    with pytest.raises(runtime.CosmeticError, match="pin mismatch"):
        session.launch()
    assert spawned == []


def test_operation_does_not_accept_old_phase_completion(tmp_path, monkeypatch):
    root = tmp_path / "session"
    (root / "requests").mkdir(parents=True)
    session = runtime.NativeSession(job(tmp_path), root, 4, threading.Event())
    samples = iter([
        {"kind": "DEMO_SAMPLE", "phase": "IDLE", "script_time_ms": 10},
        {"kind": "DEMO_SAMPLE", "phase": "PAUSING", "script_time_ms": 20},
    ])
    monkeypatch.setattr(session, "current", lambda **kwargs: next(samples))
    monkeypatch.setattr(session, "records", lambda: [{"kind": "OPERATION_COMPLETE", "operation": "position", "script_time_ms": 20}])
    def wait_for(predicate, timeout=45):
        assert predicate() is None
        raise runtime.CosmeticError("fresh IDLE sample required")
    monkeypatch.setattr(session, "wait_for", wait_for)
    with pytest.raises(runtime.CosmeticError, match="fresh IDLE sample"):
        session.operation("position")


def test_cancel_after_child_spawn_must_not_leave_unowned_child(tmp_path, monkeypatch):
    cancel = threading.Event()
    root = tmp_path / "session"
    (root / "requests").mkdir(parents=True)
    (root / "evidence").mkdir()
    session = runtime.NativeSession(job(tmp_path), root, 4, cancel)
    session.baseline = {"script_sha256": "script"}
    monkeypatch.setattr(runtime, "digest", lambda path: "script")
    monkeypatch.setattr(runtime, "processes", lambda: [])
    monkeypatch.setattr(session, "check_pins", lambda: None)
    monkeypatch.setattr(session, "argv", lambda: [r"C:\Games\Steam\cs2.exe"])

    class FakeChild:
        pid = 4321
        terminated = False

        def terminate(self):
            self.terminated = True

        def poll(self):
            return None if not self.terminated else 0

        def wait(self, timeout=None):
            return 0

    child = FakeChild()

    def spawn(*args, **kwargs):
        cancel.set()
        return child

    monkeypatch.setattr(runtime.subprocess, "Popen", spawn)
    monkeypatch.setattr(runtime.threading, "Thread", lambda **kwargs: type("T", (), {"start": lambda self: None, "join": lambda self, timeout=None: None})())
    monkeypatch.setattr(session, "audit_restore", lambda: None)
    with pytest.raises(runtime.CosmeticError, match="任务已取消"):
        session.launch()
    assert child.terminated, "cancellation must reap a child created before identity binding"


def test_stop_must_retain_handle_when_owned_process_is_still_live(tmp_path, monkeypatch):
    root = tmp_path / "session"
    (root / "requests").mkdir(parents=True)
    session = runtime.NativeSession(job(tmp_path), root, 4, threading.Event())
    session.child = type("Child", (), {"poll": lambda self: 0, "wait": lambda self, timeout=None: 0})()
    session.handle = type("Handle", (), {"closed": False, "close": lambda self: setattr(self, "closed", True) or True})()
    session.identity = {"pid": 4321, "creation_filetime": 1, "exe_final": r"C:\Tools\cs2.exe"}
    ticks = iter([0, 16])
    monkeypatch.setattr(runtime.time, "monotonic", lambda: next(ticks, 16))
    monkeypatch.setattr(runtime.native, "is_alive", lambda handle: True)
    monkeypatch.setattr(session, "assert_owner", lambda: None)
    monkeypatch.setattr(runtime, "terminate_owned", lambda identity: (_ for _ in ()).throw(runtime.CosmeticError("still live")))
    with pytest.raises(runtime.CosmeticError, match="still live"):
        session.stop()
    assert not session.handle.closed, "a live process must retain its identity handle for safe follow-up"


def _startup_gate_session(tmp_path, monkeypatch, log_text, samples):
    root = tmp_path / "session"
    (root / "requests").mkdir(parents=True)
    (root / "evidence").mkdir()
    extension = root / "extension"
    extension.mkdir()
    (extension / "nSkinzCS2.log").write_text(log_text, encoding="utf-8")
    session = runtime.NativeSession(job(tmp_path), root, 4, threading.Event())
    hook = runtime.native.comparable_path(str(session.paths["hook"]))
    dll = runtime.native.comparable_path(str(session.dll))
    monkeypatch.setattr(session, "check_pins", lambda: None)
    monkeypatch.setattr(session, "validate_context", lambda **kwargs: None)
    module_calls = []
    def module_paths():
        module_calls.append(True)
        return {hook} if len(module_calls) == 1 else {hook, dll}
    monkeypatch.setattr(session, "module_paths", module_paths)
    monkeypatch.setattr(runtime, "digest", lambda path: runtime.PINS["nskinz_dll"])
    monkeypatch.setattr(session, "current", lambda **kwargs: samples[-1] if len(samples) == 1 else samples.pop(0))
    operations = []
    monkeypatch.setattr(session, "operation", lambda name: operations.append(name))

    waits = []
    def wait_for(predicate, timeout=45):
        waits.append(timeout)
        if len(waits) == 1:
            assert predicate() is True
            return True
        for _ in range(3):
            if predicate():
                return True
        raise runtime.CosmeticError("extension startup gate timeout")
    monkeypatch.setattr(session, "wait_for", wait_for)
    return session, operations, waits


def test_load_extension_module_visible_without_bootstrap_does_not_pass(tmp_path, monkeypatch):
    session, operations, waits = _startup_gate_session(
        tmp_path, monkeypatch, "module visible\n", [{"script_time_ms": 1000}]
    )
    with pytest.raises(runtime.CosmeticError, match="startup gate timeout"):
        session.load_extension()
    assert operations == ["load"]
    assert waits == [15, 15]
    assert session.root.joinpath("evidence/extension-startup.json").exists() is False


def test_load_extension_bootstrap_with_insufficient_freshness_does_not_pass(tmp_path, monkeypatch):
    session, operations, waits = _startup_gate_session(
        tmp_path, monkeypatch, "nSkinzCS2 bootstrap completed.\n",
        [{"script_time_ms": 1000}, {"script_time_ms": 2999}],
    )
    with pytest.raises(runtime.CosmeticError, match="startup gate timeout"):
        session.load_extension()
    assert operations == ["load"]
    assert waits == [15, 15]
    assert session.root.joinpath("evidence/extension-startup.json").exists() is False


def test_load_extension_requires_fresh_two_second_sample_and_calls_verify_context(tmp_path, monkeypatch):
    session, operations, waits = _startup_gate_session(
        tmp_path, monkeypatch, "nSkinzCS2 bootstrap completed.\n",
        [{"script_time_ms": 1000}, {"script_time_ms": 3000}],
    )
    checks = []
    monkeypatch.setattr(session, "validate_context", lambda **kwargs: checks.append(kwargs))
    session.load_extension()
    assert operations == ["load"]
    assert waits == [15, 15]
    assert checks and all(item == {"paused": True} for item in checks)
    evidence = runtime.json.loads(session.root.joinpath("evidence/extension-startup.json").read_text(encoding="utf-8"))
    assert evidence["bootstrap_seen"] is True
    assert evidence["last"]["script_time_ms"] - evidence["first"]["script_time_ms"] == 2000
