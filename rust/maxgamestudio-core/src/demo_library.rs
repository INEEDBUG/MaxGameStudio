//! Read and edit metadata in the deployed Demo library schema.

mod analysis;
mod roster;
pub(crate) use analysis::AnalysisSource;
pub(crate) use roster::{normalize_players, BatchRosterSource, RosterCache, RosterSource};

use crate::wire_json::json_decode_error;
use rusqlite::{
    params_from_iter,
    types::{Value as SqlValue, ValueRef},
    Connection, OpenFlags, OptionalExtension, Params, Row, TransactionBehavior,
};
use serde_json::{json, Map, Number, Value};
use std::{
    collections::{BTreeSet, HashMap},
    fmt, fs, io,
    path::{Path, PathBuf},
    time::Duration,
};

const SCHEMA_TABLES: &str = "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table'
    AND name IN ('demo_files', 'match_results', 'demo_result_summaries', 'demo_player_stats')";
const DEMO_COLUMNS: &str = "d.id, d.path, d.filename, d.display_name, d.file_size, d.status,
    d.added_at, d.parsed_at, d.error_msg, d.map_name, d.total_rounds, d.team_a_score, d.team_b_score,
    d.team_a_name, d.team_b_name, d.duration_mins, d.match_date, d.match_date_source, d.source,
    d.remark, d.content_md5, d.origin_zip";
const RESULT_COLUMNS: &str = "r.result_json, r.created_at AS result_created_at";
const SUMMARY_COLUMNS: &str = "CASE WHEN rs.demo_path IS NULL THEN 0 ELSE 1 END AS has_result,
    COALESCE(rs.clip_count, 0) AS clip_count, rs.primary_target,
    COALESCE(rs.analyzed_targets_json, '[]') AS analyzed_targets_json,
    COALESCE(rs.four_k_count, 0) AS four_k_count, COALESCE(rs.five_k_count, 0) AS five_k_count,
    rs.result_created_at";
const RESULT_JOIN: &str = "LEFT JOIN match_results r ON r.demo_path = d.path";
const SUMMARY_JOIN: &str = "LEFT JOIN demo_result_summaries rs ON rs.demo_path = d.path";
const PLAYER_COLUMNS: &str = "demo_id, player_name AS name, player_name, steam_id64,
    COALESCE(steam_id64, steam_id) AS steamid64, steam_id, account_id, user_id, team_number,
    team_number AS team, team_name, kills, deaths, assists, kd";
const PLAYER_STATS_COLUMNS: &str = "id, demo_id, demo_path, steam_id64, steam_id, account_id,
    user_id, player_name, normalized_name, team_name, team_number, kills, deaths, assists, kd,
    indexed_at";
const STATUS_RANK: &str = "CASE lower(ifnull(d.status, '')) WHEN 'loaded' THEN 0
    WHEN 'parsing' THEN 1 WHEN 'error' THEN 2 WHEN 'done' THEN 3 ELSE 4 END ASC";
const MATCH_DATE: &str = "julianday(NULLIF(d.match_date, ''))";
const DISCOVERED_FILTER: &str = "FROM demo_files d WHERE d.status = 'pending'
    AND NOT EXISTS (
        SELECT 1 FROM demo_files ing WHERE ing.id != d.id AND ing.path COLLATE NOCASE = d.path
        AND (ing.status != 'pending' OR ing.id < d.id)
    )
    AND NOT EXISTS (
        SELECT 1 FROM demo_files ing WHERE ing.id != d.id AND ing.filename COLLATE NOCASE = d.filename
        AND (ing.status != 'pending' OR ing.id < d.id)
    )
    AND NOT EXISTS (
        SELECT 1 FROM demo_files ing WHERE ing.id != d.id
        AND d.content_md5 IS NOT NULL AND trim(d.content_md5) != '' AND ing.content_md5 = d.content_md5
        AND (ing.status != 'pending' OR ing.id < d.id)
    )";

pub(crate) enum DemoListKind {
    Full,
    Compact,
    Ids,
}

