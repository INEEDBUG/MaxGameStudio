mod metadata;

use super::read_table;
use crate::{
    native_http::{
        demo_query::{parse_path_integer, parse_project_page},
        detail, json_reply, media_stream, profile_disk, profile_work, validation_reply,
        HttpResponse, ProfileState,
    },
    profile_platform::{expand_user, resolved_path},
};
use hyper::{
    body::Incoming,
    header::{HeaderValue, CACHE_CONTROL},
    Request, StatusCode,
};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
    time::UNIX_EPOCH,
};

const ASSET_PATH: &str = "/api/lite-cut/assets";
const TABLE: &str = "lite_cut_assets";

pub(in crate::native_http) enum Route {
    List,
    Stream(String),
    Metadata(String),
}

impl Route {
    pub(super) fn from_path(path: &str) -> Option<Self> {
        if path == ASSET_PATH {
            return Some(Self::List);
        }
        let (id, action) = path
            .strip_prefix(ASSET_PATH)?
            .strip_prefix('/')?
            .split_once('/')?;
        if id.is_empty() {
            return None;
        }
        match action {
            "stream" => Some(Self::Stream(id.to_owned())),
            "metadata" => Some(Self::Metadata(id.to_owned())),
            _ => None,
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
            let page =
                match parse_project_page(request.uri().query().unwrap_or_default(), 500, 1000) {
                    Ok(page) => page,
                    Err(errors) => return validation_reply(errors),
                };
            profile_disk(profile, move |profile| list(profile, page)).await
        }
        Route::Stream(id) => {
            let id = match parse_path_integer(&id, "asset_id") {
                Ok(id) => id,
                Err(errors) => return validation_reply(errors),
            };
            let (file, font) =
                match profile_work(profile, move |profile| load_media(profile, id)).await {
                    Ok(Ok(media)) => media,
                    Ok(Err(error)) => return error.response(),
                    Err(_) => return read_error(),
                };
            let Ok(mut response) = media_stream::serve(file, request, extended_request).await
            else {
                return read_error();
            };
            if font {
                response
                    .headers_mut()
                    .insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
            }
            response
        }
        Route::Metadata(id) => match parse_path_integer(&id, "asset_id") {
            Ok(id) => metadata::respond(profile, id, extended_request).await,
            Err(errors) => validation_reply(errors),
        },
    }
}

fn list(
    profile: &ProfileState,
    (project_id, limit, offset): (Option<i64>, i64, i64),
) -> HttpResponse {
    let rows = read_table(&profile.config.database_path(), TABLE, |connection| {
        read_page(connection, project_id, limit, offset)
    });
    let Ok(rows) = rows else {
        return read_error();
    };
    let mut items = rows.unwrap_or_default();
    if !items.is_empty() {
        let Ok(root) = profile
            .config
            .lite_cut_assets_dir()
            .and_then(|path| resolved_path(&path))
        else {
            return read_error();
        };
        for row in &mut items {
            decorate_proxy(row, &root);
        }
    }
    json_reply(
        StatusCode::OK,
        json!({"items":items,"limit":limit,"offset":offset}),
    )
}

fn read_page(
    connection: &Connection,
    project_id: Option<i64>,
    limit: i64,
    offset: i64,
) -> rusqlite::Result<Vec<Value>> {
    let select = asset_select(connection)?;
    let mut query = connection.prepare(&format!(
        "{select} WHERE ?1 IS NULL OR project_id = ?1 OR project_id IS NULL
         ORDER BY created_at DESC LIMIT ?2 OFFSET ?3"
    ))?;
    let items = query.query_map(params![project_id, limit, offset], asset_json)?;
    items.collect()
}

fn asset_select(connection: &Connection) -> rusqlite::Result<String> {
    let (has_width, has_height): (bool, bool) = connection.query_row(
        "SELECT coalesce(max(lower(name) = 'width'), 0), coalesce(max(lower(name) = 'height'), 0)
         FROM pragma_table_info('lite_cut_assets')",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let width = if has_width { "width" } else { "NULL" };
    let height = if has_height { "height" } else { "NULL" };
    Ok(format!(
        "SELECT id, project_id, name, kind, mime_type, file_path, duration_sec,
                {width}, {height}, created_at FROM lite_cut_assets"
    ))
}

fn asset_json(row: &Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "id":row.get::<_, i64>(0)?,
        "project_id":row.get::<_, Option<i64>>(1)?,
        "name":row.get::<_, String>(2)?,
        "kind":row.get::<_, String>(3)?,
        "mime_type":row.get::<_, Option<String>>(4)?,
        "file_path":row.get::<_, String>(5)?,
        "duration_sec":row.get::<_, Option<f64>>(6)?,
        "width":row.get::<_, Option<i64>>(7)?,
        "height":row.get::<_, Option<i64>>(8)?,
        "created_at":row.get::<_, String>(9)?,
    }))
}

