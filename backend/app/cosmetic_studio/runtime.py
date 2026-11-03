"""Owned, fixed-version Windows offline Demo session. No attach or kill-by-name."""
from __future__ import annotations

import ctypes
from ctypes import wintypes
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import threading
import time

from . import native
from .models import JobRequest
from .catalog import PRESET_DATA, NATIVE_WEAPON_NAMES, native_weapon_matches, source_weapon_id
from .provider import CosmeticError

PINS = {
    "cs2_exe": "b99d7381a2ceb5bbdc2b0d32369618b82701436c5bb49a9e97f30bab6657adad",
    "client": "a0c195f0b6ec00915ef08c548200a010ebbe7982d3a4bc468cad939b67c8c4e3",
    "hlae_exe": "d7ba05e1e540d82ac485a9b39f13d5c4ccf9fd94f460e0c9ecf069b94c880a59",
    "hook": "adce361d0fa994d2e04627e70db8d8ddc6e8b3409237384fd7584a157ef0bd49",
    "nskinz_dll": "b2fa00a77fee32205b8f8e9936c4e82ae04ecc5aa9e881e2614cf44764b3735f",
    "obs_exe": "738bc532219cf526000b5448cb2efdb57e9129e25a6b5e6f0e0dbb95f4e3ec2e",
    "ffmpeg_exe": "1326dde4c84ff1f96fe6b8916c5bed29e163e9b5dccf995f6f3db069d143ec5e",
    "ffprobe_exe": "8251e4b57a49ba2d99d5216dc779fe41d711a6f7d8e1b5e8e59a11d76983b9cd",
}

# Keep complete qualified tuples: accepting individual hashes would accidentally
# permit a new game with an old hook. File provider only; nSkinz was not retested.
CURRENT_FILE_BUILD = {
    'cs2_exe': 'bb25389a10b381b0b50c984087f34e6b30f2919108751a0fdd97cf0dfc142c7b',
    'client': 'd7db25d48f1d10c5e0b0296e20ed803426eb9509da41760daeda39dd35ba89b9',
    'hlae_exe': 'c62d80943382537a6fa3daf0198f83a9d3f8125e555ccbe14bd82e1c83d031cb',
    'hook': '27d3298cd1063c7bd2fd1c43e23471816488dbbf668129ae7563fc459f708a08',
}

# Qualification bound, NOT a claimed upstream MAX_PATH limit. This pinned DLL
# repeatedly verified at105 UTF-16 units and failed at152 with identical input.
MAX_QUALIFIED_EXTENSION_PATH = 105


def qualify_extension_path(path: Path) -> None:
    if len(str(path).encode("utf-16-le")) // 2 > MAX_QUALIFIED_EXTENSION_PATH:
        raise CosmeticError("外观扩展尚未通过长目录验证，请选择较短的输出目录（例如 D:\\MGS）。不会自动迁移数据或启动游戏。")


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write_json(path: Path, value) -> None:
    with path.open("x", encoding="utf-8") as stream:
        json.dump(value, stream, ensure_ascii=False, indent=2)


def powershell(command: str, *, env=None) -> str:
    prefix = "$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); "
    result = subprocess.run(
        ["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", prefix + command],
        capture_output=True, check=True, timeout=20, env=env,
        creationflags=subprocess.CREATE_NO_WINDOW,
    )
    return result.stdout.decode("utf-8-sig").strip()


def processes() -> list[dict]:
    # Fixed read-only query, never shell-interpolated user input.
    raw = powershell("Get-CimInstance Win32_Process -Filter \"Name='cs2.exe' OR Name='HLAE.exe' OR Name='steam.exe' OR Name='obs64.exe' OR Name='rundll32.exe'\" | Select-Object Name,ProcessId,ParentProcessId,ExecutablePath,CommandLine | ConvertTo-Json -Compress")
    rows = json.loads(raw) if raw else []
    return rows if isinstance(rows, list) else [rows]


