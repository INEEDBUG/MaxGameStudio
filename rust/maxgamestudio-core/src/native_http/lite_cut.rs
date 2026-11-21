mod assets;
mod audio_mix;
mod export_job;
mod export_sources;
mod exports;
mod ffmpeg;
mod project_schema;
mod project_store;
mod render;

use super::{
    demo_body::{model, text},
    demo_query::{parse_offset_page, parse_path_integer, parse_project_page},
    detail, json_reply, model_body, profile_disk, validation_reply, HttpResponse, ProfileState,
};
use hyper::{body::Incoming, Method, Request, StatusCode};
use percent_encoding::percent_decode_str;
use project_store::{Mutation, Outcome};
use rusqlite::{params, types::Type, Connection, OpenFlags, Row};
use serde_json::{json, Value};
use std::{
    error::Error,
    path::Path,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};
use tokio::sync::watch;

pub(super) use export_job::Exports as ActiveExports;

const PROJECTS_PATH: &str = "/api/lite-cut/projects";
const EXPORTS_PATH: &str = "/api/lite-cut/exports";
const PROJECT_COLUMNS: &str = "id, name, body_json, created_at, updated_at";

pub(super) enum Route {
    List,
    Project(String),
    Snapshots(String),
    Restore { project: String, snapshot: String },
    Assets(assets::Route),
    ValidateAssets,
    StartExport { background: bool },
    ExportList,
    Export(String),
    CancelExport(String),
}

impl Route {
    pub(super) fn from_path(path: &str) -> Option<Self> {
        if path == PROJECTS_PATH {
            return Some(Self::List);
        }
        if path == "/api/lite-cut/assets/validate" {
            return Some(Self::ValidateAssets);
        }
        if path == "/api/lite-cut/export/start" || path == "/api/lite-cut/export" {
            return Some(Self::StartExport {
                background: path.ends_with("/start"),
            });
        }
        let path = percent_decode_str(path).decode_utf8_lossy();
        if let Some(route) = assets::Route::from_path(&path) {
            return Some(Self::Assets(route));
        }
        if path == EXPORTS_PATH {
            return Some(Self::ExportList);
        }
        if let Some(id) = path
            .strip_prefix(EXPORTS_PATH)
            .and_then(|tail| tail.strip_prefix('/'))
        {
            return match id.split_once('/') {
                Some((id, "cancel")) if !id.is_empty() => Some(Self::CancelExport(id.to_owned())),
                None if !id.is_empty() => Some(Self::Export(id.to_owned())),
                _ => None,
            };
        }
        let tail = path.strip_prefix(PROJECTS_PATH)?.strip_prefix('/')?;
        let mut parts = tail.split('/');
        let project = parts.next().filter(|id| !id.is_empty())?;
        match (parts.next(), parts.next(), parts.next(), parts.next()) {
            (None, None, None, None) => Some(Self::Project(project.to_owned())),
            (Some("snapshots"), None, None, None) => Some(Self::Snapshots(project.to_owned())),
            (Some("snapshots"), Some(snapshot), Some("restore"), None) if !snapshot.is_empty() => {
                Some(Self::Restore {
                    project: project.to_owned(),
                    snapshot: snapshot.to_owned(),
                })
            }
            _ => None,
        }
    }

    pub(super) fn permits_write(&self, method: &Method) -> bool {
        matches!(
            (self, method),
            (Self::List, &Method::POST) | (Self::Project(_), &Method::PATCH)
        )
    }

    pub(super) fn allow_header(&self) -> &'static str {
        match self {
            Self::List => "GET, POST",
            Self::Project(_) => "GET, PATCH",
            Self::Restore { .. }
            | Self::ValidateAssets
            | Self::StartExport { .. }
            | Self::CancelExport(_) => "POST",
            _ => "GET",
        }
    }
}