pub(crate) struct DemoPage {
    pub limit: i64,
    pub offset: i64,
    pub q: Option<String>,
}

pub(crate) struct DemoListQuery {
    pub kind: DemoListKind,
    pub page: DemoPage,
    pub map_names: Vec<String>,
    pub statuses: Vec<String>,
    pub min_kills: Option<i64>,
    pub max_deaths: Option<i64>,
    pub min_assists: Option<i64>,
    pub min_kd: Option<f64>,
    pub player_query: Option<String>,
    pub steam_query: Option<String>,
    pub rounds_min: Option<i64>,
    pub rounds_max: Option<i64>,
    pub duration_min: Option<f64>,
    pub duration_max: Option<f64>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub sort_key: String,
    pub sort_asc: bool,
}

pub(crate) enum DemoEdit {
    DisplayName(String),
    Remark(String),
}

#[derive(Debug)]
pub(crate) enum DemoReadError {
    Sqlite(rusqlite::Error),
    File(io::Error),
    IncompleteSchema,
    InvalidFullResult,
}

impl fmt::Display for DemoReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "{error}"),
            Self::File(error) => write!(formatter, "{error}"),
            Self::IncompleteSchema => formatter.write_str("deployed Demo tables are incomplete"),
            Self::InvalidFullResult => formatter.write_str("a stored demo result is invalid"),
        }
    }
}

impl From<rusqlite::Error> for DemoReadError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

type DemoResult<T> = Result<T, DemoReadError>;
type Object = Map<String, Value>;

pub(crate) struct DemoLibrary {
    path: PathBuf,
}

impl DemoLibrary {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub(crate) fn list(&self, query: DemoListQuery) -> DemoResult<Value> {
        read_library(&self.path, |conn| {
            let DemoPage { limit, offset, q } = &query.page;
            let mut response = json!({"limit": limit, "offset": offset, "q": q});
            let mut filter = DemoFilter::from_query(&query);
            let order = order_by(&query);
            match query.kind {
                DemoListKind::Ids => {
                    response["ids"] = match conn {
                        Some(conn) => {
                            let mut stmt = conn.prepare(&format!(
                                "SELECT DISTINCT d.id FROM demo_files d {} {order} LIMIT ? OFFSET ?",
                                filter.from_tail
                            ))?;
                            filter.values.extend([(*limit).into(), (*offset).into()]);
                            let ids = stmt.query_map(params_from_iter(&filter.values), |row| {
                                row.get::<_, i64>(0)
                            })?;
                            json!(ids.collect::<rusqlite::Result<Vec<_>>>()?)
                        }
                        None => json!([]),
                    };
                }
                DemoListKind::Full | DemoListKind::Compact => {
                    let (total, rows) = match conn {
                        Some(conn) => list_rows(conn, &query, filter, &order)?,
                        None => (0, Vec::new()),
                    };
                    response["total"] = json!(total);
                    response["items"] = Value::Array(rows.into_iter().map(Value::Object).collect());
                }
            }
            Ok(response)
        })
    }

    pub(crate) fn detail(&self, id: i64) -> DemoResult<Option<Value>> {
        read_library(&self.path, |conn| match conn {
            Some(conn) => detail_row(conn, id),
            None => Ok(None),
        })
    }

    pub(crate) fn replay_workspace(&self, demo: &Path) -> DemoResult<Option<Value>> {
        let path = crate::profile_platform::path_text(demo);
        read_library(&self.path, |connection| {
            let Some(connection) = connection else {
                return Ok(None);
            };
            let mut statement = connection.prepare(
                "SELECT result_json FROM match_results WHERE demo_path = ?1 ORDER BY id DESC LIMIT 1",
            )?;
            let mut rows = statement.query([path])?;
            let Some(row) = rows.next()? else {
                return Ok(None);
            };
            let mut result =
                decode_result(row.get_ref(0)?).map_err(|_| DemoReadError::InvalidFullResult)?;
            let workspace = result["analysis_workspace"].take();
            Ok(workspace.is_object().then_some(workspace))
        })
    }

