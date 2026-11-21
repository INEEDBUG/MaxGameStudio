mod compilations;
mod event_clips;
#[cfg(windows)]
mod execute_single;
#[cfg(windows)]
mod game_window;
#[cfg(windows)]
mod obs_capture;
#[cfg(windows)]
mod playback_gsi;
#[cfg(windows)]
mod playback_process;
#[cfg(windows)]
mod playback_spec;
#[cfg(windows)]
mod playback_stage;
mod postprocess;
mod pov_interleave;
#[cfg(windows)]
pub(super) mod queue;
mod request;
mod rounds;

use super::{detail, json_reply, model_body, HttpResponse, ProfileState};
use crate::demo_boundary;
use hyper::{body::Incoming, Request, StatusCode};
use request::{
    NormalizedRequest, Perspective, RecordingRequestDto, RequestType, SourceType, TargetPlayer,
};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::{
    fs::{self, OpenOptions},
    io::{self, BufReader},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

#[derive(Serialize)]
struct RecordingSegment {
    segment_index: usize,
    source_type: SourceType,
    start_tick: i64,
    end_tick: i64,
    anchor_ticks: Vec<i64>,
    round: Option<i64>,
    target_player_name: String,
    target_steamid64: String,
    target_spec_slot: Option<i64>,
    perspective: Perspective,
    is_final_round: bool,
    safe_seek_tick: i64,
    safe_end_tick: Option<i64>,
    disabled: bool,
    disabled_reason: Option<String>,
    metadata: Map<String, Value>,
    voice_listen_mask: Option<u64>,
    voice_listen_mask_enemy: Option<u64>,
}

struct RecordingPlan {
    request: NormalizedRequest,
    active: Vec<RecordingSegment>,
    disabled: Vec<RecordingSegment>,
}

pub(super) async fn respond(
    profile: &ProfileState,
    request: Request<Incoming>,
    extended_request: &AtomicBool,
) -> HttpResponse {
    let body = match model_body(request, "Recording plan request is too large").await {
        Ok(body) => body,
        Err(error) => return *error,
    };
    let dto = match request::decode(body) {
        Ok(dto) => dto,
        Err(error) => return detail(StatusCode::UNPROCESSABLE_ENTITY, &error),
    };
    let Ok(permit) = Arc::clone(&profile.demo_parser).try_acquire_owned() else {
        return detail(
            StatusCode::SERVICE_UNAVAILABLE,
            "正在处理其他 Demo，请稍后重试",
        );
    };
    extended_request.store(true, Ordering::Relaxed);
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        build(dto).map(RecordingPlan::preview)
    })
    .await
    {
        Ok(Ok(value)) => json_reply(StatusCode::OK, value),
        Ok(Err((status, message))) => detail(status, &message),
        Err(_) => detail(StatusCode::INTERNAL_SERVER_ERROR, "无法生成录制计划"),
    }
}

fn build(dto: RecordingRequestDto) -> Result<RecordingPlan, (StatusCode, String)> {
    let end = actual_end_tick(Path::new(&dto.demo.demo_path)).map_err(|_| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            "无法从 Demo 文件读取有效的结束 tick".to_owned(),
        )
    })?;
    let mut req =
        request::normalize(dto, end).map_err(|error| (StatusCode::UNPROCESSABLE_ENTITY, error))?;
    if req.options.use_ai_director
        && matches!(
            req.request_type,
            RequestType::Highlight | RequestType::KillCompilation
        )
    {
        return Err((
            StatusCode::NOT_IMPLEMENTED,
            "Rust 版 AI 导播计划尚未接入".to_owned(),
        ));
    }
    let (segments, warnings) = match req.request_type {
        RequestType::RoundCompilation | RequestType::TimelineRound => rounds::plan(&req),
        RequestType::Highlight
        | RequestType::Fail
        | RequestType::TimelineKill
        | RequestType::TimelineDeath => (event_clips::plan(&req), Vec::new()),
        RequestType::KillCompilation | RequestType::DeathCompilation => {
            (compilations::plan(&req), Vec::new())
        }
    };
    req.warnings.extend(warnings);
    let (active, disabled, guard_warnings) = postprocess::finish(&req, segments);
    req.warnings.extend(guard_warnings);
    Ok(RecordingPlan {
        request: req,
        active,
        disabled,
    })
}

