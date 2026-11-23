//! Composite visible upper video tracks, then apply canvas stickers and text.

use super::{length, number, probe, render_command, run, text_overlay::TextOverlay, Media};
use crate::native_http::lite_cut::{
    export_sources::{main_clip, media_path, upper_clips, upper_file_path, upper_recording_id},
    project_schema::list,
};
use crate::profile_platform::path_text;
use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use tempfile::TempDir;

const FAILED: &str = "MONTAGE_EXPORT_FAILED";
pub(in crate::native_http::lite_cut) const UNAVAILABLE: &str = "LITECUT_EXPORT_EFFECT_UNAVAILABLE";

pub(in crate::native_http::lite_cut) struct Plan(Vec<Layer>);

enum Layer {
    Media {
        asset: PathBuf,
        placement: Placement,
        trim_in: Option<f64>,
    },
    Text { start: f64, overlay: TextOverlay },
}

struct Placement {
    start: f64,
    end: f64,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    radians: f64,
    opacity: f64,
    fade_in: f64,
    fade_out: f64,
    hflip: bool,
    vflip: bool,
}

impl Plan {
    pub(in crate::native_http::lite_cut) fn from_project(
        project: &Value,
        base: usize,
        mut recording_path: impl FnMut(i64) -> Option<PathBuf>,
        mut resolve_font: impl FnMut(&Value) -> Option<PathBuf>,
    ) -> Result<Self, &'static str> {
        let hidden: HashSet<_> = list(&project["overlay_tracks"])
            .iter()
            .filter(|track| track["hidden"] == true)
            .filter_map(|track| track["id"].as_str())
            .collect();
        let mut layers = Vec::new();
        for clip in upper_clips(list(&project["tracks"]), base) {
            if !main_clip(clip) {
                if clip["source_type"] == "file" && upper_file_path(clip).is_some() {
                    return Err(UNAVAILABLE);
                }
                continue;
            }
            if !ordinary_video(clip) {
                return Err(UNAVAILABLE);
            }
            let asset = upper_file_path(clip)
                .map(media_path)
                .or_else(|| upper_recording_id(clip).and_then(&mut recording_path))
                .ok_or("MONTAGE_CLIP_FILE_MISSING")?;
            layers.push(Layer::Media {
                asset,
                placement: Placement::from_clip(clip, length(clip), true),
                trim_in: Some(number(&clip["trim_in"])),
            });
        }
        for clip in list(&project["overlays"]) {
            let track = clip["meta"]["overlay_track_id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .unwrap_or("ot1");
            if hidden.contains(track) {
                continue;
            }
            if clip["type"] == "text" {
                let font = resolve_font(&clip["text"]).ok_or(FAILED)?;
                let overlay = TextOverlay::prepare(clip, &font)?;
                layers.push(Layer::Text {
                    start: number(&clip["timeline_start"]),
                    overlay,
                });
                continue;
            }
            let Some(raw_path) = clip["asset_path"]
                .as_str()
                .filter(|path| !path.trim().is_empty())
            else {
                continue;
            };
            let asset = media_path(raw_path.trim());
            let ext = asset
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or_default();
            if !["png", "jpg", "jpeg", "webp"]
                .iter()
                .any(|item| ext.eq_ignore_ascii_case(item))
                || !list(&clip["keyframes"]).is_empty()
                || has_unsupported_transition(clip, false)
            {
                return Err(UNAVAILABLE);
            }
            layers.push(Layer::Media {
                asset,
                placement: Placement::from_clip(clip, number(&clip["duration"]), false),
                trim_in: None,
            });
        }
        Ok(Self(layers))
    }

    pub(in crate::native_http::lite_cut) fn sources(&self) -> impl Iterator<Item = &Path> {
        self.0.iter().map(|row| match row {
            Layer::Media { asset, .. } => asset.as_path(),
            Layer::Text { overlay, .. } => overlay.font(),
        })
    }

