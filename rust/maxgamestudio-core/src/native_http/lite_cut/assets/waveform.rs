use super::{asset_path, asset_root, read_error, read_table, AssetMediaError, TABLE};
use crate::{
    native_http::{
        demo_query::{parse_path_integer, parse_waveform},
        detail, json_reply, profile_disk, profile_work, validation_reply, HttpResponse, ProfileState,
    },
    recording_waveform::{self, WaveformCache, WaveformSource},
};
use hyper::StatusCode;
use rusqlite::OptionalExtension;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub(super) async fn respond(
    profile: &Arc<ProfileState>,
    id: &str,
    query: &str,
    extended_request: &AtomicBool,
) -> HttpResponse {
    let id = match parse_path_integer(id, "asset_id") {
        Ok(id) => id,
        Err(issues) => return validation_reply(issues),
    };
    let query = match parse_waveform(query) {
        Ok(query) => query,
        Err(issues) => return validation_reply(issues),
    };
    let waveform = match profile_work(profile, move |profile| load(profile, id)).await {
        Ok(Ok(waveform)) => waveform,
        Ok(Err(error)) => return error.response(),
        Err(_) => return read_error(),
    };
    if let Some(cache) = waveform.cache {
        return json_reply(StatusCode::OK, cache.view(query, true));
    }
    let Ok(_permit) = profile.media_process.try_acquire() else {
        return detail(StatusCode::SERVICE_UNAVAILABLE, "正在处理其他素材，请稍后重试");
    };
    let ffmpeg = match profile_work(profile, |profile| profile.config.ffmpeg_executable()).await {
        Ok(Ok(path)) => path,
        Ok(Err(error)) => return detail(StatusCode::UNPROCESSABLE_ENTITY, &error.to_string()),
        Err(_) => return read_error(),
    };
    extended_request.store(true, Ordering::Relaxed);
    let pcm = match recording_waveform::decode(&ffmpeg, &waveform.source.path).await {
        Ok(pcm) => pcm,
        Err(error) => return detail(StatusCode::UNPROCESSABLE_ENTITY, &error.to_string()),
    };
    profile_disk(profile, move |_| match waveform.source.publish(pcm, waveform.duration) {
        Ok(cache) => json_reply(StatusCode::OK, cache.view(query, false)),
        Err(error) => detail(StatusCode::UNPROCESSABLE_ENTITY, &error.to_string()),
    })
    .await
}

struct Waveform {
    source: WaveformSource,
    cache: Option<WaveformCache>,
    duration: Option<f64>,
}

fn load(profile: &ProfileState, id: i64) -> Result<Waveform, AssetMediaError> {
    let (raw, duration) = read_table(&profile.config.database_path(), TABLE, |connection| {
        connection
            .query_row(
                "SELECT file_path, duration_sec FROM lite_cut_assets WHERE id=?1",
                [id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<f64>>(1)?)),
            )
            .optional()
    })
    .map_err(|_| AssetMediaError::Read)?
    .flatten()
    .ok_or(AssetMediaError::Missing)?;
    let root = asset_root(profile)?;
    let path = asset_path(&root, &raw)
        .map_err(|(status, message)| AssetMediaError::File(status, message))?;
    let source = WaveformSource::open(path).map_err(|_| AssetMediaError::Read)?;
    let cache = source.cached();
    Ok(Waveform { source, cache, duration })
}
