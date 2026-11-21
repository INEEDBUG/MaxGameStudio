pub(super) mod overlay;

use super::ffmpeg::{command, inspect};
use super::project_schema::list;
use crate::profile_platform::path_text;
use serde_json::Value;
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{process::Command, sync::watch, time::timeout};

pub(super) const CLIP_FAILED: &str = "MONTAGE_CLIP_NORMALIZE_FAILED";
pub(super) const CANCELLED: &str = "MONTAGE_EXPORT_CANCELLED";
const EXPORT_FAILED: &str = "MONTAGE_EXPORT_FAILED";

pub(super) struct Segment<'a> {
    pub path: &'a Path,
    pub clip: &'a Value,
    pub track_gain: f64,
}

pub(super) struct Media<'a> {
    pub ffmpeg: &'a Path,
    pub ffprobe: &'a Path,
    pub encoder: &'a str,
    pub cancel: &'a watch::Receiver<bool>,
}

pub(super) struct Timeline<'a> {
    width: u64,
    height: u64,
    fps: u64,
    fit: &'a str,
    background: String,
    blur: u64,
    pub(super) fast: bool,
}

impl<'a> Timeline<'a> {
    pub(super) fn from_project(project: &'a Value) -> Self {
        let output = &project["output"];
        Self {
            width: output["width"].as_u64().expect("normalized width").max(320),
            height: output["height"]
                .as_u64()
                .expect("normalized height")
                .max(180),
            fps: output["fps"].as_u64().expect("normalized frame rate"),
            fit: output["canvas_fit"]
                .as_str()
                .expect("normalized canvas fit"),
            background: color(&output["background_color"]),
            blur: output["blur_amount"]
                .as_u64()
                .expect("normalized blur")
                .clamp(4, 80),
            fast: output["encoder_tier"] == "fast",
        }
    }

    fn video(&self, clip: &Value) -> String {
        let (width, height, fps) = (self.width, self.height, self.fps);
        let fit = clip["canvas_fit"]
            .as_str()
            .filter(|fit| *fit != "inherit")
            .unwrap_or(self.fit);
        let mut graph = vec!["[0:v:0]setpts=PTS-STARTPTS".to_owned()];
        let crop = &clip["crop"];
        if !crop.is_null() {
            let w = number(&crop["width"]).clamp(0.05, 1.0);
            let h = number(&crop["height"]).clamp(0.05, 1.0);
            let x = number(&crop["x"]).min(1.0 - w);
            let y = number(&crop["y"]).min(1.0 - h);
            if w < 0.9999 || h < 0.9999 {
                graph.push(format!("crop=iw*{w:.6}:ih*{h:.6}:iw*{x:.6}:ih*{y:.6}"));
            }
        }
        let surround = match fit {
            "cover" => format!(
                "scale={width}:{height}:force_original_aspect_ratio=increase,crop={width}:{height}"
            ),
            "blur" => {
                let blur = self.blur;
                format!(
                    "split=2[fg][bg];[bg]scale={width}:{height}:force_original_aspect_ratio=increase,crop={width}:{height},gblur=sigma={blur}[bgfit];[fg]scale={width}:{height}:force_original_aspect_ratio=decrease[fgfit];[bgfit][fgfit]overlay=(W-w)/2:(H-h)/2"
                )
            }
            _ => format!(
                "scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color={}",
                self.background,
            ),
        };
        graph.push(surround);
        graph.push(format!("fps={fps},setsar=1,format=yuv420p"));
        for (key, filter) in [("flip_horizontal", "hflip"), ("flip_vertical", "vflip")] {
            if clip[key] == true {
                graph.push(filter.to_owned());
            }
        }
        apply_color(&mut graph, &clip["color"]);
        if clip["reverse"] == true {
            graph.push("reverse".to_owned());
        }
        let speed = number(&clip["speed"]);
        if (speed - 1.0).abs() > 1e-6 {
            graph.push(format!("setpts=PTS/{speed:.6}"));
        }
        let freeze = number(&clip["freeze_frame_sec"]);
        if freeze > 1e-6 {
            graph.push(format!("tpad=stop_mode=clone:stop_duration={freeze:.6}"));
        }
        let duration = length(clip);
        let fade_in = number(&clip["fade_in_sec"]).min(duration);
        let fade_out = number(&clip["fade_out_sec"]).min(duration);
        if fade_in > 0.0 {
            graph.push(format!("fade=t=in:st=0:d={fade_in:.6}"));
        }
        if fade_out > 0.0 {
            let start = duration - fade_out;
            graph.push(format!("fade=t=out:st={start:.6}:d={fade_out:.6}"));
        }
        format!("{}[vout]", graph.join(","))
    }

