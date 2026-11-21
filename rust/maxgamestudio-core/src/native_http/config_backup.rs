#[cfg(windows)]
pub(super) mod snapshot;

use super::{json_reply, HttpResponse};
use crate::profile_platform::{path_text, probe_process, CS2_CONFIG_BACKUP_DIR};
use hyper::StatusCode;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{self, BufReader, Read},
    path::{Component, Path, PathBuf},
};
use tempfile::NamedTempFile;

#[derive(Deserialize)]
struct Manifest {
    entries: Vec<SavedConfig>,
}

#[derive(Deserialize)]
struct SavedConfig {
    original: PathBuf,
    #[serde(default = "existed_by_default")]
    existed: bool,
    backup_relpath: Option<PathBuf>,
    native_sha256: Option<String>,
}

fn existed_by_default() -> bool {
    true
}

pub(super) fn restore(data_dir: &Path) -> HttpResponse {
    match restore_profile(data_dir) {
        Ok(Some(restored)) => json_reply(
            StatusCode::OK,
            json!({"ok":true,"code":"CONFIG_RESTORE_OK","restored":restored}),
        ),
        Ok(None) => json_reply(
            StatusCode::OK,
            json!({"ok":true,"code":"CONFIG_RESTORE_NOT_NEEDED","restored":0}),
        ),
        Err(RestoreError::GameRunning) => json_reply(
            StatusCode::CONFLICT,
            json!({"detail":{"code":"CS2_RUNNING"}}),
        ),
        Err(RestoreError::Partial(path, error)) => partial(&path, &error),
    }
}

pub(super) enum RestoreError {
    GameRunning,
    Partial(PathBuf, io::Error),
}

pub(super) fn restore_profile(data_dir: &Path) -> Result<Option<usize>, RestoreError> {
    let backup = data_dir.join(CS2_CONFIG_BACKUP_DIR);
    let state_path = backup.join("recording_state.json");
    let mut state: Value = match read_json(&state_path) {
        Ok(state) => state,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Value::Null,
        Err(error) => return Err(RestoreError::Partial(state_path, error)),
    };
    if state["status"] != "recording" {
        return Ok(None);
    }
    let manifest = backup.join("manifest.json");
    let entries = prepare(&backup, &manifest)
        .map_err(|(original, error)| RestoreError::Partial(original, error))?;
    match probe_process("cs2.exe") {
        Ok(true) => return Err(RestoreError::GameRunning),
        Ok(false) => {}
        Err(error) => return Err(RestoreError::Partial(state_path, error)),
    }
    let mut restored = 0;
    for (target, source, hash) in entries {
        match restore_file(&target, source.as_deref(), hash.as_deref()) {
            Ok(wrote) => restored += usize::from(wrote),
            Err(error) => return Err(RestoreError::Partial(target, error)),
        }
    }
    state["status"] = json!("recorded");
    state["message"] = json!("玩家配置状态正常");
    (|| {
        let mut staged = NamedTempFile::new_in(&backup)?;
        serde_json::to_writer_pretty(&mut staged, &state)?;
        commit(staged, &state_path)
    })()
    .map_err(|error| RestoreError::Partial(state_path, error))?;
    Ok(Some(restored))
}

type RestoreFiles = Vec<(PathBuf, Option<PathBuf>, Option<String>)>;

fn prepare(backup: &Path, manifest: &Path) -> Result<RestoreFiles, (PathBuf, io::Error)> {
    let saved: Manifest = read_json(manifest).map_err(|error| (manifest.to_owned(), error))?;
    if saved.entries.is_empty() {
        return Err((manifest.to_owned(), invalid("备份清单中没有可恢复的配置")));
    }
    let backup = backup
        .canonicalize()
        .map_err(|error| (backup.to_owned(), error))?;
    let mut seen = HashSet::new();
    saved
        .entries
        .into_iter()
        .map(|entry| {
            let (target, source) =
                restore_paths(&entry, &backup).map_err(|error| (entry.original.clone(), error))?;
            let key = path_text(&target);
            let key = if cfg!(windows) {
                key.to_lowercase()
            } else {
                key
            };
            if !seen.insert(key) {
                return Err((entry.original, invalid("备份清单中有重复的配置路径")));
            }
            Ok((target, source, entry.native_sha256))
        })
        .collect()
}