enum AssetMediaError {
    Read,
    Missing,
    File(StatusCode, &'static str),
}

impl AssetMediaError {
    fn response(self) -> HttpResponse {
        match self {
            Self::Read => read_error(),
            Self::Missing => json_reply(
                StatusCode::NOT_FOUND,
                json!({"detail":{"code":"LITECUT_ASSET_NOT_FOUND"}}),
            ),
            Self::File(status, message) => detail(status, message),
        }
    }
}

fn load_media(profile: &ProfileState, id: i64) -> Result<(PathBuf, bool), AssetMediaError> {
    let selected = read_table(&profile.config.database_path(), TABLE, |connection| {
        connection
            .query_row(
                "SELECT file_path, kind FROM lite_cut_assets WHERE id = ?1",
                [id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
    });
    let (raw, kind) = selected
        .map_err(|_| AssetMediaError::Read)?
        .flatten()
        .ok_or(AssetMediaError::Missing)?;
    let root = asset_root(profile)?;
    let source = asset_path(&root, &raw)
        .map_err(|(status, message)| AssetMediaError::File(status, message))?;
    let font = kind.eq_ignore_ascii_case("font");
    if font || !needs_proxy(&source) {
        return Ok((source, font));
    }
    let ready = cached_proxy(&root, &source).ok_or(AssetMediaError::File(
        StatusCode::UNPROCESSABLE_ENTITY,
        "尚未生成素材预览代理",
    ))?;
    Ok((ready.path, false))
}

fn asset_root(profile: &ProfileState) -> Result<PathBuf, AssetMediaError> {
    profile
        .config
        .lite_cut_assets_dir()
        .and_then(|path| resolved_path(&path))
        .map_err(|_| AssetMediaError::Read)
}

fn asset_path(root: &Path, raw: &str) -> Result<PathBuf, (StatusCode, &'static str)> {
    if raw.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "asset path empty"));
    }
    let path = expand_user(raw)
        .and_then(|path| path.canonicalize())
        .ok()
        .filter(|path| path.is_file())
        .ok_or((StatusCode::NOT_FOUND, "asset file not found"))?;
    if !path.starts_with(root) {
        return Err((StatusCode::FORBIDDEN, "asset path outside storage"));
    }
    Ok(path)
}

fn needs_proxy(source: &Path) -> bool {
    source
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            ["avi", "mkv", "gif", "mov"]
                .iter()
                .any(|candidate| ext.eq_ignore_ascii_case(candidate))
        })
}

struct CachedProxy {
    path: PathBuf,
    alpha: bool,
    version: String,
}

fn cached_proxy(root: &Path, source: &Path) -> Option<CachedProxy> {
    [("preview-alpha-v3.webm", true), ("preview60.mp4", false)]
        .into_iter()
        .find_map(|(extension, alpha)| {
            let path = source.with_extension(extension).canonicalize().ok()?;
            let metadata = fs::metadata(&path).ok()?;
            if !path.starts_with(root) || !metadata.is_file() {
                return None;
            }
            let version = metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map_or_else(
                    || "ready".to_owned(),
                    |modified| modified.as_nanos().to_string(),
                );
            Some(CachedProxy {
                path,
                alpha,
                version,
            })
        })
}

fn decorate_proxy(row: &mut Value, root: &Path) {
    let raw = row["file_path"].as_str().expect("asset path from SQLite");
    let required = needs_proxy(Path::new(raw));
    let (status, message, version, mode, alpha) = if !required {
        ("not_needed", "", "source".to_owned(), "direct", false)
    } else {
        match asset_path(root, raw) {
            Ok(source) => match cached_proxy(root, &source) {
                Some(ready) => ("ready", "", ready.version, "ready", ready.alpha),
                None => (
                    "failed",
                    "尚未生成素材预览代理",
                    "unavailable".to_owned(),
                    "failed",
                    false,
                ),
            },
            Err(_) => (
                "missing",
                "素材文件不存在",
                "unavailable".to_owned(),
                "failed",
                false,
            ),
        }
    };
    row["preview_proxy_required"] = json!(required);
    row["preview_proxy_status"] = json!(status);
    row["preview_proxy_error"] = json!(message);
    row["preview_proxy_version"] = json!(version);
    row["preview_proxy_mode"] = json!(mode);
    row["has_alpha"] = json!(alpha);
}

fn read_error() -> HttpResponse {
    detail(StatusCode::INTERNAL_SERVER_ERROR, "无法读取剪辑素材")
}