    pub(in crate::native_http::lite_cut) async fn compose(
        &self,
        media: &Media<'_>,
        base: PathBuf,
        stage: &TempDir,
        fast: bool,
    ) -> Result<PathBuf, &'static str> {
        if self.0.is_empty() {
            return Ok(base);
        }
        let info = probe(media.ffprobe, &base, media.cancel, FAILED).await?;
        let video = list(&info["streams"])
            .iter()
            .find(|stream| stream["codec_type"] == "video")
            .ok_or(FAILED)?;
        let (width, height) = (
            video["width"]
                .as_u64()
                .filter(|value| *value > 0)
                .ok_or(FAILED)?,
            video["height"]
                .as_u64()
                .filter(|value| *value > 0)
                .ok_or(FAILED)?,
        );
        let duration = info["format"]["duration"]
            .as_str()
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value > 0.0)
            .ok_or(FAILED)?;
        let mut previous = base;
        // Decode only the current base and overlay. Keep only the previous
        // and current intermediate file until each encoder finishes.
        for (index, layer) in self.0.iter().enumerate() {
            let (asset, placement, trim_in) = match layer {
                Layer::Media {
                    asset,
                    placement,
                    trim_in,
                } => (asset, placement, *trim_in),
                Layer::Text { start, overlay } => {
                    if *start < duration {
                        let output = overlay
                            .compose(media, stage, index, &previous, fast)
                            .await?;
                        fs::remove_file(&previous).map_err(|_| FAILED)?;
                        previous = output;
                    }
                    continue;
                }
            };
            if placement.start >= duration {
                continue;
            }
            let asset = asset
                .canonicalize()
                .map_err(|_| "MONTAGE_CLIP_FILE_MISSING")?;
            if !asset.is_file() {
                return Err("MONTAGE_CLIP_FILE_MISSING");
            }
            let moving = trim_in.is_some();
            let graph = placement.graph(width, height, moving);
            let output = stage.path().join(format!("overlay-{index}.mp4"));
            let mut process = render_command(media.ffmpeg);
            process.arg("-i").arg(path_text(&previous));
            if let Some(trim_in) = trim_in {
                reject_webm_alpha(media, &asset).await?;
                process.args([
                    "-ss",
                    &format!("{trim_in:.6}"),
                    "-t",
                    &format!("{:.6}", placement.end - placement.start),
                ]);
            } else {
                process.args([
                    "-loop",
                    "1",
                    "-framerate",
                    "60",
                    "-t",
                    &format!("{duration:.6}"),
                ]);
            }
            process
                .arg("-i")
                .arg(path_text(&asset))
                .args(["-filter_complex", &graph, "-map", "[vout]", "-map", "0:a?"])
                .args(super::encoder_args(media.encoder, fast))
                .args(["-c:a", "copy", "-movflags", "+faststart"])
                .arg(path_text(&output));
            run(process, media.cancel, FAILED, Duration::from_secs(3600)).await?;
            fs::remove_file(&previous).map_err(|_| FAILED)?;
            previous = output;
        }
        Ok(previous)
    }
}

fn ordinary_video(clip: &Value) -> bool {
    let color = &clip["color"];
    let neutral_color = color.is_null()
        || (["brightness", "contrast", "saturation"]
            .iter()
            .all(|key| number(&color[*key]) == 0.0)
            && color["filter_preset"]
                .as_str()
                .is_none_or(|preset| preset.trim().is_empty() || preset == "none"));
    let crop = &clip["crop"];
    let neutral_crop = crop.is_null()
        || (number(&crop["x"]) == 0.0
            && number(&crop["y"]) == 0.0
            && number(&crop["width"]) == 1.0
            && number(&crop["height"]) == 1.0);
    neutral_color
        && neutral_crop
        && number(&clip["speed"]) == 1.0
        && clip["reverse"] != true
        && number(&clip["freeze_frame_sec"]) == 0.0
        && ["keyframes", "speed_keyframes"]
            .iter()
            .all(|key| list(&clip[*key]).is_empty())
        && !has_unsupported_transition(clip, true)
}

fn has_unsupported_transition(clip: &Value, allow_fade: bool) -> bool {
    ["transition_in", "transition_out"].iter().any(|key| {
        let transition = &clip[*key];
        transition["duration_sec"].as_f64().unwrap_or_default() > 0.0
            && transition["type"].as_str().is_some_and(|kind| {
                !(matches!(kind, "cut" | "none") || (allow_fade && kind == "fade"))
            })
    })
}

async fn reject_webm_alpha(media: &Media<'_>, path: &Path) -> Result<(), &'static str> {
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("webm"))
    {
        return Ok(());
    }
    // Native VP9 decoding can silently discard a WebM alpha plane.
    let info = probe(media.ffprobe, path, media.cancel, FAILED).await?;
    let video = list(&info["streams"])
        .iter()
        .find(|stream| stream["codec_type"] == "video")
        .ok_or(FAILED)?;
    if video["tags"].as_object().is_some_and(|tags| {
        tags.iter().any(|(key, val)| {
            key.eq_ignore_ascii_case("alpha_mode")
                && val.as_str().is_some_and(|value| value.trim() == "1")
        })
    }) {
        return Err(UNAVAILABLE);
    }
    Ok(())
}

