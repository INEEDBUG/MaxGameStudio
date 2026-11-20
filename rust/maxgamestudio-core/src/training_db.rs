//! Training history in the existing shared cs2-insight.db file.

use crate::training_input::{
    InputLabRequest, InputLabResult, SensitivityRecommendation, SensitivityRecommendationRequest,
    CS2_YAW,
};
use rusqlite::{params, Connection, TransactionBehavior};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS sensitivity_sessions (
        id                      INTEGER PRIMARY KEY AUTOINCREMENT,
        created_at              TEXT NOT NULL,
        dpi                     INTEGER NOT NULL,
        current_sensitivity     REAL NOT NULL,
        recommended_sensitivity REAL NOT NULL,
        game_width              INTEGER NOT NULL,
        game_height             INTEGER NOT NULL,
        display_aspect          TEXT NOT NULL,
        scaling_mode            TEXT NOT NULL,
        confidence              REAL NOT NULL,
        request_json            TEXT NOT NULL,
        result_json             TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_sensitivity_sessions_created
        ON sensitivity_sessions(created_at DESC, id DESC);
    CREATE TABLE IF NOT EXISTS input_lab_sessions (
        id              INTEGER PRIMARY KEY AUTOINCREMENT,
        created_at      TEXT NOT NULL,
        keyboard_name   TEXT NOT NULL,
        mode            TEXT NOT NULL,
        actuation_mm    REAL NOT NULL,
        rt_press_mm     REAL NOT NULL,
        rt_release_mm   REAL NOT NULL,
        stability_score REAL NOT NULL,
        request_json    TEXT NOT NULL,
        result_json     TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_input_lab_sessions_created
        ON input_lab_sessions(created_at DESC, id DESC);
";

pub type TrainingDbError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, Clone)]
pub struct TrainingDb {
    path: PathBuf,
}

