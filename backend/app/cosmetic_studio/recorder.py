"""Fresh portable OBS config; capture via Windows Graphics Capture, not injection."""
import configparser
import json
import math
from pathlib import Path
import shutil
import subprocess
import threading
import time
import uuid

from . import native
from .provider import CosmeticError
from .runtime import PINS, digest, powershell, processes, write_json, terminate_owned


def write_ini(path: Path, sections: dict):
    config = configparser.ConfigParser(interpolation=None)
    config.optionxform = str
    config.read_dict(sections)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8") as stream:
        config.write(stream, space_around_delimiters=False)


def scene_config(script: Path):
    source_id = str(uuid.uuid4())
    return {"name": "Cosmetic", "current_scene": "Cosmetic", "current_program_scene": "Cosmetic",
            "sources": [
                {"name": "Offline CS2 Demo", "uuid": source_id, "id": "window_capture", "versioned_id": "window_capture",
                 "settings": {"window": "Counter-Strike 2:SDL_app:cs2.exe", "priority": 0, "method": 2,
                              "cursor": False, "force_sdr": True, "client_area": True,
                              # OBS 32's window-source audio follows this window;
                              # never add desktop loopback or microphone sources.
                              "capture_audio": True}, "volume": 1.0, "mixers": 1},
                {"name": "Cosmetic", "id": "scene", "versioned_id": "scene", "uuid": str(uuid.uuid4()),
                 "settings": {"items": [{"name": "Offline CS2 Demo", "source_uuid": source_id, "visible": True,
                                         "locked": True, "id": 1, "pos": {"x": 0., "y": 0.}, "scale": {"x": 1., "y": 1.},
                                         "align": 5, "bounds_type": 0}], "id_counter": 1}}],
            "scene_order": [{"name": "Cosmetic"}], "resolution": {"x": 1280, "y": 720},
            "modules": {"scripts-tool": [{"path": script.as_posix(), "settings": {}}]}, "version": 2}


