//! Render a positional transition into the incoming normalized segment.

use super::{
    super::{ffmpeg::command, project_schema::list},
    encoder_args, length, number, probe, run, Media, CANCELLED,
};
use crate::profile_platform::path_text;
use serde_json::Value;
use std::{
    io,
    path::{Path, PathBuf},
    time::Duration,
};
use tempfile::TempDir;

const FAILED: &str = "MONTAGE_TRANSITION_FAILED";
const GAP_EPSILON: f64 = 0.001;

#[derive(Clone, Copy)]
enum Effect {
    Crossfade(&'static str),
    Color(&'static str),
}

pub(super) struct Boundary {
    effect: Effect,
    requested: f64,
    fps: u64,
}

impl Boundary {
    pub(super) fn between(previous: &Value, incoming: &Value, fps: u64) -> Option<Self> {
        let previous_end = number(&previous["timeline_start"]) + length(previous);
        let start = number(&incoming["timeline_start"]);
        if (start - previous_end).abs() > GAP_EPSILON || !(1..=240).contains(&fps) {
            return None;
        }
        let selected = incoming["transition_in"]
            .as_object()
            .or_else(|| previous["transition_out"].as_object())?;
        let requested = selected["duration_sec"].as_f64().unwrap_or(0.4);
        let min_duration = (1.0 / fps.max(24) as f64).max(0.02);
        if requested < min_duration {
            return None;
        }
        let effect = match selected["type"]
            .as_str()
            .unwrap_or("cut")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "none" => return None,
            "dip" | "dip_black" | "black" => Effect::Color("black"),
            "flash" | "flashwhite" => Effect::Color("white"),
            "zoom" => Effect::Crossfade("zoomin"),
            "wipe_l" => Effect::Crossfade("wipeleft"),
            "wipe_r" => Effect::Crossfade("wiperight"),
            "slide_left" => Effect::Crossfade("slideleft"),
            "slide_right" => Effect::Crossfade("slideright"),
            "slide_up" => Effect::Crossfade("slideup"),
            "slide_down" => Effect::Crossfade("slidedown"),
            "blur" => Effect::Crossfade("hblur"),
            "glitch" => Effect::Crossfade("pixelize"),
            "spin" => Effect::Crossfade("radial"),
            _ => Effect::Crossfade("fade"),
        };
        Some(Self {
            effect,
            requested,
            fps,
        })
    }

    /// Previous may itself be a transitioned segment. Keep it in the concat
    /// list, replacing only incoming with the returned path. The directory is
    /// owned by the current export and index uniquely identifies this edge.
    pub(super) async fn render(
        &self,
        media: &Media<'_>,
        stage: &TempDir,
        index: usize,
        previous: &Path,
        incoming: &Path,
        fast: bool,
    ) -> Result<PathBuf, &'static str> {
        if *media.cancel.borrow() {
            return Err(CANCELLED);
        }
        let previous_info = probe(media.ffprobe, previous, media.cancel, FAILED).await?;
        let incoming_info = probe(media.ffprobe, incoming, media.cancel, FAILED).await?;
        let previous_video = stream(&previous_info, "video")?;
        let incoming_video = stream(&incoming_info, "video")?;
        if ["width", "height"].iter().any(|dimension| {
            previous_video[dimension].as_u64().unwrap_or(0) == 0
                || previous_video[dimension] != incoming_video[dimension]
        }) {
            return Err(FAILED);
        }
        let previous_frames = self.frames(previous_video)?;
        let incoming_frames = self.frames(incoming_video)?;
        let Some(graph) = self.graph(previous_frames, incoming_frames) else {
            return Ok(incoming.to_owned());
        };
        let incoming_audio = stream(&incoming_info, "audio")?;
        // The video graph starts at zero; carry any AAC priming lead the
        // normalizer originally put before its first picture.
        let audio_lead = (stream_number(incoming_video, "start_time")?
            - stream_number(incoming_audio, "start_time")?)
        .max(0.0);
        let previous = previous.canonicalize().map_err(|_| FAILED)?;
        let incoming = incoming.canonicalize().map_err(|_| FAILED)?;
        let filename = format!("transition_{index}.ts");
        let output = stage.path().join(&filename);
        match output.symlink_metadata() {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            _ => return Err(FAILED),
        }
        let mut process = command(media.ffmpeg);
        process
            .current_dir(stage.path())
            .args(["-nostdin", "-n", "-hide_banner", "-loglevel", "error", "-i"])
            .arg(path_text(&previous));
        if audio_lead > 0.0 {
            process.args(["-itsoffset", &format!("-{audio_lead:.6}")]);
        }
        process
            .arg("-i")
            .arg(path_text(&incoming))
            .args(["-filter_complex", &graph, "-map", "[vout]", "-map", "1:a:0"])
            .args(encoder_args(media.encoder, fast))
            .args(["-c:a", "copy"])
            .arg(&filename);
        run(process, media.cancel, FAILED, Duration::from_secs(3600)).await?;
        Ok(output)
    }

