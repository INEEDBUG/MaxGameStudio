use super::{json_reply, profile_io_error, profile_work, HttpResponse, ProfileState};
use crate::{obs_wire::ObsRpc, profile_platform::path_text};
use hyper::StatusCode;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    env, fs,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::time::timeout;

pub(super) async fn respond(profile: &Arc<ProfileState>) -> HttpResponse {
    let local = profile_work(profile, |profile| {
        Ok::<_, std::io::Error>((
            profile.config.obs_settings()?,
            LocalStatus::read(profile.config.data_dir()),
        ))
    })
    .await;
    let (settings, mut local) = match local {
        Ok(Ok(values)) => values,
        Ok(Err(error)) => return profile_io_error(&error),
        Err(error) => return profile_io_error(&error),
    };
    let result = timeout(Duration::from_secs(5), async {
        let mut obs = ObsRpc::connect(settings).await?;
        local.reply["obs_connected"] = json!(true);
        fill_live(&mut obs, &mut local).await
    })
    .await;
    match result {
        Ok(Ok(())) => {}
        Ok(Err(_)) => local.reply["ws_error"] = json!("无法读取 OBS WebSocket 状态"),
        Err(_) => local.reply["ws_error"] = json!("OBS WebSocket 状态查询超时"),
    }
    json_reply(StatusCode::OK, local.reply)
}

struct LocalStatus {
    reply: Value,
    ini: Ini,
}

impl LocalStatus {
    fn read(data: &Path) -> Self {
        let root = obs_studio_root();
        let global = root
            .as_ref()
            .map_or_else(Ini::default, |root| Ini::read(&root.join("global.ini")));
        let profile = global.global("currentprofile");
        let directory = global.global("currentprofiledir").or(profile);
        let ini = match (root.as_ref(), directory.and_then(profile_directory)) {
            (Some(root), Some(directory)) => Ini::read(
                &root
                    .join("basic/profiles")
                    .join(directory)
                    .join("basic.ini"),
            ),
            _ => Ini::default(),
        };
        let (width, height) = monitor_resolution();
        let reply = json!({
            "ok":true, "obs_connected":false, "obs_config_dir":root.as_deref().map(path_text),
            "active_profile":profile, "active_scene_collection":global.global("currentscenecollection"),
            "video":{"base_width":0,"base_height":0,"output_width":0,"output_height":0,"fps":0,"fps_num":0,"fps_den":1},
            "recording":{"output_mode":"","use_stream_encoder":false,"encoder":"","format":"","output_path":""},
            "scene":{"dedicated_scene_exists":false,"capture_source_exists":false,"source_fit_to_canvas":false},
            "monitor":{"width":width,"height":height},
            "latest_backup":latest_backup(data), "obs_version":null,
        });
        Self { reply, ini }
    }
}

async fn fill_live(obs: &mut ObsRpc, status: &mut LocalStatus) -> std::io::Result<()> {
    let version = obs
        .request("GetVersion", Value::Null)
        .await
        .unwrap_or_default();
    status.reply["obs_version"] = json!(version["obsVersion"].as_str().map(str::trim));
    let video = obs.request("GetVideoSettings", Value::Null).await?;
    let video = video
        .get("videoSettings")
        .or_else(|| video.get("video_settings"))
        .unwrap_or(&video);
    let width = positive(video, "baseWidth", "base_width", 0);
    let height = positive(video, "baseHeight", "base_height", 0);
    let numerator = positive(video, "fpsNumerator", "fps_numerator", 60);
    let denominator = positive(video, "fpsDenominator", "fps_denominator", 1);
    status.reply["video"] = json!({
        "base_width":width, "base_height":height,
        "output_width":positive(video,"outputWidth","output_width",width),
        "output_height":positive(video,"outputHeight","output_height",height),
        "fps":(numerator as f64 / denominator as f64).round_ties_even() as u64,
        "fps_num":numerator, "fps_den":denominator,
    });
    status.reply["scene"] = scene_status(obs, width, height).await;
    status.reply["recording"] = recording_status(obs, &status.ini).await;
    Ok(())
}

fn positive(source: &Value, camel: &str, snake: &str, fallback: u64) -> u64 {
    [&source[camel], &source[snake]]
        .into_iter()
        .find_map(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str()?.parse::<u64>().ok())
                .filter(|&number| number > 0)
        })
        .unwrap_or(fallback)
}

