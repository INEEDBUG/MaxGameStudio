//! Relocate the desktop storage in a verified staging directory on the new volume.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::CommandExt,
    },
    path::{Component, Path, PathBuf, Prefix},
    process::Command,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{ERROR_NO_MORE_FILES, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{
        FileAttributeTagInfo, GetDiskFreeSpaceExW, GetDriveTypeW, GetFileInformationByHandleEx,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
    },
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
        SystemInformation::GetSystemDirectoryW,
    },
};

const MARKER: &str = ".storage-migration-v1.json";
const LAYOUT: [&str; 3] = ["data", "webview", "league-runtime"];
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const MOUNT_POINT_TAG: u32 = 0xa000_0003;

type LegacyPaths = (PathBuf, PathBuf, PathBuf);

#[derive(PartialEq, Eq)]
struct CopySource {
    from: PathBuf,
    relative: PathBuf,
}

#[derive(PartialEq, Eq)]
struct MigrationPlan {
    copies: Vec<CopySource>,
    invocation: Value,
    aliases: BTreeMap<PathBuf, PathBuf>,
}

#[derive(PartialEq, Eq)]
struct FileEntry {
    source: PathBuf,
    size: u64,
    digest: [u8; 32],
}

#[derive(Default, PartialEq, Eq)]
struct Contents {
    directories: BTreeSet<PathBuf>,
    files: BTreeMap<PathBuf, FileEntry>,
}

pub(crate) fn migrate(
    root: &Path,
    legacy: Option<&LegacyPaths>,
    destination: &Path,
) -> io::Result<()> {
    migrate_checked(root, legacy, destination, ensure_idle)
}

fn migrate_checked(
    root: &Path,
    legacy: Option<&LegacyPaths>,
    destination: &Path,
    check_writers: impl Fn() -> io::Result<()>,
) -> io::Result<()> {
    let destination = canonical_local(destination)?;
    let plan = plan_migration(root, legacy, &destination)?;
    check_writers()?;
    let source = capture(&plan.copies)?;
    let expected = manifest(&source)?;
    if let Some(metadata) = ordinary_metadata(&destination)? {
        if !metadata.is_dir() {
            return Err(io::Error::other("目标数据路径是文件。"));
        }
        if let Some(marker) = read_marker(&destination.join(MARKER))? {
            if marker["version"] == 1
                && marker["invocation"] == plan.invocation
                && marker["manifest"] == expected
                && destination_manifest(&destination)? == expected
            {
                return Ok(());
            }
        }
        if fs::read_dir(&destination)?.next().transpose()?.is_some() {
            return Err(io::Error::other("目标数据目录已有文件。"));
        }
    }

    let parent = destination
        .parent()
        .ok_or_else(|| io::Error::other("无效的数据目录。"))?;
    fs::create_dir_all(parent)?;
    let parent = canonical_local(parent)?;
    let required = source.files.values().try_fold(0u64, |total, file| {
        total
            .checked_add(file.size)
            .ok_or_else(|| io::Error::other("数据目录大小超出范围。"))
    })?;
    let unused = available_bytes(&parent)?;
    if unused < required.saturating_add((64 * 1024 * 1024).max(required / 10)) {
        return Err(io::Error::other("目标磁盘剩余空间不足。"));
    }
    let stage = tempfile::Builder::new()
        .prefix(".storage-migration-")
        .tempdir_in(parent)?;
    secure_staging(stage.path())?;
    for directory in &source.directories {
        fs::create_dir_all(stage.path().join(directory))?;
    }
    for (relative, entry) in &source.files {
        let source = open_ordinary_file(&entry.source)?;
        let staged = stage.path().join(relative);
        if let Some(parent) = staged.parent() {
            fs::create_dir_all(parent)?;
        }
        // Keep the exact source file open without allowing writes or replacement while copying.
        fs::copy(&entry.source, staged)?;
        drop(source);
    }
    for name in LAYOUT {
        fs::create_dir_all(stage.path().join(name))?;
    }
    if destination_manifest(stage.path())? != expected {
        return Err(io::Error::other("复制后的数据与源文件校验不一致。"));
    }
    let marker = json!({
        "version": 1,
        "invocation": plan.invocation,
        "legacy_aliases": plan.aliases,
        "rollback_sources": plan.copies.iter().map(|pair| &pair.from).collect::<Vec<_>>(),
        "file_count": source.files.len(),
        "byte_count": required,
        "manifest": expected,
    });
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(stage.path().join(MARKER))?;
    serde_json::to_writer_pretty(&mut file, &marker)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);

    check_writers()?;
    if plan_migration(root, legacy, &destination)? != plan || capture(&plan.copies)? != source {
        return Err(io::Error::other("源数据在复制期间发生变化。"));
    }
    canonical_local(&destination)?;
    if ordinary_metadata(&destination)?.is_some() {
        // remove_dir only succeeds for an empty directory, even after a concurrent write.
        fs::remove_dir(&destination)?;
    }
    fs::rename(stage.path(), destination)?;
    Ok(())
}

