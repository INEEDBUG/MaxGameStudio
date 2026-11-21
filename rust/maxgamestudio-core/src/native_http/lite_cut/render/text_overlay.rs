//! Draw an ordinary text overlay using only owned FFmpeg filter inputs.

use super::{super::ffmpeg::command, encoder_args, number, run, Media, CANCELLED};
use crate::profile_platform::path_text;
use serde_json::Value;
use std::{
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use tempfile::TempDir;

const FAILED: &str = "MONTAGE_EXPORT_FAILED";
const MAX_TEXT_BYTES: usize = 1024 * 1024;
const MAX_FONT_BYTES: u64 = 64 * 1024 * 1024;

pub(in crate::native_http::lite_cut) struct TextOverlay {
    font: PathBuf,
    content: String,
    options: String,
}

#[derive(Clone, Copy)]
enum Slide {
    Left,
    Right,
    Up,
    Down,
}

impl TextOverlay {
    pub(in crate::native_http::lite_cut) fn prepare(
        overlay: &Value,
        font: &Path,
    ) -> Result<Self, &'static str> {
        if overlay["type"] != "text" {
            return Err(FAILED);
        }
        let text = &overlay["text"];
        let meta = &overlay["meta"];
        let content = first_text(&text["content"], &meta["name"], "Text");
        if content.len() > MAX_TEXT_BYTES || content.contains('\0') {
            return Err(FAILED);
        }
        let preset = first_text(&text["preset_id"], &meta["textStyleId"], "plain")
            .trim()
            .to_ascii_lowercase();
        let color = match preset.as_str() {
            "ace" => "0xfbbf24",
            "clutch" => "0x67e8f9",
            "creator" => "0xfde047",
            "retro" => "0xf0abfc",
            "bubble" => "0x111827",
            _ => "white",
        };
        let transform = &overlay["transform"];
        let tx = number(&transform["x"]).clamp(0.0, 1.0);
        let ty = number(&transform["y"]).clamp(0.0, 1.0);
        let scale = number(&transform["scale"]).clamp(0.1, 4.0);
        let opacity = number(&transform["opacity"]);
        let font_size = text["font_size"]
            .as_f64()
            .filter(|size| *size != 0.0)
            .unwrap_or(64.0);
        let size = (scale * font_size).round_ties_even().clamp(1.0, 2000.0);

        let start = number(&overlay["timeline_start"]);
        let duration = number(&overlay["duration"]).max(0.1);
        let end = start + duration;
        let mut x = format!("w*{tx:.6}-text_w/2");
        let mut y = format!("h*{ty:.6}-text_h/2");
        let anim_in = text["anim_in"].as_str().unwrap_or_default().trim();
        let anim_out = text["anim_out"].as_str().unwrap_or_default().trim();
        let anim_time = duration.min(0.45);
        let mut fade_in = number(&overlay["fade_in_sec"]).min(duration);
        let mut fade_out = number(&overlay["fade_out_sec"]).min(duration);
        for (anim, entering, fade) in [
            (anim_in, true, &mut fade_in),
            (anim_out, false, &mut fade_out),
        ] {
            if let Some(slide) = Slide::from_name(anim) {
                slide.apply(
                    &mut x,
                    &mut y,
                    anim_time,
                    if entering { start } else { end },
                    entering,
                );
            } else if anim.eq_ignore_ascii_case("fade") {
                *fade = fade.max(anim_time);
            }
        }
        for (spec, entering, fade) in [
            (&overlay["transition_in"], true, &mut fade_in),
            (&overlay["transition_out"], false, &mut fade_out),
        ] {
            if !spec.is_object() {
                continue;
            }
            let time = number(&spec["duration_sec"]).min(duration);
            if time > 0.0 {
                if let Some(slide) = spec["type"].as_str().and_then(Slide::from_name) {
                    slide.apply(
                        &mut x,
                        &mut y,
                        time,
                        if entering { start } else { end },
                        entering,
                    );
                } else {
                    *fade = fade.max(time);
                }
            }
        }
        let mut options = format!(
            "fontsize={size:.0}:x='{x}':y='{y}':enable='between(t,{start:.4},{end:.4})':fontcolor={color}:borderw=3:bordercolor=black@0.72"
        );
        if preset == "bubble" {
            options.push_str(":box=1:boxcolor=white@0.95:boxborderw=18");
        }
        if opacity < 0.999 || fade_in > 0.0 || fade_out > 0.0 {
            let mut alpha = format!("{opacity:.6}");
            if fade_in > 0.0 {
                let fade_end = start + fade_in;
                alpha = format!(
                    "if(lt(t,{fade_end:.6}),{opacity:.6}*(t-{start:.6})/{fade_in:.6},{alpha})"
                );
            }
            if fade_out > 0.0 {
                let fade_start = (end - fade_out).max(start);
                alpha = format!(
                    "if(gt(t,{fade_start:.6}),{opacity:.6}*({end:.6}-t)/{fade_out:.6},{alpha})"
                );
            }
            options.push_str(&format!(":alpha='{alpha}'"));
        }
        Ok(Self {
            font: font.to_owned(),
            content: content.to_owned(),
            options,
        })
    }

    pub(in crate::native_http::lite_cut) fn font(&self) -> &Path {
        &self.font
    }

    /// Compose against the full staged timeline. The caller owns compositing
    /// order and the returned file, and can retire the previous pass afterward.
    pub(in crate::native_http::lite_cut) async fn compose(
        &self,
        media: &Media<'_>,
        stage: &TempDir,
        index: usize,
        base: &Path,
        fast: bool,
    ) -> Result<PathBuf, &'static str> {
        if *media.cancel.borrow() {
            return Err(CANCELLED);
        }
        let output = stage.path().join(format!("text-{index}.mp4"));
        match output.symlink_metadata() {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            _ => return Err(FAILED),
        }
        let base = base.canonicalize().map_err(|_| FAILED)?;
        let inputs = tempfile::Builder::new()
            .prefix("text-input-")
            .tempdir_in(stage.path())
            .map_err(|_| FAILED)?;
        let font_name = self.stage_inputs(&inputs)?;
        let graph = self.graph(font_name);
        let mut process = command(media.ffmpeg);
        // Working in the owned scratch directory keeps arbitrary filesystem
        // paths out of drawtext's grammar, even on a Unicode Windows path.
        process
            .current_dir(inputs.path())
            .args(["-nostdin", "-n", "-hide_banner", "-loglevel", "error", "-i"])
            .arg(path_text(&base))
            .args(["-filter_complex", &graph, "-map", "[vout]", "-map", "0:a?"])
            .args(encoder_args(media.encoder, fast))
            .args(["-c:a", "copy", "-movflags", "+faststart"])
            .arg(Path::new("..").join(output.file_name().ok_or(FAILED)?));
        run(process, media.cancel, FAILED, Duration::from_secs(3600)).await?;
        if !fs::symlink_metadata(&output).is_ok_and(|meta| meta.is_file() && meta.len() > 0) {
            return Err(FAILED);
        }
        Ok(output)
    }

    fn stage_inputs(&self, inputs: &TempDir) -> Result<&'static str, &'static str> {
        let font = File::open(self.font()).map_err(|_| "MONTAGE_SOURCE_NOT_READABLE")?;
        let meta = font.metadata().map_err(|_| "MONTAGE_SOURCE_NOT_READABLE")?;
        if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_FONT_BYTES {
            return Err(FAILED);
        }
        let extension = self
            .font
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let name = match extension.to_ascii_lowercase().as_str() {
            "otf" => "font.otf",
            "ttc" => "font.ttc",
            "woff" => "font.woff",
            "woff2" => "font.woff2",
            _ => "font.ttf",
        };
        let mut staged_font = File::create_new(inputs.path().join(name)).map_err(|_| FAILED)?;
        let copied =
            io::copy(&mut font.take(MAX_FONT_BYTES + 1), &mut staged_font).map_err(|_| FAILED)?;
        if copied != meta.len() {
            return Err(FAILED);
        }
        File::create_new(inputs.path().join("caption.txt"))
            .and_then(|mut file| file.write_all(self.content.as_bytes()))
            .map_err(|_| FAILED)?;
        Ok(name)
    }

    fn graph(&self, font_name: &'static str) -> String {
        format!(
            "[0:v:0]drawtext=fontfile={font_name}:textfile=caption.txt:expansion=none:{}[vout]",
            self.options
        )
    }
}

