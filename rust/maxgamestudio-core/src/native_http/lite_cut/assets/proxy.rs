//! Generate legacy LiteCut preview files in the background.

use super::{
    super::{
        ffmpeg::{command, find_ffprobe, inspect, video_has_alpha},
        render::{run, CANCELLED},
    },
    asset_root, cached_proxy,
};
use crate::{
    native_http::{profile_work, ProfileState},
    profile_platform::path_text,
};
use futures_util::future::{select, Either};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    process::Command,
    sync::{watch, Semaphore},
    task::JoinSet,
    time::timeout,
};

const MAX_PENDING: usize = 32;
const MAX_FAILURES: usize = 1024;
const MAX_ENCODE_TIME: Duration = Duration::from_secs(3600);
const FAILED: &str = "代理生成失败，请重试";
const MEDIA_TOOL_MISSING: &str = "未找到 FFmpeg 或 FFprobe，无法生成素材预览";
const STORAGE_CHANGED: &str = "素材或存储位置已变更，请重试";

pub(super) struct Source {
    pub(super) root: PathBuf,
    pub(super) path: PathBuf,
    pub(super) duration: Option<f64>,
}

#[derive(Clone, Copy)]
pub(super) struct Snapshot {
    pub(super) status: &'static str,
    pub(super) message: &'static str,
    mode: &'static str,
    alpha: bool,
}

impl Snapshot {
    const QUEUED: Self = Self {
        status: "queued",
        message: "",
        mode: "queued",
        alpha: false,
    };

    fn running(mode: &'static str, alpha: bool) -> Self {
        Self {
            status: "running",
            message: "",
            mode,
            alpha,
        }
    }

    fn failed(message: &'static str) -> Self {
        Self {
            status: "failed",
            message,
            mode: "failed",
            alpha: false,
        }
    }

    pub(super) fn apply(self, row: &mut Value) {
        row["preview_proxy_required"] = json!(true);
        row["preview_proxy_status"] = json!(self.status);
        row["preview_proxy_error"] = json!(self.message);
        row["preview_proxy_version"] = json!(self.status);
        row["preview_proxy_mode"] = json!(self.mode);
        row["has_alpha"] = json!(self.alpha || row["has_alpha"] == true);
    }
}

pub(in crate::native_http) struct ActiveProxies {
    state: Mutex<State>,
    serial: Semaphore,
}

#[derive(Default)]
struct State {
    stopping: bool,
    active: HashMap<PathBuf, Active>,
    deleting: HashSet<PathBuf>,
    failed: VecDeque<(PathBuf, Snapshot)>,
    tasks: JoinSet<()>,
}

struct Active {
    source: PathBuf,
    status: Snapshot,
    cancel: watch::Sender<bool>,
    done: watch::Receiver<bool>,
}

pub(super) struct Deleting {
    profile: Arc<ProfileState>,
    key: PathBuf,
    done: Option<watch::Receiver<bool>>,
}

impl Deleting {
    pub(super) async fn wait(&mut self) -> Result<(), &'static str> {
        if let Some(done) = &mut self.done {
            match timeout(Duration::from_secs(12), done.wait_for(|done| *done)).await {
                Ok(Ok(_)) => (),
                _ => return Err("素材预览仍在停止，请稍后重试"),
            }
        }
        Ok(())
    }
}

impl Drop for Deleting {
    fn drop(&mut self) {
        let mut state = self.profile.lite_cut_proxies.state.lock().expect("LiteCut proxy jobs");
        state.deleting.remove(&self.key);
    }
}

impl Default for ActiveProxies {
    fn default() -> Self {
        Self {
            state: Mutex::new(State::default()),
            serial: Semaphore::new(1),
        }
    }
}

