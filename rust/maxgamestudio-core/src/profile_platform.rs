//! Read-only platform probes used by the profile and startup routes.

use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    env, fs, io,
    path::{Component, Path, PathBuf},
};

pub(crate) fn resolved_path(path: &Path) -> io::Result<PathBuf> {
    fs::canonicalize(path).or_else(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            std::path::absolute(path)
        } else {
            Err(error)
        }
    })
}

pub(crate) fn path_text(path: &Path) -> String {
    let text = path.to_string_lossy();
    #[cfg(windows)]
    {
        if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{unc}");
        }
        if let Some(drive) = text.strip_prefix(r"\\?\") {
            return drive.to_owned();
        }
    }
    text.into_owned()
}

pub(crate) fn expand_user(value: &str) -> io::Result<PathBuf> {
    let path = Path::new(value);
    let path = if value.starts_with('~') {
        let mut components = path.components();
        let user = components.next().unwrap().as_os_str().to_string_lossy();
        let home =
            home_directory().ok_or_else(|| io::Error::other("home directory is unavailable"))?;
        let root = if user == "~" {
            home
        } else {
            expand_named_home(&home, &user[1..])
                .ok_or_else(|| io::Error::other("home directory is unavailable"))?
        };
        root.join(components.as_path())
    } else {
        path.to_owned()
    };
    let path = path
        .components()
        .filter(|part| *part != Component::CurDir)
        .collect::<PathBuf>();
    Ok(if path.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        path
    })
}

#[cfg(windows)]
fn home_directory() -> Option<PathBuf> {
    env::var_os("USERPROFILE").map(PathBuf::from).or_else(|| {
        let mut home = env::var_os("HOMEDRIVE").unwrap_or_default();
        home.push(env::var_os("HOMEPATH")?);
        Some(PathBuf::from(home))
    })
}

#[cfg(not(windows))]
fn home_directory() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from)
}

#[cfg(windows)]
fn expand_named_home(home: &Path, user: &str) -> Option<PathBuf> {
    let current = env::var_os("USERNAME")?;
    if current == user {
        Some(home.to_owned())
    } else if home.file_name()? == current {
        Some(home.parent()?.join(user))
    } else {
        None
    }
}

#[cfg(not(windows))]
fn expand_named_home(home: &Path, user: &str) -> Option<PathBuf> {
    (home.file_name()? == user).then(|| home.to_owned())
}

pub(crate) fn effective_locale(config: &str) -> &'static str {
    match config {
        "auto" => system_locale(&system_locale_name()),
        "zh-HK" => "zh-HK",
        "zh-TW" => "zh-TW",
        "en" => "en",
        "ms-MY" => "ms-MY",
        "ru-RU" => "ru-RU",
        _ => "zh",
    }
}

fn system_locale(locale: &str) -> &'static str {
    let value = locale.to_ascii_lowercase().replace('_', "-");
    for (prefix, supported) in [
        ("zh-hk", "zh-HK"),
        ("zh-mo", "zh-HK"),
        ("zh-tw", "zh-TW"),
        ("zh", "zh"),
        ("ms", "ms-MY"),
        ("ru", "ru-RU"),
    ] {
        if value.starts_with(prefix) {
            return supported;
        }
    }
    "en"
}

#[cfg(windows)]
fn system_locale_name() -> String {
    use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;
    // LOCALE_NAME_MAX_LENGTH in the Windows SDK.
    let mut buffer = [0u16; 85];
    // Win32 writes at most the buffer's declared size, including its terminator.
    let length = unsafe { GetUserDefaultLocaleName(buffer.as_mut_ptr(), buffer.len() as i32) };
    if length > 0 {
        String::from_utf16_lossy(&buffer[..length as usize - 1])
    } else {
        String::new()
    }
}

#[cfg(not(windows))]
fn system_locale_name() -> String {
    ["LC_ALL", "LC_CTYPE", "LANG", "LANGUAGE"]
        .iter()
        .find_map(|name| env::var(name).ok().filter(|value| !value.is_empty()))
        .unwrap_or_default()
}

pub(crate) fn ffmpeg_on_path() -> bool {
    ffmpeg_from_path().is_some()
}