impl Slide {
    fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "slide_left" => Some(Self::Left),
            "slide_right" => Some(Self::Right),
            "slide_up" => Some(Self::Up),
            "slide_down" => Some(Self::Down),
            _ => None,
        }
    }

    fn apply(self, x: &mut String, y: &mut String, duration: f64, edge: f64, entering: bool) {
        let (expr, offset) = match self {
            Self::Left | Self::Right => (x, "w*0.120000"),
            Self::Up | Self::Down => (y, "h*0.100000"),
        };
        let positive_entry = matches!(self, Self::Left | Self::Up);
        let sign = if positive_entry == entering { "+" } else { "-" };
        if entering {
            let end = edge + duration;
            *expr = format!(
                "if(lt(t,{end:.6}),{expr}{sign}{offset}*(1-(t-{edge:.6})/{duration:.6}),{expr})"
            );
        } else {
            let start = edge - duration;
            *expr = format!(
                "if(gt(t,{start:.6}),{expr}{sign}{offset}*((t-{start:.6})/{duration:.6}),{expr})"
            );
        }
    }
}

fn first_text<'a>(first: &'a Value, second: &'a Value, fallback: &'a str) -> &'a str {
    first
        .as_str()
        .filter(|text| !text.is_empty())
        .or_else(|| second.as_str().filter(|text| !text.is_empty()))
        .unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_http::lite_cut::project_schema::normalize;
    use serde_json::{json, Value};

    fn overlay(clip: Value) -> Value {
        normalize(Some(json!({"overlays":[clip]}))).unwrap()["overlays"][0].clone()
    }

    #[test]
    fn slides_and_fades_use_absolute_times_with_literal_content_and_fixed_files() {
        let row = overlay(json!({
            "id":"subtitle","type":"text","timeline_start":2.0,"duration":4.0,
            "transition_in":{"type":"slide_left","duration_sec":0.6},
            "transition_out":{"type":"wipe_l","duration_sec":0.5},
            "transform":{"x":0.5,"y":0.3,"opacity":0.8,"scale":1.5},
            "text":{"content":"进度 %{n}: '100%'\n下一行","font_size":40,"preset_id":"clutch"}
        }));
        let prepared = TextOverlay::prepare(&row, Path::new("任意'字体:不可插入.ttf")).unwrap();
        let graph = prepared.graph("font.ttf");
        assert!(graph.contains("fontfile=font.ttf:textfile=caption.txt:expansion=none"));
        assert!(graph.contains("fontsize=60"));
        assert!(graph.contains("enable='between(t,2.0000,6.0000)'"));
        assert!(graph.contains("x='if(lt(t,2.600000),w*0.500000-text_w/2+w*0.120000*"));
        assert!(graph.contains("y='h*0.300000-text_h/2'"));
        assert!(
            graph.contains("alpha='if(gt(t,5.500000),0.800000*(6.000000-t)/0.500000,0.800000)'")
        );
        assert!(graph.contains("fontcolor=0x67e8f9:borderw=3:bordercolor=black@0.72"));
        assert!(!graph.contains("进度"));
        assert!(!graph.contains("任意"));
        assert_eq!(prepared.content, "进度 %{n}: '100%'\n下一行");
    }

    #[test]
    fn fallback_label_bubble_preset_and_fade_preset_follow_legacy_behavior() {
        let row = overlay(json!({
            "id":"fallback","type":"text","timeline_start":1.0,"duration":3.0,
            "text":{"content":"","anim_in":"fade"},
            "meta":{"name":"字幕","textStyleId":" bubble "}
        }));
        let prepared = TextOverlay::prepare(&row, Path::new("local.ttf")).unwrap();
        let graph = prepared.graph("font.ttf");
        assert_eq!(prepared.content, "字幕");
        assert!(graph.contains("fontcolor=0x111827"));
        assert!(graph.contains("box=1:boxcolor=white@0.95:boxborderw=18"));
        assert!(
            graph.contains("alpha='if(lt(t,1.450000),1.000000*(t-1.000000)/0.450000,1.000000)'")
        );
    }

    #[test]
    fn stages_literal_utf8_and_rejects_unrepresentable_or_oversized_content() {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("字样.OTF");
        fs::write(&original, b"fixture font bytes").unwrap();
        let content = "中文：'100%' %{pts}\\方向\n第二行";
        let row = overlay(json!({
            "id":"literal","type":"text","text":{"content":content}
        }));
        let prepared = TextOverlay::prepare(&row, &original).unwrap();
        let stage = tempfile::tempdir_in(root.path()).unwrap();
        assert_eq!(prepared.stage_inputs(&stage).unwrap(), "font.otf");
        assert_eq!(
            fs::read(stage.path().join("caption.txt")).unwrap(),
            content.as_bytes()
        );
        assert_eq!(
            fs::read(stage.path().join("font.otf")).unwrap(),
            b"fixture font bytes"
        );
        assert_eq!(prepared.stage_inputs(&stage), Err(FAILED));
        assert_eq!(fs::read(&original).unwrap(), b"fixture font bytes");

        for invalid in ["a\0b".to_owned(), "x".repeat(MAX_TEXT_BYTES + 1)] {
            let row = overlay(json!({
                "id":"invalid","type":"text","text":{"content":invalid}
            }));
            assert!(matches!(TextOverlay::prepare(&row, &original), Err(FAILED)));
        }
    }

    #[test]
    #[ignore = "requires the verified FFmpeg pair and a trusted Chinese test font"]
    fn ffmpeg_renders_chinese_and_percent_literally_and_keeps_the_absolute_window() {
        let ffmpeg = PathBuf::from(std::env::var_os("MGS_LITECUT_TEXT_TEST_FFMPEG").unwrap());
        let ffprobe = ffmpeg.with_file_name(if cfg!(windows) {
            "ffprobe.exe"
        } else {
            "ffprobe"
        });
        let scratch = PathBuf::from(std::env::var_os("MGS_LITECUT_TEXT_TEST_SCRATCH").unwrap());
        let font = PathBuf::from(std::env::var_os("MGS_LITECUT_TEXT_TEST_FONT").unwrap());
        let stage = tempfile::Builder::new()
            .prefix("文字 '[测试]' ")
            .tempdir_in(scratch)
            .unwrap();
        let base = stage.path().join("合成视频.mp4");
        let base_output = std::process::Command::new(&ffmpeg)
            .args([
                "-nostdin",
                "-n",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=black:size=192x108:rate=12:duration=2",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=2",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-t",
                "2",
            ])
            .arg(&base)
            .output()
            .unwrap();
        assert!(base_output.status.success());
        let base_bytes = fs::read(&base).unwrap();
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
        let row = |content| {
            overlay(json!({
                "id":"actual","type":"text","timeline_start":0.5,"duration":1.0,
                "fade_in_sec":0.25,"fade_out_sec":0.25,
                "transition_in":{"type":"slide_left","duration_sec":0.5},
                "transform":{"x":0.5,"y":0.5,"opacity":0.8},
                "text":{"content":content,"font_size":18,"preset_id":"clutch"}
            }))
        };
        let prepared = TextOverlay::prepare(&row("中文 100% %{n}\n第二行"), &font).unwrap();
        let result = runtime
            .block_on(prepared.compose(&media, &stage, 0, &base, true))
            .unwrap();
        let pixels = decode(&ffmpeg, &result, false);
        const FRAME: usize = 192 * 108 * 3;
        assert_eq!(pixels.len(), FRAME * 24);
        let energy = |frame| {
            pixels[frame * FRAME..][..FRAME]
                .iter()
                .map(|p| u64::from(*p))
                .sum::<u64>()
        };
        assert_eq!(energy(4), 0);
        assert_eq!(energy(20), 0);
        assert!(energy(12) > 15_000);
        assert!(energy(7) > 0 && energy(7) < energy(12));
        assert!(energy(17) > 0 && energy(17) < energy(12));
        assert_eq!(decode(&ffmpeg, &result, true), decode(&ffmpeg, &base, true));
        let result_bytes = fs::read(&result).unwrap();
        assert_eq!(
            runtime.block_on(prepared.compose(&media, &stage, 0, &base, true)),
            Err(FAILED)
        );
        assert_eq!(fs::read(&result).unwrap(), result_bytes);
        assert_eq!(fs::read_dir(stage.path()).unwrap().count(), 2);
        assert_eq!(fs::read(&base).unwrap(), base_bytes);

        if let Some(reference) = std::env::var_os("MGS_LITECUT_TEXT_TEST_REFERENCE") {
            // Reference graph generated from the released composer for this
            // simple literal, which avoids its special-character escaping.
            let plain = TextOverlay::prepare(&row("ACE"), &font).unwrap();
            let plain_result = runtime
                .block_on(plain.compose(&media, &stage, 1, &base, true))
                .unwrap();
            let reference_dir = tempfile::tempdir_in(stage.path()).unwrap();
            let name = plain.stage_inputs(&reference_dir).unwrap();
            let graph = fs::read_to_string(reference)
                .unwrap()
                .replace("FONTFILE", name);
            let reference_path = reference_dir.path().join("legacy.mp4");
            let output = std::process::Command::new(&ffmpeg)
                .current_dir(reference_dir.path())
                .args(["-nostdin", "-n", "-hide_banner", "-loglevel", "error", "-i"])
                .arg(&base)
                .args([
                    "-filter_complex",
                    graph.trim(),
                    "-map",
                    "[vout]",
                    "-map",
                    "0:a?",
                ])
                .args(encoder_args(media.encoder, true))
                .args(["-c:a", "copy", "-movflags", "+faststart", "legacy.mp4"])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                decode(&ffmpeg, &plain_result, false) == decode(&ffmpeg, &reference_path, false)
            );
        }

        cancel.send_replace(true);
        assert_eq!(
            runtime.block_on(prepared.compose(&media, &stage, 2, &base, true)),
            Err(CANCELLED)
        );
        assert!(!stage.path().join("text-2.mp4").exists());
    }

    fn decode(ffmpeg: &Path, input: &Path, audio: bool) -> Vec<u8> {
        let mut process = std::process::Command::new(ffmpeg);
        process
            .args(["-hide_banner", "-loglevel", "error", "-i"])
            .arg(input);
        if audio {
            process.args(["-map", "0:a:0", "-c:a", "copy", "-f", "adts", "-"]);
        } else {
            process.args(["-an", "-pix_fmt", "rgb24", "-f", "rawvideo", "-"]);
        }
        let output = process.output().unwrap();
        assert!(output.status.success());
        output.stdout
    }
}
