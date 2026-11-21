use super::{
    demo_query, demo_read_error, detail, json_reply, profile_io_error, profile_work,
    validation_reply, HttpResponse, ProfileState,
};
use crate::demo_library::{normalize_players, DemoLibrary, RosterCache, RosterSource};
use hyper::StatusCode;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
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
    let players = match cache {
        RosterCache::Ready(players) => players,
        RosterCache::Error(error) => {
            return detail(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Demo 玩家名单解析失败：{error}"),
            )
        }
        RosterCache::Missing => {
            let Some(parser) = profile.parser_worker.as_ref() else {
                return detail(StatusCode::SERVICE_UNAVAILABLE, "未找到原生 Demo 解析组件");
            };
            let Ok(_permit) = profile.demo_parser.try_acquire() else {
                return detail(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "正在处理其他 Demo，请稍后重试",
                );
            };
            let Ok(path) = std::path::absolute(path) else {
                return detail(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Demo 玩家名单解析失败：文件路径无效",
                );
            };
            extended_request.store(true, Ordering::Relaxed);
            match parser
                .inspect_roster(&path)
                .await
                .map(|mut result| result["players"].take())
            {
                Ok(Value::Array(rows)) => normalize_players(rows),
                _ => return detail(StatusCode::INTERNAL_SERVER_ERROR, "Demo 玩家名单解析失败"),
            }
        }
    };
    json_reply(
        StatusCode::OK,
        json!({ "players": players, "match_meta": match_meta }),
    )
}
