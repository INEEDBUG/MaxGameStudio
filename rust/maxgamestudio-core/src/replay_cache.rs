//! Read whole-match replay caches and repair missing frames from complete legacy metadata.

use crate::{
    parser_worker::ParserWorker,
    profile_platform::{path_text, resolved_path},
    replay_packet::{replay_parquet_binary_packet, MAX_SAMPLE_TICKS},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

const REPLAY_MATCH_CACHE_VERSION: u64 = 5;
pub(crate) const REQUIRED_PARSER_VERSION: &str = "0.41.4+cs2insight7";
const METADATA_LIMIT: u64 = 16 * 1024 * 1024;
const CURRENT_CACHE: &str = "cache/demo-replay/matches";
const LEGACY_CACHE: &str = "replay-match";
const NAMESPACES: [(&str, &str, &str); 3] = [
    ("matches", CURRENT_CACHE, LEGACY_CACHE),
    ("frames", "cache/demo-replay/rounds", "replay-frames"),
    (
        "effects",
        "cache/demo-replay/effects",
        "replay-effects-cache",
    ),
];

#[derive(Clone, Copy, Default)]
struct CacheUsage {
    files: u64,
    bytes: u64,
}

impl CacheUsage {
    fn add(&mut self, other: Self) {
        self.files = self.files.saturating_add(other.files);
        self.bytes = self.bytes.saturating_add(other.bytes);
    }

    fn as_json(self) -> Value {
        json!({"files": self.files, "bytes": self.bytes})
    }
}

pub(crate) fn cache_summary(data: &Path) -> io::Result<Value> {
    let mut details = serde_json::Map::new();
    let mut total = CacheUsage::default();
    let mut total_legacy = CacheUsage::default();
    for (namespace, current, legacy) in NAMESPACES {
        let current = data.join(current);
        let legacy = data.join(legacy);
        let old_usage = directory_usage(&legacy)?;
        let aliases = fs::canonicalize(&current)
            .ok()
            .zip(fs::canonicalize(&legacy).ok())
            .is_some_and(|(current, legacy)| same_path(&current, &legacy));
        let usage = if aliases {
            old_usage
        } else {
            let mut usage = directory_usage(&current)?;
            usage.add(old_usage);
            usage
        };
        details.insert(namespace.to_owned(), usage.as_json());
        total.add(usage);
        total_legacy.add(old_usage);
    }
    Ok(json!({
        "path": path_text(&resolved_path(&data.join("cache/demo-replay"))?),
        "persistent": true,
        "files": total.files,
        "bytes": total.bytes,
        "legacy_files": total_legacy.files,
        "legacy_bytes": total_legacy.bytes,
        "namespaces": details,
    }))
}

pub(crate) fn clear_cache(data: &Path) -> io::Result<Value> {
    let mut removed = CacheUsage::default();
    match fs::canonicalize(data) {
        Ok(root) => {
            for (namespace, current, legacy) in NAMESPACES {
                for relative in [current, legacy] {
                    let expected = root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
                    let folder = match fs::canonicalize(&expected) {
                        Ok(path) if same_path(&path, &expected) => path,
                        Ok(_) => continue,
                        Err(error) if absent_directory(&error) => continue,
                        Err(error) => return Err(error),
                    };
                    let entries = match fs::read_dir(folder) {
                        Ok(entries) => entries,
                        Err(error) if absent_directory(&error) => continue,
                        Err(error) => return Err(error),
                    };
                    for entry in entries {
                        let entry = entry?;
                        if !entry
                            .file_name()
                            .to_str()
                            .is_some_and(|name| managed_file(namespace, name))
                        {
                            continue;
                        }
                        match entry.metadata() {
                            Ok(info) if info.is_file() => match fs::remove_file(entry.path()) {
                                Ok(()) => removed.add(CacheUsage {
                                    files: 1,
                                    bytes: info.len(),
                                }),
                                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                                Err(error) => eprintln!(
                                    "Native replay cache file could not be removed: {}",
                                    error.kind()
                                ),
                            },
                            Ok(_) => {}
                            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                            Err(error) => return Err(error),
                        }
                    }
                }
            }
        }
        Err(error) if absent_directory(&error) => {}
        Err(error) => return Err(error),
    }
    Ok(json!({
        "removed_files": removed.files,
        "removed_bytes": removed.bytes,
        "cache": cache_summary(data)?,
    }))
}

fn same_path(left: &Path, right: &Path) -> bool {
    if cfg!(windows) {
        left.as_os_str().eq_ignore_ascii_case(right.as_os_str())
    } else {
        left == right
    }
}

fn managed_file(namespace: &str, name: &str) -> bool {
    let Some((key, extension)) = name.split_once('.') else {
        return false;
    };
    key.len() == 40
        && key.bytes().all(|c| c.is_ascii_hexdigit())
        && matches!(
            (namespace, extension),
            ("matches", "parquet" | "meta.json")
                | ("frames", "json.gz")
                | ("effects", "pkl" | "meta.json")
        )
}

fn absent_directory(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
    )
}

