use super::{
    demo_body::{field_issue, model, text},
    demo_query::{parse_clip_page, parse_path_integer},
    detail, json_reply, model_body, profile_disk, profile_work, validation_reply, HttpResponse,
    ProfileState,
};
use crate::{
    obs_wire::ObsRpc,
    profile_platform::{expand_user, path_text},
    recorded_clips,
};
use http_body_util::BodyExt;
use hyper::{body::Incoming, header::RANGE, Method, Request, StatusCode};
use percent_encoding::percent_decode_str;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::time::timeout;
use tower_http::services::ServeFile;

const CLIP_PREFIX: &str = "/api/recorded-clips/";

pub(super) enum Route {
    List,
    Storage,
    Stream,
}

impl Route {
    pub(super) fn from_path(path: &str) -> Option<Self> {
        match path {
            "/api/recorded-clips" => Some(Self::List),
            "/api/recorded-clips/storage" => Some(Self::Storage),
            _ => {
                let path = percent_decode_str(path).decode_utf8_lossy();
                stream_id(&path).map(|_| Self::Stream)
            }
        }
    }
}

pub(super) async fn respond(
    profile: &Arc<ProfileState>,
    route: Route,
    request: Request<Incoming>,
    extended_request: &AtomicBool,
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
        Route::Storage if request.method() == Method::PATCH => {
            change_storage(profile, request, extended_request).await
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
        Route::Stream => stream(profile, request, extended_request).await,
    }
}

enum StorageChangeError {
    Unavailable,
    Active,
    Unconfirmed,
}

async fn change_storage(
    profile: &Arc<ProfileState>,
    request: Request<Incoming>,
    extended_request: &AtomicBool,
) -> HttpResponse {
    let input = match model_body(request, "Recording directory request is too large").await {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let value = match model(&input).and_then(|fields| {
        let path = fields.get("path").ok_or_else(|| {
            vec![field_issue(
                "path",
                &input,
                "missing",
                "Field required",
                None,
            )]
        })?;
        text(path, "path", 1, 2048)
            .map(str::to_owned)
            .map_err(|error| vec![error])
    }) {
        Ok(path) => path,
        Err(errors) => return validation_reply(errors),
    };
    let Ok(_lease) = profile.obs_writes.try_acquire() else {
        return detail(StatusCode::CONFLICT, "正在修改 OBS 录制目录，请稍后重试");
    };
    let state = profile_work(profile, move |profile| {
        let path = expand_user(value.trim())
            .ok()
            .filter(|path| path.is_absolute() && path.is_dir());
        (path, profile.config.obs_settings())
    })
    .await;
    let (path, settings) = match state {
        Ok((Some(path), Ok(settings))) => (path_text(&path), settings),
        Ok((None, _)) => return detail(StatusCode::CONFLICT, "请选择已存在的本地录制目录"),
        _ => return detail(StatusCode::BAD_GATEWAY, "无法读取 OBS 连接配置"),
    };
    extended_request.store(true, Ordering::Relaxed);
    let update = timeout(Duration::from_secs(8), async {
        let mut obs = ObsRpc::connect(settings)
            .await
            .map_err(|_| StorageChangeError::Unavailable)?;
        let status = obs
            .request("GetRecordStatus", Value::Null)
            .await
            .map_err(|_| StorageChangeError::Unavailable)?;
        match status["outputActive"].as_bool() {
            Some(false) => {}
            Some(true) => return Err(StorageChangeError::Active),
            None => return Err(StorageChangeError::Unavailable),
        }
        let mode = obs
            .request(
                "GetProfileParameter",
                json!({
                    "parameterCategory":"Output","parameterName":"Mode",
                }),
            )
            .await
            .map_err(|_| StorageChangeError::Unavailable)?;
        let advanced = mode["parameterValue"]
            .as_str()
            .is_some_and(|mode| mode.trim().eq_ignore_ascii_case("Advanced"));
        let (category, name, output_mode) = if advanced {
            ("AdvOut", "RecFilePath", "Advanced")
        } else {
            ("SimpleOutput", "FilePath", "Simple")
        };
        obs.request(
            "SetProfileParameter",
            json!({
                "parameterCategory":category,"parameterName":name,"parameterValue":path,
            }),
        )
        .await
        .map_err(|_| StorageChangeError::Unavailable)?;
        let recorded = obs
            .request(
                "GetProfileParameter",
                json!({
                    "parameterCategory":category,"parameterName":name,
                }),
            )
            .await
            .map_err(|_| StorageChangeError::Unconfirmed)?;
        let saved = recorded["parameterValue"]
            .as_str()
            .unwrap_or_default()
            .trim();
        if !same_directory(saved, &path) {
            return Err(StorageChangeError::Unconfirmed);
        }
        Ok(json!({"ok":true,"path":saved,"output_mode":output_mode,"restart_obs_required":false}))
    })
    .await;
    match update {
        Ok(Ok(receipt)) => json_reply(StatusCode::OK, receipt),
        Ok(Err(StorageChangeError::Active)) => detail(
            StatusCode::CONFLICT,
            "OBS 正在录制，请停止录制后再修改存放目录",
        ),
        Ok(Err(StorageChangeError::Unconfirmed)) => detail(
            StatusCode::CONFLICT,
            "OBS 未确认新的录制目录，请重新读取目录",
        ),
        _ => detail(StatusCode::BAD_GATEWAY, "无法连接 OBS 修改录制目录"),
    }
}

fn same_directory(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.replace('/', "\\")
            .trim_end_matches('\\')
            .eq_ignore_ascii_case(right.replace('/', "\\").trim_end_matches('\\'))
    } else {
        left.trim_end_matches('/') == right.trim_end_matches('/')
    }
}

