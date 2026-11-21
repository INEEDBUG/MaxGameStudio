#[path = "cold_tracks.rs"]
mod tracks;

use super::{
    cache_key, cache_key_for, entries, fingerprint, integer, number_or, read_metadata, read_round,
    repair_error, resolve_demo, same_path, Fingerprint, ReplayCacheError, ReplayRequest,
    CURRENT_CACHE, METADATA_LIMIT, REPLAY_MATCH_CACHE_VERSION, REQUIRED_PARSER_VERSION,
};
use crate::{demo_boundary, demo_library::DemoLibrary, parser_worker::ParserWorker};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufReader, Write},
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

const MAP_DATA: &str = include_str!("../../../../backend/assets/bundled_radar_maps/map-data.json");

pub(crate) struct ColdReplay {
    demo: PathBuf,
    source: Fingerprint,
    workspace: Value,
    rounds: Vec<Round>,
    request: ReplayRequest,
    cache: PathBuf,
    stage: tempfile::TempDir,
    key: String,
    source_end: i64,
}

struct Round {
    number: i64,
    start: i64,
    end: i64,
    team_a: String,
    team_b: String,
    shots: Vec<Value>,
    events: Vec<Value>,
    releases: Vec<Value>,
}

impl ColdReplay {
    pub(crate) fn prepare(
        data: &Path,
        uploads: &Path,
        database: PathBuf,
        request: ReplayRequest,
    ) -> Result<Option<Self>, ReplayCacheError> {
        let demo = resolve_demo(uploads, &request.path)?;
        let Some(source) = fingerprint(&demo) else {
            return Ok(None);
        };
        let Some(mut workspace) = DemoLibrary::new(database)
            .replay_workspace(&demo)
            .map_err(|error| unavailable(error.to_string()))?
        else {
            return Ok(None);
        };
        let tick_rate = number_or(&workspace["tick_rate"], 64.0)?.max(0.001);
        if (tick_rate - request.tick_rate).abs() > 0.000_000_5 {
            return Ok(None);
        }
        let rounds = clean_rounds(&mut workspace, tick_rate);
        if !rounds
            .iter()
            .any(|round| round.start == request.start_tick && round.end == request.end_tick)
        {
            return Ok(None);
        }
        let file = File::open(&demo).map_err(repair_error)?;
        let source_end =
            demo_boundary::end_tick(BufReader::new(file), source.size).map_err(repair_error)?;
        if rounds.iter().any(|round| round.end > source_end) {
            return Err(unavailable("Stored replay rounds extend past the Demo"));
        }
        fs::create_dir_all(data).map_err(repair_error)?;
        let root = data.canonicalize().map_err(repair_error)?;
        let mut cache = root;
        for part in CURRENT_CACHE.split('/') {
            cache.push(part);
            match fs::create_dir(&cache) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(repair_error(error)),
            }
            if !same_path(&cache.canonicalize().map_err(repair_error)?, &cache) {
                return Err(unavailable(
                    "Replay cache directory is linked outside the selected profile",
                ));
            }
        }
        let key = cache_key_for(&source, request.fps, request.tick_rate);
        let meta_file = cache.join(format!("{key}.meta.json"));
        if meta_file.try_exists().map_err(repair_error)? {
            // Reusing complete saved metadata is the frame-only repair path.
            return Ok(None);
        }
        let stage = tempfile::Builder::new()
            .prefix("mgs-replay-")
            .tempdir_in(&cache)
            .map_err(repair_error)?;
        Ok(Some(Self {
            demo,
            source,
            workspace,
            rounds,
            request,
            cache,
            stage,
            key,
            source_end,
        }))
    }

    pub(crate) async fn generate(
        &mut self,
        worker: &ParserWorker,
    ) -> Result<Value, ReplayCacheError> {
        let rate = self.request.tick_rate;
        let windows = self
            .rounds
            .iter()
            .map(|round| {
                json!({
                    "round_number":round.number,"start_tick":round.start,"end_tick":round.end,
                })
            })
            .collect::<Vec<_>>();
        let written = worker
            .write_replay(
                &self.demo,
                &self.staged_parquet(),
                rate,
                self.request.fps,
                &json!(windows),
            )
            .await
            .map_err(repair_error)?;
        let effect_started = Instant::now();
        let broad_start = self
            .rounds
            .iter()
            .map(|round| round.start)
            .min()
            .expect("checked replay rounds");
        let broad_end = self
            .rounds
            .iter()
            .map(|round| round.end)
            .max()
            .expect("checked replay rounds");
        let effects_enabled =
            !std::env::var("CS2_INSIGHT_DYNAMIC_UTILITY_EFFECTS").is_ok_and(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "0" | "false" | "no" | "off"
                )
            });
        let mut effects = if effects_enabled {
            worker
                .replay_effects(&self.demo, rate, self.source_end)
                .await
                .map_err(repair_error)?
        } else {
            json!({"effect_tracks":[],"warnings":["dynamic utility effects disabled by CS2_INSIGHT_DYNAMIC_UTILITY_EFFECTS"],
                "capabilities":{"inferno_cells":false,"smoke_voxels":false,"smoke_mode":"legacy_circle"}})
        };
        let Value::Array(effect_tracks) = effects["effect_tracks"].take() else {
            return Err(unavailable("Incomplete native utility effect reply"));
        };
        let mut effect_tracks = tracks::clip_effects(effect_tracks, broad_start, broad_end);
        tracks::tag_effect_sides(&mut effect_tracks, &self.rounds, &self.workspace, rate);
        let effect_parse_ms = if effects_enabled {
            (effect_started.elapsed().as_secs_f64() * 100_000.0).round_ties_even() / 100.0
        } else {
            0.0
        };
        let mut projectiles = worker
            .replay_trajectories(&self.demo, rate, self.source_end)
            .await
            .map_err(repair_error)?;
        let Value::Array(projectiles) = projectiles["trajectories"].take() else {
            return Err(unavailable("Incomplete native projectile reply"));
        };
        tracks::attach_projectiles(&mut self.rounds, &projectiles, rate);
        self.metadata(written, effects, effect_tracks, effect_parse_ms)
    }

    fn metadata(
        &mut self,
        written: Value,
        mut effects: Value,
        effect_tracks: Vec<Value>,
        effect_parse_ms: f64,
    ) -> Result<Value, ReplayCacheError> {
        let expected = entries(&written["rounds"]);
        let groups = entries(&written["row_groups"]);
        if expected.len() != self.rounds.len() || groups.len() != self.rounds.len() {
            return Err(unavailable("Parser worker omitted replay rounds"));
        }
        let rounds = self.rounds.iter_mut().zip(expected.iter().zip(groups)).map(|(round, (samples, group))| {
            if integer(&samples["round_number"]) != round.number
                || integer(&samples["start_tick"]) != round.start
                || integer(&samples["end_tick"]) != round.end
                || integer(&group["round_number"]) != round.number
                || group["row_group"].as_u64().is_none()
                || group["rows"].as_u64().is_none_or(|count| count == 0)
                || entries(&samples["sample_ticks"]).is_empty()
            {
                return Err(unavailable("Parser worker returned inconsistent replay frames"));
            }
            Ok(json!({
                "round_number":round.number, "start_tick":round.start, "end_tick":round.end,
                "row_group":group["row_group"], "frame_count":entries(&samples["sample_ticks"]).len(),
                "player_rows":group["rows"],"sample_ticks":samples["sample_ticks"],
                "shots":std::mem::take(&mut round.shots), "events":std::mem::take(&mut round.events),
            }))
        }).collect::<Result<Vec<_>, _>>()?;
        let player = entries(&self.workspace["players"])
            .iter()
            .find(|player| player.is_object());
        let first_name = player.and_then(|player| player["name"].as_str());
        let first_steam = player
            .and_then(|player| text(&player["steam_id64"]).or_else(|| text(&player["steamid64"])));
        let map = text(&self.workspace["map_name"]).unwrap_or("unknown");
        let map_transform =
            serde_json::from_str::<Value>(MAP_DATA).expect("bundled radar calibration");
        let geo = &map_transform[map.trim().to_lowercase()];
        let mut capabilities = effects["capabilities"].take();
        if !capabilities.is_object() || !effects["warnings"].is_array() {
            return Err(unavailable("Incomplete native utility effect metadata"));
        }
        capabilities["inferno_cells"] =
            json!(effect_tracks.iter().any(|track| track["type"] == "inferno"));
        capabilities["smoke_voxels"] =
            json!(effect_tracks.iter().any(|track| track["type"] == "smoke"));
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        Ok(json!({
            "version":REPLAY_MATCH_CACHE_VERSION, "parser_runtime":REQUIRED_PARSER_VERSION,
            "cache_key":self.key, "created_at":created_at, "demo_fingerprint":self.source,
            "map_name":map,"map_transform":geo,"tick_rate":self.request.tick_rate,"fps":self.request.fps,
            "pov_player_name":first_name,"pov_steamid64":first_steam,"rounds":rounds,
            "effect_tracks_version":1,"effect_capabilities":capabilities,"effect_tracks":effect_tracks,
            "effect_warnings":effects["warnings"].take(),"effect_parse_ms":effect_parse_ms,
        }))
    }

    fn staged_parquet(&self) -> PathBuf {
        self.stage.path().join("frames.parquet.partial")
    }

    pub(crate) fn finish(self, metadata: Value) -> Result<Option<Vec<u8>>, ReplayCacheError> {
        if cache_key(&self.demo, self.request.fps, self.request.tick_rate).as_deref()
            != Some(&self.key)
        {
            return Err(unavailable("Demo changed during replay generation"));
        }
        let serialized =
            serde_json::to_vec(&metadata).map_err(|error| unavailable(error.to_string()))?;
        if serialized.len() as u64 > METADATA_LIMIT {
            return Err(unavailable("Replay metadata exceeds the size limit"));
        }
        drop(metadata);
        // Serve the first request from the same JSON representation as a later cache hit.
        let packet_metadata =
            serde_json::from_slice(&serialized).map_err(|error| unavailable(error.to_string()))?;
        let staged = self.staged_parquet();
        let parquet = self.cache.join(format!("{}.parquet", self.key));
        let meta = self.cache.join(format!("{}.meta.json", self.key));
        let packet = read_round(&staged, &packet_metadata, &self.key, &self.request)?;
        if packet.is_none() {
            return Ok(None);
        }
        let mut header =
            tempfile::NamedTempFile::new_in(self.stage.path()).map_err(repair_error)?;
        header.write_all(&serialized).map_err(repair_error)?;
        header.as_file().sync_all().map_err(repair_error)?;
        let staged = tempfile::TempPath::try_from_path(&staged).map_err(repair_error)?;
        match staged.persist_noclobber(&parquet) {
            Ok(()) => {}
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                // A retry can finish publishing metadata for an identical orphaned Parquet.
                if file_digest(&error.path).map_err(repair_error)?
                    != file_digest(&parquet).map_err(repair_error)?
                {
                    return Err(unavailable(
                        "An existing replay cache has a different Parquet file",
                    ));
                }
            }
            Err(error) => return Err(repair_error(error.error)),
        }
        match header.persist_noclobber(&meta) {
            Ok(_) => Ok(packet),
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                if let Some(other) = read_metadata(&meta)?.filter(|meta| {
                    meta["cache_key"] == self.key && meta["version"] == REPLAY_MATCH_CACHE_VERSION
                }) {
                    return read_round(&parquet, &other, &self.key, &self.request);
                }
                Err(unavailable(
                    "An existing replay cache has different metadata",
                ))
            }
            Err(error) => Err(repair_error(error.error)),
        }
    }
}

