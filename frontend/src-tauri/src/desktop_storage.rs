//! One storage root for application-owned files. Registry holds only a locator;
//! existing data stays in place; changing the locator never copies/deletes data.
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

#[derive(Clone, Debug)]
struct StorageLayout {
    root: PathBuf,
    // Only legacy installations need separate, existing component paths.
    legacy: Option<(PathBuf, PathBuf, PathBuf)>,
}

impl StorageLayout {
    fn path(&self, name: &str) -> PathBuf {
        if let Some((data, webview, league)) = &self.legacy {
            for (prefix, base) in [
                ("data", data),
                ("webview", webview),
                ("league-runtime", league),
            ] {
                if let Ok(relative) = Path::new(name).strip_prefix(prefix) {
                    return base.join(relative);
                }
            }
        }
        self.root.join(name)
    }
}

static ROOT: OnceLock<StorageLayout> = OnceLock::new();
pub(crate) fn root() -> Result<PathBuf, String> {
    let path = ROOT
        .get()
        .map(|layout| layout.root.clone())
        .ok_or_else(|| "应用存储尚未初始化".to_string())?;
    if !path.is_dir() {
        return Err("已选存储目录不再可用，请检查磁盘；不会自动回退系统盘".into());
    }
    Ok(path)
}
pub(crate) fn directory(name: &str) -> Result<PathBuf, String> {
    root()?; // An unavailable selected disk must not silently create a new root.
    let path = ROOT.get().ok_or("应用存储尚未初始化")?.path(name);
    fs::create_dir_all(&path).map_err(|e| format!("无法访问存储目录 {}：{e}", path.display()))?;
    Ok(path)
}

fn distinct_trees(source: &Path, destination: &Path) -> bool {
    !destination.starts_with(source) && !source.starts_with(destination)
}

