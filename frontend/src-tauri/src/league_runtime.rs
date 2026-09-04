use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Mutex,
    },
    thread,
    time::Duration,
};

#[cfg(windows)]
use std::{
    ffi::OsStr,
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt},
    path::Prefix,
};

#[cfg(windows)]
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_CANCELLED, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
    Storage::FileSystem::FILE_SHARE_READ,
    System::SystemInformation::GetSystemDirectoryW,
    System::Threading::{GetExitCodeProcess, GetProcessId, TerminateProcess, WaitForSingleObject},
    UI::{
        Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW},
        WindowsAndMessaging::SW_HIDE,
    },
};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::{start_backend, stop_backend, CREATE_NO_WINDOW};

const MODE_ASK: u8 = 0;
const MODE_MEMORY: u8 = 1;
const MODE_PARALLEL: u8 = 2;
const EMBEDDED_ARGUMENT: &str = "--maxgamestudio-embedded";
const HOST_PID_ARGUMENT_PREFIX: &str = "--maxgamestudio-host-pid=";
const SHUTDOWN_SIGNAL_ARGUMENT_PREFIX: &str = "--maxgamestudio-shutdown-signal=";
const LEGACY_RUNTIME_ACL_MARKER: &str = ".maxgamestudio-acl-hardened";
const RUNTIME_HASH_MANIFEST: &str = "maxgamestudio-runtime-hashes.json";
const EMBEDDED_RUNTIME_HASHES: &str =
    include_str!(concat!(env!("OUT_DIR"), "/league-runtime-hashes.json"));
#[cfg(windows)]
const ADMIN_LAUNCHER_SCRIPT: &str = include_str!("league_admin_launcher.ps1");

struct ManagedLeagueRuntime {
    process: ManagedRuntimeProcess,
    shutdown_signal: PathBuf,
}

enum ManagedRuntimeProcess {
    Direct(Child),
    #[cfg(windows)]
    Elevated(ElevatedRuntimeProcess),
}

#[cfg(windows)]
struct ElevatedRuntimeProcess {
    handle: usize,
    pid: u32,
    launcher: Option<PreparedAdminLauncher>,
}

#[cfg(windows)]
struct PreparedAdminLauncher {
    path: PathBuf,
    guard: Option<File>,
}

#[cfg(windows)]
impl Drop for PreparedAdminLauncher {
    fn drop(&mut self) {
        self.guard.take();
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(windows)]
impl Drop for ElevatedRuntimeProcess {
    fn drop(&mut self) {
        if self.handle != 0 {
            unsafe {
                CloseHandle(self.handle as HANDLE);
            }
            self.handle = 0;
        }
        self.launcher.take();
    }
}

pub(crate) struct LeagueRuntimeProcess {
    child: Mutex<Option<ManagedLeagueRuntime>>,
    active: AtomicBool,
    administrator: AtomicBool,
    mode: AtomicU8,
    restoring: AtomicBool,
    suppress_restore: AtomicBool,
    last_error: Mutex<Option<String>>,
}

impl Default for LeagueRuntimeProcess {
    fn default() -> Self {
        Self {
            child: Mutex::new(None),
            active: AtomicBool::new(false),
            administrator: AtomicBool::new(false),
            mode: AtomicU8::new(MODE_ASK),
            restoring: AtomicBool::new(false),
            suppress_restore: AtomicBool::new(false),
            last_error: Mutex::new(None),
        }
    }
}

fn parse_mode(mode: &str) -> Result<u8, String> {
    match mode.trim().to_ascii_lowercase().as_str() {
        "memory" => Ok(MODE_MEMORY),
        "parallel" => Ok(MODE_PARALLEL),
        _ => Err("League runtime mode must be memory or parallel".to_string()),
    }
}

fn mode_name(mode: u8) -> &'static str {
    match mode {
        MODE_MEMORY => "memory",
        MODE_PARALLEL => "parallel",
        _ => "ask",
    }
}

fn runtime_root(app: &AppHandle) -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        if let Some(path) = std::env::var_os("MAXGAMESTUDIO_LEAGUE_RUNTIME_EXE") {
            let path = PathBuf::from(path);
            if path.is_file() {
                return path
                    .parent()
                    .map(Path::to_path_buf)
                    .ok_or_else(|| "League runtime override has no parent directory".to_string());
            }
        }
    }
    app.path()
        .resource_dir()
        .map(|path| path.join("league-runtime"))
        .map_err(|error| format!("无法解析内置英雄联盟运行时目录：{error}"))
}

fn runtime_executable(app: &AppHandle) -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        if let Some(path) = std::env::var_os("MAXGAMESTUDIO_LEAGUE_RUNTIME_EXE") {
            let path = PathBuf::from(path);
            if path.is_file() {
                return Ok(path);
            }
        }
    }
    let root = runtime_root(app)?;
    ["MaxGameStudioLeague.exe", "MaxGameStudio League.exe"]
        .into_iter()
        .map(|name| root.join(name))
        .find(|path| path.is_file())
        .ok_or_else(|| format!("未找到内置英雄联盟运行时：{}", root.display()))
}

fn administrator_launch_ready(_executable: &Path) -> bool {
    #[cfg(windows)]
    {
        embedded_runtime_hashes().is_ok() && system_admin_launch_executables().is_ok()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn embedded_runtime_hashes() -> Result<BTreeMap<String, String>, String> {
    let hashes: BTreeMap<String, String> = serde_json::from_str(EMBEDDED_RUNTIME_HASHES)
        .map_err(|error| format!("内嵌英雄联盟工作台哈希清单无效：{error}"))?;
    if hashes.is_empty() {
        return Err("当前构建未内嵌英雄联盟工作台哈希清单".to_string());
    }
    if !hashes.contains_key("MaxGameStudioLeague.exe") || !hashes.contains_key("resources/app.asar")
    {
        return Err("内嵌英雄联盟工作台哈希清单缺少必需文件".to_string());
    }
    Ok(hashes)
}

fn normalized_runtime_relative_path(value: &str) -> Result<PathBuf, String> {
    let normalized = value.replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.contains(':')
        || Path::new(&normalized).is_absolute()
    {
        return Err(format!("英雄联盟工作台哈希清单包含不安全路径：{value}"));
    }
    let path = PathBuf::from(&normalized);
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!("英雄联盟工作台哈希清单包含不安全路径：{value}"));
    }
    Ok(path)
}

fn runtime_manifest_key(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path)
        .map_err(|error| format!("无法读取英雄联盟工作台文件 {}：{error}", path.display()))?;
    sha256_reader(&mut file)
        .map_err(|error| format!("无法校验英雄联盟工作台文件 {}：{error}", path.display()))
}

fn sha256_reader(reader: &mut impl Read) -> std::io::Result<String> {
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    format!("{:x}", digest.finalize())
}

fn collect_runtime_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<PathBuf>,
) -> Result<(), String> {
    for entry in fs::read_dir(directory).map_err(|error| {
        format!(
            "无法枚举英雄联盟工作台目录 {}：{error}",
            directory.display()
        )
    })? {
        let entry = entry.map_err(|error| format!("无法读取英雄联盟工作台目录项：{error}"))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            format!("无法读取英雄联盟工作台文件属性 {}：{error}", path.display())
        })?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "英雄联盟工作台目录包含不受信任的链接：{}",
                path.display()
            ));
        }
        if metadata.is_dir() {
            collect_runtime_files(root, &path, files)?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| format!("英雄联盟工作台文件超出运行时目录：{}", path.display()))?;
            files.push(relative.to_path_buf());
        }
    }
    Ok(())
}

fn is_mutable_runtime_file(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    lower == LEGACY_RUNTIME_ACL_MARKER
        || lower == RUNTIME_HASH_MANIFEST
        || lower == "debug.log"
        || lower.starts_with("logs/")
}