impl ActiveProxies {
    pub(super) fn schedule(
        &self,
        profile: &Arc<ProfileState>,
        source: Source,
        retry: bool,
    ) -> Snapshot {
        // Legacy proxy names share a stem across supported source containers.
        let key = source.path.with_extension("preview60.mp4");
        let mut state = self.state.lock().expect("LiteCut proxy jobs");
        while state.tasks.try_join_next().is_some() {}
        if state.stopping {
            return Snapshot::failed(FAILED);
        }
        if state.deleting.contains(&key) {
            return Snapshot::QUEUED;
        }
        if let Some(active) = state.active.get(&key) {
            return active.status;
        }
        if let Some(index) = state.failed.iter().position(|(path, _)| path == &key) {
            if !retry {
                return state.failed[index].1;
            }
            state.failed.remove(index);
        }
        if state.active.len() >= MAX_PENDING {
            // The media bin polls queued entries. Admit its next page entries
            // as running work completes without allocating one task per row.
            return Snapshot::QUEUED;
        }
        let (cancel, cancelled) = watch::channel(false);
        let (done, completion) = watch::channel(false);
        state.active.insert(key.clone(), Active {
            source: source.path.clone(),
            status: Snapshot::QUEUED,
            cancel,
            done: completion,
        });
        let profile = Arc::clone(profile);
        state.tasks.spawn(async move {
            let result = generate(&profile, &key, source, cancelled).await;
            let mut state = profile.lite_cut_proxies.state.lock().expect("LiteCut proxy jobs");
            let cancelled = state.active.remove(&key).is_some_and(|job| {
                let requested = *job.cancel.borrow();
                requested
            });
            if let Some(message) = result.err().filter(|_| !cancelled) {
                if state.failed.len() >= MAX_FAILURES {
                    state.failed.pop_front();
                }
                state.failed.push_back((key, Snapshot::failed(message)));
            }
            done.send_replace(true);
        });
        Snapshot::QUEUED
    }

    pub(super) fn stop_for_deletion(
        &self,
        profile: &Arc<ProfileState>,
        source: &Path,
    ) -> Result<Deleting, &'static str> {
        let key = source.with_extension("preview60.mp4");
        let mut state = self.state.lock().expect("LiteCut proxy jobs");
        if state.stopping || !state.deleting.insert(key.clone()) {
            return Err("素材正在移除，请稍后重试");
        }
        let done = state.active.get(&key)
            .filter(|active| active.source == source)
            .map(|active| {
                active.cancel.send_replace(true);
                active.done.clone()
            });
        Ok(Deleting {
            profile: Arc::clone(profile),
            key,
            done,
        })
    }

    fn running(&self, key: &Path, mode: &'static str, alpha: bool) {
        if let Some(job) = self.state.lock().expect("LiteCut proxy jobs").active.get_mut(key) {
            job.status = Snapshot::running(mode, alpha);
        }
    }

    pub(in crate::native_http) async fn shutdown(&self) {
        let mut tasks = {
            let mut state = self.state.lock().expect("LiteCut proxy jobs");
            state.stopping = true;
            for job in state.active.values() {
                job.cancel.send_replace(true);
            }
            std::mem::take(&mut state.tasks)
        };
        if timeout(Duration::from_secs(12), async {
            while tasks.join_next().await.is_some() {}
        })
        .await
        .is_err()
        {
            tasks.shutdown().await;
        }
    }
}

