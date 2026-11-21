//! Native execution of a previously planned, ordinary Demo clip.

use super::{
    build, execute_single,
    game_window::{Bindings, GameWindow},
    obs_capture::ObsCapture,
    playback_gsi::PlaybackGsi,
    playback_process::PlaybackProcess,
    playback_spec::{self, AspectRatio, GsiTarget, Resolution},
    RecordingPlan, RecordingRequestDto,
};
use crate::{
    native_http::{
        config_backup, detail, json_reply, lite_cut::ffmpeg, model_body, HttpResponse, ProfileState,
    },
    profile_platform::{path_text, probe_process},
    recorded_clips::capture::{insert_ready, local_output, CapturedClip, VerifiedOutput},
    runtime_session::{OperationOwner, RuntimeLease},
};
use hyper::{body::Incoming, Request, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    fmt::Write,
    io,
    num::NonZeroU16,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::watch;
use windows_sys::Win32::Security::Cryptography::{
    BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
};

#[derive(Default)]
pub(in crate::native_http) struct Recorder {
    state: Mutex<RecorderState>,
    gsi: PlaybackGsi,
}

#[derive(Default)]
struct RecorderState {
    closed: bool,
    active: Option<Arc<Signals>>,
    publishing: bool,
}

struct Signals {
    cancelled: Arc<AtomicBool>,
    cancel: watch::Sender<bool>,
    done: watch::Sender<bool>,
}

impl Signals {
    fn stop(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.cancel.send_replace(true);
    }

    fn check(&self) -> io::Result<()> {
        if self.cancelled.load(Ordering::Acquire) {
            Err(io::Error::new(io::ErrorKind::Interrupted, "录制已取消"))
        } else {
            Ok(())
        }
    }
}

impl Recorder {
    fn admit(&self) -> Option<Arc<Signals>> {
        let mut state = self.state.lock().expect("recording queue state");
        if state.closed || state.active.is_some() {
            return None;
        }
        let signals = Arc::new(Signals {
            cancelled: Arc::new(AtomicBool::new(false)),
            cancel: watch::channel(false).0,
            done: watch::channel(false).0,
        });
        state.active = Some(Arc::clone(&signals));
        Some(signals)
    }

    pub(in crate::native_http) fn abort(&self) -> HttpResponse {
        let state = self.state.lock().expect("recording queue state");
        if state.publishing {
            return detail(StatusCode::CONFLICT, "录制已结束，正在登记录制素材");
        }
        if let Some(active) = &state.active {
            active.stop();
            json_reply(StatusCode::OK, json!({"status":"ok"}))
        } else {
            json_reply(StatusCode::OK, json!({"status":"idle"}))
        }
    }

    pub(in crate::native_http) async fn shutdown(&self) {
        let active = {
            let mut state = self.state.lock().expect("recording queue state");
            state.closed = true;
            if let Some(active) = state.active.as_ref().filter(|_| !state.publishing) {
                active.stop();
            }
            state.active.clone()
        };
        if let Some(active) = active {
            let _ = active.done.subscribe().wait_for(|done| *done).await;
        }
    }

    fn begin_publication(&self) -> io::Result<()> {
        let mut state = self.state.lock().expect("recording queue state");
        state.active.as_ref().expect("admitted recording").check()?;
        state.publishing = true;
        Ok(())
    }

    pub(in crate::native_http) async fn notify_gsi(
        &self,
        request: Request<Incoming>,
    ) -> HttpResponse {
        // Do not reflect the private game token in parser errors.
        let Ok(bytes) =
            crate::native_http::read_body(request.into_body(), "GSI request is too large").await
        else {
            return detail(StatusCode::BAD_REQUEST, "GSI 数据格式错误");
        };
        let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
        let Ok(payload) = serde_json::from_slice::<Value>(bytes) else {
            return detail(StatusCode::BAD_REQUEST, "GSI 数据格式错误");
        };
        match self.gsi.notify(&payload) {
            Ok(ready) => json_reply(StatusCode::OK, json!({"ok":true,"ready":ready})),
            Err(()) => detail(StatusCode::UNAUTHORIZED, "当前录制会话未授权"),
        }
    }
}

struct ActiveJob {
    profile: Arc<ProfileState>,
    signals: Arc<Signals>,
}

impl Drop for ActiveJob {
    fn drop(&mut self) {
        let mut state = self
            .profile
            .recording
            .state
            .lock()
            .expect("recording queue state");
        state.active = None;
        state.publishing = false;
        self.signals.done.send_replace(true);
    }
}

#[derive(Deserialize)]
struct QueueRequest {
    requests: Vec<RecordingRequestDto>,
    #[serde(default)]
    warmup: Option<Warmup>,
    obs: Option<ObsOverride>,
    pov_hud: Option<Value>,
    cs2_extra_launch_args: Option<String>,
    record_inject_console_lines: Option<String>,
}

#[derive(Deserialize)]
struct ObsOverride {
    host: String,
    port: u16,
    #[serde(default)]
    password: String,
}

#[derive(Deserialize)]
#[serde(default)]
struct Warmup {
    resolution_width: Option<NonZeroU16>,
    resolution_height: Option<NonZeroU16>,
    aspect_ratio: Option<String>,
    cl_draw_only_deathnotices: bool,
    hud_showtargetid_hide: bool,
    tv_nochat: bool,
    hide_demo_playback_ui: bool,
    hide_grenade_trajectory_pip: bool,
    viewmodel_fov_68: bool,
    third_person_camera: bool,
    fov_cs_debug: Option<f64>,
    spec_show_xray: i32,
    spectator_flashbang_opacity: Option<f64>,
    voice_filter: String,
    console_cmds: Option<Vec<String>>,
}

impl Default for Warmup {
    fn default() -> Self {
        Self {
            resolution_width: None,
            resolution_height: None,
            aspect_ratio: None,
            cl_draw_only_deathnotices: true,
            hud_showtargetid_hide: true,
            tv_nochat: true,
            hide_demo_playback_ui: true,
            hide_grenade_trajectory_pip: true,
            viewmodel_fov_68: false,
            third_person_camera: false,
            fov_cs_debug: None,
            spec_show_xray: 0,
            spectator_flashbang_opacity: None,
            voice_filter: "mute".to_owned(),
            console_cmds: None,
        }
    }
}

impl Warmup {
    fn resolution(&self) -> Option<Resolution> {
        Some(Resolution {
            width: self.resolution_width?,
            height: self.resolution_height?,
            aspect_ratio: match self.aspect_ratio.as_deref() {
                Some("4:3") => Some(AspectRatio::Classic),
                Some("16:9") => Some(AspectRatio::Wide),
                Some("16:10") => Some(AspectRatio::WideTall),
                _ => None,
            },
        })
    }

    fn commands(&self) -> Vec<String> {
        let mut commands = vec![
            format!(
                "cl_draw_only_deathnotices {}",
                self.cl_draw_only_deathnotices
            ),
            format!("hud_showtargetid {}", u8::from(!self.hud_showtargetid_hide)),
            format!("tv_nochat {}", u8::from(self.tv_nochat)),
        ];
        if self.hide_demo_playback_ui {
            commands.extend(["sv_cheats 1", "demoui false"].map(str::to_owned));
        }
        commands.push(format!(
            "spec_show_xray {}",
            u8::from(self.spec_show_xray != 0)
        ));
        commands.push("cl_grenadepreview 0".to_owned());
        if let Some(fov) = self.fov_cs_debug.filter(|value| value.is_finite()) {
            commands.push(format!("fov_cs_debug {fov}"));
        }
        if self.viewmodel_fov_68 {
            commands.push("viewmodel_fov 68".to_owned());
        }
        if let Some(opacity) = self
            .spectator_flashbang_opacity
            .filter(|value| value.is_finite())
        {
            commands.push(format!(
                "r_spectator_flashbang_opacity {}",
                opacity.clamp(0.2, 1.0)
            ));
        }
        let voice: &[&str] = match self.voice_filter.as_str() {
            "mute" | "all" => &[
                "tv_listen_voice_indices 0",
                "tv_listen_voice_indices_h 0",
                "voice_modenable 0",
                "snd_voipvolume 0",
            ],
            "open" => &[
                "voice_modenable 1",
                "snd_voipvolume 1",
                "tv_listen_voice_indices -1",
                "tv_listen_voice_indices_h -1",
            ],
            _ => &[],
        };
        commands.extend(voice.iter().map(|command| (*command).to_owned()));
        if self.hide_grenade_trajectory_pip {
            commands.extend(
                [
                    "sv_grenade_trajectory 0",
                    "sv_grenade_trajectory_prac_pipreview 0",
                    "sv_grenade_trajectory_time_spectator 0",
                ]
                .map(str::to_owned),
            );
        }
        commands
    }
}

pub(in crate::native_http) async fn respond(
    profile: &Arc<ProfileState>,
    port: NonZeroU16,
    request: Request<Incoming>,
    extended: &AtomicBool,
) -> HttpResponse {
    let body = match model_body(request, "录制队列请求过大").await {
        Ok(body) => body,
        Err(error) => return *error,
    };
    let Ok(input) = serde_json::from_value::<QueueRequest>(body) else {
        return detail(StatusCode::UNPROCESSABLE_ENTITY, "录制队列参数无效");
    };
    if input.requests.is_empty() {
        return detail(
            StatusCode::UNPROCESSABLE_ENTITY,
            "请选择需要录制的 Demo 片段",
        );
    }
    if input.requests.len() != 1 {
        return detail(
            StatusCode::NOT_IMPLEMENTED,
            "当前 Rust 录制路径只支持一个连续片段",
        );
    }
    let Some(signals) = profile.recording.admit() else {
        return problem(StatusCode::CONFLICT, "RECORDING_ALREADY_RUNNING");
    };
    let job = ActiveJob {
        profile: Arc::clone(profile),
        signals,
    };
    extended.store(true, Ordering::Relaxed);
    // Once admitted, the operation must finish its cleanup even if the HTTP client closes.
    match tokio::spawn(async move { run(&job, port, input).await }).await {
        Ok(response) => response,
        Err(_) => detail(
            StatusCode::INTERNAL_SERVER_ERROR,
            "录制任务异常退出，请检查配置备份状态",
        ),
    }
}

async fn run(job: &ActiveJob, port: NonZeroU16, mut input: QueueRequest) -> HttpResponse {
    let profile = &job.profile;
    let mut dto = input.requests.pop().expect("single request admitted");
    let warmup = input.warmup.take().unwrap_or_default();
    if unsupported(&input, &dto, &warmup) {
        return detail(
            StatusCode::NOT_IMPLEMENTED,
            "当前 Rust 录制路径尚未支持本次选用的叠层、转场或自定义录制参数",
        );
    }
    let Ok(session) = random_id::<16>() else {
        return detail(StatusCode::INTERNAL_SERVER_ERROR, "无法创建本次录制会话");
    };
    let start = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let lease = match profile.session.try_begin(OperationOwner {
        id: session.clone(),
        operation: "/api/recording/queue".to_owned(),
        started_at: start.as_secs_f64().to_string(),
    }) {
        Ok(lease) => lease,
        Err(busy) => return json_reply(StatusCode::CONFLICT, json!({"detail":busy})),
    };
    let Ok(_obs_write) = profile.obs_writes.try_acquire() else {
        return detail(StatusCode::CONFLICT, "OBS 正在配置，请稍候");
    };
    match probe_process("cs2.exe") {
        Ok(true) => return problem(StatusCode::CONFLICT, "RECORDING_CS2_RUNNING"),
        Ok(false) => {}
        Err(_) => return detail(StatusCode::SERVICE_UNAVAILABLE, "无法确认 CS2 是否已退出"),
    }
    if job.signals.check().is_err() {
        return result_error(&dto.request_id, "recording_aborted", true);
    }
    let source = match local_demo(&dto, &profile.upload_dir) {
        Ok(path) => path,
        Err(_) => return detail(StatusCode::NOT_FOUND, "找不到选中的 Demo 文件"),
    };
    dto.demo.demo_path = path_text(&source);
    let Ok(parser) = Arc::clone(&profile.demo_parser).try_acquire_owned() else {
        return detail(
            StatusCode::SERVICE_UNAVAILABLE,
            "正在处理其他 Demo，请稍后重试",
        );
    };
    let plan = match tokio::task::spawn_blocking(move || {
        let _parser = parser;
        build(dto)
    })
    .await
    {
        Ok(Ok(plan)) => plan,
        Ok(Err((status, message))) => return detail(status, &message),
        Err(_) => return detail(StatusCode::INTERNAL_SERVER_ERROR, "无法生成录制计划"),
    };
    match capture(job, &lease, port, &session, &plan, &input, &warmup).await {
        Ok(result) => json_reply(StatusCode::OK, json!([result])),
        Err(error) => result_error(
            &plan.request.request_id,
            &error.to_string(),
            error.kind() == io::ErrorKind::Interrupted,
        ),
    }
}

async fn capture(
    job: &ActiveJob,
    lease: &RuntimeLease<'_>,
    port: NonZeroU16,
    session: &str,
    plan: &RecordingPlan,
    input: &QueueRequest,
    warmup: &Warmup,
) -> io::Result<Value> {
    let profile = &job.profile;
    let mut cancel = job.signals.cancel.subscribe();
    let request = &plan.request;
    let [planned] = plan.active.as_slice() else {
        return Err(io::Error::other("当前 Rust 录制路径只支持一个连续片段"));
    };
    let round = planned
        .round
        .filter(|round| *round >= 1)
        .ok_or_else(|| io::Error::other("无法识别录制回合"))?;
    let slot = planned
        .target_spec_slot
        .ok_or_else(|| io::Error::other("计划未提供可校验的玩家观战槽位"))?;
    let target = planned
        .target_steamid64
        .parse::<u64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| io::Error::other("计划未提供可校验的 Steam 玩家 ID"))?;
    let seconds = |ticks: i64| {
        Duration::try_from_secs_f64(ticks.max(0) as f64 / request.demo.tick_rate)
            .map_err(|_| io::Error::other("录制片段的时间范围无效"))
    };
    let segment = execute_single::Segment {
        seek_tick: planned.safe_seek_tick,
        pre_roll: seconds(planned.start_tick - planned.safe_seek_tick)?,
        duration: seconds(planned.end_tick - planned.start_tick)?,
        view: execute_single::View {
            map: &request.demo.map_name,
            round,
            steam_id: target,
            slot,
        },
    };
    if request.demo.map_name.is_empty() || segment.duration.is_zero() {
        return Err(io::Error::other("录制片段缺少有效的地图或时间范围"));
    }
    let cfg = profile.config.view()?;
    let cs2 = cfg["cs2_path"]
        .as_str()
        .filter(|path| !path.trim().is_empty())
        .ok_or_else(|| io::Error::other("请先在设置中选择 CS2 安装路径"))?;
    let demo = Path::new(&request.demo.demo_path);
    let token = random_id::<32>()?;
    let mut feed = profile.recording.gsi.begin(token.clone())?;
    let mut spec = playback_spec::build(
        Path::new(cs2),
        demo,
        session,
        "F10",
        warmup.resolution(),
        GsiTarget {
            port,
            token: &token,
        },
    )?;
    let extra = input
        .cs2_extra_launch_args
        .as_deref()
        .unwrap_or_else(|| cfg["cs2_extra_launch_args"].as_str().unwrap_or_default());
    if !extra
        .split_ascii_whitespace()
        .all(|arg| matches!(arg, "-fullscreen" | "-windowed" | "-noborder"))
    {
        return Err(io::Error::other(
            "当前 Rust 录制路径尚未支持本次自定义启动参数",
        ));
    }
    let launch_flags = extra.split_ascii_whitespace().map(str::to_owned);
    spec.args.splice(..0, launch_flags);
    let extra_lines = input
        .record_inject_console_lines
        .as_deref()
        .unwrap_or_else(|| {
            cfg["record_inject_console_lines"]
                .as_str()
                .unwrap_or_default()
        });
    let mut commands = warmup.commands();
    let mut after_selection = Vec::new();
    for line in extra_lines
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with("//"))
    {
        match line {
            "cl_hud_telemetry_frametime_show 0"
            | "engine_no_focus_sleep 0"
            | "fps_max 0"
            | "cl_trueview_show_status 0" => commands.push(line.to_owned()),
            "cl_demo_predict 0" => after_selection.push(line),
            _ => {
                return Err(io::Error::other(
                    "当前 Rust 录制路径尚未支持本次自定义控制台参数",
                ))
            }
        }
    }
    let controls = Bindings::configure(
        &mut spec,
        planned.safe_seek_tick,
        slot,
        &commands,
        &after_selection,
    )?;
    let ffprobe = ffmpeg::find_ffprobe(&profile.config.ffmpeg_executable()?)
        .ok_or_else(|| io::Error::other("请配置与 FFmpeg 配套的 FFprobe"))?;

    let mut obs = profile.config.obs_settings()?;
    if let Some(settings) = &input.obs {
        obs = obs.with_check_input(&settings.host, settings.port, &settings.password);
    }
    let mut capture = ObsCapture::connect(obs).await?;
    capture.require_idle().await?;
    let scene = std::env::var("CS2_INSIGHT_OBS_SCENE_NAME")
        .ok()
        .or_else(|| cfg["obs_game_scene_name"].as_str().map(str::to_owned))
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "CS2 Insight Recording".to_owned());
    capture.prepare_scene(scene).await?;
    let directory =
        playback_spec::drive_path(&capture.recording_directory().await?.canonicalize()?)?;
    if !directory.is_dir() {
        return Err(io::Error::other("OBS 录制目录不存在"));
    }
    job.signals.check()?;
    let data = profile.config.data_dir().to_owned();
    let game = spec.working_dir.clone();
    tokio::task::spawn_blocking(move || config_backup::snapshot::capture_for_game(&data, &game))
        .await
        .map_err(io::Error::other)??;

    let started = SystemTime::now();
    let recorded =
        match PlaybackProcess::launch(lease, spec, Arc::clone(&job.signals.cancelled)).await {
            Ok(mut process) => {
                let recording = async {
                    process
                        .wait_ready(
                            &mut feed,
                            &request.demo.map_name,
                            &mut cancel,
                            Duration::from_secs(90),
                        )
                        .await?;
                    let mut game =
                        GameWindow::find(&mut process, controls, feed.updates.clone(), &mut cancel)
                            .await?;
                    execute_single::run(&mut game, &mut capture, segment, &mut cancel).await
                };
                let result = recording.await;
                let result = after_cleanup(result, capture.finish_if_owned().await.map(|_| ()));
                after_cleanup(result, process.finish().await)
            }
            Err(error) => Err(error),
        };
    let data = profile.config.data_dir().to_owned();
    let restored = tokio::task::spawn_blocking(move || config_backup::restore_profile(&data))
        .await
        .map_err(io::Error::other)?
        .map(|_| ())
        .map_err(|_| io::Error::other("玩家配置仍待恢复，请查看设置中的配置备份"));
    let artifact = after_cleanup(recorded, restored)?;
    job.signals.check()?;
    let path = local_output(&artifact.file, &directory, started)?;
    let media = ffmpeg::inspect(&ffprobe, &path, Some(&cancel)).await?;
    let duration = media["format"]["duration"]
        .as_str()
        .and_then(|seconds| seconds.parse::<f64>().ok())
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
        .ok_or_else(|| io::Error::other("OBS 录制文件没有有效时长"))?;
    if !media["streams"].as_array().is_some_and(|streams| {
        streams.iter().any(|stream| {
            stream["codec_type"] == "video"
                && stream["width"].as_u64().unwrap_or(0) > 0
                && stream["height"].as_u64().unwrap_or(0) > 0
        })
    }) {
        return Err(io::Error::other("OBS 录制文件没有有效的视频轨道"));
    }
    let output = VerifiedOutput::validate(&path, &directory, started, duration)?;
    let database = profile.config.database_path();
    let output_path = path_text(output.path());
    let clip_id = request
        .source_ref
        .original_clip_id
        .as_deref()
        .unwrap_or(&request.request_id)
        .to_owned();
    let demo_path = request.demo.demo_path.clone();
    let filename = request.demo.demo_filename.clone();
    let player = planned.target_player_name.clone();
    let segment_status = json!({"segment_index":planned.segment_index,"status":"ok","error":null,"duration_sec":artifact.elapsed.as_secs_f64()});
    let metadata = json!({
        "recording_request_type":request.request_type,
        "source_rounds":[round],
        "planned_segments":[planned],
        "segment_results":[segment_status],
        "warnings":request.warnings,
    });
    profile.recording.begin_publication()?;
    let id = tokio::task::spawn_blocking(move || {
        insert_ready(
            &database,
            CapturedClip {
                clip_id: &clip_id,
                demo_path: &demo_path,
                filename: Some(&filename),
                player: Some(&player),
                metadata: metadata.as_object().expect("clip metadata"),
            },
            &output,
        )
    })
    .await
    .map_err(io::Error::other)?
    .map_err(io::Error::other)?;
    Ok(json!({
        "request_id":request.request_id,"success":true,
        "output_path":output_path,"recorded_clip_id":id,"recorded_clip_duration_sec":duration,
        "warnings":request.warnings,"error":null,"segment_results":[segment_status],
    }))
}

