"""Bounded Windows file-tool invocation; no attach, shell, or game injection.

A fresh kill-on-close Job Object owns the new core and its parser children.
The stdin handshake is withheld until assignment succeeds. Never kill by name.
"""
from __future__ import annotations

import ctypes
from ctypes import wintypes as w
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import time

from .provider import CosmeticError


class CoreCleanupError(CosmeticError):
    """Do not release the host runtime claim when child cleanup is unknown."""


class _BasicLimit(ctypes.Structure):
    _fields_ = [("process_time", ctypes.c_int64), ("job_time", ctypes.c_int64), ("flags", w.DWORD),
                ("min_ws", ctypes.c_size_t), ("max_ws", ctypes.c_size_t), ("process_limit", w.DWORD),
                ("affinity", ctypes.c_size_t), ("priority", w.DWORD), ("scheduling", w.DWORD)]


class _ExtendedLimit(ctypes.Structure):
    _fields_ = [("basic", _BasicLimit), ("io", ctypes.c_uint64 * 6),
                ("process_memory", ctypes.c_size_t), ("job_memory", ctypes.c_size_t),
                ("peak_process", ctypes.c_size_t), ("peak_job", ctypes.c_size_t)]


class _Accounting(ctypes.Structure):
    _fields_ = [("times", ctypes.c_int64 * 4), ("faults", w.DWORD), ("total", w.DWORD),
                ("active", w.DWORD), ("terminated", w.DWORD)]


class _Job:
    def __init__(self):
        if os.name != "nt":
            raise CosmeticError("CORE_REQUIRES_WINDOWS")
        self.api = ctypes.WinDLL("kernel32", use_last_error=True)
        signatures = {
            "CreateJobObjectW": ([ctypes.c_void_p, w.LPCWSTR], w.HANDLE),
            "SetInformationJobObject": ([w.HANDLE, ctypes.c_int, ctypes.c_void_p, w.DWORD], w.BOOL),
            "QueryInformationJobObject": ([w.HANDLE, ctypes.c_int, ctypes.c_void_p, w.DWORD, ctypes.c_void_p], w.BOOL),
            "AssignProcessToJobObject": ([w.HANDLE, w.HANDLE], w.BOOL),
            "TerminateJobObject": ([w.HANDLE, w.UINT], w.BOOL),
            "CloseHandle": ([w.HANDLE], w.BOOL),
        }
        for name, (args, result) in signatures.items():
            fn = getattr(self.api, name)
            fn.argtypes, fn.restype = args, result
        self.handle = self.api.CreateJobObjectW(None, None)
        if not self.handle:
            raise CosmeticError("CORE_JOB_CREATE_FAILED")
        limits = _ExtendedLimit()
        limits.basic.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        if not self.api.SetInformationJobObject(self.handle, 9, ctypes.byref(limits), ctypes.sizeof(limits)):
            self.close()
            raise CosmeticError("CORE_JOB_LIMIT_FAILED")

    def assign(self, child):
        # Pinned CPython 3.12 Windows Popen retains the exact process HANDLE.
        if not self.api.AssignProcessToJobObject(self.handle, int(child._handle)):
            raise CosmeticError("CORE_JOB_ASSIGN_FAILED")

    def stop_and_wait(self):
        if not self.api.TerminateJobObject(self.handle, 1):
            raise CosmeticError("CORE_JOB_STOP_FAILED")
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            info = _Accounting()
            if not self.api.QueryInformationJobObject(self.handle, 1, ctypes.byref(info), ctypes.sizeof(info), None):
                raise CosmeticError("CORE_JOB_QUERY_FAILED")
            if info.active == 0:
                return
            time.sleep(.05)
        raise CosmeticError("CORE_CHILDREN_EXIT_UNCONFIRMED")

    def close(self):
        if self.handle:
            self.api.CloseHandle(self.handle)
            self.handle = None


def core_environment(root: Path, python: Path, executable: Path) -> dict[str, str]:
    # Do not give a third-party tool unrelated API keys, session tokens, or
    # implicit reference-app development flags inherited from our backend.
    allowed = {"SYSTEMROOT", "WINDIR", "COMSPEC", "PATHEXT", "SYSTEMDRIVE", "NUMBER_OF_PROCESSORS", "PROCESSOR_ARCHITECTURE"}
    env = {k: v for k, v in os.environ.items() if k.upper() in allowed}
    env.update(TEMP=str(root / "temp"), TMP=str(root / "temp"),
               APPDATA=str(root / "profile"), LOCALAPPDATA=str(root / "profile"),
               USERPROFILE=str(root / "profile"), PYTHONDONTWRITEBYTECODE="1")
    system = Path(env.get("SystemRoot", env.get("SYSTEMROOT", r"C:\Windows"))) / "System32"
    env["PATH"] = os.pathsep.join(map(str, (python.parent, executable.parent, system)))
    return env


def run_core(executable: Path, args: list[str], packet: bytes, root: Path,
             env: dict[str, str], cancel: threading.Event, timeout: float = 45) -> tuple[int, str]:
    if cancel.is_set():
        raise CosmeticError("CORE_CANCELLED")
    job, child, assigned = _Job(), None, False
    # Temporary diagnostic bytes are never copied into application logs/reports.
    try:
        with tempfile.TemporaryFile(dir=root / "temp") as diagnostic:
            child = subprocess.Popen([str(executable), *args], cwd=root, env=env, stdin=subprocess.PIPE,
                stdout=subprocess.DEVNULL, stderr=diagnostic, creationflags=subprocess.CREATE_NO_WINDOW)
            job.assign(child)
            assigned = True
            try:
                child.stdin.write(packet)
                child.stdin.close()
            except BrokenPipeError:
                # Early authenticated-parent rejection can close stdin before
                # this small packet is flushed. Still collect only a bounded
                # error category and reap the exact job below.
                pass
            deadline = time.monotonic() + timeout
            while child.poll() is None:
                if cancel.is_set():
                    raise CosmeticError("CORE_CANCELLED")
                if time.monotonic() > deadline:
                    raise CosmeticError("CORE_TIMEOUT")
                if os.fstat(diagnostic.fileno()).st_size > 256 * 1024:
                    raise CosmeticError("CORE_DIAGNOSTIC_LIMIT")
                time.sleep(.05)
            if cancel.is_set():
                raise CosmeticError("CORE_CANCELLED")
            diagnostic.seek(max(0, os.fstat(diagnostic.fileno()).st_size - 4096))
            message = diagnostic.read(4096).decode("utf-8", errors="replace")
            # Return only the known classification, not third-party raw text.
            code = "CORE_PARENT_NOT_SUPPORTED" if "no ancestor PE SHA-256 in allowlist" in message else "CORE_PROCESS_FAILED"
            return child.returncode, code
    finally:
        try:
            if assigned:
                job.stop_and_wait()
            elif child and child.poll() is None:
                child.terminate()  # Exact Popen handle; no handshake was sent.
            if child:
                child.wait(timeout=5)
                if child.stdin and not child.stdin.closed:
                    try:
                        child.stdin.close()
                    except BrokenPipeError:
                        pass
        except Exception as exc:
            raise CoreCleanupError("CORE_CHILDREN_EXIT_UNCONFIRMED") from exc
        finally:
            job.close()
