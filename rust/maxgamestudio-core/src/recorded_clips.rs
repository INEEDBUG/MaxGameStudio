//! Read and update recorded clip metadata in the existing shared SQLite database.

use rusqlite::{params, types::ValueRef, Connection, OpenFlags, Params, Row};
use serde_json::{Map, Number, Value};
use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug)]
pub(crate) enum RecordedClipsError {
    File(io::Error),
    Sqlite(rusqlite::Error),
}

impl fmt::Display for RecordedClipsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File(error) => error.fmt(formatter),
            Self::Sqlite(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for RecordedClipsError {}

impl From<rusqlite::Error> for RecordedClipsError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

/// Return a page ordered by the database ID, before applying metadata overrides.
/// The HTTP caller validates the page size and offset.
pub(crate) fn list(path: &Path, limit: i64, offset: i64) -> Result<Vec<Value>, RecordedClipsError> {
    read(
        path,
        "ORDER BY id DESC LIMIT ?1 OFFSET ?2",
        params![limit, offset],
    )
}

pub(crate) fn output_path(path: &Path, id: i64) -> Result<Option<String>, RecordedClipsError> {
    Ok(read(path, "WHERE id = ?1", [id])?
        .into_iter()
        .next()
        .map(|item| item["output_path"].as_str().unwrap_or_default().to_owned()))
}

pub(crate) fn update_duration(
    path: &Path,
    id: i64,
    seconds: f64,
) -> Result<bool, RecordedClipsError> {
    if id <= 0 {
        return Ok(false);
    }
    let Some(connection) = open_existing(path, OpenFlags::SQLITE_OPEN_READ_WRITE)? else {
        return Ok(false);
    };
    let available: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'recorded_clips')",
        [],
        |row| row.get(0),
    )?;
    if !available {
        return Ok(false);
    }
    Ok(connection.execute(
        "UPDATE recorded_clips SET duration_sec = ?1 WHERE id = ?2",
        params![seconds, id],
    )? > 0)
}

fn read(
    path: &Path,
    selection: &str,
    bindings: impl Params,
) -> Result<Vec<Value>, RecordedClipsError> {
    let Some(mut connection) = open_existing(path, OpenFlags::SQLITE_OPEN_READ_ONLY)? else {
        return Ok(Vec::new());
    };
    let transaction = connection.transaction()?;
    let has_metadata: Option<bool> = transaction.query_row(
        "SELECT max(lower(name) = 'clip_meta') FROM pragma_table_info('recorded_clips')",
        [],
        |row| row.get(0),
    )?;
    let Some(has_metadata) = has_metadata else {
        return Ok(Vec::new());
    };
    let metadata_column = if has_metadata { "clip_meta" } else { "NULL" };
    let items = {
        let mut statement = transaction.prepare(&format!(
            "SELECT id, clip_id, demo_path, demo_filename, player_name, output_path,
             duration_sec, status, created_at, {metadata_column} AS clip_meta
             FROM recorded_clips {selection}"
        ))?;
        let rows = statement.query_map(bindings, row_to_json)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    transaction.commit()?;
    Ok(items)
}

fn open_existing(path: &Path, mode: OpenFlags) -> Result<Option<Connection>, RecordedClipsError> {
    match fs::metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(RecordedClipsError::File(error)),
        Ok(metadata) if !metadata.is_file() => {
            return Err(RecordedClipsError::File(io::Error::new(
                io::ErrorKind::InvalidInput,
                "recorded clips database path must be a file",
            )));
        }
        Ok(_) => {}
    }
    let connection = Connection::open_with_flags(path, mode | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    Ok(Some(connection))
}
/// Read a lexical path from the newest expanded record. Recordings can reside on an
/// unplugged drive, so the directory and recording themselves need no filesystem check.
pub(crate) fn recent_output_dir(path: &Path) -> Result<String, RecordedClipsError> {
    let Some(row) = list(path, 1, 0)?.into_iter().next() else {
        return Ok(String::new());
    };
    Ok(match &row["output_path"] {
        Value::String(text) => output_parent(text),
        Value::Null | Value::Bool(false) => String::new(),
        Value::Number(number) if number.as_f64() == Some(0.0) => String::new(),
        Value::Array(values) if values.is_empty() => String::new(),
        Value::Object(values) if values.is_empty() => String::new(),
        value => output_parent(&value.to_string()),
    })
}

fn row_to_json(row: &Row<'_>) -> rusqlite::Result<Value> {
    let mut item = Map::new();
    for index in 0..row.as_ref().column_count() {
        let field = row.as_ref().column_name(index)?;
        let raw = row.get_ref(index)?;
        if field == "clip_meta" {
            if matches!(raw, ValueRef::Text(_)) {
                let stored: String = row.get(index)?;
                if let Ok(Value::Object(metadata)) = serde_json::from_str(&stored) {
                    item.extend(metadata);
                }
            }
            continue;
        }
        let invalid =
            || rusqlite::Error::InvalidColumnType(index, field.to_owned(), raw.data_type());
        let value = match raw {
            ValueRef::Null => Value::Null,
            ValueRef::Integer(value) => Value::from(value),
            ValueRef::Real(value) => Value::Number(Number::from_f64(value).ok_or_else(invalid)?),
            ValueRef::Text(_) => Value::String(row.get(index)?),
            ValueRef::Blob(_) => return Err(invalid()),
        };
        item.insert(field.to_owned(), value);
    }
    Ok(Value::Object(item))
}

fn output_parent(text: &str) -> String {
    let text =
        text.trim_matches(|ch: char| ch.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&ch));
    if text.is_empty() {
        return String::new();
    }
    let normalized: PathBuf = Path::new(text).components().collect();
    let parent = normalized.parent().unwrap_or(&normalized);
    if parent.as_os_str().is_empty() {
        ".".to_owned()
    } else {
        parent.to_string_lossy().into_owned()
    }
}

#[cfg(test)]
#[path = "recorded_clips_tests.rs"]
mod tests;
