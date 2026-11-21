use crate::{
    native_http::{detail, media_stream, HttpResponse, ProfileState},
    profile_platform::expand_user,
};
use hyper::{
    body::Incoming,
    header::{HeaderValue, CONTENT_TYPE},
    Request, StatusCode,
};
use serde_json::Value;
use std::{env, path::PathBuf, sync::atomic::AtomicBool};

const LEGACY_DEFAULT: &str = "NotoSansSC-Bold.ttf";
const LEGACY_MEDIUM: &str = "NotoSansSC-Medium.ttf";
const BUILTIN_FONTS: &[&str] = &[
    LEGACY_DEFAULT,
    LEGACY_MEDIUM,
    "Rajdhani-Bold.ttf",
    "Rajdhani-SemiBold.ttf",
];

fn built_in(profile: &ProfileState, name: &str) -> Option<PathBuf> {
    if !BUILTIN_FONTS.contains(&name) {
        return None;
    }
    let path = profile
        .config
        .bundle_data_dir()
        .parent()?
        .join("backend/assets/fonts")
        .join(name);
    path.is_file().then_some(path)
}

pub(super) fn for_export(profile: &ProfileState, config: &Value) -> Option<PathBuf> {
    if let Some(path) = config["font_file"]
        .as_str()
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        return Some(expand_user(path).unwrap_or_else(|_| PathBuf::from(path)));
    }
    let family = config["font_family"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    let system = |filename: &str| {
        let path = PathBuf::from(env::var_os("WINDIR")?)
            .join("Fonts")
            .join(filename);
        path.is_file().then_some(path)
    };
    let preferred_system = match family.as_str() {
        "微软雅黑" => system("msyhbd.ttc"),
        "impact" => system("impact.ttf"),
        _ => None,
    };
    preferred_system
        .or_else(|| {
            if family == "思源黑体 medium" {
                built_in(profile, LEGACY_MEDIUM)
            } else {
                None
            }
        })
        .or_else(|| built_in(profile, LEGACY_DEFAULT))
        .or_else(|| system("msyhbd.ttc"))
        .or_else(|| system("segoeuib.ttf"))
}

pub(super) async fn serve(
    profile: &ProfileState,
    name: &str,
    request: Request<Incoming>,
    extended: &AtomicBool,
) -> HttpResponse {
    let Some(path) = built_in(profile, name) else {
        return detail(StatusCode::NOT_FOUND, "font not found");
    };
    let Ok(mut response) = media_stream::serve(path, request, extended).await else {
        return detail(StatusCode::INTERNAL_SERVER_ERROR, "无法读取剪辑字体");
    };
    if response.status().is_success() {
        response
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("font/ttf"));
    }
    response
}
