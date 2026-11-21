use super::{
    asset_json, asset_path, asset_root, asset_select, read_error, read_table, AssetMediaError,
    TABLE,
};
use crate::native_http::{json_reply, profile_work, HttpResponse, ProfileState};
use hyper::StatusCode;
use rusqlite::OptionalExtension;
use serde_json::{json, Value};
use std::{
    env,
    fs::File,
    io::{self, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Command, time::timeout};

pub(super) async fn respond(
    profile: &Arc<ProfileState>,
    id: i64,
    extended_request: &AtomicBool,
) -> HttpResponse {
    let mut source = match profile_work(profile, move |profile| load(profile, id)).await {
        Ok(Ok(source)) => source,
        Ok(Err(error)) => return error.response(),
        Err(_) => return read_error(),
    };
    if let Some(ffprobe) = source.ffprobe {
        if let Ok(_permit) = profile.media_process.try_acquire() {
            extended_request.store(true, Ordering::Relaxed);
            if let Ok(Ok(info)) =
                timeout(Duration::from_secs(12), inspect(&ffprobe, &source.file)).await
            {
                apply_probe(&mut source.response, &info);
            }
        }
    }
    json_reply(StatusCode::OK, source.response)
}

struct Metadata {
    response: Value,
    file: PathBuf,
    ffprobe: Option<PathBuf>,
}

fn load(profile: &ProfileState, id: i64) -> Result<Metadata, AssetMediaError> {
    let row = read_table(&profile.config.database_path(), TABLE, |connection| {
        let select = asset_select(connection)?;
        connection
            .query_row(&format!("{select} WHERE id=?1"), [id], asset_json)
            .optional()
    })
    .map_err(|_| AssetMediaError::Read)?
    .flatten()
    .ok_or(AssetMediaError::Missing)?;
    let root = asset_root(profile)?;
    let file = asset_path(
        &root,
        row["file_path"].as_str().expect("asset path from SQLite"),
    )
    .map_err(|(status, message)| AssetMediaError::File(status, message))?;
    let kind = row["kind"]
        .as_str()
        .filter(|name| !name.is_empty())
        .unwrap_or("file")
        .to_ascii_lowercase();
    let name = row["name"]
        .as_str()
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            file.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        });
    let mut response = json!({
        "id":id, "kind":kind, "name":name,
        "extension":file.extension().map(|extension| extension.to_string_lossy().to_uppercase()).unwrap_or_default(),
        "mime_type":row["mime_type"], "duration_sec":row["duration_sec"],
        "width":row["width"], "height":row["height"], "fps":null,"codec_name":null,"has_audio":null,
    });
    let ffprobe = match kind.as_str() {
        "image" => {
            if let Ok(Some((width, height))) = image_dimensions(&file) {
                response["width"] = json!(width);
                response["height"] = json!(height);
            }
            None
        }
        "font" => None,
        _ => profile
            .config
            .ffmpeg_executable()
            .ok()
            .and_then(|ffmpeg| find_ffprobe(&ffmpeg)),
    };
    Ok(Metadata {
        response,
        file,
        ffprobe,
    })
}

fn find_ffprobe(ffmpeg: &Path) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    };
    let adjacent = ffmpeg.with_file_name(name);
    if adjacent.is_file() {
        return adjacent.canonicalize().ok();
    }
    env::var_os("PATH").and_then(|path| {
        env::split_paths(&path)
            .filter(|directory| directory.is_absolute())
            .map(|directory| directory.join(name))
            .find(|path| path.is_file())
            .and_then(|path| path.canonicalize().ok())
    })
}

async fn inspect(ffprobe: &Path, file: &Path) -> io::Result<Value> {
    const MAX_METADATA: u64 = 128 * 1024;
    let mut command = Command::new(ffprobe);
    command
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration:stream=codec_type,codec_name,width,height,r_frame_rate",
            "-of",
            "json",
        ])
        .arg(file)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    let mut child = command.spawn()?;
    let mut bytes = Vec::new();
    child
        .stdout
        .take()
        .expect("FFprobe stdout")
        .take(MAX_METADATA + 1)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() as u64 > MAX_METADATA || !child.wait().await?.success() {
        return Err(io::Error::other(
            "FFprobe did not return bounded media metadata",
        ));
    }
    let data: Value = serde_json::from_slice(&bytes)?;
    if !data.is_object() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "FFprobe returned an invalid document",
        ));
    }
    Ok(data)
}

