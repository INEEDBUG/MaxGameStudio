use serde_json::Value;

#[cfg(windows)]
#[path = "steam_accounts_vdf.rs"]
mod vdf;

#[tauri::command]
pub(crate) async fn get_steam_accounts() -> Result<Value, String> {
    #[cfg(windows)]
    return tauri::async_runtime::spawn_blocking(windows::get_accounts)
        .await
        .map_err(|_| "steam_read_failed".to_owned())?
        .map_err(str::to_owned);

    #[cfg(not(windows))]
    Ok(serde_json::json!({"installed": false, "running": false, "accounts": []}))
}

#[tauri::command]
pub(crate) async fn switch_steam_account(steam_id: String) -> Result<Value, String> {
    #[cfg(windows)]
    return tauri::async_runtime::spawn_blocking(move || windows::switch_account(&steam_id))
        .await
        .map_err(|_| "steam_switch_failed".to_owned())?
        .map_err(str::to_owned);

    #[cfg(not(windows))]
    {
        let _ = steam_id;
        Err("steam_not_installed".into())
    }
}

#[cfg(windows)]
mod windows {
    use super::vdf::{account_id, LoginUsers};
    use serde_json::{json, Value};
    use std::{
        env,
        ffi::{OsStr, OsString},
        fs::{self, File, OpenOptions},
        io::{self, Read, Write},
        os::windows::{
            ffi::{OsStrExt, OsStringExt},
            process::CommandExt,
        },
        path::{Path, PathBuf},
        process::{self, Command},
        ptr::null_mut,
        sync::{
            atomic::{AtomicU64, Ordering},
            Mutex, TryLockError,
        },
        thread,
        time::Duration,
    };
    use windows_sys::Win32::{
        Foundation::{
            CloseHandle, GetLastError, ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES,
            ERROR_PATH_NOT_FOUND, HANDLE, INVALID_HANDLE_VALUE,
        },
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
                TH32CS_SNAPPROCESS,
            },
            Registry::{
                RegCloseKey, RegDeleteValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW, HKEY,
                HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_SET_VALUE, REG_SZ, RRF_RT_REG_DWORD,
                RRF_RT_REG_SZ, RRF_SUBKEY_WOW6432KEY,
            },
            Threading::{
                OpenProcess, QueryFullProcessImageNameW, CREATE_NO_WINDOW, PROCESS_NAME_WIN32,
                PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
    };

    const STEAM_KEY: &str = r"Software\Valve\Steam";
    const ACTIVE_KEY: &str = r"Software\Valve\Steam\ActiveProcess";
    const LOGIN_FILE_LIMIT: usize = 1024 * 1024;
    const POLL_INTERVAL: Duration = Duration::from_millis(250);
    static SWITCH: Mutex<()> = Mutex::new(());
    static TEMP_FILE: AtomicU64 = AtomicU64::new(0);

