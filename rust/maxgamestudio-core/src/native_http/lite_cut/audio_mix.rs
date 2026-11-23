//! Mix the rendered full timeline with the project's remaining audio sources.

use super::{
    export_sources::{base_track, main_clip, media_path},
    ffmpeg::command,
    project_schema::list,
    render::{self, number, Media},
};
use crate::profile_platform::path_text;
use serde_json::Value;
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use tempfile::TempDir;

const FAILED: &str = "MONTAGE_EXPORT_FAILED";
const SOURCE_MISSING: &str = "MONTAGE_CLIP_FILE_MISSING";
const SOURCE_UNREADABLE: &str = "MONTAGE_SOURCE_NOT_READABLE";
const EPSILON: f64 = 1e-6;
const AUDIO_EXTENSIONS: &[&str] = &["mp3", "wav", "m4a", "aac", "ogg", "flac"];

pub(super) struct Plan {
    clips: Vec<Clip>,
    master: f64,
}

struct Clip {
    path: PathBuf,
    start: f64,
    segments: Vec<SpeedSegment>,
    has_ramp: bool,
    preserve_pitch: bool,
    reverse: bool,
    gain: String,
    fade_in: f64,
    fade_out: f64,
    duration: f64,
    kind: Kind,
}

pub(super) struct SpeedSegment {
    pub(super) start: f64,
    pub(super) end: f64,
    pub(super) speed: f64,
}

#[derive(Clone, Copy)]
enum Kind {
    Foreground,
    Bgm { ducking: Option<f64> },
}

impl Plan {
    /// Collect sources from a normalized project. Resolve recorded upper video
    /// layers with the same paths as the caller's visual compositor.
    pub(super) fn from_project(
        project: &Value,
        mut recording_path: impl FnMut(i64) -> Option<PathBuf>,
    ) -> Self {
        let tracks = list(&project["tracks"]);
        let solo = tracks
            .iter()
            .any(|track| track["type"] == "audio" && track["solo"] == true);
        let mut clips = Vec::new();
        for track in tracks.iter().filter(|track| {
            track["type"] == "audio"
                && track["hidden"] == false
                && track["muted"] == false
                && (!solo || track["solo"] == true)
        }) {
            let gain = number(&track["volume"]).min(2.0);
            for clip in list(&track["clips"]) {
                if clip["source_type"] != "file" {
                    continue;
                }
                let Some(path) = source_path(&clip["file_path"]) else {
                    continue;
                };
                let audio_extension =
                    path.extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| {
                            AUDIO_EXTENSIONS
                                .iter()
                                .any(|item| ext.eq_ignore_ascii_case(item))
                        });
                if audio_extension || clip["meta"]["kind"] == "audio" {
                    clips.push(Clip::foreground(clip, path, gain, false));
                }
            }
        }
        clips.sort_by(|left, right| left.start.total_cmp(&right.start));

        let mut layers = Vec::new();
        for track in tracks[..base_track(tracks).unwrap_or(tracks.len())]
            .iter()
            .filter(|track| track["type"] == "video" && track["hidden"] == false)
        {
            let gain = number(&track["volume"]).min(2.0);
            for clip in list(&track["clips"]).iter().filter(|clip| main_clip(clip)) {
                let path = source_path(&clip["file_path"])
                    .or_else(|| recording_path(clip["source_id"].as_i64()?));
                if let Some(path) = path {
                    layers.push(Clip::foreground(
                        clip,
                        path,
                        gain,
                        solo || track["muted"] == true,
                    ));
                }
            }
        }
        layers.sort_by(|left, right| left.start.total_cmp(&right.start));
        clips.extend(layers);