async fn generate(
    profile: &Arc<ProfileState>,
    key: &Path,
    source: Source,
    cancel: watch::Receiver<bool>,
) -> Result<(), &'static str> {
    // Only one proxy waits for the shared media lane at a time, so a page
    // of old assets cannot queue ahead of a user's interactive media request.
    let acquire = async {
        let serial = profile.lite_cut_proxies.serial.acquire().await?;
        let media = profile.media_process.acquire().await?;
        Ok::<_, tokio::sync::AcquireError>((serial, media))
    };
    let mut waiting_cancel = cancel.clone();
    let (_serial, _media) = match select(
        Box::pin(acquire),
        Box::pin(waiting_cancel.wait_for(|cancel| *cancel)),
    )
    .await
    {
        Either::Left((Ok(permits), _)) if !*cancel.borrow() => permits,
        _ => return Err(FAILED),
    };
    profile.lite_cut_proxies.running(key, "probing", false);

    let settings = profile_work(profile, |profile| {
        let ffmpeg = profile.config.ffmpeg_executable().map_err(|_| MEDIA_TOOL_MISSING)?;
        let ffprobe = find_ffprobe(&ffmpeg).ok_or(MEDIA_TOOL_MISSING)?;
        let resolution = profile.config.lite_cut_proxy_resolution().map_err(|_| FAILED)?;
        let root = asset_root(profile).map_err(|_| STORAGE_CHANGED)?;
        Ok::<_, &'static str>((ffmpeg, ffprobe, resolution, root))
    })
    .await
    .map_err(|_| FAILED)??;
    let (ffmpeg, ffprobe, edge, current_root) = settings;
    if current_root != source.root || !valid_source(&source) {
        return Err(STORAGE_CHANGED);
    }
    let original = source_stamp(&source.path).map_err(|_| STORAGE_CHANGED)?;
    if cached_proxy(&source.root, &source.path).is_some() {
        return Ok(());
    }

    let info = inspect(&ffprobe, &source.path, Some(&cancel)).await.map_err(|_| FAILED)?;
    let mut plan = Encoding::from_probe(&source, &info)?;
    let parent = source.path.parent().ok_or(STORAGE_CHANGED)?;
    let temp = tempfile::Builder::new()
        .prefix(".litecut-preview-")
        .suffix(".tmp")
        .tempfile_in(parent)
        .map_err(|_| FAILED)?
        .into_temp_path();
    let output = source.path.with_extension(plan.mode.extension());
    loop {
        profile.lite_cut_proxies.running(key, plan.mode.name(), plan.mode == Mode::Alpha);
        let process = plan.command(&ffmpeg, &source.path, &temp, edge);
        let valid = match run(process, &cancel, FAILED, MAX_ENCODE_TIME).await {
            Ok(()) => inspect(&ffprobe, &temp, Some(&cancel))
                .await
                .is_ok_and(|info| plan.valid_output(&info)),
            Err(CANCELLED) => return Err(FAILED),
            Err(_) => false,
        };
        if *cancel.borrow() {
            return Err(FAILED);
        }
        if valid {
            break;
        }
        if plan.mode != Mode::Remux {
            return Err(FAILED);
        }
        plan.mode = Mode::Transcode;
        plan.copy_audio = false;
    }
    profile_work(profile, move |profile| {
        let current_root = asset_root(profile).map_err(|_| STORAGE_CHANGED)?;
        if current_root != source.root || !valid_source(&source) || *cancel.borrow()
            || source_stamp(&source.path).map_err(|_| STORAGE_CHANGED)? != original
        {
            return Err(STORAGE_CHANGED);
        }
        match temp.persist_noclobber(&output) {
            Ok(()) => Ok(()),
            Err(_) if cached_proxy(&source.root, &source.path).is_some() => Ok(()),
            Err(_) => Err(FAILED),
        }
    })
    .await
    .map_err(|_| FAILED)?
}

fn source_stamp(path: &Path) -> std::io::Result<(u64, std::time::SystemTime)> {
    let metadata = fs::metadata(path)?;
    Ok((metadata.len(), metadata.modified()?))
}

fn valid_source(source: &Source) -> bool {
    source.path.starts_with(&source.root)
        && source.path.is_file()
        && source.path.canonicalize().is_ok_and(|path| path == source.path)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Remux,
    Transcode,
    Alpha,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::Remux => "remux",
            Self::Transcode => "transcode",
            Self::Alpha => "alpha_transcode",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Remux | Self::Transcode => "preview60.mp4",
            Self::Alpha => "preview-alpha-v3.webm",
        }
    }
}

struct Encoding {
    mode: Mode,
    duration: Option<f64>,
    audio_count: usize,
    copy_audio: bool,
    alpha_vp9_input: bool,
}