fn verify_runtime_integrity_against(
    executable: &Path,
    expected: &BTreeMap<String, String>,
) -> Result<(), String> {
    let root = executable
        .parent()
        .ok_or_else(|| "内置英雄联盟工作台可执行文件没有父目录".to_string())?;
    let canonical_root = fs::canonicalize(root)
        .map_err(|error| format!("无法解析英雄联盟工作台目录 {}：{error}", root.display()))?;
    let mut normalized_expected = BTreeMap::new();
    for (raw_key, raw_hash) in expected {
        let relative = normalized_runtime_relative_path(raw_key)?;
        let key = runtime_manifest_key(&relative);
        let expected_hash = raw_hash.trim().to_ascii_lowercase();
        if expected_hash.len() != 64 || !expected_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(format!("英雄联盟工作台哈希清单包含无效摘要：{raw_key}"));
        }
        if normalized_expected
            .insert(key.clone(), expected_hash)
            .is_some()
        {
            return Err(format!("英雄联盟工作台哈希清单包含重复路径：{key}"));
        }
    }
    for required in ["MaxGameStudioLeague.exe", "resources/app.asar"] {
        if !normalized_expected.contains_key(required) {
            return Err(format!("英雄联盟工作台哈希清单缺少必需文件：{required}"));
        }
    }

    for (key, expected_hash) in &normalized_expected {
        let relative = normalized_runtime_relative_path(key)?;
        let target = root.join(&relative);
        let metadata = fs::symlink_metadata(&target)
            .map_err(|error| format!("英雄联盟工作台文件缺失 {}：{error}", target.display()))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(format!(
                "英雄联盟工作台文件类型不安全：{}",
                target.display()
            ));
        }
        let canonical_target = fs::canonicalize(&target)
            .map_err(|error| format!("无法解析英雄联盟工作台文件 {}：{error}", target.display()))?;
        if !canonical_target.starts_with(&canonical_root) {
            return Err(format!(
                "英雄联盟工作台文件超出受控目录：{}",
                target.display()
            ));
        }
        let actual_hash = sha256_file(&target)?;
        if actual_hash != *expected_hash {
            return Err(format!("英雄联盟工作台完整性校验失败：{key}"));
        }
    }

    let expected_keys = normalized_expected.keys().cloned().collect::<HashSet<_>>();
    let mut actual_files = Vec::new();
    collect_runtime_files(root, root, &mut actual_files)?;
    for relative in actual_files {
        let key = runtime_manifest_key(&relative);
        if !expected_keys.contains(&key) && !is_mutable_runtime_file(&key) {
            return Err(format!("英雄联盟工作台目录包含未签入清单的文件：{key}"));
        }
    }
    Ok(())
}

fn verify_runtime_integrity(executable: &Path) -> Result<(), String> {
    let expected = embedded_runtime_hashes()?;
    verify_runtime_integrity_against(executable, &expected)
}

fn runtime_user_data_dir(_app: &AppHandle) -> Result<PathBuf, String> {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "Windows APPDATA 环境变量不存在".to_string())?;

    #[cfg(not(windows))]
    let base = _app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法解析应用数据目录：{error}"))?;

    let path = base.join("MaxGameStudio").join("league-runtime");
    fs::create_dir_all(&path)
        .map_err(|error| format!("无法创建英雄联盟工作台数据目录 {}：{error}", path.display()))?;
    Ok(path)
}

fn runtime_user_data_argument(path: &Path) -> String {
    format!("--user-data-dir={}", path.display())
}

fn runtime_shutdown_signal(path: &Path) -> PathBuf {
    path.join("runtime-shutdown.request")
}

fn runtime_arguments(user_data_dir: &Path, shutdown_signal: &Path, host_pid: u32) -> Vec<String> {
    vec![
        runtime_user_data_argument(user_data_dir),
        EMBEDDED_ARGUMENT.to_string(),
        format!("{HOST_PID_ARGUMENT_PREFIX}{host_pid}"),
        format!(
            "{SHUTDOWN_SIGNAL_ARGUMENT_PREFIX}{}",
            shutdown_signal.display()
        ),
    ]
}

#[cfg(windows)]
fn wide_null(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn system_admin_launch_executables() -> Result<(PathBuf, PathBuf), String> {
    let mut buffer = vec![0u16; 32_768];
    let length = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
    if length == 0 || length as usize >= buffer.len() {
        return Err(format!(
            "无法解析 Windows 系统目录：{}",
            std::io::Error::last_os_error()
        ));
    }
    buffer.truncate(length as usize);
    let system_directory = PathBuf::from(String::from_utf16_lossy(&buffer));
    let powershell = system_directory
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    let cmd = system_directory.join("cmd.exe");
    if !powershell.is_file() {
        return Err(format!(
            "Windows 受保护的 PowerShell 启动器不存在：{}",
            powershell.display()
        ));
    }
    if !cmd.is_file() {
        return Err(format!(
            "Windows 受保护的命令监督器不存在：{}",
            cmd.display()
        ));
    }
    Ok((powershell, cmd))
}

#[cfg(windows)]
fn random_admin_session_name() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| format!("无法生成管理员工作台隔离会话标识：{error}"))?;
    Ok(format!(
        "session-{}",
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}

#[cfg(windows)]
fn normalized_local_windows_drive_path(source_root: &Path) -> Result<(PathBuf, u8), String> {
    if !source_root.is_absolute() {
        return Err("管理员工作台仅支持本地 Windows 磁盘路径".to_string());
    }

    let mut components = source_root.components();
    let drive = match components.next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => drive,
            _ => return Err("管理员工作台仅支持本地 Windows 磁盘路径".to_string()),
        },
        _ => return Err("管理员工作台仅支持本地 Windows 磁盘路径".to_string()),
    };
    if !matches!(components.next(), Some(Component::RootDir)) {
        return Err("管理员工作台仅支持本地 Windows 磁盘路径".to_string());
    }

    // std::fs::canonicalize and Tauri's resource_dir can legitimately return
    // a verbatim drive path (\\?\D:\...). Convert only that drive-prefixed
    // form back to a regular DOS path. UNC, device namespace, parent traversal,
    // and every other prefix remain rejected before elevation.
    let mut normalized = PathBuf::from(format!("{}:\\", char::from(drive)));
    for component in components {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            _ => return Err("管理员工作台路径包含不安全的路径片段".to_string()),
        }
    }
    Ok((normalized, drive))
}

#[cfg(windows)]
fn protected_admin_session_root(source_root: &Path, session_name: &str) -> Result<PathBuf, String> {
    let (_, drive) = normalized_local_windows_drive_path(source_root)?;
    let volume_root = PathBuf::from(format!("{}:\\", char::from(drive)));
    if !session_name.starts_with("session-")
        || session_name.len() != 40
        || !session_name[8..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("管理员工作台隔离会话标识无效".to_string());
    }
    Ok(volume_root
        .join("MaxGameStudioAdminRuntime")
        .join("Sessions")
        .join(session_name))
}

#[cfg(windows)]
fn admin_launcher_script(config: &Value) -> Result<Vec<u8>, String> {
    let config_bytes = serde_json::to_vec(config)
        .map_err(|error| format!("无法生成管理员工作台启动配置：{error}"))?;
    let config_base64 = BASE64_STANDARD.encode(config_bytes);
    let script = ADMIN_LAUNCHER_SCRIPT.replace("__CONFIG_BASE64__", &config_base64);
    if script.contains("__CONFIG_BASE64__") {
        return Err("管理员工作台启动脚本配置未完成替换".to_string());
    }
    Ok(script.into_bytes())
}