pub(crate) fn ffmpeg_from_path() -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    #[cfg(windows)]
    let names = env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned())
        .split(';')
        .filter(|extension| !extension.is_empty())
        .map(|extension| format!("ffmpeg{extension}"))
        .collect::<Vec<_>>();
    #[cfg(not(windows))]
    let names = ["ffmpeg"];
    env::split_paths(&path).find_map(|directory| {
        names
            .iter()
            .map(|name| directory.join(name))
            .find(|file| is_executable(file))
    })
}

#[cfg(windows)]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

pub(crate) fn detect_cs2_path() -> Option<PathBuf> {
    let mut roots = registry_steam_path().into_iter().collect::<Vec<_>>();
    roots.extend(
        [
            r"C:\Program Files (x86)\Steam",
            r"C:\Program Files\Steam",
            r"D:\Steam",
            r"D:\SteamLibrary",
            r"E:\Steam",
            r"F:\Steam",
        ]
        .into_iter()
        .map(PathBuf::from),
    );
    if let Some(programs) = env::var_os("ProgramFiles(x86)").or_else(|| env::var_os("ProgramFiles"))
    {
        roots.push(Path::new(&programs).join("Steam"));
    }
    detect_cs2_in_roots(roots)
}

fn detect_cs2_in_roots(mut roots: Vec<PathBuf>) -> Option<PathBuf> {
    roots.retain(|root| root.is_dir());
    let relative =
        Path::new("steamapps/common/Counter-Strike Global Offensive/game/bin/win64/cs2.exe");
    if let Some(path) = roots
        .iter()
        .map(|root| root.join(relative))
        .find(|path| path.is_file())
    {
        return Some(path);
    }
    for root in roots {
        let Ok(bytes) = fs::read(root.join("config/libraryfolders.vdf")) else {
            continue;
        };
        let vdf = String::from_utf8_lossy(&bytes);
        for (start, label) in vdf.match_indices(r#""path""#) {
            let tail = &vdf[start + label.len()..];
            if !tail.starts_with(char::is_whitespace) {
                continue;
            }
            let Some((path, _)) = tail
                .trim_start()
                .strip_prefix('"')
                .and_then(|value| value.split_once('"'))
            else {
                continue;
            };
            let unescaped = path.replace(r"\\", r"\");
            let path = unescaped.trim();
            if !path.is_empty() {
                let executable = Path::new(path).join(relative);
                if executable.is_file() {
                    return Some(executable);
                }
            }
        }
    }
    None
}

#[cfg(windows)]
fn registry_steam_path() -> Option<PathBuf> {
    use std::ptr::null_mut;
    use windows_sys::Win32::{
        Foundation::ERROR_SUCCESS,
        System::Registry::{RegGetValueW, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ},
    };
    let value = "InstallPath\0".encode_utf16().collect::<Vec<_>>();
    for (hive, key) in [
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Valve\Steam"),
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\Valve\Steam"),
        (HKEY_CURRENT_USER, r"Software\Valve\Steam"),
    ] {
        let key = key.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let mut length = 0;
        // Query the value's byte count before allocating a buffer for its UTF-16 data.
        let status = unsafe {
            RegGetValueW(
                hive,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                null_mut(),
                null_mut(),
                &mut length,
            )
        };
        if status != ERROR_SUCCESS || length == 0 || length > 65536 {
            continue;
        }
        let mut buffer = vec![0u16; (length as usize).div_ceil(2)];
        // The allocated buffer covers the byte count passed to Win32.
        let status = unsafe {
            RegGetValueW(
                hive,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut length,
            )
        };
        if status == ERROR_SUCCESS {
            let path = String::from_utf16_lossy(&buffer);
            let root = PathBuf::from(path.trim_end_matches('\0').trim());
            if root.exists() {
                return Some(root);
            }
        }
    }
    None
}

#[cfg(not(windows))]
fn registry_steam_path() -> Option<PathBuf> {
    None
}

pub(crate) fn backup_status(data_dir: &Path) -> io::Result<Value> {
    let root = data_dir.join(".cs2_config_backup");
    let state = read_backup_object(&root.join("recording_state.json"));
    let manifest = read_backup_object(&root.join("manifest.json"));
    let restore_required = state["status"] == "recording";
    let created_at = state["started_at_iso"]
        .as_str()
        .filter(|value| !value.is_empty())
        .or_else(|| manifest["created_at_iso"].as_str())
        .unwrap_or_default();
    let accounts = manifest["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(backup_account)
        .collect::<BTreeSet<_>>();
    Ok(json!({
        "status": if restore_required { "recording" } else { "recorded" },
        "restore_required": restore_required,
        "backup_dir": path_text(&resolved_path(&root)?),
        "created_at_iso": created_at,
        "accounts": accounts,
        "cs2_running": cs2_running(),
    }))
}

fn read_backup_object(path: &Path) -> Value {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null)
}

fn backup_account(entry: &Value) -> Option<String> {
    let id = &entry["account_id"];
    if let Some(id) = id.as_str().filter(|id| !id.is_empty()) {
        return Some(id.to_owned());
    }
    if id.as_u64().is_some_and(|id| id != 0) {
        return Some(id.to_string());
    }
    let mut parts = entry["original"].as_str()?.split(['/', '\\']);
    parts.find(|part| part.eq_ignore_ascii_case("userdata"))?;
    parts
        .next()
        .filter(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
        .map(str::to_owned)
}

#[cfg(windows)]
fn cs2_running() -> bool {
    use std::{
        mem::size_of,
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    };
    use windows_sys::Win32::{
        Foundation::INVALID_HANDLE_VALUE,
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
    };
    // Take ownership of the new, valid snapshot handle once.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return false;
    }
    let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot) };
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let handle = snapshot.as_raw_handle();
    // The process entry buffer is initialized with the size required by Win32.
    let mut found = unsafe { Process32FirstW(handle, &mut entry) };
    while found != 0 {
        let end = entry
            .szExeFile
            .iter()
            .position(|&value| value == 0)
            .unwrap_or(entry.szExeFile.len());
        if String::from_utf16_lossy(&entry.szExeFile[..end]).eq_ignore_ascii_case("cs2.exe") {
            return true;
        }
        found = unsafe { Process32NextW(handle, &mut entry) };
    }
    false
}