fn plan_migration(
    root: &Path,
    legacy: Option<&LegacyPaths>,
    destination: &Path,
) -> io::Result<MigrationPlan> {
    let root = canonical_local(root)?;
    if overlaps(&root, destination) {
        return Err(io::Error::other("新旧数据目录不能相同或相互包含。"));
    }
    let mut aliases = BTreeMap::new();
    let mut copies = Vec::new();
    let mut invocation = json!({
        "mode": "source", "source": root, "destination": destination,
        "legacy_appdata": null, "legacy_localappdata": null,
    });
    if let Some(legacy) = legacy {
        let paths = [
            canonical_local(&legacy.0)?,
            canonical_local(&legacy.1)?,
            canonical_local(&legacy.2)?,
        ];
        for (name, path) in LAYOUT.into_iter().zip(paths.iter()) {
            if ordinary_metadata(path)?.is_some_and(|metadata| metadata.is_dir()) {
                if name == "data" {
                    add_legacy_data(path, destination, &mut copies, &mut aliases)?;
                } else {
                    copies.push(CopySource {
                        from: path.clone(),
                        relative: PathBuf::from(name),
                    });
                }
            }
        }
        for name in ["window-state.json", ".window-state.json"] {
            let state = root.join(name);
            if ordinary_metadata(&state)?.is_some_and(|metadata| metadata.is_file()) {
                copies.push(CopySource {
                    from: state,
                    relative: PathBuf::from("window-state.json"),
                });
                break;
            }
        }
        invocation["mode"] = json!("selected_legacy");
        invocation["source"] = Value::Null;
        invocation["layout"] = json!([&root, &paths[0], &paths[1], &paths[2]]);
    } else {
        if !ordinary_metadata(&root)?.is_some_and(|metadata| metadata.is_dir()) {
            return Err(io::Error::other("原数据目录不存在。"));
        }
        copies.push(CopySource {
            from: root,
            relative: PathBuf::new(),
        });
    }
    if copies.iter().any(|copy| overlaps(&copy.from, destination)) {
        return Err(io::Error::other("目标目录与旧数据目录重叠。"));
    }
    copies.sort_by(|left, right| left.relative.cmp(&right.relative));
    Ok(MigrationPlan {
        copies,
        invocation,
        aliases,
    })
}

fn add_legacy_data(
    data: &Path,
    destination: &Path,
    copies: &mut Vec<CopySource>,
    aliases: &mut BTreeMap<PathBuf, PathBuf>,
) -> io::Result<()> {
    let children = fs::read_dir(data)?.collect::<Result<Vec<_>, _>>()?;
    let mut split = Vec::new();
    for child in children {
        let source = child.path();
        let metadata = fs::symlink_metadata(&source)?;
        let mut copy_path = source.clone();
        if reparse(&metadata) {
            copy_path = legacy_cache_target(&source, destination)?;
            if overlaps(data, &copy_path) {
                return Err(io::Error::other("旧缓存位置与原数据目录重叠。"));
            }
            aliases.insert(source, copy_path.clone());
        }
        split.push(CopySource {
            from: copy_path,
            relative: Path::new("data").join(child.file_name()),
        });
    }
    if aliases.is_empty() {
        copies.push(CopySource {
            from: data.to_path_buf(),
            relative: PathBuf::from("data"),
        });
    } else {
        copies.extend(split);
    }
    Ok(())
}