    pub(crate) fn edit(&self, id: i64, edit: DemoEdit) -> DemoResult<Option<Value>> {
        let Some(mut connection) = open_library(&self.path, OpenFlags::SQLITE_OPEN_READ_WRITE)?
        else {
            return Ok(None);
        };
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !schema_ready(&transaction)? {
            return Ok(None);
        }
        let (sql, value) = match &edit {
            DemoEdit::DisplayName(value) => (
                "UPDATE demo_files SET display_name = ? WHERE id = ?",
                value.trim(),
            ),
            DemoEdit::Remark(value) => (
                "UPDATE demo_files SET remark = ? WHERE id = ?",
                value.as_str(),
            ),
        };
        if transaction.execute(
            sql,
            rusqlite::params![(!value.is_empty()).then_some(value), id],
        )? == 0
        {
            return Ok(None);
        }
        let response = match edit {
            DemoEdit::DisplayName(_) => detail_row(&transaction, id)?,
            DemoEdit::Remark(_) => Some(json!({"status": "ok", "demo_id": id})),
        };
        if response.is_some() {
            transaction.commit()?;
        }
        Ok(response)
    }

    pub(crate) fn discovered(&self, page: DemoPage) -> DemoResult<Value> {
        let DemoPage { limit, offset, q } = page;
        let (total, items) = read_library(&self.path, |conn| {
            let Some(conn) = conn else {
                return Ok((0i64, Vec::new()));
            };
            let mut sql = DISCOVERED_FILTER.to_owned();
            let mut values = Vec::new();
            if let Some(query) = &q {
                sql.push_str(" AND d.filename LIKE ?");
                values.push(SqlValue::Text(format!("%{query}%")));
            }
            let total = conn.query_row(
                &format!("SELECT COUNT(*) {sql}"),
                params_from_iter(&values),
                |row| row.get::<_, i64>(0),
            )?;
            values.extend([limit.into(), offset.into()]);
            let items = select_objects(
                conn,
                &format!(
                    "SELECT d.id, d.path, d.filename, d.file_size, d.source, d.added_at
                    {sql} ORDER BY d.id DESC LIMIT ? OFFSET ?"
                ),
                params_from_iter(&values),
            )?;
            Ok((total, items))
        })?;
        Ok(json!({"items": items, "limit": limit, "offset": offset, "total": total, "q": q}))
    }

    pub(crate) fn player_stats(&self, id: i64) -> DemoResult<Option<Value>> {
        read_library(&self.path, |conn| {
            let Some(conn) = conn else {
                return Ok(None);
            };
            let exists = conn
                .query_row("SELECT 1 FROM demo_files WHERE id = ?", [id], |row| {
                    row.get::<_, i64>(0)
                })
                .optional()?
                .is_some();
            if !exists {
                return Ok(None);
            }
            let rows = select_objects(
                conn,
                &format!(
                    "SELECT {PLAYER_STATS_COLUMNS} FROM demo_player_stats
                 WHERE demo_id = ? ORDER BY kills DESC, player_name ASC"
                ),
                [id],
            )?;
            Ok(Some(json!({"demo_id": id, "players": rows})))
        })
    }
}

/// A page, its total and its roster read from the same SQLite snapshot.
fn read_library<T>(
    path: &Path,
    operation: impl FnOnce(Option<&Connection>) -> DemoResult<T>,
) -> DemoResult<T> {
    let Some(mut connection) = open_library(path, OpenFlags::SQLITE_OPEN_READ_ONLY)? else {
        return operation(None);
    };
    let transaction = connection.transaction()?;
    let result = operation(schema_ready(&transaction)?.then_some(&*transaction))?;
    transaction.commit()?;
    Ok(result)
}

fn open_library(path: &Path, flags: OpenFlags) -> DemoResult<Option<Connection>> {
    match fs::metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(DemoReadError::File(error)),
        Ok(_) => {}
    }
    let connection = Connection::open_with_flags(path, flags | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    Ok(Some(connection))
}

