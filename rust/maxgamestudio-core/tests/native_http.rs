#![cfg(feature = "native-http")]

#[path = "fixtures/replay_fixture.rs"]
mod replay_fixture;

use replay_fixture::ReplayFixture;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Deserialize)]
struct Fixture {
    version: String,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Option<String>,
    status: u16,
    response_headers: BTreeMap<String, String>,
    response_json: Option<Value>,
    response_text: Option<String>,
}

struct RunningServer {
    child: Child,
    addr: SocketAddr,
}

impl RunningServer {
    fn start(version: &str) -> Self {
        Self::launch(version, None)
    }

    fn launch(version: &str, profile: Option<(&Path, &Path)>) -> Self {
        Self::launch_with_config(version, profile, None)
    }

    fn launch_with_config(
        version: &str,
        profile: Option<(&Path, &Path)>,
        config_path: Option<&Path>,
    ) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_maxgamestudio-native-http"));
        command
            .args(["--port", "0", "--app-version", version])
            .env("CS2_INSIGHT_SESSION_TOKEN", "synthetic-token")
            .env_remove("CS2_INSIGHT_DATA_DIR")
            .env_remove("CS2_INSIGHT_CONFIG")
            .env_remove("CS2_INSIGHT_BUNDLE_DATA_DIR")
            .env_remove("CS2_INSIGHT_TEMP_DIR")
            .env_remove("CS2_INSIGHT_LIBRARY_SSE_DEBOUNCE_SEC");
        if let Some((data, bundle)) = profile {
            command
                .env("CS2_INSIGHT_DATA_DIR", data)
                .env(
                    "CS2_INSIGHT_CONFIG",
                    config_path
                        .map(Path::to_owned)
                        .unwrap_or_else(|| data.join("custom-config.json")),
                )
                .env("CS2_INSIGHT_BUNDLE_DATA_DIR", bundle)
                .env("CS2_INSIGHT_TEMP_DIR", data.join("fixture-temp"))
                .env("CS2_INSIGHT_INSTANCE_ID", " fixture-process ");
        }
        let child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("launch native HTTP");
        let mut running = Self {
            child,
            addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        };
        let stdout = running.child.stdout.take().unwrap();
        let (send, receive) = mpsc::channel();
        thread::spawn(move || {
            let mut line = String::new();
            let read = BufReader::new(stdout).read_line(&mut line);
            let _ = send.send(read.map(|_| line));
        });
        let line = receive
            .recv_timeout(RESPONSE_TIMEOUT)
            .expect("native startup deadline")
            .unwrap();
        let ready: Value = serde_json::from_str(&line).expect("native startup JSON");
        let url = ready["backendUrl"].as_str().expect("announced backendUrl");
        let addr: SocketAddr = url.strip_prefix("http://").unwrap().parse().unwrap();
        assert_eq!(addr.ip(), Ipv4Addr::LOCALHOST);
        assert_ne!(addr.port(), 0);
        running.addr = addr;
        running
    }

    fn stop(mut self) {
        // EOF is the owned sidecar's shutdown signal.
        drop(self.child.stdin.take());
        let deadline = Instant::now() + RESPONSE_TIMEOUT;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success());
                return;
            }
            assert!(Instant::now() < deadline, "native shutdown deadline");
            thread::sleep(Duration::from_millis(10));
        }
    }
}

fn profile_request(
    server: &RunningServer,
    method: &str,
    path: &str,
    input: &Value,
    auth: bool,
) -> (u16, Value) {
    let response = profile_response(server, method, path, input, auth);
    (
        response.status,
        serde_json::from_str(&response.body).unwrap(),
    )
}

fn profile_response(
    server: &RunningServer,
    method: &str,
    path: &str,
    input: &Value,
    auth: bool,
) -> Response {
    let body = if input.is_null() {
        String::new()
    } else {
        input.to_string()
    };
    let auth = if auth {
        "X-CS2-Insight-Token: synthetic-token\r\n"
    } else {
        ""
    };
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{auth}\r\n{body}",
        server.addr, body.len()
    );
    send_request(server.addr, &request)
}

fn recorded_profile(mut value: Value) -> Value {
    if let Some(paths) = value
        .get_mut("demo_watch_paths")
        .and_then(Value::as_array_mut)
    {
        for path in paths {
            let parts = path.as_str().unwrap().split(['/', '\\']);
            *path = json!(parts.collect::<std::path::PathBuf>());
        }
    }
    value
}

#[test]
fn config_routes_replay_python_and_keep_unrecognized_profile_data() {
    let scratch = tempfile::tempdir().unwrap();
    let profile = scratch.path().join("profile");
    let bundle = scratch.path().join("bundle");
    fs::create_dir(&profile).unwrap();
    fs::create_dir(&bundle).unwrap();
    let config_path = profile.join("custom-config.json");
    let ini_path = bundle.join("basic.ini");
    let ini = b"[General]\r\nName=fixture\r\n";
    fs::write(&ini_path, ini).unwrap();
    let fixture: Value = serde_json::from_str(include_str!("fixtures/config_http.json")).unwrap();
    fs::write(&config_path, format!("\u{feff}{}\n", fixture["old"])).unwrap();
    let server = RunningServer::launch("3.2.0-fixture", Some((&profile, &bundle)));
    assert_eq!(
        profile_request(&server, "GET", "/api/config", &Value::Null, false).0,
        401
    );
    assert_eq!(
        profile_request(
            &server,
            "PUT",
            "/api/config",
            &json!({"locale":"en"}),
            false
        )
        .0,
        401
    );
    assert_eq!(
        profile_request(&server, "GET", "/api/health", &Value::Null, false),
        (200, json!({"status":"ok","version":"3.2.0-fixture"}))
    );
    assert_eq!(
        profile_request(
            &server,
            "GET",
            "/api/config?_session=synthetic-token",
            &Value::Null,
            false
        )
        .0,
        200
    );
    for case in fixture["cases"].as_array().unwrap() {
        let method = case["method"].as_str().unwrap();
        let path = case["path"].as_str().unwrap();
        let (status, response) = profile_request(&server, method, path, &case["body"], true);
        assert_eq!(
            u64::from(status),
            case["status"].as_u64().unwrap(),
            "{method} {path}"
        );
        assert_eq!(
            response,
            recorded_profile(case["response"].clone()),
            "{method} {path}"
        );
    }
    let bytes = fs::read(&config_path).unwrap();
    let mut saved: Value = serde_json::from_slice(&bytes).unwrap();
    for (observed, original) in [
        (
            saved
                .as_object_mut()
                .unwrap()
                .remove("future_setting")
                .unwrap(),
            &fixture["old"]["future_setting"],
        ),
        (
            saved["obs"]
                .as_object_mut()
                .unwrap()
                .remove("unrecognized")
                .unwrap(),
            &fixture["old"]["obs"]["unrecognized"],
        ),
        (
            saved["llm"]
                .as_object_mut()
                .unwrap()
                .remove("unrecognized")
                .unwrap(),
            &fixture["old"]["llm"]["unrecognized"],
        ),
    ] {
        assert_eq!(&observed, original);
    }
    assert_eq!(saved, recorded_profile(fixture["saved"].clone()));
    assert_eq!(
        profile_request(&server, "PUT", "/api/config", &json!({}), true).0,
        200
    );
    assert_eq!(
        profile_request(&server, "GET", "/api/config", &Value::Null, true).0,
        200
    );
    assert_eq!(fs::read(&config_path).unwrap(), bytes);
    assert_eq!(fs::read(ini_path).unwrap(), ini);
    assert!(!profile.join("cs2-insight.config.json").exists());
    let barrier = std::sync::Barrier::new(2);
    thread::scope(|scope| {
        for input in [
            json!({"close_action":"tray"}),
            json!({"kill_fx_enabled":true}),
        ] {
            let server = &server;
            let barrier = &barrier;
            scope.spawn(move || {
                barrier.wait();
                assert_eq!(
                    profile_request(server, "PUT", "/api/config", &input, true),
                    (200, json!({"status":"ok"}))
                );
            });
        }
    });
    let persisted: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert_eq!(persisted["close_action"], "tray");
    assert_eq!(persisted["close_to_tray"], true);
    assert_eq!(persisted["kill_fx_enabled"], true);
    assert_eq!(
        persisted["future_setting"],
        fixture["old"]["future_setting"]
    );
    server.stop();
}

