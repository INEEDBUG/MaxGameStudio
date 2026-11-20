use maxgamestudio_core::{
    training_db::TrainingDb,
    training_input::{
        analyze_input_session, recommend_sensitivity, InputLabRequest,
        SensitivityRecommendationRequest,
    },
};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::{path::Path, process::Command, sync::Barrier, thread};

fn requests() -> (SensitivityRecommendationRequest, InputLabRequest) {
    let cases: Value = serde_json::from_str(include_str!("fixtures/training_input.json")).unwrap();
    let sensitivity = serde_json::from_value(cases["sensitivity"][0]["input"].clone()).unwrap();
    let mut keyboard = cases["keyboard"][0]["input"].clone();
    keyboard["keyboard_name"] = json!("台式键盘");
    (sensitivity, serde_json::from_value(keyboard).unwrap())
}

#[test]
fn saves_and_lists_both_training_session_shapes() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("nested").join("cs2-insight.db");
    let store = TrainingDb::new(&path);
    store.init_tables().unwrap();
    let (sensitivity, keyboard) = requests();
    let recommendation = recommend_sensitivity(&sensitivity).unwrap();
    let analysis = analyze_input_session(&keyboard).unwrap();
    let saved_sensitivity = store
        .save_sensitivity_session(&sensitivity, &recommendation)
        .unwrap();
    let saved_input = store.save_input_session(&keyboard, &analysis).unwrap();
    store.init_tables().unwrap();

    for saved in [&saved_sensitivity, &saved_input] {
        assert_eq!(saved["id"], 1);
        let stamp = saved["created_at"].as_str().unwrap();
        assert_eq!(&stamp[10..11], "T");
        assert!(stamp.ends_with("+00:00"));
    }
    assert_eq!(saved_sensitivity["diagnosis"], recommendation.diagnosis);
    assert_eq!(saved_input["diagnosis"], analysis.diagnosis);
    let saved_row = store.list_sensitivity_sessions(20).unwrap();
    assert_eq!(saved_row.len(), 1);
    assert_eq!(saved_row[0]["created_at"], saved_sensitivity["created_at"]);
    assert_eq!(saved_row[0]["display_aspect"], "16:9");
    assert_eq!(saved_row[0]["scaling_mode"], "stretched");
    assert_eq!(saved_row[0]["m_yaw"], 0.022);
    assert!(saved_row[0].get("diagnosis").is_none());
    let saved_row = store.list_input_sessions(20).unwrap();
    assert_eq!(saved_row.len(), 1);
    assert_eq!(saved_row[0]["created_at"], saved_input["created_at"]);
    assert_eq!(saved_row[0]["keyboard_name"], "台式键盘");
    assert_eq!(saved_row[0]["rapid_trigger_press_mm"], 0.2);
    assert_eq!(saved_row[0]["diagnosis"], analysis.diagnosis);
}

#[test]
fn preserves_existing_rows_history_order_and_limit_clamps() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("cs2-insight.db");
    let store = TrainingDb::new(&path);
    store.init_tables().unwrap();
    let mut db = Connection::open(&path).unwrap();
    let transaction = db.transaction().unwrap();
    transaction
        .execute_batch(
            "CREATE TABLE profile_canary(note TEXT); INSERT INTO profile_canary VALUES ('保留');",
        )
        .unwrap();
    for _ in 0..102 {
        transaction.execute(
            "INSERT INTO sensitivity_sessions VALUES (NULL, '2024-02-03T04:05:06+00:00', 800, 1.0, 0.9, 1024, 1080, '4:3', 'native', 0.5, '{}', '{}')",
            [],
        ).unwrap();
        transaction.execute(
            "INSERT INTO input_lab_sessions VALUES (NULL, '2024-02-03T04:05:06+00:00', '旧键盘', 'gameplay', 1.0, 0.2, 0.3, 71.0, '{}', '{\"legacy_note\":\"保留\", \"stability_score\": 70.0}')",
            [],
        ).unwrap();
    }
    transaction.commit().unwrap();
    store.init_tables().unwrap();
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM profile_canary WHERE note='保留'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);

    let history = store.list_sensitivity_sessions(104).unwrap();
    assert_eq!(history.len(), 100);
    assert_eq!(history[0]["id"], 102);
    assert_eq!(history[99]["id"], 3);
    assert_eq!(history[0]["m_yaw"], 0.022);
    assert_eq!(history[0]["edpi"], Value::Null);
    assert_eq!(history[0]["cm_per_360"], Value::Null);
    assert_eq!(store.list_sensitivity_sessions(0).unwrap().len(), 1);

    let history = store.list_input_sessions(104).unwrap();
    assert_eq!(history.len(), 100);
    assert_eq!(history[0]["id"], 102);
    assert_eq!(history[99]["id"], 3);
    assert_eq!(history[0]["legacy_note"], "保留");
    assert_eq!(history[0]["stability_score"], 70.0);
    assert_eq!(store.list_input_sessions(-5).unwrap().len(), 1);
}

