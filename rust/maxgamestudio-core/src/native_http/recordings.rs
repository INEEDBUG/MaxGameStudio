use super::{
    demo_query::parse_clip_page, detail, json_reply, profile_disk, profile_work, validation_reply,
    HttpResponse, ProfileState,
};
use crate::{obs_wire::ObsRpc, recorded_clips};
use hyper::{body::Incoming, Request, StatusCode};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tokio::time::timeout;

pub(super) enum Route {
    List,
    Storage,
}

impl Route {
    pub(super) fn from_path(path: &str) -> Option<Self> {
        match path {
            "/api/recorded-clips" => Some(Self::List),
            "/api/recorded-clips/storage" => Some(Self::Storage),
            _ => None,
        }
    }
}

pub(super) async fn read(
    profile: &Arc<ProfileState>,
    route: Route,
    request: Request<Incoming>,
) -> HttpResponse {
    match route {
        Route::List => {
            let (limit, offset) = match parse_clip_page(request.uri().query().unwrap_or_default()) {
                Ok(page) => page,
                Err(errors) => return validation_reply(errors),
            };
            profile_disk(profile, move |profile| {
                match recorded_clips::list(&profile.config.database_path(), limit, offset) {
                    Ok(items) => json_reply(
                        StatusCode::OK,
                        json!({"items":items,"limit":limit,"offset":offset}),
                    ),
                    Err(_) => read_error(),
                }
            })
            .await
        }
        Route::Storage => {
            let result = profile_work(profile, |profile| {
                let recent = recorded_clips::recent_output_dir(&profile.config.database_path())?;
                Ok::<_, recorded_clips::RecordedClipsError>((recent, profile.config.obs_settings()))
            })
            .await;
            let Ok(Ok((recent_path, settings))) = result else {
                return read_error();
            };
            let mut configured_path = String::new();
            let mut obs_connected = false;
            if let Ok(settings) = settings {
                let _ = timeout(Duration::from_secs(3), async {
                    let Ok(mut obs) = ObsRpc::connect(settings).await else {
                        return;
                    };
                    obs_connected = true;
                    if let Ok(reply) = obs.request("GetRecordDirectory", Value::Null).await {
                        configured_path = reply["recordDirectory"]
                            .as_str()
                            .unwrap_or_default()
                            .trim()
                            .to_owned();
                    }
                })
                .await;
            }
            json_reply(
                StatusCode::OK,
                json!({
                    "configured_path":configured_path,"recent_path":recent_path,"obs_connected":obs_connected,
                }),
            )
        }
    }
}

fn read_error() -> HttpResponse {
    detail(StatusCode::INTERNAL_SERVER_ERROR, "无法读取已录制视频")
}