fn legacy_cache_target(link: &Path, destination: &Path) -> io::Result<PathBuf> {
    let unsupported = || io::Error::other("旧缓存链接必须指向所选的本地非系统盘。");
    let name = link
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(unsupported)?;
    if !matches!(name.to_ascii_lowercase().as_str(), "cache" | "trash") {
        return Err(unsupported());
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(link)?;
    let mut tag = FILE_ATTRIBUTE_TAG_INFO::default();
    let status = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileAttributeTagInfo,
            (&mut tag as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            std::mem::size_of_val(&tag) as u32,
        )
    };
    if status == 0 || tag.ReparseTag != MOUNT_POINT_TAG {
        return Err(unsupported());
    }
    let target = canonical_local(&fs::read_link(link)?)?;
    let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    let target_drive = drive_letter(&target).ok_or_else(unsupported)?;
    if Some(target_drive) != drive_letter(destination)
        || system
            .as_bytes()
            .first()
            .is_some_and(|letter| letter.eq_ignore_ascii_case(&target_drive))
        || !target.is_dir()
    {
        return Err(unsupported());
    }
    Ok(target)
}

fn drive_letter(path: &Path) -> Option<u8> {
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return None;
    };
    match prefix.kind() {
        Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => Some(letter.to_ascii_uppercase()),
        _ => None,
    }
}

