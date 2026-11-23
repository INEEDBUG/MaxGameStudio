//! Analyze a selected Demo using the bundled native parser.

mod player_timeline;

use super::{
    demo_query, demo_read_error, detail, json_reply, model_body, profile_io_error, profile_work,
    validation_reply, HttpResponse, ProfileState,
};
use crate::{
    demo_library::{AnalysisSource, DemoLibrary},
    profile_platform::path_text,
    replay_cache::{analysis_map_transform, resolve_demo, ReplayCacheError},
};
use hyper::{body::Incoming, Request, StatusCode};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::{
    collections::HashSet,
    fs::{File, OpenOptions},
    io,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

#[derive(Deserialize)]
struct AnalysisRequest {
    target_players: Vec<String>,
    freeze_to_death_rounds: Option<Vec<i64>>,
}

pub(super) async fn respond(
    profile: &Arc<ProfileState>,
    request: Request<Incoming>,
    from_library: bool,
    extended_request: &AtomicBool,
) -> HttpResponse {
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or_default().to_owned();
    let body = match model_body(request, "Demo 分析请求过大").await {
        Ok(body) => body,
        Err(error) => return *error,
    };
    let Ok(mut input) = serde_json::from_value::<AnalysisRequest>(body) else {
        return detail(StatusCode::UNPROCESSABLE_ENTITY, "Demo 分析参数无效");
    };
    let mut seen = HashSet::new();
    for name in &mut input.target_players {
        *name = name.trim().to_owned();
    }
    input
        .target_players
        .retain(|name| !name.is_empty() && seen.insert(name.clone()));
    if input.target_players.is_empty() {
        return detail(StatusCode::BAD_REQUEST, "请选择需要分析的玩家");
    }
    extended_request.store(true, Ordering::Relaxed);
    let source = if from_library {
        let id = match demo_query::parse_id(&path) {
            Ok(id) => id,
            Err(errors) => return validation_reply(errors),
        };
        match profile_work(profile, move |profile| {
            DemoLibrary::new(profile.config.database_path()).analysis_source(id)
        })
        .await
        {
            Ok(Ok(Some(source))) => Some(source),
            Ok(Ok(None)) => return detail(StatusCode::NOT_FOUND, "Demo 库中没有这条记录"),
            Ok(Err(error)) => return demo_read_error(&error),
            Err(error) => return profile_io_error(&error),
        }
    } else {
        None
    };
    let resolved = match source.as_ref() {
        Some(row) => Path::new(&row.path)
            .canonicalize()
            .ok()
            .filter(|path| path.is_file())
            .ok_or_else(|| ReplayCacheError::MissingDemo(row.path.clone())),
        None => resolve_demo(&profile.upload_dir, &submitted_path(&query)),
    };
    let demo = match resolved {
        Ok(demo) => demo,
        Err(ReplayCacheError::EmptyPath) => {
            return detail(StatusCode::BAD_REQUEST, "Demo 路径为空")
        }
        Err(_) => return demo_problem(StatusCode::NOT_FOUND, "DEMO_FILE_NOT_FOUND"),
    };
    let Some(worker) = profile.parser_worker.as_ref() else {
        return detail(
            StatusCode::SERVICE_UNAVAILABLE,
            "未找到随应用安装的 Demo 解析器",
        );
    };
    let Ok(_permit) = Arc::clone(&profile.demo_parser).try_acquire_owned() else {
        return detail(
            StatusCode::SERVICE_UNAVAILABLE,
            "正在处理其他 Demo，请稍后重试",
        );
    };
    // Hold the source open through analysis and persistence. On Windows this
    // also prevents replacement or concurrent writes while the worker runs.
    let Ok(source_file) = stable_source(&demo) else {
        return detail(StatusCode::CONFLICT, "Demo 正在写入或无法读取，请稍后重试");
    };
    let value = match worker
        .analyze(
            &demo,
            &input.target_players,
            input.freeze_to_death_rounds.as_deref(),
        )
        .await
    {
        Ok(value) => value,
        Err(_) => return demo_problem(StatusCode::INTERNAL_SERVER_ERROR, "DEMO_ANALYSIS_FAILED"),
    };
    let Some(mut result) = assemble(value, &demo, source.as_ref(), &input.target_players) else {
        return demo_problem(StatusCode::INTERNAL_SERVER_ERROR, "DEMO_ANALYSIS_EMPTY");
    };
    if let Some(source) = source {
        let saved = profile_work(profile, move |profile| {
            let _source_file = source_file;
            let library = DemoLibrary::new(profile.config.database_path());
            library
                .save_analysis(&source, &result)
                .map(|saved| (saved, result, source.filename))
        })
        .await;
        return match saved {
            Ok(Ok((true, mut result, filename))) => {
                profile.demo_events.changed();
                json_reply(
                    StatusCode::OK,
                    json!({
                        "players":result["players"].take(),
                        "analysis_workspace":result["analysis_workspace"].take(),
                        "demo_filename":filename,
                    }),
                )
            }
            Ok(Ok((false, _, _))) => detail(StatusCode::CONFLICT, "Demo 库记录已变化，请重新打开"),
            Ok(Err(error)) => demo_read_error(&error),
            Err(error) => profile_io_error(&error),
        };
    }
    json_reply(
        StatusCode::OK,
        json!({
            "players":result["players"].take(),
            "analysis_workspace":result["analysis_workspace"].take(),
        }),
    )
}

fn submitted_path(query: &str) -> String {
    let query =
        form_urlencoded::parse(query.as_bytes()).collect::<std::collections::HashMap<_, _>>();
    query
        .get("path")
        .filter(|path| !path.trim().is_empty())
        .or_else(|| query.get("filename"))
        .map_or_else(String::new, |value| value.to_string())
}

pub(super) fn stable_source(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;
        options.share_mode(FILE_SHARE_READ);
    }
    options.open(path)
}