fn directory_usage(path: &Path) -> io::Result<CacheUsage> {
    let files = match fs::read_dir(path) {
        Ok(files) => files,
        Err(error) if absent_directory(&error) => {
            return Ok(CacheUsage::default());
        }
        Err(error) => return Err(error),
    };
    let mut usage = CacheUsage::default();
    for entry in files {
        let entry = entry?;
        // A cache file may vanish while a window closes. Skip it and do not descend into folders.
        match fs::metadata(entry.path()) {
            Ok(info) if info.is_file() => usage.add(CacheUsage {
                files: 1,
                bytes: info.len(),
            }),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(usage)
}

#[cfg(test)]
mod cache_usage_tests {
    use super::*;

    #[test]
    fn counts_flat_current_and_legacy_replay_namespaces_without_creating_directories() {
        let fixture = tempfile::tempdir().unwrap();
        let data = fixture.path().join("Existing Profile");
        let matches = data.join(CURRENT_CACHE);
        let legacy_effects = data.join("replay-effects-cache");
        fs::create_dir_all(matches.join("ignored-directory")).unwrap();
        fs::create_dir_all(&legacy_effects).unwrap();
        fs::write(matches.join("synthetic.parquet"), [0; 5]).unwrap();
        fs::write(matches.join("synthetic.meta.json"), [0; 7]).unwrap();
        fs::write(matches.join("ignored-directory/unknown.bin"), [0; 32]).unwrap();
        fs::write(legacy_effects.join("effects.json"), [0; 11]).unwrap();
        fs::write(data.join("unrelated-profile.json"), [0; 64]).unwrap();

        let summary = cache_summary(&data).unwrap();
        assert_eq!(summary["files"], 3);
        assert_eq!(summary["bytes"], 23);
        assert_eq!(summary["legacy_files"], 1);
        assert_eq!(summary["legacy_bytes"], 11);
        assert_eq!(
            summary["namespaces"],
            json!({
                "matches": {"files": 2, "bytes": 12},
                "frames": {"files": 0, "bytes": 0},
                "effects": {"files": 1, "bytes": 11},
            })
        );
        assert!(!data.join("cache/demo-replay/rounds").exists());
        assert_eq!(
            fs::metadata(data.join("unrelated-profile.json"))
                .unwrap()
                .len(),
            64
        );
    }
}

#[derive(Clone)]
pub(crate) struct ReplayRequest {
    pub path: String,
    pub start_tick: i64,
    pub end_tick: i64,
    pub tick_rate: f64,
    pub fps: f64,
}

impl ReplayRequest {
    pub(crate) fn validate_range(&self) -> Result<(), String> {
        if self.end_tick <= self.start_tick {
            return Err("end_tick must be greater than start_tick".to_owned());
        }
        let ticks = self.end_tick - self.start_tick;
        if ticks > (self.tick_rate * 10.0 * 60.0) as i64 {
            return Err("Replay range cannot exceed 10 minutes".to_owned());
        }
        let estimate = ((ticks as f64 / self.tick_rate) * self.fps) as usize + 1;
        if estimate > MAX_SAMPLE_TICKS {
            return Err(format!(
                "Binary replay request would generate about {estimate} frames; maximum is {MAX_SAMPLE_TICKS}"
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) enum ReplayCacheError {
    EmptyPath,
    MissingDemo(String),
    Unavailable(String),
}

pub(crate) fn load_packet(
    data_dir: &Path,
    upload_dir: &Path,
    request: &ReplayRequest,
) -> Result<Option<Vec<u8>>, ReplayCacheError> {
    let demo = resolve_demo(upload_dir, &request.path)?;
    let Some(key) = cache_key(&demo, request.fps, request.tick_rate) else {
        return Ok(None);
    };

    for root in [data_dir.join(CURRENT_CACHE), data_dir.join(LEGACY_CACHE)] {
        let parquet = root.join(format!("{key}.parquet"));
        let meta_file = root.join(format!("{key}.meta.json"));
        if !parquet.is_file() || !meta_file.is_file() {
            continue;
        }
        let Some(meta) = read_metadata(&meta_file)? else {
            continue;
        };
        if meta["version"].as_u64() != Some(REPLAY_MATCH_CACHE_VERSION) || meta["cache_key"] != key
        {
            continue;
        }
        return read_round(&parquet, &meta, &key, request);
    }
    Ok(None)
}

/// A repair may reuse complete, matching metadata while rebuilding the missing Parquet.
/// The existing metadata and all unrelated cache files stay untouched.
pub(crate) struct ReplayRepair {
    demo: PathBuf,
    parquet: PathBuf,
    meta_file: PathBuf,
    meta: Value,
    key: String,
    stage: tempfile::TempDir,
    request: ReplayRequest,
}

impl ReplayRepair {
    pub(crate) fn prepare(
        data: &Path,
        uploads: &Path,
        request: ReplayRequest,
    ) -> Result<Option<Self>, ReplayCacheError> {
        let demo = resolve_demo(uploads, &request.path)?;
        let Some(key) = cache_key(&demo, request.fps, request.tick_rate) else {
            return Ok(None);
        };
        for root in [data.join(CURRENT_CACHE), data.join(LEGACY_CACHE)] {
            let parquet = root.join(format!("{key}.parquet"));
            let meta_file = root.join(format!("{key}.meta.json"));
            if parquet.try_exists().map_err(repair_error)? {
                continue;
            }
            let Some(meta) = read_metadata(&meta_file)? else {
                continue;
            };
            if meta["version"].as_u64() != Some(REPLAY_MATCH_CACHE_VERSION)
                || meta["parser_runtime"] != REQUIRED_PARSER_VERSION
                || meta["cache_key"] != key
                || !entries(&meta["rounds"]).iter().any(|round| {
                    integer(&round["start_tick"]) == request.start_tick
                        && integer(&round["end_tick"]) == request.end_tick
                })
            {
                continue;
            }
            let stage = tempfile::Builder::new()
                .prefix("mgs-replay-")
                .tempdir_in(root)
                .map_err(repair_error)?;
            return Ok(Some(Self {
                demo,
                parquet,
                meta_file,
                meta,
                key,
                stage,
                request,
            }));
        }
        Ok(None)
    }

    pub(crate) async fn generate(&self, worker: &ParserWorker) -> Result<Value, ReplayCacheError> {
        let rounds: Vec<_> = entries(&self.meta["rounds"])
            .iter()
            .map(|round| {
                json!({
                    "round_number": integer(&round["round_number"]),
                    "start_tick": integer(&round["start_tick"]),
                    "end_tick": integer(&round["end_tick"]),
                })
            })
            .collect();
        worker
            .write_replay(
                &self.demo,
                &self.temporary_path(),
                self.request.tick_rate,
                self.request.fps,
                &json!(rounds),
            )
            .await
            .map_err(repair_error)
    }

    fn temporary_path(&self) -> PathBuf {
        self.stage.path().join("rebuilt.parquet.partial")
    }

    pub(crate) fn finish(self, result: &Value) -> Result<Option<Vec<u8>>, ReplayCacheError> {
        let saved = entries(&self.meta["rounds"]);
        let generated = entries(&result["rounds"]);
        let groups = entries(&result["row_groups"]);
        if saved.len() != generated.len()
            || saved.len() != groups.len()
            || !saved
                .iter()
                .zip(generated.iter().zip(groups))
                .all(|(old, (new, group))| {
                    old["round_number"] == new["round_number"]
                        && old["start_tick"] == new["start_tick"]
                        && old["end_tick"] == new["end_tick"]
                        && old["sample_ticks"] == new["sample_ticks"]
                        && old["round_number"] == group["round_number"]
                        && old["row_group"] == group["row_group"]
                        && old["player_rows"] == group["rows"]
                })
        {
            return Err(ReplayCacheError::Unavailable(
                "Parser worker did not reproduce the stored replay frames".into(),
            ));
        }
        let stable_demo = cache_key(&self.demo, self.request.fps, self.request.tick_rate)
            .as_deref()
            == Some(self.key.as_str());
        if !stable_demo || read_metadata(&self.meta_file)?.as_ref() != Some(&self.meta) {
            return Err(ReplayCacheError::Unavailable(
                "Replay source or metadata changed during repair".into(),
            ));
        }
        let staged = self.temporary_path();
        let packet = read_round(&staged, &self.meta, &self.key, &self.request)?;
        if packet.is_none() {
            return Ok(None);
        }
        let result = tempfile::TempPath::try_from_path(&staged)
            .map_err(repair_error)?
            .persist_noclobber(&self.parquet);
        match result {
            Ok(()) => Ok(packet),
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                read_round(&self.parquet, &self.meta, &self.key, &self.request)
            }
            Err(error) => Err(repair_error(error.error)),
        }
    }
}

fn repair_error(error: io::Error) -> ReplayCacheError {
    ReplayCacheError::Unavailable(error.to_string())
}

fn resolve_demo(upload_dir: &Path, path: &str) -> Result<PathBuf, ReplayCacheError> {
    let requested = path.trim();
    if requested.is_empty() {
        return Err(ReplayCacheError::EmptyPath);
    }
    let candidate = Path::new(requested);
    let resolved = if candidate.is_file() {
        fs::canonicalize(candidate).ok()
    } else {
        candidate
            .file_name()
            .map(|name| upload_dir.join(name))
            .filter(|path| path.is_file())
            .and_then(|path| fs::canonicalize(path).ok())
    };
    resolved.ok_or_else(|| ReplayCacheError::MissingDemo(requested.to_owned()))
}

fn cache_key(path: &Path, fps: f64, tick_rate: f64) -> Option<String> {
    let stat = fs::metadata(path).ok()?;
    let modified = stat.modified().ok()?;
    let mtime_ns = match modified.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos() as i128,
        Err(error) => -(error.duration().as_nanos() as i128),
    };
    let identity = format!(
        "{REPLAY_MATCH_CACHE_VERSION}|{REQUIRED_PARSER_VERSION}|{}|{}|{mtime_ns}|{fps:.6}|{tick_rate:.6}",
        path_text(path),
        stat.len(),
    );
    let hash = format!("{:x}", Sha256::digest(identity.as_bytes()));
    Some(hash[..40].to_owned())
}

fn read_metadata(path: &Path) -> Result<Option<Value>, ReplayCacheError> {
    let Ok(file) = File::open(path) else {
        return Ok(None);
    };
    let Ok(stat) = file.metadata() else {
        return Ok(None);
    };
    if stat.len() > METADATA_LIMIT {
        return Err(ReplayCacheError::Unavailable(
            "replay metadata exceeds the size limit".to_owned(),
        ));
    }
    let mut data = Vec::with_capacity(stat.len() as usize);
    if file
        .take(METADATA_LIMIT + 1)
        .read_to_end(&mut data)
        .is_err()
    {
        return Ok(None);
    }
    if data.len() as u64 > METADATA_LIMIT {
        return Err(ReplayCacheError::Unavailable(
            "replay metadata exceeds the size limit".to_owned(),
        ));
    }
    Ok(serde_json::from_slice::<Value>(&data)
        .ok()
        .filter(Value::is_object))
}

fn read_round(
    parquet: &Path,
    meta: &Value,
    key: &str,
    request: &ReplayRequest,
) -> Result<Option<Vec<u8>>, ReplayCacheError> {
    let start = request.start_tick;
    let end = request.end_tick;
    let Some(spec) = entries(&meta["rounds"]).iter().find(|row| {
        row.is_object() && integer(&row["start_tick"]) == start && integer(&row["end_tick"]) == end
    }) else {
        return Ok(None);
    };
    let row_group = usize::try_from(integer(&spec["row_group"]))
        .map_err(|_| ReplayCacheError::Unavailable("invalid replay row group".to_owned()))?;
    let ticks = entries(&spec["sample_ticks"]);
    if ticks.is_empty() || ticks.len() > MAX_SAMPLE_TICKS {
        return Err(ReplayCacheError::Unavailable(
            "invalid replay sample tick count".to_owned(),
        ));
    }
    let ticks = ticks
        .iter()
        .map(|value| i32::try_from(integer(value)))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ReplayCacheError::Unavailable("invalid replay sample tick".to_owned()))?;

    let shots = entries(&spec["shots"])
        .iter()
        .filter(|value| value.is_object())
        .collect::<Vec<_>>();
    let events = entries(&spec["events"])
        .iter()
        .filter(|value| value.is_object())
        .collect::<Vec<_>>();
    let effect_tracks = entries(&meta["effect_tracks"])
        .iter()
        .filter(|track| {
            track.is_object()
                && integer(&track["end_tick"]) >= start
                && integer(&track["start_tick"]) <= end
        })
        .collect::<Vec<_>>();
    let version = integer(&meta["effect_tracks_version"]);
    let capabilities = &meta["effect_capabilities"];
    let metadata = json!({
        "round_number": integer(&spec["round_number"]),
        "start_tick": start,
        "end_tick": end,
        "tick_rate": number_or(&meta["tick_rate"], request.tick_rate)?,
        "fps": number_or(&meta["fps"], request.fps)?,
        "map_transform": meta["map_transform"],
        "pov_player_name": meta["pov_player_name"],
        "pov_steamid64": meta["pov_steamid64"],
        "shots": shots,
        "events": events,
        "effect_tracks_version": if version == 0 { 1 } else { version },
        "effect_capabilities": if capabilities.is_null() { json!({}) } else { capabilities.clone() },
        "effect_warnings": entries(&meta["effect_warnings"]),
        "effect_tracks": effect_tracks,
        "effects_pending": false,
        "demo_fingerprint": meta["demo_fingerprint"],
        "cache_key": key,
        "cache": {"frames": "parquet_binary_hit", "effects": "parquet_hit", "parsed": false},
    });

    replay_parquet_binary_packet(parquet, row_group, &ticks, metadata)
        .map(Some)
        .map_err(ReplayCacheError::Unavailable)
}

fn entries(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or_default()
}

fn integer(value: &Value) -> i64 {
    match value {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value as i64))
            .unwrap_or_default(),
        Value::String(value) => value.trim().parse().unwrap_or_default(),
        Value::Bool(value) => i64::from(*value),
        _ => 0,
    }
}

fn number_or(value: &Value, fallback: f64) -> Result<f64, ReplayCacheError> {
    let number = match value {
        Value::Null => Some(fallback),
        Value::Number(value) => value.as_f64(),
        Value::String(value) if value.is_empty() => Some(fallback),
        Value::String(value) => value.trim().parse().ok(),
        Value::Bool(value) => Some(f64::from(u8::from(*value))),
        _ => None,
    };
    match number {
        Some(0.0) => Ok(fallback),
        Some(value) if value.is_finite() => Ok(value),
        _ => Err(ReplayCacheError::Unavailable(
            "invalid replay cache rate".to_owned(),
        )),
    }
}