impl Placement {
    fn from_clip(clip: &Value, duration: f64, moving: bool) -> Self {
        let start = number(&clip["timeline_start"]);
        let transform = &clip["transform"];
        let value = |key: &str, default| transform[key].as_f64().unwrap_or(default);
        let scale = value("scale", 1.0);
        let rotation = value("rotation", 0.0);
        let fade = |key, transition: &str| {
            let effect = &clip[transition];
            let transition_duration = if moving && effect["type"] == "fade" {
                number(&effect["duration_sec"])
            } else {
                0.0
            };
            number(&clip[key]).max(transition_duration).min(duration)
        };
        Self {
            start,
            end: start + duration,
            x: value("x", 0.5).clamp(0.0, 1.0),
            y: value("y", 0.5).clamp(0.0, 1.0),
            width: (value("width", 1.0) * scale).clamp(0.01, 10.0),
            height: (value("height", 1.0) * scale).clamp(0.01, 10.0),
            radians: if moving && rotation.abs() <= 0.5 {
                0.0
            } else {
                let rotation = if moving { rotation } else { rotation.rem_euclid(360.0) };
                rotation.to_radians()
            },
            opacity: value("opacity", 1.0),
            fade_in: fade("fade_in_sec", "transition_in"),
            fade_out: fade("fade_out_sec", "transition_out"),
            hflip: clip["flip_horizontal"] == true,
            vflip: clip["flip_vertical"] == true,
        }
    }

    fn graph(&self, canvas_width: u64, canvas_height: u64, moving: bool) -> String {
        let width = (self.width * canvas_width as f64)
            .round_ties_even()
            .max(1.0);
        let height = (self.height * canvas_height as f64)
            .round_ties_even()
            .max(1.0);
        let mut filters = Vec::new();
        if moving {
            filters.push(format!("setpts=PTS-STARTPTS+{:.6}/TB", self.start));
        }
        filters.push(format!("format=rgba,scale={width:.0}:{height:.0}"));
        if !moving {
            self.add_flips(&mut filters);
        }
        if self.radians.abs() > 1e-6 {
            let theta = self.radians;
            filters.push(format!(
                "rotate={theta:.6}:ow=rotw({theta:.6}):oh=roth({theta:.6}):c=none"
            ));
        }
        if moving {
            self.add_flips(&mut filters);
        }
        let opaque = if moving { 0.999 } else { 1.0 };
        if self.opacity < opaque {
            filters.push(format!("colorchannelmixer=aa={:.6}", self.opacity));
        }
        if self.fade_in > 0.0 {
            filters.push(format!(
                "fade=t=in:st={:.6}:d={:.6}:alpha=1",
                self.start, self.fade_in
            ));
        }
        if self.fade_out > 0.0 {
            let start = self.end - self.fade_out;
            filters.push(format!(
                "fade=t=out:st={start:.6}:d={:.6}:alpha=1",
                self.fade_out
            ));
        }
        // Layers have the legacy last-frame hold inside their trim window.
        let eof = if moving { "repeat" } else { "pass" };
        format!(
            "[1:v]{}[layer];[0:v][layer]overlay=x=main_w*{:.6}-overlay_w/2:y=main_h*{:.6}-overlay_h/2:enable='between(t,{:.6},{:.6})':eof_action={eof},format=yuv420p[vout]",
            filters.join(","), self.x, self.y, self.start, self.end,
        )
    }