#[test]
fn config_startup_uses_bundle_data_and_reports_owned_identity_and_recovery() {
    let scratch = tempfile::tempdir().unwrap();
    let profile = scratch.path().join("profile");
    let bundle = scratch.path().join("bundle");
    let backup = profile.join(".cs2_config_backup");
    fs::create_dir_all(&backup).unwrap();
    fs::create_dir(&bundle).unwrap();
    let example = bundle.join("cs2-insight.config.example.json");
    let mut seed: Value = serde_json::from_str(include_str!(
        "../../../data/cs2-insight.config.example.json"
    ))
    .unwrap();
    seed["cs2_path"] = json!("fixture-missing-cs2.exe");
    let seed = seed.to_string();
    fs::write(&example, &seed).unwrap();
    fs::write(
        profile.join("recovery-required.json"),
        r#"{"reason":"fixture"}"#,
    )
    .unwrap();
    fs::write(
        backup.join("recording_state.json"),
        r#"{"status":"recording","started_at_iso":"fixture-time"}"#,
    )
    .unwrap();
    fs::write(backup.join("manifest.json"), r#"{"entries":[{"account_id":"456"},{"original":"D:\\Steam\\userdata\\123\\730\\local\\cfg\\cs2_user_keys.vcfg"},{"account_id":"456"}]}"#).unwrap();
    let server = RunningServer::launch("3.2.0-fixture", Some((&profile, &bundle)));
    let (status, config) = profile_request(&server, "GET", "/api/config", &Value::Null, true);
    let mut first_load = serde_json::from_str::<Value>(include_str!("fixtures/config_http.json"))
        .unwrap()["first_load"]
        .clone();
    first_load["cs2_path"] = json!("fixture-missing-cs2.exe");
    assert_eq!((status, config), (200, first_load));
    assert_eq!(fs::read_to_string(&example).unwrap(), seed);
    assert!(profile.join("custom-config.json").is_file());

    assert_eq!(
        profile_request(
            &server,
            "GET",
            "/api/app/runtime-state",
            &Value::Null,
            false
        )
        .0,
        401
    );
    let (status, runtime) =
        profile_request(&server, "GET", "/api/app/runtime-state", &Value::Null, true);
    assert_eq!(status, 200);
    assert_eq!(runtime["pid"], server.child.id());
    assert_eq!(runtime["instance_id"], "fixture-process");
    assert_eq!(runtime["version"], "3.2.0-fixture");
    assert_eq!(runtime["data_dir"], profile.to_string_lossy().as_ref());
    assert_eq!(runtime["recovery_required"], true);
    assert_eq!(
        runtime["runtime_session"],
        json!({"busy": false, "owner": null})
    );
    assert_eq!(runtime["demoparser_runtime"]["ready"], false);
    assert!(!runtime.to_string().contains("synthetic-token"));

    let (status, backup_status) = profile_request(
        &server,
        "GET",
        "/api/config-backup/status",
        &Value::Null,
        true,
    );
    assert_eq!(status, 200);
    assert_eq!(backup_status["status"], "recording");
    assert_eq!(backup_status["restore_required"], true);
    assert_eq!(
        backup_status["backup_dir"],
        backup.to_string_lossy().as_ref()
    );
    assert_eq!(backup_status["created_at_iso"], "fixture-time");
    assert_eq!(backup_status["accounts"], json!(["123", "456"]));
    assert!(backup_status["cs2_running"].is_boolean());
    let (_, dir_info) = profile_request(
        &server,
        "GET",
        "/api/config/data-dir-info",
        &Value::Null,
        true,
    );
    assert_eq!(dir_info["path"], profile.to_string_lossy().as_ref());
    assert_eq!(
        dir_info["logs_path"],
        profile.join("logs").to_string_lossy().as_ref()
    );
    assert_eq!(dir_info["exists"], true);
    assert!(dir_info["size_bytes"].as_u64().unwrap() > 100);
    let (_, no_ffmpeg) = profile_request(
        &server,
        "GET",
        "/api/config/ffmpeg-check",
        &Value::Null,
        true,
    );
    assert_eq!(
        no_ffmpeg,
        json!({"ok": false, "reason": "not_configured", "ffmpeg_path": ""})
    );
    let executable = bundle.join("fixture-ffmpeg.exe");
    fs::write(&executable, []).unwrap();
    profile_request(
        &server,
        "PUT",
        "/api/config",
        &json!({"ffmpeg_path": executable}),
        true,
    );
    let (_, found) = profile_request(
        &server,
        "GET",
        "/api/config/ffmpeg-check",
        &Value::Null,
        true,
    );
    assert_eq!(found, json!({"ok": true, "ffmpeg_path": executable}));
    server.stop();
    assert!(profile.join("recovery-required.json").is_file());
}

#[test]
fn demo_library_matches_release_routes_from_readonly_wal_database() {
    let scratch = tempfile::tempdir().unwrap();
    let data = scratch.path().join("data");
    let config_home = scratch.path().join("配置 # 50%");
    fs::create_dir(&config_home).unwrap();
    let profile_config = config_home.join("settings.json");
    let path = config_home.join("cs2-insight.db");
    let wal = config_home.join("cs2-insight.db-wal");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .unwrap();
    let fixture = seed_demo_library(&connection);
    let before = fs::read(&path).unwrap();
    let wal_before = fs::read(&wal).unwrap();
    let server = RunningServer::launch_with_config(
        "3.2.0-fixture",
        Some((&data, scratch.path())),
        Some(&profile_config),
    );
    assert_eq!(
        profile_request(&server, "GET", "/api/demos/compact", &Value::Null, false).0,
        401
    );
    for case in fixture["cases"].as_array().unwrap() {
        let route = case["path"].as_str().unwrap();
        let response = profile_response(&server, "GET", route, &Value::Null, true);
        assert_eq!(
            u64::from(response.status),
            case["status"].as_u64().unwrap(),
            "{route}"
        );
        if let Some(expected) = case.get("response") {
            let value: Value = serde_json::from_str(&response.body).unwrap();
            assert_eq!(&value, expected, "{route}");
        } else {
            assert_eq!(response.body, case["text"].as_str().unwrap(), "{route}");
        }
    }
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read(&wal).unwrap(), wal_before);
    assert!(!data.exists());
    assert!(!profile_config.exists());

    let writer = connection.unchecked_transaction().unwrap();
    writer
        .execute("UPDATE demo_files SET status = 'loaded' WHERE id = 6", [])
        .unwrap();
    let (status, locked) = profile_request(&server, "GET", "/api/demos/ids", &Value::Null, true);
    assert_eq!(status, 200);
    assert!(!locked["ids"].as_array().unwrap().contains(&json!(6)));
    writer.commit().unwrap();
    let (status, committed) = profile_request(&server, "GET", "/api/demos/ids", &Value::Null, true);
    assert_eq!(status, 200);
    assert!(committed["ids"].as_array().unwrap().contains(&json!(6)));

    for case in fixture["catalog_edits"].as_array().unwrap() {
        let method = case["method"].as_str().unwrap();
        let route = case["path"].as_str().unwrap();
        let (status, response) = profile_request(&server, method, route, &case["body"], true);
        assert_eq!(
            u64::from(status),
            case["status"].as_u64().unwrap(),
            "{method} {route}"
        );
        assert_eq!(response, case["response"], "{method} {route}");
    }
    server.stop();
}

