"""Capture only the owned CS2 HWND; no desktop capture or focus/input changes."""
import ctypes
from ctypes import wintypes
from pathlib import Path

from .provider import CosmeticError


def _owned_window(u32, pid_value: int):
    """Return the one plausible visible top-level window owned by pid."""
    hwnd_type = wintypes.HWND
    callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, hwnd_type, wintypes.LPARAM)
    candidates = []

    def visit(hwnd, _lparam):
        observed = wintypes.DWORD()
        if not u32.GetWindowThreadProcessId(hwnd, ctypes.byref(observed)) or observed.value != pid_value:
            return True
        if not u32.IsWindowVisible(hwnd) or u32.IsIconic(hwnd):
            return True
        rect = wintypes.RECT()
        if not u32.GetWindowRect(hwnd, ctypes.byref(rect)):
            return True
        width, height = rect.right - rect.left, rect.bottom - rect.top
        if 100 <= width <= 4000 and 100 <= height <= 3000:
            candidates.append((hwnd, width, height))
        return True

    callback = callback_type(visit)
    if not u32.EnumWindows(callback, 0):
        raise CosmeticError("无法枚举制片窗口")
    if len(candidates) != 1:
        raise CosmeticError("制片窗口不存在或存在多个候选窗口")
    return candidates[0]


def capture_preview(session, destination: Path):
    from PIL import Image
    session.validate_context(paused=True)
    u32 = ctypes.WinDLL("user32", use_last_error=True)
    g32 = ctypes.WinDLL("gdi32", use_last_error=True)
    u32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
    u32.GetWindowRect.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.RECT)]
    u32.GetDC.argtypes = [wintypes.HWND]
    u32.GetDC.restype = wintypes.HDC
    u32.ReleaseDC.argtypes = [wintypes.HWND, wintypes.HDC]
    u32.PrintWindow.argtypes = [wintypes.HWND, wintypes.HDC, wintypes.UINT]
    u32.PrintWindow.restype = wintypes.BOOL
    u32.EnumWindows.argtypes = [ctypes.c_void_p, wintypes.LPARAM]
    u32.EnumWindows.restype = wintypes.BOOL
    u32.IsWindowVisible.argtypes = [wintypes.HWND]
    u32.IsWindowVisible.restype = wintypes.BOOL
    u32.IsIconic.argtypes = [wintypes.HWND]
    u32.IsIconic.restype = wintypes.BOOL
    u32.SetThreadDpiAwarenessContext.argtypes = [ctypes.c_void_p]
    u32.SetThreadDpiAwarenessContext.restype = ctypes.c_void_p
    g32.CreateCompatibleDC.argtypes = [wintypes.HDC]
    g32.CreateCompatibleDC.restype = wintypes.HDC
    g32.CreateCompatibleBitmap.argtypes = [wintypes.HDC, ctypes.c_int, ctypes.c_int]
    g32.CreateCompatibleBitmap.restype = wintypes.HBITMAP
    g32.SelectObject.argtypes = [wintypes.HDC, wintypes.HANDLE]
    g32.SelectObject.restype = wintypes.HANDLE
    g32.GetDIBits.argtypes = [wintypes.HDC, wintypes.HBITMAP, wintypes.UINT, wintypes.UINT, ctypes.c_void_p, ctypes.c_void_p, wintypes.UINT]
    g32.DeleteObject.argtypes = [wintypes.HANDLE]
    g32.DeleteDC.argtypes = [wintypes.HDC]
    hwnd, width, height = _owned_window(u32, session.identity["pid"])
    pid = wintypes.DWORD()
    if not hwnd or not u32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid)) or pid.value != session.identity["pid"]:
        raise CosmeticError("预览窗口不属于此制片会话")
    old_dpi = u32.SetThreadDpiAwarenessContext(ctypes.c_void_p(-4))
    source = target = bitmap = old = None
    try:
        rect = wintypes.RECT()
        if not u32.GetWindowRect(hwnd, ctypes.byref(rect)):
            raise CosmeticError("无法读取预览尺寸")
        width, height = rect.right - rect.left, rect.bottom - rect.top
        if not 100 <= width <= 4000 or not 100 <= height <= 3000:
            raise CosmeticError("预览尺寸异常")
        source = u32.GetDC(hwnd)
        target = g32.CreateCompatibleDC(source)
        bitmap = g32.CreateCompatibleBitmap(source, width, height)
        if not source or not target or not bitmap:
            raise CosmeticError("无法创建预览缓冲区")
        old = g32.SelectObject(target, bitmap)
        if not old or not u32.PrintWindow(hwnd, target, 2):
            raise CosmeticError("原生窗口预览失败")
        g32.SelectObject(target, old)
        old = None
        # BITMAPINFOHEADER, top-down uncompressed BGRA.
        import struct
        header = ctypes.create_string_buffer(struct.pack("<IiiHHIIiiII", 40, width, -height, 1, 32, 0, width*height*4, 0, 0, 0, 0))
        pixels = ctypes.create_string_buffer(width*height*4)
        if g32.GetDIBits(source, bitmap, 0, height, pixels, header, 0) != height:
            raise CosmeticError("预览像素读取不完整")
        session.validate_context(paused=True)
        u32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
        if pid.value != session.identity["pid"]:
            raise CosmeticError("预览窗口身份已变化")
        image = Image.frombuffer("RGB", (width, height), pixels.raw, "raw", "BGRX", 0, 1)
        if max(high-low for low, high in image.getextrema()) < 8:
            raise CosmeticError("原生预览为空白；不能确认外观")
        with destination.open("xb") as stream:
            image.save(stream, format="PNG")
    finally:
        if old and target:
            g32.SelectObject(target, old)
        if bitmap:
            g32.DeleteObject(bitmap)
        if target:
            g32.DeleteDC(target)
        if source:
            u32.ReleaseDC(hwnd, source)
        if old_dpi:
            u32.SetThreadDpiAwarenessContext(old_dpi)
