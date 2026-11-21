use super::{
    demo_query::{parse_offset_page, parse_path_integer},
    detail, json_reply, profile_disk, validation_reply, HttpResponse, ProfileState,
};
use hyper::{body::Incoming, Request, StatusCode};
use percent_encoding::percent_decode_str;
use rusqlite::{params, types::Type, Connection, OpenFlags, OptionalExtension, Row};
use serde_json::{json, Value};
use std::{error::Error, path::Path, sync::Arc, time::Duration};

const PROJECTS_PATH: &str = "/api/lite-cut/projects";
const PROJECT_COLUMNS: &str = "id, name, body_json, created_at, updated_at";

pub(super) enum Route {
    List,
    Project,
    Snapshots,
}

impl Route {
    pub(super) fn from_path(path: &str) -> Option<Self> {
        if path == PROJECTS_PATH {
            return Some(Self::List);
        }
        let path = percent_decode_str(path).decode_utf8_lossy();
        let tail = path.strip_prefix(PROJECTS_PATH)?.strip_prefix('/')?;
        match tail.split_once('/') {
            None if !tail.is_empty() => Some(Self::Project),
            Some((id, "snapshots")) if !id.is_empty() => Some(Self::Snapshots),
            _ => None,
        }
    }
}

pub(super) async fn respond(
    profile: &Arc<ProfileState>,
    route: Route,
    request: Request<Incoming>,
) -> HttpResponse {
    let uri = request.uri();
    let selection = match route {
        Route::List => match parse_offset_page(uri.query().unwrap_or_default(), 100, 500) {
            Ok((limit, offset)) => ProjectRead::Page { limit, offset },
            Err(errors) => return validation_reply(errors),
        },
        Route::Project | Route::Snapshots => {
            let path = percent_decode_str(uri.path()).decode_utf8_lossy();
            let id = path[PROJECTS_PATH.len() + 1..]
                .split('/')
                .next()
                .expect("project path");
            match parse_path_integer(id, "project_id") {
                Ok(id) if matches!(route, Route::Snapshots) => ProjectRead::Snapshots(id),
                Ok(id) => ProjectRead::One(id),
                Err(errors) => return validation_reply(errors),
            }
        }
    };
    profile_disk(profile, move |profile| {
        match selection.read(&profile.config.database_path()) {
            Ok(Some(value)) => json_reply(StatusCode::OK, value),
            Ok(None) => json_reply(
                StatusCode::NOT_FOUND,
                json!({"detail":{"code":"LITECUT_PROJECT_NOT_FOUND"}}),
            ),
            Err(_) => detail(StatusCode::INTERNAL_SERVER_ERROR, "无法读取剪辑工程"),
        }
    })
    .await
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
        if !path.try_exists()? {
            return Ok(empty_database());
        }
        let mut connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(Duration::from_secs(5))?;
        let transaction = connection.transaction()?;
        if !has_table(&transaction, "lite_cut_projects")? {
            return Ok(empty_database());
        }
        let result = match self {
            Self::Page { limit, offset } => {
                let mut query = transaction.prepare(&format!(
                    "SELECT {PROJECT_COLUMNS} FROM lite_cut_projects
                     ORDER BY updated_at DESC LIMIT ?1 OFFSET ?2"
                ))?;
                let rows = query.query_map(params![limit, offset], project_json)?;
                let items = rows.collect::<rusqlite::Result<Vec<_>>>()?;
                Some(json!({"items":items,"limit":limit,"offset":offset}))
            }
            Self::One(id) => transaction
                .query_row(
                    &format!("SELECT {PROJECT_COLUMNS} FROM lite_cut_projects WHERE id = ?1"),
                    [id],
                    project_json,
                )
                .optional()?,
            Self::Snapshots(id) => read_snapshots(&transaction, id)?,
        };
        transaction.commit()?;
        Ok(result)
    }
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