fn schema_ready(connection: &Connection) -> DemoResult<bool> {
    let count: i64 = connection.query_row(SCHEMA_TABLES, [], |row| row.get(0))?;
    match count {
        0 => Ok(false), // A shared DB can already contain training tables on first launch.
        4 => Ok(true),
        _ => Err(DemoReadError::IncompleteSchema),
    }
}

fn detail_row(conn: &Connection, id: i64) -> DemoResult<Option<Value>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT DISTINCT {DEMO_COLUMNS}, {RESULT_COLUMNS} FROM demo_files d
         {RESULT_JOIN} WHERE d.id = ?"
    ))?;
    let mut rows = stmt.query([id])?;
    let mut item = None;
    // The deployed batch detail handler retains the last matching joined result.
    while let Some(row) = rows.next()? {
        item = Some(full_row(row, true)?);
    }
    if let Some(row) = &mut item {
        attach_players(conn, std::slice::from_mut(row))?;
    }
    Ok(item.map(Value::Object))
}

struct DemoFilter {
    from_tail: String,
    values: Vec<SqlValue>,
}

impl DemoFilter {
    fn from_query(query: &DemoListQuery) -> Self {
        let mut clauses = vec!["d.status != 'pending'".to_owned()];
        let mut values = Vec::new();
        if let Some(q) = &query.page.q {
            clauses.push(
                "(instr(lower(d.filename), lower(?)) > 0 OR
                           instr(lower(ifnull(d.display_name, '')), lower(?)) > 0)"
                    .to_owned(),
            );
            values.extend([q.clone().into(), q.clone().into()]);
        }
        for (column, selections) in [
            ("d.map_name", &query.map_names),
            ("d.status", &query.statuses),
        ] {
            if !selections.is_empty() {
                clauses.push(format!("{column} IN ({})", placeholders(selections.len())));
                values.extend(selections.iter().cloned().map(SqlValue::Text));
            }
        }
        let player_join = query.player_query.is_some()
            || query.steam_query.is_some()
            || query.min_kills.is_some()
            || query.max_deaths.is_some()
            || query.min_assists.is_some()
            || query.min_kd.is_some();
        if let Some(player) = &query.player_query {
            clauses.push(
                "(instr(lower(ifnull(ps.player_name, '')), lower(?)) > 0 OR
                           instr(lower(ifnull(ps.normalized_name, '')), lower(?)) > 0)"
                    .to_owned(),
            );
            values.extend([player.clone().into(), player.clone().into()]);
        }
        if let Some(steam) = &query.steam_query {
            clauses.push(
                "(instr(ifnull(ps.steam_id64, ''), ?) > 0 OR
                           instr(ifnull(ps.steam_id, ''), ?) > 0 OR
                           instr(ifnull(ps.account_id, ''), ?) > 0)"
                    .to_owned(),
            );
            values.extend([
                steam.clone().into(),
                steam.clone().into(),
                steam.clone().into(),
            ]);
        }
        for (clause, value) in [
            ("d.total_rounds >= ?", query.rounds_min),
            ("d.total_rounds <= ?", query.rounds_max),
            ("ps.kills >= ?", query.min_kills),
            ("ps.deaths <= ?", query.max_deaths),
            ("ps.assists >= ?", query.min_assists),
        ] {
            if let Some(value) = value {
                clauses.push(clause.to_owned());
                values.push(value.into());
            }
        }
        for (clause, value) in [
            ("d.duration_mins >= ?", query.duration_min),
            ("d.duration_mins <= ?", query.duration_max),
            ("ps.kd >= ?", query.min_kd),
        ] {
            if let Some(value) = value {
                clauses.push(clause.to_owned());
                values.push(value.into());
            }
        }
        if let Some(date) = &query.date_from {
            clauses.push(format!("{MATCH_DATE} >= julianday(?)"));
            values.push(date.clone().into());
        }
        if let Some(date) = &query.date_to {
            let end = if date_only(date) {
                "< julianday(?, '+1 day')"
            } else {
                "<= julianday(?)"
            };
            clauses.push(format!("{MATCH_DATE} {end}"));
            values.push(date.clone().into());
        }
        let player = if player_join {
            "JOIN demo_player_stats ps ON ps.demo_id = d.id"
        } else {
            ""
        };
        Self {
            from_tail: format!("{player} WHERE {}", clauses.join(" AND ")),
            values,
        }
    }
}

