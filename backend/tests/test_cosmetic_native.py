import sys
from pathlib import Path

import pytest

from app.cosmetic_studio import native


def test_import_is_platform_safe_and_path_comparison():
    expected = native.comparable_path(r"C:\Tool\x.exe")
    assert native.comparable_path(r"\\?\C:\Tool\x.exe") == expected


def test_guarded_handle_is_idempotent_and_refuses_after_close():
    calls = []
    handle = native.GuardedHandle("token", lambda value: calls.append(value) or True)
    assert handle.call(lambda value: value) == "token"
    assert handle.close() is True
    assert handle.close() is True
    assert calls == ["token"]
    with pytest.raises(native.ClosedHandleError):
        handle.call(lambda value: value)


def test_alive_maps_wait_results_and_unknown_is_rejected(monkeypatch):
    monkeypatch.setattr(native, "_require_windows", lambda: None)
    monkeypatch.setattr(native, "_api", lambda: (type("K", (), {"WaitForSingleObject": lambda _, __, timeout: native.WAIT_TIMEOUT})(), None))
    assert native.is_alive(native.GuardedHandle("h")) is True


@pytest.mark.skipif(sys.platform != "win32", reason="requires Windows ctypes declarations")
def test_local_free_signature_is_pointer_safe():
    kernel32, _ = native._api()
    assert kernel32.LocalFree.argtypes == [native.ctypes.c_void_p]
    assert kernel32.LocalFree.restype is native.ctypes.c_void_p


@pytest.mark.skipif(sys.platform == "win32", reason="non-Windows refusal check")
def test_native_operations_refuse_non_windows():
    with pytest.raises(native.NativeUnavailableError):
        native.final_path("C:/not-used")
    with pytest.raises(native.NativeUnavailableError):
        native.read_process_identity(1)
    with pytest.raises(native.NativeUnavailableError):
        native.parse_command_line("tool.exe -insecure")


@pytest.mark.skipif(sys.platform != "win32", reason="requires Windows current Python only")
def test_current_python_identity_only():
    identity, handle = native.read_process_identity(__import__("os").getpid())
    try:
        assert identity["pid"] == __import__("os").getpid()
        assert identity["creation_filetime"] > 0
        assert native.comparable_path(identity["exe_final"])
        assert native.is_alive(handle)
    finally:
        assert handle.close()