async fn scene_status(obs: &mut ObsRpc, width: u64, height: u64) -> Value {
    let name_from_env = |key: &str, default: &str| {
        env::var(key)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| default.to_owned())
    };
    let scene = name_from_env("CS2_INSIGHT_OBS_SCENE_NAME", "CS2 Insight Recording");
    let capture = name_from_env(
        "CS2_INSIGHT_OBS_GAME_CAPTURE_NAME",
        "CS2 Insight Game Capture",
    );
    let scenes = obs
        .request("GetSceneList", Value::Null)
        .await
        .unwrap_or_default();
    let dedicated = scenes["scenes"].as_array().is_some_and(|scenes| {
        scenes
            .iter()
            .any(|candidate| candidate["sceneName"].as_str() == Some(&scene))
    });
    let items = obs
        .request("GetSceneItemList", json!({"sceneName":scene}))
        .await
        .unwrap_or_default();
    let item = items["sceneItems"].as_array().and_then(|items| {
        items.iter().find(|item| {
            item["sourceName"]
                .as_str()
                .or_else(|| item["sceneItemSourceName"].as_str())
                == Some(&capture)
        })
    });
    let fitted = if let Some(id) = item
        .and_then(|item| item["sceneItemId"].as_i64())
        .filter(|_| width > 0)
    {
        let transform = obs
            .request(
                "GetSceneItemTransform",
                json!({"sceneName":scene,"sceneItemId":id}),
            )
            .await
            .unwrap_or_default();
        let transform = transform.get("sceneItemTransform").unwrap_or(&transform);
        fits_canvas(transform, width as f64, height as f64)
    } else {
        false
    };
    json!({"dedicated_scene_exists":dedicated,"capture_source_exists":item.is_some(),"source_fit_to_canvas":fitted})
}

fn fits_canvas(transform: &Value, width: f64, height: f64) -> bool {
    let number = |key: &str| transform[key].as_f64().unwrap_or_default();
    let bounds = transform["boundsType"].as_str().unwrap_or_default();
    let fits = |w: f64, h: f64, tolerance: f64| {
        w > 0.0 && h > 0.0 && w >= width - tolerance && h >= height - tolerance
    };
    if [
        "OBS_BOUNDS_STRETCH",
        "OBS_BOUNDS_SCALE_INNER",
        "OBS_BOUNDS_SCALE_OUTER",
    ]
    .contains(&bounds)
        && fits(number("boundsWidth"), number("boundsHeight"), 4.0)
    {
        return true;
    }
    if number("positionX").abs() > 4.0 || number("positionY").abs() > 4.0 {
        return false;
    }
    let (scale_x, scale_y) = (number("scaleX"), number("scaleY"));
    let (source_width, source_height) = (number("sourceWidth"), number("sourceHeight"));
    fits(number("width"), number("height"), 4.0)
        || (scale_x > 0.0
            && scale_y > 0.0
            && source_width > 0.0
            && source_height > 0.0
            && fits(scale_x * source_width, scale_y * source_height, 8.0))
}