impl Encoding {
    fn from_probe(source: &Source, info: &Value) -> Result<Self, &'static str> {
        let streams = info["streams"].as_array().ok_or(FAILED)?;
        let video = streams.iter().find(|stream| stream["codec_type"] == "video").ok_or(FAILED)?;
        let extension = source.path.extension().and_then(|ext| ext.to_str()).unwrap_or_default();
        let alpha = !extension.eq_ignore_ascii_case("gif") && video_has_alpha(video);
        let codec = video["codec_name"].as_str().unwrap_or_default();
        let pixels = video["pix_fmt"].as_str().unwrap_or_default();
        let mode = if alpha {
            Mode::Alpha
        } else if codec == "h264" && matches!(pixels, "nv12" | "yuv420p" | "yuvj420p") {
            Mode::Remux
        } else {
            Mode::Transcode
        };
        let mut audio = streams.iter().filter(|stream| stream["codec_type"] == "audio");
        let duration = source.duration
            .filter(|duration| duration.is_finite() && *duration > 0.0)
            .or_else(|| duration(info));
        Ok(Self {
            mode,
            duration: if alpha { duration.map(|seconds| seconds.min(600.0)) } else { duration },
            audio_count: audio.clone().count(),
            copy_audio: audio.all(|stream| stream["codec_name"] == "aac"),
            alpha_vp9_input: alpha && codec == "vp9",
        })
    }

    fn command(&self, ffmpeg: &Path, source: &Path, output: &Path, edge: u64) -> Command {
        let mut process = command(ffmpeg);
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Threading::{BELOW_NORMAL_PRIORITY_CLASS, CREATE_NO_WINDOW};
            process.creation_flags(BELOW_NORMAL_PRIORITY_CLASS | CREATE_NO_WINDOW);
        }
        process.args(["-nostdin", "-y", "-hide_banner", "-loglevel", "error", "-filter_threads", "1"]);
        if self.alpha_vp9_input {
            process.args(["-c:v", "libvpx-vp9"]);
        }
        process.args(["-threads", "2", "-protocol_whitelist", "file,pipe", "-i"])
            .arg(path_text(source));
        if let Some(duration) = self.duration {
            process.args(["-t", &format!("{duration:.6}")]);
        }
        process.args(["-map", "0:v:0", "-map", "0:a?"]);
        if self.mode == Mode::Remux {
            process.args(["-c:v", "copy", "-avoid_negative_ts", "make_zero"]);
        } else {
            // Preserve aspect ratio, never upscale normal sources, and keep
            // both dimensions valid for chroma-subsampled preview encoders.
            let edge = edge.clamp(360, 2160);
            let mut scale = format!(
                "scale=w='if(gte(iw,ih),max(2,trunc(min({edge},iw)/2)*2),-2)':h='if(gte(iw,ih),-2,max(2,trunc(min({edge},ih)/2)*2))'"
            );
            if self.mode == Mode::Alpha {
                scale.push_str(",format=yuva420p");
                process.args([
                    "-vf", &scale, "-c:v", "libvpx-vp9", "-pix_fmt", "yuva420p",
                    "-auto-alt-ref", "0", "-deadline", "good", "-cpu-used", "5", "-row-mt", "1",
                    "-threads", "2", "-b:v", "0", "-crf", "28", "-fpsmax", "30",
                    "-metadata:s:v:0", "alpha_mode=1",
                ]);
            } else {
                process.args([
                    "-vf", &scale, "-c:v", "libx264", "-preset", "faster", "-crf", "20",
                    "-threads", "2", "-pix_fmt", "yuv420p", "-fpsmax", "60", "-g", "30",
                    "-force_key_frames", "expr:gte(t,n_forced*0.5)",
                ]);
            }
        }
        if self.mode == Mode::Alpha {
            process.args(["-c:a", "libopus", "-b:a", "128k", "-f", "webm"]);
        } else {
            if self.copy_audio {
                process.args(["-c:a", "copy"]);
            } else {
                process.args(["-c:a", "aac", "-b:a", "96k"]);
            }
            process.args(["-movflags", "+faststart", "-f", "mp4"]);
        }
        process.arg(path_text(output));
        process
    }

    fn valid_output(&self, info: &Value) -> bool {
        let Some(streams) = info["streams"].as_array() else {
            return false;
        };
        if streams.iter().filter(|stream| stream["codec_type"] == "audio").count() != self.audio_count
            || duration(info).is_none()
        {
            return false;
        }
        streams.iter().find(|stream| stream["codec_type"] == "video").is_some_and(|video| {
            if self.mode == Mode::Alpha {
                video["codec_name"] == "vp9" && video_has_alpha(video)
            } else {
                video["codec_name"] == "h264"
            }
        })
    }
}