    fn audio(input: &Segment<'_>, has_audio: bool) -> String {
        let duration = length(input.clip);
        if !has_audio || input.clip["muted"] == true || input.track_gain <= 0.0 {
            return silence(duration);
        }
        let mut graph = vec!["[0:a:0]asetpts=PTS-STARTPTS".to_owned()];
        if input.clip["reverse"] == true {
            graph.push("areverse".to_owned());
        }
        let speed = number(&input.clip["speed"]);
        if (speed - 1.0).abs() > 1e-6 {
            if input.clip["preserve_pitch"] == false {
                graph.push(format!(
                    "aresample=48000,asetrate={:.6},aresample=48000",
                    48_000.0 * speed
                ));
            } else {
                let first = speed.clamp(0.5, 2.0);
                graph.push(format!("atempo={first:.6}"));
                let second = speed / first;
                if (second - 1.0).abs() > 1e-6 {
                    graph.push(format!("atempo={second:.6}"));
                }
            }
        }
        let volume = (number(&input.clip["volume"]) * input.track_gain).min(5.0);
        graph.push(format!(
            "volume={volume:.6},aformat=sample_rates=48000:channel_layouts=stereo"
        ));
        graph.push(format!(
            "apad=whole_dur={duration:.6},atrim=duration={duration:.6}"
        ));
        format!("{}[aout]", graph.join(","))
    }

    async fn write_segment(
        &self,
        media: &Media<'_>,
        input: Option<&Segment<'_>>,
        duration: f64,
        destination: &Path,
    ) -> Result<(), &'static str> {
        let mut process = render_command(media.ffmpeg);
        let graph = if let Some(input) = input {
            let info = probe(media.ffprobe, input.path, media.cancel, CLIP_FAILED).await?;
            let types = list(&info["streams"]);
            if !types.iter().any(|stream| stream["codec_type"] == "video") {
                return Err(CLIP_FAILED);
            }
            let has_audio = types.iter().any(|stream| stream["codec_type"] == "audio");
            process
                .args(["-ss", &format!("{:.6}", number(&input.clip["trim_in"]))])
                .args(["-t", &format!("{:.6}", source_duration(input.clip)), "-i"])
                .arg(path_text(input.path));
            format!(
                "{};{}",
                self.video(input.clip),
                Self::audio(input, has_audio)
            )
        } else {
            format!(
                "color=c={}:s={}x{}:r={}:d={duration:.6},format=yuv420p[vout];{}",
                self.background,
                self.width,
                self.height,
                self.fps,
                silence(duration),
            )
        };
        process
            .args([
                "-filter_complex",
                &graph,
                "-map",
                "[vout]",
                "-map",
                "[aout]",
            ])
            .args(["-t", &format!("{duration:.6}")])
            .args(encoder_args(media.encoder, self.fast))
            .args(["-c:a", "aac", "-b:a", "192k", "-ar", "48000", "-ac", "2"])
            .arg(path_text(destination));
        run(
            process,
            media.cancel,
            CLIP_FAILED,
            Duration::from_secs(3600),
        )
        .await
    }

    pub(super) async fn render_base(
        &self,
        media: &Media<'_>,
        clips: &[Segment<'_>],
        stage: &Path,
        mut progress: impl FnMut(f64, &'static str),
    ) -> Result<PathBuf, &'static str> {
        let mut files = Vec::new();
        let mut cursor = 0.0;
        for (index, segment) in clips.iter().enumerate() {
            let start = number(&segment.clip["timeline_start"]);
            if start > cursor + 0.001 {
                let name = format!("gap_{index}.ts");
                self.write_segment(media, None, start - cursor, &stage.join(&name))
                    .await?;
                files.push(name);
            }
            let name = format!("clip_{index}.ts");
            let duration = length(segment.clip);
            self.write_segment(media, Some(segment), duration, &stage.join(&name))
                .await?;
            cursor = start + duration;
            files.push(name);
            progress(
                0.10 + 0.55 * (index + 1) as f64 / clips.len() as f64,
                "normalizing",
            );
        }
        let playlist: String = files
            .iter()
            .map(|name| format!("file '{name}'\n"))
            .collect();
        fs::write(stage.join("concat.txt"), playlist).map_err(|_| EXPORT_FAILED)?;
        let joined = stage.join("timeline.mp4");
        let mut process = render_command(media.ffmpeg);
        process
            .current_dir(stage)
            .args([
                "-f",
                "concat",
                "-safe",
                "1",
                "-i",
                "concat.txt",
                "-c",
                "copy",
            ])
            .args(["-movflags", "+faststart", "timeline.mp4"]);
        progress(0.68, "concat");
        run(
            process,
            media.cancel,
            EXPORT_FAILED,
            Duration::from_secs(3600),
        )
        .await?;
        for name in files {
            fs::remove_file(stage.join(name)).map_err(|_| EXPORT_FAILED)?;
        }
        Ok(joined)
    }

    pub(super) async fn trim_range(
        &self,
        media: &Media<'_>,
        input: &Path,
        (start, end): (f64, Option<f64>),
        stage: &Path,
    ) -> Result<PathBuf, &'static str> {
        let result = stage.join("range.mp4");
        let mut process = render_command(media.ffmpeg);
        process
            .args(["-ss", &format!("{start:.6}"), "-i"])
            .arg(path_text(input));
        if let Some(end) = end {
            process.args(["-t", &format!("{:.6}", end - start)]);
        }
        process
            .args(encoder_args(media.encoder, self.fast))
            .args(["-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart"])
            .arg(path_text(&result));
        run(
            process,
            media.cancel,
            EXPORT_FAILED,
            Duration::from_secs(3600),
        )
        .await?;
        Ok(result)
    }
}

