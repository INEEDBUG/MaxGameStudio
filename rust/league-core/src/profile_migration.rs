//! Stage a League profile before publishing the active location used by the runtime.
#![cfg(windows)]

use crate::profile_location::ProfileLocation;
use crate::profile_path::{league_profile_path, same_local_path, ProfilePathError};
use crate::profile_snapshot::snapshot_profile_database;
use std::error::Error;
use std::ffi::OsStr;
use std::fs::{self, File, Metadata};
use std::io::{self, BufRead, BufReader, Write};
use std::os::windows::{ffi::OsStrExt, fs::MetadataExt};
use std::path::{Path, PathBuf};

const ACTIVE_LOCATOR: &str = "data-location.txt";
const PENDING_LOCATOR: &str = "installer-data-location.txt";
const PROFILE_MARKER: &str = ".maxgamestudio-league-profile";
const MARKER_VERSION: &[u8] = b"1\n";
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

#[link(name = "kernel32")]
extern "system" {
    fn MoveFileExW(existing: *const u16, destination: *const u16, flags: u32) -> i32;
}

#[derive(Debug)]
pub enum ProfileMigrationError {
    Path(ProfilePathError),
    Io(io::Error),
    SourceUnavailable,
    Overlap,
    ProgramOverlap,
    ControlOverlap,
    Occupied,
    UnsupportedEntry,
    SettingsChanged,
    Database(Box<dyn Error + Send + Sync>),
}

impl std::fmt::Display for ProfileMigrationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::Path(error) => return std::fmt::Display::fmt(error, formatter),
            Self::Io(_) => "工具箱无法完成数据准备，请确认原目录可读，目标磁盘可用。",
            Self::SourceUnavailable => "原数据文件夹暂时无法访问。",
            Self::Overlap => "新旧数据文件夹不能相互包含。",
            Self::ProgramOverlap => "数据文件夹必须与程序安装文件夹分开。",
            Self::ControlOverlap => "数据文件夹不能包含工具箱的位置设置目录。",
            Self::Occupied => "目标数据文件夹已有其他文件，请选择空文件夹。",
            Self::UnsupportedEntry => "数据准备时发现不支持的链接或特殊文件。",
            Self::SettingsChanged => "设置文件在迁移过程中发生变化，请重试。",
            Self::Database(_) => "旧版工具箱数据库无法备份或未通过检查。",
        };
        formatter.write_str(message)
    }
}

impl Error for ProfileMigrationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Path(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Database(error) => Some(error.as_ref()),
            _ => None,
        }
    }
}

impl From<ProfilePathError> for ProfileMigrationError {
    fn from(error: ProfilePathError) -> Self {
        Self::Path(error)
    }
}

impl From<io::Error> for ProfileMigrationError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Prepare the profile under the caller's single-instance lock.
/// `legacy` is the resolved embedded profile, used only when there is no active location.
pub fn prepare_league_profile(
    control: &Path,
    install_root: &Path,
    legacy: Option<&Path>,
) -> Result<PathBuf, ProfileMigrationError> {
    let control = league_profile_path(control)?;
    let program = league_profile_path(install_root)?;
    let active_file = control.join(ACTIVE_LOCATOR);
    ordinary_file(&active_file)?;
    ordinary_file(&control.join(PENDING_LOCATOR))?;

    let location = ProfileLocation::read(&control)?;
    let destination = league_profile_path(&location.selected)?;
    if contains(&program, &destination)? || contains(&destination, &program)? {
        return Err(ProfileMigrationError::ProgramOverlap);
    }
    if contains(&destination, &control)? {
        return Err(ProfileMigrationError::ControlOverlap);
    }

    let source = location
        .active
        .as_deref()
        .or(legacy)
        .map(league_profile_path)
        .transpose()?;
    if let Some(original) = source.as_deref() {
        if entry_metadata(original)?.is_none() {
            return Err(ProfileMigrationError::SourceUnavailable);
        }
        if location.active.is_some() && same_local_path(original, &destination)? {
            return Ok(destination);
        }
        if contains(original, &destination)? || contains(&destination, original)? {
            return Err(ProfileMigrationError::Overlap);
        }
    }

    if !marked_profile(&destination)? {
        migrate_to_empty_destination(source.as_deref(), &destination)?;
    }
    // Leave the installer selection intact. If this final rename fails, the marker allows retry.
    fs::create_dir_all(&control)?;
    league_profile_path(&control)?;
    ordinary_file(&active_file)?;
    let mut locator = tempfile::NamedTempFile::new_in(&control)?;
    let bytes: Vec<_> = [0xfeff]
        .into_iter()
        .chain(destination.as_os_str().encode_wide())
        .flat_map(u16::to_le_bytes)
        .collect();
    locator.write_all(&bytes)?;
    locator.as_file().sync_all()?;
    locator.persist(&active_file).map_err(|error| error.error)?;
    Ok(destination)
}

