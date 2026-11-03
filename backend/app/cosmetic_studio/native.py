"""Small, ownership-oriented Windows process identity helpers.

The module is importable on every platform, but native operations are refused
unless running on Windows.  It deliberately has no process discovery, launch,
termination, DLL loading, event signalling, or remote-memory operations.
"""

from __future__ import annotations

import ctypes
import os
import sys
import threading
from ctypes import wintypes
from typing import Any, Callable

PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
SYNCHRONIZE = 0x00100000
WAIT_OBJECT_0 = 0
WAIT_TIMEOUT = 0x102
WAIT_FAILED = 0xFFFFFFFF
OPEN_EXISTING = 3
FILE_SHARE_READ = 1
FILE_SHARE_WRITE = 2
FILE_SHARE_DELETE = 4
FILE_ATTRIBUTE_NORMAL = 0x80
FILE_FLAG_BACKUP_SEMANTICS = 0x02000000
INVALID_HANDLE_VALUE = ctypes.c_void_p(-1).value


class NativeUnavailableError(RuntimeError):
    """The requested Windows-only operation is unavailable."""


class NativeCallError(RuntimeError):
    """A required native call failed; ``winerror`` is captured at the call."""

    def __init__(self, operation: str, winerror: int | None = None):
        self.operation, self.winerror = operation, winerror
        super().__init__(f"{operation} failed" + (f" (winerror {winerror})" if winerror is not None else ""))


def _require_windows() -> None:
    if sys.platform != "win32":
        raise NativeUnavailableError("WINDOWS_NATIVE_API_REQUIRED")


_libraries: tuple[Any, Any] | None = None


def _api() -> tuple[Any, Any]:
    global _libraries
    _require_windows()
    if _libraries is None:
        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        shell32 = ctypes.WinDLL("shell32", use_last_error=True)
        kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
        kernel32.CloseHandle.restype = wintypes.BOOL
        kernel32.LocalFree.argtypes = [ctypes.c_void_p]
        kernel32.LocalFree.restype = ctypes.c_void_p
        kernel32.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
        kernel32.WaitForSingleObject.restype = wintypes.DWORD
        _libraries = kernel32, shell32
    return _libraries


def _last_error() -> int:
    """Read immediately after a failed ctypes call."""
    return int(ctypes.get_last_error())


class ClosedHandleError(RuntimeError):
    pass


class GuardedHandle:
    """A single native handle whose wrapper prevents use after close."""

    def __init__(self, handle: Any, close_fn: Callable[[Any], Any] | None = None):
        self.handle = handle
        self._close_fn = close_fn
        self.closed = False
        self._lock = threading.RLock()

    def call(self, function: Callable[..., Any], *args: Any) -> Any:
        with self._lock:
            if self.closed:
                raise ClosedHandleError("NATIVE_HANDLE_CLOSED")
            return function(self.handle, *args)

    def close(self) -> bool:
        with self._lock:
            if self.closed:
                return True
            close_fn = self._close_fn or _api()[0].CloseHandle
            if not close_fn(self.handle):
                return False
            self.closed = True
            self.handle = None
            return True


def _filetime_value(value: wintypes.FILETIME) -> int:
    return (int(value.dwHighDateTime) << 32) | int(value.dwLowDateTime)


def comparable_path(path: str) -> str:
    if not isinstance(path, str) or not path:
        return ""
    value = path.replace("\\", "/")
    if value.startswith("//?/"):
        value = value[4:]
    return os.path.normcase(value).rstrip("/")


