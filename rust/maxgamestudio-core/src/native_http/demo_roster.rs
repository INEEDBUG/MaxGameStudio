use super::{
    demo_query, demo_read_error, detail, json_reply, profile_io_error, profile_work,
    validation_reply, HttpResponse, ProfileState,
};
use crate::demo_library::{normalize_players, DemoLibrary, RosterCache, RosterSource};
use hyper::StatusCode;
use serde_json::{json, Value};
use std::{
    io,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

pub(super) async fn players(
    profile: &Arc<ProfileState>,
    route: &str,
    extended_request: &AtomicBool,
) -> HttpResponse {
    let id = match demo_query::parse_id(route) {
        Ok(id) => id,
        Err(errors) => return validation_reply(errors),
    };
    let RosterSource {
        path,
        match_meta,
        cache,
    } = match profile_work(profile, move |profile| {
        DemoLibrary::new(profile.config.database_path()).roster(id)
    })
    .await
    {
        Ok(Ok(Some(source))) => source,
        Ok(Ok(None)) => return detail(StatusCode::NOT_FOUND, &format!("Demo not found: {id}")),
        Ok(Err(error)) => return demo_read_error(&error),
        Err(error) => return profile_io_error(&error),
    };
    match load_players(profile, &path, cache, extended_request).await {
        Ok(players) => json_reply(
            StatusCode::OK,
            json!({ "players":players,"match_meta":match_meta }),
        ),
        Err(error) => error.response(),
    }
}

pub(super) enum RosterError {
    Saved(String),
    WorkerMissing,
    Busy,
    InvalidPath,
    Parse,
    Timeout,
}

impl RosterError {
    fn response(self) -> HttpResponse {
        match self {
            Self::Saved(error) => detail(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Demo 玩家名单解析失败：{error}"),
            ),
            Self::WorkerMissing => {
                detail(StatusCode::SERVICE_UNAVAILABLE, "未找到原生 Demo 解析组件")
            }
            Self::Busy => detail(
                StatusCode::SERVICE_UNAVAILABLE,
                "正在处理其他 Demo，请稍后重试",
            ),
            Self::InvalidPath => detail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Demo 玩家名单解析失败：文件路径无效",
            ),
            Self::Parse | Self::Timeout => {
                detail(StatusCode::INTERNAL_SERVER_ERROR, "Demo 玩家名单解析失败")
            }
        }
    }
}

pub(super) async fn load_players(
    profile: &ProfileState,
    path: &Path,
    cache: RosterCache,
    extended_request: &AtomicBool,
) -> Result<Vec<Value>, RosterError> {
    match cache {
        RosterCache::Ready(players) => Ok(players),
        RosterCache::Error(error) => Err(RosterError::Saved(error)),
        RosterCache::Missing => {
            let Some(parser) = profile.parser_worker.as_ref() else {
                return Err(RosterError::WorkerMissing);
            };
            let Ok(_permit) = profile.demo_parser.try_acquire() else {
                return Err(RosterError::Busy);
            };
            let path = std::path::absolute(path).map_err(|_| RosterError::InvalidPath)?;
            extended_request.store(true, Ordering::Relaxed);
            match parser
                .inspect_roster(&path)
                .await
                .map(|mut result| result["players"].take())
            {
                Ok(Value::Array(rows)) => Ok(normalize_players(rows)),
                Err(error) if error.kind() == io::ErrorKind::TimedOut => Err(RosterError::Timeout),
                _ => Err(RosterError::Parse),
            }
        }
    }
}