fn seed_demo_library(connection: &rusqlite::Connection) -> Value {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/demo_library_http.json")).unwrap();
    for statement in fixture["schema"].as_array().unwrap() {
        connection
            .execute_batch(statement.as_str().unwrap())
            .unwrap();
    }
    connection
        .execute_batch(include_str!("fixtures/demo_library_seed.sql"))
        .unwrap();
    fixture
}

#[test]
fn demo_players_reuses_only_a_matching_legacy_roster_snapshot() {
    let scratch = tempfile::tempdir().unwrap();
    let demo = scratch.path().join("测试 roster.dem");
    fs::write(&demo, b"cached demo fixture").unwrap();
    let metadata = fs::metadata(&demo).unwrap();
    let modified = metadata
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as i64;
    let config = scratch.path().join("settings.json");
    let database = scratch.path().join("cs2-insight.db");
    let wal = database.with_file_name("cs2-insight.db-wal");
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .unwrap();
    seed_demo_library(&connection);
    let demo_path = demo.to_str().unwrap();
    connection
        .execute("UPDATE demo_files SET path = ? WHERE id = 1", [demo_path])
        .unwrap();
    connection
        .execute_batch(
            "CREATE TABLE demo_roster_cache (demo_id INTEGER PRIMARY KEY, demo_path TEXT NOT NULL,
          cache_version INTEGER NOT NULL, source_content_md5 TEXT, source_file_size INTEGER,
          source_mtime_ns INTEGER, state TEXT NOT NULL, row_count INTEGER NOT NULL,
          error_msg TEXT, updated_at TEXT NOT NULL)",
        )
        .unwrap();
    connection.execute(
        "INSERT INTO demo_roster_cache VALUES (1, ?, 1, 'fixture-md5-one', ?, ?, 'ready', 2, NULL, '')",
        rusqlite::params![demo_path, i64::try_from(metadata.len()).unwrap(), modified],
    ).unwrap();
    let before = fs::read(&database).unwrap();
    let wal_before = fs::read(&wal).unwrap();
    let data = scratch.path().join("data");
    let server = RunningServer::launch_with_config(
        "3.2.0-fixture",
        Some((&data, scratch.path())),
        Some(&config),
    );
    let route = "/api/demos/1/players";
    assert_eq!(
        profile_request(&server, "GET", route, &Value::Null, false).0,
        401
    );
    let (status, data) = profile_request(&server, "GET", route, &Value::Null, true);
    assert_eq!(status, 200);
    assert_eq!(
        data["players"][0],
        json!({
            "name": "Q'uote%_玩家", "player_name": "Q'uote%_玩家", "team": 2, "team_number": 2,
            "team_name": "甲", "steam_id": "STEAM_1:1:40", "steam_id64": "76561198000000123",
            "steamid64": "76561198000000123", "account_id": "39734400", "user_id": "12",
            "kills": 17, "deaths": 5, "assists": 8, "kd": 3.4,
        })
    );
    assert_eq!(data["players"].as_array().unwrap().len(), 2);
    assert_eq!(
        data["match_meta"],
        json!({
            "map_name": "de_荒漠", "total_rounds": 20, "team_a_score": 13, "team_b_score": 7,
            "duration_mins": 24.5, "match_date": "2025-03-18T23:59:59Z",
        })
    );
    assert_eq!(fs::read(&database).unwrap(), before);
    assert_eq!(fs::read(&wal).unwrap(), wal_before);
    assert_eq!(
        profile_request(&server, "GET", "/api/demos/441/players", &Value::Null, true).0,
        404
    );
    fs::write(&demo, b"changed source is longer than cached one").unwrap();
    assert_eq!(
        profile_request(&server, "GET", route, &Value::Null, true).0,
        503
    );
    server.stop();
}