        let bgm = &project["audio"]["bgm"];
        if !solo {
            if let Some(path) = source_path(&bgm["path"]) {
                clips.push(Clip::bgm(bgm, path));
            }
        }
        Self {
            clips,
            master: number(&project["audio"]["master_volume"]).min(2.0),
        }
    }

    pub(super) fn sources(&self) -> impl Iterator<Item = &Path> {
        self.clips.iter().map(|clip| clip.path.as_path())
    }

    /// Keep the returned media in the export's owned staging directory until
    /// range trimming or the caller's no-clobber publish completes.
    pub(super) async fn mix(
        &self,
        media: &Media<'_>,
        stage: &TempDir,
        base: &Path,
    ) -> Result<PathBuf, &'static str> {
        if *media.cancel.borrow() {
            return Err(render::CANCELLED);
        }
        if self.clips.is_empty() && (self.master - 1.0).abs() <= EPSILON {
            return Ok(base.to_owned());
        }
        let inputs = self
            .sources()
            .map(resolve_source)
            .collect::<Result<Vec<_>, _>>()?;
        let base_input = base.canonicalize().map_err(|_| FAILED)?;
        let streams = render::probe(media.ffprobe, &base_input, media.cancel, FAILED).await?;
        let has_base_audio = list(&streams["streams"])
            .iter()
            .any(|stream| stream["codec_type"] == "audio");
        let graph = self.graph(has_base_audio);
        if graph.is_empty() {
            return Ok(base.to_owned());
        }

        const GRAPH: &str = "audio-mix.ffgraph";
        const OUTPUT: &str = "audio-mix.mp4";
        match stage.path().join(OUTPUT).symlink_metadata() {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(FAILED),
        }
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(stage.path().join(GRAPH))
            .and_then(|mut file| file.write_all(graph.as_bytes()))
            .map_err(|_| FAILED)?;
        let mut process = command(media.ffmpeg);
        process
            .current_dir(stage.path())
            .args(["-nostdin", "-n", "-hide_banner", "-loglevel", "error", "-i"])
            .arg(path_text(&base_input));
        for path in inputs {
            process.arg("-i").arg(path_text(&path));
        }
        process.args([
            "-filter_complex_script",
            GRAPH,
            "-map",
            "0:v:0",
            "-map",
            "[mixa]",
            "-c:v",
            "copy",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-ar",
            "48000",
            "-ac",
            "2",
            "-shortest",
            OUTPUT,
        ]);
        render::run(process, media.cancel, FAILED, Duration::from_secs(3600)).await?;
        let output = stage.path().join(OUTPUT);
        if fs::metadata(&output).is_ok_and(|meta| meta.is_file() && meta.len() > 0) {
            Ok(output)
        } else {
            Err(FAILED)
        }
    }

    fn graph(&self, has_base_audio: bool) -> String {
        let mut parts = Vec::new();
        let mut foreground = Vec::new();
        if has_base_audio {
            parts.push("[0:a:0]asetpts=PTS-STARTPTS[basea]".to_owned());
            foreground.push("[basea]".to_owned());
        }
        let mut bgm = None;
        for (index, clip) in (1..).zip(&self.clips) {
            let output = clip.append_filters(index, &mut parts);
            match clip.kind {
                Kind::Foreground => foreground.push(output),
                Kind::Bgm { ducking } => bgm = Some((output, ducking)),
            }
        }
        if let Some((label, ducking)) = bgm {
            if let Some(volume) = ducking.filter(|_| !foreground.is_empty()) {
                mix_labels(&mut parts, &foreground, "[duckforeground]");
                parts.push("[duckforeground]asplit=2[foreground][duckside]".to_owned());
                let ratio = 1.0 + (1.0 - volume) * 18.0;
                parts.push(format!(
                    "{label}[duckside]sidechaincompress=threshold=0.015:ratio={ratio:.6}:attack=25:release=280[bgmduck]"
                ));
                foreground = vec!["[foreground]".to_owned(), "[bgmduck]".to_owned()];
            } else {
                foreground.push(label);
            }
        }
        if foreground.is_empty() {
            return String::new();
        }
        mix_labels(&mut parts, &foreground, "[premaster]");
        if (self.master - 1.0).abs() > EPSILON {
            parts.push(format!("[premaster]volume={:.6}[mixa]", self.master));
        } else {
            parts.push("[premaster]anull[mixa]".to_owned());
        }
        parts.join(";")
    }
}

