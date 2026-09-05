//! One storage root for application-owned files. Registry holds only a locator;
//! migration copies and verifies before changing it, and never deletes sources.
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

static ROOT: OnceLock<PathBuf> = OnceLock::new();
pub(crate) fn root() -> Result<PathBuf, String> {
    let path = ROOT
        .get()
        .cloned()
        .ok_or_else(|| "应用存储尚未初始化".to_string())?;
    if !path.join(".storage-migration-v1.json").is_file() {
        return Err("已选存储目录不再可用，请检查磁盘；不会自动回退系统盘".into());
    }
    Ok(path)
}
pub(crate) fn directory(name: &str) -> Result<PathBuf, String> {
    let path = root()?.join(name);
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
    use std::{
        ffi::OsStr,
        os::windows::{ffi::OsStrExt, process::CommandExt},
        process::{Command, Stdio},
        sync::Mutex,
    };
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
    // native choice, migration and locator commit in one cross-process section.
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
            // keep another desktop process alive and block Python's writer gate.
            let result = unsafe { WaitForSingleObject(handle, 0) };
            if result == WAIT_OBJECT_0 || result == WAIT_ABANDONED {
                // An interrupted owner may have left a partial transaction.
                // initialize still re-reads and validates the persisted state;
                // Python's nonempty-target/hash checks are never bypassed.
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
        let target = validate_path(&parent.join("MaxGameStudioData"))?;
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
    fn migration(destination: &Path, source: Option<&Path>) -> Result<(), String> {
        let code_root = if cfg!(debug_assertions) {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
        } else {
            std::env::current_exe()
                .map_err(|e| e.to_string())?
                .parent()
                .ok_or("无安装目录")?
                .to_path_buf()
        };
        let python = if cfg!(debug_assertions) {
            code_root.join(".venv/Scripts/python.exe")
        } else {
            code_root.join("python/python.exe")
        };
        let script = code_root.join("backend/app/desktop_storage_migration.py");
        // Staging temp lives on the chosen volume, never inherited system TEMP.
        let parent = destination.parent().ok_or("存储目录没有父目录")?;
        fs::create_dir_all(parent).map_err(|e| format!("无法创建存储父目录：{e}"))?;
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|e| e.to_string())?;
        let nonce = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let temp = parent.join(format!(".mgs-migration-temp-{nonce}"));
        fs::create_dir(&temp).map_err(|e| format!("无法创建迁移临时目录：{e}"))?;
        let mut command = Command::new(python);
        command
            .arg("-I")
            .arg(script)
            .arg("--destination")
            .arg(destination)
            .arg("--host-pid")
            .arg(std::process::id().to_string())
            .env("TEMP", &temp)
            .env("TMP", &temp)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .creation_flags(crate::CREATE_NO_WINDOW);
        if let Some(source) = source {
            command.arg("--source").arg(source);
        } else {
            command
                .arg("--legacy-appdata")
                .arg(std::env::var_os("APPDATA").ok_or("APPDATA 不存在")?)
                .arg("--legacy-localappdata")
                .arg(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA 不存在")?);
        }
        let output = command.output();
        let _ = fs::remove_dir(&temp); // only remove our empty temporary directory
        let output = output.map_err(|e| format!("无法启动数据复制校验：{e}"))?;
        if !output.status.success() {
            return Err(format!(
                "数据复制未完成，原数据保持不变：{}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok(())
    }
    pub(super) fn initialize() -> Result<Option<PathBuf>, String> {
        let identity = std::env::var_os("USERPROFILE").ok_or("USERPROFILE 不存在")?;
        let Some(_startup_lock) = StartupLock::acquire(&identity)? else {
            return Ok(None);
        };
        let mut config = settings()?;
        let current = config["root"].as_str().map(PathBuf::from);
        if let Some(path) = current.as_ref() {
            validate_path(path)?;
            if !path.join(".storage-migration-v1.json").is_file() {
                return Err(format!("已选存储目录不可用：{}。请重新连接磁盘或恢复原目录；不会自动改用 C 盘或建立空配置。", path.display()));
            }
        }
        let pending = config["pending"].as_str().map(PathBuf::from);
        let chosen = match pending.or(current.clone()) {
            Some(path) => validate_path(&path)?,
            None => {
                let proposed = suggest();
                match proposed {
                    Some(path) if rfd::MessageDialog::new().set_title("MaxGameStudio 数据位置")
                        .set_description(format!("建议存储位置：{}\n\n将复制并校验旧数据，保留原目录作为回滚副本。缓存、日志、WebView 和英雄联盟数据将写入这里。数据较多时可能需要几分钟，完成后才会打开主窗口，请勿重复启动。选择“否”可自行指定位置。", path.display()))
                        .set_buttons(rfd::MessageButtons::YesNo).show() == rfd::MessageDialogResult::Yes => validate_path(&path)?,
                    _ => choose()?,
                }
            }
        };
        if current.as_ref() != Some(&chosen) {
            if let Some(source) = &current {
                if !distinct_trees(source, &chosen) {
                    return Err("新旧目录不能相同或互相包含".into());
                }
            }
            if let Err(error) = migration(&chosen, current.as_deref()) {
                if let Some(source) = current.as_ref() {
                    if rfd::MessageDialog::new()
                        .set_title("数据迁移未完成")
                        .set_description(format!(
                            "{error}\n\n原数据未修改。是否取消此次切换，并继续使用原目录？"
                        ))
                        .set_buttons(rfd::MessageButtons::YesNo)
                        .show()
                        == rfd::MessageDialogResult::Yes
                    {
                        config["pending"] = Value::Null;
                        save(&config)?;
                        return Ok(Some(source.clone()));
                    }
                }
                return Err(error);
            }
            config["previous"] = current.map(|p| json!(p)).unwrap_or(Value::Null);
            config["root"] = json!(chosen);
            config["pending"] = Value::Null;
            save(&config)?;
        }
        Ok(Some(chosen))
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
            json!({"root": root, "bytes": storage_bytes(&root).ok(), "protected_root": protected_root, "pending": config["pending"], "previous": config["previous"], "system_drive": system_drive(&root), "restart_required": config["pending"].is_string()}),
        )
    }
    pub(super) fn schedule() -> Result<Value, String> {
        let target = choose()?;
        if !distinct_trees(&root()?, &target) {
            return Err("新旧目录不能相同或互相包含".into());
        }
        if target.exists()
            && fs::read_dir(&target)
                .map_err(|e| e.to_string())?
                .next()
                .is_some()
        {
            return Err("目标数据文件夹不是空目录；请选择新的位置，避免覆盖已有数据".into());
        }
        let _lock = SETTINGS_LOCK.lock().map_err(|_| "存储设置忙")?;
        let mut config = settings()?;
        config["pending"] = json!(target);
        save(&config)?;
        drop(_lock);
        status()
    }
    pub(super) fn cancel() -> Result<Value, String> {
        let _lock = SETTINGS_LOCK.lock().map_err(|_| "存储设置忙")?;
        let mut config = settings()?;
        config["pending"] = Value::Null;
        save(&config)?;
        drop(_lock);
        status()
    }
}

fn storage_bytes(root: &Path) -> Result<u64, String> {
    let mut pending = vec![root.to_path_buf()];
    let mut bytes = 0u64;
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(&path).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let metadata = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 {
                    return Err("存储目录包含链接，无法完整统计".into());
                }
            }
            if metadata.is_symlink() {
                return Err("存储目录包含链接，无法完整统计".into());
            }
            if metadata.is_dir() {
                pending.push(entry.path());
            } else {
                bytes = bytes.saturating_add(metadata.len());
            }
        }
    }
    Ok(bytes)
}

pub(crate) fn initialize() -> Result<Option<PathBuf>, String> {
    #[cfg(windows)]
    let Some(path) = windows::initialize()?
    else {
        return Ok(None);
    };
    #[cfg(not(windows))]
    let path = std::env::var_os("MAXGAMESTUDIO_DATA_ROOT")
        .map(PathBuf::from)
        .ok_or("设置 MAXGAMESTUDIO_DATA_ROOT 后启动")?;
    ROOT.set(path.clone()).map_err(|_| "存储已经初始化")?;
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
        std::thread::sleep(std::time::Duration::from_millis(500));
        let exited_while_initializing = child.try_wait().unwrap();
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
}