#[test]
fn saves_recorded_media_duration_in_pre_upgrade_database() {
    let scratch = tempfile::tempdir().unwrap();
    let db = scratch.path().join("cs2-insight.db");
    let data = scratch.path().join("data");
    let settings = scratch.path().join("settings.json");
    let clip = scratch.path().join("old clip.mp4");
    fs::write(&clip, b"existing recorded clip").unwrap();
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL;
         CREATE TABLE recorded_clips (id INTEGER PRIMARY KEY, clip_id TEXT, demo_path TEXT,
           demo_filename TEXT, player_name TEXT, output_path TEXT, duration_sec REAL,
           status TEXT, created_at TEXT);
         INSERT INTO recorded_clips VALUES (2, 'other', 'old.dem', NULL, NULL, 'other.mp4',
           99, 'ready', '2026-11-20');",
        )
        .unwrap();
    connection.execute(
        "INSERT INTO recorded_clips VALUES (1, 'first', 'old.dem', NULL, NULL, ?, NULL, 'ready', '2026-11-20')",
        [clip.to_str().unwrap()],
    ).unwrap();
    let server = RunningServer::launch_with_config(
        "3.2.0-fixture",
        Some((&data, scratch.path())),
        Some(&settings),
    );
    let route = "/api/recorded-clips/1/duration";
    let duration = json!({"duration_sec":"17.5"});
    assert_eq!(
        profile_request(&server, "PATCH", route, &duration, false).0,
        401
    );
    for input in [
        json!({}),
        json!({"duration_sec":0.05}),
        json!({"duration_sec":86401}),
    ] {
        assert_eq!(
            profile_request(&server, "PATCH", route, &input, true).0,
            422
        );
    }
    let (status, values) = profile_request(&server, "PATCH", route, &duration, true);
    assert_eq!((status, values), (200, json!({"id":1,"duration_sec":17.5})));
    let (status, rows) = profile_request(&server, "GET", "/api/recorded-clips", &Value::Null, true);
    assert_eq!(status, 200);
    assert_eq!(rows["items"][0]["duration_sec"], 99.0);
    assert_eq!(rows["items"][1]["duration_sec"], 17.5);
    let media = fs::metadata(&clip).unwrap();
    let modified = media
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as i64;
    let cache_path = clip.with_extension("waveform-v1.json");
    let cache_bytes = serde_json::to_vec(&json!({
        "version":1, "source_size":media.len(), "source_mtime_ns":modified,
        "duration_sec":17.5, "peaks":[0.0,0.25,0.5,1.0],
    }))
    .unwrap();
    fs::write(&cache_path, &cache_bytes).unwrap();
    let waveform = "/api/recorded-clips/1/waveform?buckets=8";
    assert_eq!(
        profile_request(&server, "GET", waveform, &Value::Null, false).0,
        401
    );
    let (status, wave) = profile_request(&server, "GET", waveform, &Value::Null, true);
    assert_eq!(status, 200);
    assert_eq!(
        wave,
        json!({
            "cached":true, "duration_sec":17.5, "start_sec":0.0, "end_sec":17.5, "buckets":8,
            "peaks":[0.04,0.04,0.25,0.25,0.5,0.5,1.0,1.0],
        })
    );
    assert_eq!(fs::read(cache_path).unwrap(), cache_bytes);
    let missing = profile_request(
        &server,
        "PATCH",
        "/api/recorded-clips/99/duration",
        &duration,
        true,
    );
    assert_eq!(
        missing,
        (
            404,
            json!({"detail":{"code":"MONTAGE_CLIP_NOT_FOUND","params":{"id":"99"}}})
        )
    );
    assert_eq!(fs::read(clip).unwrap(), b"existing recorded clip");
    server.stop();
}

#[test]
fn opens_saved_lite_cut_projects_and_snapshots_in_the_existing_profile() {
    let scratch = tempfile::tempdir().unwrap();
    let database = scratch.path().join("cs2-insight.db");
    let settings = scratch.path().join("settings.json");
    let data = scratch.path().join("data");
    let server = RunningServer::launch_with_config(
        "3.2.0-fixture",
        Some((&data, scratch.path())),
        Some(&settings),
    );
    let list = "/api/lite-cut/projects";
    let detail = "/api/lite-cut/projects/7";
    let snapshots = "/api/lite-cut/projects/7/snapshots";
    let get = |path, auth| profile_request(&server, "GET", path, &Value::Null, auth);
    assert_eq!(get(list, false).0, 401);
    assert_eq!(
        get(list, true),
        (200, json!({"items":[],"limit":100,"offset":0}))
    );
    assert!(!database.exists());

    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute_batch(
            "PRAGMA journal_mode=WAL;
         CREATE TABLE lite_cut_projects(
             id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL DEFAULT '',
             body_json TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL
         );
         INSERT INTO lite_cut_projects VALUES (6,'Older','{}','2026-11-18','2026-11-18');",
        )
        .unwrap();
    let body = json!({"version":1,"timeline":{"tracks":[]},"future_field":{"preserve":true}});
    connection
        .execute(
            "INSERT INTO lite_cut_projects VALUES (7,'旧工程',?1,'2026-11-19','2026-11-20')",
            [body.to_string()],
        )
        .unwrap();
    let expected = json!({
        "id":7,"name":"旧工程","body":body,"created_at":"2026-11-19","updated_at":"2026-11-20",
    });
    assert_eq!(get(detail, true), (200, expected.clone()));
    assert_eq!(
        get("/api/lite-cut/projects?limit=1&offset=0", true),
        (200, json!({"items":[expected],"limit":1,"offset":0})),
    );
    assert_eq!(get("/api/lite-cut/projects?limit=501", true).0, 422);
    assert_eq!(get("/api/lite-cut/projects/unknown", true).0, 422);
    assert_eq!(get(snapshots, true), (200, json!({"items":[]})));
    assert_eq!(
        get("/api/lite-cut/projects/8/snapshots", true),
        (404, json!({"detail":{"code":"LITECUT_PROJECT_NOT_FOUND"}})),
    );

    connection.execute_batch(
        "CREATE TABLE lite_cut_project_snapshots(
            id INTEGER PRIMARY KEY AUTOINCREMENT, project_id INTEGER NOT NULL,
            name TEXT NOT NULL, body_json TEXT NOT NULL, reason TEXT NOT NULL DEFAULT 'save',
            created_at TEXT NOT NULL, FOREIGN KEY(project_id) REFERENCES lite_cut_projects(id)
         );
         INSERT INTO lite_cut_project_snapshots VALUES (2,7,'旧工程','{}','save','2026-11-19');
         INSERT INTO lite_cut_project_snapshots VALUES (3,6,'Older','{}','save','2026-11-20');
         INSERT INTO lite_cut_project_snapshots VALUES (4,7,'旧工程','{}','before_restore','2026-11-20');",
    ).unwrap();
    let wal = database.with_extension("db-wal");
    let before = [fs::read(&database).unwrap(), fs::read(&wal).unwrap()];
    assert_eq!(
        get(snapshots, true),
        (
            200,
            json!({"items":[
                {"id":4,"project_id":7,"name":"旧工程","reason":"before_restore","created_at":"2026-11-20"},
                {"id":2,"project_id":7,"name":"旧工程","reason":"save","created_at":"2026-11-19"},
            ]})
        ),
    );
    assert_eq!(
        [fs::read(&database).unwrap(), fs::read(&wal).unwrap()],
        before
    );
    server.stop();
}