#[cfg(windows)]
fn transient_admin_launcher_path(
    source_root: &Path,
    session_name: &str,
) -> Result<PathBuf, String> {
    let (_, drive) = normalized_local_windows_drive_path(source_root)?;
    let _ = protected_admin_session_root(source_root, session_name)?;
    Ok(PathBuf::from(format!(
        "{}:\\MaxGameStudioAdminLauncher-{}.ps1",
        char::from(drive),
        &session_name[8..]
    )))
}

#[cfg(windows)]
fn prepare_admin_launcher(path: PathBuf, config: &Value) -> Result<PreparedAdminLauncher, String> {
    let script = admin_launcher_script(config)?;
    let expected_hash = sha256_bytes(&script);
    let result = (|| {
        let mut writer = OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(0)
            .open(&path)
            .map_err(|error| format!("无法创建一次性管理员启动脚本 {}：{error}", path.display()))?;
        writer
            .write_all(&script)
            .map_err(|error| format!("无法写入一次性管理员启动脚本 {}：{error}", path.display()))?;
        writer
            .sync_all()
            .map_err(|error| format!("无法同步一次性管理员启动脚本 {}：{error}", path.display()))?;
        drop(writer);

        // Re-open read-only with no write/delete sharing. After this handle is
        // acquired, neither the file nor an ancestor can be replaced before
        // the elevated PowerShell process has finished reading and executing
        // the exact bytes verified below.
        let mut guard = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&path)
            .map_err(|error| format!("无法锁定一次性管理员启动脚本 {}：{error}", path.display()))?;
        let actual_hash = sha256_reader(&mut guard)
            .map_err(|error| format!("无法校验一次性管理员启动脚本 {}：{error}", path.display()))?;
        if actual_hash != expected_hash {
            return Err("一次性管理员启动脚本在启动前发生变化，已拒绝提权".to_string());
        }
        Ok(PreparedAdminLauncher {
            path: path.clone(),
            guard: Some(guard),
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&path);
    }
    result
}

#[cfg(windows)]
fn supervised_admin_command_line(
    powershell: &Path,
    launcher_path: &Path,
) -> Result<String, String> {
    let unsafe_path = |path: &Path| {
        path.to_string_lossy().chars().any(|character| {
            matches!(
                character,
                '"' | '%' | '!' | '&' | '|' | '<' | '>' | '^' | '\r' | '\n'
            )
        })
    };
    if unsafe_path(powershell) || unsafe_path(launcher_path) {
        return Err("管理员启动路径包含不安全字符，已拒绝启动".to_string());
    }
    let command = format!(
        "/d /s /c \"\"{}\" -NoLogo -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File \"{}\"\"",
        powershell.display(),
        launcher_path.display()
    );
    if command.chars().count() > 8_000 {
        return Err(format!(
            "管理员启动监督命令过长（{} 字符）",
            command.chars().count()
        ));
    }
    Ok(command)
}

#[cfg(windows)]
fn quote_windows_argument(value: &str) -> String {
    if !value.is_empty()
        && !value
            .chars()
            .any(|character| character.is_whitespace() || character == '"')
    {
        return value.to_string();
    }

    let mut quoted = String::from("\"");
    let mut backslashes = 0usize;
    for character in value.chars() {
        if character == '\\' {
            backslashes += 1;
            continue;
        }
        if character == '"' {
            quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
            quoted.push('"');
        } else {
            quoted.push_str(&"\\".repeat(backslashes));
            quoted.push(character);
        }
        backslashes = 0;
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

#[cfg(windows)]
fn elevated_runtime_command_line(arguments: &[String]) -> String {
    arguments
        .iter()
        // An elevated Chromium profile must never be loaded from the caller's
        // user-writable APPDATA. The protected launcher supplies its own
        // persistent profile beneath MaxGameStudioAdminRuntime.
        .filter(|argument| !argument.starts_with("--user-data-dir="))
        .map(|argument| quote_windows_argument(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(windows)]
fn launch_elevated_runtime(
    _executable: &Path,
    working_directory: &Path,
    arguments: &[String],
) -> Result<ElevatedRuntimeProcess, String> {
    let (powershell, command_supervisor) = system_admin_launch_executables()?;
    let system_directory = command_supervisor
        .parent()
        .ok_or_else(|| "无法解析 Windows 系统目录".to_string())?;
    let session_name = random_admin_session_name()?;
    let (normalized_working_directory, _) = normalized_local_windows_drive_path(working_directory)?;
    let _session_root = protected_admin_session_root(&normalized_working_directory, &session_name)?;
    let runtime_command_line = elevated_runtime_command_line(arguments);
    let config = json!({
        "sourceRoot": normalized_working_directory.to_string_lossy(),
        "manifestSha256": sha256_bytes(EMBEDDED_RUNTIME_HASHES.as_bytes()),
        "sessionName": session_name.clone(),
        "commandLine": runtime_command_line,
        "hostPid": std::process::id(),
    });
    let launcher_path =
        transient_admin_launcher_path(&normalized_working_directory, &session_name)?;
    let launcher = prepare_admin_launcher(launcher_path.clone(), &config)?;
    // ShellExecuteEx does not reliably preserve Windows PowerShell's exit code
    // across `runas` on every supported Windows configuration. A trusted,
    // full-path cmd.exe is therefore used only as a fixed supervisor. The
    // generated script is held read-only without write/delete sharing until
    // the supervisor exits. Only fixed, internally generated local paths cross
    // cmd.exe's parser; runtime/config values remain Base64 inside the script.
    let supervised_command = supervised_admin_command_line(&powershell, &launcher_path)?;
    let verb = wide_null(OsStr::new("runas"));
    let file = wide_null(command_supervisor.as_os_str());
    let directory = wide_null(system_directory.as_os_str());
    let parameters = wide_null(OsStr::new(&supervised_command));
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = parameters.as_ptr();
    info.lpDirectory = directory.as_ptr();
    info.nShow = SW_HIDE;

    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_CANCELLED as i32) {
            return Err("UAC_CANCELLED: 用户取消了管理员权限请求".to_string());
        }
        return Err(format!("无法以管理员权限启动英雄联盟工作台：{error}"));
    }
    if info.hProcess.is_null() {
        return Err("管理员启动未返回可监管的进程句柄".to_string());
    }
    let pid = unsafe { GetProcessId(info.hProcess) };
    if pid == 0 {
        unsafe {
            CloseHandle(info.hProcess);
        }
        return Err(format!(
            "管理员启动成功，但无法读取工作台进程 ID：{}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(ElevatedRuntimeProcess {
        handle: info.hProcess as usize,
        pid,
        launcher: Some(launcher),
    })
}

fn launch_runtime_process(
    executable: &Path,
    arguments: &[String],
    administrator: bool,
) -> Result<ManagedRuntimeProcess, String> {
    let working_directory = executable.parent().unwrap_or_else(|| Path::new("."));

    if administrator {
        #[cfg(windows)]
        {
            return launch_elevated_runtime(executable, working_directory, arguments)
                .map(ManagedRuntimeProcess::Elevated);
        }
        #[cfg(not(windows))]
        {
            return Err("管理员启动仅支持 Windows".to_string());
        }
    }

    let mut command = Command::new(executable);
    command
        .current_dir(working_directory)
        .args(arguments)
        .env("MAXGAMESTUDIO_EMBEDDED", "1")
        .env("MAXGAMESTUDIO_HOST_PID", std::process::id().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
        .spawn()
        .map(ManagedRuntimeProcess::Direct)
        .map_err(|error| format!("无法启动内置英雄联盟工作台：{error}"))
}

fn manifest_value(app: &AppHandle) -> Option<Value> {
    let path = runtime_root(app)
        .ok()?
        .join("maxgamestudio-runtime-manifest.json");
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

#[cfg(windows)]
fn process_tree_working_set_bytes(root_pid: u32) -> Option<u64> {
    use std::os::windows::process::CommandExt;

    let script = concat!(
        "$root=__ROOT_PID__;",
        "$rows=Get-CimInstance Win32_Process|Select-Object ProcessId,ParentProcessId;",
        "$ids=[System.Collections.Generic.HashSet[uint32]]::new();",
        "[void]$ids.Add([uint32]$root);",
        "do{$changed=$false;foreach($row in $rows){",
        "if($ids.Contains([uint32]$row.ParentProcessId)-and $ids.Add([uint32]$row.ProcessId)){$changed=$true}",
        "}}while($changed);",
        "$sum=0L;foreach($id in $ids){try{$sum+=(Get-Process -Id $id -ErrorAction Stop).WorkingSet64}catch{}};",
        "[Console]::Write($sum)"
    )
    .replace("__ROOT_PID__", &root_pid.to_string());
    let mut command = Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    command.creation_flags(CREATE_NO_WINDOW);
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

#[cfg(not(windows))]
fn process_tree_working_set_bytes(_root_pid: u32) -> Option<u64> {
    None
}

fn create_main_window(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        window.show().map_err(|error| error.to_string())?;
        window.unminimize().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        return Ok(());
    }
    WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("MaxGameStudio")
        .inner_size(1440.0, 900.0)
        .min_inner_size(1100.0, 700.0)
        .center()
        .decorations(false)
        .disable_drag_drop_handler()
        .visible(true)
        .build()
        .map(|_| ())
        .map_err(|error| format!("无法恢复 MaxGameStudio 主窗口：{error}"))
}

fn close_host_webviews_for_memory_mode(app: &AppHandle) {
    for label in ["league-mini", "league-ongoing", "league-cd-timer", "main"] {
        if let Some(window) = app.get_webview_window(label) {
            let _ = window.destroy();
        }
    }
}

fn terminate_direct_runtime_tree(child: &mut Child) -> Result<(), String> {
    if child
        .try_wait()
        .map_err(|error| format!("读取英雄联盟工作台进程状态失败：{error}"))?
        .is_some()
    {
        return Ok(());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut taskkill = Command::new("taskkill");
        taskkill.args(["/pid", &child.id().to_string(), "/f", "/t"]);
        taskkill.creation_flags(CREATE_NO_WINDOW);
        let _ = taskkill.status();
    }
    if let Err(error) = child.kill() {
        if child
            .try_wait()
            .map_err(|wait_error| format!("读取英雄联盟工作台进程状态失败：{wait_error}"))?
            .is_some()
        {
            return Ok(());
        }
        return Err(format!("终止英雄联盟工作台进程失败：{error}"));
    }
    for _ in 0..30 {
        match child
            .try_wait()
            .map_err(|error| format!("读取英雄联盟工作台进程状态失败：{error}"))?
        {
            Some(_) => return Ok(()),
            None => thread::sleep(Duration::from_millis(100)),
        }
    }
    Err("英雄联盟工作台进程在 3 秒内未退出".to_string())
}

impl ManagedRuntimeProcess {
    fn pid(&self) -> u32 {
        match self {
            Self::Direct(child) => child.id(),
            #[cfg(windows)]
            Self::Elevated(runtime) => runtime.pid,
        }
    }

    fn has_exited(&mut self) -> Result<bool, String> {
        match self {
            Self::Direct(child) => child
                .try_wait()
                .map(|status| status.is_some())
                .map_err(|error| format!("读取英雄联盟工作台进程状态失败：{error}")),
            #[cfg(windows)]
            Self::Elevated(runtime) => {
                let result = unsafe { WaitForSingleObject(runtime.handle as HANDLE, 0) };
                match result {
                    WAIT_OBJECT_0 => Ok(true),
                    WAIT_TIMEOUT => Ok(false),
                    _ => Err(format!(
                        "读取管理员工作台进程状态失败：{}",
                        std::io::Error::last_os_error()
                    )),
                }
            }
        }
    }

    fn completion_error(&mut self) -> Result<Option<String>, String> {
        match self {
            Self::Direct(child) => child
                .try_wait()
                .map(|status| {
                    status.and_then(|status| {
                        (!status.success())
                            .then(|| format!("英雄联盟工作台异常退出（状态 {status}）"))
                    })
                })
                .map_err(|error| format!("读取英雄联盟工作台退出状态失败：{error}")),
            #[cfg(windows)]
            Self::Elevated(runtime) => {
                let mut exit_code = 0u32;
                if unsafe { GetExitCodeProcess(runtime.handle as HANDLE, &mut exit_code) } == 0 {
                    return Err(format!(
                        "读取管理员工作台退出代码失败：{}",
                        std::io::Error::last_os_error()
                    ));
                }
                Ok(match exit_code {
                    0 => None,
                    201 => Some(
                        "管理员工作台启动器初始化失败，已拒绝启动；请重新安装完整的 MaxGameStudio 安装包"
                            .to_string(),
                    ),
                    211..=213 => Some(format!(
                        "管理员工作台无法创建或验证受保护目录（阶段 {exit_code}），已拒绝启动"
                    )),
                    214..=216 => Some(format!(
                        "管理员工作台的运行时完整性校验失败（阶段 {exit_code}），已拒绝启动；请重新安装完整的 MaxGameStudio 安装包"
                    )),
                    217 => Some(
                        "管理员工作台的受保护副本已经就绪，但进程启动失败（阶段 217）"
                            .to_string(),
                    ),
                    202 => Some("管理员英雄联盟工作台异常退出".to_string()),
                    code => Some(format!("管理员工作台异常退出（代码 {code}）")),
                })
            }
        }
    }

    fn force_terminate(&mut self) -> Result<(), String> {
        match self {
            Self::Direct(child) => terminate_direct_runtime_tree(child),
            #[cfg(windows)]
            Self::Elevated(runtime) => {
                if runtime.handle == 0 {
                    return Ok(());
                }
                use std::os::windows::process::CommandExt;
                match unsafe { WaitForSingleObject(runtime.handle as HANDLE, 0) } {
                    WAIT_OBJECT_0 => return Ok(()),
                    WAIT_TIMEOUT => {}
                    result => {
                        return Err(format!(
                            "读取管理员工作台进程状态失败（等待结果 {result}）：{}",
                            std::io::Error::last_os_error()
                        ));
                    }
                }
                let mut taskkill = Command::new("taskkill");
                taskkill.args(["/pid", &runtime.pid.to_string(), "/f", "/t"]);
                taskkill.creation_flags(CREATE_NO_WINDOW);
                let _ = taskkill.status();
                match unsafe { WaitForSingleObject(runtime.handle as HANDLE, 0) } {
                    WAIT_OBJECT_0 => return Ok(()),
                    WAIT_TIMEOUT => {}
                    result => {
                        return Err(format!(
                            "读取管理员工作台进程状态失败（等待结果 {result}）：{}",
                            std::io::Error::last_os_error()
                        ));
                    }
                }
                if unsafe { TerminateProcess(runtime.handle as HANDLE, 0) } == 0 {
                    if unsafe { WaitForSingleObject(runtime.handle as HANDLE, 0) } == WAIT_OBJECT_0
                    {
                        return Ok(());
                    }
                    return Err(format!(
                        "管理员工作台未响应退出请求，请在其窗口中关闭：{}",
                        std::io::Error::last_os_error()
                    ));
                }
                match unsafe { WaitForSingleObject(runtime.handle as HANDLE, 3_000) } {
                    WAIT_OBJECT_0 => Ok(()),
                    WAIT_TIMEOUT => Err("管理员英雄联盟工作台进程在 3 秒内未退出".to_string()),
                    result => Err(format!(
                        "等待管理员工作台退出失败（等待结果 {result}）：{}",
                        std::io::Error::last_os_error()
                    )),
                }
            }
        }
    }
}

impl Drop for ManagedRuntimeProcess {
    fn drop(&mut self) {
        let should_terminate = !matches!(self.has_exited(), Ok(true));
        if should_terminate {
            let _ = self.force_terminate();
        }
    }
}

impl ManagedLeagueRuntime {
    fn request_exit(&mut self) -> Result<(), String> {
        let mut status_error = match self.process.has_exited() {
            Ok(true) => {
                let _ = fs::remove_file(&self.shutdown_signal);
                return Ok(());
            }
            Ok(false) => None,
            Err(error) => Some(error),
        };
        let signal_written = fs::write(&self.shutdown_signal, b"shutdown\n").is_ok();
        if status_error.is_none() && signal_written {
            for _ in 0..30 {
                match self.process.has_exited() {
                    Ok(true) => {
                        let _ = fs::remove_file(&self.shutdown_signal);
                        return Ok(());
                    }
                    Ok(false) => thread::sleep(Duration::from_millis(100)),
                    Err(error) => {
                        status_error = Some(error);
                        break;
                    }
                }
            }
        }
        let result = self.process.force_terminate();
        let _ = fs::remove_file(&self.shutdown_signal);
        match (status_error, result) {
            (Some(_), Ok(())) => Ok(()),
            (Some(status_error), Err(force_error)) => Err(format!(
                "{status_error}；强制终止英雄联盟工作台也失败：{force_error}"
            )),
            (None, result) => result,
        }
    }
}

impl Drop for ManagedLeagueRuntime {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.shutdown_signal);
    }
}

fn restore_host(app: AppHandle, memory_mode: bool) {
    let state = app.state::<LeagueRuntimeProcess>();
    if state.suppress_restore.load(Ordering::SeqCst) {
        return;
    }
    if state.restoring.swap(true, Ordering::SeqCst) {
        return;
    }
    if state.suppress_restore.load(Ordering::SeqCst) {
        state.restoring.store(false, Ordering::SeqCst);
        return;
    }
    state.active.store(false, Ordering::SeqCst);
    state.administrator.store(false, Ordering::SeqCst);
    state.mode.store(MODE_ASK, Ordering::SeqCst);

    let backend_result = if memory_mode {
        start_backend(&app)
    } else {
        Ok(())
    };
    let window_result = if memory_mode && !state.suppress_restore.load(Ordering::SeqCst) {
        create_main_window(&app)
    } else {
        Ok(())
    };
    state.restoring.store(false, Ordering::SeqCst);

    if state.suppress_restore.load(Ordering::SeqCst) {
        return;
    }

    if let Err(error) = backend_result {
        if let Ok(mut last_error) = state.last_error.lock() {
            *last_error = Some(error.clone());
        }
        let _ = app.emit("league-runtime-restore-error", error);
    }
    if let Err(error) = window_result {
        if let Ok(mut last_error) = state.last_error.lock() {
            *last_error = Some(error.clone());
        }
        let _ = app.emit("league-runtime-restore-error", error);
    }
    let _ = app.emit("league-runtime-restored", ());
}

fn monitor_runtime(app: AppHandle) {
    let mut completion_error = None;
    loop {
        thread::sleep(Duration::from_millis(500));
        let exited = {
            let state = app.state::<LeagueRuntimeProcess>();
            let Ok(mut guard) = state.child.lock() else {
                return;
            };
            match guard.as_mut() {
                Some(runtime) => match runtime.process.has_exited() {
                    Ok(true) => {
                        completion_error = match runtime.process.completion_error() {
                            Ok(error) => error,
                            Err(error) => Some(error),
                        };
                        *guard = None;
                        true
                    }
                    Ok(false) => false,
                    Err(status_error) => match runtime.request_exit() {
                        Ok(()) => {
                            *guard = None;
                            true
                        }
                        Err(cleanup_error) => {
                            eprintln!(
                                "英雄联盟工作台状态查询失败，保留运行时以便后续清理：{status_error}；{cleanup_error}"
                            );
                            return;
                        }
                    },
                },
                None => true,
            }
        };
        if exited {
            break;
        }
    }
    let memory_mode = app
        .state::<LeagueRuntimeProcess>()
        .mode
        .load(Ordering::SeqCst)
        == MODE_MEMORY;
    if app
        .state::<LeagueRuntimeProcess>()
        .suppress_restore
        .load(Ordering::SeqCst)
    {
        let state = app.state::<LeagueRuntimeProcess>();
        state.active.store(false, Ordering::SeqCst);
        state.administrator.store(false, Ordering::SeqCst);
        state.mode.store(MODE_ASK, Ordering::SeqCst);
        return;
    }
    let event_app = app.clone();
    if let Some(error) = completion_error.as_ref() {
        if let Ok(mut last_error) = app.state::<LeagueRuntimeProcess>().last_error.lock() {
            *last_error = Some(error.clone());
        }
    }
    restore_host(app, memory_mode);
    if let Some(error) = completion_error {
        let _ = event_app.emit("league-runtime-restore-error", error);
    }
}

#[tauri::command]
pub(crate) async fn get_league_runtime_status(app: AppHandle) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<LeagueRuntimeProcess>();
        let manifest = manifest_value(&app);
        let executable = runtime_executable(&app).ok();
        let runtime_pid = state
            .child
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|runtime| runtime.process.pid()));
        let last_error = state
            .last_error
            .lock()
            .ok()
            .and_then(|error| error.clone());
        json!({
            "available": executable.is_some(),
            "administrator_available": executable.as_deref().is_some_and(administrator_launch_ready),
            "active": state.active.load(Ordering::SeqCst),
            "administrator": state.administrator.load(Ordering::SeqCst),
            "mode": mode_name(state.mode.load(Ordering::SeqCst)),
            "pid": runtime_pid,
            "host_working_set_bytes": process_tree_working_set_bytes(std::process::id()),
            "expected_runtime_memory_mb": manifest.as_ref().and_then(|value| value.get("expectedRuntimeMemoryMb")).cloned(),
            "source_version": manifest.as_ref().and_then(|value| value.get("sourceVersion")).cloned(),
            "last_error": last_error,
        })
    })
    .await
    .map_err(|error| format!("读取英雄联盟工作台状态失败：{error}"))
}

