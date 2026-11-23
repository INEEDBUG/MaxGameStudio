//! Commit analysis and the legacy library indexes in one SQLite transaction.

use super::{open_library, read_library, schema_ready, DemoLibrary, DemoReadError, DemoResult};
use rusqlite::{params, OpenFlags, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};

pub(crate) struct AnalysisSource {
    pub(crate) id: i64,
    pub(crate) path: String,
    pub(crate) filename: String,
    pub(crate) match_date: Option<String>,
}

impl DemoLibrary {
    pub(crate) fn analysis_source(&self, id: i64) -> DemoResult<Option<AnalysisSource>> {
        read_library(&self.path, |connection| {
            let Some(connection) = connection else {
                return Ok(None);
            };
            Ok(connection
                .query_row(
                    "SELECT path, filename, match_date FROM demo_files WHERE id = ?1",
                    [id],
                    |row| {
                        Ok(AnalysisSource {
                            id,
                            path: row.get(0)?,
                            filename: row.get(1)?,
                            match_date: row.get(2)?,
                        })
                    },
                )
                .optional()?)
        })
    }

    pub(crate) fn save_analysis(
        &self,
        source: &AnalysisSource,
        result: &Value,
    ) -> DemoResult<bool> {
        let Some(mut conn) = open_library(&self.path, OpenFlags::SQLITE_OPEN_READ_WRITE)? else {
            return Ok(false);
        };
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !schema_ready(&transaction)? {
            return Ok(false);
        }
        let path = &source.path;
        let valid: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM demo_files WHERE id=?1 AND path=?2)",
            params![source.id, path],
            |row| row.get(0),
        )?;
        if !valid {
            return Ok(false);
        }
        let primary = result["auto_target_player"]
            .as_str()
            .ok_or(DemoReadError::InvalidFullResult)?;
        let players = result["players"]
            .as_object()
            .ok_or(DemoReadError::InvalidFullResult)?;
        let clips = result["clips"]
            .as_array()
            .ok_or(DemoReadError::InvalidFullResult)?;
        let counts = clips
            .iter()
            .filter(|clip| clip["category"] == "highlight")
            .fold((0, 0), |(four, five), clip| {
                let kills = clip["kill_count"].as_i64().unwrap_or_default();
                (four + i64::from(kills == 4), five + i64::from(kills >= 5))
            });
        let now: String = transaction.query_row(
            "SELECT strftime('%Y-%m-%dT%H:%M:%f','now') || '+00:00'",
            [],
            |row| row.get(0),
        )?;
        transaction.execute("DELETE FROM match_results WHERE demo_path=?1", [path])?;
        transaction.execute(
            "INSERT INTO match_results(demo_path,result_json,created_at) VALUES (?1,?2,?3)",
            params![path, result.to_string(), now],
        )?;
        transaction.execute(
            "INSERT INTO demo_result_summaries (
                demo_path,clip_count,primary_target,analyzed_targets_json,four_k_count,
                five_k_count,result_created_at,summary_version
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,2)
             ON CONFLICT(demo_path) DO UPDATE SET clip_count=excluded.clip_count,
                primary_target=excluded.primary_target,analyzed_targets_json=excluded.analyzed_targets_json,
                four_k_count=excluded.four_k_count,five_k_count=excluded.five_k_count,
                result_created_at=excluded.result_created_at,summary_version=excluded.summary_version",
            params![path, clips.len() as i64, primary, json!(players.keys().collect::<Vec<_>>()).to_string(), counts.0, counts.1, now],
        )?;
        replace_timeline(&transaction, path, players, &now)?;
        let meta = &result["match_meta"];
        let text = |key: &str| meta[key].as_str().filter(|value| !value.is_empty());
        transaction.execute(
            "UPDATE demo_files SET status='done',parsed_at=?1,error_msg=NULL,
                map_name=COALESCE(NULLIF(map_name,''),?2),total_rounds=COALESCE(total_rounds,?3),
                team_a_score=COALESCE(team_a_score,?4),team_b_score=COALESCE(team_b_score,?5),
                team_a_name=COALESCE(NULLIF(team_a_name,''),?6),
                team_b_name=COALESCE(NULLIF(team_b_name,''),?7),
                duration_mins=COALESCE(duration_mins,?8),match_date=COALESCE(NULLIF(match_date,''),?9)
             WHERE path=?10",
            params![now,text("map_name"),meta["total_rounds"].as_i64(),meta["team_a_score"].as_i64(),
                meta["team_b_score"].as_i64(),text("team_a_name"),text("team_b_name"),
                meta["duration_mins"].as_i64(),text("match_date"),path],
        )?;
        transaction.commit()?;
        Ok(true)
    }
}

fn replace_timeline(
    conn: &rusqlite::Connection,
    path: &str,
    players: &serde_json::Map<String, Value>,
    now: &str,
) -> DemoResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS demo_timeline_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,demo_path TEXT NOT NULL,target_player TEXT NOT NULL,
            event_id TEXT NOT NULL,round_number INTEGER NOT NULL,tick INTEGER NOT NULL,
            record_type TEXT NOT NULL,tags_json TEXT NOT NULL,event_json TEXT NOT NULL,created_at TEXT NOT NULL,
            UNIQUE(demo_path,target_player,event_id)
         );
         CREATE INDEX IF NOT EXISTS idx_dte_demo ON demo_timeline_events(demo_path);
         CREATE INDEX IF NOT EXISTS idx_dte_demo_player ON demo_timeline_events(demo_path,target_player);
         CREATE INDEX IF NOT EXISTS idx_dte_tick ON demo_timeline_events(demo_path,tick);",
    )?;
    let mut insert = conn.prepare(
        "INSERT INTO demo_timeline_events(
           demo_path,target_player,event_id,round_number,tick,record_type,tags_json,event_json,created_at
         ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
    )?;
    for (player, result) in players {
        let rounds = result["round_timeline"]
            .as_array()
            .ok_or(DemoReadError::InvalidFullResult)?;
        conn.execute(
            "DELETE FROM demo_timeline_events WHERE demo_path=?1 AND target_player=?2",
            params![path, player],
        )?;
        for round in rounds {
            let number = round["round_number"]
                .as_i64()
                .ok_or(DemoReadError::InvalidFullResult)?;
            if let Some(tags) = round["highlight_tags"]
                .as_array()
                .filter(|tags| !tags.is_empty())
            {
                let event = json!({"round_number":number,"highlight_tags":tags});
                insert.execute(params![
                    path,
                    player,
                    format!("hr{number}-highlight"),
                    number,
                    -1i64,
                    "highlight_round",
                    json!(tags).to_string(),
                    event.to_string(),
                    now
                ])?;
            }
            for event in round["events"].as_array().into_iter().flatten() {
                let Some(kind @ ("kill" | "death" | "assist_only")) = event["type"].as_str() else {
                    continue;
                };
                let id = event["id"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .ok_or(DemoReadError::InvalidFullResult)?;
                let tick = event["tick"]
                    .as_i64()
                    .ok_or(DemoReadError::InvalidFullResult)?;
                let mut event = event
                    .as_object()
                    .ok_or(DemoReadError::InvalidFullResult)?
                    .clone();
                event.remove("tags");
                insert.execute(params![
                    path,
                    player,
                    id,
                    number,
                    tick,
                    kind,
                    "[]",
                    Value::Object(event).to_string(),
                    now
                ])?;
            }
        }
    }
    Ok(())
}