def parse_console(text: str, tag: str) -> list[dict]:
    rows, pending = [], ""
    for line in text.splitlines():
        line = re.sub(r"^\d{2}/\d{2} \d{2}:\d{2}:\d{2} ", "", line)
        pos = line.find(tag + " {")
        if pos >= 0:
            pending = line[pos + len(tag) + 1:]
        elif pending:
            pending += line
        if not pending or len(pending) > 8192:
            pending = ""
            continue
        try:
            row = json.loads(pending)
        except ValueError:
            continue
        pending = ""
        if isinstance(row, dict) and row.get("session") == tag:
            rows.append(row)
    return rows


def profile_for(xuid: str) -> dict:
    # Observed public configuration schema v5, no third-party core implementation.
    finish = {"enabled": False, "explicit_vanilla_finish": False, "mode": "finish_only",
              "model_definition": 0, "name": "Disabled", "paint_kit": 0, "quality": 0,
              "seed": 0, "wear": 0.0000001, "xuid": 0}
    return {"version": 5, "experimental_allow_hud_alias_synthetic_proxy_model_write": False,
            "profiles": [{"base_item": 9, "custom_name": "", "enabled": True,
                          "explicit_vanilla_finish": False, "name": "MGS offline AWP preview",
                          "override_item": 0, "paint_kit": 344, "quality": 0, "seed": 0,
                          "stat_trak": 0, "wear": 0.0000001, "xuid": int(xuid),
                          "stickers": [{"kit_id": 0, "rotation": 0.0, "scale": 1.0,
                                        "wear": 0.0000001} for _ in range(5)]}],
            "knife_profiles": [{**finish, "custom_name": "", "stat_trak": 0}],
            "glove_profiles": [finish],
            "keychain_profiles": [{"base_item": 7, "enabled": False, "explicit_clear": False,
                                   "keychain_kit": 0, "name": "Disabled", "xuid": 0}]}


def refresh_fields_ready(text: str, xuid: str) -> bool:
    """Pinned DLL diagnostics for this request only; NEVER a visual success ACK."""
    failures = ("Weapon appearance apply failed", "Rolled back weapon appearance",
                "Weapon appearance stage failed")
    if any(message in text for message in failures):
        raise CosmeticError("外观扩展报告应用失败或已回滚；已停止，不会录制原皮肤")
    syncs = list(re.finditer(r"Standard-weapon in-place attribute synchronization: xuid=" + re.escape(xuid)
                            + r", definition=9, paint=344,[^\r\n]*?verified=([01])(?:,|\.)", text))
    if any(sync.group(1) == "0" for sync in syncs):
        raise CosmeticError("外观扩展未通过属性校验；已停止制片")
    material = re.search(r"Standard-weapon native material initialization:[^\r\n]*paint=344,[^\r\n]*material_created=1(?:,|\.)", text)
    return bool(syncs and material and syncs[0].start() < material.start())


def refresh_armed(text: str) -> bool:
    return all(marker in text for marker in (
        "Runtime reloaded nSkinzCS2.json and queued Update",
        "FrameStageNotify hook became active",
        "Manual refresh rebuilding runtime entities",
        "Profiles: enabled=1",
    ))


def initial_demo_progressed(previous: dict | None, current: dict | None) -> bool:
    # The qualified build exposes tick1/paused while cold Demo initialization
    # is still pending. The successful fixture starts seeking only after normal
    # playback has advanced. Keep its conservative 1024-tick warmup for this MVP.
    return bool(previous and current and previous.get("paused") is False and current.get("paused") is False
                and previous.get("phase") == current.get("phase") == "IDLE"
                and previous.get("tick", -1) >= 1024 and current.get("tick", -1) > previous["tick"]
                and current.get("script_time_ms", 0) - previous.get("script_time_ms", 0) >= 250)


