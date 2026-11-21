//! Save the player configuration before any recording process can change it.

use super::{
    commit, digest, existing_file, invalid, read_json, same_content, valid_config_dir,
    valid_config_name,
};
use crate::profile_platform::{path_text, probe_process, CS2_CONFIG_BACKUP_DIR};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tempfile::{Builder, NamedTempFile};

const ALWAYS: &[&str] = &[
    "config.cfg",
    "cs2_user.cfg",
    "cs2_machine_convars.vcfg",
    "video.txt",
    "cs2_video.txt",
    "user_convars_0_slot0.vcfg",
    "cs2_user_convars_0_slot0.vcfg",
    "cs2_user_keys_0_slot0.vcfg",
    "localconfig.vdf",
];
const GENERATION_PREFIX: &str = "native-player-config-";

pub(in crate::native_http) fn capture_for_game(data: &Path, game: &Path) -> io::Result<()> {
    let steam = crate::profile_platform::registry_steam_path();
    let directories = config_directories(game, steam.as_deref())?;
    capture(data, &directories)
}

fn config_directories(game: &Path, registered_steam: Option<&Path>) -> io::Result<Vec<PathBuf>> {
    let mut directories = vec![game.join("csgo/cfg")];
    let install = game.parent();
    if let Some(directory) = install
        .map(|path| path.join("csgo/cfg"))
        .filter(|path| path.is_dir())
    {
        directories.push(directory);
    }
    let library = install
        .and_then(Path::parent)
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("common"))
        })
        .and_then(Path::parent)
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("steamapps"))
        })
        .and_then(Path::parent);
    for root in library.into_iter().chain(registered_steam) {
        let userdata = match fs::read_dir(root.join("userdata")) {
            Ok(userdata) => userdata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for account in userdata {
            let account = account?;
            if !account
                .file_name()
                .to_str()
                .is_some_and(|id| !id.is_empty() && id.bytes().all(|digit| digit.is_ascii_digit()))
            {
                continue;
            }
            for folder in ["730/local/cfg", "config"] {
                let path = account.path().join(folder);
                match fs::metadata(&path) {
                    Ok(metadata) if metadata.is_dir() => directories.push(path),
                    Ok(_) => {
                        return Err(invalid(
                            "Steam player configuration path is not a directory",
                        ))
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
        }
    }
    Ok(directories)
}

pub(in crate::native_http) fn capture(data: &Path, directories: &[PathBuf]) -> io::Result<()> {
    if probe_process("cs2.exe")? {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "CS2 must be closed before saving player configuration",
        ));
    }
    let backup = data.join(CS2_CONFIG_BACKUP_DIR);
    fs::create_dir_all(&backup)?;
    let backup_metadata = fs::symlink_metadata(&backup)?;
    if backup_metadata.file_type().is_symlink() || redirected(&backup_metadata) {
        return Err(invalid("Player backup directory is redirected"));
    }
    let backup = backup.canonicalize()?;
    let state_path = backup.join("recording_state.json");
    let manifest_path = backup.join("manifest.json");
    let prior_state: Value = optional_json(&state_path)?;
    if prior_state["status"] == "recording" {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Existing player configuration recovery is pending",
        ));
    }
    let prior_manifest: Value = optional_json(&manifest_path)?;
    let targets = config_paths(directories)?;
    let snapshots = Builder::new()
        .prefix(GENERATION_PREFIX)
        .tempdir_in(&backup)?;
    let generation = snapshots
        .path()
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| invalid("Invalid native backup generation name"))?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_secs_f64();
    let mut entries = Vec::with_capacity(targets.len());
    for (index, original) in targets.into_values().enumerate() {
        let name = original
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| invalid("Invalid player configuration filename"))?;
        let path = path_text(&original);
        let exists = existing_file(&original)?;
        let mut entry = json!({"original":path,"filename":name,"existed":exists});
        if exists {
            let owned_name = format!("{index}.cfg");
            let saved = snapshots.path().join(&owned_name);
            let mut source = File::open(&original)?;
            let mut target = File::create_new(&saved)?;
            io::copy(&mut source, &mut target)?;
            target.sync_all()?;
            drop(target);
            if !same_content(&original, &saved)? {
                return Err(io::Error::other(
                    "Player configuration changed during its backup",
                ));
            }
            entry["backup_relpath"] = json!(format!("{generation}/{owned_name}"));
            entry["native_sha256"] = json!(format!("{:x}", digest(&saved)?));
        }
        entries.push(entry);
    }
    let manifest = json!({
        "version":4,"created_at":timestamp,"native_generation":generation,"entries":entries,
    });
    let state = json!({
        "version":1,"status":"recording","started_at":timestamp,
        "backup_dir":path_text(&backup),"message":"录制中，玩家配置已备份，等待正常恢复",
    });
    let manifest_file = json_file(&backup, &manifest)?;
    let state_file = json_file(&backup, &state)?;
    commit(manifest_file, &manifest_path)?;
    let _saved = snapshots.keep();
    // Recording cannot begin until both parts of the existing recovery
    // contract have been published. A failed state write keeps both copies.
    commit(state_file, &state_path)?;
    retire_previous_generation(&backup, &prior_manifest);
    Ok(())
}

fn optional_json(path: &Path) -> io::Result<Value> {
    if existing_file(path)? {
        read_json(path)
    } else {
        Ok(Value::Null)
    }
}

fn json_file(root: &Path, value: &Value) -> io::Result<NamedTempFile> {
    let mut file = NamedTempFile::new_in(root)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    Ok(file)
}

