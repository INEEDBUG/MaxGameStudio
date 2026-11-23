use super::{list, Timeline};
use crate::native_http::lite_cut::curve;
use serde_json::Value;

pub(super) fn place(timeline: &Timeline<'_>, clip: &Value, fitted: &str, duration: f64) -> String {
    let Timeline { width, height, fps, background, .. } = timeline;
    let base = &clip["transform"];
    let frames = list(&clip["keyframes"]);
    let position = |field| {
        let baseline = value(base, field, 0.5).clamp(0.0, 1.0);
        motion(frames, duration, "t", baseline, |item| {
            value(item, field, baseline).clamp(0.0, 1.0)
        })
    };
    let size = |field| {
        let baseline = value(base, field, 1.0).clamp(0.01, 3.0)
            * value(base, "scale", 1.0).clamp(0.1, 3.0);
        motion(frames, duration, "t", baseline, |item| {
            let scale = value(item, "scale", 1.0);
            let fraction = value(item, field, baseline / scale.max(0.01));
            (fraction * scale).clamp(0.01, 10.0)
        })
    };
    let fallback_angle = value(base, "rotation", 0.0).clamp(-180.0, 180.0);
    let angle = motion(frames, duration, "t", fallback_angle, |item| {
        value(item, "rotation", fallback_angle)
    });
    let fallback_opacity = value(base, "opacity", 1.0).clamp(0.0, 1.0);
    let opacity = motion(frames, duration, "T", fallback_opacity, |item| {
        value(item, "opacity", fallback_opacity).clamp(0.0, 1.0)
    });
    let x = position("x");
    let y = position("y");
    let horizontal = size("width");
    let vertical = size("height");

    let scale = if horizontal.animated || vertical.animated {
        format!(
            "scale=w='max(2,trunc({width}*({})/2)*2)':h='max(2,trunc({height}*({})/2)*2)':eval=frame",
            horizontal.expr, vertical.expr
        )
    } else {
        let even = |canvas: u64, fraction: f64| ((canvas as f64 * fraction / 2.0).round_ties_even() * 2.0).max(2.0);
        format!("scale={:.0}:{:.0}", even(*width, horizontal.initial), even(*height, vertical.initial))
    };
    let alpha = if opacity.animated {
        format!("geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='alpha(X,Y)*({})'", opacity.expr)
    } else {
        format!("colorchannelmixer=aa={:.6}", opacity.initial)
    };
    let rotation = if angle.animated {
        format!(",rotate=angle='({})*PI/180':c=none:ow='hypot(iw,ih)':oh='hypot(iw,ih)'", angle.expr)
    } else if angle.initial.abs() > 0.001 {
        let theta = angle.initial.to_radians();
        format!(",rotate={theta:.8}:c=none:ow=rotw({theta:.8}):oh=roth({theta:.8})")
    } else {
        String::new()
    };
    let evaluate = if [&x, &y, &horizontal, &vertical, &angle, &opacity].iter().any(|part| part.animated) {
        "frame"
    } else {
        "init"
    };
    format!(
        "{fitted},{scale},format=rgba,{alpha}{rotation}[object];color=c={background}:s={width}x{height}:r={fps}:d={duration:.6}[canvas];[canvas][object]overlay=x='W*({})-w/2':y='H*({})-h/2':eval={evaluate}:shortest=1,format=yuv420p[vout]",
        x.expr, y.expr
    )
}

struct Motion {
    expr: String,
    initial: f64,
    animated: bool,
}

fn motion(
    frames: &[Value],
    duration: f64,
    time: &str,
    fallback: f64,
    transform: impl Fn(&Value) -> f64,
) -> Motion {
    let mut points: Vec<_> = frames.iter().filter_map(|row| {
        let item = &row["transform"];
        item.is_object().then(|| (value(row, "time_sec", 0.0).clamp(0.0, duration), transform(item)))
    }).collect();
    points.sort_by(|left, right| left.0.total_cmp(&right.0));
    points.dedup_by(|later, earlier| {
        if (later.0 - earlier.0).abs() < 1e-6 {
            *earlier = *later;
            true
        } else {
            false
        }
    });
    let Some(&(start, initial)) = points.first() else {
        return Motion { expr: format!("{fallback:.6}"), initial: fallback, animated: false };
    };
    let animated = points.iter().skip(1).any(|&(_, value)| (value - initial).abs() > 1e-7);
    let expr = if animated {
        if start > 1e-6 {
            points.insert(0, (0.0, initial));
        }
        let (end, last) = *points.last().expect("animated transform points");
        let linear = curve::interpolate(&points, time);
        format!("if(lt({time},{end:.6}),{linear},{last:.6})")
    } else {
        format!("{initial:.6}")
    };
    Motion { expr, initial, animated }
}

fn value(object: &Value, field: &str, fallback: f64) -> f64 {
    let raw = &object[field];
    raw.as_f64().or_else(|| raw.as_str()?.parse::<f64>().ok())
        .filter(|value| value.is_finite()).unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::process::Command;

    #[test]
    #[ignore = "requires the pinned FFmpeg media tool"]
    fn ffmpeg_base_canvas_places_animated_position_size_and_opacity() {
        let ffmpeg = std::env::var_os("MGS_LITECUT_VIDEO_TEST_FFMPEG").unwrap();
        let project = json!({"output":{
            "width":320,"height":180,"fps":10,"canvas_fit":"contain",
            "background_color":"#000000","blur_amount":24,"encoder_tier":"quality"
        }});
        let timeline = Timeline::from_project(&project);
        let clip = json!({
            "transform":{"x":0.5,"y":0.5,"width":0.3,"height":0.3},
            "keyframes":[
                {"time_sec":0.2,"transform":{"x":0.25,"width":0.3,"opacity":0.5}},
                {"time_sec":0.8,"transform":{"x":0.75,"width":0.4,"opacity":0.8}},
            ]
        });
        let graph = place(&timeline, &clip, "[0:v]setpts=PTS-STARTPTS,format=yuv420p", 1.0);
        let video = Command::new(ffmpeg).args([
            "-nostdin", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i",
            "color=c=red:s=320x180:r=10:d=1", "-filter_complex_threads", "1",
            "-filter_complex", &graph, "-map", "[vout]", "-pix_fmt", "rgb24",
            "-frames:v", "10", "-f", "rawvideo", "pipe:1",
        ]).output().unwrap();
        assert!(video.status.success(), "{}", String::from_utf8_lossy(&video.stderr));
        assert_eq!(video.stdout.len(), 320 * 180 * 3 * 10);
        let red = |frame, x: usize| video.stdout[((frame * 180 + 90) * 320 + x) * 3];
        assert!((116..=138).contains(&red(1, 80)));
        assert!(red(1, 240) < 5);
        assert!(red(8, 80) < 5);
        assert!((190..=220).contains(&red(8, 240)));
    }
}