impl Clip {
    fn foreground(clip: &Value, path: PathBuf, track_gain: f64, force_muted: bool) -> Self {
        let start = number(&clip["trim_in"]);
        let source_duration = finite_number(&clip["trim_out"])
            .or_else(|| finite_number(&clip["meta"]["duration_sec"]))
            .map_or(5.0, |end| (end - start).max(0.1));
        let end = start + source_duration;
        let speed = number(&clip["speed"]);
        let mut segments = speed_ramp(clip, start, end);
        let has_ramp = !segments.is_empty();
        if !has_ramp {
            segments.push(SpeedSegment { start, end, speed });
        }
        let duration = segments
            .iter()
            .map(|part| (part.end - part.start) / part.speed)
            .sum::<f64>()
            .max(0.1)
            + number(&clip["freeze_frame_sec"]);
        let gain = if force_muted {
            "volume=0.000000".to_owned()
        } else {
            clip_gain_filter(clip, track_gain, duration)
        };
        Self {
            path,
            start: number(&clip["timeline_start"]),
            segments,
            has_ramp,
            preserve_pitch: clip["preserve_pitch"] == true,
            reverse: clip["reverse"] == true,
            gain,
            fade_in: number(&clip["fade_in_sec"])
                .min(source_duration)
                .min(duration),
            fade_out: number(&clip["fade_out_sec"])
                .min(source_duration)
                .min(duration),
            duration,
            kind: Kind::Foreground,
        }
    }

    fn bgm(bgm: &Value, path: PathBuf) -> Self {
        let duration = finite_number(&bgm["duration_sec"])
            .filter(|seconds| *seconds > 0.0)
            .map_or(5.0, |seconds| seconds.max(0.1));
        let ducking = (bgm["ducking_enabled"] == true)
            .then(|| number(&bgm["ducking_volume"]).clamp(0.05, 1.0));
        Self {
            path,
            start: number(&bgm["start_sec"]),
            segments: vec![SpeedSegment {
                start: 0.0,
                end: duration,
                speed: 1.0,
            }],
            has_ramp: false,
            preserve_pitch: true,
            reverse: false,
            gain: format!("volume={:.6}", number(&bgm["volume"])),
            fade_in: number(&bgm["fade_in_sec"]).min(duration),
            fade_out: number(&bgm["fade_out_sec"]).min(duration),
            duration,
            kind: Kind::Bgm { ducking },
        }
    }

    fn append_filters(&self, index: usize, parts: &mut Vec<String>) -> String {
        let (input, mut chain) = if !self.has_ramp {
            (
                format!("[{index}:a:0]"),
                self.segments[0].filters(self.reverse, self.preserve_pitch, false),
            )
        } else {
            let mut labels = String::new();
            for (part_index, segment) in self.segments.iter().enumerate() {
                let label = format!("[ars{index}_{part_index}]");
                let filters = segment.filters(false, self.preserve_pitch, true).join(",");
                parts.push(format!("[{index}:a:0]{filters}{label}"));
                labels.push_str(&label);
            }
            let ramp_label = format!("[arr{index}]");
            parts.push(format!(
                "{labels}concat=n={}:v=0:a=1{ramp_label}",
                self.segments.len()
            ));
            let reverse = if self.reverse {
                vec!["areverse".to_owned()]
            } else {
                Vec::new()
            };
            (ramp_label, reverse)
        };
        chain.push(self.gain.clone());
        if self.fade_in > 0.0 {
            chain.push(format!("afade=t=in:st=0:d={:.6}", self.fade_in));
        }
        if self.fade_out > 0.0 {
            chain.push(format!(
                "afade=t=out:st={:.6}:d={:.6}",
                self.duration - self.fade_out,
                self.fade_out
            ));
        }
        let delay = (self.start * 1000.0).round_ties_even() as u64;
        chain.push(format!("adelay={delay}:all=1"));
        let output = format!("[a{index}]");
        parts.push(format!("{input}{}{output}", chain.join(",")));
        output
    }
}

impl SpeedSegment {
    pub(super) fn filters(&self, reverse: bool, preserve_pitch: bool, has_ramp: bool) -> Vec<String> {
        let mut filters = vec![
            format!("atrim=start={:.6}:end={:.6}", self.start, self.end),
            "asetpts=PTS-STARTPTS".to_owned(),
        ];
        if reverse {
            filters.push("areverse".to_owned());
        }
        if !has_ramp && (self.speed - 1.0).abs() <= EPSILON {
            return filters;
        }
        if preserve_pitch {
            let first = self.speed.clamp(0.5, 2.0);
            filters.push(format!("atempo={first:.6}"));
            let second = self.speed / first;
            if (second - 1.0).abs() > EPSILON {
                filters.push(format!("atempo={second:.6}"));
            }
        } else {
            filters.push(format!(
                "aresample=48000,asetrate={:.6},aresample=48000",
                48_000.0 * self.speed
            ));
        }
        filters
    }
}

