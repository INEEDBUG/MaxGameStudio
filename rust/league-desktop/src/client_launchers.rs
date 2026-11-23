//! Detect and open the same League launchers as the pinned renderer.

use serde_json::{json, Value};
use std::{
    env,
    ffi::OsStr,
    fs, io,
    os::windows::ffi::OsStrExt,
    path::{Component, Path, PathBuf, Prefix},
    ptr,
};
use windows_sys::Win32::{
    Foundation::ERROR_SUCCESS,
    Storage::FileSystem::GetDriveTypeW,
    System::{
        Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE},
        Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ},
        WindowsProgramming::DRIVE_FIXED,
    },
    UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
};

pub(crate) const NAMESPACE: &str = "client-installation-main";

pub(crate) enum Launcher {
    Tcls,
    WeGameLeague,
    WeGame,
    Riot,
}

impl Launcher {
    pub(crate) fn from_method(method: &str) -> Option<Self> {
        Some(match method {
            "launchTencentTcls" => Self::Tcls,
            "launchWeGameLeagueOfLegends" => Self::WeGameLeague,
            "launchWeGame" => Self::WeGame,
            "launchDefaultRiotClient" => Self::Riot,
            _ => return None,
        })
    }

    pub(crate) fn launch(self) -> Result<Value, io::Error> {
        let installs = Installations::detect();
        let executable = match self {
            Self::Tcls => installs.tcls,
            Self::WeGameLeague => installs.wegame_league,
            Self::WeGame => installs.wegame,
            Self::Riot => installs.riot,
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "未找到所选客户端。"))?;
        let args = matches!(self, Self::Riot)
            .then_some("--launch-product=league_of_legends --launch-patchline=live");
        open_executable(&executable, args)?;
        Ok(Value::Null)
    }
}

#[derive(Default)]
struct Installations {
    tencent_root: Option<PathBuf>,
    tcls: Option<PathBuf>,
    wegame_league: Option<PathBuf>,
    wegame: Option<PathBuf>,
    riot: Option<PathBuf>,
}

impl Installations {
    fn detect() -> Self {
        let mut installs = Self::default();
        if let Some(root) = registry_string(r"Software\Tencent\LOL", "InstallPath") {
            installs.probe_tencent(Path::new(root.trim().trim_matches('"')));
        }
        for letter in b'C'..=b'Z' {
            if installs.tcls.is_some() && installs.wegame_league.is_some() {
                break;
            }
            if fixed_drive(letter) {
                let root = PathBuf::from(format!(r"{}:\WeGameApps\英雄联盟", char::from(letter)));
                installs.probe_tencent(&root);
            }
        }
        installs.wegame = registry_string(r"wegame\DefaultIcon", "")
            .as_deref()
            .and_then(icon_executable)
            .and_then(existing_executable);
        installs.riot = riot_client();
        installs
    }

    fn probe_tencent(&mut self, root: &Path) {
        if self.tcls.is_none() {
            self.tcls = existing_executable(root.join(r"Launcher\Client.exe"));
        }
        if self.wegame_league.is_none() {
            self.wegame_league = existing_executable(root.join(r"WeGameLauncher\launcher.exe"));
        }
        if self.tencent_root.is_none() && (self.tcls.is_some() || self.wegame_league.is_some()) {
            self.tencent_root = Some(root.to_owned());
        }
    }
}

pub(crate) fn state() -> Value {
    let paths = Installations::detect();
    json!({
        "leagueClientExecutablePaths": [],
        "tencentInstallationPath": paths.tencent_root,
        "tclsExecutablePath": paths.tcls,
        "weGameLauncherExecutablePath": paths.wegame_league,
        "weGameExecutablePath": paths.wegame,
        "officialRiotClientExecutablePath": paths.riot,
    })
}

fn fixed_drive(letter: u8) -> bool {
    let root = [u16::from(letter), b':' as u16, b'\\' as u16, 0];
    // The slice holds a complete NUL-terminated drive root.
    unsafe { GetDriveTypeW(root.as_ptr()) == DRIVE_FIXED }
}

fn existing_executable(executable: impl Into<PathBuf>) -> Option<PathBuf> {
    let executable = executable.into();
    let mut parts = executable.components();
    let Some(Component::Prefix(prefix)) = parts.next() else {
        return None;
    };
    let (Prefix::Disk(letter) | Prefix::VerbatimDisk(letter)) = prefix.kind() else {
        return None;
    };
    if parts.next() != Some(Component::RootDir)
        || !executable.extension()?.eq_ignore_ascii_case("exe")
        || !fixed_drive(letter)
    {
        return None;
    }
    executable.is_file().then_some(executable)
}

fn icon_executable(icon: &str) -> Option<&Path> {
    let icon = icon.trim();
    let path = match icon.strip_prefix('"') {
        Some(quoted) => quoted.split_once('"')?.0,
        None => icon.rsplit_once(',').map_or(icon, |(path, _)| path).trim(),
    };
    Some(Path::new(path))
}

fn riot_client() -> Option<PathBuf> {
    let root = env::var_os("ProgramData")?;
    let registry_file = PathBuf::from(root).join(r"Riot Games\RiotClientInstalls.json");
    let installs: Value = serde_json::from_slice(&fs::read(registry_file).ok()?).ok()?;
    installs["associated_client"].as_object()?.values().find_map(|value| {
        let path = Path::new(value.as_str()?);
        if !path.file_name()?.eq_ignore_ascii_case("RiotClientServices.exe") {
            return None;
        }
        existing_executable(path)
    })
}

fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain([0]).collect()
}

fn registry_string(key: &str, name: &str) -> Option<String> {
    let key = wide(key);
    let name = wide(name);
    let mut units = vec![0u16; 32_768];
    let mut bytes = (units.len() * size_of::<u16>()) as u32;
    // Read only the named current-user value into the buffer advertised in bytes.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
            ptr::null_mut(),
            units.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let end = units.iter().position(|&unit| unit == 0)?;
    String::from_utf16(&units[..end]).ok()
}

fn open_executable(executable: &Path, args: Option<&str>) -> io::Result<()> {
    let directory = wide(executable.parent().expect("an absolute executable has a parent"));
    let executable = wide(executable);
    let verb = wide("open");
    let args = args.map(wide);
    // Shell execution takes place on a blocking worker. Initialize its COM
    // apartment here and balance any successful initialization.
    let initialized = unsafe {
        CoInitializeEx(ptr::null(), (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32)
    };
    if initialized < 0 {
        return Err(io::Error::other(format!("无法初始化 Windows 启动服务 ({initialized:#x})。")));
    }
    let result = unsafe {
        let handle = ShellExecuteW(
            ptr::null_mut(),
            verb.as_ptr(),
            executable.as_ptr(),
            args.as_ref().map_or(ptr::null(), |args| args.as_ptr()),
            directory.as_ptr(),
            SW_SHOWNORMAL,
        );
        CoUninitialize();
        handle as isize
    };
    if result <= 32 {
        return Err(io::Error::other(format!("Windows 启动客户端失败 ({result})。")));
    }
    Ok(())
}