class NativeSession:
    def __init__(self, request: JobRequest, root: Path, slot: int, cancel: threading.Event):
        self.request, self.root, self.cancel = request, root, cancel
        self.file_mode = request.provider_id == "skin_core"
        self.paths = {key: Path(value) for key, value in request.tools.model_dump().items()
                      if value and key not in ("output_dir", "core_exe") and not (self.file_mode and key == "nskinz_dll")}
        self.game_dir = self.paths["cs2_exe"].parents[2] / "csgo"
        self.paths.update(client=self.game_dir / "bin/win64/client.dll",
                          hook=self.paths["hlae_exe"].parent / "x64/AfxHookSource2.dll")
        self.demo = Path(request.demo_path)
        self.console = self.game_dir / "console.log"
        self.dll = None if self.file_mode else root / "extension" / self.paths["nskinz_dll"].name
        self.profile = root / ("extension/appearance.json" if self.file_mode else "extension/nSkinzCS2.json")
        self.tag = "MGS_COSMETIC_" + root.name
        self.slot = slot
        self.child = None
        self.identity = None
        self.handle = None
        self.baseline = None
        self._lease_stop = threading.Event()
        self._lease_thread = None
        self._step = 0
        self._restored = False
        self.env = os.environ.copy()
        self.env.update(TEMP=str(root / "temp"), TMP=str(root / "temp"), SteamAppId="730", SteamGameId="730")

    def check_pins(self):
        expected_pins = PINS
        actual_build = {key: digest(self.paths[key]) for key in CURRENT_FILE_BUILD}
        if ((source_weapon_id(self.request.preset_id) == 507 or self.request.glove_preset_id)
                and (not self.file_mode or actual_build != CURRENT_FILE_BUILD)):
            raise CosmeticError('刀与手套仅支持已验证的当前 CS2/HLAE 版本组合')
        if self.file_mode and actual_build == CURRENT_FILE_BUILD:
            expected_pins = {**PINS, **CURRENT_FILE_BUILD}
        for key, expected in expected_pins.items():
            if self.file_mode and key == "nskinz_dll":
                continue
            path = self.paths[key]
            if digest(path) != expected:
                raise CosmeticError("工具版本未经此实验验证：" + key)
        if digest(self.demo) != self.demo_hash:
            raise CosmeticError("原始 Demo 已变化")

    def argv(self):
        return [str(self.paths["cs2_exe"]), "-steam", "-insecure", "-worldwide", "+sv_lan", "1",
                "-console", "-condebug", "-novid", "-sw", "-w", "1280", "-h", "720",
                "-afxDisableSteamStorage", "+mirv_script_load", str(self.root / "runtime.js"),
                "+playdemo", str(self.demo)]

    def prepare(self):
        if os.name != "nt" or not 1 <= self.slot <= 64:
            raise CosmeticError("不支持的制片环境或观察槽位")
        if not self.file_mode:
            qualify_extension_path(self.dll)
        if self.request.sky_id!='original':
            from .sky import available_skies
            if not any(row['id']==self.request.sky_id and row['available'] for row in available_skies(self.paths['cs2_exe'])):
                raise CosmeticError('此天空资源未安装，不会启动。')
        existing = processes()
        if any(p["Name"].lower() in ("cs2.exe", "hlae.exe", "obs64.exe") for p in existing):
            raise CosmeticError("请先自行退出 CS2、HLAE 和 OBS；实验不会接管已有进程")
        if not any(p["Name"].lower() == "steam.exe" for p in existing):
            raise CosmeticError("请先打开 Steam；本功能不会修改其离线、登录或启动设置")
        self.demo_hash = digest(self.demo)
        if self.request.expected_demo_sha256 and self.demo_hash != self.request.expected_demo_sha256:
            raise CosmeticError("Demo 与检查时的指纹不一致；请重新检查")
        self.check_pins()
        # Canonical local targets prevent junction aliases from weakening path checks.
        for key, value in list(self.paths.items()):
            resolved = native.final_path(str(value)).removeprefix("\\\\?\\")
            if resolved.startswith("UNC\\") or not re.match(r"^[A-Za-z]:[\\/]", resolved):
                raise CosmeticError("工具必须位于本地磁盘")
            self.paths[key] = Path(resolved)
        self.demo = Path(native.final_path(str(self.demo)).removeprefix("\\\\?\\"))
        if not re.match(r"^[A-Za-z]:[\\/]", str(self.demo)):
            raise CosmeticError("Demo 必须位于本地磁盘")
        for folder in ("backup", "evidence", "requests", "extension", "movie/cfg", "temp"):
            (self.root / folder).mkdir(parents=True, exist_ok=False)
        # Only ordinary game cfg and the active Steam library's user cfg subtree.
        steam = next(p for p in existing if p["Name"].lower() == "steam.exe")
        if not steam.get("ExecutablePath"):
            raise CosmeticError("无法确认 Steam 用户配置位置")
        user_root = Path(steam["ExecutablePath"]).parent / "userdata"
        roots = [self.game_dir / "cfg"] + list(user_root.glob("*/730/local/cfg"))
        cfg = []
        for directory in roots:
            for file in sorted(directory.rglob("*")):
                if file.is_file() and file.suffix.lower() in (".cfg", ".vcfg"):
                    backup = self.root / "backup" / f"cfg-{len(cfg):04d}{file.suffix}"
                    shutil.copy2(file, backup)
                    cfg.append({"path": str(file), "sha256": digest(file), "backup": str(backup)})
        console_before = None
        if self.console.exists():
            shutil.copy2(self.console, self.root / "backup/console.log")
            console_before = digest(self.console)
        if not self.file_mode:
            shutil.copy2(self.paths["nskinz_dll"], self.dll)
        # The reviewed single-DLL release creates its catalog from the user's
        # local CS2 VPK after loading. Do not distribute a game-asset catalog.
        write_json(self.profile, {"provider": "skin_core", "playback_sha256": self.demo_hash,
                   "xuid": self.request.xuid, "preset_id": self.request.preset_id,
                   "pattern_seed": self.request.pattern_seed, "wear": self.request.wear}
                   if self.file_mode else profile_for(self.request.xuid))
        config = {"root": self.root.as_posix(), "tag": self.tag, "demo": self.demo.as_posix(),
                  "xuid": self.request.xuid, "slot": self.slot, "dll": self.dll.as_posix() if self.dll else "",
                  "extensionMode": "file" if self.file_mode else "nskinz",
                  "weaponNames": list(NATIVE_WEAPON_NAMES[PRESET_DATA[self.request.preset_id][0]]),
                  "startTick": self.request.start_tick, "endTick": self.request.end_tick}
        if self.request.sky_id!='original':
            from .sky import sky_commands
            config['skyCommands']=sky_commands(self.request.sky_id)
        script = Path(__file__).with_suffix(".js").read_text(encoding="utf-8")
        (self.root / "runtime.js").write_text(script.replace("__COSMETIC_CONFIG__", json.dumps(config)), encoding="utf-8")
        self.baseline = {"cfg": cfg, "roots": [str(p) for p in roots], "console_sha256": console_before,
                         "demo_sha256": self.demo_hash, "profile_sha256": digest(self.profile),
                         "script_sha256": digest(self.root / "runtime.js"),
                         "pins": {key: digest(value) for key, value in self.paths.items()}}
        write_json(self.root / "evidence/baseline.json", self.baseline)

    def _heartbeat(self):
        sequence = 0
        while not self._lease_stop.is_set():
            try:
                (self.root / "requests" / f"lease-{sequence}.flag").touch(exist_ok=False)
                sequence += 1
            except OSError:
                return  # Native controller will expire the lease and quit.
            self._lease_stop.wait(.5)

    def launch(self):
        self.check_cancel()
        self.check_pins()
        if digest(self.root / "runtime.js") != self.baseline["script_sha256"]:
            raise CosmeticError("制片脚本已变化")
        if any(p["Name"].lower() in ("cs2.exe", "hlae.exe", "obs64.exe") for p in processes()):
            raise CosmeticError("检测到其他游戏或录制进程，已停止启动")
        self._lease_thread = threading.Thread(target=self._heartbeat, daemon=True)
        self._lease_thread.start()
        args = [str(self.paths["hlae_exe"]), "-noConfig", "-customLoader", "-noGui", "-autoStart",
                "-hookDllPath", str(self.paths["hook"]), "-programPath", str(self.paths["cs2_exe"]),
                "-cmdLine", subprocess.list2cmdline(self.argv()[1:]), "-addEnv", "USRLOCALCSGO=" + str(self.root / "movie")]
        self.launched_at = time.time()
        self.child = subprocess.Popen(args, cwd=self.root, env=self.env, creationflags=subprocess.CREATE_NO_WINDOW)
        write_json(self.root / "evidence/launch.json", {"hlae_pid": self.child.pid, "argv": self.argv(), "time": self.launched_at})
        try:
            deadline = time.monotonic() + 90
            while time.monotonic() < deadline:
                self.check_cancel()
                games = [p for p in processes() if p["Name"].lower() == "cs2.exe"]
                if games:
                    if len(games) != 1:
                        raise CosmeticError("发现非独占游戏会话")
                    self.bind_game(games[0])
                    break
                time.sleep(.5)
            if not self.handle:
                raise CosmeticError("专用游戏启动超时")
            self.wait_for(lambda: self.current(required=False), timeout=90)
            previous = None
            def initialized():
                nonlocal previous
                sample = self.current(required=False)
                if initial_demo_progressed(previous, sample):
                    write_json(self.root / "evidence/preseek.json", {"before": previous, "after": sample})
                    return True
                if sample and (previous is None or sample.get("tick", -1) <= previous.get("tick", -1)
                               or previous.get("tick", -1) < 1024):
                    previous = sample
                return False
            self.wait_for(initialized, timeout=45)
            self.operation("position")
        except Exception:
            self.stop()
            raise

    def bind_game(self, game):
        if game["ParentProcessId"] != self.child.pid or native.parse_command_line(game.get("CommandLine") or "") != self.argv():
            raise CosmeticError("游戏会话启动来源或参数不匹配")
        identity, handle = native.read_process_identity(game["ProcessId"])
        if (native.comparable_path(identity["exe_final"]) != native.comparable_path(native.final_path(str(self.paths["cs2_exe"])))
                or identity["creation_filetime"] / 10_000_000 - 11644473600 < self.launched_at - 1):
            handle.close()
            raise CosmeticError("游戏会话身份不匹配")
        self.identity, self.handle = identity, handle
        write_json(self.root / "evidence/owner.json", identity)

    def check_cancel(self):
        if self.cancel.is_set():
            raise CosmeticError("任务已取消")

    def assert_owner(self):
        if not self.handle or not native.is_alive(self.handle):
            raise CosmeticError("专用游戏进程已退出")
        current, handle = native.read_process_identity(self.identity["pid"])
        try:
            if current != self.identity:
                raise CosmeticError("游戏会话身份已变化")
        finally:
            handle.close()

    def records(self):
        if not self.console.exists():
            return []
        with self.console.open("rb") as stream:
            stream.seek(max(0, self.console.stat().st_size - 262144))
            return parse_console(stream.read().decode("utf-8", errors="replace"), self.tag)

    def current(self, *, required=True):
        self.assert_owner()
        rows = self.records()
        stopped = next((r for r in reversed(rows) if r.get("kind") == "STOPPING"), None)
        if stopped:
            reason = {
                "RECORD_TARGET_CHANGED": "录制期间观察目标或武器发生变化，请重新检查并选择片段",
                "HOST_LEASE_LOST": "制片控制连接失效，专用会话已停止",
                "DEMO_LOST": "Demo 回放已结束，专用会话已停止",
                "SESSION_DEADLINE": "专用会话超过限定时间，已停止",
                "REQUESTED_STOP": "专用会话已按请求停止",
            }.get(stopped.get("reason"), "原生环境检查已停止会话")
            raise CosmeticError(reason)
        row = next((r for r in reversed(rows) if r.get("kind") == "DEMO_SAMPLE"), None)
        if row and (row.get("valid") is not True or row.get("demo") != self.demo.as_posix().lower()):
            raise CosmeticError("Demo 状态与专用会话不匹配")
        if not row or not 0 <= time.time()*1000-row.get("script_time_ms", 0) <= 2000:
            if not required:
                return None
            raise CosmeticError("Demo 状态未知或已过期")
        return row

    def validate_context(self, *, paused: bool):
        self.check_cancel()
        row = self.current()
        pov = row.get("pov") or {}
        if (row.get("paused") is not paused or row.get("phase") != "IDLE" or pov.get("mode") != 2 or pov.get("xuid") != self.request.xuid
                or not any(native_weapon_matches(pov.get(field), PRESET_DATA[self.request.preset_id][0]) for field in ('weapon', 'weapon_debug'))):
            raise CosmeticError("当前观察目标、武器或暂停状态不匹配")
        expected = self.request.start_tick if self._step >= 3 else self.request.start_tick - 64
        if abs(row["tick"] - expected) > 4:
            raise CosmeticError("当前 Demo 不在已确认的片段起点")

    def wait_for(self, predicate, timeout=45):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self.check_cancel()
            value = predicate()
            if value:
                return value
            time.sleep(.2)
        raise CosmeticError("原生制片操作超时，未将其视为成功")

    def operation(self, name, monitor=None):
        operations = ["position", "load", "warm", "record"]
        if self._step >= len(operations) or name != operations[self._step]:
            raise CosmeticError("制片操作顺序不正确")
        self.current()
        (self.root / "requests" / (name + ".flag")).touch(exist_ok=False)
        def complete():
            # Before any cosmetic DLL load, an explicitly requested cold seek
            # may be pending. Wait without applying anything, bounded by the
            # operation deadline; STOPPING/wrong Demo/owner failure still abort.
            sample = self.current(required=name != "position")
            if sample is None:
                return None
            if monitor:
                monitor()
            done = next((r for r in reversed(self.records()) if r.get("kind") == "OPERATION_COMPLETE" and r.get("operation") == name), None)
            if done and sample.get("phase") == "IDLE" and sample.get("script_time_ms", 0) >= done.get("script_time_ms", 0):
                return done
            return None
        result = self.wait_for(complete)
        self._step += 1
        return result

    def module_paths(self):
        self.assert_owner()
        raw = powershell(f"(Get-Process -Id {self.identity['pid']}).Modules | Select-Object -ExpandProperty FileName | ConvertTo-Json -Compress", env=self.env)
        rows = json.loads(raw)
        return {native.comparable_path(p) for p in (rows if isinstance(rows, list) else [rows])}

    def load_extension(self):
        if self.file_mode:
            raise CosmeticError("文件制片模式禁止加载换肤 DLL")
        self.check_pins()
        self.validate_context(paused=True)
        mods = self.module_paths()
        if native.comparable_path(str(self.paths["hook"])) not in mods or any("nskinz" in p.lower() for p in mods):
            raise CosmeticError("HLAE 未就绪或已存在非本任务扩展")
        if digest(self.dll) != PINS["nskinz_dll"]:
            raise CosmeticError("扩展文件变化")
        self.operation("load")
        self.wait_for(lambda: native.comparable_path(str(self.dll)) in self.module_paths(), timeout=15)
        # OpenEvent becomes available during bootstrap, before all deferred
        # startup work has settled. The successful reference control sequence
        # waits here BEFORE its first Update, not after signaling the event.
        first = None
        def startup_complete():
            nonlocal first
            self.validate_context(paused=True)
            log = self.root / "extension/nSkinzCS2.log"
            if not log.is_file() or log.stat().st_size > 2 * 1024**2:
                return False
            if "nSkinzCS2 bootstrap completed." not in log.read_text(encoding="utf-8", errors="replace"):
                return False
            sample = self.current()
            if first is None:
                first = sample
                return False
            if sample["script_time_ms"] - first["script_time_ms"] < 2000:
                return False
            write_json(self.root / "evidence/extension-startup.json", {"first": first, "last": sample,
                                                                      "bootstrap_seen": True, "visual_verified": None})
            return True
        self.wait_for(startup_complete, timeout=15)

    def prepare_file_playback(self, expected_hash):
        if not self.file_mode or self.demo_hash != expected_hash:
            raise CosmeticError("制片副本未绑定当前会话")
        self.check_pins()
        self.validate_context(paused=True)
        mods = self.module_paths()
        if native.comparable_path(str(self.paths["hook"])) not in mods or any("nskinz" in p.lower() for p in mods):
            raise CosmeticError("文件制片会话不是独占 HLAE 回放，已停止")
        self.operation("load")  # Reviewed file-mode controller: explicitly no DLL load.
        self.operation("warm")
        self.validate_context(paused=True)
        if any("nskinz" in p.lower() for p in self.module_paths()):
            raise CosmeticError("文件制片模式检测到额外换肤扩展")
        write_json(self.root / "evidence/file-playback.json", {
            "playback_sha256": expected_hash, "runtime_cosmetic_dll": False, "visual_verified": None})

    def request_refresh(self, profile_digest):
        if self.file_mode:
            raise CosmeticError("文件制片模式不使用扩展刷新接口")
        self.validate_context(paused=True)
        if profile_digest != self.baseline["profile_sha256"] or digest(self.profile) != profile_digest:
            raise CosmeticError("外观预设已变化")
        k32 = ctypes.WinDLL("kernel32", use_last_error=True)
        k32.OpenEventW.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.LPCWSTR]
        k32.OpenEventW.restype = wintypes.HANDLE
        k32.SetEvent.argtypes = [wintypes.HANDLE]
        k32.SetEvent.restype = wintypes.BOOL
        k32.CloseHandle.argtypes = [wintypes.HANDLE]
        k32.CloseHandle.restype = wintypes.BOOL
        event = None
        deadline = time.monotonic() + 10
        while not event and time.monotonic() < deadline:
            self.validate_context(paused=True)
            event = k32.OpenEventW(2, False, "Local\\nSkinzCS2.ManualRefresh." + str(self.identity["pid"]))
            if not event:
                time.sleep(.2)
        if not event:
            raise CosmeticError("已审查的扩展刷新接口未就绪")
        try:
            self.validate_context(paused=True)
            log = self.root / "extension/nSkinzCS2.log"
            if not log.is_file():
                raise CosmeticError("扩展诊断日志缺失，状态未知")
            offset = log.stat().st_size
            requested_at = time.time()
            if not k32.SetEvent(event):
                raise CosmeticError("外观更新请求未送达")
        finally:
            k32.CloseHandle(event)
        write_json(self.root / "evidence/refresh.json", {"submitted": True, "visual_verified": None, "profile_sha256": profile_digest,
                                                       "requested_at": requested_at, "log_offset": offset})
        def fresh_diagnostics():
            self.validate_context(paused=True)
            size = log.stat().st_size
            if not offset <= size <= offset + 2 * 1024**2:
                raise CosmeticError("扩展诊断日志重置或超出范围，状态未知")
            with log.open("rb") as stream:
                stream.seek(offset)
                return stream.read(2 * 1024**2).decode("utf-8", errors="replace")
        # A signaled event does not mean the worker has attached its hook and
        # finished the paused Update rebuild. Do not race it with demo_resume.
        self.wait_for(lambda: refresh_armed(fresh_diagnostics()), timeout=10)
        write_json(self.root / "evidence/refresh-armed.json", {"time": time.time(), "log_offset": offset,
                                                             "visual_verified": None})
        self.operation("warm")
        def initialized():
            return refresh_fields_ready(fresh_diagnostics(), self.request.xuid)
        self.wait_for(initialized, timeout=5)
        write_json(self.root / "evidence/refresh-fields.json", {"fields_initialized": True, "visual_verified": None,
                                                              "log_offset": offset, "profile_sha256": profile_digest})

    def stop(self):
        if self._restored:
            return
        try:
            if self.child:
                (self.root / "requests/stop.flag").touch(exist_ok=True)
                # Reap our loader handle first: it must not spawn a late game
                # after cancellation/cleanup was reported. This is our Popen,
                # never a loader discovered by process name.
                if self.child.poll() is None:
                    self.child.terminate()
                    self.child.wait(timeout=5)
                if not self.handle:
                    games = [p for p in processes() if p["Name"].lower() == "cs2.exe"]
                    owned_games = [p for p in games if p["ParentProcessId"] == self.child.pid]
                    if len(owned_games) > 1:
                        raise CosmeticError("启动过程中出现多个子游戏，清理状态未知")
                    if owned_games:
                        self.bind_game(owned_games[0])
                deadline = time.monotonic() + 15
                while self.handle and native.is_alive(self.handle) and time.monotonic() < deadline:
                    time.sleep(.2)
                if self.handle and native.is_alive(self.handle):
                    # Exact reopened identity + retained handle; never process-name termination.
                    self.assert_owner()
                    terminate_owned(self.identity)
                if self.handle and native.is_alive(self.handle):
                    raise CosmeticError("专用游戏未退出，清理状态未知")
                remaining = processes()
                helper_deadline = time.monotonic() + 5
                def own_helper(p):
                    return p["Name"].lower() == "rundll32.exe" and self.root.as_posix().lower() in (p.get("CommandLine") or "").replace("\\", "/").lower()
                while any(own_helper(p) for p in remaining) and time.monotonic() < helper_deadline:
                    time.sleep(.2)
                    remaining = processes()
                if any(p["Name"].lower() == "cs2.exe" for p in remaining):
                    raise CosmeticError("另一个游戏进程正在运行，停止恢复共享日志")
                if any(own_helper(p) for p in remaining):
                    raise CosmeticError("扩展辅助窗口尚未退出")
                self.child.wait(timeout=10)
                self.audit_restore()
            self._restored = True
        finally:
            self._lease_stop.set()
            if self._lease_thread:
                self._lease_thread.join(timeout=2)
            if self.handle and not native.is_alive(self.handle):
                self.handle.close()
                self.handle = None

    def audit_restore(self):
        self.check_pins()
        cfg = self.baseline["cfg"]
        changed = [r["path"] for r in cfg if not Path(r["path"]).exists() or digest(Path(r["path"])) != r["sha256"]]
        expected = {r["path"].lower() for r in cfg}
        extra = [str(p) for root in self.baseline["roots"] for p in Path(root).rglob("*")
                 if p.is_file() and p.suffix.lower() in (".cfg", ".vcfg") and str(p).lower() not in expected]
        if self.console.exists():
            shutil.copy2(self.console, self.root / "evidence/native-console.log")
        before = self.baseline["console_sha256"]
        if before is not None:
            backup = self.root / "backup/console.log"
            if digest(backup) != before:
                raise CosmeticError("原始日志备份损坏，未覆盖原文件")
            shutil.copy2(backup, self.console)
        elif self.console.exists():
            # Resolved exact file, archived rather than deleted; never a broad move.
            shutil.move(str(self.console.resolve()), str((self.root / "evidence/new-console.log").resolve()))
        write_json(self.root / "evidence/cleanup.json", {"changed_configs": changed, "new_configs": extra, "game_exited": True, "demo_unchanged": True})
        if changed or extra:
            raise CosmeticError("检测到配置差异；已保留备份，请人工核对，未自动覆盖")