pub(super) async fn respond(
    profile: &Arc<ProfileState>,
    route: Route,
    request: Request<Incoming>,
    extended_request: &AtomicBool,
    shutdown: watch::Receiver<bool>,
) -> HttpResponse {
    let uri = request.uri();
    let selection = match route {
        Route::List if request.method() == Method::POST => {
            return save(profile, request, None).await;
        }
        Route::List => match parse_offset_page(uri.query().unwrap_or_default(), 100, 500) {
            Ok((limit, offset)) => ProjectRead::Page { limit, offset },
            Err(errors) => return validation_reply(errors),
        },
        Route::Project(id) => match parse_path_integer(&id, "project_id") {
            Ok(id) if request.method() == Method::PATCH => {
                return save(profile, request, Some(id)).await
            }
            Ok(id) => ProjectRead::One(id),
            Err(errors) => return validation_reply(errors),
        },
        Route::Snapshots(id) => match parse_path_integer(&id, "project_id") {
            Ok(id) => ProjectRead::Snapshots(id),
            Err(errors) => return validation_reply(errors),
        },
        Route::Restore { project, snapshot } => {
            let id = match parse_path_integer(&project, "project_id") {
                Ok(id) => id,
                Err(errors) => return validation_reply(errors),
            };
            let snapshot = match parse_path_integer(&snapshot, "snapshot_id") {
                Ok(id) => id,
                Err(errors) => return validation_reply(errors),
            };
            return mutate(profile, Mutation::Restore { id, snapshot }).await;
        }
        Route::Assets(route) => {
            return assets::respond(profile, route, request, extended_request).await
        }
        Route::ValidateAssets => return export_sources::validate(profile, request).await,
        Route::StartExport { background } => {
            return export_job::start(profile, request, background, shutdown, extended_request)
                .await;
        }
        Route::ExportList => {
            let page = parse_project_page(uri.query().unwrap_or_default(), 20, 100);
            let (project_id, limit, offset) = match page {
                Ok(page) => page,
                Err(errors) => return validation_reply(errors),
            };
            return exports::respond(
                profile,
                exports::Read::Page {
                    project_id,
                    limit,
                    offset,
                },
            )
            .await;
        }
        Route::Export(id) => match parse_path_integer(&id, "export_id") {
            Ok(id) => return exports::respond(profile, exports::Read::One(id)).await,
            Err(errors) => return validation_reply(errors),
        },
        Route::CancelExport(id) => match parse_path_integer(&id, "export_id") {
            Ok(id) => return export_job::cancel(profile, id).await,
            Err(errors) => return validation_reply(errors),
        },
    };
    profile_disk(profile, move |profile| {
        match selection.read(&profile.config.database_path()) {
            Ok(Some(value)) => json_reply(StatusCode::OK, value),
            Ok(None) => project_not_found(),
            Err(_) => detail(StatusCode::INTERNAL_SERVER_ERROR, "无法读取剪辑工程"),
        }
    })
    .await
}

async fn save(
    profile: &Arc<ProfileState>,
    request: Request<Incoming>,
    id: Option<i64>,
) -> HttpResponse {
    let input = match model_body(request, "LiteCut project is too large").await {
        Ok(input) => input,
        Err(error) => return *error,
    };
    let object = match model(&input) {
        Ok(object) => object,
        Err(errors) => return validation_reply(errors),
    };
    let name = match object
        .get("name")
        .filter(|value| id.is_none() || !value.is_null())
    {
        Some(value) => match text(value, "name", 0, 240) {
            Ok(name) => Some(name.trim().to_owned()),
            Err(issue) => return validation_reply(vec![issue]),
        },
        None => None,
    };
    let body = object.get("body").filter(|body| !body.is_null()).cloned();
    if id.is_some() && name.is_none() && body.is_none() {
        return json_reply(
            StatusCode::BAD_REQUEST,
            json!({"detail":{"code":"LITECUT_PROJECT_NOTHING_TO_UPDATE"}}),
        );
    }
    let action = match id {
        Some(id) => {
            let body = match body
                .map(|body| project_schema::normalize(Some(body)))
                .transpose()
            {
                Ok(body) => body,
                Err(message) => return invalid_project(&message),
            };
            Mutation::Update { id, name, body }
        }
        None => match project_schema::normalize(body) {
            Ok(body) => Mutation::Create {
                name: name.unwrap_or_default(),
                body,
            },
            Err(message) => return invalid_project(&message),
        },
    };
    mutate(profile, action).await
}