def final_path(path: str) -> str:
    _require_windows()
    kernel32, _ = _api()
    kernel32.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, ctypes.c_void_p, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
    kernel32.CreateFileW.restype = wintypes.HANDLE
    kernel32.GetFinalPathNameByHandleW.argtypes = [wintypes.HANDLE, wintypes.LPWSTR, wintypes.DWORD, wintypes.DWORD]
    kernel32.GetFinalPathNameByHandleW.restype = wintypes.DWORD
    handle = kernel32.CreateFileW(path, 0, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, None, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL | FILE_FLAG_BACKUP_SEMANTICS, None)
    if handle in (None, 0, INVALID_HANDLE_VALUE):
        error = _last_error()
        raise NativeCallError("CreateFileW", error)
    try:
        size = 32768
        buffer = ctypes.create_unicode_buffer(size)
        length = kernel32.GetFinalPathNameByHandleW(handle, buffer, size, 0)
        if not length:
            error = _last_error()
            raise NativeCallError("GetFinalPathNameByHandleW", error)
        if length >= size:
            raise NativeCallError("GetFinalPathNameByHandleW", 122)
        return buffer.value
    finally:
        if not kernel32.CloseHandle(handle):
            error = _last_error()
            raise NativeCallError("CloseHandle(file)", error)


def read_process_identity(pid: int) -> tuple[dict[str, Any], GuardedHandle]:
    _require_windows()
    if type(pid) is not int or pid <= 0:
        raise ValueError("PID_MUST_BE_POSITIVE_INT")
    kernel32, _ = _api()
    kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel32.OpenProcess.restype = wintypes.HANDLE
    kernel32.GetProcessTimes.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4
    kernel32.GetProcessTimes.restype = wintypes.BOOL
    kernel32.QueryFullProcessImageNameW.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.LPWSTR, ctypes.POINTER(wintypes.DWORD)]
    kernel32.QueryFullProcessImageNameW.restype = wintypes.BOOL
    raw = kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, False, pid)
    if raw in (None, 0, INVALID_HANDLE_VALUE):
        error = _last_error()
        raise NativeCallError("OpenProcess", error)
    handle = GuardedHandle(raw, kernel32.CloseHandle)
    try:
        creation, exit_time, kernel_time, user_time = (wintypes.FILETIME() for _ in range(4))
        if not handle.call(kernel32.GetProcessTimes, ctypes.byref(creation), ctypes.byref(exit_time), ctypes.byref(kernel_time), ctypes.byref(user_time)):
            error = _last_error()
            raise NativeCallError("GetProcessTimes", error)
        length = wintypes.DWORD(32768)
        buffer = ctypes.create_unicode_buffer(length.value)
        if not handle.call(kernel32.QueryFullProcessImageNameW, 0, buffer, ctypes.byref(length)):
            error = _last_error()
            raise NativeCallError("QueryFullProcessImageNameW", error)
        exe_final = final_path(buffer.value)
        return {"pid": pid, "creation_filetime": _filetime_value(creation), "exe_final": exe_final}, handle
    except Exception:
        handle.close()
        raise


def parse_command_line(text: str) -> list[str]:
    _require_windows()
    if not isinstance(text, str):
        raise TypeError("COMMAND_LINE_MUST_BE_STRING")
    _, shell32 = _api()
    shell32.CommandLineToArgvW.argtypes = [wintypes.LPWSTR, ctypes.POINTER(ctypes.c_int)]
    shell32.CommandLineToArgvW.restype = ctypes.POINTER(wintypes.LPWSTR)
    argc = ctypes.c_int()
    pointer = shell32.CommandLineToArgvW(text, ctypes.byref(argc))
    if not pointer:
        error = _last_error()
        raise NativeCallError("CommandLineToArgvW", error)
    try:
        return [pointer[i] for i in range(argc.value)]
    finally:
        if _api()[0].LocalFree(pointer):
            error = _last_error()
            raise NativeCallError("LocalFree", error)


def is_alive(handle: GuardedHandle) -> bool:
    _require_windows()
    result = int(handle.call(_api()[0].WaitForSingleObject, 0))
    if result == WAIT_TIMEOUT:
        return True
    if result == WAIT_OBJECT_0:
        return False
    if result == WAIT_FAILED:
        error = _last_error()
        raise NativeCallError("WaitForSingleObject", error)
    raise NativeCallError("WaitForSingleObject", result)