#[test]
fn demo_library_distinguishes_an_empty_profile_from_a_damaged_database() {
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("cs2-insight.db");
    let server = RunningServer::launch("3.2.0-fixture", Some((scratch.path(), scratch.path())));
    let (status, empty) = profile_request(&server, "GET", "/api/demos/compact", &Value::Null, true);
    assert_eq!(
        (status, empty),
        (
            200,
            json!({"items":[], "limit":200, "offset":0, "total":0, "q":null})
        )
    );
    assert!(!path.exists());
    assert_eq!(
        profile_request(&server, "GET", "/api/demos/123", &Value::Null, true),
        (404, json!({"detail":"Demo not found: 123"}))
    );

    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute(
            "CREATE TABLE sensitivity_sessions (id INTEGER PRIMARY KEY)",
            [],
        )
        .unwrap();
    drop(connection);
    let training_only = fs::read(&path).unwrap();
    let (status, only_training) =
        profile_request(&server, "GET", "/api/demos/ids", &Value::Null, true);
    assert_eq!(
        (status, only_training),
        (200, json!({"ids":[], "limit":1000, "offset":0, "q":null}))
    );
    assert_eq!(fs::read(&path).unwrap(), training_only);

    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute("CREATE TABLE demo_files (id INTEGER PRIMARY KEY)", [])
        .unwrap();
    drop(connection);
    let incomplete = fs::read(&path).unwrap();
    assert_eq!(
        profile_request(&server, "GET", "/api/demos/compact", &Value::Null, true),
        (500, json!({"detail":"Demo library database is unreadable"}))
    );
    assert_eq!(fs::read(&path).unwrap(), incomplete);

    let damaged = b"fixture damaged sqlite database";
    fs::write(&path, damaged).unwrap();
    assert_eq!(
        profile_request(&server, "GET", "/api/demos/compact", &Value::Null, true),
        (500, json!({"detail":"Demo library database is unreadable"}))
    );
    assert_eq!(fs::read(&path).unwrap(), damaged);
    assert_eq!(
        profile_request(&server, "GET", "/api/demos/scan", &Value::Null, true).0,
        404
    );
    server.stop();
}

fn subscribe_demos(server: &RunningServer) -> Result<BufReader<TcpStream>, u16> {
    let mut socket = TcpStream::connect_timeout(&server.addr, RESPONSE_TIMEOUT).unwrap();
    socket.set_read_timeout(Some(RESPONSE_TIMEOUT)).unwrap();
    let request = format!("GET /api/demos/stream?_session=synthetic-token HTTP/1.1\r\nHost: {}\r\nAccept: text/event-stream\r\nOrigin: https://tauri.localhost\r\n\r\n", server.addr);
    socket.write_all(request.as_bytes()).unwrap();
    let mut reader = BufReader::new(socket);
    let mut status = String::new();
    reader.read_line(&mut status).unwrap();
    let code = status
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse::<u16>()
        .unwrap();
    if code != 200 {
        return Err(code);
    }
    let mut headers = BTreeMap::new();
    loop {
        let mut header = String::new();
        assert_ne!(reader.read_line(&mut header).unwrap(), 0);
        if header == "\r\n" {
            break;
        }
        let (key, value) = header.split_once(':').unwrap();
        headers.insert(key.to_ascii_lowercase(), value.trim().to_owned());
    }
    assert_eq!(headers["content-type"], "text/event-stream; charset=utf-8");
    assert_eq!(headers["cache-control"], "no-store, no-transform");
    assert_eq!(headers["x-accel-buffering"], "no");
    assert_eq!(headers["connection"], "keep-alive");
    assert_eq!(
        headers["access-control-allow-origin"],
        "https://tauri.localhost"
    );
    assert_eq!(headers["transfer-encoding"], "chunked");
    assert_eq!(stream_chunk(&mut reader).unwrap().unwrap(), ": ok\n\n");
    Ok(reader)
}

fn stream_chunk(reader: &mut BufReader<TcpStream>) -> std::io::Result<Option<String>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let len = usize::from_str_radix(line.trim(), 16).unwrap();
    if len == 0 {
        return Ok(None);
    }
    let mut chunk = vec![0; len + 2];
    reader.read_exact(&mut chunk)?;
    assert_eq!(&chunk[len..], b"\r\n");
    chunk.truncate(len);
    Ok(Some(String::from_utf8(chunk).unwrap()))
}