    fn add_flips(&self, filters: &mut Vec<String>) {
        if self.hflip {
            filters.push("hflip".to_owned());
        }
        if self.vflip {
            filters.push("vflip".to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_http::lite_cut::project_schema::normalize;
    use serde_json::json;
    use std::process::Command;

    fn video_project(top: Value, lower: Value) -> Value {
        normalize(Some(json!({
            "tracks":[
                {"id":"top","type":"video","label":"Top","clips":[top]},
                {"id":"hidden","type":"video","label":"Hidden","hidden":true,
                    "clips":[{"id":"unused","source_id":99,"speed":2.0}]},
                {"id":"lower","type":"video","label":"Lower","clips":[lower]},
                {"id":"base","type":"video","label":"Base",
                    "clips":[{"id":"base","source_type":"file","file_path":"base.mp4"}]}
            ]
        })))
        .unwrap()
    }

    #[test]
    fn upper_video_orders_recorded_and_local_layers_without_silently_dropping_effects() {
        let top = json!({"id":"top","source_type":"file","file_path":"top.mp4","timeline_start":0.8});
        let lower = json!({"id":"lower","source_id":42,"timeline_start":0.9});
        let mut project = video_project(top, lower);
        let plan = Plan::from_project(
            &project,
            3,
            |id| {
                assert_eq!(id, 42);
                Some(PathBuf::from("recording.mp4"))
            },
            |_| None,
        )
        .unwrap();
        assert_eq!(
            plan.sources().collect::<Vec<_>>(),
            [Path::new("recording.mp4"), Path::new("top.mp4")]
        );
        project["tracks"][0]["clips"][0]["speed"] = json!(1.5);
        assert!(matches!(
            Plan::from_project(&project, 3, |_| Some(PathBuf::from("recording.mp4")), |_| None),
            Err(UNAVAILABLE)
        ));
    }

    #[test]
    #[ignore = "requires the verified FFmpeg pair and an owned scratch directory"]
    fn ffmpeg_upper_video_uses_source_trim_layer_order_fades_and_copies_base_aac() {
        let ffmpeg = PathBuf::from(std::env::var_os("MGS_LITECUT_VIDEO_TEST_FFMPEG").unwrap());
        let ffprobe = ffmpeg.with_file_name(if cfg!(windows) { "ffprobe.exe" } else { "ffprobe" });
        let scratch = PathBuf::from(std::env::var_os("MGS_LITECUT_VIDEO_TEST_SCRATCH").unwrap());
        let stage = tempfile::Builder::new()
            .prefix("upper-video-")
            .tempdir_in(scratch)
            .unwrap();
        let base = stage.path().join("base.mp4");
        let green = stage.path().join("green.mp4");
        let red = stage.path().join("red.mp4");

        let mut command = Command::new(&ffmpeg);
        command.args([
            "-nostdin", "-n", "-v", "error", "-f", "lavfi", "-i",
            "color=blue:s=96x64:r=20:d=3",
            "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:duration=3",
            "-c:v", "libx264", "-c:a", "aac",
        ]).arg(&base);
        checked_output(command);
        for (path, filter) in [
            (&green, "color=lime:s=64x32:r=20:d=2.4"),
            (&red, concat!(
                "color=red:s=64x32:r=20:d=2.4",
                ",drawbox=x=0:y=0:w=32:h=32:c=white:t=fill:enable='between(t,0.6,1.14)'",
                ",drawbox=x=32:y=0:w=32:h=32:c=white:t=fill:enable='gte(t,1.15)'",
            )),
        ] {
            let mut command = Command::new(&ffmpeg);
            command.args(["-nostdin", "-n", "-v", "error", "-f", "lavfi", "-i", filter,
                "-c:v", "libx264"]).arg(path);
            checked_output(command);
        }

        let audio = |path: &Path| {
            let mut command = Command::new(&ffmpeg);
            command.args(["-nostdin", "-v", "error", "-i"]).arg(path)
                .args(["-map", "0:a:0", "-c", "copy", "-f", "md5", "-"]);
            checked_output(command)
        };
        let original_audio = audio(&base);
        let project = video_project(
            json!({"id":"top","source_type":"file","file_path":path_text(&red),"timeline_start":0.8,
                "trim_in":0.6,"trim_out":1.6,"fade_in_sec":0.2,"fade_out_sec":0.2,
                "transform":{"width":0.5,"height":0.75,"opacity":0.75}}),
            json!({"id":"lower","source_id":42,"timeline_start":0.4,"trim_out":1.8}),
        );
        let plan = Plan::from_project(&project, 3, |_| Some(green.clone()), |_| None).unwrap();
        let (_sender, signal) = tokio::sync::watch::channel(false);
        let media = Media {
            ffmpeg: &ffmpeg,
            ffprobe: &ffprobe,
            encoder: "libx264",
            cancel: &signal,
        };
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let result = runtime
            .block_on(plan.compose(&media, base.clone(), &stage, true))
            .unwrap();
        assert!(!base.exists());
        assert!(!stage.path().join("overlay-0.mp4").exists());
        assert_eq!(audio(&result), original_audio);

        let mut command = Command::new(&ffmpeg);
        command.args(["-nostdin", "-v", "error", "-i"]).arg(result)
            .args(["-map", "0:v:0", "-pix_fmt", "rgb24", "-f", "rawvideo", "-"]);
        let rgb = checked_output(command);
        assert_eq!(rgb.len(), 60 * 96 * 64 * 3);
        let pixel = |frame: usize, x: usize, y: usize| {
            let offset = (((frame * 64) + y) * 96 + x) * 3;
            [rgb[offset], rgb[offset + 1], rgb[offset + 2]]
        };
        assert!(pixel(3, 35, 32)[2] > 220);
        assert!(pixel(13, 35, 32)[1] > 220);
        assert!(pixel(22, 12, 32)[1] > 220);
        assert!(pixel(22, 35, 12)[2] > 130);
        assert!(pixel(18, 35, 32)[2] < pixel(22, 35, 32)[2]);
        assert!(pixel(22, 60, 32)[2] < 25);
        assert!(pixel(29, 35, 32)[2] < 25);
        assert!(pixel(29, 60, 32)[2] > 130);
        assert!(pixel(34, 60, 32)[2] < pixel(29, 60, 32)[2]);
        assert!(pixel(39, 60, 32)[1] > 220);
        assert!(pixel(47, 60, 32)[2] > 220);
    }

    fn checked_output(mut command: Command) -> Vec<u8> {
        let output = command.output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        output.stdout
    }
}
