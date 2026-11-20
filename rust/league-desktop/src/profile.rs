use maxgamestudio_league_core::{
    profile_database::LeagueProfileDatabase, profile_location::ProfileLocation,
    profile_migration::prepare_league_profile, profile_path::league_profile_path,
};
use serde_json::Value;
use std::{
    error::Error,
    fs, io,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::Manager;
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS},
    System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ},
};

pub(crate) struct DesktopProfile {
    pub(crate) directory: PathBuf,
    pub(crate) database: Mutex<LeagueProfileDatabase>,
}

impl DesktopProfile {
    pub(crate) fn prepare(app: &tauri::App) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let roaming = app.path().data_dir()?;
        let control = roaming.join("MaxGameStudioLeague");
        let install_root = std::env::current_exe()?;
        let install_root = install_root.parent().ok_or("工具箱安装目录不可用。")?;
        let legacy = if ProfileLocation::read(&control)?.active.is_none() {
            legacy_profile(&roaming, read_main_location()?.as_deref())?
        } else {
            None
        };
        let directory = prepare_league_profile(&control, install_root, legacy.as_deref())?;
        let database = Mutex::new(LeagueProfileDatabase::open(&directory)?);
        Ok(Self {
            directory,
            database,
        })
    }

    pub(crate) fn webview_directory(&self) -> io::Result<PathBuf> {
        let directory =
            league_profile_path(&self.directory.join("WebView2")).map_err(io::Error::other)?;
        fs::create_dir_all(&directory)?;
        Ok(directory)
    }
}

fn legacy_profile(roaming: &Path, location: Option<&str>) -> io::Result<Option<PathBuf>> {
    if let Some(saved) = location {
        let value: Value = serde_json::from_str(saved)?;
        let prior_profile = if value["mode"] == "legacy_in_place" {
            value["paths"]["league_runtime"].as_str().map(PathBuf::from)
        } else {
            value["root"]
                .as_str()
                .map(|root| Path::new(root).join("league-runtime"))
        };
        if let Some(profile) = prior_profile {
            if profile.try_exists()? {
                return Ok(Some(profile));
            }
        }
    }
    let fallback = roaming.join("MaxGameStudio").join("league-runtime");
    Ok(fallback.try_exists()?.then_some(fallback))
}

fn read_main_location() -> io::Result<Option<String>> {
    let wide = |text: &str| {
        std::ffi::OsStr::new(text)
            .encode_wide()
            .chain([0])
            .collect::<Vec<_>>()
    };
    let key = wide(r"Software\MaxGameStudio\Storage");
    let value = wide("LocationV1");
    let mut bytes = 0;
    // Read the current user's existing locator. The first call obtains its size.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut bytes,
        )
    };
    match status {
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => return Ok(None),
        ERROR_SUCCESS if (2..=32_768).contains(&bytes) && bytes % 2 == 0 => {}
        ERROR_SUCCESS => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "旧版位置记录无效。",
            ))
        }
        error => return Err(io::Error::from_raw_os_error(error as i32)),
    }
    let mut units = vec![0u16; bytes as usize / 2];
    // The buffer has exactly the advertised writable length in bytes.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            units.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let utf16 = units.split(|unit| *unit == 0).next().unwrap_or_default();
    let location = String::from_utf16(utf16)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok((!location.trim().is_empty()).then_some(location))
}
