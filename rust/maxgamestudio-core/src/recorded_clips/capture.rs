//! Register a confirmed local OBS result in the existing clip library.

use super::{Connection, OpenFlags, RecordedClipsError};
use crate::profile_platform::path_text;
use rusqlite::params;
use serde_json::Map;
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::SystemTime,
};

pub(crate) struct VerifiedOutput {
    path: PathBuf,
    duration: f64,
}

impl VerifiedOutput {
    /// The path is the actual StopRecord result. The recording directory and
    /// start time come from the same OBS session before capture.
    pub(crate) fn validate(
        path: &Path,
        directory: &Path,
        started: SystemTime,
        duration: f64,
    ) -> io::Result<Self> {
        if !duration.is_finite() || duration <= 0.0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid OBS recording result",
            ));
        }
        let path = local_output(path, directory, started)?;
        Ok(Self { path, duration })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn duration(&self) -> f64 {
        self.duration
    }
}

pub(crate) fn local_output(
    path: &Path,
    directory: &Path,
    started: SystemTime,
) -> io::Result<PathBuf> {
    if !path.is_absolute() || !directory.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid OBS recording path",
        ));
    }
    let root = directory.canonicalize()?;
    let path = path.canonicalize()?;
    if !path.starts_with(root) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "OBS recording output is outside its configured directory",
        ));
    }
    let file = fs::metadata(&path)?;
    if !file.is_file() || file.len() == 0 || file.modified()? < started {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "OBS did not produce a fresh recording file",
        ));
    }
    Ok(path)
}

pub(crate) struct CapturedClip<'a> {
    pub(crate) clip_id: &'a str,
    pub(crate) demo_path: &'a str,
    pub(crate) filename: Option<&'a str>,
    pub(crate) player: Option<&'a str>,
    pub(crate) metadata: &'a Map<String, serde_json::Value>,
}

pub(crate) fn insert_ready(
    path: &Path,
    clip: CapturedClip<'_>,
    output: &VerifiedOutput,
) -> Result<i64, RecordedClipsError> {
    let directory = path.parent().ok_or_else(|| {
        RecordedClipsError::File(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Missing clip library parent",
        ))
    })?;
    fs::create_dir_all(directory).map_err(RecordedClipsError::File)?;
    let mut db = super::open_existing(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?
        .map_or_else(|| Connection::open(path), Ok)?;
    let transaction = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS recorded_clips(
          id INTEGER PRIMARY KEY AUTOINCREMENT, clip_id TEXT NOT NULL,
          demo_path TEXT NOT NULL, demo_filename TEXT, player_name TEXT, output_path TEXT NOT NULL,
          duration_sec REAL, status TEXT NOT NULL DEFAULT 'ready', created_at TEXT NOT NULL,
          clip_meta TEXT
         );
         CREATE INDEX IF NOT EXISTS idx_recorded_clips_created ON recorded_clips(created_at DESC);",
    )?;
    let has_meta: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('recorded_clips') WHERE lower(name)='clip_meta')",
        [], |row| row.get(0),
    )?;
    if !has_meta {
        transaction.execute("ALTER TABLE recorded_clips ADD COLUMN clip_meta TEXT", [])?;
    }
    let metadata = (!clip.metadata.is_empty())
        .then(|| serde_json::Value::Object(clip.metadata.clone()).to_string());
    transaction.execute(
        "INSERT INTO recorded_clips(clip_id,demo_path,demo_filename,player_name,output_path,
         duration_sec,status,created_at,clip_meta)
         VALUES(?1,?2,?3,?4,?5,?6,'ready',strftime('%Y-%m-%dT%H:%M:%f','now') || '+00:00',?7)",
        params![
            clip.clip_id,
            clip.demo_path,
            clip.filename,
            clip.player,
            path_text(output.path()),
            output.duration(),
            metadata
        ],
    )?;
    let id = transaction.last_insert_rowid();
    transaction.commit()?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_capture_inserts_in_the_old_library_without_replacing_records() {
        let scratch = tempfile::tempdir().unwrap();
        let db_path = scratch.path().join("cs2-insight.db");
        let db = Connection::open(&db_path).unwrap();
        db.execute_batch(
            "CREATE TABLE recorded_clips(
               id INTEGER PRIMARY KEY AUTOINCREMENT, clip_id TEXT NOT NULL,
               demo_path TEXT NOT NULL, demo_filename TEXT, player_name TEXT,
               output_path TEXT NOT NULL, duration_sec REAL,
               status TEXT NOT NULL DEFAULT 'ready', created_at TEXT NOT NULL
             );
             INSERT INTO recorded_clips(clip_id,demo_path,output_path,duration_sec,created_at)
             VALUES('old','old.dem','old.mp4',4.25,'2026-01-01');",
        )
        .unwrap();
        drop(db);

        let output_dir = scratch.path().join("rec");
        fs::create_dir(&output_dir).unwrap();
        let recording = output_dir.join("新录制.mkv");
        let start = std::time::UNIX_EPOCH;
        fs::write(&recording, b"fixture output").unwrap();
        let outside = scratch.path().join("outside.mkv");
        fs::write(&outside, b"not this output").unwrap();
        assert!(VerifiedOutput::validate(&outside, &output_dir, start, 5.5).is_err());
        assert!(VerifiedOutput::validate(&recording, &output_dir, start, f64::NAN).is_err());
        let result = VerifiedOutput::validate(&recording, &output_dir, start, 5.5).unwrap();
        let metadata = json!({"recording_request_type":"timeline_kill","source_rounds":[2]});
        let id = insert_ready(
            &db_path,
            CapturedClip {
                clip_id: "new",
                demo_path: "source.dem",
                filename: Some("source.dem"),
                player: Some("fixture player"),
                metadata: metadata.as_object().unwrap(),
            },
            &result,
        )
        .unwrap();
        let old = super::super::get(&db_path, 1).unwrap().unwrap();
        let new = super::super::get(&db_path, id).unwrap().unwrap();
        assert_eq!(old["duration_sec"], 4.25);
        assert_eq!(new["output_path"], path_text(result.path()));
        assert_eq!(new["duration_sec"], 5.5);
        assert_eq!(new["status"], "ready");
        assert_eq!(new["source_rounds"], json!([2]));
        assert_eq!(fs::read(recording).unwrap(), b"fixture output");
    }
}