#[test]
fn demo_catalog_commits_concurrent_edits_and_notifies_long_lived_subscribers() {
    let scratch = tempfile::tempdir().unwrap();
    let database = scratch.path().join("cs2-insight.db");
    let connection = rusqlite::Connection::open(database).unwrap();
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .unwrap();
    seed_demo_library(&connection);
    connection
        .execute_batch(
            "CREATE TRIGGER block_fixture_name AFTER UPDATE OF display_name ON demo_files
        WHEN NEW.display_name = 'blocked fixture' BEGIN UPDATE demo_files SET file_size = x'FF'
        WHERE id = NEW.id; END;",
        )
        .unwrap();
    let server = RunningServer::launch("3.2.0-fixture", Some((scratch.path(), scratch.path())));
    assert_eq!(
        profile_request(&server, "GET", "/api/demos/stream", &Value::Null, false).0,
        401
    );
    assert_eq!(
        profile_request(
            &server,
            "PATCH",
            "/api/demos/1",
            &json!({"display_name":"not authorized"}),
            false
        )
        .0,
        401
    );
    let mut first = subscribe_demos(&server).unwrap();
    let mut second = subscribe_demos(&server).unwrap();
    let third = subscribe_demos(&server).unwrap();
    let fourth = subscribe_demos(&server).unwrap();
    assert_eq!(
        profile_request(&server, "GET", "/api/demos/stream", &Value::Null, true).0,
        503
    );
    drop(third);
    drop(fourth);
    // A separate long-lived client exercises the freed slots without changing the data.
    let deadline = Instant::now() + RESPONSE_TIMEOUT;
    loop {
        match subscribe_demos(&server) {
            Ok(replacement) => {
                drop(replacement);
                break;
            }
            Err(503) => {
                assert!(
                    Instant::now() < deadline,
                    "disconnected subscribers retained all slots"
                );
                thread::sleep(Duration::from_millis(20));
            }
            Err(code) => panic!("unexpected subscription response {code}"),
        }
    }

    let gate = std::sync::Barrier::new(2);
    thread::scope(|scope| {
        for (route, payload) in [
            ("/api/demos/1", json!({"display_name":"  測試 title  "})),
            ("/api/demos/1/remark", json!({"remark":"  保留空白  "})),
        ] {
            let server = &server;
            let gate = &gate;
            scope.spawn(move || {
                gate.wait();
                assert_eq!(
                    profile_request(server, "PATCH", route, &payload, true).0,
                    200
                );
            });
        }
    });
    let expected = "event: library\ndata: {\"reason\": \"changed\"}\n\n";
    assert_eq!(stream_chunk(&mut first).unwrap().unwrap(), expected);
    let committed: (String, String, String, String) = connection
        .query_row(
            "SELECT display_name, remark, filename, content_md5 FROM demo_files WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        committed,
        (
            "測試 title".to_owned(),
            "  保留空白  ".to_owned(),
            "Round_50% Q'uote.dem".to_owned(),
            "fixture-md5-one".to_owned()
        )
    );
    assert_eq!(stream_chunk(&mut second).unwrap().unwrap(), expected);
    let rejected = profile_response(
        &server,
        "PATCH",
        "/api/demos/1",
        &json!({"display_name":"blocked fixture"}),
        true,
    );
    assert_eq!(rejected.status, 500);
    assert_eq!(rejected.body, "Internal Server Error");
    assert_eq!(
        profile_request(
            &server,
            "PATCH",
            "/api/demos/999/remark",
            &json!({"remark":"missing"}),
            true
        )
        .0,
        404
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT display_name FROM demo_files WHERE id = 1",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "測試 title"
    );
    assert_eq!(
        connection
            .query_row("SELECT file_size FROM demo_files WHERE id = 1", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        150
    );

    // The next chunk is a real 20-second heartbeat, past the regular connection limit.
    first
        .get_ref()
        .set_read_timeout(Some(Duration::from_secs(24)))
        .unwrap();
    assert_eq!(
        stream_chunk(&mut first).unwrap().unwrap(),
        ": keepalive\n\n"
    );
    server.stop();
    assert!(stream_chunk(&mut first).unwrap().is_none());
}

#[test]
fn demo_watch_inspection_counts_without_opening_candidates_or_mutating_catalog() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("检验 Demos");
    fs::create_dir_all(root.join("sub/deep")).unwrap();
    for file in [
        "one.DEM",
        "one.ZIP",
        "ignored.dem.bz2",
        "sub/two.dem",
        "sub/two.ZiP",
        "sub/deep/three.dem",
    ] {
        fs::write(
            root.join(file),
            b"fixture intentionally contains neither demo nor archive bytes",
        )
        .unwrap();
    }
    let server = RunningServer::launch("3.2.0-fixture", Some((scratch.path(), scratch.path())));
    for (depth, demos, zips) in [(0, 1, 1), (1, 2, 2), (2, 3, 2)] {
        let (status, inspected) = profile_request(
            &server,
            "POST",
            "/api/demos/watch-path/inspect",
            &json!({"path": root, "max_depth": depth}),
            true,
        );
        assert_eq!(
            (status, inspected),
            (
                200,
                json!({"valid":true,"path":root,"demo_count":demos,"zip_count":zips,"max_depth":depth})
            )
        );
    }
    let missing = root.join("missing");
    let (status, invalid) = profile_request(
        &server,
        "POST",
        "/api/demos/watch-path/inspect",
        &json!({"path": missing}),
        true,
    );
    assert_eq!(
        (status, invalid),
        (
            200,
            json!({"valid":false,"path":missing,"demo_count":0,"zip_count":0,"error":"目录不存在或无法访问"})
        )
    );
    assert!(!scratch.path().join("cs2-insight.db").exists());
    assert!(!scratch.path().join("custom-config.json").exists());
    server.stop();
}

const REPLAY_ROUTE: &str = "/api/demo/replay/binary";

fn binary_replay(server: &RunningServer, payload: &Value) -> Vec<u8> {
    let body = payload.to_string();
    let mut socket = TcpStream::connect_timeout(&server.addr, RESPONSE_TIMEOUT).unwrap();
    socket.set_read_timeout(Some(RESPONSE_TIMEOUT)).unwrap();
    let request = format!(
        "POST {REPLAY_ROUTE} HTTP/1.1\r\nHost: {}\r\nX-CS2-Insight-Token: synthetic-token\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        server.addr,
        body.len(),
    );
    socket.write_all(request.as_bytes()).unwrap();
    let mut response = Vec::new();
    socket.read_to_end(&mut response).unwrap();
    let head_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap();
    let headers = std::str::from_utf8(&response[..head_end])
        .unwrap()
        .to_ascii_lowercase();
    assert!(
        headers.starts_with("http/1.1 200 "),
        "{headers}: {}",
        String::from_utf8_lossy(&response[head_end + 4..])
    );
    assert!(headers.contains("content-type: application/vnd.cs2-insight.replay-v1"));
    assert!(headers.contains("cache-control: no-store"));
    assert!(headers.contains("x-cs2-replay-protocol: 1"));
    response.split_off(head_end + 4)
}

fn replay_header(bytes: &[u8]) -> (Value, usize) {
    assert_eq!(&bytes[..8], b"CS2RPL01");
    assert_eq!(&bytes[8..12], &[1, 0, 0, 0]);
    let end = 16 + u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let header = serde_json::from_slice(&bytes[16..end]).unwrap();
    (header, end.next_multiple_of(4))
}