def terminate_owned(identity):
    """Last-resort termination of precisely the saved instance, never a name/PID alone."""
    k32 = ctypes.WinDLL("kernel32", use_last_error=True)
    k32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    k32.OpenProcess.restype = wintypes.HANDLE
    k32.GetProcessTimes.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4
    k32.GetProcessTimes.restype = wintypes.BOOL
    k32.TerminateProcess.argtypes = [wintypes.HANDLE, wintypes.UINT]
    k32.TerminateProcess.restype = wintypes.BOOL
    k32.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
    k32.WaitForSingleObject.restype = wintypes.DWORD
    k32.CloseHandle.argtypes = [wintypes.HANDLE]
    raw = k32.OpenProcess(0x1000 | 0x100000 | 1, False, identity["pid"])
    if not raw:
        raise CosmeticError("无法确认待结束进程")
    try:
        times = [wintypes.FILETIME() for _ in range(4)]
        if not k32.GetProcessTimes(raw, *[ctypes.byref(t) for t in times]):
            raise CosmeticError("无法读取待结束进程身份")
        created = (times[0].dwHighDateTime << 32) | times[0].dwLowDateTime
        if created != identity["creation_filetime"]:
            raise CosmeticError("拒绝结束被复用的 PID")
        if not k32.TerminateProcess(raw, 1) or k32.WaitForSingleObject(raw, 5000) != 0:
            raise CosmeticError("已拥有的专用进程未退出")
    finally:
        k32.CloseHandle(raw)