fn order_by(query: &DemoListQuery) -> String {
    if matches!(query.kind, DemoListKind::Ids) {
        return format!("ORDER BY {STATUS_RANK}, d.id DESC");
    }
    let direction = if query.sort_asc { "ASC" } else { "DESC" };
    let key = if query.sort_key.is_empty() {
        "library".to_owned()
    } else {
        query.sort_key.trim().to_ascii_lowercase()
    };
    let expression = match key.as_str() {
        "library" => {
            return format!(
                "ORDER BY {STATUS_RANK}, julianday(d.added_at) {direction}, d.id {direction}"
            )
        }
        "match_time" => {
            return format!(
                "ORDER BY CASE WHEN NULLIF(d.match_date, '') IS NULL THEN 1 ELSE 0 END ASC,
            {MATCH_DATE} {direction}, d.id {direction}"
            )
        }
        "added_time" | "date" => "julianday(d.added_at)",
        "size" => "COALESCE(d.file_size, 0)",
        "duration" => "COALESCE(d.duration_mins, 0)",
        "rounds" => "COALESCE(d.total_rounds, -1)",
        "status" => "lower(ifnull(d.status, ''))",
        "map" => "lower(ifnull(d.map_name, ''))",
        "filename" => "lower(ifnull(d.filename, ''))",
        _ => return format!("ORDER BY {STATUS_RANK}, d.id DESC"),
    };
    format!("ORDER BY {expression} {direction}, d.id {direction}")
}

fn date_only(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    let digits = |slice: &[u8]| -> Option<u32> {
        slice.iter().try_fold(0, |number, byte| {
            byte.is_ascii_digit()
                .then(|| number * 10 + u32::from(byte - b'0'))
        })
    };
    let (Some(year), Some(month), Some(day)) = (
        digits(&bytes[..4]),
        digits(&bytes[5..7]),
        digits(&bytes[8..]),
    ) else {
        return false;
    };
    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        2 if leap_year => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 0,
    };
    year > 0 && (1..=days).contains(&day)
}

fn list_rows(
    conn: &Connection,
    query: &DemoListQuery,
    mut filter: DemoFilter,
    order: &str,
) -> DemoResult<(i64, Vec<Object>)> {
    let total: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(DISTINCT d.id) FROM demo_files d {}",
            filter.from_tail
        ),
        params_from_iter(&filter.values),
        |row| row.get(0),
    )?;
    let compact = matches!(query.kind, DemoListKind::Compact);
    let (columns, join) = if compact {
        (SUMMARY_COLUMNS, SUMMARY_JOIN)
    } else {
        (RESULT_COLUMNS, RESULT_JOIN)
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT DISTINCT {DEMO_COLUMNS}, {columns} FROM demo_files d {join} {} {order}
         LIMIT ? OFFSET ?",
        filter.from_tail
    ))?;
    filter
        .values
        .extend([query.page.limit.into(), query.page.offset.into()]);
    let mut rows = stmt.query(params_from_iter(&filter.values))?;
    let mut items = Vec::new();
    while let Some(row) = rows.next()? {
        items.push(if compact {
            compact_row(row)?
        } else {
            full_row(row, false)?
        });
    }
    attach_players(conn, &mut items)?;
    Ok((total, items))
}

fn placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(",")
}