#[test]
fn replay_cache_clear_requires_the_session_and_preserves_unrelated_files() {
    let scratch = tempfile::tempdir().unwrap();
    let data = scratch.path();
    let key = "0123456789012345678901234567890123456789";
    let managed = [
        (format!("cache/demo-replay/matches/{key}.parquet"), 3),
        (format!("replay-match/{key}.meta.json"), 5),
        (format!("cache/demo-replay/rounds/{key}.json.gz"), 7),
        (format!("replay-effects-cache/{key}.pkl"), 11),
    ];
    let keep = [
        "replay-match/source.dem",
        "replay-match/unrelated.json",
        "replay-match/subdirectory/1234.parquet",
        "cs2-insight.db",
    ];
    for relative in keep {
        let path = data.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"keep original").unwrap();
    }
    for (relative, size) in &managed {
        let path = data.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![0u8; *size]).unwrap();
    }
    let server = RunningServer::launch("3.2.0-fixture", Some((data, data)));
    let route = "/api/demo/replay/cache";
    assert_eq!(
        profile_request(&server, "DELETE", route, &Value::Null, false).0,
        401
    );
    assert!(managed.iter().all(|(path, _)| data.join(path).is_file()));
    let (status, cleared) = profile_request(&server, "DELETE", route, &Value::Null, true);
    assert_eq!(status, 200);
    assert_eq!(cleared["removed_files"], 4);
    assert_eq!(cleared["removed_bytes"], 26);
    assert!(managed.iter().all(|(path, _)| !data.join(path).exists()));
    for path in keep {
        assert_eq!(fs::read(data.join(path)).unwrap(), b"keep original");
    }
    assert_eq!(
        cleared["cache"],
        profile_request(&server, "GET", route, &Value::Null, true).1
    );
    assert_eq!(
        profile_request(&server, "DELETE", route, &Value::Null, true).1["removed_files"],
        0
    );
    server.stop();
}

#[test]
fn replay_reads_exact_round_from_primary_or_legacy_cache_without_writes() {
    let scratch = tempfile::tempdir().unwrap();
    let fixture = ReplayFixture::new(scratch.path());
    let server = RunningServer::launch("3.2.0-fixture", Some((scratch.path(), scratch.path())));
    let request = fixture.request();
    assert_eq!(
        profile_request(&server, "POST", REPLAY_ROUTE, &request, false).0,
        401
    );
    let (status, miss) = profile_request(&server, "POST", REPLAY_ROUTE, &request, true);
    assert_eq!(status, 409);
    assert!(miss["detail"]
        .as_str()
        .unwrap()
        .contains("analyze the demo again"));
    assert!(!scratch.path().join(replay_fixture::CURRENT).exists());
    assert!(!scratch.path().join(replay_fixture::LEGACY).exists());

    let (legacy_parquet, legacy_meta) = fixture.write(scratch.path(), replay_fixture::LEGACY);
    assert!(fs::metadata(&legacy_parquet).unwrap().len() < 16_384);
    let (current_parquet, current_meta) = fixture.write(scratch.path(), replay_fixture::CURRENT);
    let mut wrong = fixture.metadata.clone();
    wrong["version"] = json!(4);
    fs::write(&current_meta, wrong.to_string()).unwrap();
    let legacy_before = [
        fs::read(&legacy_parquet).unwrap(),
        fs::read(&legacy_meta).unwrap(),
    ];
    let mut upload_name = request.clone();
    upload_name["path"] = json!(fixture.demo.file_name().unwrap().to_str().unwrap());
    let packet = binary_replay(&server, &upload_name);
    let (header, data) = replay_header(&packet);
    assert_eq!(header["cache_key"], fixture.key);
    assert_eq!(header["round_number"], 2);
    assert_eq!(header["frame_count"], 6);
    assert_eq!(header["row_count"], 4);
    assert_eq!(header["protocol"], "cs2-replay-binary-v1");
    assert_eq!(
        header["cache"],
        json!({"frames":"parquet_binary_hit","effects":"parquet_hit","parsed":false})
    );
    assert_eq!(
        header["effect_tracks"],
        json!([
            {"kind":"smoke","start_tick":98,"end_tick":100,"payload":{"name":"烟"}},
            {"kind":"fire","start_tick":112,"end_tick":118}
        ])
    );
    assert_eq!(header["shots"], json!([{"tick":102,"player":"玩家"}]));
    assert_eq!(header["events"], json!([{"type":"kill","tick":109}]));
    assert!(header["strings"]
        .as_array()
        .unwrap()
        .contains(&json!("Bench")));
    assert!(header["strings"]
        .as_array()
        .unwrap()
        .contains(&json!("Not sampled")));
    let mut words = packet[data..]
        .as_chunks::<4>()
        .0
        .iter()
        .copied()
        .map(i32::from_le_bytes);
    assert_eq!(
        words.by_ref().take(6).collect::<Vec<_>>(),
        replay_fixture::TICKS
    );
    assert_eq!(words.take(7).collect::<Vec<_>>(), [0, 1, 2, 2, 3, 4, 4]);
    assert_eq!(
        [
            fs::read(&legacy_parquet).unwrap(),
            fs::read(&legacy_meta).unwrap()
        ],
        legacy_before
    );

    let mut primary = fixture.metadata.clone();
    primary["effect_warnings"] = json!(["primary cache"]);
    fs::write(&current_meta, primary.to_string()).unwrap();
    assert_eq!(
        replay_header(&binary_replay(&server, &request)).0["effect_warnings"],
        json!(["primary cache"])
    );
    let mut another_range = request.clone();
    another_range["end_tick"] = json!(113);
    assert_eq!(
        profile_request(&server, "POST", REPLAY_ROUTE, &another_range, true).0,
        409
    );
    let mut different_fps = request.clone();
    different_fps["fps"] = json!(30);
    assert_eq!(
        profile_request(&server, "POST", REPLAY_ROUTE, &different_fps, true).0,
        409
    );

    let malformed = b"fixture damaged parquet";
    fs::write(&current_parquet, malformed).unwrap();
    assert_eq!(
        profile_request(&server, "POST", REPLAY_ROUTE, &request, true),
        (
            503,
            json!({"detail":"Binary replay unavailable: Exception"})
        )
    );
    assert_eq!(fs::read(current_parquet).unwrap(), malformed);
    assert_eq!(
        fs::read_to_string(current_meta).unwrap(),
        primary.to_string()
    );
    fs::write(
        &fixture.demo,
        b"fixture source replacement makes the old cache stale",
    )
    .unwrap();
    assert_eq!(
        profile_request(&server, "POST", REPLAY_ROUTE, &request, true).0,
        409
    );
    server.stop();
}

