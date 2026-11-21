use super::{detail, json_reply, profile_disk, read_table, HttpResponse, ProfileState};
use hyper::StatusCode;
use rusqlite::{params, OptionalExtension, Row};
use serde_json::{json, Value};
use std::{error::Error, path::Path, sync::Arc};

const TABLE: &str = "lite_cut_exports";
const COLUMNS: &str = "id, project_id, status, error_msg, output_path, created_at, updated_at";

pub(super) enum Read {
    Page {
        project_id: Option<i64>,
        limit: i64,
        offset: i64,
    },
    One(i64),
}

pub(super) async fn respond(profile: &Arc<ProfileState>, selection: Read) -> HttpResponse {
    profile_disk(profile, move |profile| {
        match selection.read(&profile.config.database_path()) {
            Ok(Some(mut value)) => {
                if let Some(items) = value.get_mut("items").and_then(Value::as_array_mut) {
                    for item in items {
                        profile.lite_cut_exports.apply_progress(item);
                    }
                } else {
                    profile.lite_cut_exports.apply_progress(&mut value);
                }
                json_reply(StatusCode::OK, value)
            }
            Ok(None) => json_reply(
                StatusCode::NOT_FOUND,
                json!({"detail":{"code":"LITECUT_EXPORT_NOT_FOUND"}}),
            ),
            Err(_) => detail(StatusCode::INTERNAL_SERVER_ERROR, "无法读取剪辑导出记录"),
        }
    })
    .await
}

impl Read {
    pub(super) fn read(self, path: &Path) -> Result<Option<Value>, Box<dyn Error>> {
        match self {
            Self::One(id) => Ok(read_table(path, TABLE, |db| {
                db.query_row(
                    &format!("SELECT {COLUMNS} FROM {TABLE} WHERE id = ?1"),
                    [id],
                    export_json,
                )
                .optional()
            })?
            .flatten()),
            Self::Page {
                project_id,
                limit,
                offset,
            } => {
                let items = read_table(path, TABLE, |db| {
                    let mut query = db.prepare(&format!(
                        "SELECT {COLUMNS} FROM {TABLE} WHERE (?1 IS NULL OR project_id = ?1)
                         ORDER BY updated_at DESC LIMIT ?2 OFFSET ?3"
                    ))?;
                    let rows = query.query_map(params![project_id, limit, offset], export_json)?;
                    rows.collect::<rusqlite::Result<Vec<_>>>()
                })?
                .unwrap_or_default();
                Ok(Some(json!({"items":items,"limit":limit,"offset":offset})))
            }
        }
    }
}

fn export_json(row: &Row<'_>) -> rusqlite::Result<Value> {
    let status = row
        .get::<_, Option<String>>(2)?
        .filter(|status| !status.is_empty());
    let status = status.as_deref().unwrap_or("unknown");
    Ok(json!({
        "export_id":row.get::<_, i64>(0)?,
        "project_id":row.get::<_, Option<i64>>(1)?,
        "status":status,
        "progress":if status == "done" {1.0} else {0.0},
        "stage":status,
        "error":row.get::<_, Option<String>>(3)?.unwrap_or_default(),
        "output_path":row.get::<_, Option<String>>(4)?.unwrap_or_default(),
        "created_at":row.get::<_, String>(5)?,
        "updated_at":row.get::<_, String>(6)?,
    }))
}