fn compact_row(row: &Row<'_>) -> rusqlite::Result<Object> {
    let mut item = row_fields(row, &["analyzed_targets_json"])?;
    let targets = if let ValueRef::Text(raw) = row.get_ref("analyzed_targets_json")? {
        serde_json::from_slice::<Vec<Value>>(raw)
            .unwrap_or_default()
            .into_iter()
            .filter(Value::is_string)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    item.insert("analyzed_targets".to_owned(), json!(targets));
    item.insert(
        "has_result".to_owned(),
        json!(row.get::<_, i64>("has_result")? != 0),
    );
    Ok(item)
}

fn full_row(row: &Row<'_>, keep_invalid: bool) -> DemoResult<Object> {
    let mut item = row_fields(row, &["result_json"])?;
    let (result, error) = match decode_result(row.get_ref("result_json")?) {
        Ok(result) => (result, Value::Null),
        Err(reason) if keep_invalid => (Value::Null, json!(format!("损坏的解析结果：{reason}"))),
        Err(_) => return Err(DemoReadError::InvalidFullResult),
    };
    item.insert("result".to_owned(), result);
    if keep_invalid {
        item.insert("result_error".to_owned(), error);
    }
    Ok(item)
}

fn decode_result(raw: ValueRef<'_>) -> Result<Value, String> {
    let bytes = match raw {
        ValueRef::Null => return Ok(Value::Null),
        ValueRef::Text(bytes) => bytes,
        ValueRef::Blob(bytes) => bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes),
        _ => return Err("Stored demo result is not JSON text".to_owned()),
    };
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    let text =
        std::str::from_utf8(bytes).map_err(|_| "Stored demo result is not UTF-8".to_owned())?;
    serde_json::from_str(text).map_err(|error| {
        let (offset, message) = json_decode_error(text, &error);
        let mut line = 1;
        let mut column = 1;
        for character in text.chars().take(offset) {
            if character == '\n' {
                line += 1;
                column = 1;
            } else {
                column += 1;
            }
        }
        format!("{message}: line {line} column {column} (char {offset})")
    })
}

fn attach_players(conn: &Connection, items: &mut [Object]) -> rusqlite::Result<()> {
    let ids = items
        .iter()
        .map(|item| item["id"].as_i64().expect("SQLite demo ID"))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut grouped: HashMap<i64, Vec<Value>> = HashMap::new();
    for ids in ids.chunks(500) {
        let query = format!(
            "SELECT {PLAYER_COLUMNS} FROM demo_player_stats WHERE demo_id IN ({})
             ORDER BY demo_id, kills DESC, player_name ASC",
            placeholders(ids.len())
        );
        let mut stmt = conn.prepare(&query)?;
        let mut rows = stmt.query(params_from_iter(ids))?;
        while let Some(row) = rows.next()? {
            let id = row.get("demo_id")?;
            grouped
                .entry(id)
                .or_default()
                .push(Value::Object(row_fields(row, &["demo_id"])?));
        }
    }
    for item in items {
        let players = grouped
            .get(&item["id"].as_i64().unwrap())
            .map(Vec::as_slice)
            .unwrap_or_default();
        item.insert("players".to_owned(), json!(players));
    }
    Ok(())
}

fn select_objects(
    conn: &Connection,
    sql: &str,
    params: impl Params,
) -> rusqlite::Result<Vec<Object>> {
    let mut statement = conn.prepare(sql)?;
    let rows = statement.query_map(params, |row| row_fields(row, &[]))?;
    rows.collect()
}

fn row_fields(row: &Row<'_>, excluded: &[&str]) -> rusqlite::Result<Object> {
    let columns = row.as_ref();
    (0..columns.column_count())
        .filter_map(|index| {
            let key = columns.column_name(index).expect("SQLite column index");
            if excluded.contains(&key) {
                return None;
            }
            Some(row.get_ref(index).and_then(|cell| {
                let value = match cell {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(number) => json!(number),
                    ValueRef::Real(number) => Value::Number(Number::from_f64(number).ok_or(
                        rusqlite::Error::InvalidColumnType(index, key.to_owned(), cell.data_type()),
                    )?),
                    ValueRef::Text(_) => Value::String(row.get(index)?),
                    ValueRef::Blob(_) => {
                        return Err(rusqlite::Error::InvalidColumnType(
                            index,
                            key.to_owned(),
                            cell.data_type(),
                        ))
                    }
                };
                Ok((key.to_owned(), value))
            }))
        })
        .collect()
}