#[cfg(not(windows))]
fn cs2_running() -> bool {
    false
}

pub(crate) fn data_dir_info(data: &Path) -> io::Result<Value> {
    let mut size = 0u64;
    let mut pending = vec![data.to_owned()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                pending.push(entry.path());
            } else if let Ok(metadata) = fs::metadata(entry.path()) {
                if metadata.is_file() {
                    size = size.saturating_add(metadata.len());
                }
            }
        }
    }
    let size_str = match size {
        0..=1023 => format!("{size} B"),
        1024..=1_048_575 => format!("{:.1} KB", size as f64 / 1024.0),
        1_048_576..=1_073_741_823 => format!("{:.1} MB", size as f64 / 1_048_576.0),
        _ => format!("{:.2} GB", size as f64 / 1_073_741_824.0),
    };
    Ok(json!({
        "path": path_text(&resolved_path(data)?),
        "logs_path": path_text(&resolved_path(&data.join("logs"))?),
        "exists": data.exists(),
        "size_bytes": size,
        "size_str": size_str,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_cs2_in_direct_root_then_linked_unicode_library() {
        let scratch = tempfile::tempdir().unwrap();
        let steam = scratch.path().join("Steam");
        let library = scratch.path().join("游戏 Library");
        fs::create_dir_all(steam.join("config")).unwrap();
        let vdf = format!(
            "\"libraryfolders\"\n{{\n\"1\" {{ \"path\" \"{}\" }}\n}}",
            path_text(&library).replace('\\', r"\\")
        );
        fs::write(steam.join("config/libraryfolders.vdf"), vdf).unwrap();
        let executable =
            Path::new("steamapps/common/Counter-Strike Global Offensive/game/bin/win64/cs2.exe");
        let linked = library.join(executable);
        fs::create_dir_all(linked.parent().unwrap()).unwrap();
        fs::write(&linked, []).unwrap();
        assert_eq!(detect_cs2_in_roots(vec![steam.clone()]), Some(linked));
        let direct = steam.join(executable);
        fs::create_dir_all(direct.parent().unwrap()).unwrap();
        fs::write(&direct, []).unwrap();
        assert_eq!(detect_cs2_in_roots(vec![steam]), Some(direct));
    }

    #[test]
    fn legacy_locale_mapping_accepts_regional_preferences() {
        assert_eq!(system_locale("zh_MO"), "zh-HK");
        assert_eq!(system_locale("zh-TW"), "zh-TW");
        assert_eq!(system_locale("ms-MY"), "ms-MY");
        assert_eq!(system_locale("ru_RU"), "ru-RU");
        assert_eq!(system_locale("de-DE"), "en");
        assert_eq!(effective_locale("fr-FR"), "zh");
    }
}