impl execute_single::SegmentCapture for ObsCapture {
    async fn start(&mut self) -> io::Result<()> {
        self.start().await
    }

    async fn stop(&mut self) -> io::Result<Option<PathBuf>> {
        self.stop().await
    }
}

fn after_cleanup<T>(result: io::Result<T>, cleanup: io::Result<()>) -> io::Result<T> {
    match (result, cleanup) {
        (result, Ok(())) => result,
        (Ok(_), Err(cleanup)) => Err(cleanup),
        (Err(error), Err(cleanup)) => Err(io::Error::other(format!("{error}。{cleanup}"))),
    }
}

fn local_demo(dto: &RecordingRequestDto, uploaded: &Path) -> io::Result<PathBuf> {
    let candidate = Path::new(if dto.demo.demo_path.trim().is_empty() {
        dto.demo.demo_filename.trim()
    } else {
        dto.demo.demo_path.trim()
    });
    if candidate.is_absolute() && candidate.is_file() {
        return playback_spec::drive_path(&candidate.canonicalize()?);
    }
    let file = candidate
        .file_name()
        .ok_or_else(|| io::Error::other("缺少 Demo 文件名"))?;
    playback_spec::drive_path(&uploaded.join(file).canonicalize()?)
}

fn unsupported(input: &QueueRequest, dto: &RecordingRequestDto, warmup: &Warmup) -> bool {
    let opt = &dto.options;
    input
        .pov_hud
        .as_ref()
        .is_some_and(|pov| pov["enabled"] == true)
        || opt.obs_transition_enabled == Some(true)
        || opt.kb_overlay_enabled == Some(true)
        || opt.kill_fx_enabled == Some(true)
        || warmup.third_person_camera
        || !matches!(
            warmup.voice_filter.as_str(),
            "mute" | "all" | "open" | "off"
        )
        || warmup
            .console_cmds
            .as_ref()
            .is_some_and(|raw| !raw.is_empty() && *raw != warmup.commands())
}