impl TrainingDb {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_owned(),
        }
    }

    fn connect(&self) -> rusqlite::Result<Connection> {
        let connection = Connection::open(&self.path)?;
        // Match the Python connection timeout and honor the database's journal mode.
        connection.busy_timeout(Duration::from_secs(5))?;
        Ok(connection)
    }

    pub fn init_tables(&self) -> Result<(), TrainingDbError> {
        if let Some(parent) = self
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(SCHEMA)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn save_sensitivity_session(
        &self,
        request: &SensitivityRecommendationRequest,
        result: &SensitivityRecommendation,
    ) -> Result<Value, TrainingDbError> {
        let now = SystemTime::now();
        let request_json = serde_json::to_string(request)?;
        let result_json = serde_json::to_string(result)?;
        let payload = serde_json::from_str(&result_json)?;
        let connection = self.connect()?;
        let created_at = utc_iso(&connection, now)?;
        connection.execute(
            "INSERT INTO sensitivity_sessions(
                 created_at, dpi, current_sensitivity, recommended_sensitivity,
                 game_width, game_height, display_aspect, scaling_mode,
                 confidence, request_json, result_json
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                created_at,
                request.dpi,
                request.current_sensitivity,
                result.recommended_sensitivity,
                request.game_width,
                request.game_height,
                enum_text(request.display_aspect)?,
                enum_text(request.scaling_mode)?,
                result.confidence,
                request_json,
                result_json,
            ],
        )?;
        Ok(insert_metadata(
            connection.last_insert_rowid(),
            created_at,
            payload,
        ))
    }

    pub fn save_input_session(
        &self,
        request: &InputLabRequest,
        result: &InputLabResult,
    ) -> Result<Value, TrainingDbError> {
        let now = SystemTime::now();
        let request_json = serde_json::to_string(request)?;
        let result_json = serde_json::to_string(result)?;
        let payload = serde_json::from_str(&result_json)?;
        let connection = self.connect()?;
        let created_at = utc_iso(&connection, now)?;
        connection.execute(
            "INSERT INTO input_lab_sessions(
                 created_at, keyboard_name, mode, actuation_mm, rt_press_mm,
                 rt_release_mm, stability_score, request_json, result_json
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                created_at,
                request.keyboard_name,
                enum_text(request.mode)?,
                request.actuation_mm,
                request.rapid_trigger_press_mm,
                request.rapid_trigger_release_mm,
                result.stability_score,
                request_json,
                result_json,
            ],
        )?;
        Ok(insert_metadata(
            connection.last_insert_rowid(),
            created_at,
            payload,
        ))
    }

    pub fn list_sensitivity_sessions(&self, limit: i64) -> Result<Vec<Value>, TrainingDbError> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, created_at, dpi, current_sensitivity,
                    recommended_sensitivity, game_width, game_height,
                    display_aspect, scaling_mode, confidence, result_json
             FROM sensitivity_sessions
             ORDER BY created_at DESC, id DESC
             LIMIT ?",
        )?;
        let mut cursor = statement.query([limit.clamp(1, 100)])?;
        let mut sessions = Vec::new();
        while let Some(row) = cursor.next()? {
            let payload: Map<String, Value> =
                serde_json::from_str(&row.get::<_, String>("result_json")?)?;
            sessions.push(json!({
                "id": row.get::<_, i64>("id")?,
                "created_at": row.get::<_, String>("created_at")?,
                "dpi": row.get::<_, i64>("dpi")?,
                "current_sensitivity": row.get::<_, f64>("current_sensitivity")?,
                "recommended_sensitivity": row.get::<_, f64>("recommended_sensitivity")?,
                "game_width": row.get::<_, i64>("game_width")?,
                "game_height": row.get::<_, i64>("game_height")?,
                "display_aspect": row.get::<_, String>("display_aspect")?,
                "scaling_mode": row.get::<_, String>("scaling_mode")?,
                "confidence": row.get::<_, f64>("confidence")?,
                "edpi": payload.get("edpi"),
                "cm_per_360": payload.get("cm_per_360"),
                "m_yaw": payload.get("m_yaw").cloned().unwrap_or_else(|| json!(CS2_YAW)),
            }));
        }
        Ok(sessions)
    }

    pub fn list_input_sessions(&self, limit: i64) -> Result<Vec<Value>, TrainingDbError> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, created_at, keyboard_name, mode, actuation_mm,
                    rt_press_mm, rt_release_mm, stability_score, result_json
             FROM input_lab_sessions
             ORDER BY created_at DESC, id DESC
             LIMIT ?",
        )?;
        let mut cursor = statement.query([limit.clamp(1, 100)])?;
        let mut sessions = Vec::new();
        while let Some(row) = cursor.next()? {
            let mut record = json!({
                "id": row.get::<_, i64>("id")?,
                "created_at": row.get::<_, String>("created_at")?,
                "keyboard_name": row.get::<_, String>("keyboard_name")?,
                "mode": row.get::<_, String>("mode")?,
                "actuation_mm": row.get::<_, f64>("actuation_mm")?,
                "rapid_trigger_press_mm": row.get::<_, f64>("rt_press_mm")?,
                "rapid_trigger_release_mm": row.get::<_, f64>("rt_release_mm")?,
                "stability_score": row.get::<_, f64>("stability_score")?,
            });
            let payload: Map<String, Value> =
                serde_json::from_str(&row.get::<_, String>("result_json")?)?;
            record
                .as_object_mut()
                .expect("record is an object")
                .extend(payload);
            sessions.push(record);
        }
        Ok(sessions)
    }
}

fn enum_text(value: impl Serialize) -> Result<String, TrainingDbError> {
    match serde_json::to_value(value)? {
        Value::String(label) => Ok(label),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "expected a training setting label",
        )
        .into()),
    }
}

fn insert_metadata(id: i64, created_at: String, mut payload: Map<String, Value>) -> Value {
    payload.insert("id".to_owned(), json!(id));
    payload.insert("created_at".to_owned(), json!(created_at));
    Value::Object(payload)
}

fn utc_iso(connection: &Connection, now: SystemTime) -> Result<String, TrainingDbError> {
    let elapsed = now.duration_since(UNIX_EPOCH)?;
    let seconds = i64::try_from(elapsed.as_secs())?;
    let wall_time: String = connection.query_row(
        "SELECT strftime('%Y-%m-%dT%H:%M:%S', ?1, 'unixepoch')",
        [seconds],
        |row| row.get(0),
    )?;
    let micros = elapsed.subsec_micros();
    if micros == 0 {
        Ok(format!("{wall_time}+00:00"))
    } else {
        Ok(format!("{wall_time}.{micros:06}+00:00"))
    }
}