fn duration(probe: &Value) -> Option<f64> {
    probe["format"]["duration"].as_str()?.parse::<f64>().ok()
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(name: &str) -> Source {
        Source {
            root: PathBuf::from("media"),
            path: Path::new("media").join(name),
            duration: None,
        }
    }

    fn arguments(plan: &Encoding, source: &Path, edge: u64) -> Vec<String> {
        plan.command(Path::new("ffmpeg"), source, Path::new("staged.tmp"), edge)
            .as_std()
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect()
    }

    fn has_pair(args: &[String], flag: &str, value: &str) -> bool {
        args.windows(2).any(|pair| pair[0] == flag && pair[1] == value)
    }

    #[test]
    fn preview_losslessly_remuxes_supported_h264_and_converts_incompatible_audio() {
        let source = source("recording.mkv");
        let mut info = json!({
            "format":{"duration":"6.0"},
            "streams":[
                {"codec_type":"video","codec_name":"h264","pix_fmt":"yuv420p"},
                {"codec_type":"audio","codec_name":"aac"}
            ]
        });
        let plan = Encoding::from_probe(&source, &info).unwrap();
        let args = arguments(&plan, &source.path, 720);
        assert!(matches!(plan.mode, Mode::Remux));
        assert!(has_pair(&args, "-c:v", "copy"));
        assert!(has_pair(&args, "-c:a", "copy"));
        assert!(plan.valid_output(&info));
        info["streams"][1]["codec_name"] = json!("opus");
        let plan = Encoding::from_probe(&source, &info).unwrap();
        let args = arguments(&plan, &source.path, 720);
        assert!(has_pair(&args, "-c:v", "copy"));
        assert!(has_pair(&args, "-c:a", "aac"));

        info["streams"][0]["pix_fmt"] = json!("yuv420p10le");
        let plan = Encoding::from_probe(&source, &info).unwrap();
        let args = arguments(&plan, &source.path, 1080);
        assert!(has_pair(&args, "-c:v", "libx264"));
        assert!(args.iter().any(|arg| arg.contains("min(1080,iw)")));
        assert!(has_pair(&args, "-pix_fmt", "yuv420p"));
    }

    #[test]
    fn preview_detects_transparent_vp9_and_requires_alpha_in_the_rendered_proxy() {
        let source = source("overlay.mkv");
        let mut info = json!({
            "format":{"duration":"605.0"},
            "streams":[{"codec_type":"video","codec_name":"vp9","pix_fmt":"yuv420p",
                        "tags":{"ALPHA_MODE":"1"}}]
        });
        let plan = Encoding::from_probe(&source, &info).unwrap();
        assert!(matches!(plan.mode, Mode::Alpha));
        assert_eq!(plan.duration, Some(600.0));
        let args = arguments(&plan, &source.path, 720);
        assert_eq!(args.windows(2).filter(|pair| pair[0] == "-c:v" && pair[1] == "libvpx-vp9").count(), 2);
        assert!(has_pair(&args, "-f", "webm"));
        assert!(plan.valid_output(&info));
        info["streams"][0]["tags"] = json!({});
        assert!(!plan.valid_output(&info));
    }
}