#[cfg(windows)]
mod windows {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt, sync::Mutex};
    use windows_sys::Win32::{
        Foundation::{
            CloseHandle, ERROR_FILE_NOT_FOUND, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT,
        },
        Storage::FileSystem::GetDriveTypeW,
        System::Registry::{
            RegCloseKey, RegCreateKeyExW, RegGetValueW, RegSetValueExW, HKEY_CURRENT_USER,
            KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
        },
        System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject},
    };
    static SETTINGS_LOCK: Mutex<()> = Mutex::new(());
    const KEY: &str = "Software\\MaxGameStudio\\Storage";
    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }
    // Bootstrap precedes Tauri's single-instance plugin. Keep the locator read,
    // layout selection and locator commit in one cross-process section.
    pub(super) struct StartupLock(HANDLE);
    impl StartupLock {
        pub(super) fn acquire(identity: &OsStr) -> Result<Option<Self>, String> {
            let digest = Sha256::digest(identity.to_string_lossy().to_lowercase().as_bytes());
            // HKCU is shared across a user's logon sessions, so use the global
            // mutex namespace. This is a mutex, not a privileged file mapping.
            // Default Windows object ACL; no filesystem or system TEMP writes.
            let name = wide(OsStr::new(&format!(
                "Global\\MaxGameStudio.StorageBootstrap.{digest:x}"
            )));
            let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
            if handle.is_null() {
                return Err(format!(
                    "无法保护存储初始化：{}",
                    std::io::Error::last_os_error()
                ));
            }
            // A duplicate must exit before reading settings. Waiting here would
            // keep another desktop process alive during initialization.
            let result = unsafe { WaitForSingleObject(handle, 0) };
            if result == WAIT_OBJECT_0 || result == WAIT_ABANDONED {
                // An interrupted owner may have left a pending location change.
                // Re-read and validate persisted state; never copy user data.
                Ok(Some(Self(handle)))
            } else if result == WAIT_TIMEOUT {
                unsafe { CloseHandle(handle) };
                Ok(None)
            } else {
                let error = std::io::Error::last_os_error();
                unsafe { CloseHandle(handle) };
                Err(format!("无法等待存储初始化：{error}"))
            }
        }
    }
    impl Drop for StartupLock {
        fn drop(&mut self) {
            unsafe {
                ReleaseMutex(self.0);
                CloseHandle(self.0);
            }
        }
    }
    #[cfg(test)]
    pub(super) fn abandon_lock_for_test(identity: &OsStr) -> isize {
        let guard = StartupLock::acquire(identity).unwrap().unwrap();
        let handle = guard.0 as isize;
        // Keep a handle open while this test thread exits owning the mutex.
        std::mem::forget(guard);
        handle
    }
    fn settings() -> Result<Value, String> {
        let key = wide(OsStr::new(KEY));
        let name = wide(OsStr::new("LocationV1"));
        let mut bytes = 0;
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut bytes,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(json!({}));
        }
        if status != 0 || bytes > 65536 {
            return Err(format!("无法读取存储位置设置：{status}"));
        }
        let mut buffer = vec![0u16; (bytes as usize).div_ceil(2)];
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if status != 0 {
            return Err(format!("无法读取存储位置：{status}"));
        }
        let text = String::from_utf16_lossy(&buffer)
            .trim_end_matches('\0')
            .to_string();
        let value: Value =
            serde_json::from_str(&text).map_err(|e| format!("存储位置设置损坏：{e}"))?;
        if !value.is_object() {
            return Err("存储位置设置不是有效对象，未修改数据".into());
        }
        Ok(value)
    }
    fn save(value: &Value) -> Result<(), String> {
        let mut handle = std::ptr::null_mut();
        let key = wide(OsStr::new(KEY));
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                std::ptr::null(),
                &mut handle,
                std::ptr::null_mut(),
            )
        };
        if status != 0 {
            return Err(format!("无法保存存储位置：{status}"));
        }
        let value = wide(OsStr::new(&value.to_string()));
        let name = wide(OsStr::new("LocationV1"));
        let status = unsafe {
            RegSetValueExW(
                handle,
                name.as_ptr(),
                0,
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * 2) as u32,
            )
        };
        unsafe {
            RegCloseKey(handle);
        }
        if status != 0 {
            return Err(format!("无法保存存储位置：{status}"));
        }
        Ok(())
    }
    fn system_drive(path: &Path) -> bool {
        let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        path.to_string_lossy()
            .to_ascii_lowercase()
            .starts_with(&drive.to_ascii_lowercase())
    }
    pub(super) fn validate_path(path: &Path) -> Result<PathBuf, String> {
        use std::path::{Component, Prefix};
        let mut parts = path.components();
        if !matches!(parts.next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_)))
            || !matches!(parts.next(), Some(Component::RootDir))
            || !parts.clone().all(|p| matches!(p, Component::Normal(_)))
            || parts.count() == 0
        {
            return Err("请选择本地磁盘内的专用数据文件夹，不要选择盘符根目录或网络路径".into());
        }
        // These paths also cross the fixed cmd.exe elevation supervisor parser.
        if path.to_string_lossy().chars().any(|c| {
            matches!(
                c,
                '"' | '%' | '!' | '&' | '|' | '<' | '>' | '^' | '\r' | '\n'
            )
        }) {
            return Err("数据目录含有管理员启动不支持的特殊字符，请选择其他文件夹".into());
        }
        let mut ancestor = Some(path);
        while let Some(current) = ancestor {
            match fs::symlink_metadata(current) {
                Ok(meta) => {
                    use std::os::windows::fs::MetadataExt;
                    if meta.file_attributes() & 0x400 != 0 {
                        return Err("数据目录不能经过目录链接或重解析点，请选择实际路径".into());
                    }
                    if !meta.is_dir() {
                        return Err("数据目录包含非文件夹路径".into());
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!("无法验证存储路径 {}：{error}", current.display()))
                }
            }
            ancestor = current.parent();
        }
        Ok(path.to_path_buf())
    }
    fn suggest() -> Option<PathBuf> {
        let profile = std::env::var_os("USERPROFILE")?;
        let identity = format!(
            "{:x}",
            Sha256::digest(profile.to_string_lossy().to_lowercase().as_bytes())
        );
        for letter in b'D'..=b'Z' {
            let drive = PathBuf::from(format!("{}:\\", char::from(letter)));
            // Win32 DRIVE_FIXED = 3. Never choose an absent/removable/network drive.
            if system_drive(&drive)
                || unsafe { GetDriveTypeW(wide(drive.as_os_str()).as_ptr()) } != 3
            {
                continue;
            }
            return Some(drive.join("MaxGameStudioData").join(&identity[..16]));
        }
        None
    }
    fn choose() -> Result<PathBuf, String> {
        let mut dialog =
            rfd::FileDialog::new().set_title("选择 MaxGameStudio 数据位置（推荐非系统盘）");
        if let Some(suggested) = suggest() {
            dialog = dialog.set_directory(suggested.parent().unwrap());
        }
        let parent = dialog
            .pick_folder()
            .ok_or_else(|| "已取消选择存储位置，未修改数据".to_string())?;
        let target = validate_path(&parent)?;
        if system_drive(&target)
            && rfd::MessageDialog::new()
                .set_title("确认使用系统盘")
                .set_description(
                    "该目录位于系统盘。缓存、日志和对局数据可能占用较多空间。仍要使用吗？",
                )
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                != rfd::MessageDialogResult::Yes
        {
            return Err("已取消使用系统盘".into());
        }
        Ok(target)
    }
    fn has_payload(data: &Path) -> bool {
        ["cs2-insight.config.json", "cs2-insight.db"]
            .iter()
            .any(|name| data.join(name).is_file())
    }

    fn known_root(path: &Path) -> bool {
        path.join(".storage-location-v2.json").is_file()
            || path.join(".storage-migration-v1.json").is_file()
            || has_payload(&path.join("data"))
    }

    // Detection is read-only. Never turn a legacy data root into a copy source
    // during startup, and never make its logs/caches hide an older real database.
    fn legacy_layout(appdata: &Path, local: &Path) -> Option<StorageLayout> {
        let candidates = [
            appdata.join("CS2 Insight Agent/data"),
            appdata.join("com.cs2insightagent.app/data"),
            appdata.join("cs2-insight-agent/data"),
            appdata.join("com.cs2insightagent.app"),
            appdata.join("cs2-insight-agent"),
        ];
        let data = candidates.iter().find(|path| has_payload(path)).cloned();
        let webview = local.join("com.cs2insightagent.app");
        let league = appdata.join("MaxGameStudio/league-runtime");
        if data.is_none() && !webview.is_dir() && !league.is_dir() {
            // Unknown legacy files are still user data; retain them in place.
            if !candidates[..3].iter().any(|path| path.is_dir()) {
                return None;
            }
        }
        let data = data
            .or_else(|| candidates[..3].iter().find(|p| p.is_dir()).cloned())
            .unwrap_or_else(|| candidates[0].clone());
        let root = if data.file_name().is_some_and(|name| name == "data") {
            data.parent()?.to_path_buf()
        } else {
            data.clone()
        };
        Some(StorageLayout {
            root,
            legacy: Some((data, webview, league)),
        })
    }

    fn layout_from_config(config: &Value, root: PathBuf) -> Result<StorageLayout, String> {
        if let Some(required) = config["required_paths"].as_array() {
            for path in required {
                let path = Path::new(path.as_str().ok_or("已保存的数据路径格式错误")?);
                validate_path(path)?;
                if !path.is_dir() || fs::read_dir(path).is_err() {
                    return Err(format!(
                        "原有数据目录暂不可用：{}。为避免空配置启动，未创建替代目录。",
                        path.display()
                    ));
                }
            }
        }
        let legacy = if config["mode"] == "legacy_in_place" {
            let paths = &config["paths"];
            let read = |name: &str| -> Result<PathBuf, String> {
                let path = paths[name]
                    .as_str()
                    .ok_or("旧版存储路径设置不完整，未修改数据")?;
                validate_path(Path::new(path))
            };
            Some((read("data")?, read("webview")?, read("league_runtime")?))
        } else {
            None
        };
        Ok(StorageLayout { root, legacy })
    }

    fn save_layout(config: &mut Value, layout: &StorageLayout) {
        config["root"] = json!(layout.root);
        config["mode"] = json!(if layout.legacy.is_some() {
            "legacy_in_place"
        } else {
            "unified"
        });
        config["paths"] = if let Some((data, webview, league)) = &layout.legacy {
            json!({"data": data, "webview": webview, "league_runtime": league})
        } else {
            Value::Null
        };
        // Remember only components that already exist; a legacy installation
        // may never have used League/WebView yet. Missing known data is not a
        // request to replace it with an empty configuration on the next launch.
        config["required_paths"] = json!(["data", "webview", "league-runtime"]
            .iter()
            .map(|name| layout.path(name))
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>());
        config["pending"] = Value::Null;
        config["pending_kind"] = Value::Null;
    }

    fn prepare_root(path: &Path) -> Result<(), String> {
        validate_path(path)?;
        if path.is_dir()
            && !known_root(path)
            && fs::read_dir(path)
                .map_err(|e| e.to_string())?
                .next()
                .is_some()
        {
            return Err(
                "所选目录包含其他数据，请选择空文件夹或已有的 MaxGameStudio 数据目录；不会覆盖文件"
                    .into(),
            );
        }
        fs::create_dir_all(path).map_err(|e| format!("无法使用存储目录：{e}"))?;
        if !known_root(path) {
            use std::io::Write;
            let mut marker = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path.join(".storage-location-v2.json"))
                .map_err(|e| e.to_string())?;
            marker
                .write_all(b"{\"version\":2}\n")
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    // Inputs are explicit so all selection/switch tests use fixtures, not the
    // developer's registry, profile, or game settings.
    pub(super) fn select_layout(
        config: &mut Value,
        suggested: &Path,
        appdata: &Path,
        local: &Path,
    ) -> Result<StorageLayout, String> {
        // Commit locator changes only after validation. A failed requested switch
        // must not strand the user outside the UI that can cancel/retry it.
        let mut candidate = config.clone();
        match select_layout_inner(&mut candidate, suggested, appdata, local) {
            Ok(layout) => {
                if config["pending_kind"] == "switch-only-v2" {
                    candidate["last_switch_error"] = Value::Null;
                }
                *config = candidate;
                Ok(layout)
            }
            Err(error) => {
                if config["pending_kind"] != "switch-only-v2"
                    || !config["pending"].is_string()
                    || !config["root"].is_string()
                {
                    return Err(error);
                }
                let mut restored = config.clone();
                restored["pending"] = Value::Null;
                restored["pending_kind"] = Value::Null;
                // Revalidate the original root AND all known components. Never
                // replace missing original data with an empty fallback profile.
                let layout = select_layout_inner(&mut restored, suggested, appdata, local)
                    .map_err(|original_error| {
                        format!("更改存储位置失败：{error}；原目录也不可用：{original_error}")
                    })?;
                restored["last_switch_error"] = json!(format!(
                    "未能切换至 {}，已取消这次更改并继续使用原目录 {}。原因：{error}",
                    config["pending"].as_str().unwrap_or_default(),
                    layout.root.display()
                ));
                *config = restored;
                Ok(layout)
            }
        }
    }

    fn select_layout_inner(
        config: &mut Value,
        suggested: &Path,
        appdata: &Path,
        local: &Path,
    ) -> Result<StorageLayout, String> {
        let current = config["root"].as_str().map(PathBuf::from);
        // Old queued migration requests are cancelled, NOT silently reinterpreted
        // as permission to start with an empty configuration.
        if config["pending_kind"] == "switch-only-v2" {
            if let Some(target) = config["pending"].as_str().map(PathBuf::from) {
                let saved_layout = config["locations"][target.to_string_lossy().as_ref()].clone();
                let layout = if saved_layout.is_object() {
                    if !target.is_dir() {
                        return Err("所选旧数据目录暂不可用，未切换位置".into());
                    }
                    layout_from_config(&saved_layout, validate_path(&target)?)?
                } else {
                    prepare_root(&target)?;
                    StorageLayout {
                        root: target,
                        legacy: None,
                    }
                };
                if let Some(current_root) = &current {
                    let snapshot = json!({"mode": config["mode"], "paths": config["paths"], "required_paths": config["required_paths"]});
                    if !config["locations"].is_object() {
                        config["locations"] = json!({});
                    }
                    config["locations"][current_root.to_string_lossy().as_ref()] = snapshot;
                }
                config["previous"] = current.map(|path| json!(path)).unwrap_or(Value::Null);
                save_layout(config, &layout);
                return Ok(layout);
            }
        }
        config["pending"] = Value::Null;
        config["pending_kind"] = Value::Null;
        let layout = if let Some(path) = current {
            validate_path(&path)?;
            if !path.is_dir() {
                return Err(format!("已选数据目录暂不可用：{}。请连接该磁盘后重试；现有数据未搬移，也不会建立空配置。", path.display()));
            }
            layout_from_config(config, path)?
        } else if known_root(suggested) {
            // Already-used unified data is authoritative even if its locator
            // disappeared. Runtime writes do NOT invalidate a completed migration.
            validate_path(suggested)?;
            StorageLayout {
                root: suggested.to_path_buf(),
                legacy: None,
            }
        } else if let Some(layout) = legacy_layout(appdata, local) {
            validate_path(&layout.root)?;
            fs::create_dir_all(&layout.root).map_err(|e| e.to_string())?;
            layout
        } else {
            prepare_root(suggested)?;
            StorageLayout {
                root: suggested.to_path_buf(),
                legacy: None,
            }
        };
        save_layout(config, &layout);
        Ok(layout)
    }

    pub(super) fn initialize() -> Result<Option<StorageLayout>, String> {
        let identity = std::env::var_os("USERPROFILE").ok_or("USERPROFILE 不存在")?;
        let Some(_startup_lock) = StartupLock::acquire(&identity)? else {
            return Ok(None);
        };
        let mut config = settings()?;
        let before = config.clone();
        let appdata = PathBuf::from(std::env::var_os("APPDATA").ok_or("APPDATA 不存在")?);
        let local = PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA 不存在")?);
        let suggested = suggest().unwrap_or_else(|| appdata.join("MaxGameStudioData"));
        let layout = select_layout(&mut config, &suggested, &appdata, &local)?;
        if config != before {
            save(&config)?;
        }
        Ok(Some(layout))
    }
    pub(super) fn status() -> Result<Value, String> {
        let _lock = SETTINGS_LOCK.lock().map_err(|_| "存储设置忙")?;
        let config = settings()?;
        let root = root()?;
        let protected_root = root
            .ancestors()
            .last()
            .ok_or("无存储卷")?
            .join("MaxGameStudioAdminRuntime");
        Ok(
            json!({"root": root, "mode": config["mode"], "last_switch_error": config["last_switch_error"], "paths": {
                "data": ROOT.get().unwrap().path("data"),
                "logs": ROOT.get().unwrap().path("data/logs"),
                "cache": ROOT.get().unwrap().path("data/cache"),
                "webview": ROOT.get().unwrap().path("webview"),
                "league_runtime": ROOT.get().unwrap().path("league-runtime"),
                "temp": ROOT.get().unwrap().path("temp")
            }, "protected_root": protected_root, "pending": config["pending"], "previous": config["previous"], "system_drive": system_drive(&root), "restart_required": config["pending"].is_string()}),
        )
    }
    pub(super) fn schedule() -> Result<Value, String> {
        let target = choose()?;
        if !distinct_trees(&root()?, &target) {
            return Err("新旧目录不能相同或互相包含".into());
        }
        let _lock = SETTINGS_LOCK.lock().map_err(|_| "存储设置忙")?;
        let mut config = settings()?;
        if target.exists()
            && !known_root(&target)
            && !config["locations"][target.to_string_lossy().as_ref()].is_object()
            && fs::read_dir(&target)
                .map_err(|e| e.to_string())?
                .next()
                .is_some()
        {
            return Err("请选择空文件夹或已有的 MaxGameStudio 数据目录；不会覆盖其他文件".into());
        }
        if rfd::MessageDialog::new().set_title("确认更改数据位置")
            .set_description("重启后将使用所选目录。不复制、搬移或删除原目录。空目录会使用全新设置；需要旧数据时可以重新选择原目录。是否继续？")
            .set_buttons(rfd::MessageButtons::YesNo).show() != rfd::MessageDialogResult::Yes {
            return Err("已取消更改，原数据位置保持不变".into());
        }
        config["pending"] = json!(target);
        config["pending_kind"] = json!("switch-only-v2");
        config["last_switch_error"] = Value::Null;
        save(&config)?;
        drop(_lock);
        status()
    }
    pub(super) fn cancel() -> Result<Value, String> {
        let _lock = SETTINGS_LOCK.lock().map_err(|_| "存储设置忙")?;
        let mut config = settings()?;
        config["pending"] = Value::Null;
        config["pending_kind"] = Value::Null;
        config["last_switch_error"] = Value::Null;
        save(&config)?;
        drop(_lock);
        status()
    }
}