fn restore_paths(entry: &SavedConfig, backup: &Path) -> io::Result<(PathBuf, Option<PathBuf>)> {
    let original = &entry.original;
    let parent = original
        .parent()
        .filter(|_| original.is_absolute())
        .ok_or_else(|| invalid("配置文件的路径必须为绝对路径"))?;
    let name = original
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| valid_config_name(&name.to_ascii_lowercase()))
        .ok_or_else(|| invalid("备份清单包含未识别的 CS2 配置文件"))?;
    let parent = parent.canonicalize()?;
    if !valid_config_dir(&parent) {
        return Err(invalid("配置文件须位于 CS2 或 Steam 玩家配置目录内"));
    }
    let target = parent.join(name);
    // Checking the file type without following links also prevents removing a directory.
    existing_file(&target)?;
    let source = if entry.existed {
        let relative = entry
            .backup_relpath
            .as_ref()
            .filter(|path| {
                path.components()
                    .all(|part| matches!(part, Component::Normal(_)))
            })
            .ok_or_else(|| invalid("备份清单中的相对路径无效"))?;
        let source = backup.join(relative).canonicalize()?;
        if !source.starts_with(backup) || !existing_file(&source)? {
            return Err(invalid("配置备份文件须位于当前备份目录内"));
        }
        verify_digest(&source, entry.native_sha256.as_deref())?;
        Some(source)
    } else {
        None
    };
    Ok((target, source))
}

fn valid_config_name(name: &str) -> bool {
    matches!(
        name,
        "config.cfg"
            | "cs2_user.cfg"
            | "cs2_machine_convars.vcfg"
            | "video.txt"
            | "cs2_video.txt"
            | "localconfig.vdf"
    ) || !name.contains(':')
        && (name.ends_with(".vcfg_lastclouded")
            || name.ends_with(".vcfg")
                && [
                    "user_convars_0_slot",
                    "cs2_user_convars_0_slot",
                    "cs2_user_keys",
                ]
                .iter()
                .any(|prefix| name.starts_with(prefix)))
}

fn valid_config_dir(path: &Path) -> bool {
    let tail: Vec<_> = path
        .components()
        .rev()
        .take(5)
        .map(|part| part.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect();
    let tail: Vec<_> = tail.iter().map(String::as_str).collect();
    match tail.as_slice() {
        ["cfg", "csgo", ..] => true,
        ["cfg", "local", "730", id, "userdata"] | ["config", id, "userdata", ..] => {
            !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit())
        }
        _ => false,
    }
}

fn restore_file(target: &Path, source: Option<&Path>, hash: Option<&str>) -> io::Result<bool> {
    let exists = existing_file(target)?;
    let Some(source) = source else {
        if exists {
            fs::remove_file(target)?;
            if existing_file(target)? {
                return Err(io::Error::other("录制期间生成的文件仍然存在"));
            }
        }
        return Ok(exists);
    };
    if exists {
        let already_restored = match hash {
            Some(hash) => format!("{:x}", digest(target)?) == hash,
            None => same_content(target, source)?,
        };
        if already_restored {
            return Ok(false);
        }
    }
    let mut staged = NamedTempFile::new_in(target.parent().expect("validated config path"))?;
    io::copy(&mut File::open(source)?, &mut staged)?;
    verify_digest(staged.path(), hash)?;
    commit(staged, target)?;
    if !same_content(target, source)? {
        return Err(io::Error::other("还原后的文件与备份内容不同"));
    }
    Ok(true)
}

fn verify_digest(path: &Path, hash: Option<&str>) -> io::Result<()> {
    if let Some(hash) = hash {
        if format!("{:x}", digest(path)?) != hash {
            return Err(invalid("玩家配置备份内容校验失败"));
        }
    }
    Ok(())
}

fn digest(path: &Path) -> io::Result<sha2::digest::Output<Sha256>> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut bytes = [0_u8; 32 * 1024];
    loop {
        let read = file.read(&mut bytes)?;
        if read == 0 {
            break;
        }
        digest.update(&bytes[..read]);
    }
    Ok(digest.finalize())
}

fn existing_file(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(info) if info.is_file() => Ok(true),
        Ok(_) => Err(invalid("备份或配置路径须指向普通文件")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn same_content(left: &Path, right: &Path) -> io::Result<bool> {
    let (mut left, mut right) = (File::open(left)?, File::open(right)?);
    let mut remaining = left.metadata()?.len();
    if remaining != right.metadata()?.len() {
        return Ok(false);
    }
    let (mut a, mut b) = ([0; 16 * 1024], [0; 16 * 1024]);
    while remaining > 0 {
        let count = remaining.min(a.len() as u64) as usize;
        left.read_exact(&mut a[..count])?;
        right.read_exact(&mut b[..count])?;
        if a[..count] != b[..count] {
            return Ok(false);
        }
        remaining -= count as u64;
    }
    Ok(true)
}

fn commit(file: NamedTempFile, destination: &Path) -> io::Result<()> {
    file.as_file().sync_all()?;
    file.persist(destination).map_err(|error| error.error)?;
    Ok(())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> io::Result<T> {
    serde_json::from_reader(BufReader::new(File::open(path)?)).map_err(io::Error::from)
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn partial(path: &Path, error: &io::Error) -> HttpResponse {
    json_reply(
        StatusCode::OK,
        json!({"ok":false,"code":"CONFIG_RESTORE_PARTIAL","failed":[{"original":path_text(path),"error":error.to_string()}]}),
    )
}