pub(super) fn speed_ramp(clip: &Value, start: f64, end: f64) -> Vec<SpeedSegment> {
    let mut points = keyframes(&clip["speed_keyframes"], "source_sec", "speed", start..=end);
    if points.len() < 2 {
        return Vec::new();
    }
    if points[0].0 > start + EPSILON {
        points.insert(0, (start, number(&clip["speed"])));
    }
    let (last_time, last_speed) = points[points.len() - 1];
    if last_time < end - EPSILON {
        points.push((end, last_speed));
    }
    points.windows(2).map(|pair| SpeedSegment {
        start: pair[0].0, end: pair[1].0, speed: pair[0].1,
    }).collect()
}

fn source_path(value: &Value) -> Option<PathBuf> {
    let path = value.as_str()?.trim();
    (!path.is_empty()).then(|| media_path(path))
}

fn resolve_source(path: &Path) -> Result<PathBuf, &'static str> {
    fn source_error(error: io::Error) -> &'static str {
        if error.kind() == io::ErrorKind::NotFound {
            SOURCE_MISSING
        } else {
            SOURCE_UNREADABLE
        }
    }

    let path = path.canonicalize().map_err(source_error)?;
    let info = fs::metadata(&path).map_err(source_error)?;
    if !info.is_file() {
        return Err(SOURCE_UNREADABLE);
    }
    fs::File::open(&path).map_err(source_error)?;
    Ok(path)
}

fn finite_number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse().ok())
        .filter(|number| number.is_finite())
}

fn keyframes(
    values: &Value,
    time: &str,
    amount: &str,
    bounds: std::ops::RangeInclusive<f64>,
) -> Vec<(f64, f64)> {
    let mut points: Vec<_> = list(values)
        .iter()
        .filter_map(|point| {
            Some((
                finite_number(&point[time])?.clamp(*bounds.start(), *bounds.end()),
                finite_number(&point[amount])?,
            ))
        })
        .collect();
    points.sort_by(|left, right| left.0.total_cmp(&right.0));
    points.dedup_by(|later, earlier| {
        if (later.0 - earlier.0).abs() <= EPSILON {
            *earlier = *later;
            true
        } else {
            false
        }
    });
    points
}

pub(super) fn clip_gain_filter(clip: &Value, track_gain: f64, duration: f64) -> String {
    if clip["muted"] == true {
        return "volume=0.000000".to_owned();
    }
    let mut points = keyframes(&clip["audio_keyframes"], "time_sec", "volume", 0.0..=duration);
    for (_, gain) in &mut points {
        *gain = (*gain * track_gain).min(2.0);
    }
    gain_filter((number(&clip["volume"]) * track_gain).min(5.0), &points)
}

fn gain_filter(gain: f64, points: &[(f64, f64)]) -> String {
    let Some(&(first, initial)) = points.first() else {
        return format!("volume={gain:.6}");
    };
    let &(last, final_gain) = &points[points.len() - 1];
    let mut expression = format!("{final_gain:.6}");
    if points.len() > 1 {
        // Keep expression recursion logarithmic even with the 500 legal keyframes.
        let linear = super::curve::interpolate(points, "t");
        expression = format!("if(lt(t,{last:.6}),{linear},{expression})");
    }
    if first > EPSILON {
        expression = format!("if(lt(t,{first:.6}),{initial:.6},{expression})");
    }
    format!("volume='{expression}':eval=frame")
}