#[tauri::command]
pub(crate) async fn launch_league_runtime(
    app: AppHandle,
    mode: String,
    administrator: bool,
) -> Result<(), String> {
    let mode = parse_mode(&mode)?;
    let app_for_worker = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app_for_worker.state::<LeagueRuntimeProcess>();
        if state.suppress_restore.load(Ordering::SeqCst) {
            return Err("MaxGameStudio 正在退出，无法启动英雄联盟工作台".to_string());
        }
        if state.restoring.load(Ordering::SeqCst) {
            return Err("MaxGameStudio 正在恢复主窗口，请稍后重试".to_string());
        }
        if let Ok(mut last_error) = state.last_error.lock() {
            *last_error = None;
        }
        let executable = runtime_executable(&app_for_worker)?;
        if administrator && !administrator_launch_ready(&executable) {
            return Err(
                "当前系统缺少受保护的管理员启动链或完整运行时清单，已拒绝管理员启动；请重新安装完整的 MaxGameStudio 安装包"
                    .to_string(),
            );
        }
        if administrator {
            verify_runtime_integrity(&executable).map_err(|error| {
                format!("管理员启动前的英雄联盟工作台完整性检查未通过：{error}")
            })?;
        }
        if state.active.swap(true, Ordering::SeqCst) {
            return Err("内置英雄联盟工作台已在运行".to_string());
        }
        let user_data_dir = match runtime_user_data_dir(&app_for_worker) {
            Ok(path) => path,
            Err(error) => {
                state.active.store(false, Ordering::SeqCst);
                state.administrator.store(false, Ordering::SeqCst);
                state.mode.store(MODE_ASK, Ordering::SeqCst);
                return Err(error);
            }
        };
        let shutdown_signal = runtime_shutdown_signal(&user_data_dir);
        let _ = fs::remove_file(&shutdown_signal);
        let arguments = runtime_arguments(&user_data_dir, &shutdown_signal, std::process::id());
        let process = match launch_runtime_process(&executable, &arguments, administrator) {
            Ok(process) => process,
            Err(error) => {
                state.active.store(false, Ordering::SeqCst);
                state.administrator.store(false, Ordering::SeqCst);
                state.mode.store(MODE_ASK, Ordering::SeqCst);
                return Err(error);
            }
        };
        let mut runtime = ManagedLeagueRuntime {
            process,
            shutdown_signal,
        };
        let mut guard = match state.child.lock() {
            Ok(guard) => guard,
            Err(_) => {
                let cleanup_error = runtime.request_exit().err();
                state.active.store(false, Ordering::SeqCst);
                state.administrator.store(false, Ordering::SeqCst);
                state.mode.store(MODE_ASK, Ordering::SeqCst);
                return Err(match cleanup_error {
                    Some(error) => {
                        format!("英雄联盟运行时状态锁已损坏，且清理已启动进程失败：{error}")
                    }
                    None => "英雄联盟运行时状态锁已损坏".to_string(),
                });
            }
        };
        // Register the child before stopping the backend. Holding this lock
        // across the suppression check and registration closes the small
        // window in which app exit could otherwise observe child=None and
        // leave this newly-spawned process behind.
        if state.suppress_restore.load(Ordering::SeqCst) {
            let cleanup_error = runtime.request_exit().err();
            drop(guard);
            state.active.store(false, Ordering::SeqCst);
            state.administrator.store(false, Ordering::SeqCst);
            state.mode.store(MODE_ASK, Ordering::SeqCst);
            return Err(match cleanup_error {
                Some(error) => {
                    format!("MaxGameStudio 正在退出，且清理已启动的英雄联盟工作台失败：{error}")
                }
                None => "MaxGameStudio 正在退出，无法启动英雄联盟工作台".to_string(),
            });
        }
        if guard.is_some() {
            let cleanup_error = runtime.request_exit().err();
            drop(guard);
            // The child slot is authoritative. Preserve the existing
            // runtime's privilege/mode metadata and repair only the stale
            // active flag after disposing the duplicate process.
            state.active.store(true, Ordering::SeqCst);
            return Err(match cleanup_error {
                Some(error) => {
                    format!("英雄联盟运行时状态异常，且清理重复启动的工作台失败：{error}")
                }
                None => "英雄联盟运行时状态异常，拒绝覆盖现有工作台".to_string(),
            });
        }
        *guard = Some(runtime);
        state.administrator.store(administrator, Ordering::SeqCst);
        state.mode.store(mode, Ordering::SeqCst);
        drop(guard);

        if mode == MODE_MEMORY {
            stop_backend(&app_for_worker);
        }

        let monitor_app = app_for_worker.clone();
        if let Err(error) = thread::Builder::new()
            .name("league-runtime-monitor".to_string())
            .spawn(move || monitor_runtime(monitor_app))
        {
            let cleanup_result = {
                let state = app_for_worker.state::<LeagueRuntimeProcess>();
                let result = match state.child.lock() {
                    Ok(mut guard) => match guard.as_mut() {
                        Some(runtime) => {
                            let result = runtime.request_exit();
                            if result.is_ok() {
                                let _ = guard.take();
                            }
                            result
                        }
                        None => Ok(()),
                    },
                    Err(_) => Err("英雄联盟运行时状态锁已损坏".to_string()),
                };
                result
            };
            if cleanup_result.is_ok() {
                state.active.store(false, Ordering::SeqCst);
                state.administrator.store(false, Ordering::SeqCst);
                state.mode.store(MODE_ASK, Ordering::SeqCst);
                if mode == MODE_MEMORY {
                    let _ = start_backend(&app_for_worker);
                }
            }
            return Err(match cleanup_result {
                Ok(()) => format!("无法启动英雄联盟运行时监控线程：{error}"),
                Err(cleanup_error) => {
                    format!("无法启动英雄联盟运行时监控线程，且清理工作台失败：{cleanup_error}")
                }
            });
        }
        // Let the Tauri IPC response reach the renderer before destroying the
        // invoking WebView. Otherwise the JS coordinator can observe a false
        // rejection and clear its same-session relaunch guard even though the
        // runtime started successfully. The monitor waits 500 ms before its
        // first exit check, so this handoff still completes before restoration.
        if mode == MODE_MEMORY {
            let close_app = app_for_worker.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(150));
                close_host_webviews_for_memory_mode(&close_app);
            });
        }
        Ok(())
    })
    .await
    .map_err(|error| format!("英雄联盟工作台启动任务失败：{error}"))?
}

