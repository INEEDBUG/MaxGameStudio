use super::{read_library, select_objects, DemoLibrary, DemoResult, PLAYER_STATS_COLUMNS};
use crate::profile_platform::path_text;
use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

pub(crate) enum RosterCache {
    Missing,
    Ready(Vec<Value>),
    Error(String),
}

pub(crate) struct RosterSource {
    pub path: PathBuf,
    pub match_meta: Value,
    pub cache: RosterCache,
}

impl DemoLibrary {
    pub(crate) fn roster(&self, id: i64) -> DemoResult<Option<RosterSource>> {
        read_library(&self.path, |connection| {
            let Some(connection) = connection else {
                return Ok(None);
            };
            let mut demos = select_objects(
                connection,
                "SELECT path, content_md5, map_name, total_rounds, team_a_score,
                    team_b_score, duration_mins, match_date FROM demo_files WHERE id = ?",
                [id],
            )?;
            let Some(mut demo) = demos.pop() else {
                return Ok(None);
            };
            let path = PathBuf::from(
                demo.remove("path")
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .unwrap_or_default(),
            );
            let md5 = demo.remove("content_md5").unwrap_or(Value::Null);
            let cache = read_cache(connection, id, &path, md5.as_str())?;
            Ok(Some(RosterSource {
                path,
                match_meta: Value::Object(demo),
                cache,
            }))
        })
    }
}

fn read_cache(
    connection: &Connection,
    id: i64,
    path: &Path,
    md5: Option<&str>,
) -> DemoResult<RosterCache> {
    let has_table: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'demo_roster_cache')",
        [],
        |row| row.get(0),
    )?;
    if !has_table {
        return Ok(RosterCache::Missing);
    }
    let cache = connection.query_row(
        "SELECT demo_path, source_content_md5, source_file_size, source_mtime_ns, state, row_count, error_msg
         FROM demo_roster_cache WHERE demo_id = ? AND cache_version = 1",
        [id],
        |row| Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<i64>>(2)?,
            row.get::<_, Option<i64>>(3)?,
            row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
            row.get::<_, Option<String>>(6)?,
        )),
    ).optional()?;
    let Some((cached_path, hash, size, mtime, state, count, error)) = cache else {
        return Ok(RosterCache::Missing);
    };
    let source = source_stat(path);
    if (size, mtime) != (source.map(|(size, _)| size), source.map(|(_, mtime)| mtime))
        || !path_matches(path, Path::new(&cached_path))
        || !hash
            .as_deref()
            .unwrap_or_default()
            .trim()
            .eq_ignore_ascii_case(md5.unwrap_or_default().trim())
    {
        return Ok(RosterCache::Missing);
    }
    match (state.as_str(), count) {
        ("empty", 0) => Ok(RosterCache::Ready(Vec::new())),
        ("error", 0) => Ok(RosterCache::Error(
            error
                .filter(|message| !message.is_empty())
                .unwrap_or_else(|| "Demo 玩家名单解析失败".into()),
        )),
        ("ready", count) if count > 0 => {
            let rows = select_objects(
                connection,
                &format!(
                    "SELECT {PLAYER_STATS_COLUMNS} FROM demo_player_stats
                    WHERE demo_id = ? ORDER BY kills DESC, player_name ASC"
                ),
                [id],
            )?;
            let players = normalize_players(rows.into_iter().map(Value::Object));
            if i64::try_from(players.len()) == Ok(count) {
                Ok(RosterCache::Ready(players))
            } else {
                Ok(RosterCache::Missing)
            }
        }
        _ => Ok(RosterCache::Missing),
    }
}

fn source_stat(path: &Path) -> Option<(i64, i64)> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let modified = metadata.modified().ok()?;
    let mtime = match modified.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos() as i128,
        Err(error) => -(error.duration().as_nanos() as i128),
    };
    Some((metadata.len().try_into().ok()?, mtime.try_into().ok()?))
}

fn path_matches(path: &Path, cached: &Path) -> bool {
    let (Ok(path), Ok(cached)) = (std::path::absolute(path), std::path::absolute(cached)) else {
        return false;
    };
    if cfg!(windows) {
        path_text(&path).replace('/', "\\").to_lowercase()
            == path_text(&cached).replace('/', "\\").to_lowercase()
    } else {
        path == cached
    }
}

pub(crate) fn normalize_players(rows: impl IntoIterator<Item = Value>) -> Vec<Value> {
    rows.into_iter().filter_map(|row| {
        let name = text(&row["name"]).or_else(|| text(&row["player_name"]))?;
        let team = row["team"].as_i64().or_else(|| row["team_number"].as_i64()).unwrap_or(0);
        let steam = text(&row["steam_id"]);
        let steam_id64 = text(&row["steam_id64"]).or_else(|| {
            steam.filter(|id| id.len() >= 15 && id.bytes().all(|byte| byte.is_ascii_digit()))
        });
        let account = text(&row["account_id"]).map(str::to_owned).or_else(|| {
            steam_id64?.parse::<u64>().ok()?.checked_sub(76561197960265728).map(|id| id.to_string())
        });
        let user = text(&row["user_id"]).map(str::to_owned)
            .or_else(|| row["user_id"].as_i64().map(|id| id.to_string()));
        let (kills, deaths, assists) = (
            row["kills"].as_i64().unwrap_or(0),
            row["deaths"].as_i64().unwrap_or(0),
            row["assists"].as_i64().unwrap_or(0),
        );
        let kd = row["kd"].as_f64().unwrap_or(kills as f64 / deaths.max(1) as f64);
        Some(json!({
            "name": name, "player_name": name, "team": team, "team_number": team,
            "team_name": text(&row["team_name"]), "steam_id": steam.or(steam_id64),
            "steam_id64": steam_id64, "steamid64": steam_id64, "account_id": account, "user_id": user,
            "kills": kills, "deaths": deaths, "assists": assists, "kd": (kd * 1000.0).round_ties_even() / 1000.0,
        }))
    }).collect()
}

fn text(value: &Value) -> Option<&str> {
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}
