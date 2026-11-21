use super::{
    audio_mix,
    export_sources::{base_track, main_clip, media_path, sort_timeline},
    exports::{self, Read},
    ffmpeg::{command, find_ffprobe},
    has_table, json_reply, model_body, profile_disk,
    project_schema::{list, normalize},
    project_store, read_table, render, HttpResponse, ProfileState,
};
use crate::{
    native_http::{demo_body::field_issue, profile_work, validation_reply},
    profile_platform::{expand_user, path_text},
    recorded_clips,
};
use hyper::{body::Incoming, Request, StatusCode};
use rusqlite::{params, Connection, TransactionBehavior};
use serde_json::{json, Value};
use std::{
    collections::{BTreeSet, HashMap},
    error::Error,
    fs, io,
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicI64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    sync::{oneshot, watch},
    time::timeout,
};

const FAILED: &str = "MONTAGE_EXPORT_FAILED";

#[derive(Default)]
pub(crate) struct Exports(Mutex<Option<Arc<Job>>>);

impl Exports {
    fn current(&self) -> Option<Arc<Job>> {
        self.0.lock().expect("LiteCut active job").clone()
    }

    pub(in crate::native_http) async fn shutdown(&self) {
        if let Some(job) = self.current() {
            job.cancel.send_replace(true);
            let mut done = job.done.subscribe();
            let _ = timeout(Duration::from_secs(12), done.wait_for(|done| *done)).await;
        }
    }

    pub(super) fn apply_progress(&self, row: &mut Value) {
        if let Some(job) = self.current().filter(|job| row["export_id"] == job.id()) {
            let status = job.status.lock().expect("LiteCut export progress");
            if matches!(row["status"].as_str(), Some("queued" | "running")) {
                row["status"] = json!(status.state);
                row["stage"] = json!(status.stage);
                row["progress"] = json!(status.progress);
            }
        }
    }
}

struct Job {
    id: AtomicI64,
    status: Mutex<Progress>,
    cancel: watch::Sender<bool>,
    done: watch::Sender<bool>,
}

struct Progress {
    state: &'static str,
    stage: &'static str,
    progress: f64,
}

impl Job {
    fn id(&self) -> i64 {
        self.id.load(Ordering::Relaxed)
    }

    fn update(&self, progress: f64, stage: &'static str) {
        let mut status = self.status.lock().expect("LiteCut export progress");
        if status.state != "cancelling" {
            status.state = "running";
            status.stage = stage;
            status.progress = progress.max(status.progress);
        }
    }
}

struct ExportRequest {
    project_id: Option<i64>,
    body: Option<Value>,
    output: String,
}

impl ExportRequest {
    fn parse(json: &Value) -> Result<Self, Vec<Value>> {
        let fields = super::model(json)?;
        let project_id = match fields.get("project_id").filter(|value| !value.is_null()) {
            None => None,
            Some(value) => {
                let id = value
                    .as_i64()
                    .or_else(|| value.as_str()?.trim().parse().ok());
                Some(id.ok_or_else(|| {
                    vec![field_issue(
                        "project_id",
                        value,
                        "int_parsing",
                        "Input should be a valid integer",
                        None,
                    )]
                })?)
            }
        };
        let body = match fields.get("body").filter(|value| !value.is_null()) {
            Some(value) if !value.is_object() => {
                return Err(vec![field_issue(
                    "body",
                    value,
                    "dict_type",
                    "Input should be a valid dictionary",
                    None,
                )])
            }
            value => value.cloned(),
        };
        let output = fields.get("output_path").ok_or_else(|| {
            vec![field_issue(
                "output_path",
                json,
                "missing",
                "Field required",
                None,
            )]
        })?;
        let output =
            super::text(output, "output_path", 0, usize::MAX).map_err(|issue| vec![issue])?;
        Ok(Self {
            project_id,
            body,
            output: output.to_owned(),
        })
    }
}

struct Prepared {
    project_id: Option<i64>,
    project_name: Option<String>,
    body: Value,
    clips: Vec<Value>,
    paths: Vec<PathBuf>,
    track_gain: f64,
    audio: audio_mix::Plan,
    overlays: render::overlay::Plan,
    range: Option<(f64, Option<f64>)>,
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    requested_encoder: String,
    output: PathBuf,
}

