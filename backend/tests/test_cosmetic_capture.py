"""Window selection tests for capture; no windows are created or captured."""

import ctypes
from types import SimpleNamespace

import pytest

from app.cosmetic_studio.capture import _owned_window
from app.cosmetic_studio.provider import CosmeticError


class FakeUser32:
    def __init__(self, windows):
        self.windows = windows

    def EnumWindows(self, callback, _lparam):
        for row in self.windows:
            callback(row["hwnd"], 0)
        return 1

    def GetWindowThreadProcessId(self, hwnd, ptr):
        row = next(r for r in self.windows if r["hwnd"] == hwnd)
        ptr._obj.value = row["pid"]
        return 1

    def IsWindowVisible(self, hwnd):
        return next(r for r in self.windows if r["hwnd"] == hwnd)["visible"]

    def IsIconic(self, hwnd):
        return next(r for r in self.windows if r["hwnd"] == hwnd)["iconic"]

    def GetWindowRect(self, hwnd, rect):
        row = next(r for r in self.windows if r["hwnd"] == hwnd)
        out = ctypes.cast(rect, ctypes.POINTER(ctypes.wintypes.RECT)).contents
        out.left, out.top = 0, 0
        out.right, out.bottom = row["width"], row["height"]
        return 1


def win(hwnd, pid=7, width=1456, height=909, visible=True, iconic=False):
    return {"hwnd": hwnd, "pid": pid, "width": width, "height": height,
            "visible": visible, "iconic": iconic}


def test_owned_window_ignores_same_pid_helper_and_wrong_windows():
    api = FakeUser32([win(10, width=16, height=16), win(11, pid=99), win(12)])
    hwnd, width, height = _owned_window(api, 7)
    assert (hwnd, width, height) == (12, 1456, 909)


def test_owned_window_rejects_ambiguous_plausible_windows():
    with pytest.raises(CosmeticError, match="多个候选"):
        _owned_window(FakeUser32([win(10), win(11, width=1280, height=720)]), 7)


def test_owned_window_rejects_unknown_or_only_tiny_window():
    with pytest.raises(CosmeticError):
        _owned_window(FakeUser32([win(10, width=16, height=16)]), 7)
    with pytest.raises(CosmeticError):
        _owned_window(FakeUser32([win(10, visible=False)]), 7)