fn clean_rounds(workspace: &mut Value, tick_rate: f64) -> Vec<Round> {
    let raw = workspace["rounds"].take();
    let mut raw = match raw {
        Value::Array(raw) => raw,
        _ => return Vec::new(),
    };
    let mut round_ends = std::collections::HashMap::new();
    let mut rounds = Vec::new();
    for raw in &mut raw {
        if !raw.is_object() {
            continue;
        }
        let number = integer(&raw["round_number"]);
        let start = nonzero(&raw["freeze_end_tick"]).unwrap_or_else(|| integer(&raw["start_tick"]));
        let end = nonzero(&raw["record_end_tick"])
            .or_else(|| nonzero(&raw["end_tick"]))
            .unwrap_or_else(|| integer(&raw["round_end_tick"]));
        let decision = nonzero(&raw["round_end_tick"]).unwrap_or_else(|| integer(&raw["end_tick"]));
        if number <= 0 || start < 0 || end <= start {
            continue;
        }
        round_ends.entry(number).or_insert(decision);
        rounds.push(Round {
            number,
            start,
            end,
            team_a: raw["team_a_side"]
                .as_str()
                .unwrap_or_default()
                .to_uppercase(),
            team_b: raw["team_b_side"]
                .as_str()
                .unwrap_or_default()
                .to_uppercase(),
            shots: take_objects(&mut raw["shots"]),
            events: take_objects(&mut raw["events"]),
            releases: take_objects(&mut raw["grenade_releases"]),
        });
    }
    rounds.sort_by_key(|round| (round.number, round.start));
    if let Some(last) = rounds.last_mut() {
        let decision = round_ends[&last.number];
        let demo_end = integer(&workspace["demo_end_tick"]);
        if demo_end > decision {
            let desired_end =
                decision.saturating_add(((tick_rate * 3.0).round_ties_even() as i64).max(1));
            last.end = last.end.max(desired_end.min(demo_end));
        }
    }
    rounds
}

fn nonzero(value: &Value) -> Option<i64> {
    let value = integer(value);
    (value != 0).then_some(value)
}

fn take_objects(value: &mut Value) -> Vec<Value> {
    match value.take() {
        Value::Array(mut items) => {
            items.retain(Value::is_object);
            items
        }
        _ => Vec::new(),
    }
}

fn text(value: &Value) -> Option<&str> {
    value.as_str().filter(|text| !text.is_empty())
}

fn file_digest(path: &Path) -> io::Result<[u8; 32]> {
    let mut digest = Sha256::new();
    io::copy(&mut OpenOptions::new().read(true).open(path)?, &mut digest)?;
    Ok(digest.finalize().into())
}

fn unavailable(message: impl Into<String>) -> ReplayCacheError {
    ReplayCacheError::Unavailable(message.into())
}