    fn frames(&self, stream: &Value) -> Result<u64, &'static str> {
        // MPEG-TS container duration can extend past the last video frame
        // because normalized audio is packetized separately.
        let duration = stream_number(stream, "duration")?;
        let count = (duration * self.fps as f64).round();
        if !count.is_finite() || count < 1.0 || count >= u64::MAX as f64 {
            return Err(FAILED);
        }
        Ok(count as u64)
    }

    fn graph(&self, previous: u64, incoming: u64) -> Option<String> {
        if incoming < 2 {
            // A single-frame segment has no visual tail to transition into.
            return None;
        }
        let fps = self.fps;
        let rate = fps as f64;
        let frame = 1.0 / fps.max(24) as f64;
        let duration = self
            .requested
            .min(1.5)
            .min(incoming as f64 / rate - frame)
            .max(frame);
        let front = ((duration * rate - 1e-6).ceil() as u64).clamp(1, incoming - 1);
        let cadence = format!("fps={fps},settb=1/{fps},setpts=N,setsar=1");
        let mut parts = vec![
            format!("[1:v:0]setpts=PTS-STARTPTS,{cadence},split=2[intro][tailsrc]"),
            format!("[tailsrc]trim=start_frame={front},setpts=PTS-STARTPTS[tail]"),
        ];
        match self.effect {
            Effect::Crossfade(name) => {
                parts.push(format!(
                    "[0:v:0]setpts=PTS-STARTPTS,{cadence},trim=start_frame={},setpts=PTS-STARTPTS,tpad=stop_mode=clone:stop={front}[hold]",
                    previous - 1
                ));
                parts.push(format!("[intro]trim=end_frame={front}[head]"));
                parts.push(format!(
                    "[hold][head]xfade=transition={name}:duration={duration:.6}:offset=0,trim=end_frame={front}[fx]"
                ));
                parts.push("[fx][tail]concat=n=2:v=1:a=0".to_owned());
            }
            Effect::Color(color) if front > 1 => {
                let half = duration / 2.0;
                let midpoint = ((half * rate - 1e-6).ceil() as u64).clamp(1, front - 1);
                let available = midpoint.min(previous);
                let pad = midpoint - available;
                parts.push(format!(
                    "[0:v:0]setpts=PTS-STARTPTS,{cadence},trim=start_frame={},setpts=PTS-STARTPTS,tpad=start_mode=clone:start={pad},fade=t=out:st=0:d={half:.6}:color={color}[outphase]",
                    previous - available
                ));
                parts.push(format!(
                    "[intro]trim=start_frame={midpoint}:end_frame={front},setpts=PTS-STARTPTS,fade=t=in:st=0:d={half:.6}:color={color}[inphase]"
                ));
                parts.push("[outphase][inphase][tail]concat=n=3:v=1:a=0".to_owned());
            }
            Effect::Color(color) => {
                parts.push(format!(
                    "[intro]trim=end_frame=1,fade=t=in:n=1:color={color}[fx]"
                ));
                parts.push("[fx][tail]concat=n=2:v=1:a=0".to_owned());
            }
        }
        parts
            .last_mut()
            .expect("video output")
            .push_str(&format!(",fps={fps},format=yuv420p,setsar=1[vout]"));
        Some(parts.join(";"))
    }
}