struct Problem(StatusCode, &'static str, Option<Value>);

impl Problem {
    fn input(code: &'static str) -> Self {
        Self(StatusCode::BAD_REQUEST, code, None)
    }

    fn response(self) -> HttpResponse {
        let mut result = self.2.unwrap_or_else(|| json!({}));
        result["code"] = json!(self.1);
        json_reply(self.0, json!({"detail":result}))
    }
}

pub(super) async fn start(
    profile: &Arc<ProfileState>,
    request: Request<Incoming>,
    background: bool,
    shutdown: watch::Receiver<bool>,
    extended: &AtomicBool,
) -> HttpResponse {
    let input = match model_body(request, "LiteCut export request is too large").await {
        Ok(input) => input,
        Err(error) => return *error,
    };
    let input = match ExportRequest::parse(&input) {
        Ok(input) => input,
        Err(errors) => return validation_reply(errors),
    };
    let Ok(permit) = Arc::clone(&profile.media_process).try_acquire_owned() else {
        return Problem(
            StatusCode::CONFLICT,
            FAILED,
            Some(json!({"message":"已有媒体任务正在运行"})),
        )
        .response();
    };
    extended.store(true, Ordering::Relaxed);
    let prepared = match profile_work(profile, move |profile| prepare(profile, input)).await {
        Ok(Ok(prepared)) => prepared,
        Ok(Err(error)) => return error.response(),
        Err(_) => return export_failed(),
    };
    let (cancel, signal) = watch::channel(false);
    if timeout(
        Duration::from_secs(12),
        command(&prepared.ffmpeg).arg("-version").status(),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .is_none_or(|status| !status.success())
    {
        return Problem::input("MONTAGE_FFMPEG_NOT_RUNNABLE").response();
    }

    let job = Arc::new(Job {
        id: AtomicI64::new(0),
        status: Mutex::new(Progress {
            state: "queued",
            stage: "queued",
            progress: 0.0,
        }),
        cancel,
        done: watch::channel(false).0,
    });
    *profile
        .lite_cut_exports
        .0
        .lock()
        .expect("LiteCut active job") = Some(Arc::clone(&job));
    let profile = Arc::clone(profile);
    let (send_ready, receive_ready) = oneshot::channel();
    let task = tokio::spawn(async move {
        let _permit = permit;
        let result = async {
            let (id, queued, prepared) = profile_work(&profile, move |profile| {
                create(&profile.config.database_path(), &prepared)
                    .map(|(id, row)| (id, row, prepared))
                    .map_err(|_| FAILED)
            })
            .await
            .map_err(|_| FAILED)??;
            job.id.store(id, Ordering::Relaxed);
            let _ = send_ready.send((id, queued));
            finish(&profile, &job, prepared, signal, shutdown).await
        }
        .await;
        *profile
            .lite_cut_exports
            .0
            .lock()
            .expect("LiteCut active job") = None;
        job.done.send_replace(true);
        result
    });
    let Ok((id, queued)) = receive_ready.await else {
        return export_failed();
    };
    if background {
        return json_reply(StatusCode::OK, queued);
    }
    match task.await {
        Ok(Ok(path)) => json_reply(
            StatusCode::OK,
            json!({"export_id": id, "status": "done", "output_path": path_text(&path)}),
        ),
        Ok(Err(code)) => Problem::input(code).response(),
        Err(_) => export_failed(),
    }
}

pub(super) async fn cancel(profile: &Arc<ProfileState>, id: i64) -> HttpResponse {
    match profile
        .lite_cut_exports
        .current()
        .filter(|job| id > 0 && job.id() == id)
    {
        Some(job) => {
            {
                let mut status = job.status.lock().expect("LiteCut export progress");
                if status.stage != "publishing" {
                    status.state = "cancelling";
                    status.stage = "cancelling";
                    job.cancel.send_replace(true);
                }
            }
            exports::respond(profile, Read::One(id)).await
        }
        None => {
            profile_disk(profile, move |profile| {
                match Read::One(id).read(&profile.config.database_path()) {
                    Ok(Some(_)) => {
                        Problem(StatusCode::CONFLICT, "LITECUT_EXPORT_NOT_ACTIVE", None).response()
                    }
                    Ok(None) => {
                        Problem(StatusCode::NOT_FOUND, "LITECUT_EXPORT_NOT_FOUND", None).response()
                    }
                    Err(_) => export_failed(),
                }
            })
            .await
        }
    }
}

fn prepare(profile: &ProfileState, input: ExportRequest) -> Result<Prepared, Problem> {
    let ffmpeg = profile
        .config
        .ffmpeg_executable()
        .map_err(|_| Problem::input("MONTAGE_FFMPEG_NOT_FOUND"))?;
    let ffprobe =
        find_ffprobe(&ffmpeg).ok_or_else(|| Problem::input("MONTAGE_FFPROBE_NOT_FOUND"))?;
    let database = profile.config.database_path();
    let (project_name, body) = match input.project_id {
        Some(id) => {
            let project = read_table(&database, "lite_cut_projects", |db| {
                project_store::get(db, id)
            })
            .map_err(|_| Problem(StatusCode::INTERNAL_SERVER_ERROR, FAILED, None))?
            .flatten()
            .ok_or(Problem(
                StatusCode::NOT_FOUND,
                "LITECUT_PROJECT_NOT_FOUND",
                None,
            ))?;
            (
                project["name"].as_str().map(str::to_owned),
                project["body"].clone(),
            )
        }
        None => (
            None,
            input
                .body
                .ok_or_else(|| Problem::input("LITECUT_EXPORT_NO_BODY"))?,
        ),
    };
    let body = normalize(Some(body)).map_err(|message| {
        Problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "LITECUT_PROJECT_INVALID",
            Some(json!({"message":message})),
        )
    })?;
    let tracks = list(&body["tracks"]);
    let base = base_track(tracks).ok_or_else(|| Problem::input("MONTAGE_NO_CLIPS"))?;
    let track = &tracks[base];
    let mut clips: Vec<_> = list(&track["clips"])
        .iter()
        .filter(|clip| main_clip(clip))
        .collect();
    sort_timeline(&mut clips);
    validate_effects(&body, base, &clips)?;
    let track_gain = if track["muted"] == true
        || tracks
            .iter()
            .any(|track| track["type"] == "audio" && track["solo"] == true)
    {
        0.0
    } else {
        render::number(&track["volume"]).min(2.0)
    };

    let mut end = 0.0;
    let mut previous = None;
    for clip in &clips {
        let start = render::number(&clip["timeline_start"]);
        if start < end - 0.001 {
            return Err(Problem(
                StatusCode::UNPROCESSABLE_ENTITY,
                "LITECUT_TIMELINE_OVERLAP",
                Some(json!({"params":{"previous_clip_id":previous,"clip_id":clip["id"]}})),
            ));
        }
        end = start + render::length(clip);
        previous = clip["id"].as_str();
    }

    let source_ids: BTreeSet<_> = clips
        .iter()
        .filter(|clip| clip["source_type"] != "file")
        .filter_map(|clip| clip["source_id"].as_i64())
        .collect();
    let recordings = recorded_clips::output_paths(&database, &source_ids)
        .map_err(|_| Problem(StatusCode::INTERNAL_SERVER_ERROR, FAILED, None))?;
    let resolved = resolve_media(&clips, &recordings)?;
    let audio = audio_mix::Plan::from_project(&body, |id| recordings.get(&id).map(PathBuf::from));
    let overlays = render::overlay::Plan::from_project(&body).map_err(|()| unavailable_effect())?;
    let mut source_lengths: HashMap<_, _> = resolved
        .iter()
        .map(|(path, bytes)| (path.clone(), *bytes))
        .collect();
    for path in audio.sources().chain(overlays.sources()) {
        let (path, bytes) = readable_media(path)?;
        source_lengths.insert(path, bytes);
    }
    let source_bytes = source_lengths
        .into_values()
        .fold(0_u64, u64::saturating_add);
    let paths = resolved.into_iter().map(|(path, _)| path).collect();

    let output_settings = &body["output"];
    let range = if output_settings["range_mode"] == "custom" {
        let start = render::number(&output_settings["range_start_sec"]);
        let stop = output_settings["range_end_sec"].as_f64();
        if start >= end {
            return Err(Problem::input("MONTAGE_OUTPUT_NOT_PLAYABLE"));
        }
        Some((start, stop))
    } else {
        None
    };
    let configuration = profile
        .config
        .view()
        .map_err(|_| Problem(StatusCode::INTERNAL_SERVER_ERROR, FAILED, None))?;
    let requested_encoder = output_settings["encoder"]
        .as_str()
        .filter(|value| *value != "auto")
        .unwrap_or_else(|| configuration["montage_encoder"].as_str().unwrap_or("auto"))
        .to_owned();
    let output = unique_output(&input.output)?;
    let working_bytes = (end * 5_000_000.0) as u64 + (source_bytes / 10).min(4 * 1024_u64.pow(3));
    verify_disk(
        output.parent().expect("absolute output parent"),
        working_bytes.max(512 * 1024 * 1024),
    )?;
    let clips = clips.into_iter().cloned().collect();
    Ok(Prepared {
        project_id: input.project_id,
        project_name,
        body,
        clips,
        paths,
        track_gain,
        audio,
        overlays,
        range,
        ffmpeg,
        ffprobe,
        requested_encoder,
        output,
    })
}

fn validate_effects(project: &Value, base: usize, clips: &[&Value]) -> Result<(), Problem> {
    let tracks = list(&project["tracks"]);
    let parallel = tracks[..base].iter().any(|track| {
        track["hidden"] == false && track["type"] == "video" && !list(&track["clips"]).is_empty()
    });
    let effects = clips.iter().any(|clip| {
        ["transition_in", "transition_out"].iter().any(|key| {
            let effect = &clip[*key];
            effect["type"]
                .as_str()
                .is_some_and(|kind| !matches!(kind, "none" | "cut"))
                && effect["duration_sec"].as_f64().unwrap_or_default() > 0.0
        }) || !clip["transform"].is_null()
            || ["keyframes", "speed_keyframes", "audio_keyframes"]
                .iter()
                .any(|key| !list(&clip[*key]).is_empty())
    });
    if parallel || effects {
        return Err(unavailable_effect());
    }
    Ok(())
}

fn unavailable_effect() -> Problem {
    Problem(
        StatusCode::UNPROCESSABLE_ENTITY,
        "LITECUT_EXPORT_EFFECT_UNAVAILABLE",
        Some(json!({"message":"此 Rust 测试版本暂未支持工程中的部分叠加、转场或关键帧效果"})),
    )
}

fn resolve_media(
    clips: &[&Value],
    recordings: &HashMap<i64, String>,
) -> Result<Vec<(PathBuf, u64)>, Problem> {
    clips
        .iter()
        .map(|clip| {
            let raw = if clip["source_type"] == "file" {
                clip["file_path"]
                    .as_str()
                    .filter(|value| !value.trim().is_empty())
            } else {
                clip["source_id"]
                    .as_i64()
                    .and_then(|id| recordings.get(&id))
                    .map(String::as_str)
            }
            .ok_or_else(|| Problem::input("MONTAGE_CLIP_NOT_FOUND"))?;
            readable_media(&media_path(raw))
        })
        .collect()
}

fn readable_media(input: &Path) -> Result<(PathBuf, u64), Problem> {
    let problem = |error: io::Error| {
        let code = if error.kind() == io::ErrorKind::NotFound {
            "MONTAGE_CLIP_FILE_MISSING"
        } else {
            "MONTAGE_SOURCE_NOT_READABLE"
        };
        Problem(
            StatusCode::BAD_REQUEST,
            code,
            Some(json!({"params":{"name":input.file_name().map(|name| name.to_string_lossy())}})),
        )
    };
    let path = input.canonicalize().map_err(problem)?;
    let file = fs::File::open(&path).map_err(problem)?;
    let metadata = file.metadata().map_err(problem)?;
    if !metadata.is_file() {
        return Err(problem(io::Error::new(
            io::ErrorKind::NotFound,
            "not a media file",
        )));
    }
    Ok((path, metadata.len()))
}

fn unique_output(raw: &str) -> Result<PathBuf, Problem> {
    if raw.trim().is_empty() {
        return Err(Problem::input("MONTAGE_OUTPUT_PATH_EMPTY"));
    }
    let output =
        expand_user(raw.trim()).map_err(|_| Problem::input("MONTAGE_OUTPUT_PATH_INVALID"))?;
    if !output.is_absolute() {
        return Err(Problem::input("MONTAGE_OUTPUT_PATH_NOT_ABSOLUTE"));
    }
    if !output
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"))
    {
        return Err(Problem::input("MONTAGE_OUTPUT_NOT_MP4"));
    }
    if output.components().any(|part| part == Component::ParentDir) {
        return Err(Problem::input("MONTAGE_OUTPUT_PATH_INVALID"));
    }
    let dir = output
        .parent()
        .ok_or_else(|| Problem::input("MONTAGE_OUTPUT_PATH_INVALID"))?;
    fs::create_dir_all(dir).map_err(|_| Problem::input("MONTAGE_OUTPUT_PARENT_CREATE_FAILED"))?;
    let directory = dir
        .canonicalize()
        .map_err(|_| Problem::input("MONTAGE_OUTPUT_PATH_INVALID"))?;
    let name = output
        .file_name()
        .ok_or_else(|| Problem::input("MONTAGE_OUTPUT_PATH_INVALID"))?;
    let output = directory.join(name);
    let stem = output.file_stem().expect("mp4 stem").to_string_lossy();
    for index in 0..10_000 {
        let candidate = if index == 0 {
            output.clone()
        } else {
            directory.join(format!("{stem} ({index}).mp4"))
        };
        match candidate.symlink_metadata() {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(candidate),
            Err(_) => return Err(Problem::input("MONTAGE_OUTPUT_PATH_INVALID")),
            Ok(_) => {}
        }
    }
    Err(Problem::input("MONTAGE_OUTPUT_NAME_EXHAUSTED"))
}

#[cfg(windows)]
fn verify_disk(directory: &Path, required: u64) -> Result<(), Problem> {
    use std::{os::windows::ffi::OsStrExt, ptr};
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<_> = directory.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free = 0_u64;
    let ok =
        unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, ptr::null_mut(), ptr::null_mut()) };
    if ok == 0 {
        return Err(Problem::input("MONTAGE_OUTPUT_SPACE_CHECK_FAILED"));
    }
    if free < required {
        let gib = |bytes| format!("{:.1}", bytes as f64 / 1024_f64.powi(3));
        return Err(Problem(
            StatusCode::BAD_REQUEST,
            "MONTAGE_OUTPUT_DISK_FULL",
            Some(json!({"params":{"required_gb":gib(required),"free_gb":gib(free)}})),
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn verify_disk(_: &Path, _: u64) -> Result<(), Problem> {
    Ok(())
}

fn create(database: &Path, prepared: &Prepared) -> Result<(i64, Value), Box<dyn Error>> {
    fs::create_dir_all(database.parent().ok_or("missing database directory")?)?;
    let mut db = Connection::open(database)?;
    db.busy_timeout(Duration::from_secs(5))?;
    let transaction = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if let Some(id) = prepared.project_id {
        if !has_table(&transaction, "lite_cut_projects")?
            || project_store::get(&transaction, id)?.is_none()
        {
            return Err("requested project disappeared while preparing export".into());
        }
    }
    project_store::ensure_projects(&transaction)?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS lite_cut_exports (
           id INTEGER PRIMARY KEY AUTOINCREMENT, project_id INTEGER, body_json TEXT NOT NULL,
           status TEXT NOT NULL DEFAULT 'pending', error_msg TEXT, output_path TEXT,
           created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
           FOREIGN KEY(project_id) REFERENCES lite_cut_projects(id)
         );
         CREATE INDEX IF NOT EXISTS idx_lite_cut_exports_updated ON lite_cut_exports(updated_at DESC);",
    )?;
    let time = project_store::timestamp(&transaction)?;
    if let (Some(id), Some(name)) = (prepared.project_id, &prepared.project_name) {
        project_store::snapshot(
            &transaction,
            id,
            name,
            &prepared.body,
            "before_export",
            &time,
        )?;
    }
    let output = path_text(&prepared.output);
    transaction.execute(
        "INSERT INTO lite_cut_exports(project_id,body_json,status,error_msg,output_path,created_at,updated_at) VALUES(?1,?2,'queued','',?3,?4,?4)",
        params![prepared.project_id, prepared.body.to_string(), output, time],
    )?;
    let id = transaction.last_insert_rowid();
    transaction.commit()?;
    Ok((
        id,
        json!({
            "export_id": id, "project_id": prepared.project_id, "status": "queued",
            "progress": 0.0, "stage": "queued", "error": "", "output_path": output,
            "created_at": time, "updated_at": time,
        }),
    ))
}

async fn finish(
    profile: &Arc<ProfileState>,
    job: &Job,
    prepared: Prepared,
    signal: watch::Receiver<bool>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<PathBuf, &'static str> {
    let sender = job.cancel.clone();
    let watcher = tokio::spawn(async move {
        let _ = shutdown.wait_for(|value| *value).await;
        sender.send_replace(true);
    });
    job.update(0.01, "starting");
    let result = execute(&prepared, job, &signal).await;
    watcher.abort();

    let (status, error) = match &result {
        Ok(_) => ("done", ""),
        Err(code) if *code == render::CANCELLED => ("cancelled", ""),
        Err(code) => ("error", *code),
    };
    let id = job.id();
    let saved = profile_work(profile, move |profile| -> Result<(), ()> {
        let update = || -> Result<(), Box<dyn Error>> {
            let db = Connection::open(profile.config.database_path())?;
            db.busy_timeout(Duration::from_secs(5))?;
            let time = project_store::timestamp(&db)?;
            let updated = db.execute(
                "UPDATE lite_cut_exports SET status=?1,error_msg=?2,updated_at=?3 WHERE id=?4",
                params![status, error, time, id],
            )?;
            if updated != 1 {
                return Err::<(), Box<dyn Error>>("export row disappeared".into());
            }
            Ok(())
        };
        update().map_err(|_| ())
    })
    .await;
    if !matches!(saved, Ok(Ok(()))) {
        return Err(FAILED);
    }
    result
}

async fn execute(
    prepared: &Prepared,
    job: &Job,
    signal: &watch::Receiver<bool>,
) -> Result<PathBuf, &'static str> {
    let stage = tempfile::Builder::new()
        .prefix("cs2_lite_cut_")
        .tempdir_in(prepared.output.parent().expect("output parent"))
        .map_err(|_| FAILED)?;
    let settings = render::Timeline::from_project(&prepared.body);
    let encoder = select_encoder(&prepared.ffmpeg, &prepared.requested_encoder, signal).await?;
    let media = render::Media {
        ffmpeg: &prepared.ffmpeg,
        ffprobe: &prepared.ffprobe,
        encoder,
        cancel: signal,
    };
    let clips: Vec<_> = prepared
        .clips
        .iter()
        .zip(&prepared.paths)
        .map(|(clip, path)| render::Segment {
            path,
            clip,
            track_gain: prepared.track_gain,
        })
        .collect();
    let base = settings
        .render_base(&media, &clips, stage.path(), |progress, name| {
            job.update(progress, name)
        })
        .await?;
    job.update(0.72, "overlays");
    let base = prepared
        .overlays
        .compose(&media, base, stage.path(), settings.fast)
        .await?;
    job.update(0.84, "audio");
    let mixed = prepared.audio.mix(&media, &stage, &base).await?;
    if mixed != base {
        fs::remove_file(&base).map_err(|_| FAILED)?;
    }
    let path = if let Some(range) = prepared.range {
        job.update(0.98, "range");
        settings
            .trim_range(&media, &mixed, range, stage.path())
            .await?
    } else {
        mixed
    };
    let probe = render::probe(
        &prepared.ffprobe,
        &path,
        signal,
        "MONTAGE_OUTPUT_NOT_PLAYABLE",
    )
    .await?;
    if !list(&probe["streams"])
        .iter()
        .any(|stream| stream["codec_type"] == "video")
    {
        return Err("MONTAGE_OUTPUT_NOT_PLAYABLE");
    }
    let mut state = job.status.lock().expect("LiteCut export progress");
    if *signal.borrow() {
        return Err(render::CANCELLED);
    }
    state.stage = "publishing";
    drop(state);
    tempfile::TempPath::try_from_path(path)
        .map_err(|_| FAILED)?
        .persist_noclobber(&prepared.output)
        .map_err(|_| FAILED)?;
    Ok(prepared.output.clone())
}

async fn select_encoder(
    ffmpeg: &Path,
    requested: &str,
    cancel: &watch::Receiver<bool>,
) -> Result<&'static str, &'static str> {
    let hardware = ["h264_nvenc", "h264_qsv", "h264_amf"];
    let choices = hardware
        .into_iter()
        .filter(|codec| requested == "auto" || requested == *codec)
        .chain(["libx264"]);
    for codec in choices {
        let mut process = render::render_command(ffmpeg);
        let (source, count) = if codec == "h264_amf" {
            ("testsrc2=s=1920x1080:r=60:d=1", "60")
        } else {
            ("testsrc2=s=320x240:r=1:d=1", "1")
        };
        process
            .args(["-f", "lavfi", "-i", source, "-frames:v", count, "-an"])
            .args(render::encoder_args(codec, true))
            .args(["-f", "null", "-"]);
        match render::run(process, cancel, FAILED, Duration::from_secs(45)).await {
            Ok(()) => return Ok(codec),
            Err(render::CANCELLED) => return Err(render::CANCELLED),
            Err(_) => {}
        }
    }
    Err("MONTAGE_ENCODER_NOT_AVAILABLE")
}

fn export_failed() -> HttpResponse {
    Problem(StatusCode::INTERNAL_SERVER_ERROR, FAILED, None).response()
}
