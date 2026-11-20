use super::*;
use rusqlite::params;
use serde_json::json;

const LEGACY_TABLE: &str = "
    CREATE TABLE recorded_clips (
        id INTEGER PRIMARY KEY AUTOINCREMENT, clip_id TEXT NOT NULL, demo_path TEXT NOT NULL,
        demo_filename TEXT, player_name TEXT, output_path TEXT NOT NULL,
        duration_sec REAL, status TEXT NOT NULL DEFAULT 'ready', created_at TEXT NOT NULL
    );
    CREATE INDEX idx_recorded_clips_created ON recorded_clips(created_at DESC);
";

#[test]
fn opens_an_old_active_wal_without_migrating_or_showing_uncommitted_rows() {
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("cs2-insight.db");
    let writer = Connection::open(&path).unwrap();
    writer
        .execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
        .unwrap();
    writer.execute_batch(LEGACY_TABLE).unwrap();
    let first_path = scratch.path().join("历史片段").join("recorded.mp4");
    writer
        .execute(
            "INSERT INTO recorded_clips(clip_id, demo_path, output_path, player_name, created_at)
         VALUES ('first', 'fixture.dem', ?1, '测试玩家', '2026-11-20T00:00:00Z')",
            [first_path.to_str().unwrap()],
        )
        .unwrap();
    let db_bytes = fs::read(&path).unwrap();
    let wal_path = scratch.path().join("cs2-insight.db-wal");
    let wal_bytes = fs::read(wal_path).unwrap();
    writer
        .execute_batch(
            "BEGIN IMMEDIATE;
         INSERT INTO recorded_clips(clip_id, demo_path, output_path, created_at)
         VALUES ('uncommitted', 'fixture.dem', 'future.mp4', '2000-01-01T00:00:00Z');",
        )
        .unwrap();

    assert_eq!(
        list(&path, 500, 0).unwrap(),
        [json!({
            "id": 1,
            "clip_id": "first",
            "demo_path": "fixture.dem",
            "demo_filename": null,
            "player_name": "测试玩家",
            "output_path": first_path.to_str().unwrap(),
            "duration_sec": null,
            "status": "ready",
            "created_at": "2026-11-20T00:00:00Z",
        })]
    );
    assert_eq!(
        recent_output_dir(&path).unwrap(),
        first_path.parent().unwrap().to_string_lossy()
    );
    assert_eq!(fs::read(&path).unwrap(), db_bytes);
    assert_eq!(
        fs::read(scratch.path().join("cs2-insight.db-wal")).unwrap(),
        wal_bytes
    );
    assert_eq!(
        writer
            .query_row(
                "SELECT count(*) FROM pragma_table_info('recorded_clips')",
                [],
                |row| row.get::<_, i32>(0)
            )
            .unwrap(),
        9
    );
    writer.execute_batch("COMMIT;").unwrap();
    assert_eq!(list(&path, 1, 0).unwrap()[0]["id"], 2);
    assert_eq!(recent_output_dir(&path).unwrap(), ".");
}

#[test]
fn metadata_overrides_the_sql_row_but_keeps_sql_pagination_and_recent_order() {
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("cs2-insight.db");
    let writer = Connection::open(&path).unwrap();
    writer.execute_batch(LEGACY_TABLE).unwrap();
    writer
        .execute_batch("ALTER TABLE recorded_clips ADD COLUMN clip_meta TEXT;")
        .unwrap();
    let updated_path = scratch.path().join("新目录").join("中文片段.mp4");
    let meta = json!({
        "id": 9001,
        "duration_sec": 8,
        "output_path": format!(" {} ", updated_path.display()),
        "recording_perspective": "victim",
        "victim_pov_segments": [{"start_tick": 7, "end_tick": 9}],
        "clip_meta": {"future": "kept"},
    });
    for (date, metadata) in [
        ("2027-01-01", "[]".to_owned()),
        ("2026-01-01", "{not json".to_owned()),
        ("2000-01-01", meta.to_string()),
    ] {
        writer.execute(
            "INSERT INTO recorded_clips (clip_id, demo_path, output_path, duration_sec, created_at, clip_meta)
             VALUES ('clip', 'fixture.dem', 'stored.mp4', 1.25, ?1, ?2)",
            params![date, metadata],
        ).unwrap();
    }
    let rows = list(&path, 2, 0).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["id"], 9001);
    assert_eq!(rows[0]["duration_sec"], 8);
    assert_eq!(rows[0]["victim_pov_segments"], meta["victim_pov_segments"]);
    assert_eq!(rows[0]["clip_meta"], meta["clip_meta"]);
    assert_eq!(rows[1]["id"], 2);
    assert_eq!(rows[1]["duration_sec"], 1.25);
    assert!(rows[1].get("clip_meta").is_none());
    let oldest = list(&path, 2, 2).unwrap();
    assert_eq!(oldest.len(), 1);
    assert_eq!(oldest[0]["id"], 1);
    assert!(oldest[0].get("clip_meta").is_none());
    assert_eq!(
        recent_output_dir(&path).unwrap(),
        updated_path.parent().unwrap().to_string_lossy()
    );

    writer
        .execute(
            "UPDATE recorded_clips SET clip_meta = '{\"output_path\":null}' WHERE id=3",
            [],
        )
        .unwrap();
    assert_eq!(recent_output_dir(&path).unwrap(), "");
}

#[test]
fn missing_data_is_empty_but_invalid_databases_and_incomplete_tables_fail() {
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("cs2-insight.db");
    assert!(list(&path, 1, 0).unwrap().is_empty());
    assert_eq!(recent_output_dir(&path).unwrap(), "");
    assert!(!path.exists());

    let writer = Connection::open(&path).unwrap();
    writer
        .execute("CREATE TABLE training_profiles(id INTEGER PRIMARY KEY)", [])
        .unwrap();
    assert!(list(&path, 1, 0).unwrap().is_empty());
    writer
        .execute("CREATE TABLE recorded_clips(id INTEGER PRIMARY KEY)", [])
        .unwrap();
    assert!(matches!(
        list(&path, 1, 0),
        Err(RecordedClipsError::Sqlite(_))
    ));

    let invalid = scratch.path().join("invalid.db");
    fs::write(&invalid, b"synthetic invalid SQLite database").unwrap();
    assert!(matches!(
        list(&invalid, 1, 0),
        Err(RecordedClipsError::Sqlite(_))
    ));
    assert!(matches!(
        recent_output_dir(scratch.path()),
        Err(RecordedClipsError::File(_))
    ));
}

#[cfg(windows)]
#[test]
fn recent_directory_preserves_python_windows_lexical_paths() {
    for (input, expected) in [
        ("clip.mp4", "."),
        ("C:clip.mp4", "C:"),
        (" C:/Output//./a/../clip.mp4 ", r"C:\Output\a\.."),
        (r"\\server\share\segment.mp4", r"\\server\share\"),
        (r"\\?\C:\clips\segment.mp4", r"\\?\C:\clips"),
        (" \u{1c}clip.mp4\u{1c} ", "."),
    ] {
        assert_eq!(output_parent(input), expected, "{input}");
    }
}