    type SteamResult<T> = Result<T, &'static str>;

    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }

    fn missing_reg(status: u32) -> bool {
        matches!(status, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND)
    }

    fn reg_string(hive: HKEY, key: &str, name: &str, flags: u32) -> SteamResult<Option<OsString>> {
        let key = wide(OsStr::new(key));
        let name = wide(OsStr::new(name));
        let flags = flags | RRF_RT_REG_SZ;
        let mut bytes = 0;
        let status = unsafe {
            RegGetValueW(
                hive,
                key.as_ptr(),
                name.as_ptr(),
                flags,
                null_mut(),
                null_mut(),
                &mut bytes,
            )
        };
        if missing_reg(status) {
            return Ok(None);
        }
        if status != 0 || bytes > 65536 || bytes % 2 != 0 {
            return Err("steam_registry_read_failed");
        }
        let mut units = vec![0u16; bytes as usize / 2];
        let status = unsafe {
            RegGetValueW(
                hive,
                key.as_ptr(),
                name.as_ptr(),
                flags,
                null_mut(),
                units.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if status != 0 || bytes % 2 != 0 {
            return Err("steam_registry_read_failed");
        }
        units.truncate(bytes as usize / 2);
        while units.last() == Some(&0) {
            units.pop();
        }
        if units.contains(&0) {
            return Err("steam_registry_read_failed");
        }
        Ok(Some(OsString::from_wide(&units)))
    }

    fn reg_dword(key: &str, name: &str) -> Option<u32> {
        let key = wide(OsStr::new(key));
        let name = wide(OsStr::new(name));
        let mut value = 0u32;
        let mut bytes = 4;
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_DWORD,
                null_mut(),
                (&mut value as *mut u32).cast(),
                &mut bytes,
            )
        };
        (status == 0 && bytes == 4).then_some(value)
    }

    fn write_autologin(value: Option<&OsStr>) -> SteamResult<()> {
        let key = wide(OsStr::new(STEAM_KEY));
        let name = wide(OsStr::new("AutoLoginUser"));
        let mut handle = null_mut();
        // Steam owns this key. Do not create a new Steam profile from our switcher.
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                0,
                KEY_SET_VALUE,
                &mut handle,
            )
        };
        if status != 0 {
            return Err("steam_registry_write_failed");
        }
        let status = match value {
            Some(value) => {
                let value = wide(value);
                unsafe {
                    RegSetValueExW(
                        handle,
                        name.as_ptr(),
                        0,
                        REG_SZ,
                        value.as_ptr().cast(),
                        (value.len() * 2) as u32,
                    )
                }
            }
            None => unsafe { RegDeleteValueW(handle, name.as_ptr()) },
        };
        unsafe { RegCloseKey(handle) };
        if status != 0 && !(value.is_none() && missing_reg(status)) {
            return Err("steam_registry_write_failed");
        }
        Ok(())
    }

    fn process_image(pid: u32) -> Option<PathBuf> {
        let handle = Handle(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) });
        if handle.0.is_null() {
            return None;
        }
        let mut image = vec![0u16; 32768];
        let mut length = image.len() as u32;
        let found = unsafe {
            QueryFullProcessImageNameW(
                handle.0,
                PROCESS_NAME_WIN32,
                image.as_mut_ptr(),
                &mut length,
            )
        };
        (found != 0).then(|| PathBuf::from(OsString::from_wide(&image[..length as usize])))
    }

    fn steam_processes() -> SteamResult<Vec<(u32, PathBuf)>> {
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err("steam_process_check_failed");
        }
        let snapshot = Handle(snapshot);
        let mut row = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let active_pid = reg_dword(ACTIVE_KEY, "pid");
        let mut valid = unsafe { Process32FirstW(snapshot.0, &mut row) };
        let mut matches = Vec::new();
        while valid != 0 {
            let name = row
                .szExeFile
                .split(|unit| *unit == 0)
                .next()
                .unwrap_or_default();
            if String::from_utf16_lossy(name).eq_ignore_ascii_case("steam.exe") {
                if let Some(path) =
                    process_image(row.th32ProcessID).and_then(|path| validated_exe(&path))
                {
                    matches.push((row.th32ProcessID, path));
                } else if Some(row.th32ProcessID) == active_pid {
                    return Err("steam_process_check_failed");
                }
            }
            valid = unsafe { Process32NextW(snapshot.0, &mut row) };
        }
        if unsafe { GetLastError() } != ERROR_NO_MORE_FILES {
            return Err("steam_process_check_failed");
        }
        Ok(matches)
    }

    fn validated_exe(path: &Path) -> Option<PathBuf> {
        let path = path.canonicalize().ok()?;
        (path.is_file()
            && path
                .file_name()?
                .to_str()?
                .eq_ignore_ascii_case("steam.exe"))
        .then_some(path)
    }

    fn same_path(first: &Path, second: &Path) -> bool {
        first == second
            || first
                .to_string_lossy()
                .eq_ignore_ascii_case(&second.to_string_lossy())
    }

    fn discover() -> SteamResult<Option<SteamInstall>> {
        // The registered active PID is authoritative only while it still points to steam.exe.
        if let Some(path) = reg_dword(ACTIVE_KEY, "pid")
            .and_then(process_image)
            .and_then(|path| validated_exe(&path))
        {
            return Ok(Some(SteamInstall { exe: path }));
        }
        let direct = reg_string(HKEY_CURRENT_USER, STEAM_KEY, "SteamExe", 0)?.map(PathBuf::from);
        let roots = [
            reg_string(HKEY_CURRENT_USER, STEAM_KEY, "SteamPath", 0)?,
            reg_string(
                HKEY_LOCAL_MACHINE,
                STEAM_KEY,
                "InstallPath",
                RRF_SUBKEY_WOW6432KEY,
            )?,
            reg_string(HKEY_LOCAL_MACHINE, STEAM_KEY, "InstallPath", 0)?,
            env::var_os("ProgramFiles(x86)")
                .map(|path| PathBuf::from(path).join("Steam").into_os_string()),
            env::var_os("ProgramFiles")
                .map(|path| PathBuf::from(path).join("Steam").into_os_string()),
        ];
        let registered = direct.into_iter().chain(
            roots
                .into_iter()
                .flatten()
                .filter(|path| !path.is_empty())
                .map(|path| PathBuf::from(path).join("steam.exe")),
        );
        for path in registered {
            if path.is_absolute() {
                if let Some(exe) = validated_exe(&path) {
                    return Ok(Some(SteamInstall { exe }));
                }
            }
        }
        // A portable copy can be found while running, before Steam adds a registry path.
        let mut running = steam_processes()?;
        if running.len() == 1 {
            let (_, exe) = running.remove(0);
            return Ok(Some(SteamInstall { exe }));
        }
        Ok(None)
    }

    #[derive(Default)]
    struct ClientState {
        running: bool,
        active_account: Option<u32>,
        game_running: bool,
    }

    trait SteamControl {
        fn client(&self) -> SteamResult<ClientState>;
        fn users(&self) -> SteamResult<Option<String>>;
        fn replace_users(&self, expected: &str, replacement: &str) -> SteamResult<()>;
        fn auto_login(&self) -> SteamResult<Option<OsString>>;
        fn set_auto_login(&self, user: Option<&OsStr>) -> SteamResult<()>;
        fn start(&self, shutdown: bool) -> SteamResult<()>;
        fn wait(&self) {
            thread::sleep(POLL_INTERVAL);
        }
    }

    struct SteamInstall {
        exe: PathBuf,
    }

    impl SteamInstall {
        fn root(&self) -> &Path {
            self.exe
                .parent()
                .expect("a validated Steam executable has a parent")
        }

        fn login_file(&self) -> PathBuf {
            self.root().join("config").join("loginusers.vdf")
        }

        fn display_root(&self) -> String {
            let path = self.root().to_string_lossy();
            if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
                return format!(r"\\{unc}");
            }
            path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned()
        }
    }

    impl SteamControl for SteamInstall {
        fn client(&self) -> SteamResult<ClientState> {
            let processes = steam_processes()?;
            let current_pid = reg_dword(ACTIVE_KEY, "pid");
            let mut state = ClientState::default();
            for (pid, path) in processes {
                if same_path(&path, &self.exe) {
                    state.running = true;
                    if Some(pid) == current_pid {
                        state.active_account =
                            reg_dword(ACTIVE_KEY, "ActiveUser").filter(|id| *id != 0);
                        state.game_running =
                            reg_dword(STEAM_KEY, "RunningAppID").is_some_and(|id| id != 0);
                    }
                }
            }
            Ok(state)
        }

        fn users(&self) -> SteamResult<Option<String>> {
            let file = match File::open(self.login_file()) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(_) => return Err("steam_read_failed"),
            };
            let mut bytes = Vec::new();
            file.take((LOGIN_FILE_LIMIT + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|_| "steam_read_failed")?;
            if bytes.len() > LOGIN_FILE_LIMIT {
                return Err("steam_invalid_accounts");
            }
            String::from_utf8(bytes)
                .map(Some)
                .map_err(|_| "steam_invalid_accounts")
        }

        fn replace_users(&self, expected: &str, replacement: &str) -> SteamResult<()> {
            let destination = self.login_file();
            let suffix = TEMP_FILE.fetch_add(1, Ordering::Relaxed);
            let staging = destination.with_file_name(format!(
                ".maxgamestudio-loginusers-{}-{suffix}.tmp",
                process::id()
            ));
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&staging)
                .map_err(|_| "steam_write_failed")?;
            let written = file
                .write_all(replacement.as_bytes())
                .and_then(|()| file.sync_all());
            drop(file);
            let result = written.map_err(|_| "steam_write_failed").and_then(|()| {
                // Steam may have been opened separately while the file was being prepared.
                if self.client()?.running || self.users()?.as_deref() != Some(expected) {
                    return Err("steam_state_changed");
                }
                fs::rename(&staging, &destination).map_err(|_| "steam_write_failed")
            });
            if result.is_err() {
                let _ = fs::remove_file(staging);
            }
            result
        }

        fn auto_login(&self) -> SteamResult<Option<OsString>> {
            reg_string(HKEY_CURRENT_USER, STEAM_KEY, "AutoLoginUser", 0)
        }

        fn set_auto_login(&self, user: Option<&OsStr>) -> SteamResult<()> {
            write_autologin(user)
        }

        fn start(&self, shutdown: bool) -> SteamResult<()> {
            let mut command = Command::new(&self.exe);
            command.current_dir(self.root());
            if shutdown {
                command.arg("-shutdown").creation_flags(CREATE_NO_WINDOW);
            }
            command.spawn().map(drop).map_err(|_| {
                if shutdown {
                    "steam_shutdown_failed"
                } else {
                    "steam_launch_failed"
                }
            })
        }
    }

    pub(super) fn get_accounts() -> SteamResult<Value> {
        let Some(install) = discover()? else {
            return Ok(
                json!({"installed": false, "steam_path": null, "running": false, "game_running": false, "current_steam_id": null, "accounts": []}),
            );
        };
        let source = install.users()?;
        let mut parsed = source
            .as_deref()
            .map(LoginUsers::parse)
            .transpose()?
            .unwrap_or_default();
        let client = install.client()?;
        let configured_user = install.auto_login()?;
        parsed.accounts.sort_by_cached_key(|account| {
            std::cmp::Reverse((
                client
                    .active_account
                    .is_some_and(|id| account_id(&account.steam_id) == Some(id)),
                configured_user.as_ref().is_some_and(|user| {
                    user.to_string_lossy()
                        .eq_ignore_ascii_case(&account.account_name)
                }),
                account.preferred,
                account.timestamp,
            ))
        });
        let current = parsed
            .accounts
            .iter()
            .find(|account| {
                client
                    .active_account
                    .is_some_and(|id| account_id(&account.steam_id) == Some(id))
            })
            .map(|account| account.steam_id.as_str());
        let accounts: Vec<_> = parsed
            .accounts
            .iter()
            .map(|account| {
                json!({
                    "steam_id": account.steam_id,
                    "account_name": account.account_name,
                    "persona_name": account.persona_name,
                    "remembered": account.remembered,
                    "preferred": account.preferred,
                })
            })
            .collect();
        Ok(
            json!({"installed": true, "steam_path": install.display_root(), "running": client.running,
                  "game_running": client.game_running, "current_steam_id": current, "accounts": accounts}),
        )
    }

    pub(super) fn switch_account(steam_id: &str) -> SteamResult<Value> {
        let _lock = SWITCH.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => "steam_switch_in_progress",
            TryLockError::Poisoned(_) => "steam_switch_failed",
        })?;
        let install = discover()?.ok_or("steam_not_installed")?;
        let status = switch_with(&install, steam_id, 80, 24)?;
        Ok(json!({"status": status, "steam_id": steam_id}))
    }

    fn read_selection(
        steam: &impl SteamControl,
        steam_id: &str,
    ) -> SteamResult<(String, String, OsString)> {
        let source = steam.users()?.ok_or("steam_unknown_account")?;
        let parsed = LoginUsers::parse(&source)?;
        let username = OsString::from(&parsed.remembered_account(steam_id)?.account_name);
        let selected = parsed.select(&source, steam_id)?;
        Ok((source, selected, username))
    }

    fn switch_with(
        steam: &impl SteamControl,
        steam_id: &str,
        shutdown_checks: usize,
        sign_in_checks: usize,
    ) -> SteamResult<&'static str> {
        let target = account_id(steam_id).ok_or("steam_unknown_account")?;
        let client = steam.client()?;
        if client.active_account == Some(target) {
            return Ok("already_current");
        }
        if client.game_running {
            return Err("steam_game_running");
        }
        let mut selection = read_selection(steam, steam_id)?;
        if client.running {
            steam.start(true)?;
            let mut running = true;
            for _ in 0..shutdown_checks {
                steam.wait();
                running = steam.client()?.running;
                if !running {
                    break;
                }
            }
            if running {
                return Err("steam_shutdown_timeout");
            }
            // Steam writes loginusers.vdf on exit. Select against that fresh copy.
            selection = read_selection(steam, steam_id)?;
        }
        let (source, selected, username) = selection;
        let prior_username = steam.auto_login()?;
        let changed = selected != source;
        if steam.client()?.running {
            return Err("steam_state_changed");
        }
        if changed {
            steam.replace_users(&source, &selected)?;
        }
        if let Err(error) = steam.set_auto_login(Some(&username)) {
            if changed && steam.replace_users(&selected, &source).is_err() {
                return Err("steam_rollback_failed");
            }
            return Err(error);
        }
        if let Err(error) = steam.start(false) {
            let registry_restored = steam.client().is_ok_and(|state| !state.running)
                && steam
                    .auto_login()
                    .is_ok_and(|current| current.as_deref() == Some(&username))
                && steam.set_auto_login(prior_username.as_deref()).is_ok();
            let file_restored = !changed || steam.replace_users(&selected, &source).is_ok();
            return Err(if registry_restored && file_restored {
                error
            } else {
                "steam_rollback_failed"
            });
        }
        for _ in 0..sign_in_checks {
            steam.wait();
            if steam
                .client()
                .is_ok_and(|state| state.active_account == Some(target))
            {
                return Ok("signed_in");
            }
        }
        Ok("waiting_login")
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::{cell::RefCell, collections::VecDeque};

        const TARGET: &str = "76561197960265731";
        const SOURCE: &str = concat!(
            "users { 76561197960265730 { AccountName old_user MostRecent 1 RememberPassword 1 } ",
            "76561197960265731 { AccountName next_user MostRecent 0 RememberPassword 1 } }",
        );

        struct MockSteam {
            states: RefCell<VecDeque<ClientState>>,
            vdf: RefCell<String>,
            registry: RefCell<Option<OsString>>,
            actions: RefCell<Vec<&'static str>>,
            write_fails: bool,
            launch_fails: bool,
        }

        impl MockSteam {
            fn new(states: impl Into<VecDeque<ClientState>>) -> Self {
                Self {
                    states: RefCell::new(states.into()),
                    vdf: RefCell::new(SOURCE.to_owned()),
                    registry: RefCell::new(Some("old_user".into())),
                    actions: RefCell::default(),
                    write_fails: false,
                    launch_fails: false,
                }
            }
        }

        impl SteamControl for MockSteam {
            fn client(&self) -> SteamResult<ClientState> {
                Ok(self.states.borrow_mut().pop_front().unwrap_or_default())
            }

            fn users(&self) -> SteamResult<Option<String>> {
                Ok(Some(self.vdf.borrow().clone()))
            }

            fn replace_users(&self, expected: &str, replacement: &str) -> SteamResult<()> {
                self.actions.borrow_mut().push("file");
                if self.write_fails || *self.vdf.borrow() != expected {
                    return Err("steam_write_failed");
                }
                *self.vdf.borrow_mut() = replacement.to_owned();
                Ok(())
            }

            fn auto_login(&self) -> SteamResult<Option<OsString>> {
                Ok(self.registry.borrow().clone())
            }

            fn set_auto_login(&self, user: Option<&OsStr>) -> SteamResult<()> {
                self.actions.borrow_mut().push("registry");
                *self.registry.borrow_mut() = user.map(OsStr::to_owned);
                Ok(())
            }

            fn start(&self, shutdown: bool) -> SteamResult<()> {
                self.actions
                    .borrow_mut()
                    .push(if shutdown { "shutdown" } else { "launch" });
                if !shutdown && self.launch_fails {
                    return Err("steam_launch_failed");
                }
                Ok(())
            }

            fn wait(&self) {}
        }

        #[test]
        fn detects_game_and_does_not_request_shutdown() {
            let steam = MockSteam::new([ClientState {
                running: true,
                game_running: true,
                active_account: Some(2),
            }]);
            assert_eq!(switch_with(&steam, TARGET, 2, 2), Err("steam_game_running"));
            assert!(steam.actions.borrow().is_empty());
        }

        #[test]
        fn waits_for_exit_before_writing_and_checks_target_login() {
            let steam = MockSteam::new([
                ClientState {
                    running: true,
                    active_account: Some(2),
                    game_running: false,
                },
                ClientState {
                    running: true,
                    active_account: Some(2),
                    game_running: false,
                },
                ClientState::default(),
                ClientState::default(),
                ClientState {
                    running: true,
                    active_account: Some(3),
                    game_running: false,
                },
            ]);
            assert_eq!(switch_with(&steam, TARGET, 2, 2), Ok("signed_in"));
            assert_eq!(
                *steam.actions.borrow(),
                ["shutdown", "file", "registry", "launch"]
            );
            assert!(steam.vdf.borrow().contains("next_user MostRecent 1"));
        }

        #[test]
        fn timeout_leaves_preferences_intact() {
            let steam = MockSteam::new([
                ClientState {
                    running: true,
                    ..Default::default()
                },
                ClientState {
                    running: true,
                    ..Default::default()
                },
            ]);
            assert_eq!(
                switch_with(&steam, TARGET, 1, 1),
                Err("steam_shutdown_timeout")
            );
            assert_eq!(*steam.actions.borrow(), ["shutdown"]);
        }

        #[test]
        fn refuses_to_launch_if_preference_file_cannot_be_written() {
            let steam = MockSteam {
                write_fails: true,
                ..MockSteam::new([])
            };
            assert_eq!(switch_with(&steam, TARGET, 1, 1), Err("steam_write_failed"));
            assert_eq!(*steam.actions.borrow(), ["file"]);
            assert_eq!(&*steam.vdf.borrow(), SOURCE);
        }

        #[test]
        fn failed_launch_restores_prior_selection() {
            let steam = MockSteam {
                launch_fails: true,
                ..MockSteam::new([])
            };
            assert_eq!(
                switch_with(&steam, TARGET, 1, 1),
                Err("steam_launch_failed")
            );
            assert_eq!(&*steam.vdf.borrow(), SOURCE);
            assert_eq!(
                steam.registry.borrow().as_deref(),
                Some(OsStr::new("old_user"))
            );
        }

        #[test]
        fn launched_without_confirmed_login_reports_waiting() {
            let steam = MockSteam::new([
                ClientState::default(),
                ClientState::default(),
                ClientState {
                    running: true,
                    ..Default::default()
                },
            ]);
            assert_eq!(switch_with(&steam, TARGET, 1, 1), Ok("waiting_login"));
        }

        #[test]
        fn concurrent_switch_is_rejected_before_discovery() {
            let _in_progress = SWITCH.lock().unwrap();
            assert_eq!(switch_account(TARGET), Err("steam_switch_in_progress"));
        }
    }
}
