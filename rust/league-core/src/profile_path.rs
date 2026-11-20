//! Qualification of Windows profile paths without creating or changing them.
#![cfg(windows)]

use std::os::windows::{ffi::OsStrExt, fs::MetadataExt};
use std::path::{Component, Path, PathBuf, Prefix};
use std::{error::Error, ffi::OsStr, fmt, fs, io};

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
const DRIVE_REMOTE: u32 = 4;
const CSTR_EQUAL: i32 = 2;

#[link(name = "kernel32")]
extern "system" {
    fn GetDriveTypeW(root: *const u16) -> u32;
    fn CompareStringOrdinal(
        first: *const u16,
        first_length: i32,
        second: *const u16,
        second_length: i32,
        ignore_case: i32,
    ) -> i32;
}

#[derive(Debug)]
pub enum ProfilePathError {
    NotLocalDisk,
    DiskRoot,
    NotDirectory,
    Link,
    InvalidName,
    Io(io::Error),
}

impl fmt::Display for ProfilePathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotLocalDisk => "请选择本机磁盘上的数据文件夹。",
            Self::DiskRoot => "不能将磁盘根目录用作数据文件夹。",
            Self::NotDirectory => "数据路径必须是普通文件夹。",
            Self::Link => "数据路径不能经过快捷方式、联接或符号链接。",
            Self::InvalidName => "数据路径包含无效的文件夹名称。",
            Self::Io(_) => "无法核验数据目录。请确认磁盘可用且当前账户有访问权限。",
        })
    }
}

impl Error for ProfilePathError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for ProfilePathError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Validate a local, absolute Windows directory and return a regular DOS path.
/// Missing final directories are allowed. This checks the current filesystem state only.
pub fn league_profile_path(input: &Path) -> Result<PathBuf, ProfilePathError> {
    let mut components = input.components();
    let prefix = match components.next() {
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_)) => prefix,
        _ => return Err(ProfilePathError::NotLocalDisk),
    };
    if !matches!(components.next(), Some(Component::RootDir)) {
        return Err(ProfilePathError::NotLocalDisk);
    }

    let mut volume_root = prefix.as_os_str().to_os_string();
    volume_root.push("\\");
    let root = PathBuf::from(volume_root);
    let mut directory = root.clone();
    for component in components {
        match component {
            Component::Normal(name) if ordinary_name(name) => directory.push(name),
            Component::ParentDir => {
                directory.pop();
            }
            Component::CurDir => {}
            _ => return Err(ProfilePathError::InvalidName),
        }
    }
    if directory == root {
        return Err(ProfilePathError::DiskRoot);
    }

    let volume: Vec<_> = root.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: the original drive prefix and root separator are NUL-terminated.
    if unsafe { GetDriveTypeW(volume.as_ptr()) } == DRIVE_REMOTE {
        return Err(ProfilePathError::NotLocalDisk);
    }

    let mut existing = root.clone();
    for segment in directory
        .strip_prefix(&root)
        .expect("built on the drive root")
    {
        existing.push(segment);
        let metadata = match fs::symlink_metadata(&existing) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                existing.pop();
                break;
            }
            Err(error) => return Err(error.into()),
        };
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(if existing == directory {
                ProfilePathError::NotDirectory
            } else {
                ProfilePathError::Link
            });
        }
        if !metadata.is_dir() {
            return Err(ProfilePathError::NotDirectory);
        }
    }

    let canonical = fs::canonicalize(&existing)?;
    if !same_local_path(&existing, &canonical)? {
        return Err(ProfilePathError::Link);
    }
    // Win32 may return a verbatim prefix or different letter case. Preserve a validated
    // ordinary drive path and the user's spelling for the existing locator contract.
    Ok(directory)
}

fn ordinary_name(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    if name.ends_with(['.', ' '])
        || name.chars().any(|character| {
            character < ' ' || matches!(character, '<' | '>' | ':' | '"' | '|' | '?' | '*')
        })
    {
        return false;
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ');
    let device = stem.to_ascii_uppercase();
    if matches!(
        device.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return false;
    }
    !matches!(
        device
            .strip_prefix("COM")
            .or_else(|| device.strip_prefix("LPT")),
        Some("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³")
    )
}

pub(crate) fn same_local_path(input: &Path, canonical: &Path) -> Result<bool, ProfilePathError> {
    let prefix_length = match canonical.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(_) => 0,
            Prefix::VerbatimDisk(_) => 4, // The UTF-16 length of \\?\.
            _ => return Ok(false),
        },
        _ => return Ok(false),
    };
    let first: Vec<_> = input.as_os_str().encode_wide().collect();
    let canonical: Vec<_> = canonical.as_os_str().encode_wide().collect();
    let second = &canonical[prefix_length..];
    let first_length = i32::try_from(first.len()).map_err(|_| ProfilePathError::InvalidName)?;
    let second_length = i32::try_from(second.len()).map_err(|_| ProfilePathError::InvalidName)?;
    // SAFETY: both arrays stay alive for the call, with exact lengths in UTF-16 units.
    let comparison = unsafe {
        CompareStringOrdinal(
            first.as_ptr(),
            first_length,
            second.as_ptr(),
            second_length,
            1,
        )
    };
    if comparison == 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(comparison == CSTR_EQUAL)
}