fn config_paths(directories: &[PathBuf]) -> io::Result<BTreeMap<String, PathBuf>> {
    let mut paths = BTreeMap::new();
    for raw in directories {
        let directory = raw.canonicalize()?;
        if !valid_config_dir(&directory) {
            return Err(invalid("Unexpected CS2 or Steam player config directory"));
        }
        let remember = |paths: &mut BTreeMap<String, PathBuf>, path: PathBuf| {
            let key = path_text(&path).to_lowercase();
            paths.entry(key).or_insert(path);
        };
        for name in ALWAYS {
            remember(&mut paths, directory.join(name));
        }
        for file in fs::read_dir(&directory)? {
            let file = file?;
            if file
                .file_name()
                .to_str()
                .is_some_and(|name| valid_config_name(&name.to_ascii_lowercase()))
            {
                remember(&mut paths, file.path());
            }
        }
    }
    if paths.is_empty() {
        return Err(invalid("No player configuration directories were located"));
    }
    Ok(paths)
}

fn retire_previous_generation(root: &Path, manifest: &Value) {
    let Some(name) = manifest["native_generation"].as_str() else {
        return;
    };
    let Some(suffix) = name.strip_prefix(GENERATION_PREFIX) else {
        return;
    };
    if suffix.len() != 6 || !suffix.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return;
    }
    let dir = root.join(name);
    if !dir
        .symlink_metadata()
        .is_ok_and(|attrs| attrs.is_dir() && !attrs.file_type().is_symlink() && !redirected(&attrs))
    {
        return;
    }
    let Some(entries) = manifest["entries"].as_array() else {
        return;
    };
    for entry in entries {
        let Some(path) = entry["backup_relpath"].as_str() else {
            continue;
        };
        let Some(file) = path
            .strip_prefix(name)
            .and_then(|tail| tail.strip_prefix('/'))
        else {
            continue;
        };
        let Some(index) = file.strip_suffix(".cfg") else {
            continue;
        };
        if index.is_empty() || !index.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let path = dir.join(file);
        if let Some(expected) = entry["native_sha256"].as_str() {
            if matches!(existing_file(&path), Ok(true))
                && digest(&path).is_ok_and(|hash| format!("{hash:x}") == expected)
            {
                let _ = fs::remove_file(path);
            }
        }
    }
    // A file outside an earlier generated manifest is never removed.
    let _ = fs::remove_dir(dir);
}

#[cfg(windows)]
fn redirected(attributes: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
    attributes.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn redirected(_: &fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_and_restores_real_fixture_bytes_and_preserves_other_files() {
        let isolated = tempfile::tempdir().unwrap();
        let profile = isolated.path().join("profile");
        let game = isolated.path().join("game/csgo/cfg");
        let steam = isolated.path().join("steam/userdata/123/730/local/cfg");
        for dir in [&profile, &game, &steam] {
            fs::create_dir_all(dir).unwrap();
        }
        let convars = game.join("cs2_machine_convars.vcfg");
        let keys = steam.join("cs2_user_keys_0_slot0.vcfg");
        let generated = steam.join("cs2_video.txt");
        let generated_keys = game.join("cs2_user_keys_0_slot0.vcfg");
        let unrelated = steam.join("unrelated.cfg");
        fs::write(&convars, b"unchanged original convars").unwrap();
        fs::write(&keys, b"original keybindings").unwrap();
        fs::write(&unrelated, b"original unrelated").unwrap();

        let dirs = [game, steam];
        capture(&profile, &dirs).unwrap();
        let backup = profile.join(CS2_CONFIG_BACKUP_DIR);
        let before = fs::read(backup.join("manifest.json")).unwrap();
        let old: Value = serde_json::from_slice(&before).unwrap();
        let previous_generation = backup.join(old["native_generation"].as_str().unwrap());
        fs::write(&convars, b"recording changed a convar").unwrap();
        fs::write(&generated, b"game generated").unwrap();
        fs::write(&generated_keys, b"game generated keybindings").unwrap();
        fs::write(&unrelated, b"preserve independent change").unwrap();
        assert_eq!(
            capture(&profile, &dirs).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(backup.join("manifest.json")).unwrap(), before);

        let saved_keys = old["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["original"] == path_text(&keys.canonicalize().unwrap()))
            .unwrap()["backup_relpath"]
            .as_str()
            .unwrap();
        let saved_keys = backup.join(saved_keys);
        fs::write(&saved_keys, b"truncated backup").unwrap();
        assert!(matches!(
            super::super::restore_profile(&profile),
            Err(super::super::RestoreError::Partial(_, _))
        ));
        assert_eq!(fs::read(&convars).unwrap(), b"recording changed a convar");
        assert!(generated_keys.exists());
        let pending: Value = read_json(&backup.join("recording_state.json")).unwrap();
        assert_eq!(pending["status"], "recording");
        fs::write(&saved_keys, b"original keybindings").unwrap();

        assert_eq!(
            super::super::restore(&profile).status(),
            hyper::StatusCode::OK
        );
        assert_eq!(fs::read(&convars).unwrap(), b"unchanged original convars");
        assert_eq!(fs::read(&keys).unwrap(), b"original keybindings");
        assert_eq!(
            fs::read(&unrelated).unwrap(),
            b"preserve independent change"
        );
        assert!(!generated.exists());
        assert!(!generated_keys.exists());
        let state: Value = read_json(&backup.join("recording_state.json")).unwrap();
        assert_eq!(state["status"], "recorded");
        assert!(previous_generation.exists());
        capture(&profile, &dirs).unwrap();
        assert!(!previous_generation.exists());
    }
}
