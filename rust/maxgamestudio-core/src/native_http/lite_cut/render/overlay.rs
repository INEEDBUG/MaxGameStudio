//! Still stickers use the timeline's canvas and stacking order.

use super::{number, probe, render_command, run, Media};
use crate::native_http::lite_cut::{export_sources::media_path, project_schema::list};
use crate::profile_platform::path_text;
use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

const FAILED: &str = "MONTAGE_EXPORT_FAILED";

pub(in crate::native_http::lite_cut) struct Plan(Vec<Sticker>);

struct Sticker {
    asset: PathBuf,
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
    pub(in crate::native_http::lite_cut) fn from_project(project: &Value) -> Result<Self, ()> {
        let hidden: HashSet<_> = list(&project["overlay_tracks"])
            .iter()
            .filter(|track| track["hidden"] == true)
            .filter_map(|track| track["id"].as_str())
            .collect();
        let mut stickers = Vec::new();
        for clip in list(&project["overlays"]) {
            let track = clip["meta"]["overlay_track_id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .unwrap_or("ot1");
            if hidden.contains(track) {
                continue;
            }
            if clip["type"] == "text" {
                return Err(());
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
                || ["transition_in", "transition_out"].iter().any(|key| {
                    let transition = &clip[*key];
                    transition["duration_sec"].as_f64().unwrap_or_default() > 0.0
                        && transition["type"]
                            .as_str()
                            .is_some_and(|kind| !matches!(kind, "cut" | "none"))
                })
            {
                return Err(());
            }
            let start = number(&clip["timeline_start"]);
            let duration = number(&clip["duration"]);
            let transform = &clip["transform"];
            let scale = number(&transform["scale"]);
            stickers.push(Sticker {
                asset,
                start,
                end: start + duration,
                x: number(&transform["x"]).clamp(0.0, 1.0),
                y: number(&transform["y"]).clamp(0.0, 1.0),
                width: (number(&transform["width"]) * scale).clamp(0.01, 10.0),
                height: (number(&transform["height"]) * scale).clamp(0.01, 10.0),
                radians: number(&transform["rotation"])
                    .rem_euclid(360.0)
                    .to_radians(),
                opacity: number(&transform["opacity"]),
                fade_in: number(&clip["fade_in_sec"]).min(duration),
                fade_out: number(&clip["fade_out_sec"]).min(duration),
                hflip: clip["flip_horizontal"] == true,
                vflip: clip["flip_vertical"] == true,
            });
        }
        Ok(Self(stickers))
    }

    pub(in crate::native_http::lite_cut) fn sources(&self) -> impl Iterator<Item = &Path> {
        self.0.iter().map(|row| row.asset.as_path())
    }

    pub(in crate::native_http::lite_cut) async fn compose(
        &self,
        media: &Media<'_>,
        base: PathBuf,
        stage: &Path,
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
        // Each pass retains only the current base and one image in memory.
        // Only the last encoded pass is needed after its successor completes.
        for (index, sticker) in self.0.iter().enumerate() {
            if sticker.start >= duration {
                continue;
            }
            let asset = sticker
                .asset
                .canonicalize()
                .map_err(|_| "MONTAGE_CLIP_FILE_MISSING")?;
            if !asset.is_file() {
                return Err("MONTAGE_CLIP_FILE_MISSING");
            }
            let graph = sticker.graph(width, height);
            let output = stage.join(format!("sticker-{index}.mp4"));
            let mut process = render_command(media.ffmpeg);
            process
                .arg("-i")
                .arg(path_text(&previous))
                .args([
                    "-loop",
                    "1",
                    "-framerate",
                    "60",
                    "-t",
                    &format!("{duration:.6}"),
                    "-i",
                ])
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

impl Sticker {
    fn graph(&self, canvas_width: u64, canvas_height: u64) -> String {
        let width = (self.width * canvas_width as f64)
            .round_ties_even()
            .max(1.0);
        let height = (self.height * canvas_height as f64)
            .round_ties_even()
            .max(1.0);
        let mut filters = vec![format!("format=rgba,scale={width:.0}:{height:.0}")];
        if self.hflip {
            filters.push("hflip".to_owned());
        }
        if self.vflip {
            filters.push("vflip".to_owned());
        }
        if self.radians.abs() > 1e-6 {
            let theta = self.radians;
            filters.push(format!(
                "rotate={theta:.6}:ow=rotw({theta:.6}):oh=roth({theta:.6}):c=none"
            ));
        }
        if self.opacity < 1.0 {
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
        format!(
            "[1:v]{}[sticker];[0:v][sticker]overlay=x=main_w*{:.6}-overlay_w/2:y=main_h*{:.6}-overlay_h/2:enable='between(t,{:.6},{:.6})':eof_action=pass,format=yuv420p[vout]",
            filters.join(","), self.x, self.y, self.start, self.end,
        )
    }
}
