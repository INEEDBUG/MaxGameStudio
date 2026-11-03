"""Execute the shipped OBS Lua controller in a private Lua 5.1 VM.

No OBS process is loaded: only the standard Lua interpreter DLL is used and
the obslua API is a small in-VM mock.
"""

import ctypes
import hashlib
from pathlib import Path

import pytest


LUA_DLL = Path(r"D:\SettingTools\OBSstudio\bin\64bit\lua51.dll")
LUA_SHA256 = "182dcd9bf0e5d35e870227a1a1a1e760cac9dd1f6a257b2e6cb60bed1b619df5"
SCRIPT = Path(__file__).parents[1] / "app/cosmetic_studio/obs-control.lua"


class Lua:
    def __init__(self, source):
        if not LUA_DLL.is_file() or hashlib.sha256(LUA_DLL.read_bytes()).hexdigest() != LUA_SHA256:
            pytest.skip("qualified Lua 5.1 DLL unavailable")
        self.lib = ctypes.CDLL(str(LUA_DLL))
        p = ctypes.c_void_p
        self.lib.luaL_newstate.restype = p
        self.lib.luaL_openlibs.argtypes = [p]
        self.lib.luaL_loadstring.argtypes = [p, ctypes.c_char_p]
        self.lib.luaL_loadstring.restype = ctypes.c_int
        self.lib.lua_pcall.argtypes = [p, ctypes.c_int, ctypes.c_int, ctypes.c_int]
        self.lib.lua_pcall.restype = ctypes.c_int
        self.lib.lua_settop.argtypes = [p, ctypes.c_int]
        self.lib.lua_getfield.argtypes = [p, ctypes.c_int, ctypes.c_char_p]
        self.lib.lua_type.argtypes = [p, ctypes.c_int]
        self.lib.lua_type.restype = ctypes.c_int
        self.lib.lua_toboolean.argtypes = [p, ctypes.c_int]
        self.lib.lua_toboolean.restype = ctypes.c_int
        self.lib.lua_tointeger.argtypes = [p, ctypes.c_int]
        self.lib.lua_tointeger.restype = ctypes.c_longlong
        self.lib.lua_tolstring.argtypes = [p, ctypes.c_int, ctypes.POINTER(ctypes.c_size_t)]
        self.lib.lua_tolstring.restype = ctypes.c_char_p
        self.lib.lua_close.argtypes = [p]
        self.L = self.lib.luaL_newstate()
        self.lib.luaL_openlibs(self.L)
        self.exec(source)

    def exec(self, text):
        data = text.encode()
        assert self.lib.luaL_loadstring(self.L, data) == 0
        assert self.lib.lua_pcall(self.L, 0, 0, 0) == 0

    def call(self, name, results=0):
        # LUA_GLOBALSINDEX for Lua 5.1.
        self.lib.lua_getfield(self.L, -10002, name.encode())
        rc = self.lib.lua_pcall(self.L, 0, results, 0)
        if rc:
            n = ctypes.c_size_t()
            msg = self.lib.lua_tolstring(self.L, -1, ctypes.byref(n))
            raise AssertionError((rc, msg[:n.value].decode(errors="replace") if msg else "<no error>"))
        if results:
            vals = [bool(self.lib.lua_toboolean(self.L, i - results + 1)) if self.lib.lua_type(self.L, i - results + 1) == 1
                    else int(self.lib.lua_tointeger(self.L, i - results + 1)) for i in range(results)]
            self.lib.lua_settop(self.L, 0)
            return vals

    def __del__(self):
        if getattr(self, "L", None):
            self.lib.lua_close(self.L)
            self.L = None


def harness(tmp_path):
    root = tmp_path.as_posix()
    (tmp_path / "requests").mkdir(parents=True)
    (tmp_path / "evidence").mkdir(parents=True)
    prelude = f'''\
MOCK_TIME=1000; ACTIVE=false; STARTS=0; STOPS=0
os.time=function() return MOCK_TIME end
obslua={{}}
function obslua.timer_add(f,ms) TIMER=f end
function obslua.timer_remove(f) TIMER=nil end
function obslua.obs_frontend_recording_active() return ACTIVE end
function obslua.obs_frontend_recording_start() ACTIVE=true;STARTS=STARTS+1 end
function obslua.obs_frontend_recording_stop() ACTIVE=false;STOPS=STOPS+1 end
function obslua.obs_get_source_by_name(n) return nil end
function obslua.obs_source_get_width(s) return 1280 end
function obslua.obs_source_get_height(s) return 720 end
function obslua.obs_source_release(s) end
'''
    script = SCRIPT.read_text(encoding="utf-8").replace("__COSMETIC_ROOT__", repr(root))
    vm = Lua(prelude + "\n" + script)
    vm.call("script_load")
    return vm


def set_lease(root, text):
    p = root / "requests/obs-lease.txt"
    if text is None:
        p.unlink(missing_ok=True)
    else:
        p.write_text(text, encoding="ascii")


def state(vm):
    vm.exec("return STARTS,STOPS,ACTIVE")
    # not used; state assertions use a harmless Lua assertion instead


def test_valid_lease_starts_recording_and_transient_missing_does_not_stop(tmp_path):
    vm = harness(tmp_path)
    (tmp_path / "requests/obs-start.flag").touch()
    set_lease(tmp_path, "1000")
    vm.call("TIMER")
    vm.exec("assert(STARTS==1 and STOPS==0 and ACTIVE==true)")
    set_lease(tmp_path, None)
    vm.exec("MOCK_TIME=1001")
    vm.call("TIMER")
    vm.exec("assert(STOPS==0 and ACTIVE==true)")


def test_missing_lease_past_grace_and_future_or_malformed_cannot_renew(tmp_path):
    vm = harness(tmp_path)
    (tmp_path / "requests/obs-start.flag").touch(); set_lease(tmp_path, "1000")
    vm.call("TIMER")
    set_lease(tmp_path, "not-a-time"); vm.exec("MOCK_TIME=1004"); vm.call("TIMER")
    vm.exec("assert(STOPS==1 and ACTIVE==false)")
    vm2 = harness(tmp_path / "future")
    root2 = tmp_path / "future"
    (root2 / "requests/obs-start.flag").touch(); set_lease(root2, "1000")
    vm2.call("TIMER"); set_lease(root2, "2000"); vm2.exec("MOCK_TIME=1004"); vm2.call("TIMER")
    vm2.exec("assert(STOPS==1 and ACTIVE==false)")


def test_deadline_stop_stopflag_and_unload(tmp_path):
    vm = harness(tmp_path)
    (tmp_path / "requests/obs-start.flag").touch(); set_lease(tmp_path, "1000")
    vm.call("TIMER"); vm.exec("MOCK_TIME=1041"); set_lease(tmp_path, "1041"); vm.call("TIMER")
    vm.exec("assert(STOPS==1 and ACTIVE==false)")

    vm2 = harness(tmp_path / "second"); root = tmp_path / "second"
    (root / "requests/obs-start.flag").touch(); set_lease(root, "1000"); vm2.call("TIMER")
    (root / "requests/obs-stop.flag").touch(); vm2.call("TIMER")
    vm2.exec("assert(STOPS==1 and ACTIVE==false)")
    vm3 = harness(tmp_path / "unload"); root3 = tmp_path / "unload"
    (root3 / "requests/obs-start.flag").touch(); set_lease(root3, "1000"); vm3.call("TIMER")
    vm3.call("script_unload")
    vm3.exec("assert(STOPS==1 and ACTIVE==false)")