pub(crate) fn initialize() -> Result<Option<PathBuf>, String> {
    #[cfg(windows)]
    let Some(layout) = windows::initialize()?
    else {
        return Ok(None);
    };
    #[cfg(not(windows))]
    let layout = StorageLayout {
        root: std::env::var_os("MAXGAMESTUDIO_DATA_ROOT")
            .map(PathBuf::from)
            .ok_or("设置 MAXGAMESTUDIO_DATA_ROOT 后启动")?,
        legacy: None,
    };
    let path = layout.root.clone();
    ROOT.set(layout).map_err(|_| "存储已经初始化")?;
    let temp = directory("temp")?;
    // Called before the Tauri runtime/worker threads are created. Process-local only.
    std::env::set_var("TEMP", &temp);
    std::env::set_var("TMP", &temp);
    std::env::set_var("TMPDIR", &temp);
    Ok(Some(path))
}

#[tauri::command]
pub(crate) async fn get_desktop_storage() -> Result<Value, String> {
    #[cfg(windows)]
    {
        tauri::async_runtime::spawn_blocking(windows::status)
            .await
            .map_err(|e| e.to_string())?
    }
    #[cfg(not(windows))]
    {
        Ok(json!({"root": root()?}))
    }
}
#[tauri::command]
pub(crate) async fn choose_desktop_storage() -> Result<Value, String> {
    #[cfg(windows)]
    {
        tauri::async_runtime::spawn_blocking(windows::schedule)
            .await
            .map_err(|e| e.to_string())?
    }
    #[cfg(not(windows))]
    {
        Err("仅支持 Windows".into())
    }
}
#[tauri::command]
pub(crate) async fn cancel_desktop_storage_change() -> Result<Value, String> {
    #[cfg(windows)]
    {
        tauri::async_runtime::spawn_blocking(windows::cancel)
            .await
            .map_err(|e| e.to_string())?
    }
    #[cfg(not(windows))]
    {
        Err("仅支持 Windows".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    fn fixture() -> PathBuf {
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).unwrap();
        let nonce: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let path = std::env::temp_dir().join(format!("mgs-storage-startup-{nonce}"));
        fs::create_dir(&path).unwrap();
        path
    }

    #[cfg(windows)]
    fn wait_for_file(path: &Path) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !path.exists() {
            assert!(std::time::Instant::now() < deadline, "{}", path.display());
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    // Runs only inside the test executable, never reads the real registry or
    // legacy APPDATA. Parent and child use an isolated fixture as their identity.
    #[cfg(windows)]
    #[test]
    fn startup_lock_child() {
        let Some(root) = std::env::var_os("MGS_TEST_STORAGE_LOCK_FIXTURE") else {
            return;
        };
        let root = PathBuf::from(root);
        fs::write(root.join("child-ready"), b"ready").unwrap();
        let Some(_guard) = windows::StartupLock::acquire(root.as_os_str()).unwrap() else {
            return;
        };
        fs::write(root.join("child-entered"), b"entered").unwrap();
        let locator = fs::read_to_string(root.join("locator.json")).unwrap();
        assert_eq!(locator, "committed destination");
        assert_eq!(
            fs::read_to_string(root.join("live.log")).unwrap(),
            "new runtime data"
        );
    }

    #[cfg(windows)]
    #[test]
    fn duplicate_startup_exits_before_read_and_later_launch_reads_committed_locator() {
        use std::os::windows::process::CommandExt;
        let root = fixture();
        fs::write(root.join("locator.json"), b"not migrated").unwrap();
        let guard = windows::StartupLock::acquire(root.as_os_str())
            .unwrap()
            .unwrap();
        let spawn = || {
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "desktop_storage::tests::startup_lock_child",
                    "--nocapture",
                ])
                .env("MGS_TEST_STORAGE_LOCK_FIXTURE", &root)
                .creation_flags(0x08000000)
                .spawn()
                .unwrap()
        };
        let mut child = spawn();
        wait_for_file(&root.join("child-ready"));
        // Keep the initializer lock held throughout: a waiting implementation
        // still fails, but slow test-runner teardown on a busy disk is allowed.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let exited_while_initializing = loop {
            let status = child.try_wait().unwrap();
            if status.is_some() || std::time::Instant::now() >= deadline {
                break status;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        let entered_before_commit = root.join("child-entered").exists();
        fs::write(root.join("locator.json"), b"committed destination").unwrap();
        fs::write(root.join("live.log"), b"new runtime data").unwrap();
        drop(guard);
        let status = child.wait().unwrap();
        assert!(
            exited_while_initializing.is_some(),
            "duplicate blocked migration's active-writer gate"
        );
        assert!(
            !entered_before_commit,
            "second startup read a stale locator before migration committed"
        );
        assert!(status.success());
        assert!(spawn().wait().unwrap().success());
        assert!(root.join("child-entered").exists());
        assert_eq!(
            fs::read_to_string(root.join("live.log")).unwrap(),
            "new runtime data"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn startup_lock_is_taken_before_the_locator_snapshot() {
        let source = include_str!("desktop_storage.rs");
        let (_, initialize) = source.split_once("pub(super) fn initialize()").unwrap();
        assert!(
            initialize.find("StartupLock::acquire").unwrap()
                < initialize.find("settings()?").unwrap()
        );
    }

    #[cfg(windows)]
    #[test]
    fn interrupted_initializer_releases_abandoned_mutex() {
        let root = fixture();
        let identity = root.clone();
        let abandoned =
            std::thread::spawn(move || windows::abandon_lock_for_test(identity.as_os_str()))
                .join()
                .unwrap();
        let guard = windows::StartupLock::acquire(root.as_os_str())
            .unwrap()
            .unwrap();
        unsafe { windows_sys::Win32::Foundation::CloseHandle(abandoned as _) };
        drop(guard);
        fs::remove_dir(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn failed_initializer_releases_mutex_for_retry() {
        let root = fixture();
        let identity = root.clone();
        let result: Result<(), String> = std::thread::spawn(move || {
            let _guard = windows::StartupLock::acquire(identity.as_os_str())?.unwrap();
            Err("injected initialization error".into())
        })
        .join()
        .unwrap();
        assert!(result.is_err());
        let guard = windows::StartupLock::acquire(root.as_os_str())
            .unwrap()
            .unwrap();
        drop(guard);
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn migration_roots_must_not_overlap() {
        assert!(!distinct_trees(
            Path::new("/data"),
            Path::new("/data/child")
        ));
        assert!(!distinct_trees(Path::new("/data"), Path::new("/data")));
        assert!(!distinct_trees(
            Path::new("/data/child"),
            Path::new("/data")
        ));
        assert!(distinct_trees(Path::new("/old"), Path::new("/new")));
    }

    #[cfg(windows)]
    #[test]
    fn storage_paths_reject_roots_network_and_command_metacharacters() {
        for path in [
            r"C:\",
            r"relative\data",
            r"\\server\share\data",
            r"D:\data\..\escape",
            r"D:\data&command",
            r"D:\%TEMP%\data",
        ] {
            assert!(windows::validate_path(Path::new(path)).is_err(), "{path}");
        }
        assert!(windows::validate_path(Path::new(r"E:\MaxGameStudio Data\用户")).is_ok());
    }

    #[test]
    fn every_window_and_backend_share_the_selected_storage() {
        let source = include_str!("lib.rs").replace("\r\n", "\n");
        // Windows CI may check sources out with CRLF; test both representations.
        for shell in [source.clone(), source.replace('\n', "\r\n")] {
            assert!(shell.contains("window.create = false;"));
            let (_, builder) = shell
                .split_once("WebviewWindowBuilder::from_config(app, config)?")
                .expect("startup windows must use the explicit native builder");
            assert!(builder
                .trim_start()
                .starts_with(".data_directory(desktop_storage::directory(\"webview\")?)"));
            assert!(shell.contains(".with_filename(storage_root.join(\"window-state.json\")"));
            assert!(shell.contains(".env(\"CS2_INSIGHT_DATA_DIR\", &data_root)"));
            assert!(!shell.contains(".arg(\"--appdata\")"));
        }
        let runtime = include_str!("league_runtime.rs");
        assert!(runtime.contains("desktop_storage::directory(\"league-runtime\")"));
        assert!(runtime.contains("desktop_storage::directory(\"temp/admin-launchers\")"));
    }

    #[cfg(windows)]
    fn layout_fixture() -> (PathBuf, PathBuf, PathBuf, PathBuf) {
        let root = fixture();
        let appdata = root.join("appdata");
        let local = root.join("local");
        let suggested = root.join("suggested");
        fs::create_dir_all(&appdata).unwrap();
        fs::create_dir_all(&local).unwrap();
        (root, appdata, local, suggested)
    }

    #[cfg(windows)]
    #[test]
    fn existing_root_without_marker_is_used_in_place() {
        let (root, appdata, local, suggested) = layout_fixture();
        let current = root.join("existing");
        fs::create_dir_all(current.join("data")).unwrap();
        let mut config = json!({"root": current});
        let selected = windows::select_layout(&mut config, &suggested, &appdata, &local).unwrap();
        assert_eq!(selected.root, current);
        assert!(selected.legacy.is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn existing_migration_marker_remains_authoritative() {
        let (root, appdata, local, suggested) = layout_fixture();
        let current = root.join("migrated");
        fs::create_dir_all(&current).unwrap();
        fs::write(current.join(".storage-migration-v1.json"), b"{}\n").unwrap();
        let mut config = json!({"root": current});
        let selected = windows::select_layout(&mut config, &suggested, &appdata, &local).unwrap();
        assert_eq!(selected.root, current);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn missing_locator_uses_existing_suggested_unified_root() {
        let (root, appdata, local, suggested) = layout_fixture();
        fs::create_dir_all(&suggested).unwrap();
        fs::write(
            suggested.join(".storage-location-v2.json"),
            b"{\"version\":2}\n",
        )
        .unwrap();
        let mut config = json!({});
        let selected = windows::select_layout(&mut config, &suggested, &appdata, &local).unwrap();
        assert_eq!(selected.root, suggested);
        assert!(selected.legacy.is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn legacy_layout_is_in_place_and_preserves_all_component_paths() {
        let (root, appdata, local, suggested) = layout_fixture();
        let data = appdata.join("CS2 Insight Agent/data");
        let webview = local.join("com.cs2insightagent.app");
        let league = appdata.join("MaxGameStudio/league-runtime");
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(&webview).unwrap();
        fs::create_dir_all(&league).unwrap();
        fs::write(data.join("cs2-insight.db"), b"db").unwrap();
        fs::write(webview.join("profile"), b"webview").unwrap();
        fs::write(league.join("runtime"), b"league").unwrap();
        let mut config = json!({});
        let selected = windows::select_layout(&mut config, &suggested, &appdata, &local).unwrap();
        let (selected_data, selected_webview, selected_league) = selected.legacy.unwrap();
        assert_eq!(selected_data, data);
        assert_eq!(selected_webview, webview);
        assert_eq!(selected_league, league);
        assert_eq!(fs::read(data.join("cs2-insight.db")).unwrap(), b"db");
        assert_eq!(fs::read(webview.join("profile")).unwrap(), b"webview");
        assert_eq!(fs::read(league.join("runtime")).unwrap(), b"league");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn log_only_canonical_layout_does_not_hide_legacy_database() {
        let (root, appdata, local, suggested) = layout_fixture();
        fs::create_dir_all(appdata.join("CS2 Insight Agent/data/logs")).unwrap();
        fs::write(
            appdata.join("CS2 Insight Agent/data/logs/start.log"),
            b"log",
        )
        .unwrap();
        let legacy = appdata.join("cs2-insight-agent/data");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("cs2-insight.db"), b"db").unwrap();
        let mut config = json!({});
        let selected = windows::select_layout(&mut config, &suggested, &appdata, &local).unwrap();
        assert_eq!(selected.legacy.unwrap().0, legacy);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn new_install_creates_empty_unified_root_marker() {
        let (root, appdata, local, suggested) = layout_fixture();
        let mut config = json!({});
        let selected = windows::select_layout(&mut config, &suggested, &appdata, &local).unwrap();
        assert_eq!(selected.root, suggested);
        assert!(suggested.join(".storage-location-v2.json").is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn stale_pending_without_switch_kind_is_cancelled() {
        let (root, appdata, local, suggested) = layout_fixture();
        let current = root.join("current");
        fs::create_dir_all(&current).unwrap();
        let mut config = json!({"root": current, "pending": root.join("stale")});
        let selected = windows::select_layout(&mut config, &suggested, &appdata, &local).unwrap();
        assert_eq!(selected.root, current);
        assert!(config["pending"].is_null());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn switch_only_uses_empty_target_without_copying_old_data() {
        let (root, appdata, local, suggested) = layout_fixture();
        let current = root.join("current");
        let target = root.join("target");
        fs::create_dir_all(&current).unwrap();
        fs::write(current.join("old.db"), b"old").unwrap();
        fs::create_dir_all(&target).unwrap();
        let mut config =
            json!({"root": current, "pending": target, "pending_kind": "switch-only-v2"});
        let selected = windows::select_layout(&mut config, &suggested, &appdata, &local).unwrap();
        assert_eq!(selected.root, target);
        assert!(!target.join("old.db").exists());
        assert!(current.join("old.db").is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn failed_pending_switch_keeps_original_data_and_clears_retry_loop() {
        let (root, appdata, local, suggested) = layout_fixture();
        let current = root.join("current");
        let target = root.join("target");
        fs::create_dir_all(current.join("data")).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::write(current.join("data/cs2-insight.db"), b"original").unwrap();
        fs::write(target.join("unrelated.txt"), b"do not touch").unwrap();
        let mut config =
            json!({"root": current, "pending": target, "pending_kind": "switch-only-v2"});
        let selected = windows::select_layout(&mut config, &suggested, &appdata, &local).unwrap();
        assert_eq!(selected.root, current);
        assert!(config["pending"].is_null());
        assert!(config["pending_kind"].is_null());
        assert!(config["last_switch_error"]
            .as_str()
            .unwrap()
            .contains("继续使用原目录"));
        let again = windows::select_layout(&mut config, &suggested, &appdata, &local).unwrap();
        assert_eq!(again.root, current);
        assert_eq!(
            fs::read(current.join("data/cs2-insight.db")).unwrap(),
            b"original"
        );
        assert_eq!(
            fs::read(target.join("unrelated.txt")).unwrap(),
            b"do not touch"
        );
        assert!(!target.join(".storage-location-v2.json").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn failed_pending_switch_restores_legacy_layout_but_not_missing_original_data() {
        let (root, appdata, local, suggested) = layout_fixture();
        let data = appdata.join("CS2 Insight Agent/data");
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join("cs2-insight.db"), b"original").unwrap();
        let mut config = json!({});
        windows::select_layout(&mut config, &suggested, &appdata, &local).unwrap();
        let target = root.join("offline-saved-location");
        config["locations"] = json!({target.to_string_lossy().as_ref(): {"mode": "unified"}});
        config["pending"] = json!(target);
        config["pending_kind"] = json!("switch-only-v2");
        let selected = windows::select_layout(&mut config, &suggested, &appdata, &local).unwrap();
        assert_eq!(selected.path("data"), data);
        assert!(!target.exists());
        config["pending"] = json!(target);
        config["pending_kind"] = json!("switch-only-v2");
        fs::rename(&data, root.join("preserved-data")).unwrap();
        let before = config.clone();
        assert!(windows::select_layout(&mut config, &suggested, &appdata, &local).is_err());
        assert_eq!(config, before);
        assert!(!data.exists());
        assert!(!suggested.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn selecting_saved_legacy_location_restores_legacy_paths() {
        let (root, appdata, local, suggested) = layout_fixture();
        let data = appdata.join("CS2 Insight Agent/data");
        let webview = local.join("com.cs2insightagent.app");
        let league = appdata.join("MaxGameStudio/league-runtime");
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(&webview).unwrap();
        fs::create_dir_all(&league).unwrap();
        fs::write(data.join("cs2-insight.db"), b"db").unwrap();
        let legacy_root = data.parent().unwrap().to_path_buf();
        let mut config = json!({"root": legacy_root, "mode": "legacy_in_place",
            "paths": {"data": data, "webview": webview, "league_runtime": league}});
        let target = root.join("new");
        fs::create_dir_all(&target).unwrap();
        config["pending"] = json!(target);
        config["pending_kind"] = json!("switch-only-v2");
        let _ = windows::select_layout(&mut config, &suggested, &appdata, &local).unwrap();
        config["pending"] = json!(legacy_root);
        config["pending_kind"] = json!("switch-only-v2");
        let restored = windows::select_layout(&mut config, &suggested, &appdata, &local).unwrap();
        assert!(restored.legacy.is_some());
        assert_eq!(restored.legacy.unwrap().0, data);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn missing_known_component_never_creates_an_empty_replacement() {
        let (root, appdata, local, suggested) = layout_fixture();
        let data = appdata.join("CS2 Insight Agent/data");
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join("cs2-insight.db"), b"original").unwrap();
        let mut config = json!({});
        windows::select_layout(&mut config, &suggested, &appdata, &local).unwrap();
        fs::rename(&data, root.join("disconnected-data")).unwrap();
        assert!(windows::select_layout(&mut config, &suggested, &appdata, &local).is_err());
        assert!(!data.exists());
        assert_eq!(
            fs::read(root.join("disconnected-data/cs2-insight.db")).unwrap(),
            b"original"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn missing_current_disk_fails_closed() {
        let (root, appdata, local, suggested) = layout_fixture();
        let missing = root.join("missing");
        let mut config = json!({"root": missing});
        let error = match windows::select_layout(&mut config, &suggested, &appdata, &local) {
            Ok(_) => panic!("missing current disk unexpectedly selected"),
            Err(error) => error,
        };
        assert!(error.contains("暂不可用"));
        assert!(!suggested.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