async fn mutate(profile: &Arc<ProfileState>, action: Mutation) -> HttpResponse {
    profile_disk(profile, move |profile| {
        match project_store::write(&profile.config.database_path(), action) {
            Ok(Outcome::Saved(project)) => json_reply(StatusCode::OK, project),
            Ok(Outcome::ProjectMissing) => project_not_found(),
            Ok(Outcome::SnapshotMissing) => detail(StatusCode::NOT_FOUND, "snapshot not found"),
            Ok(Outcome::InvalidSnapshot(message)) => invalid_project(&message),
            Err(_) => json_reply(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"detail":{"code":"LITECUT_PROJECT_SAVE_FAILED"}}),
            ),
        }
    })
    .await
}

fn invalid_project(message: &str) -> HttpResponse {
    json_reply(
        StatusCode::UNPROCESSABLE_ENTITY,
        json!({"detail":{"code":"LITECUT_PROJECT_INVALID","message":message}}),
    )
}

fn project_not_found() -> HttpResponse {
    json_reply(
        StatusCode::NOT_FOUND,
        json!({"detail":{"code":"LITECUT_PROJECT_NOT_FOUND"}}),
    )
}

enum ProjectRead {
    Page { limit: i64, offset: i64 },
    One(i64),
    Snapshots(i64),
}

impl ProjectRead {
    fn read(self, path: &Path) -> Result<Option<Value>, Box<dyn Error>> {
        let empty_database = || match self {
            Self::Page { limit, offset } => Some(json!({"items":[],"limit":limit,"offset":offset})),
            _ => None,
        };
        let result = read_table(path, "lite_cut_projects", |transaction| {
            Ok(match self {
                Self::Page { limit, offset } => {
                    let mut query = transaction.prepare(&format!(
                        "SELECT {PROJECT_COLUMNS} FROM lite_cut_projects
                     ORDER BY updated_at DESC LIMIT ?1 OFFSET ?2"
                    ))?;
                    let rows = query.query_map(params![limit, offset], project_json)?;
                    let items = rows.collect::<rusqlite::Result<Vec<_>>>()?;
                    Some(json!({"items":items,"limit":limit,"offset":offset}))
                }
                Self::One(id) => project_store::get(transaction, id)?,
                Self::Snapshots(id) => read_snapshots(transaction, id)?,
            })
        })?;
        Ok(result.unwrap_or_else(empty_database))
    }
}

fn read_table<T>(
    path: &Path,
    table: &str,
    read: impl FnOnce(&Connection) -> rusqlite::Result<T>,
) -> Result<Option<T>, Box<dyn Error>> {
    if !path.try_exists()? {
        return Ok(None);
    }
    let mut connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(Duration::from_secs(5))?;
    let transaction = connection.transaction()?;
    if !has_table(&transaction, table)? {
        return Ok(None);
    }
    let result = read(&transaction)?;
    transaction.commit()?;
    Ok(Some(result))
}

fn has_table(connection: &Connection, name: &str) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1)",
        [name],
        |row| row.get(0),
    )
}

fn project_json(row: &Row<'_>) -> rusqlite::Result<Value> {
    let stored: String = row.get(2)?;
    let body: Value = serde_json::from_str(&stored).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(2, Type::Text, Box::new(error))
    })?;
    Ok(json!({
        "id":row.get::<_, i64>(0)?,
        "name":row.get::<_, String>(1)?,
        "body":body,
        "created_at":row.get::<_, String>(3)?,
        "updated_at":row.get::<_, String>(4)?,
    }))
}

fn read_snapshots(connection: &Connection, id: i64) -> rusqlite::Result<Option<Value>> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM lite_cut_projects WHERE id = ?1)",
        [id],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(None);
    }
    let mut items = Vec::new();
    if has_table(connection, "lite_cut_project_snapshots")? {
        let mut query = connection.prepare(
            "SELECT id, name, reason, created_at FROM lite_cut_project_snapshots
             WHERE project_id = ?1 ORDER BY id DESC LIMIT 50",
        )?;
        let rows = query.query_map([id], |row| {
            Ok(json!({
                "id":row.get::<_, i64>(0)?,
                "project_id":id,
                "name":row.get::<_, String>(1)?,
                "reason":row.get::<_, String>(2)?,
                "created_at":row.get::<_, String>(3)?,
            }))
        })?;
        items = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    }
    Ok(Some(json!({"items":items})))
}