fn stream_id(path: &str) -> Option<&str> {
    let id = path.strip_prefix(CLIP_PREFIX)?.strip_suffix("/stream")?;
    (!id.is_empty() && !id.contains('/')).then_some(id)
}

async fn stream(
    profile: &Arc<ProfileState>,
    mut request: Request<Incoming>,
    extended_request: &AtomicBool,
) -> HttpResponse {
    let decoded = percent_decode_str(request.uri().path()).decode_utf8_lossy();
    let id = match parse_path_integer(stream_id(&decoded).expect("clip route"), "clip_id") {
        Ok(id) => id,
        Err(errors) => return validation_reply(errors),
    };
    let resolved = profile_work(profile, move |profile| {
        let raw = recorded_clips::output_path(&profile.config.database_path(), id)?;
        Ok::<_, recorded_clips::RecordedClipsError>(raw.map(|raw| {
            if raw.trim().is_empty() {
                return Err((StatusCode::BAD_REQUEST, "clip output_path is empty"));
            }
            canonical_media(&raw).ok_or((StatusCode::NOT_FOUND, "clip file not found"))
        }))
    })
    .await;
    let file = match resolved {
        Ok(Ok(Some(Ok(file)))) => file,
        Ok(Ok(Some(Err((status, message))))) => return detail(status, message),
        Ok(Ok(None)) => {
            return json_reply(
                StatusCode::NOT_FOUND,
                json!({
                    "detail":{"code":"MONTAGE_CLIP_NOT_FOUND","params":{"id":id.to_string()}},
                }),
            )
        }
        _ => return read_error(),
    };
    // Direct browser seeking sends one range. A multipart request may fetch the
    // complete file as allowed by HTTP rather than allocating a compound buffer.
    if request
        .headers()
        .get(RANGE)
        .is_some_and(|range| range.as_bytes().contains(&b','))
    {
        request.headers_mut().remove(RANGE);
    }
    match ServeFile::new(file).try_call(request).await {
        Ok(response) => {
            if response.status().is_success() {
                extended_request.store(true, Ordering::Relaxed);
            }
            response.map(BodyExt::boxed_unsync)
        }
        Err(_) => detail(StatusCode::INTERNAL_SERVER_ERROR, "无法读取录制文件"),
    }
}

fn canonical_media(raw: &str) -> Option<PathBuf> {
    let path = expand_user(raw).ok()?.canonicalize().ok()?;
    path.is_file().then_some(path)
}

fn read_error() -> HttpResponse {
    detail(StatusCode::INTERNAL_SERVER_ERROR, "无法读取已录制视频")
}