fn contains(base: &Path, target: &Path) -> Result<bool, ProfileMigrationError> {
    for parent in target.ancestors() {
        if same_local_path(base, parent)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn entry_metadata(path: &Path) -> Result<Option<Metadata>, ProfileMigrationError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(ProfileMigrationError::UnsupportedEntry);
    }
    Ok(Some(metadata))
}

fn ordinary_file(path: &Path) -> Result<Option<Metadata>, ProfileMigrationError> {
    let metadata = entry_metadata(path)?;
    if metadata
        .as_ref()
        .is_some_and(|metadata| !metadata.is_file())
    {
        return Err(ProfileMigrationError::UnsupportedEntry);
    }
    Ok(metadata)
}

fn marked_profile(destination: &Path) -> Result<bool, ProfileMigrationError> {
    let marker = destination.join(PROFILE_MARKER);
    let metadata = ordinary_file(&marker)?;
    Ok(
        metadata.is_some_and(|metadata| metadata.len() == MARKER_VERSION.len() as u64)
            && fs::read(marker)? == MARKER_VERSION,
    )
}

fn destination_exists_and_is_empty(destination: &Path) -> Result<bool, ProfileMigrationError> {
    if entry_metadata(destination)?.is_none() {
        return Ok(false);
    }
    if fs::read_dir(destination)?.next().transpose()?.is_some() {
        return Err(ProfileMigrationError::Occupied);
    }
    Ok(true)
}

fn migrate_to_empty_destination(
    source: Option<&Path>,
    destination: &Path,
) -> Result<(), ProfileMigrationError> {
    destination_exists_and_is_empty(destination)?;
    let parent = destination.parent().expect("validated non-root path");
    fs::create_dir_all(parent)?;
    // The volume root is valid as a staging parent. Validate the full destination again instead.
    league_profile_path(destination)?;
    let stage = tempfile::Builder::new()
        .prefix(".mgs-league-")
        .tempdir_in(parent)?;
    if let Some(source) = source {
        for name in ["base-config.json", "AkariConfig"] {
            let setting = source.join(name);
            if let Some(metadata) = entry_metadata(&setting)? {
                copy_settings(&setting, &stage.path().join(name), metadata)?;
            }
        }
        let database = source.join("LeagueAkari.db");
        if ordinary_file(&database)?.is_some() {
            for sidecar in ["LeagueAkari.db-wal", "LeagueAkari.db-shm"] {
                ordinary_file(&source.join(sidecar))?;
            }
            snapshot_profile_database(&database, &stage.path().join("LeagueAkari.db"))
                .map_err(ProfileMigrationError::Database)?;
        }
    }
    let mut marker = File::create_new(stage.path().join(PROFILE_MARKER))?;
    marker.write_all(MARKER_VERSION)?;
    marker.sync_all()?;
    drop(marker);

    // The folder has not been touched while staging. Refuse newly arrived files and never replace them.
    league_profile_path(destination)?;
    if destination_exists_and_is_empty(destination)? {
        fs::remove_dir(destination)?;
    }
    move_directory_without_replacing(stage.path(), destination)?;
    Ok(())
}

fn copy_settings(
    source: &Path,
    target: &Path,
    metadata: Metadata,
) -> Result<(), ProfileMigrationError> {
    if metadata.is_dir() {
        fs::create_dir(target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            let child = entry.path();
            let metadata = entry_metadata(&child)?.ok_or(ProfileMigrationError::SettingsChanged)?;
            copy_settings(&child, &target.join(entry.file_name()), metadata)?;
        }
    } else if metadata.is_file() {
        let mut saved = File::create_new(target)?;
        io::copy(&mut File::open(source)?, &mut saved)?;
        saved.sync_all()?;
        drop(saved);
        if !same_contents(source, target)? {
            return Err(ProfileMigrationError::SettingsChanged);
        }
    } else {
        return Err(ProfileMigrationError::UnsupportedEntry);
    }
    Ok(())
}

fn same_contents(first: &Path, second: &Path) -> io::Result<bool> {
    let mut first = BufReader::new(File::open(first)?);
    let mut second = BufReader::new(File::open(second)?);
    loop {
        let a = first.fill_buf()?;
        let b = second.fill_buf()?;
        let count = a.len().min(b.len());
        if count == 0 {
            return Ok(a.is_empty() && b.is_empty());
        }
        if a[..count] != b[..count] {
            return Ok(false);
        }
        first.consume(count);
        second.consume(count);
    }
}

fn move_directory_without_replacing(source: &Path, destination: &Path) -> io::Result<()> {
    fn extended(path: &Path) -> Vec<u16> {
        OsStr::new(r"\\?\")
            .encode_wide()
            .chain(path.as_os_str().encode_wide())
            .chain([0])
            .collect()
    }
    let from = extended(source);
    let to = extended(destination);
    // SAFETY: Both DOS paths are absolute and NUL-terminated and remain live during the call.
    // With no flags, the move fails if another entry occupies the destination.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