#[test]
fn concurrent_writes_keep_wal_reader_snapshot_and_unique_ids() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("cs2-insight.db");
    let store = TrainingDb::new(&path);
    store.init_tables().unwrap();
    let mut reader = Connection::open(&path).unwrap();
    let mode: String = reader
        .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    let (sensitivity, _) = requests();
    let recommendation = recommend_sensitivity(&sensitivity).unwrap();
    store
        .save_sensitivity_session(&sensitivity, &recommendation)
        .unwrap();
    let snapshot = reader.transaction().unwrap();
    let snapshot_count = |connection: &Connection| -> i64 {
        connection
            .query_row("SELECT count(*) FROM sensitivity_sessions", [], |row| {
                row.get(0)
            })
            .unwrap()
    };
    assert_eq!(snapshot_count(&snapshot), 1);

    let start = Barrier::new(4);
    let mut ids = thread::scope(|scope| {
        let pending: Vec<_> = (0..4)
            .map(|_| {
                scope.spawn(|| {
                    start.wait();
                    store
                        .save_sensitivity_session(&sensitivity, &recommendation)
                        .unwrap()["id"]
                        .as_i64()
                        .unwrap()
                })
            })
            .collect();
        pending
            .into_iter()
            .map(|task| task.join().unwrap())
            .collect::<Vec<_>>()
    });
    ids.sort_unstable();
    assert_eq!(ids, [2, 3, 4, 5]);
    assert_eq!(snapshot_count(&snapshot), 1);
    snapshot.commit().unwrap();
    assert_eq!(snapshot_count(&reader), 5);
    let mode: String = reader
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
}

fn python(action: &str, path: &Path) -> Value {
    let executable = std::env::var_os("MGS_TRAINING_DB_PYTHON")
        .expect("set MGS_TRAINING_DB_PYTHON to the original Python 3.12 interpreter");
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = Command::new(executable)
        .arg("-B")
        .arg(directory.join("tests/fixtures/training_db_python.py"))
        .arg(action)
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
#[ignore = "requires the app's original Python dependencies and MGS_TRAINING_DB_PYTHON"]
fn python_and_rust_share_existing_profile_schema_and_records() {
    let fixture = tempfile::tempdir().unwrap();
    let python_path = fixture.path().join("cs2-insight.db");
    let baseline = python("seed", &python_path);
    let store = TrainingDb::new(&python_path);
    store.init_tables().unwrap();
    assert_eq!(
        store.list_sensitivity_sessions(20).unwrap(),
        baseline["sensitivity"].as_array().unwrap().clone()
    );
    assert_eq!(
        store.list_input_sessions(20).unwrap(),
        baseline["input"].as_array().unwrap().clone()
    );

    let rust_path = fixture.path().join("rust-initialized.db");
    TrainingDb::new(&rust_path).init_tables().unwrap();
    assert_eq!(python("layout", &rust_path), baseline["layout"]);

    let (sensitivity, keyboard) = requests();
    let recommendation = recommend_sensitivity(&sensitivity).unwrap();
    let analysis = analyze_input_session(&keyboard).unwrap();
    let new_sensitivity = store
        .save_sensitivity_session(&sensitivity, &recommendation)
        .unwrap();
    let new_input = store.save_input_session(&keyboard, &analysis).unwrap();
    assert_eq!(new_sensitivity["id"], 2);
    assert_eq!(new_input["id"], 2);
    let after = python("read", &python_path);
    assert_eq!(after["canary"], "keep profile");
    assert_eq!(after["journal"], "wal");
    assert_eq!(after["layout"], baseline["layout"]);
    assert_eq!(after["sensitivity"][0]["id"], new_sensitivity["id"]);
    assert_eq!(
        after["sensitivity"][0]["created_at"],
        new_sensitivity["created_at"]
    );
    assert_eq!(after["input"][0]["id"], new_input["id"]);
    assert_eq!(after["input"][0]["created_at"], new_input["created_at"]);
    assert_eq!(after["sensitivity"][1], baseline["sensitivity"][0]);
    assert_eq!(after["input"][1], baseline["input"][0]);
    assert_eq!(after["sensitivity_request"]["display_aspect"], "16:9");
    assert_eq!(after["sensitivity_request"]["m_yaw"], 0.022);
    assert_eq!(after["sensitivity_request"]["trials"][0]["clicks"], 0);
    assert_eq!(after["input_request"]["keyboard_name"], "台式键盘");
    assert_eq!(after["input_result"]["diagnosis"], analysis.diagnosis);
    assert_eq!(
        after["sensitivity_result"]["diagnosis"],
        recommendation.diagnosis
    );
}