async fn recording_status(obs: &mut ObsRpc, ini: &Ini) -> Value {
    let advanced = parameter(obs, "Output", "Mode")
        .await
        .eq_ignore_ascii_case("advanced");
    let mode = if advanced { "Advanced" } else { "Simple" };
    let category = if advanced { "AdvOut" } else { "SimpleOutput" };
    let simple = |key: &str| ini.get("simpleoutput", key);
    let mut stream_encoder = simple("recusestreamencoder")
        .or_else(|| simple("usestreamencoder"))
        .map(enabled);
    if stream_encoder.is_none() {
        for name in ["RecUseStreamEncoder", "UseStreamEncoder"] {
            let result = parameter(obs, "SimpleOutput", name).await;
            if !result.is_empty() {
                stream_encoder = Some(enabled(&result));
                break;
            }
        }
    }
    let encoder = parameter(obs, category, "RecEncoder").await;
    let format = parameter(obs, category, "RecFormat2").await;
    let mut output = simple("filepath")
        .or_else(|| simple("filepath2"))
        .or_else(|| ini.get("advout", "recfilepath"))
        .unwrap_or_default()
        .to_owned();
    if output.is_empty() {
        let result = obs
            .request("GetRecordDirectory", Value::Null)
            .await
            .unwrap_or_default();
        output = result["recordDirectory"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_owned();
    }
    let fallback = |current: String, previous: Option<&str>| {
        if current.is_empty() {
            previous.unwrap_or_default().to_owned()
        } else {
            current
        }
    };
    let mut recording = json!({
        "output_mode":mode,"use_stream_encoder":stream_encoder.unwrap_or(false),
        "encoder":fallback(encoder,simple("recencoder").or_else(||simple("encoder"))),
        "format":fallback(format,simple("recformat2").or_else(||simple("recformat"))),
        "output_path":output,
    });
    if !advanced {
        let quality = parameter(obs, category, "RecQuality").await;
        recording["rec_quality"] = json!(fallback(quality, simple("recquality")));
    }
    recording
}

async fn parameter(obs: &mut ObsRpc, category: &str, key: &str) -> String {
    let reply = obs
        .request(
            "GetProfileParameter",
            json!({"parameterCategory":category,"parameterName":key}),
        )
        .await
        .unwrap_or_default();
    reply["parameterValue"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn enabled(value: &str) -> bool {
    matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes")
}

#[derive(Default)]
struct Ini(HashMap<String, HashMap<String, String>>);

impl Ini {
    fn read(path: &Path) -> Self {
        let Some(bytes) = read_small(path) else {
            return Self::default();
        };
        let mut fields: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut section = String::new();
        let text = String::from_utf8_lossy(&bytes);
        for line in text.trim_start_matches('\u{feff}').lines() {
            let line = line.trim();
            if let Some(name) = line
                .strip_prefix('[')
                .and_then(|line| line.strip_suffix(']'))
            {
                section = name.trim().to_ascii_lowercase();
            } else if let Some((key, value)) = line.split_once('=') {
                fields
                    .entry(section.clone())
                    .or_default()
                    .insert(key.trim().to_ascii_lowercase(), value.trim().to_owned());
            }
        }
        Self(fields)
    }

    fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.0
            .get(section)?
            .get(key)
            .map(String::as_str)
            .filter(|value| !value.is_empty())
    }

    fn global(&self, key: &str) -> Option<&str> {
        self.get("basic", key).or_else(|| self.get("", key))
    }
}

fn read_small(path: &Path) -> Option<Vec<u8>> {
    const LIMIT: u64 = 512 * 1024;
    let mut content = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(LIMIT + 1)
        .read_to_end(&mut content)
        .ok()?;
    (content.len() as u64 <= LIMIT).then_some(content)
}

fn profile_directory(name: &str) -> Option<&str> {
    let mut components = Path::new(name).components();
    (matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none())
        .then_some(name)
}

fn latest_backup(data: &Path) -> Option<Value> {
    let mut newest = None;
    for entry in fs::read_dir(data.join(".obs_config_backups"))
        .ok()?
        .flatten()
    {
        let Ok(info) = entry.metadata() else { continue };
        if !info.is_dir() {
            continue;
        }
        let Ok(time) = info.modified() else { continue };
        if newest
            .as_ref()
            .is_some_and(|(previous, _)| time <= *previous)
        {
            continue;
        }
        let Some(bytes) = read_small(&entry.path().join("manifest.json")) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        if manifest.is_object() {
            newest = Some((
                time,
                json!({
                    "id":entry.file_name().to_string_lossy(),
                    "created_at":manifest["created_at"].as_str().unwrap_or_default(),
                }),
            ));
        }
    }
    newest.map(|(_, value)| value)
}

fn obs_studio_root() -> Option<PathBuf> {
    if cfg!(windows) {
        env::var_os("APPDATA")
            .filter(|path| !path.is_empty())
            .map(|path| PathBuf::from(path).join("obs-studio"))
    } else {
        None
    }
}

#[cfg(windows)]
fn monitor_resolution() -> (i32, i32) {
    use std::ptr::null_mut;
    use windows_sys::Win32::Graphics::Gdi::{
        GetDC, GetDeviceCaps, ReleaseDC, DESKTOPHORZRES, DESKTOPVERTRES,
    };
    // A null window selects the screen DC. Release it with the same window argument.
    unsafe {
        let dc = GetDC(null_mut());
        if !dc.is_null() {
            let width = GetDeviceCaps(dc, DESKTOPHORZRES as i32);
            let height = GetDeviceCaps(dc, DESKTOPVERTRES as i32);
            ReleaseDC(null_mut(), dc);
            if width > 0 && height > 0 {
                return (width, height);
            }
        }
    }
    (1920, 1080)
}

#[cfg(not(windows))]
fn monitor_resolution() -> (i32, i32) {
    (1920, 1080)
}