fn mix_labels(parts: &mut Vec<String>, labels: &[String], output: &str) {
    let inputs = labels.len();
    let filter = if inputs == 1 {
        "anull".to_owned()
    } else {
        format!("amix=inputs={inputs}:duration=longest:dropout_transition=0")
    };
    parts.push(format!("{}{filter}{output}", labels.concat()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_http::lite_cut::project_schema::normalize;
    use serde_json::json;

    #[test]
    fn solo_tracks_match_the_release_even_when_the_solo_track_is_hidden() {
        let project = normalize(Some(json!({
            "tracks": [
                {"id":"solo","type":"audio","label":"solo","solo":true,"hidden":true},
                {"id":"plain","type":"audio","label":"plain","clips":[
                    {"id":"normal","source_type":"file","file_path":"normal.wav"}
                ]}
            ],
            "audio":{"bgm":{"path":"bgm.wav"}}
        })))
        .unwrap();
        let plan = Plan::from_project(&project, |_| None);
        assert!(plan.clips.is_empty());
    }

    #[test]
    fn trims_once_and_applies_keyframe_track_gain_before_fades_and_master() {
        let project = normalize(Some(json!({
            "tracks":[{"id":"a","type":"audio","label":"a","volume":2,"clips":[{
                "id":"1","source_type":"file","file_path":"voice.wav","trim_in":0.5,"trim_out":2.5,
                "timeline_start":1.25,"speed":2.0,"fade_out_sec":0.25,
                "audio_keyframes":[{"time_sec":"0","volume":"0.2"},{"time_sec":1.0,"volume":3}]
            }]}],
            "audio":{"master_volume":4}
        })))
        .unwrap();
        let plan = Plan::from_project(&project, |_| None);
        let graph = plan.graph(true);
        assert!(graph
            .contains("atrim=start=0.500000:end=2.500000,asetpts=PTS-STARTPTS,atempo=2.000000"));
        assert!(graph.contains("0.400000+(2.000000-0.400000)"));
        assert!(graph.contains(":eval=frame,afade=t=out:st=0.750000:d=0.250000,adelay=1250:all=1"));
        assert!(graph.ends_with("[premaster]volume=2.000000[mixa]"));
        assert!(!graph.contains("voice.wav"));
    }

    #[test]
    fn even_one_ramped_interval_reverses_after_speed_processing() {
        let project = normalize(Some(json!({
            "tracks":[{"id":"a","type":"audio","label":"a","clips":[{
                "id":"1","source_type":"file","file_path":"voice.wav",
                "trim_out":2.0,"reverse":true,"preserve_pitch":false,
                "speed_keyframes":[{"source_sec":0.0,"speed":1.0},{"source_sec":2.0,"speed":2.0}]
            }]}]
        })))
        .unwrap();
        let plan = Plan::from_project(&project, |_| None);
        let graph = plan.graph(true);
        assert!(graph.contains("aresample=48000,asetrate=48000.000000,aresample=48000[ars1_0]"));
        assert!(graph.contains("[ars1_0]concat=n=1:v=0:a=1[arr1];[arr1]areverse,volume="));
    }

    #[test]
    fn reports_a_source_that_disappears_after_preflight_before_running_ffmpeg() {
        let stage = tempfile::tempdir().unwrap();
        let missing = stage.path().join("missing.wav");
        fs::write(&missing, b"fixture").unwrap();
        let project = normalize(Some(json!({
            "tracks":[{"id":"a","type":"audio","label":"a","clips":[{
                "id":"1","source_type":"file","file_path":missing
            }]}]
        })))
        .unwrap();
        let plan = Plan::from_project(&project, |_| None);
        assert_eq!(plan.sources().collect::<Vec<_>>(), [&missing]);
        fs::remove_file(&missing).unwrap();
        let (cancel, signal) = tokio::sync::watch::channel(false);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let sentinel = stage.path().join("must not run");
        let media = Media {
            ffmpeg: &sentinel,
            ffprobe: &sentinel,
            encoder: "",
            cancel: &signal,
        };
        assert_eq!(
            runtime.block_on(plan.mix(&media, &stage, &sentinel)),
            Err(SOURCE_MISSING)
        );
        assert_eq!(fs::read_dir(stage.path()).unwrap().count(), 0);
        cancel.send_replace(true);
        assert_eq!(
            runtime.block_on(plan.mix(&media, &stage, &sentinel)),
            Err(render::CANCELLED)
        );
    }

    #[test]
    #[ignore = "requires a verified FFmpeg/FFprobe pair and owned scratch directory"]
    fn real_mix_preserves_foreground_and_ducks_music_in_the_owned_output() {
        let ffmpeg = PathBuf::from(std::env::var_os("MGS_LITECUT_AUDIO_TEST_FFMPEG").unwrap());
        let scratch = PathBuf::from(std::env::var_os("MGS_LITECUT_AUDIO_TEST_SCRATCH").unwrap());
        let ffprobe = ffmpeg.with_file_name(if cfg!(windows) {
            "ffprobe.exe"
        } else {
            "ffprobe"
        });
        let inputs = tempfile::tempdir_in(&scratch).unwrap();
        let base = inputs.path().join("base clip.mp4");
        let voice = inputs.path().join("voice track.wav");
        let music = inputs.path().join("background music.wav");
        for (source, frequency) in [(&voice, "880"), (&music, "440")] {
            assert!(std::process::Command::new(&ffmpeg)
                .args([
                    "-hide_banner",
                    "-nostdin",
                    "-loglevel",
                    "error",
                    "-f",
                    "lavfi",
                    "-i"
                ])
                .arg(format!(
                    "sine=frequency={frequency}:sample_rate=48000:duration=3"
                ))
                .arg(source)
                .status()
                .unwrap()
                .success());
        }
        assert!(std::process::Command::new(&ffmpeg)
            .args([
                "-hide_banner",
                "-nostdin",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=black:size=64x64:rate=24:duration=2",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=220:sample_rate=48000:duration=2",
                "-c:v",
                "mpeg4",
                "-c:a",
                "aac",
                "-shortest",
            ])
            .arg(&base)
            .status()
            .unwrap()
            .success());
        let original = fs::read(&base).unwrap();
        let (_cancel, signal) = tokio::sync::watch::channel(false);
        let media = Media {
            ffmpeg: &ffmpeg,
            ffprobe: &ffprobe,
            encoder: "mpeg4",
            cancel: &signal,
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let levels = |ducking_enabled| {
            let project = normalize(Some(json!({
                "tracks":[{"id":"a","type":"audio","label":"a","volume":1.3,"clips":[{
                    "id":"1","source_type":"file","file_path":voice,
                    "trim_in":0.25,"trim_out":2.25,"timeline_start":0.4,"speed":2.0,
                    "speed_keyframes":[{"source_sec":0.25,"speed":2.0},
                                       {"source_sec":1.25,"speed":1.5},
                                       {"source_sec":2.25,"speed":1.5}],
                    "fade_in_sec":0.1,"fade_out_sec":0.1,
                    "audio_keyframes":[{"time_sec":0.0,"volume":0.3},{"time_sec":0.8,"volume":1.0}]
                }]}],
                "audio":{"bgm":{"path":music,"duration_sec":2.0,"ducking_enabled":ducking_enabled,
                                "ducking_volume":0.2}}
            })))
            .unwrap();
            let plan = Plan::from_project(&project, |_| None);
            let stage = tempfile::tempdir_in(&scratch).unwrap();
            let output = runtime.block_on(plan.mix(&media, &stage, &base)).unwrap();
            assert_eq!(output.parent(), Some(stage.path()));
            let decoded = std::process::Command::new(&ffmpeg)
                .args(["-hide_banner", "-loglevel", "error", "-i"])
                .arg(&output)
                .args(["-vn", "-ar", "48000", "-ac", "1", "-f", "s16le", "-"])
                .output()
                .unwrap();
            assert!(decoded.status.success());
            assert!(decoded.stdout.len() >= 48_000 * 2 * 19 / 10);

            let mixed = fs::read(&output).unwrap();
            assert_eq!(
                runtime.block_on(plan.mix(&media, &stage, &base)),
                Err(FAILED)
            );
            assert_eq!(fs::read(&output).unwrap(), mixed);
            let result = |frequency, start, end| tone_level(&decoded.stdout, frequency, start, end);
            (
                result(440.0, 0.8, 1.2),
                result(880.0, 0.8, 1.2),
                result(880.0, 0.1, 0.3),
            )
        };
        let (plain_bgm, plain_voice, early_voice) = levels(false);
        let (ducked_bgm, ducked_voice, early_ducked_voice) = levels(true);
        assert!(plain_bgm > 0.01 && plain_voice > 0.01);
        assert!(ducked_bgm < plain_bgm * 0.85);
        assert!(ducked_voice > plain_voice * 0.6);
        assert!(early_voice < plain_voice * 0.1 && early_ducked_voice < ducked_voice * 0.1);
        assert_eq!(fs::read(&base).unwrap(), original);
    }

    fn tone_level(data: &[u8], frequency: f64, start: f64, end: f64) -> f64 {
        let start = (start * 48_000.0) as usize;
        let end = (end * 48_000.0) as usize;
        let (real, imaginary) = data[start * 2..end * 2]
            .as_chunks::<2>()
            .0
            .iter()
            .enumerate()
            .fold((0.0, 0.0), |(real, imaginary), (index, pair)| {
                let sample = f64::from(i16::from_le_bytes(*pair)) / 32768.0;
                let phase = std::f64::consts::TAU * index as f64 * frequency / 48_000.0;
                (
                    real + sample * phase.cos(),
                    imaginary + sample * phase.sin(),
                )
            });
        2.0 * real.hypot(imaginary) / (end - start) as f64
    }
}