pub(super) fn length(clip: &Value) -> f64 {
    (source_duration(clip) / number(&clip["speed"])).max(0.1) + number(&clip["freeze_frame_sec"])
}

fn source_duration(clip: &Value) -> f64 {
    let trimmed = clip["trim_out"].as_f64().or_else(|| {
        let value = &clip["meta"]["duration_sec"];
        value
            .as_f64()
            .or_else(|| value.as_str()?.parse().ok())
            .filter(|value| (0.0..=86400.0).contains(value))
    });
    trimmed.map_or(5.0, |end| (end - number(&clip["trim_in"])).max(0.1))
}

pub(super) fn number(value: &Value) -> f64 {
    value.as_f64().expect("normalized numeric value")
}

fn silence(duration: f64) -> String {
    format!("anullsrc=r=48000:cl=stereo:d={duration:.6}[aout]")
}

fn color(value: &Value) -> String {
    let raw = value.as_str().unwrap_or_default().trim();
    let hex = raw.strip_prefix('#').unwrap_or(raw);
    if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return "black".to_owned();
    }
    match hex.len() {
        3 => format!(
            "0x{}",
            hex.chars().flat_map(|ch| [ch, ch]).collect::<String>()
        ),
        6 => format!("0x{hex}"),
        _ => "black".to_owned(),
    }
}

fn apply_color(filters: &mut Vec<String>, color: &Value) {
    if color.is_null() {
        return;
    }
    static CONTRACT: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| {
        serde_json::from_str(include_str!(
            "../../../../../data/lite_cut_effect_contract.json"
        ))
        .expect("valid bundled LiteCut effect contract")
    });
    if let Some(id) = color["filter_preset"].as_str() {
        if let Some(filter) = list(&CONTRACT["filter_presets"])
            .iter()
            .find(|preset| preset["id"] == id.trim().to_ascii_lowercase())
            .and_then(|preset| preset["ffmpeg"].as_str())
            .filter(|filter| !filter.is_empty())
        {
            filters.push(filter.to_owned());
        }
    }
    let brightness = 1.0 + number(&color["brightness"]) / 100.0;
    let contrast = 1.0 + number(&color["contrast"]) / 100.0;
    let saturation = 1.0 + number(&color["saturation"]) / 100.0;
    if (brightness - 1.0).abs() >= 1e-6 {
        filters.push(format!(
            "colorchannelmixer=rr={brightness:.4}:gg={brightness:.4}:bb={brightness:.4}"
        ));
    }
    if (contrast - 1.0).abs() >= 1e-6 || (saturation - 1.0).abs() >= 1e-6 {
        filters.push(format!(
            "eq=contrast={contrast:.4}:saturation={saturation:.4}"
        ));
    }
}