fn canonical_local(path: &Path) -> io::Result<PathBuf> {
    let letter =
        drive_letter(path).ok_or_else(|| io::Error::other("数据目录必须使用本地绝对路径。"))?;
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(io::Error::other("数据目录必须使用本地绝对路径。"));
    }
    let volume = [u16::from(letter), b':' as u16, b'\\' as u16, 0];
    // DRIVE_REMOVABLE and DRIVE_FIXED are the ordinary local volume types.
    if !matches!(unsafe { GetDriveTypeW(volume.as_ptr()) }, 2 | 3) {
        return Err(io::Error::other("数据目录必须位于本地磁盘。"));
    }
    let ancestors: Vec<_> = path.ancestors().collect();
    for component in ancestors.iter().rev() {
        ordinary_metadata(component)?;
    }
    for ancestor in ancestors {
        match ancestor.canonicalize() {
            Ok(base) => {
                let suffix = path.strip_prefix(ancestor).map_err(io::Error::other)?;
                let joined = if suffix.as_os_str().is_empty() {
                    base
                } else {
                    base.join(suffix)
                };
                let full = joined
                    .to_str()
                    .ok_or_else(|| io::Error::other("数据路径无法编码。"))?;
                return Ok(PathBuf::from(full.strip_prefix(r"\\?\").unwrap_or(full)));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::other("找不到数据目录所在磁盘。"))
}

fn overlaps(left: &Path, right: &Path) -> bool {
    let left = PathBuf::from(left.to_string_lossy().to_lowercase());
    let right = PathBuf::from(right.to_string_lossy().to_lowercase());
    left.starts_with(&right) || right.starts_with(&left)
}

fn reparse(metadata: &Metadata) -> bool {
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn ordinary_metadata(path: &Path) -> io::Result<Option<Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if reparse(&metadata) => Err(io::Error::other("数据路径包含链接或重解析点。")),
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn capture(copies: &[CopySource]) -> io::Result<Contents> {
    let mut contents = Contents::default();
    for copy in copies {
        let mut pending = vec![(copy.from.clone(), copy.relative.clone())];
        while let Some((source, relative)) = pending.pop() {
            let metadata = ordinary_metadata(&source)?
                .ok_or_else(|| io::Error::other("源数据文件已移动。"))?;
            if metadata.is_dir() {
                contents.directories.insert(relative.clone());
                for child in fs::read_dir(&source)? {
                    let child = child?;
                    if source == copy.from && child.file_name() == MARKER {
                        continue;
                    }
                    pending.push((child.path(), relative.join(child.file_name())));
                }
            } else if metadata.is_file() {
                let digest = digest(&source)?;
                if contents
                    .files
                    .insert(
                        relative,
                        FileEntry {
                            source,
                            size: metadata.len(),
                            digest,
                        },
                    )
                    .is_some()
                {
                    return Err(io::Error::other("多个旧文件对应同一迁移路径。"));
                }
            } else {
                return Err(io::Error::other("源数据包含不支持的文件类型。"));
            }
        }
    }
    Ok(contents)
}

fn open_ordinary_file(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let metadata = file.metadata()?;
    if reparse(&metadata) || !metadata.is_file() {
        return Err(io::Error::other("迁移源已变成链接或其他文件类型。"));
    }
    Ok(file)
}

fn digest(path: &Path) -> io::Result<[u8; 32]> {
    let mut file = open_ordinary_file(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(hash.finalize().into())
}

fn manifest(contents: &Contents) -> io::Result<Value> {
    let mut files = serde_json::Map::new();
    for (relative, file) in &contents.files {
        let name = relative
            .to_str()
            .ok_or_else(|| io::Error::other("文件名无法编码。"))?
            .replace('\\', "/");
        let sha256: String = file
            .digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        files.insert(name, json!({"size": file.size, "sha256": sha256}));
    }
    Ok(Value::Object(files))
}

fn destination_manifest(destination: &Path) -> io::Result<Value> {
    manifest(&capture(&[CopySource {
        from: destination.to_path_buf(),
        relative: PathBuf::new(),
    }])?)
}

fn read_marker(path: &Path) -> io::Result<Option<Value>> {
    if ordinary_metadata(path)?.is_some_and(|metadata| metadata.is_file()) {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        if reparse(&file.metadata()?) {
            return Err(io::Error::other("迁移记录是链接。"));
        }
        return Ok(serde_json::from_reader(io::BufReader::new(file)).ok());
    }
    Ok(None)
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
}

fn available_bytes(path: &Path) -> io::Result<u64> {
    let mut free = 0u64;
    let status = unsafe {
        GetDiskFreeSpaceExW(
            wide(path).as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if status == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(free)
}

fn secure_staging(path: &Path) -> io::Result<()> {
    let mut base = [0u16; 1024];
    let length = unsafe { GetSystemDirectoryW(base.as_mut_ptr(), base.len() as u32) } as usize;
    if length == 0 || length >= base.len() {
        return Err(io::Error::last_os_error());
    }
    let system = PathBuf::from(OsString::from_wide(&base[..length]));
    let output = Command::new(system.join("whoami.exe"))
        .args(["/user", "/fo", "csv", "/nh"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    let principal = String::from_utf8_lossy(&output.stdout);
    let sid = principal
        .trim()
        .rsplit(',')
        .next()
        .unwrap_or_default()
        .trim_matches('"');
    if !output.status.success()
        || !sid.starts_with("S-1-")
        || !sid[4..].split('-').all(|segment| {
            !segment.is_empty() && segment.bytes().all(|digit| digit.is_ascii_digit())
        })
    {
        return Err(io::Error::other("无法验证启动用户身份。"));
    }
    let applied = Command::new(system.join("icacls.exe"))
        .arg(path)
        .args([
            "/inheritance:r",
            "/grant:r",
            &format!("*{sid}:(OI)(CI)F"),
            "*S-1-5-18:(OI)(CI)F",
            "*S-1-5-32-544:(OI)(CI)F",
            "/q",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    if !applied.status.success() {
        return Err(io::Error::other("无法保护数据迁移暂存目录。"));
    }
    Ok(())
}

fn ensure_idle() -> io::Result<()> {
    let endpoint = SocketAddr::from((Ipv4Addr::LOCALHOST, 19871));
    for attempt in 0..3 {
        if TcpStream::connect_timeout(&endpoint, Duration::from_millis(400)).is_ok() {
            return Err(io::Error::other("旧版服务仍在运行。关闭旧版后可重新迁移。"));
        }
        if !another_desktop_is_running()? {
            return Ok(());
        }
        if attempt < 2 {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    Err(io::Error::other(
        "旧版桌面程序仍在运行。关闭旧版后可重新迁移。",
    ))
}

fn another_desktop_is_running() -> io::Result<bool> {
    let handle = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let snapshot = unsafe { OwnedHandle::from_raw_handle(handle) };
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut next = unsafe { Process32FirstW(snapshot.as_raw_handle(), &mut entry) } != 0;
    while next {
        let end = entry
            .szExeFile
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(entry.szExeFile.len());
        if entry.th32ProcessID != std::process::id()
            && matches!(
                String::from_utf16_lossy(&entry.szExeFile[..end])
                    .to_ascii_lowercase()
                    .as_str(),
                "maxgamestudioleague.exe"
                    | "cs2-insight-agent-desktop.exe"
                    | "maxgamestudio.exe"
                    | "cs2 insight agent.exe"
                    | "cs2 ultimate insight studio.exe"
            )
        {
            return Ok(true);
        }
        next = unsafe { Process32NextW(snapshot.as_raw_handle(), &mut entry) } != 0;
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() != Some(ERROR_NO_MORE_FILES as i32) {
        return Err(error);
    }
    Ok(false)
}

#[cfg(test)]
#[path = "storage_migration_tests.rs"]
mod tests;
