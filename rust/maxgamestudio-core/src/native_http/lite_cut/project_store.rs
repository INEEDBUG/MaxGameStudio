use super::{has_table, project_json, project_schema, PROJECT_COLUMNS};
use crate::case_fold::case_fold;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde_json::Value;
use std::{
    collections::HashSet,
    error::Error,
    fs,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const PROJECTS: &str = "lite_cut_projects";
const SNAPSHOTS: &str = "lite_cut_project_snapshots";

pub(super) enum Mutation {
    Create {
        name: String,
        body: Value,
    },
    Update {
        id: i64,
        name: Option<String>,
        body: Option<Value>,
    },
    Restore {
        id: i64,
        snapshot: i64,
    },
}

pub(super) enum Outcome {
    Saved(Value),
    ProjectMissing,
    SnapshotMissing,
    InvalidSnapshot(String),
}

pub(super) fn write(path: &Path, action: Mutation) -> Result<Outcome, Box<dyn Error>> {
    let can_create = matches!(action, Mutation::Create { .. });
    let mut mode = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    if can_create {
        if let Some(directory) = path.parent() {
            fs::create_dir_all(directory)?;
        }
        mode |= OpenFlags::SQLITE_OPEN_CREATE;
    } else if !path.try_exists()? {
        return Ok(Outcome::ProjectMissing);
    }
    let mut db = Connection::open_with_flags(path, mode)?;
    db.busy_timeout(Duration::from_secs(5))?;
    let transaction = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let outcome = match action {
        Mutation::Create { name, body } => create(&transaction, &name, &body)?,
        Mutation::Update { id, name, body } => {
            update(&transaction, id, name.as_deref(), body.as_ref())?
        }
        Mutation::Restore { id, snapshot } => restore(&transaction, id, snapshot)?,
    };
    transaction.commit()?;
    Ok(outcome)
}

pub(super) fn get(db: &Connection, id: i64) -> rusqlite::Result<Option<Value>> {
    db.query_row(
        &format!("SELECT {PROJECT_COLUMNS} FROM lite_cut_projects WHERE id = ?1"),
        [id],
        project_json,
    )
    .optional()
}

fn create(db: &Connection, name: &str, body: &Value) -> Result<Outcome, Box<dyn Error>> {
    ensure_projects(db)?;
    let name = unique_name(db, name, None)?;
    let now = timestamp(db)?;
    db.execute(
        "INSERT INTO lite_cut_projects(name,body_json,created_at,updated_at) VALUES(?1,?2,?3,?3)",
        params![name, body.to_string(), now],
    )?;
    updated_project(db, db.last_insert_rowid())
}

pub(super) fn ensure_projects(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS lite_cut_projects(
            id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL DEFAULT '',
            body_json TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_lite_cut_projects_updated
         ON lite_cut_projects(updated_at DESC);",
    )
}

fn update(
    db: &Connection,
    id: i64,
    requested_name: Option<&str>,
    body: Option<&Value>,
) -> Result<Outcome, Box<dyn Error>> {
    let Some(current) = existing_project(db, id)? else {
        return Ok(Outcome::ProjectMissing);
    };
    let existing_name = current["name"].as_str().expect("stored project name");
    let new_name = requested_name
        .map(|name| unique_name(db, name, Some(id)))
        .transpose()?;
    let now = timestamp(db)?;
    if let Some(body) = body {
        snapshot(
            db,
            id,
            requested_name.unwrap_or(existing_name),
            body,
            "save",
            &now,
        )?;
    }
    db.execute(
        "UPDATE lite_cut_projects SET name=coalesce(?1,name), body_json=coalesce(?2,body_json),
             updated_at=?3 WHERE id=?4",
        params![new_name, body.map(Value::to_string), now, id],
    )?;
    updated_project(db, id)
}

fn restore(db: &Connection, id: i64, snapshot_id: i64) -> Result<Outcome, Box<dyn Error>> {
    let Some(current) = existing_project(db, id)? else {
        return Ok(Outcome::ProjectMissing);
    };
    if !has_table(db, SNAPSHOTS)? {
        return Ok(Outcome::SnapshotMissing);
    }
    let raw: Option<String> = db
        .query_row(
            "SELECT body_json FROM lite_cut_project_snapshots WHERE project_id=?1 AND id=?2",
            params![id, snapshot_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(raw) = raw else {
        return Ok(Outcome::SnapshotMissing);
    };
    let normalized = match serde_json::from_str(&raw) {
        Ok(body) => match project_schema::normalize(Some(body)) {
            Ok(body) => body,
            Err(message) => return Ok(Outcome::InvalidSnapshot(message)),
        },
        Err(_) => {
            return Ok(Outcome::InvalidSnapshot(
                "恢复快照里的工程内容无效".to_owned(),
            ))
        }
    };
    let now = timestamp(db)?;
    snapshot(
        db,
        id,
        current["name"].as_str().expect("stored project name"),
        &current["body"],
        "before_restore",
        &now,
    )?;
    db.execute(
        "UPDATE lite_cut_projects SET body_json=?1, updated_at=?2 WHERE id=?3",
        params![normalized.to_string(), now, id],
    )?;
    updated_project(db, id)
}

fn existing_project(db: &Connection, id: i64) -> rusqlite::Result<Option<Value>> {
    if has_table(db, PROJECTS)? {
        get(db, id)
    } else {
        Ok(None)
    }
}

fn updated_project(db: &Connection, id: i64) -> Result<Outcome, Box<dyn Error>> {
    let record = get(db, id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
    Ok(Outcome::Saved(record))
}

pub(super) fn snapshot(
    db: &Connection,
    id: i64,
    name: &str,
    body: &Value,
    reason: &str,
    now: &str,
) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS lite_cut_project_snapshots(
            id INTEGER PRIMARY KEY AUTOINCREMENT, project_id INTEGER NOT NULL, name TEXT NOT NULL,
            body_json TEXT NOT NULL, reason TEXT NOT NULL DEFAULT 'save', created_at TEXT NOT NULL,
            FOREIGN KEY(project_id) REFERENCES lite_cut_projects(id)
         );
         CREATE INDEX IF NOT EXISTS idx_lite_cut_snapshots_project_created
         ON lite_cut_project_snapshots(project_id, created_at DESC);",
    )?;
    let newest: Option<String> = db.query_row(
        "SELECT body_json FROM lite_cut_project_snapshots WHERE project_id=?1 ORDER BY id DESC LIMIT 1",
        [id],
        |row| row.get(0),
    ).optional()?;
    if newest
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .as_ref()
        == Some(body)
    {
        return Ok(());
    }
    db.execute(
        "INSERT INTO lite_cut_project_snapshots(project_id,name,body_json,reason,created_at)
         VALUES(?1,?2,?3,?4,?5)",
        params![id, name, body.to_string(), reason, now],
    )?;
    db.execute(
        "DELETE FROM lite_cut_project_snapshots WHERE project_id=?1 AND id NOT IN
         (SELECT id FROM lite_cut_project_snapshots WHERE project_id=?1 ORDER BY id DESC LIMIT 50)",
        [id],
    )?;
    Ok(())
}

fn unique_name(db: &Connection, name: &str, exclude: Option<i64>) -> rusqlite::Result<String> {
    let trim = |ch: char| ch.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&ch);
    let base = match name.trim_matches(trim) {
        "" => "未命名工程",
        name => name,
    };
    let mut query = db.prepare("SELECT name FROM lite_cut_projects WHERE ?1 IS NULL OR id<>?1")?;
    let names = query
        .query_map([exclude], |row| {
            let name: String = row.get(0)?;
            Ok(case_fold(name.trim_matches(trim)))
        })?
        .collect::<rusqlite::Result<HashSet<_>>>()?;
    let folded = case_fold(base);
    if !names.contains(&folded) {
        return Ok(base.to_owned());
    }
    for suffix in 1usize.. {
        if !names.contains(&format!("{folded} ({suffix})")) {
            return Ok(format!("{base} ({suffix})"));
        }
    }
    unreachable!("a finite project catalogue always has an available name")
}

pub(super) fn timestamp(db: &Connection) -> Result<String, Box<dyn Error>> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH)?;
    let second: String = db.query_row(
        "SELECT strftime('%Y-%m-%dT%H:%M:%S', ?1, 'unixepoch')",
        [i64::try_from(elapsed.as_secs())?],
        |row| row.get(0),
    )?;
    Ok(format!("{second}.{:06}+00:00", elapsed.subsec_micros()))
}
