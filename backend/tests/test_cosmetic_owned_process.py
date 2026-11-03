"""Windows ownership checks with our Python fixtures only, never a game/core."""
import ctypes
from ctypes import wintypes
import os
import sys
import threading
from pathlib import Path

import pytest

from app.cosmetic_studio.owned_process import core_environment, run_core
from app.cosmetic_studio.provider import CosmeticError

pytestmark = pytest.mark.skipif(os.name != "nt", reason="Windows Job Object contract")


def setup(tmp_path):
    (tmp_path / "temp").mkdir()
    (tmp_path / "profile").mkdir()
    python = Path(sys.executable)
    return python, core_environment(tmp_path, python, python)


def test_exact_process_receives_stdin_after_job_assignment(tmp_path):
    python, env = setup(tmp_path)
    code, _ = run_core(python, ["-c", "import sys; sys.exit(0 if sys.stdin.buffer.read()==b'fixture' else 4)"],
                       b"fixture", tmp_path, env, threading.Event(), timeout=5)
    assert code == 0


def test_timeout_reaps_owned_descendant(tmp_path):
    python, env = setup(tmp_path)
    pid_file = tmp_path / "child.pid"
    fixture = ("import pathlib,subprocess,sys,time; sys.stdin.buffer.read(); "
               "p=subprocess.Popen([sys.executable,'-c','import time; time.sleep(30)'],creationflags=0x08000000); "
               "pathlib.Path(sys.argv[1]).write_text(str(p.pid)); time.sleep(30)")
    with pytest.raises(CosmeticError, match="CORE_TIMEOUT"):
        run_core(python, ["-c", fixture, str(pid_file)], b"fixture", tmp_path, env, threading.Event(), timeout=2)
    pid = int(pid_file.read_text())
    api = ctypes.WinDLL("kernel32", use_last_error=True)
    api.OpenProcess.argtypes, api.OpenProcess.restype = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD], wintypes.HANDLE
    api.WaitForSingleObject.argtypes, api.WaitForSingleObject.restype = [wintypes.HANDLE, wintypes.DWORD], wintypes.DWORD
    api.CloseHandle.argtypes, api.CloseHandle.restype = [wintypes.HANDLE], wintypes.BOOL
    handle = api.OpenProcess(0x100000, False, pid)
    if handle:
        try:
            assert api.WaitForSingleObject(handle, 0) == 0  # Signaled, not still running.
        finally:
            api.CloseHandle(handle)


def test_pre_cancel_never_creates_job_or_child(tmp_path, monkeypatch):
    python, env = setup(tmp_path)
    cancel = threading.Event()
    cancel.set()
    monkeypatch.setattr("app.cosmetic_studio.owned_process._Job", lambda: pytest.fail("cancel must precede native creation"))
    with pytest.raises(CosmeticError, match="CORE_CANCELLED"):
        run_core(python, [], b"fixture", tmp_path, env, cancel)