fn apply_probe(reply: &mut Value, info: &Value) {
    if let Some(duration) = number(&info["format"]["duration"]).filter(|duration| *duration > 0.0) {
        reply["duration_sec"] = json!(duration);
    }
    let mut width = 1920;
    let mut height = 1080;
    let mut fps = 60.0;
    let mut codec = "";
    let mut audio = false;
    if let Some(streams) = info["streams"].as_array() {
        for stream in streams {
            match stream["codec_type"].as_str() {
                Some("video") => {
                    width = dimension(&stream["width"]).unwrap_or(width);
                    height = dimension(&stream["height"]).unwrap_or(height);
                    fps = frame_rate(stream["r_frame_rate"].as_str().unwrap_or_default());
                    codec = stream["codec_name"].as_str().unwrap_or_default().trim();
                }
                Some("audio") => audio = true,
                _ => {}
            }
        }
    }
    if reply["kind"] == "audio" {
        reply["width"] = Value::Null;
        reply["height"] = Value::Null;
        reply["fps"] = Value::Null;
    } else {
        reply["width"] = json!(width);
        reply["height"] = json!(height);
        reply["fps"] = json!(fps);
    }
    reply["codec_name"] = json!((!codec.is_empty()).then(|| codec.to_ascii_lowercase()));
    reply["has_audio"] = json!(audio);
}

fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse().ok())
        .filter(|value| value.is_finite())
}

fn dimension(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str()?.trim().parse().ok())
        .filter(|&value| value > 0)
}

fn frame_rate(raw: &str) -> f64 {
    let raw = raw.trim();
    let rate = if let Some((a, b)) = raw.split_once('/') {
        a.parse::<f64>()
            .ok()
            .zip(b.parse::<f64>().ok())
            .map(|(a, b)| a / b)
    } else {
        raw.parse().ok()
    };
    rate.filter(|rate| rate.is_finite()).unwrap_or(60.0)
}

fn image_dimensions(path: &Path) -> io::Result<Option<(u32, u32)>> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut first = [0; 2];
    reader.read_exact(&mut first)?;
    let dimensions = match first {
        [0x89, b'P'] => {
            let mut png = [0; 22];
            reader.read_exact(&mut png)?;
            if &png[..6] != b"NG\r\n\x1a\n" {
                return Ok(None);
            }
            (
                u32::from_be_bytes(png[14..18].try_into().unwrap()),
                u32::from_be_bytes(png[18..22].try_into().unwrap()),
            )
        }
        [0xff, 0xd8] => return jpeg_dimensions(reader),
        _ => return Ok(None),
    };
    Ok((dimensions.0 > 0 && dimensions.1 > 0).then_some(dimensions))
}

fn jpeg_dimensions(mut source: BufReader<File>) -> io::Result<Option<(u32, u32)>> {
    let mut remaining = 1024 * 1024;
    let mut marker = [0];
    while remaining > 0 {
        source.read_exact(&mut marker)?;
        remaining -= 1;
        if marker[0] != 0xff {
            continue;
        }
        while marker[0] == 0xff && remaining > 0 {
            source.read_exact(&mut marker)?;
            remaining -= 1;
        }
        if matches!(marker[0], 0x00 | 0xd0..=0xd7 | 0xd8) {
            continue;
        }
        if matches!(marker[0], 0xda | 0xd9) {
            break;
        }
        let mut length = [0; 2];
        source.read_exact(&mut length)?;
        let length = usize::from(u16::from_be_bytes(length));
        if length < 2 || length > remaining {
            break;
        }
        remaining -= length;
        if matches!(marker[0], 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) && length >= 7
        {
            let mut frame = [0; 5];
            source.read_exact(&mut frame)?;
            let height = u16::from_be_bytes([frame[1], frame[2]]);
            let width = u16::from_be_bytes([frame[3], frame[4]]);
            return Ok((width > 0 && height > 0).then_some((u32::from(width), u32::from(height))));
        }
        source.seek(SeekFrom::Current((length - 2) as i64))?;
    }
    Ok(None)
}