fn random_id<const N: usize>() -> io::Result<String> {
    let mut bytes = [0_u8; N];
    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            N as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status < 0 {
        return Err(io::Error::other("系统随机数生成失败"));
    }
    let mut id = String::with_capacity(N * 2);
    for byte in bytes {
        write!(id, "{byte:02x}").expect("write to string");
    }
    Ok(id)
}

fn problem(status: StatusCode, code: &str) -> HttpResponse {
    json_reply(status, json!({"detail":{"code":code}}))
}

fn result_error(request: &str, error: &str, cancelled: bool) -> HttpResponse {
    json_reply(
        StatusCode::OK,
        json!([{
            "request_id":request,"success":false,"aborted":cancelled,
            "error":error,"output_path":null,"segment_results":[],
        }]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_abort_or_clip_publication_but_never_both() {
        let cancelled = Recorder::default();
        let signals = cancelled.admit().unwrap();
        assert_eq!(cancelled.abort().status(), StatusCode::OK);
        assert_eq!(
            cancelled.begin_publication().unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
        assert!(*signals.cancel.borrow());

        let recording = Recorder::default();
        let signals = recording.admit().unwrap();
        recording.begin_publication().unwrap();
        assert_eq!(recording.abort().status(), StatusCode::CONFLICT);
        assert!(!*signals.cancel.borrow());
    }
}