class IsolatedRecorder:
    def __init__(self, session):
        self.session = session
        self.root = session.root
        self.portable = self.root / "obs-portable"
        self.exe = self.portable / "bin/64bit/obs64.exe"
        self.handle = self.identity = self.child = None
        self._lease_stop = threading.Event()
        self._thread = None

    def prepare(self):
        if any(p["Name"].lower() == "obs64.exe" for p in processes()):
            raise CosmeticError("OBS 正在运行；实验不会复用或停止你的录制")
        source = self.session.paths["obs_exe"].parents[2]
        if digest(self.session.paths["obs_exe"]) != PINS["obs_exe"]:
            raise CosmeticError("OBS 版本尚未验证")
        self.portable.mkdir(exist_ok=False)
        for name in ("bin", "data", "obs-plugins"):
            shutil.copytree(source / name, self.portable / name)
        if digest(self.exe) != PINS["obs_exe"]:
            raise CosmeticError("OBS 副本校验失败")
        # No normal user config or external plugin directory is copied.
        # Bundled-only still requires removing known vendor plugin from this copy.
        vendor = self.portable / "obs-plugins/64bit/logi_obs_plugin_x64.dll"
        if vendor.exists():
            vendor.rename(self.root / "backup/logi_obs_plugin_x64.dll")
        cfg = self.portable / "config/obs-studio"
        write_ini(cfg / "global.ini", {"General": {"EnableAutoUpdates": "false", "MaxLogs": "10"}, "Video": {"Renderer": "Direct3D 11"}})
        write_ini(cfg / "user.ini", {"General": {"FirstRun": "true", "ConfirmOnExit": "false"},
                                     "Basic": {"Profile": "Cosmetic", "ProfileDir": "Cosmetic", "SceneCollection": "Cosmetic", "SceneCollectionFile": "Cosmetic"},
                                     "BasicWindow": {"SysTrayEnabled": "false", "PreviewEnabled": "true"}})
        write_ini(cfg / "basic/profiles/Cosmetic/basic.ini", {
            "General": {"Name": "Cosmetic"}, "Output": {"Mode": "Simple", "FilenameFormatting": "cosmetic-%CCYY-%MM-%DD-%hh-%mm-%ss"},
            "SimpleOutput": {"FilePath": (self.root / "evidence").as_posix(), "RecFormat2": "mkv", "RecQuality": "HQ", "RecEncoder": "x264", "VBitrate": "6000"},
            "Video": {"BaseCX": "1280", "BaseCY": "720", "OutputCX": "1280", "OutputCY": "720", "FPSType": "0", "FPSCommon": "30"},
            "Audio": {"SampleRate": "48000", "ChannelSetup": "Stereo"}})
        script = Path(__file__).with_name("obs-control.lua").read_text(encoding="utf-8")
        script_path = self.root / "obs-control.lua"
        script_path.write_text(script.replace("__COSMETIC_ROOT__", json.dumps(self.root.as_posix(), ensure_ascii=False)), encoding="utf-8")
        (cfg / "basic/scenes").mkdir(parents=True)
        write_json(cfg / "basic/scenes/Cosmetic.json", scene_config(script_path))
        (cfg / "plugin_config/obs-websocket").mkdir(parents=True)
        write_json(cfg / "plugin_config/obs-websocket/config.json", {"server_enabled": False, "alerts_enabled": False, "first_load": False})
        self.script_hash = digest(script_path)

    def launch(self):
        self.session.validate_context(paused=True)
        if any(p["Name"].lower() == "obs64.exe" for p in processes()):
            raise CosmeticError("发现已有 OBS，会话停止")
        if digest(self.exe) != PINS["obs_exe"] or digest(self.root / "obs-control.lua") != self.script_hash:
            raise CosmeticError("录制组件已变化")
        args = [str(self.exe), "--portable", "--only-bundled-plugins", "--disable-updater", "--profile", "Cosmetic", "--collection", "Cosmetic"]
        self.child = subprocess.Popen(args, cwd=self.exe.parent, env=self.session.env, creationflags=subprocess.CREATE_NO_WINDOW)
        self.identity, self.handle = native.read_process_identity(self.child.pid)
        if native.comparable_path(self.identity["exe_final"]) != native.comparable_path(native.final_path(str(self.exe))):
            raise CosmeticError("录制进程身份不匹配")
        write_json(self.root / "evidence/obs-owner.json", self.identity)
        self.session.wait_for(self.status, timeout=30)
        self.wait_capture_ready()

    def wait_capture_ready(self):
        first = None
        def ready():
            nonlocal first
            self.session.validate_context(paused=True)
            row = self.status()
            if not row or row.get("width") != 1280 or row.get("height") != 720:
                first = None
                return False
            if first is None:
                first = row["time"]
            return row["time"] - first >= 2
        # OBS process startup is not capture startup. Keep the reviewed Demo
        # paused until WGC has the expected source size and fresh settled frames.
        self.session.wait_for(ready, timeout=15)

    def status(self):
        if self.handle and not native.is_alive(self.handle):
            raise CosmeticError("录制进程意外退出")
        try:
            # Only newline-committed records are published. Lua never truncates
            # a status the host may be reading. A partial newest write cannot
            # erase the preceding sample; its original freshness still applies.
            with (self.root / "evidence/obs-status.jsonl").open("rb") as stream:
                stream.seek(max(0, stream.seek(0, 2) - 4096))
                data = stream.read()
            complete = data.split(b"\n")[:-1]
            if not complete:
                return None
            value = json.loads(complete[-1])
            stamp = value.get("time")
            if (isinstance(stamp, bool) or not isinstance(stamp, (int, float)) or not math.isfinite(stamp)
                    or not 0 <= time.time() - stamp <= 2
                    or any(type(value.get(key)) is not bool for key in ("active", "requested", "stopped"))):
                return None
            return value
        except (OSError, ValueError, KeyError, AttributeError, TypeError):
            return None

    def _heartbeat(self):
        while not self._lease_stop.is_set():
            try:
                lease = self.root / "requests/obs-lease.txt"
                pending = lease.with_suffix(".pending")
                pending.write_text(str(int(time.time())), encoding="ascii")
                pending.replace(lease)
            except OSError:
                # A Windows reader may briefly deny replacement. Retry at the
                # next cadence; never renew an old timestamp on the Lua side.
                # Persistent failure still expires its three-second lease.
                pass
            self._lease_stop.wait(.2)

    def record(self):
        self.session.validate_context(paused=True)
        self._thread = threading.Thread(target=self._heartbeat, daemon=True)
        self._thread.start()
        (self.root / "requests/obs-start.flag").touch(exist_ok=False)
        try:
            def active():
                self.session.validate_context(paused=True)
                row = self.status()
                if row and row.get("stopped"):
                    raise CosmeticError("录制提前停止")
                return row and row.get("active") is True
            self.session.wait_for(active, timeout=10)
            def monitor():
                row = self.status()
                if not row or row.get("active") is not True or row.get("stopped") is not False:
                    raise CosmeticError("录制状态未知或提前停止")
            self.session.operation("record", monitor=monitor)
        finally:
            self.stop_recording()
        files = list((self.root / "evidence").glob("cosmetic-*.mkv"))
        if len(files) != 1 or files[0].stat().st_size < 4096:
            raise CosmeticError("未得到唯一完整的录制文件")
        return files[0]

    def stop_recording(self):
        (self.root / "requests/obs-stop.flag").touch(exist_ok=True)
        self._lease_stop.set()
        if self._thread:
            self._thread.join(timeout=2)
        if not self.child:
            return
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            row = self.status()
            if row and row.get("stopped") is True and row.get("active") is False:
                return
            time.sleep(.2)
        raise CosmeticError("无法确认录制已停止；文件不标记成功")

    def close(self):
        if not self.child:
            return
        try:
            if not self.handle and self.child.poll() is None:
                # Popen retains the exact created process handle even when a
                # later identity-query failed. Never use a discovered PID.
                self.child.terminate()
                self.child.wait(timeout=5)
                raise CosmeticError("OBS 启动身份检查失败，已结束本次新建的录制进程")
            if self.handle and native.is_alive(self.handle):
                try:
                    self.stop_recording()
                    identity, check = native.read_process_identity(self.identity["pid"])
                    try:
                        if identity != self.identity:
                            raise CosmeticError("OBS 身份变化")
                        powershell(f"(Get-Process -Id {identity['pid']}).CloseMainWindow()", env=self.session.env)
                    finally:
                        check.close()
                    self.child.wait(timeout=8)
                except Exception:
                    if native.is_alive(self.handle):
                        terminate_owned(self.identity)
                    raise
        finally:
            if self.handle and not native.is_alive(self.handle):
                self.handle.close()