impl RecordingPlan {
    fn preview(self) -> Value {
        let Self {
            request: req,
            active,
            disabled,
        } = self;
        let estimates = active
            .iter()
            .map(|segment| i128::from(segment.end_tick) - i128::from(segment.start_tick))
            .sum::<i128>();
        json!({
        "request_id":req.request_id, "request_type":req.request_type, "demo_path":req.demo.demo_path,
        "tick_rate":req.demo.tick_rate, "estimated_duration_sec":estimates as f64 / req.demo.tick_rate,
        "summary":{
            "active_count":active.len(), "disabled_count":disabled.len(),
            "final_round_segments":active.iter().filter(|segment| segment.is_final_round).count(),
            "victim_segments_disabled_no_steamid":disabled.iter().filter(|segment|segment.disabled_reason.as_deref()==Some("missing_victim_steamid64")).count(),
            "demo_end_guard_warnings":req.warnings.iter().filter(|warning|warning.contains("demo_end")).count(),
        },
        "warnings":req.warnings,
        "active_segments":active.into_iter().map(segment_debug).collect::<Vec<_>>(),
        "disabled_segments":disabled.into_iter().map(segment_debug).collect::<Vec<_>>(),
        })
    }
}

fn event_segment(
    req: &NormalizedRequest,
    source_type: SourceType,
    perspective: Perspective,
    round: i64,
    target: &TargetPlayer,
    (start, end): (i64, i64),
    anchor_ticks: Vec<i64>,
) -> RecordingSegment {
    let start_tick = start.max(req.demo.first_tick);
    let end_tick = end.min(req.demo.demo_end_tick);
    let preroll = (5.0 * req.demo.tick_rate) as i64;
    RecordingSegment {
        segment_index: 0,
        source_type,
        start_tick,
        end_tick,
        anchor_ticks,
        round: Some(round),
        target_player_name: target.name.clone(),
        target_steamid64: target.steamid64.clone(),
        target_spec_slot: target.spec_slot,
        perspective,
        is_final_round: round == req.demo.final_round,
        safe_seek_tick: start_tick.saturating_sub(preroll).max(req.demo.first_tick),
        safe_end_tick: None,
        disabled: false,
        disabled_reason: None,
        metadata: Map::new(),
        voice_listen_mask: None,
        voice_listen_mask_enemy: None,
    }
}

fn segment_debug(segment: RecordingSegment) -> Value {
    let mut record = serde_json::to_value(&segment).expect("JSON recording segment");
    let object = record.as_object_mut().expect("recording segment object");
    object.remove("target_spec_slot");
    object.remove("voice_listen_mask");
    object.remove("voice_listen_mask_enemy");
    object.remove("metadata");
    let mut meta = segment.metadata;
    meta.insert(
        "demo_end_guard_applied".to_owned(),
        json!(segment.safe_end_tick.is_some()),
    );
    meta.insert(
        "target_steamid64_missing".to_owned(),
        json!(segment.target_steamid64.is_empty()),
    );
    object.extend(meta);
    record
}

fn actual_end_tick(path: &Path) -> io::Result<Option<i64>> {
    match fs::metadata(path) {
        Ok(info) if info.is_file() => {}
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Keep the exact file length and frame headers stable for the scan.
        options.share_mode(1);
    }
    let file = options.open(path)?;
    let length = file.metadata()?.len();
    demo_boundary::end_tick(BufReader::new(file), length).map(Some)
}