#[tauri::command]
pub(crate) fn stop_league_runtime(app: AppHandle) -> Result<(), String> {
    let state = app.state::<LeagueRuntimeProcess>();
    let mut guard = state
        .child
        .lock()
        .map_err(|_| "英雄联盟运行时状态锁已损坏".to_string())?;
    let Some(runtime) = guard.as_mut() else {
        return Ok(());
    };
    runtime.request_exit()
}

pub(crate) fn shutdown_league_runtime(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<LeagueRuntimeProcess>();
    state.suppress_restore.store(true, Ordering::SeqCst);
    state.active.store(false, Ordering::SeqCst);
    state.administrator.store(false, Ordering::SeqCst);
    state.mode.store(MODE_ASK, Ordering::SeqCst);
    let mut guard = state
        .child
        .lock()
        .map_err(|_| "英雄联盟运行时状态锁已损坏".to_string())?;
    let Some(runtime) = guard.as_mut() else {
        return Ok(());
    };
    runtime.request_exit()?;
    let _ = guard.take();
    Ok(())
}

pub(crate) fn suppress_runtime_restore(app: &AppHandle) {
    app.state::<LeagueRuntimeProcess>()
        .suppress_restore
        .store(true, Ordering::SeqCst);
}

pub(crate) fn wait_for_restore_idle(app: &AppHandle) {
    // start_backend has a bounded 12-second readiness loop. Give an in-flight
    // restore enough time to observe AppLifecycle::quitting and unwind before
    // the final backend reap and process exit.
    for _ in 0..130 {
        if !app
            .state::<LeagueRuntimeProcess>()
            .restoring
            .load(Ordering::SeqCst)
        {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::{
        mode_name, parse_mode, runtime_arguments, runtime_user_data_argument, sha256_file,
        verify_runtime_integrity_against, MODE_MEMORY, MODE_PARALLEL,
    };
    use std::{
        collections::BTreeMap,
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    #[cfg(windows)]
    use super::{
        admin_launcher_script, elevated_runtime_command_line, prepare_admin_launcher,
        protected_admin_session_root, quote_windows_argument, supervised_admin_command_line,
        transient_admin_launcher_path, ADMIN_LAUNCHER_SCRIPT,
    };

    #[test]
    fn runtime_modes_are_strict_and_round_trip() {
        assert_eq!(parse_mode("memory").unwrap(), MODE_MEMORY);
        assert_eq!(parse_mode("parallel").unwrap(), MODE_PARALLEL);
        assert_eq!(mode_name(MODE_MEMORY), "memory");
        assert_eq!(mode_name(MODE_PARALLEL), "parallel");
        assert!(parse_mode("ask").is_err());
        assert!(parse_mode("unknown").is_err());
    }

    #[test]
    fn embedded_runtime_uses_an_isolated_maxgamestudio_profile() {
        let source = include_str!("league_runtime.rs");
        assert!(source.contains("base.join(\"MaxGameStudio\").join(\"league-runtime\")"));
        assert!(source.contains("runtime_user_data_argument(user_data_dir)"));
        assert!(source.contains(".env(\"MAXGAMESTUDIO_EMBEDDED\", \"1\")"));
    }

    #[test]
    fn chromium_user_data_switch_uses_equals_form() {
        let argument = runtime_user_data_argument(Path::new(
            r"C:\Users\Tester\AppData\Roaming\MaxGameStudio\league-runtime",
        ));
        assert_eq!(
            argument,
            r"--user-data-dir=C:\Users\Tester\AppData\Roaming\MaxGameStudio\league-runtime"
        );
    }

    #[test]
    fn embedded_runtime_arguments_include_supervision_contract() {
        let arguments = runtime_arguments(
            Path::new(r"C:\Users\Tester\AppData\Roaming\MaxGameStudio\league-runtime"),
            Path::new(r"C:\Users\Tester\runtime-shutdown.request"),
            4242,
        );
        assert!(arguments
            .iter()
            .any(|value| value == "--maxgamestudio-embedded"));
        assert!(arguments
            .iter()
            .any(|value| value == "--maxgamestudio-host-pid=4242"));
        assert!(arguments.iter().any(|value| {
            value == r"--maxgamestudio-shutdown-signal=C:\Users\Tester\runtime-shutdown.request"
        }));
    }

    fn integrity_fixture(name: &str) -> (PathBuf, BTreeMap<String, String>) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "maxgamestudio-league-integrity-{name}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("resources")).unwrap();
        fs::write(root.join("MaxGameStudioLeague.exe"), b"runtime-executable").unwrap();
        fs::write(root.join("resources/app.asar"), b"runtime-asar").unwrap();
        let expected = ["MaxGameStudioLeague.exe", "resources/app.asar"]
            .into_iter()
            .map(|key| {
                let hash = sha256_file(&root.join(key)).unwrap();
                (key.to_string(), hash)
            })
            .collect();
        (root, expected)
    }

    #[test]
    fn administrator_integrity_accepts_exact_payload_and_mutable_logs() {
        let (root, expected) = integrity_fixture("valid");
        fs::write(root.join("maxgamestudio-runtime-hashes.json"), b"{}").unwrap();
        fs::write(root.join("debug.log"), b"runtime log").unwrap();
        fs::create_dir_all(root.join("logs")).unwrap();
        fs::write(root.join("logs/session.log"), b"runtime log").unwrap();
        verify_runtime_integrity_against(&root.join("MaxGameStudioLeague.exe"), &expected).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn administrator_integrity_rejects_modified_payload() {
        let (root, expected) = integrity_fixture("modified");
        fs::write(root.join("resources/app.asar"), b"modified-asar").unwrap();
        let error =
            verify_runtime_integrity_against(&root.join("MaxGameStudioLeague.exe"), &expected)
                .unwrap_err();
        assert!(error.contains("完整性校验失败"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn administrator_integrity_rejects_unlisted_runtime_file() {
        let (root, expected) = integrity_fixture("unlisted");
        fs::write(root.join("version.dll"), b"unlisted-library").unwrap();
        let error =
            verify_runtime_integrity_against(&root.join("MaxGameStudioLeague.exe"), &expected)
                .unwrap_err();
        assert!(error.contains("未签入清单"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn administrator_integrity_rejects_parent_traversal() {
        let (root, mut expected) = integrity_fixture("traversal");
        expected.insert("../outside.dll".to_string(), "0".repeat(64));
        let error =
            verify_runtime_integrity_against(&root.join("MaxGameStudioLeague.exe"), &expected)
                .unwrap_err();
        assert!(error.contains("不安全路径"));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn elevated_command_line_quotes_paths_with_spaces() {
        assert_eq!(
            quote_windows_argument(r"--user-data-dir=D:\Max Game Studio\league-runtime"),
            r#""--user-data-dir=D:\Max Game Studio\league-runtime""#
        );
        assert_eq!(
            quote_windows_argument("--maxgamestudio-embedded"),
            "--maxgamestudio-embedded"
        );
    }

    #[cfg(windows)]
    #[test]
    fn elevated_runtime_never_uses_the_callers_writable_chromium_profile() {
        let arguments = vec![
            r"--user-data-dir=C:\Users\Tester\AppData\Roaming\MaxGameStudio\league-runtime"
                .to_string(),
            "--maxgamestudio-embedded".to_string(),
            "--maxgamestudio-host-pid=4242".to_string(),
        ];
        let command_line = elevated_runtime_command_line(&arguments);
        assert!(!command_line.contains("user-data-dir"));
        assert!(command_line.contains("--maxgamestudio-embedded"));
        assert!(command_line.contains("--maxgamestudio-host-pid=4242"));
    }

    #[cfg(windows)]
    #[test]
    fn administrator_runtime_is_staged_on_the_source_volume() {
        let path = protected_admin_session_root(
            Path::new(r"D:\MaxGameStudio\league-runtime"),
            "session-0123456789abcdef0123456789abcdef",
        )
        .unwrap();
        assert_eq!(
            path,
            PathBuf::from(
                r"D:\MaxGameStudioAdminRuntime\Sessions\session-0123456789abcdef0123456789abcdef"
            )
        );
        let verbatim_path = protected_admin_session_root(
            Path::new(r"\\?\D:\MaxGameStudio\league-runtime"),
            "session-fedcba9876543210fedcba9876543210",
        )
        .unwrap();
        assert_eq!(
            verbatim_path,
            PathBuf::from(
                r"D:\MaxGameStudioAdminRuntime\Sessions\session-fedcba9876543210fedcba9876543210"
            )
        );
        assert!(protected_admin_session_root(
            Path::new(r"\\server\share\league-runtime"),
            "session-0123456789abcdef0123456789abcdef"
        )
        .is_err());
        assert!(protected_admin_session_root(
            Path::new(r"\\?\UNC\server\share\league-runtime"),
            "session-0123456789abcdef0123456789abcdef"
        )
        .is_err());
    }

    #[cfg(windows)]
    #[test]
    fn administrator_launcher_embeds_only_base64_configuration() {
        let script = admin_launcher_script(&serde_json::json!({
            "sourceRoot": r"D:\Max Game Studio\league-runtime",
            "manifestSha256": "a".repeat(64),
            "sessionName": "session-0123456789abcdef0123456789abcdef",
            "commandLine": r#""--user-data-dir=C:\Users\Tester\AppData\Roaming\MaxGameStudio\league-runtime" --maxgamestudio-embedded --maxgamestudio-host-pid=4242"#,
            "hostPid": 4242,
        }))
        .unwrap();
        let script = String::from_utf8(script).unwrap();
        assert!(!script.contains("__CONFIG_BASE64__"));
        assert!(script.contains("[Convert]::FromBase64String('"));
        assert!(!script.contains(r"D:\Max Game Studio"));
    }

    #[cfg(windows)]
    #[test]
    fn administrator_supervisor_executes_only_the_locked_file() {
        let command = supervised_admin_command_line(
            Path::new(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"),
            Path::new(r"D:\MaxGameStudioAdminLauncher-0123456789abcdef.ps1"),
        )
        .unwrap();
        assert!(command.starts_with(
            r#"/d /s /c ""C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe""#
        ));
        assert!(command.ends_with(r#"-File "D:\MaxGameStudioAdminLauncher-0123456789abcdef.ps1"""#));
        for unsafe_path in [
            r"D:\unsafe&whoami.ps1",
            r"D:\unsafe%PATH%.ps1",
            r"D:\unsafe!x!.ps1",
        ] {
            assert!(supervised_admin_command_line(
                Path::new(r"C:\Windows\powershell.exe"),
                Path::new(unsafe_path)
            )
            .is_err());
        }
    }

    #[cfg(windows)]
    #[test]
    fn administrator_launcher_is_randomized_locked_and_deleted_on_drop() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "MaxGameStudioAdminLauncher-test-{}-{unique}.ps1",
            std::process::id()
        ));
        let prepared = prepare_admin_launcher(
            path.clone(),
            &serde_json::json!({
                "sourceRoot": r"D:\MaxGameStudio\league-runtime",
                "manifestSha256": "a".repeat(64),
                "sessionName": "session-0123456789abcdef0123456789abcdef",
                "commandLine": "--maxgamestudio-embedded",
                "hostPid": 4242,
            }),
        )
        .unwrap();
        assert!(path.is_file());
        assert!(fs::OpenOptions::new().write(true).open(&path).is_err());
        drop(prepared);
        assert!(!path.exists());
    }

    #[cfg(windows)]
    #[test]
    fn administrator_launcher_uses_a_fixed_safe_path_on_the_source_volume() {
        let path = transient_admin_launcher_path(
            Path::new(r"D:\MaxGameStudio\league-runtime"),
            "session-0123456789abcdef0123456789abcdef",
        )
        .unwrap();
        assert_eq!(
            path,
            PathBuf::from(r"D:\MaxGameStudioAdminLauncher-0123456789abcdef0123456789abcdef.ps1")
        );
    }

    #[cfg(windows)]
    #[test]
    fn administrator_launcher_revalidates_and_locks_a_protected_copy() {
        assert!(ADMIN_LAUNCHER_SCRIPT.contains("New-ProtectedDirectory $sessionRoot"));
        assert!(ADMIN_LAUNCHER_SCRIPT.contains("Ensure-ProtectedDirectory $parent"));
        assert!(ADMIN_LAUNCHER_SCRIPT.contains("$profilesRoot"));
        assert!(ADMIN_LAUNCHER_SCRIPT.contains("$protectedUserDataArgument"));
        assert!(ADMIN_LAUNCHER_SCRIPT.contains("Remove-ProtectedSession $sessionRoot"));
        assert!(!ADMIN_LAUNCHER_SCRIPT.contains("Grant-StandardUserCleanup"));
        assert!(ADMIN_LAUNCHER_SCRIPT.contains("manifestSha256"));
        assert!(
            ADMIN_LAUNCHER_SCRIPT.contains("The runtime manifest does not match the signed host")
        );
        assert!(ADMIN_LAUNCHER_SCRIPT.contains("[IO.FileShare]::Read"));
        assert!(!ADMIN_LAUNCHER_SCRIPT.contains("Add-Type"));
        assert!(!ADMIN_LAUNCHER_SCRIPT.contains("Open-DirectoryLock"));
        assert!(ADMIN_LAUNCHER_SCRIPT.contains("NODE|ELECTRON"));
        assert!(ADMIN_LAUNCHER_SCRIPT.contains("$env:PSModulePath"));
        assert!(ADMIN_LAUNCHER_SCRIPT.contains("$protectedTemp"));
        assert!(ADMIN_LAUNCHER_SCRIPT.contains("MAXGAMESTUDIO_HOST_PID"));
        assert!(!ADMIN_LAUNCHER_SCRIPT.contains("Invoke-Expression"));
    }

    #[test]
    fn only_memory_mode_destroys_the_host_webview() {
        let source = include_str!("league_runtime.rs");
        let launch = source
            .split_once("pub(crate) async fn launch_league_runtime")
            .and_then(|(_, body)| body.split_once("pub(crate) fn stop_league_runtime"))
            .map(|(body, _)| body)
            .expect("launch command should exist");
        let memory_guard = launch
            .find("if mode == MODE_MEMORY")
            .expect("memory mode should guard host WebView destruction");
        let close_worker = launch
            .find("thread::sleep(Duration::from_millis(150))")
            .expect("host close should be deferred");
        let close_call = launch
            .find("close_host_webviews_for_memory_mode(&close_app)")
            .expect("deferred worker should close the host WebViews");
        assert!(memory_guard < close_worker);
        assert!(close_worker < close_call);
        assert!(!launch.contains("close_host_webviews_for_parallel_mode"));
    }

    #[test]
    fn runtime_child_is_registered_before_memory_backend_stop() {
        let source = include_str!("league_runtime.rs");
        let launch = source
            .split_once("pub(crate) async fn launch_league_runtime")
            .and_then(|(_, body)| body.split_once("pub(crate) fn stop_league_runtime"))
            .map(|(body, _)| body)
            .expect("launch command should exist");
        let process = launch
            .find("let process = match launch_runtime_process")
            .expect("runtime process should be spawned");
        let guard = launch
            .find("let mut guard = match state.child.lock()")
            .expect("runtime state should be locked");
        let registration = launch
            .find("*guard = Some(runtime)")
            .expect("spawned runtime should be registered");
        let backend_stop = launch
            .find("stop_backend(&app_for_worker)")
            .expect("memory mode should stop the backend");
        assert!(process < guard);
        assert!(guard < registration);
        assert!(registration < backend_stop);
        assert!(launch[guard..registration].contains("suppress_restore"));
    }

    #[test]
    fn administrator_integrity_is_verified_before_process_spawn() {
        let source = include_str!("league_runtime.rs");
        let launch = source
            .split_once("pub(crate) async fn launch_league_runtime")
            .and_then(|(_, body)| body.split_once("pub(crate) fn stop_league_runtime"))
            .map(|(body, _)| body)
            .expect("launch command should exist");
        let verification = launch
            .find("verify_runtime_integrity(&executable)")
            .expect("administrator launch should verify the embedded runtime");
        let activation = launch
            .find("state.active.swap(true, Ordering::SeqCst)")
            .expect("runtime should have an active-state guard");
        let spawn = launch
            .find("launch_runtime_process(&executable, &arguments, administrator)")
            .expect("runtime process should be spawned");
        assert!(verification < activation);
        assert!(activation < spawn);
    }

    #[test]
    fn monitor_keeps_the_child_when_status_cleanup_cannot_be_confirmed() {
        let source = include_str!("league_runtime.rs");
        let implementation = source
            .split_once("#[cfg(test)]")
            .map(|(implementation, _)| implementation)
            .expect("runtime implementation should precede tests");
        let monitor = implementation
            .split_once("fn monitor_runtime")
            .and_then(|(_, body)| body.split_once("#[tauri::command]"))
            .map(|(body, _)| body)
            .expect("monitor function should exist");
        let status_error = monitor
            .find("Err(status_error)")
            .expect("monitor should handle status errors");
        let cleanup_error = monitor
            .find("Err(cleanup_error)")
            .expect("monitor should handle cleanup errors");
        let retain = monitor
            .find("保留运行时以便后续清理")
            .expect("monitor should retain an unconfirmed child");
        assert!(status_error < cleanup_error);
        assert!(cleanup_error < retain);
    }

    #[cfg(windows)]
    #[test]
    fn elevated_force_termination_checks_tree_and_wait_result() {
        let source = include_str!("league_runtime.rs");
        let implementation = source
            .split_once("#[cfg(test)]")
            .map(|(implementation, _)| implementation)
            .expect("runtime implementation should precede tests");
        let force = implementation
            .split_once("fn force_terminate")
            .and_then(|(_, body)| body.split_once("impl Drop for ManagedRuntimeProcess"))
            .map(|(body, _)| body)
            .expect("force termination implementation should exist");
        assert!(force.contains("taskkill"));
        assert!(force.contains("/t"));
        assert!(force.contains("WAIT_TIMEOUT => Err"));
        assert!(!force
            .contains("let _ = unsafe { WaitForSingleObject(runtime.handle as HANDLE, 3_000) }"));
    }
}
