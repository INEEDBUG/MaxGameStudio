//! Look up the shipped whole-match replay cache without creating or rebuilding it.

use crate::{
    profile_platform::path_text,
    replay_packet::{replay_parquet_binary_packet, MAX_SAMPLE_TICKS},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

const REPLAY_MATCH_CACHE_VERSION: u64 = 5;
pub(crate) const REQUIRED_PARSER_VERSION: &str = "0.41.4+cs2insight7";
const METADATA_LIMIT: u64 = 16 * 1024 * 1024;
const CURRENT_CACHE: &str = "cache/demo-replay/matches";
const LEGACY_CACHE: &str = "replay-match";

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