fn stream<'a>(info: &'a Value, kind: &str) -> Result<&'a Value, &'static str> {
    list(&info["streams"])
        .iter()
        .find(|stream| stream["codec_type"] == kind)
        .ok_or(FAILED)
}

fn stream_number(stream: &Value, field: &str) -> Result<f64, &'static str> {
    let value = &stream[field];
    value
        .as_f64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|number| number.is_finite())
        .ok_or(FAILED)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_http::lite_cut::project_schema::normalize;
    use serde_json::json;

    fn clip(start: f64, end: f64, incoming: Value, outgoing: Value) -> Value {
        let mut project = normalize(Some(json!({
            "tracks":[{"id":"v1","type":"video","label":"V1","clips":[{
                "id":"clip","timeline_start":start,"trim_out":end-start,
                "transition_in":incoming,"transition_out":outgoing
            }]}]
        })))
        .unwrap();
        project["tracks"][0]["clips"][0].take()
    }

    #[test]
    fn incoming_wins_even_when_none_and_gap_stays_a_cut() {
        let start = clip(
            0.0,
            1.0,
            Value::Null,
            json!({"type":"wipe_l","duration_sec":0.5}),
        );
        let none = clip(
            1.0,
            2.0,
            json!({"type":"none","duration_sec":0.5}),
            Value::Null,
        );
        assert!(Boundary::between(&start, &none, 30).is_none());
        let gap = clip(1.1, 2.0, Value::Null, Value::Null);
        assert!(Boundary::between(&start, &gap, 30).is_none());
        let contiguous = clip(1.0, 2.0, Value::Null, Value::Null);
        let boundary = Boundary::between(&start, &contiguous, 30).unwrap();
        assert!(boundary
            .graph(30, 30)
            .unwrap()
            .contains("transition=wipeleft:duration=0.500000:offset=0"));
    }

    #[test]
    fn clamps_to_the_incoming_tail_and_preserves_midpoint_effect_aliases() {
        let start = clip(0.0, 1.0, Value::Null, Value::Null);
        for (alias, color) in [("dip", "black"), ("flashwhite", "white")] {
            let next = clip(
                1.0,
                2.0,
                json!({"type":alias,"duration_sec":3.0}),
                Value::Null,
            );
            let boundary = Boundary::between(&start, &next, 30).unwrap();
            let graph = boundary.graph(30, 30).unwrap();
            assert!(graph.contains("trim=start_frame=15"));
            assert!(graph.contains(&format!("color={color}")));
            assert!(!graph.contains("xfade="));
        }
    }

    #[test]
    #[ignore = "requires a verified FFmpeg/FFprobe pair and owned scratch directory"]
    fn only_the_incoming_pictures_change_for_two_consecutive_boundaries() {
        let ffmpeg = PathBuf::from(std::env::var_os("MGS_LITECUT_TRANSITION_TEST_FFMPEG").unwrap());
        let scratch =
            PathBuf::from(std::env::var_os("MGS_LITECUT_TRANSITION_TEST_SCRATCH").unwrap());
        let ffprobe = ffmpeg.with_file_name(if cfg!(windows) {
            "ffprobe.exe"
        } else {
            "ffprobe"
        });
        let stage = tempfile::tempdir_in(scratch).unwrap();
        let previous = stage.path().join("previous clip.ts");
        let incoming = stage.path().join("incoming clip.ts");
        let following = stage.path().join("third clip.ts");
        for (source, color, tone) in [
            (&previous, "red", 220),
            (&incoming, "blue", 440),
            (&following, "lime", 880),
        ] {
            assert!(std::process::Command::new(&ffmpeg)
                .args([
                    "-nostdin",
                    "-n",
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-f",
                    "lavfi",
                    "-i"
                ])
                .arg(format!("color={color}:size=64x64:rate=30:duration=1.2"))
                .args(["-f", "lavfi", "-i"])
                .arg(format!(
                    "sine=frequency={tone}:sample_rate=48000:duration=1.2"
                ))
                .args([
                    "-c:v",
                    "libx264",
                    "-preset",
                    "ultrafast",
                    "-pix_fmt",
                    "yuv420p",
                    "-c:a",
                    "aac",
                    "-ar",
                    "48000",
                    "-ac",
                    "2",
                    "-t",
                    "1.2"
                ])
                .arg(source)
                .status()
                .unwrap()
                .success());
        }
        let original_previous = std::fs::read(&previous).unwrap();
        let original_incoming = std::fs::read(&incoming).unwrap();
        let original_following = std::fs::read(&following).unwrap();
        let (cancel, signal) = tokio::sync::watch::channel(false);
        let media = Media {
            ffmpeg: &ffmpeg,
            ffprobe: &ffprobe,
            encoder: "libx264",
            cancel: &signal,
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let a = clip(0.0, 1.2, Value::Null, Value::Null);
        let b = clip(
            1.2,
            2.4,
            json!({"type":"fade","duration_sec":0.6}),
            Value::Null,
        );
        let c = clip(
            2.4,
            3.6,
            json!({"type":"flashwhite","duration_sec":0.8}),
            Value::Null,
        );
        let fade = Boundary::between(&a, &b, 30).unwrap();
        let flashed = Boundary::between(&b, &c, 30).unwrap();
        let blue = runtime
            .block_on(fade.render(&media, &stage, 1, &previous, &incoming, true))
            .unwrap();
        let green = runtime
            .block_on(flashed.render(&media, &stage, 2, &blue, &following, true))
            .unwrap();
        for (output, input) in [(&blue, &incoming), (&green, &following)] {
            let received = av_offset(&ffprobe, output);
            let source = av_offset(&ffprobe, input);
            assert!((received - source).abs() < 0.0001);
            println!("Synthetic video/audio offset source={source:.6}s transition={received:.6}s");
        }
        assert_eq!(
            mux_stream(&ffmpeg, &blue, false),
            mux_stream(&ffmpeg, &incoming, false)
        );
        assert_eq!(
            mux_stream(&ffmpeg, &green, false),
            mux_stream(&ffmpeg, &following, false)
        );
        let blue_pixels = mux_stream(&ffmpeg, &blue, true);
        let green_pixels = mux_stream(&ffmpeg, &green, true);
        assert_eq!(blue_pixels.len(), 36 * 3);
        assert_eq!(green_pixels.len(), 36 * 3);
        assert!(blue_pixels[0] > 240 && blue_pixels[2] < 8); // held outgoing image
        assert!(blue_pixels[9 * 3] > 95 && blue_pixels[9 * 3 + 2] > 95);
        assert!(blue_pixels[30 * 3] < 8 && blue_pixels[30 * 3 + 2] > 240);
        for (actual, expected) in green_pixels[..3].iter().zip(&blue_pixels[35 * 3..36 * 3]) {
            assert!(actual.abs_diff(*expected) <= 4);
        }
        assert!(green_pixels[12 * 3..13 * 3]
            .iter()
            .all(|component| *component >= 248));
        assert!(green_pixels[30 * 3] < 8 && green_pixels[30 * 3 + 1] > 240);
        let existing = std::fs::read(&green).unwrap();
        assert_eq!(
            runtime.block_on(flashed.render(&media, &stage, 2, &blue, &following, true)),
            Err(FAILED)
        );
        assert_eq!(std::fs::read(&green).unwrap(), existing);
        cancel.send_replace(true);
        assert_eq!(
            runtime.block_on(flashed.render(&media, &stage, 3, &blue, &following, true)),
            Err(CANCELLED)
        );
        assert!(!stage.path().join("transition_3.ts").exists());
        assert_eq!(std::fs::read(&previous).unwrap(), original_previous);
        assert_eq!(std::fs::read(&incoming).unwrap(), original_incoming);
        assert_eq!(std::fs::read(&following).unwrap(), original_following);

        if let Some(reference) = std::env::var_os("MGS_LITECUT_TRANSITION_TEST_REFERENCE") {
            let graph: Value = serde_json::from_slice(&std::fs::read(reference).unwrap()).unwrap();
            check_released_pictures(&ffmpeg, &previous, &incoming, &graph["fade"], &blue_pixels);
            check_released_pictures(&ffmpeg, &blue, &following, &graph["flash"], &green_pixels);
        }
    }

    fn mux_stream(ffmpeg: &Path, file: &Path, video: bool) -> Vec<u8> {
        let mut process = std::process::Command::new(ffmpeg);
        process
            .args(["-hide_banner", "-loglevel", "error", "-i"])
            .arg(file);
        if video {
            process.args([
                "-vf",
                "scale=1:1,format=rgb24",
                "-an",
                "-f",
                "rawvideo",
                "-",
            ]);
        } else {
            process.args(["-map", "0:a:0", "-c:a", "copy", "-f", "adts", "-"]);
        }
        let output = process.output().unwrap();
        assert!(output.status.success());
        output.stdout
    }

    fn av_offset(ffprobe: &Path, file: &Path) -> f64 {
        let output = std::process::Command::new(ffprobe)
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_type,start_time",
                "-of",
                "json",
            ])
            .arg(file)
            .output()
            .unwrap();
        assert!(output.status.success());
        let info: Value = serde_json::from_slice(&output.stdout).unwrap();
        let start = |kind| {
            list(&info["streams"])
                .iter()
                .find(|stream| stream["codec_type"] == kind)
                .unwrap()["start_time"]
                .as_str()
                .unwrap()
                .parse::<f64>()
                .unwrap()
        };
        start("video") - start("audio")
    }

    fn check_released_pictures(
        ffmpeg: &Path,
        earlier: &Path,
        later: &Path,
        graph: &Value,
        new: &[u8],
    ) {
        let graph = format!(
            "{};[vout]scale=1:1,format=rgb24[comparison]",
            graph.as_str().unwrap()
        );
        let output = std::process::Command::new(ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-i"])
            .arg(earlier)
            .arg("-i")
            .arg(later)
            .args([
                "-filter_complex",
                &graph,
                "-map",
                "[comparison]",
                "-map",
                "[aout]",
                "-c:v",
                "rawvideo",
                "-pix_fmt",
                "rgb24",
                "-c:a",
                "pcm_s16le",
                "-f",
                "nut",
                "-",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        use std::io::Write;
        use std::process::Stdio;
        let mut decode = std::process::Command::new(ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-i",
                "-",
                "-vf",
                "fps=30",
                "-an",
                "-pix_fmt",
                "rgb24",
                "-f",
                "rawvideo",
                "-",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        decode
            .stdin
            .take()
            .unwrap()
            .write_all(&output.stdout)
            .unwrap();
        let decode = decode.wait_with_output().unwrap();
        assert!(decode.status.success());
        assert_eq!(decode.stdout.len(), 72 * 3);
        for (byte, (reference, native)) in decode.stdout[36 * 3..].iter().zip(new).enumerate() {
            assert!(
                reference.abs_diff(*native) <= 8,
                "frame {} channel {} expected {reference}, got {native}, released fade={}",
                byte / 3,
                byte % 3,
                graph.contains("xfade="),
            );
        }
    }
}