#[test]
fn replay_validates_range_and_demo_path_before_reading_cache() {
    let scratch = tempfile::tempdir().unwrap();
    let server = RunningServer::launch("3.2.0-fixture", Some((scratch.path(), scratch.path())));
    for (request, code, detail) in [
        (
            json!({"path":"nonexisting.dem","start_tick":"20","end_tick":"20"}),
            422,
            "end_tick must be greater than start_tick",
        ),
        (
            json!({"path":"nonexisting.dem","start_tick":0,"end_tick":38401}),
            422,
            "Replay range cannot exceed 10 minutes",
        ),
        (
            json!({"path":" ","start_tick":1,"end_tick":2}),
            400,
            "Demo 路径为空",
        ),
        (
            json!({"path":"fixture-missing-replay.dem","start_tick":1,"end_tick":2}),
            404,
            "未找到 Demo 文件: fixture-missing-replay.dem",
        ),
    ] {
        assert_eq!(
            profile_request(&server, "POST", REPLAY_ROUTE, &request, true),
            (code, json!({"detail":detail}))
        );
    }
    let (code, body) = profile_request(
        &server,
        "POST",
        REPLAY_ROUTE,
        &json!({"path":"","start_tick":-1,"end_tick":0,"tick_rate":257,"fps":0}),
        true,
    );
    assert_eq!(code, 422);
    assert_eq!(
        body["detail"]
            .as_array()
            .unwrap()
            .iter()
            .map(|error| error["type"].clone())
            .collect::<Vec<_>>(),
        json!([
            "string_too_short",
            "greater_than_equal",
            "greater_than",
            "less_than_equal",
            "greater_than_equal"
        ])
        .as_array()
        .unwrap()
        .clone()
    );
    assert!(!scratch.path().join(replay_fixture::CURRENT).exists());
    server.stop();
}

#[test]
#[ignore = "requires the pinned demoparser2 wheel and MGS_REPLAY_PYTHON"]
fn replay_packet_bytes_match_the_shipped_parser_wheel() {
    let scratch = tempfile::tempdir().unwrap();
    let fixture = ReplayFixture::new(scratch.path());
    fixture.write(scratch.path(), replay_fixture::LEGACY);
    let python = std::env::var_os("MGS_REPLAY_PYTHON").expect("cached Python 3.12 interpreter");
    let expected = Command::new(python)
        .arg("-B")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/replay_binary_python.py"))
        .arg(scratch.path())
        .arg(&fixture.demo)
        .output()
        .unwrap();
    assert!(
        expected.status.success(),
        "{}",
        String::from_utf8_lossy(&expected.stderr)
    );
    let server = RunningServer::launch("3.2.0-fixture", Some((scratch.path(), scratch.path())));
    assert_eq!(binary_replay(&server, &fixture.request()), expected.stdout);
    server.stop();
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

struct Response {
    status: u16,
    headers: BTreeMap<String, String>,
    body: String,
}

fn send_request(addr: SocketAddr, request: &str) -> Response {
    let mut socket = TcpStream::connect_timeout(&addr, RESPONSE_TIMEOUT).unwrap();
    socket.set_read_timeout(Some(RESPONSE_TIMEOUT)).unwrap();
    socket.write_all(request.as_bytes()).unwrap();
    let mut raw = String::new();
    socket.read_to_string(&mut raw).unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").expect("valid HTTP headers");
    let mut head = head.split("\r\n");
    let status = head
        .next()
        .unwrap()
        .split(' ')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = head
        .map(|line| {
            let (name, value) = line.split_once(':').unwrap();
            (name.to_ascii_lowercase(), value.trim().to_owned())
        })
        .collect();
    Response {
        status,
        headers,
        body: body.to_owned(),
    }
}

fn replay(server: &RunningServer, case: &Case) -> Response {
    let body = case.body.as_deref().unwrap_or_default();
    let mut request = format!(
        "{} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Length: {}\r\n",
        case.method,
        case.path,
        server.addr,
        body.len(),
    );
    for (name, value) in &case.headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    request.push_str(body);
    send_request(server.addr, &request)
}

#[test]
fn native_startup_and_http_routes_match_the_recorded_python_responses() {
    let fixture: Fixture = serde_json::from_str(include_str!("fixtures/native_http.json")).unwrap();
    let server = RunningServer::start(&fixture.version);
    let mut last_ready = None;
    for case in fixture.cases {
        let response = replay(&server, &case);
        let headers = response
            .headers
            .into_iter()
            .filter(|(key, _)| {
                matches!(key.as_str(), "allow" | "vary" | "content-type")
                    || key.starts_with("access-control-")
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(response.status, case.status, "{}", case.name);
        assert_eq!(headers, case.response_headers, "{}", case.name);
        if let Some(expected) = case.response_json {
            let mut body: Value = serde_json::from_str(&response.body).unwrap();
            if let (Some(payload), Some(ready)) = (
                body["last_payload_at"].as_f64(),
                body["last_ready_at"].as_f64(),
            ) {
                assert!(ready <= payload || (ready - payload).abs() < 0.001);
                if ready > 0.0 {
                    if let Some(previous) = last_ready {
                        assert_eq!(ready, previous);
                    } else {
                        last_ready = Some(ready);
                    }
                }
                for key in ["last_payload_at", "last_ready_at"] {
                    if body[key].as_f64().is_some_and(|value| value > 0.0) {
                        body[key] = Value::String("<positive monotonic time>".to_owned());
                    }
                }
            }
            assert_eq!(body, expected, "{}", case.name);
        } else {
            assert_eq!(
                response.body,
                case.response_text.unwrap_or_default(),
                "{}",
                case.name
            );
        }
    }
    server.stop();
}

#[test]
fn oversized_bodies_are_rejected_and_stop_releases_an_idle_connection() {
    let server = RunningServer::start("3.2.0-parity");
    let response = send_request(server.addr, &format!(
        "POST /api/gsi/cs2 HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: 1048577\r\nConnection: close\r\n\r\n",
        server.addr
    ));
    assert_eq!(response.status, 413);
    let body = "0".repeat(1_048_577);
    let response = send_request(server.addr, &format!(
        "POST /api/gsi/cs2 HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n",
        server.addr, body.len()
    ));
    assert_eq!(response.status, 413);

    let address = server.addr;
    let mut idle = TcpStream::connect_timeout(&address, RESPONSE_TIMEOUT).unwrap();
    idle.write_all(b"GET /api/health HTTP/1.1\r\n").unwrap();
    let redirect = send_request(address, &format!(
        "GET /api/health/?_session=synthetic-token&keep=1 HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    ));
    assert_eq!(redirect.status, 307);
    assert_eq!(
        redirect.headers["location"],
        format!("http://{address}/api/health?keep=1")
    );
    server.stop();
    assert!(TcpStream::connect_timeout(&address, RESPONSE_TIMEOUT).is_err());
}