fn assemble(
    mut native: Value,
    demo: &Path,
    source: Option<&AnalysisSource>,
    requested: &[String],
) -> Option<Value> {
    let path = path_text(demo);
    let mut facts = native["match_facts"].take();
    let mut metadata = facts["match_meta"].take();
    let map = metadata["map_name"].as_str()?.to_owned();
    let match_date = source
        .and_then(|source| source.match_date.as_deref())
        .unwrap_or_default();
    metadata["match_date"] = json!(match_date);
    let roster = facts["players"].take();
    let roster = roster.as_array()?;
    let mut clips = facts["round_compilations"].take();
    let stats = facts["player_stats"].take();
    let stats = stats.as_array()?;
    let slots = facts["spec_slots"].as_object()?;
    let deaths = facts["recording_timeline"]["deaths"].as_array()?;
    let round_records = facts["recording_timeline"]["rounds"].as_array()?;
    let player_slot = |name: &str| slots.get(&name.to_lowercase()).and_then(Value::as_i64);
    let initial_side = |key: &str| -> Option<&'static str> {
        match key {
            "a" => Some("T"),
            "b" => Some("CT"),
            _ => None,
        }
    };
    let all_players = roster
        .iter()
        .filter_map(|player| {
            let name = player["name"].as_str()?;
            let info = stats
                .iter()
                .find(|stats| stats["steam_id64"] == player["steam_id64"])?;
            let key = player["team_key"].as_str().unwrap_or_default();
            let team_name = if key == "a" {
                "team_a_name"
            } else {
                "team_b_name"
            };
            Some(json!({
                "name":name,"steam_id64":player["steam_id64"],"steamid64":player["steam_id64"],
                "user_id":player_slot(name),
                "team":initial_side(key).map(|side| if side=="T" {2} else {3}),
                "team_name":metadata[team_name],
                "kills":info["kills"],"deaths":info["deaths"],"assists":info["assists"],
            }))
        })
        .collect::<Vec<_>>();
    let mut players = Map::new();
    let mut analyzed = Vec::new();
    let mut total = metadata["total_rounds"]
        .as_u64()
        .and_then(|rounds| usize::try_from(rounds).ok())
        .unwrap_or_default();
    if total == 0 {
        total = round_records.len();
    }
    for name in requested {
        let Some(player) = roster.iter().find(|row| row["name"].as_str() == Some(name)) else {
            continue;
        };
        let Some(performance) = stats
            .iter()
            .find(|row| row["steam_id64"] == player["steam_id64"])
        else {
            continue;
        };
        let key = player["team_key"].as_str().unwrap_or_default();
        let side_key = if key == "a" { "side_a" } else { "side_b" };
        let target_rounds = round_records
            .iter()
            .map(|round| {
                let mut round = round.clone();
                round["side"] = round[side_key].take();
                round
            })
            .collect::<Vec<_>>();
        let mut player_meta = metadata.clone();
        player_meta["target_player"] = json!(name);
        player_meta["target_player_user_id"] = json!(player_slot(name));
        player_meta["target_steam_id"] = player["steam_id64"].clone();
        player_meta["target_kills"] = performance["kills"].clone();
        player_meta["target_deaths"] = performance["deaths"].clone();
        player_meta["all_players"] = json!(all_players);
        player_meta["meme_series_badges"] = json!([]);
        let list = clips[name].take();
        let items = list.as_array()?;
        let mut timeline = player_timeline::project(player_timeline::Input {
            demo_path: &path,
            map_name: Some(&map),
            target_name: name,
            target_steamid64: player["steam_id64"].as_str(),
            target_spec_slot: player_slot(name),
            initial_side: initial_side(key),
            tick_rate: 64.0,
            total_rounds: total,
            deaths,
            rounds: &target_rounds,
            clips: items,
            spec_slots: slots,
        });
        players.insert(
            name.clone(),
            json!({
                "match_meta":player_meta,"clips":list,"timeline":timeline["timeline"].take(),
                "round_timeline":timeline["round_timeline"].take(),
            }),
        );
        analyzed.push(name.as_str());
    }
    let primary = *analyzed.first()?;
    let first = players.get(primary)?;
    let rounds = native["rounds"].take();
    let count = rounds.as_array()?.len();
    let workspace = json!({
        "version":1,"algorithm_version":"match-workspace-2026.11.20",
        "data_source":"demo_parser_with_derived_metrics","team_assignment_source":facts["team_assignment_source"],
        "derived_fields":["kast","trade_kills","trade_deaths","economy_type","clutch_attempts","clutch_wins","special_events","phase_meta"],
        "map_name":map,"tick_rate":64.0,"match_start_tick":native["match_start_tick"],"demo_end_tick":native["demo_end_tick"],
        "duration_mins":metadata["duration_mins"],"match_date":match_date,
        "team_a_name":metadata["team_a_name"],"team_b_name":metadata["team_b_name"],
        "team_a_score":metadata["team_a_score"],"team_b_score":metadata["team_b_score"],
        "team_a_group":facts["team_a_group"],"team_b_group":facts["team_b_group"],
        "map_transform":analysis_map_transform(&map),"phase_meta":facts["phase_meta"],
        "players":stats,"rounds":rounds,
        "effect_tracks_version":1,"effect_capabilities":{"inferno_cells":false,"smoke_voxels":false,"smoke_mode":"legacy_circle","source":"replay_effects_api"},
        "effect_tracks":[],"effect_warnings":[],"summary":{"total_rounds":count},
    });
    Some(json!({
        "analysis_workspace":workspace,"auto_target_player":primary,"analyzed_target_players":analyzed,
        "clips":first["clips"],"match_meta":first["match_meta"],"timeline":first["timeline"],
        "round_timeline":first["round_timeline"],"players":players,
    }))
}

fn demo_problem(status: StatusCode, code: &str) -> HttpResponse {
    json_reply(status, json!({"detail":{"code":code}}))
}