pub(super) fn encoder_args(codec: &str, fast: bool) -> Vec<&'static str> {
    match (codec, fast) {
        ("h264_nvenc", false) => vec![
            "-c:v",
            "h264_nvenc",
            "-preset",
            "p4",
            "-rc",
            "vbr",
            "-cq",
            "20",
            "-bf",
            "2",
            "-profile:v",
            "high",
            "-pix_fmt",
            "yuv420p",
        ],
        ("h264_nvenc", true) => vec![
            "-c:v",
            "h264_nvenc",
            "-preset",
            "p6",
            "-rc",
            "vbr",
            "-cq",
            "22",
            "-bf",
            "2",
            "-profile:v",
            "high",
            "-pix_fmt",
            "yuv420p",
        ],
        ("h264_qsv", fast) => vec![
            "-c:v",
            "h264_qsv",
            "-preset",
            if fast { "fast" } else { "medium" },
            "-global_quality",
            if fast { "24" } else { "22" },
            "-profile:v",
            "high",
            "-pix_fmt",
            "yuv420p",
        ],
        ("h264_amf", fast) => vec![
            "-c:v",
            "h264_amf",
            "-usage",
            "transcoding",
            "-quality",
            if fast { "speed" } else { "balanced" },
            "-rc",
            "cqp",
            "-qp_i",
            if fast { "22" } else { "20" },
            "-qp_p",
            if fast { "24" } else { "22" },
            "-vbaq",
            "false",
            "-pix_fmt",
            "yuv420p",
        ],
        _ => vec![
            "-c:v",
            "libx264",
            "-preset",
            if fast { "faster" } else { "medium" },
            "-crf",
            if fast { "20" } else { "18" },
            "-profile:v",
            "main",
            "-pix_fmt",
            "yuv420p",
        ],
    }
}

pub(super) fn render_command(ffmpeg: &Path) -> Command {
    let mut process = command(ffmpeg);
    process.args(["-nostdin", "-y", "-hide_banner", "-loglevel", "error"]);
    process
}

pub(super) async fn probe(
    ffprobe: &Path,
    file: &Path,
    cancel: &watch::Receiver<bool>,
    failure: &'static str,
) -> Result<Value, &'static str> {
    inspect(ffprobe, file, Some(cancel)).await.map_err(|error| {
        if error.kind() == io::ErrorKind::Interrupted {
            CANCELLED
        } else {
            failure
        }
    })
}

pub(super) async fn run(
    mut command: Command,
    cancel: &watch::Receiver<bool>,
    failure: &'static str,
    limit: Duration,
) -> Result<(), &'static str> {
    use futures_util::future::{select, Either};
    if *cancel.borrow() {
        return Err(CANCELLED);
    }
    let mut child = command.spawn().map_err(|_| failure)?;
    let mut signal = cancel.clone();
    let cancelled = signal.wait_for(|value| *value);
    let code = match select(Box::pin(timeout(limit, child.wait())), Box::pin(cancelled)).await {
        Either::Left((Ok(Ok(status)), _)) if status.success() => return Ok(()),
        Either::Left((Ok(_), _)) => return Err(failure),
        Either::Left(_) => failure,
        Either::Right(_) => CANCELLED,
    };
    child
        .kill()
        .await
        .or_else(|error| {
            if error.kind() == io::ErrorKind::InvalidInput {
                Ok(())
            } else {
                Err(error)
            }
        })
        .map_err(|_| failure)?;
    Err(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_http::lite_cut::project_schema::normalize;
    use serde_json::json;

    #[test]
    fn preserves_the_old_minimum_cut_length() {
        let body = normalize(Some(json!({
            "tracks":[{"id":"v1","type":"video","label":"V1","clips":[
                {"id":"short","trim_in":0.0,"trim_out":0.1,"speed":4.0,"freeze_frame_sec":0.25}
            ]}]
        })))
        .unwrap();
        assert_eq!(length(&body["tracks"][0]["clips"][0]), 0.35);
    }
}
