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

fn resolved_fixture_path(path: &Path) -> String {
    let path = fs::canonicalize(path).unwrap();
    let text = path.to_string_lossy();
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
}

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
                .env("CS2_INSIGHT_INSTANCE_ID", " fixture-process ")
                .env("APPDATA", data.join("fixture-appdata"));
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
fn restores_existing_cs2_config_backup_and_preserves_failed_recovery() {
    let scratch = tempfile::tempdir().unwrap();
    let profile = scratch.path().join("测试数据");
    let bundle = scratch.path().join("bundle");
    let backup = profile.join(".cs2_config_backup");
    let steam_cfg = scratch.path().join("Steam/userdata/123/730/local/cfg");
    fs::create_dir_all(&steam_cfg).unwrap();
    fs::create_dir_all(&backup).unwrap();
    fs::create_dir(&bundle).unwrap();
    let state = backup.join("recording_state.json");
    let marker = br#"{"version":1,"status":"recording","started_at_iso":"fixture-time"}"#;
    fs::write(&state, marker).unwrap();
    let current = steam_cfg.join("cs2_machine_convars.vcfg");
    let generated = steam_cfg.join("cs2_video.txt");
    let untouched = steam_cfg.join("my-video.txt");
    fs::write(&current, b"modified during capture").unwrap();
    fs::write(&generated, b"generated during capture").unwrap();
    fs::write(&untouched, b"other data").unwrap();
    let backup_rel = "Steam账号_123/730/local/cfg/cs2_machine_convars.vcfg";
    let saved = backup.join(backup_rel);
    fs::create_dir_all(saved.parent().unwrap()).unwrap();
    let original = b"pre-recording\r\nsettings\xff";
    fs::write(&saved, original).unwrap();
    let manifest = backup.join("manifest.json");
    let entries = json!([
        {"original":current,"account_id":"123","existed":true,"backup_relpath":backup_rel},
        {"original":generated,"existed":false},
    ]);
    let mut invalid = entries.clone();
    invalid[1]["original"] = json!(untouched);
    fs::write(
        &manifest,
        json!({"version":4,"entries":invalid}).to_string(),
    )
    .unwrap();

    let server = RunningServer::launch("3.2.0-fixture", Some((&profile, &bundle)));
    let restore = "/api/config-backup/restore";
    assert_eq!(
        profile_request(&server, "POST", restore, &Value::Null, false).0,
        401
    );
    assert_eq!(
        profile_request(&server, "GET", restore, &Value::Null, true).0,
        405
    );
    let (status, refused) = profile_request(&server, "POST", restore, &Value::Null, true);
    assert_eq!(status, 200);
    assert_eq!(refused["ok"], false);
    assert_eq!(refused["code"], "CONFIG_RESTORE_PARTIAL");
    assert_eq!(fs::read(&state).unwrap(), marker);
    assert_eq!(fs::read(&current).unwrap(), b"modified during capture");
    assert_eq!(fs::read(&untouched).unwrap(), b"other data");

    let valid = json!({"version":4,"entries":entries}).to_string();
    fs::write(&manifest, &valid).unwrap();
    assert_eq!(
        profile_request(&server, "POST", restore, &Value::Null, true),
        (
            200,
            json!({"ok":true,"code":"CONFIG_RESTORE_OK","restored":2})
        )
    );
    assert_eq!(fs::read(&current).unwrap(), original);
    assert!(!generated.exists());
    assert_eq!(fs::read(&untouched).unwrap(), b"other data");
    assert_eq!(fs::read(&saved).unwrap(), original);
    assert_eq!(fs::read_to_string(&manifest).unwrap(), valid);
    let (_, status) = profile_request(
        &server,
        "GET",
        "/api/config-backup/status",
        &Value::Null,
        true,
    );
    assert_eq!(status["restore_required"], false);
    assert_eq!(
        profile_request(&server, "POST", restore, &Value::Null, true),
        (
            200,
            json!({"ok":true,"code":"CONFIG_RESTORE_NOT_NEEDED","restored":0})
        )
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
    assert_eq!(runtime["data_dir"], resolved_fixture_path(&profile));
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
fn library_analysis_keeps_the_saved_result_when_a_same_name_upload_exists() {
    let scratch = tempfile::tempdir().unwrap();
    let database = scratch.path().join("cs2-insight.db");
    let config = scratch.path().join("settings.json");
    let missing = scratch.path().join("saved").join("match.dem");
    let profile = scratch.path().join("profile");
    let uploads = profile.join("fixture-temp/cs2_insight_demos");
    fs::create_dir_all(&uploads).unwrap();
    fs::write(uploads.join("match.dem"), b"a different Demo").unwrap();
    let connection = rusqlite::Connection::open(&database).unwrap();
    seed_demo_library(&connection);
    let path = missing.to_str().unwrap();
    connection.execute(
        "INSERT INTO demo_files(id,path,filename,status,added_at,remark) VALUES (91,?1,'match.dem','done','fixture','retained')",
        [path],
    ).unwrap();
    connection.execute(
        "INSERT INTO match_results(demo_path,result_json,created_at) VALUES (?1,'{\"legacy\":true}','fixture')",
        [path],
    ).unwrap();
    drop(connection);
    let before = fs::read(&database).unwrap();
    let server = RunningServer::launch_with_config(
        "3.2.0-fixture",
        Some((&profile, scratch.path())),
        Some(&config),
    );
    assert_eq!(
        profile_request(
            &server, "POST", "/api/demos/91/analyze", &json!({"target_players":["Fixture"]}), true,
        ),
        (404, json!({"detail":{"code":"DEMO_FILE_NOT_FOUND"}}))
    );
    assert_eq!(fs::read(&database).unwrap(), before);
    assert_eq!(fs::read(uploads.join("match.dem")).unwrap(), b"a different Demo");
    server.stop();
}

#[test]
#[ignore = "requires the pinned native parser and the public Demo"]
fn analyzes_a_real_demo_from_both_entry_points_and_rolls_back_a_failed_save() {
    let scratch = tempfile::tempdir().unwrap();
    let demo = std::path::PathBuf::from(std::env::var_os("MGS_NATIVE_PUBLIC_DEMO").unwrap());
    let parser = std::path::PathBuf::from(std::env::var_os("MGS_NATIVE_PARSER_EXE").unwrap());
    let bundle = parser.parent().unwrap().join("data");
    let original = fs::metadata(&demo).unwrap().modified().unwrap();
    let connection = rusqlite::Connection::open(scratch.path().join("cs2-insight.db")).unwrap();
    seed_demo_library(&connection);
    connection.execute(
        "INSERT INTO demo_files(id,path,filename,status,added_at,remark) VALUES(91,?1,'public.dem','loaded','fixture','保留备注')",
        [demo.to_str().unwrap()],
    ).unwrap();
    let server = RunningServer::launch_with_config(
        "3.2.0-fixture", Some((scratch.path(), &bundle)), Some(&scratch.path().join("cs2-insight.config.json")),
    );
    let (status, roster) = profile_request(&server, "GET", "/api/demos/91/players", &Value::Null, true);
    assert_eq!(status, 200, "{roster}");
    let names: Vec<_> = roster["players"].as_array().unwrap().iter().take(2)
        .map(|player| player["name"].as_str().unwrap()).collect();
    assert_eq!(names.len(), 2);
    let query = form_urlencoded::Serializer::new(String::new())
        .append_pair("path", demo.to_str().unwrap()).finish();
    let dto = json!({"target_players":names,"freeze_to_death_rounds":[2,4,5]});
    let (status, manual) = profile_request(&server, "POST", &format!("/api/demo/parse-multi?{query}"), &dto, true);
    assert_eq!(status, 200);
    assert_eq!(manual["players"].as_object().unwrap().len(), 2);
    assert_eq!(manual["analysis_workspace"]["rounds"].as_array().unwrap().len(), 9);
    let saved = || connection.query_row(
        "SELECT result_json FROM match_results WHERE demo_path=?1", [demo.to_str().unwrap()],
        |row| row.get::<_, String>(0),
    );
    assert!(matches!(saved(), Err(rusqlite::Error::QueryReturnedNoRows)));
    let library = "/api/demos/91/analyze";
    let (status, recorded) = profile_request(&server, "POST", library, &dto, true);
    assert_eq!(status, 200);
    assert_eq!(recorded["demo_filename"], "public.dem");
    assert_eq!(recorded["players"], manual["players"]);
    let snapshot = saved().unwrap();
    let events = || connection.query_row(
        "SELECT COUNT(*) FROM demo_timeline_events WHERE demo_path=?1", [demo.to_str().unwrap()],
        |row| row.get::<_, i64>(0),
    ).unwrap();
    let event_count = events();
    assert!(event_count > 0);
    connection.execute_batch(
        "CREATE TRIGGER reject_fixture_timeline BEFORE INSERT ON demo_timeline_events
         BEGIN SELECT RAISE(ABORT, 'fixture rollback'); END;",
    ).unwrap();
    let (status, _) = profile_request(&server, "POST", library, &json!({"target_players":names,"freeze_to_death_rounds":[]}), true);
    assert_eq!(status, 500);
    assert_eq!(saved().unwrap(), snapshot);
    assert_eq!(events(), event_count);
    assert_eq!(connection.query_row("SELECT remark FROM demo_files WHERE id=91", [], |row| row.get::<_, String>(0)).unwrap(), "保留备注");
    assert_eq!(fs::metadata(&demo).unwrap().modified().unwrap(), original);
    server.stop();
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
    let summary_route = "/api/demos/batch-summary";
    let ids = json!({"ids":[1,441,1]});
    assert_eq!(
        profile_request(&server, "POST", summary_route, &ids, false).0,
        401
    );
    let (status, batch) = profile_request(&server, "POST", summary_route, &ids, true);
    assert_eq!(status, 200);
    assert_eq!(batch["items"].as_array().unwrap().len(), 2);
    assert_eq!(batch["items"][0]["players"], data["players"]);
    assert_eq!(batch["items"][0]["match_meta"], data["match_meta"]);
    assert_eq!(batch["items"][0], batch["items"][1]);
    let missing = json!({"filename":"441","id":441,"code":"DEMO_FILE_NOT_FOUND"});
    assert_eq!(batch["failed"], json!([missing]));
    let names = json!({
        "demo_ids":[1,441],"mode":"manual","manual_lines":[" q ' uote % _ 玩家 ", "Q'uote"]
    });
    assert_eq!(
        profile_request(
            &server,
            "POST",
            "/api/demos/batch-resolve-players",
            &names,
            true
        ),
        (
            200,
            json!({"resolved":{"1":["Q'uote%_玩家"],"441":[]},"failed":[missing]})
        ),
    );
    let saved_settings = br#"{"expected_parse_players":["q'uote","q'uote",""]}"#;
    fs::write(&config, saved_settings).unwrap();
    let saved = json!({"demo_ids":[1],"mode":"config_expected"});
    assert_eq!(
        profile_request(
            &server,
            "POST",
            "/api/demos/batch-resolve-players",
            &saved,
            true
        ),
        (200, json!({"resolved":{"1":["Q'uote%_玩家"]},"failed":[]})),
    );
    assert_eq!(fs::read(&config).unwrap(), saved_settings);
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
fn checks_obs_preflight_without_starting_an_unconfigured_or_missing_program() {
    let scratch = tempfile::tempdir().unwrap();
    let settings = scratch.path().join("settings.json");
    let data = scratch.path().join("data");
    let server = RunningServer::launch_with_config(
        "3.2.0-fixture",
        Some((&data, scratch.path())),
        Some(&settings),
    );
    let check = "/api/obs/config-check";
    assert_eq!(
        profile_request(&server, "POST", check, &Value::Null, false).0,
        401
    );
    assert_eq!(
        profile_request(&server, "POST", check, &Value::Null, true),
        (
            200,
            json!({
                "path_ok":false, "connected":false, "error":"请先配置 OBS 路径，再点击配置检查",
            })
        ),
    );
    let absent = scratch.path().join("missing OBS.exe");
    let config = json!({"obs":{"obs_path":absent}});
    let config = config.to_string();
    fs::write(&settings, &config).unwrap();
    assert_eq!(
        profile_request(&server, "POST", "/api/obs/is-running", &Value::Null, true),
        (200, json!({"running":false, "obs_path":absent})),
    );
    assert_eq!(
        profile_request(
            &server,
            "POST",
            check,
            &json!({"host":"localhost","port":"4455"}),
            true
        ),
        (200, json!({"path_ok":false, "error":"OBS 路径不存在"})),
    );
    assert_eq!(
        profile_request(
            &server,
            "POST",
            check,
            &json!({"host":"localhost","port":65536}),
            true
        )
        .0,
        422,
    );
    assert_eq!(fs::read_to_string(&settings).unwrap(), config);
    server.stop();
}

#[test]
fn reports_obs_profile_and_backup_without_launching_or_connecting_to_obs() {
    let scratch = tempfile::tempdir().unwrap();
    let settings = scratch.path().join("settings.json");
    let data = scratch.path().join("data");
    let obs = data.join("fixture-appdata/obs-studio");
    let backup = data.join(".obs_config_backups/fixture-save");
    fs::create_dir_all(&obs).unwrap();
    fs::create_dir_all(&backup).unwrap();
    let global = "[Basic]\nCurrentProfile=录制配置\nCurrentProfileDir=fixture\nCurrentSceneCollection=桌面\n";
    fs::write(obs.join("global.ini"), global).unwrap();
    fs::write(
        backup.join("manifest.json"),
        br#"{"created_at":"2026-11-20"}"#,
    )
    .unwrap();
    let config = json!({"obs":{"host":"127.0.0.1","port":1}}).to_string();
    fs::write(&settings, &config).unwrap();
    let server = RunningServer::launch_with_config(
        "3.2.0-fixture",
        Some((&data, scratch.path())),
        Some(&settings),
    );
    let route = "/api/obs-config/status";
    assert_eq!(
        profile_request(&server, "GET", route, &Value::Null, false).0,
        401
    );
    let (code, response) = profile_request(&server, "GET", route, &Value::Null, true);
    assert_eq!(code, 200);
    assert_eq!(response["obs_connected"], false);
    #[cfg(windows)]
    {
        assert_eq!(response["active_profile"], "录制配置");
        assert_eq!(response["active_scene_collection"], "桌面");
    }
    assert_eq!(
        response["latest_backup"],
        json!({"id":"fixture-save","created_at":"2026-11-20"})
    );
    assert_eq!(response["video"]["fps"], 0);
    assert_eq!(fs::read_to_string(&settings).unwrap(), config);
    assert_eq!(fs::read_to_string(obs.join("global.ini")).unwrap(), global);
    server.stop();
}

#[test]
fn round_recording_plan_uses_the_demo_file_end_instead_of_saved_tick_estimates() {
    let scratch = tempfile::tempdir().unwrap();
    let demo = scratch.path().join("old fixture.dem");
    let mut source = b"PBDEMS2\0\0\0\0\0\0\0\0\0".to_vec();
    source.extend([1, 244, 3, 0, 0, 255, 255, 255, 255, 15, 0]);
    fs::write(&demo, &source).unwrap();
    let data = scratch.path().join("data");
    let settings = scratch.path().join("config.json");
    let server = RunningServer::launch_with_config(
        "3.2.0-fixture",
        Some((&data, scratch.path())),
        Some(&settings),
    );
    let path = "/api/recording/plan";
    let mut request = json!({
        "request_id":"round probe","request_type":"round_compilation","source_type":"round",
        "demo":{
            "demo_path":demo,"demo_filename":"old fixture.dem","map_name":"de_fixture",
            "tick_rate":64.0,"first_tick":32,"demo_end_tick":2000, "final_round":1,
            "final_round_start_tick":32,"final_round_end_tick":495,
        },
        "target_player":{"name":"Player","steamid64":"76561198000000001","spec_slot":1},
        "rounds":[{
            "round":1,"round_start_tick":32,"round_end_tick":495,"freeze_end_tick":160,
        }],
    });
    assert_eq!(
        profile_request(&server, "POST", path, &request, false).0,
        401
    );
    let (code, plan) = profile_request(&server, "POST", path, &request, true);
    assert_eq!(code, 200);
    assert_eq!(plan["summary"]["active_count"], 1);
    assert_eq!(plan["summary"]["final_round_segments"], 1);
    let segment = &plan["active_segments"][0];
    assert_eq!(segment["start_tick"], 32);
    assert_eq!(segment["end_tick"], 404);
    assert_eq!(segment["safe_end_tick"], 404);
    assert_eq!(plan["estimated_duration_sec"], 5.8125);
    request["demo"]["first_tick"] = json!(500);
    assert_eq!(
        profile_request(&server, "POST", path, &request, true).0,
        422
    );
    assert_eq!(fs::read(demo).unwrap(), source);
    assert!(!settings.exists());
    server.stop();
}

#[test]
#[cfg(windows)]
fn recording_routes_keep_desktop_and_game_sessions_separate() {
    let scratch = tempfile::tempdir().unwrap();
    let profile = scratch.path().join("profile");
    let server = RunningServer::launch("3.2.0-fixture", Some((&profile, scratch.path())));
    let queue = "/api/recording/queue";
    let abort = "/api/recording/abort";
    let recording_gsi = "/api/gsi/recording";
    assert_eq!(
        profile_request(&server, "POST", queue, &json!({}), false).0,
        401
    );
    assert_eq!(
        profile_request(&server, "POST", queue, &json!({"requests":[]}), true).0,
        422
    );
    assert_eq!(
        profile_request(&server, "POST", abort, &Value::Null, false).0,
        401
    );
    assert_eq!(
        profile_request(&server, "POST", abort, &Value::Null, true),
        (200, json!({"status":"idle"}))
    );
    let arbitrary = json!({"provider":{"appid":730},"auth":{"token":"fixture-invalid-game-token"}});
    let (status, reply) = profile_request(&server, "POST", recording_gsi, &arbitrary, true);
    assert_eq!(status, 401);
    assert!(!reply.to_string().contains("fixture-invalid-game-token"));
    assert!(!profile.join(".cs2_config_backup").exists());
    assert!(!profile.join("cs2-insight.db").exists());
    server.stop();
}

#[cfg(windows)]
#[test]
#[ignore = "uses only a compiled synthetic playback process and trusted FFmpeg binaries"]
fn cancelled_recording_queue_restores_fixture_player_data() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_websockets::{Message, ServerBuilder};

    // Running this actual backend fixture on a Steam host could expose real
    // userdata to the recorder's ordinary configuration snapshot.
    let system = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap());
    for key in [
        r"HKLM\SOFTWARE\WOW6432Node\Valve\Steam",
        r"HKLM\SOFTWARE\Valve\Steam",
        r"HKCU\Software\Valve\Steam",
    ] {
        let registered = Command::new(system.join("System32/reg.exe"))
            .args(["query", key, "/v", "InstallPath"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("read-only Steam registration check");
        assert!(
            !registered.success(),
            "run this fixture on a Windows runner without Steam registration"
        );
    }

    let fixture_root =
        std::path::PathBuf::from(std::env::var_os("MGS_PLAYBACK_TEST_ROOT").unwrap());
    let ffmpeg = std::path::PathBuf::from(std::env::var_os("MGS_QUEUE_TEST_FFMPEG").unwrap());
    let scratch = tempfile::tempdir_in(fixture_root.as_path()).unwrap();
    let root = scratch.path();
    let game = root.join("game");
    let csgo = game.join("csgo");
    let cfg = csgo.join("cfg");
    let executable = game.join("bin/win64/cs2.exe");
    let recordings = root.join("recordings");
    fs::create_dir_all(&cfg).unwrap();
    fs::create_dir_all(executable.parent().unwrap()).unwrap();
    fs::create_dir(&recordings).unwrap();
    fs::copy(
        fixture_root.join("synthetic-playback-fixture.exe"),
        &executable,
    )
    .unwrap();
    let original = b"original fixture player.cfg\r\n";
    let player = cfg.join("cs2_user.cfg");
    fs::write(&player, original).unwrap();
    fs::write(cfg.join("unrelated.cfg"), b"leave alone").unwrap();
    let demo = root.join("original.dem");
    let mut original_demo = b"PBDEMS2\0\0\0\0\0\0\0\0\0".to_vec();
    original_demo.extend([1, 244, 3, 0, 0, 255, 255, 255, 255, 15, 0]);
    fs::write(&demo, &original_demo).unwrap();

    let obs_socket = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let obs_port = obs_socket.local_addr().unwrap().port();
    obs_socket.set_nonblocking(true).unwrap();
    let obs = thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async move {
                tokio::time::timeout(Duration::from_secs(15), async move {
                    let listener = tokio::net::TcpListener::from_std(obs_socket).unwrap();
                    let (stream, _) = listener.accept().await.unwrap();
                    let (_, mut wire) = ServerBuilder::new().accept(stream).await.unwrap();
                    wire.send(Message::text(
                        json!({"op":0,"d":{"rpcVersion":1}}).to_string(),
                    ))
                    .await
                    .unwrap();
                    let greeting = wire.next().await.unwrap().unwrap();
                    let greeting: Value =
                        serde_json::from_str(greeting.as_text().unwrap()).unwrap();
                    assert_eq!(greeting["d"]["eventSubscriptions"], 64);
                    wire.send(Message::text(
                        json!({"op":2,"d":{"negotiatedRpcVersion":1}}).to_string(),
                    ))
                    .await
                    .unwrap();
                    for (method, reply) in [
                        (
                            "GetRecordStatus",
                            json!({"outputActive":false,"outputPaused":false}),
                        ),
                        (
                            "GetCurrentProgramScene",
                            json!({"currentProgramSceneName":"CS2 Insight Recording"}),
                        ),
                        ("GetRecordDirectory", json!({"recordDirectory":recordings})),
                    ] {
                        let request = wire.next().await.unwrap().unwrap();
                        let request: Value =
                            serde_json::from_str(request.as_text().unwrap()).unwrap();
                        assert_eq!(request["d"]["requestType"], method);
                        wire.send(Message::text(
                            json!({"op":7,"d":{
                                "requestType":method,"requestId":request["d"]["requestId"],
                                "requestStatus":{"result":true,"code":100},"responseData":reply
                            }})
                            .to_string(),
                        ))
                        .await
                        .unwrap();
                    }
                    assert!(wire
                        .next()
                        .await
                        .and_then(Result::ok)
                        .and_then(|message| message.as_text().map(str::to_owned))
                        .is_none());
                })
                .await
                .expect("fixture OBS preflight and disconnect")
            })
    });

    let profile = root.join("profile");
    let settings = root.join("config.json");
    fs::create_dir(&profile).unwrap();
    let database = profile.join("cs2-insight.db");
    let old = rusqlite::Connection::open(&database).unwrap();
    old.execute_batch(
        "CREATE TABLE old_library_note(id INTEGER); INSERT INTO old_library_note VALUES(9);",
    )
    .unwrap();
    drop(old);
    let original_db = fs::read(&database).unwrap();
    fs::write(
        &settings,
        json!({
            "cs2_path":executable,"ffmpeg_path":ffmpeg,
            "obs":{"host":"127.0.0.1","port":obs_port,"password":""},
        })
        .to_string(),
    )
    .unwrap();
    let server =
        RunningServer::launch_with_config("3.2.0-fixture", Some((&profile, root)), Some(&settings));
    let mut payload = json!({"requests":[{
        "request_id":"round abort fixture","request_type":"round_compilation","source_type":"round",
        "demo":{
            "demo_path":demo,"demo_filename":"original.dem","map_name":"de_fixture",
            "tick_rate":64.0,"first_tick":32,"demo_end_tick":2000,"final_round":1,
            "final_round_start_tick":32,"final_round_end_tick":495
        },
        "target_player":{"name":"Fixture","steamid64":"76561198000000001","spec_slot":1},
        "rounds":[{"round":1,"round_start_tick":32,"round_end_tick":495,"freeze_end_tick":160}]
    }]});
    let mut pending = payload["requests"][0].clone();
    pending["request_id"] = json!("pending round fixture");
    payload["requests"].as_array_mut().unwrap().push(pending);
    thread::scope(|scope| {
        let recording = scope
            .spawn(|| profile_request(&server, "POST", "/api/recording/queue", &payload, true));
        let deadline = Instant::now() + Duration::from_secs(4);
        while fs::read(&player).unwrap() == original {
            assert!(
                Instant::now() < deadline,
                "synthetic player fixture did not start"
            );
            thread::sleep(Duration::from_millis(15));
        }
        assert_eq!(
            profile_request(
                &server,
                "POST",
                "/api/config-backup/restore",
                &Value::Null,
                true
            )
            .0,
            409
        );
        assert_eq!(
            profile_request(&server, "POST", "/api/recording/queue", &payload, true).0,
            409
        );
        assert_eq!(
            profile_request(&server, "POST", "/api/recording/abort", &Value::Null, true),
            (200, json!({"status":"ok"}))
        );
        let (status, reply) = recording.join().unwrap();
        assert_eq!(status, 200);
        assert_eq!(reply.as_array().unwrap().len(), 2);
        assert_eq!(reply[0]["aborted"], true, "{reply}");
        assert_eq!(reply[1]["request_id"], "pending round fixture");
        assert_eq!(reply[1]["aborted"], true);
        assert_eq!(reply[1]["error"], "recording_aborted");
    });
    assert_eq!(fs::read(&player).unwrap(), original);
    assert_eq!(fs::read(cfg.join("unrelated.cfg")).unwrap(), b"leave alone");
    assert_eq!(fs::read(&demo).unwrap(), original_demo);
    assert_eq!(fs::read(database).unwrap(), original_db);
    assert_eq!(fs::read_dir(&cfg).unwrap().count(), 2);
    assert_eq!(fs::read_dir(&csgo).unwrap().count(), 1);
    assert_eq!(
        profile_request(&server, "POST", "/api/recording/abort", &Value::Null, true),
        (200, json!({"status":"idle"}))
    );
    obs.join().unwrap();
    server.stop();
}

#[test]
fn uploads_lite_cut_browser_form_data_to_the_selected_asset_directory() {
    let scratch = tempfile::tempdir().unwrap();
    let profile = scratch.path().join("profile");
    let config = scratch.path().join("settings.json");
    let assets = scratch.path().join("素材目录");
    fs::write(&config, json!({"lite_cut_assets_dir":assets}).to_string()).unwrap();
    let server = RunningServer::launch_with_config(
        "3.2.0-fixture", Some((&profile, scratch.path())), Some(&config),
    );
    let (status, project) = profile_request(
        &server, "POST", "/api/lite-cut/projects", &json!({"name":"Fixture project"}), true,
    );
    assert_eq!(status, 200);
    let project_id = project["id"].as_i64().unwrap();
    let path = format!("/api/lite-cut/assets/upload?project_id={project_id}&client_duration_sec=1.25");
    let part = concat!(
        "--mgs-browser-boundary\r\n",
        "Content-Disposition: form-data; name=\"file\"; filename=\"test.mp4\"\r\n",
        "Content-Type: video/mp4\r\n\r\nfixture media bytes",
    );
    let upload = |path: &str, complete: bool, authorized: bool| {
        let ending = if complete { "\r\n--mgs-browser-boundary--\r\n" } else { "" };
        let auth = if authorized { "X-CS2-Insight-Token: synthetic-token\r\n" } else { "" };
        send_request(server.addr, &format!(
            "POST {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Type: multipart/form-data; boundary=mgs-browser-boundary\r\nContent-Length: {}\r\n{auth}\r\n{part}{ending}",
            server.addr, part.len() + ending.len(),
        ))
    };
    let method = profile_response(&server, "GET", &path, &Value::Null, true);
    assert_eq!(method.status, 405);
    assert_eq!(method.headers["allow"], "POST");
    assert_eq!(upload(&path, true, false).status, 401);
    assert_eq!(upload("/api/lite-cut/assets/upload?project_id=99999", true, true).status, 404);

    let mut paths = Vec::new();
    for _ in 0..2 {
        let response = upload(&path, true, true);
        assert_eq!(response.status, 200, "{}", response.body);
        let row: Value = serde_json::from_str(&response.body).unwrap();
        assert_eq!(row["project_id"], project_id);
        assert_eq!(row["kind"], "video");
        assert_eq!(row["name"], "test.mp4");
        assert_eq!(row["duration_sec"], 1.25);
        let saved = Path::new(row["file_path"].as_str().unwrap()).canonicalize().unwrap();
        assert!(saved.starts_with(assets.canonicalize().unwrap()));
        assert_eq!(fs::read(&saved).unwrap(), b"fixture media bytes");
        paths.push(saved);
    }
    assert_ne!(paths[0], paths[1]);
    assert_eq!(upload(&path, false, true).status, 400);
    assert_eq!(fs::read_dir(paths[0].parent().unwrap()).unwrap().count(), 2);
    let (status, listed) = profile_request(
        &server, "GET", &format!("/api/lite-cut/assets?project_id={project_id}"), &Value::Null, true,
    );
    assert_eq!(status, 200);
    assert_eq!(listed["items"].as_array().unwrap().len(), 2);
    server.stop();
}

#[test]
#[ignore = "requires the pinned FFmpeg and FFprobe fixture executables"]
fn lite_cut_generates_preview_and_waveform_for_an_existing_asset() {
    let scratch = tempfile::tempdir().unwrap();
    let ffmpeg = std::path::PathBuf::from(std::env::var_os("MGS_LITECUT_VIDEO_TEST_FFMPEG").unwrap());
    let assets = scratch.path().join("素材目录");
    fs::create_dir(&assets).unwrap();
    let source = assets.join("existing.mkv");
    let media = Command::new(&ffmpeg)
        .args([
            "-nostdin", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i",
            "color=c=blue:s=96x64:r=12:d=1", "-f", "lavfi", "-i",
            "sine=f=660:d=1", "-c:v", "libx264", "-pix_fmt", "yuv420p",
            "-c:a", "aac", "-shortest",
        ])
        .arg(&source)
        .output().unwrap();
    assert!(media.status.success(), "{}", String::from_utf8_lossy(&media.stderr));
    let original = fs::read(&source).unwrap();
    fs::write(scratch.path().join("custom-config.json"), json!({
        "lite_cut_assets_dir":assets,"ffmpeg_path":ffmpeg
    }).to_string()).unwrap();
    let database = rusqlite::Connection::open(scratch.path().join("cs2-insight.db")).unwrap();
    database.execute_batch(
        "CREATE TABLE lite_cut_assets(id INTEGER PRIMARY KEY, project_id INTEGER, name TEXT,
             kind TEXT, mime_type TEXT, file_path TEXT, duration_sec REAL, created_at TEXT);",
    ).unwrap();
    database.execute(
        "INSERT INTO lite_cut_assets VALUES (1,NULL,'Existing','video','video/x-matroska',?1,1.0,'2026-11-20')",
        [source.to_string_lossy().as_ref()],
    ).unwrap();
    drop(database);

    let server = RunningServer::launch("3.2.0-fixture", Some((scratch.path(), scratch.path())));
    let stream = "/api/lite-cut/assets/1/stream";
    let pending = profile_response(&server, "GET", stream, &Value::Null, true);
    assert_eq!(pending.status, 425);
    assert_eq!(pending.headers["retry-after"], "1");
    let deadline = Instant::now() + Duration::from_secs(20);
    let ready = loop {
        let (status, listed) = profile_request(&server, "GET", "/api/lite-cut/assets", &Value::Null, true);
        assert_eq!(status, 200);
        let asset = &listed["items"][0];
        if asset["preview_proxy_status"] == "ready" {
            break asset.clone();
        }
        assert!(asset["preview_proxy_status"] != "failed" && Instant::now() < deadline, "{asset}");
        thread::sleep(Duration::from_millis(100));
    };
    let version = ready["preview_proxy_version"].as_str().unwrap();
    let range = send_request(server.addr, &format!(
        "GET {stream}?v={version} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nX-CS2-Insight-Token: synthetic-token\r\nRange: bytes=4-7\r\n\r\n", server.addr
    ));
    assert_eq!(range.status, 206);
    assert_eq!(range.body, "ftyp");
    let retry = profile_request(&server, "POST", "/api/lite-cut/assets/1/proxy/retry", &Value::Null, true);
    assert_eq!(retry, (200, ready));

    let waveform = "/api/lite-cut/assets/1/waveform?buckets=8";
    let (status, mut first) = profile_request(&server, "GET", waveform, &Value::Null, true);
    assert_eq!(status, 200, "{first}");
    assert_eq!(first["cached"], false);
    assert_eq!(first["peaks"].as_array().unwrap().len(), 8);
    first["cached"] = json!(true);
    assert_eq!(profile_request(&server, "GET", waveform, &Value::Null, true), (200, first));
    assert_eq!(fs::read(&source).unwrap(), original);
    assert!(source.with_extension("preview60.mp4").is_file());
    assert!(source.with_extension("waveform-v1.json").is_file());
    assert_eq!(fs::read_dir(assets).unwrap().count(), 3);
    let (status, storage) = profile_request(&server, "GET", "/api/lite-cut/storage", &Value::Null, true);
    assert_eq!(status, 200, "{storage}");
    assert_eq!(storage["path"], resolved_fixture_path(source.parent().unwrap()));
    assert_eq!(storage["custom"], true);
    let proxy = "/api/lite-cut/proxy-cache";
    let (status, cache) = profile_request(&server, "GET", proxy, &Value::Null, true);
    assert_eq!(status, 200, "{cache}");
    assert_eq!(cache["proxy_files"], 1);
    assert_eq!(cache["proxy_bytes"], fs::metadata(source.with_extension("preview60.mp4")).unwrap().len());
    let settings = "/api/lite-cut/proxy-cache/settings";
    let resolution = json!({"resolution":723});
    assert_eq!(profile_request(&server, "PATCH", settings, &resolution, false).0, 401);
    assert_eq!(profile_request(&server, "PATCH", settings, &resolution, true), (200, json!({"resolution":724})));
    assert_eq!(profile_request(&server, "GET", proxy, &Value::Null, true).1["resolution"], 724);
    let deletion = "/api/lite-cut/assets/1";
    let incorrect = profile_response(&server, "GET", deletion, &Value::Null, true);
    assert_eq!((incorrect.status, incorrect.headers["allow"].as_str()), (405, "DELETE"));
    assert_eq!(profile_response(&server, "DELETE", deletion, &Value::Null, false).status, 401);
    let (status, result) = profile_request(&server, "DELETE", deletion, &Value::Null, true);
    assert_eq!(status, 200, "{result}");
    assert_eq!(result["deleted"], true);
    assert_eq!(result["id"], 1);
    assert!(!source.exists());
    let recovery = Path::new(result["recovery_directory"].as_str().unwrap());
    assert!(recovery.join("manifest.json").is_file());
    assert!(fs::read_dir(recovery.join("files")).unwrap().any(|entry| {
        fs::read(entry.unwrap().path()).is_ok_and(|content| content == original)
    }));
    assert_eq!(profile_request(&server, "DELETE", deletion, &Value::Null, true).0, 404);
    server.stop();
}

#[test]
fn opens_saved_lite_cut_projects_snapshots_and_exports_in_the_existing_profile() {
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
    let exports = "/api/lite-cut/exports";
    let get = |path, auth| profile_request(&server, "GET", path, &Value::Null, auth);
    assert_eq!(get(list, false).0, 401);
    assert_eq!(
        get(list, true),
        (200, json!({"items":[],"limit":100,"offset":0}))
    );
    assert_eq!(
        get(exports, true),
        (200, json!({"items":[],"limit":20,"offset":0}))
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
         INSERT INTO lite_cut_project_snapshots VALUES (4,7,'旧工程','{}','before_restore','2026-11-20');
         CREATE TABLE lite_cut_exports (
            id INTEGER PRIMARY KEY AUTOINCREMENT, project_id INTEGER, body_json TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending', error_msg TEXT, output_path TEXT,
            created_at TEXT NOT NULL, updated_at TEXT NOT NULL
         );
         INSERT INTO lite_cut_exports VALUES (10,7,'{}','done',NULL,'D:/历史/视频.mp4','2026-11-18','2026-11-20');
         INSERT INTO lite_cut_exports VALUES (11,6,'{}','error','MONTAGE_EXPORT_FAILED',NULL,'2026-11-20','2026-11-21');",
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
    let completed = json!({
        "export_id":10,"project_id":7,"status":"done","progress":1.0,"stage":"done",
        "output_path":"D:/历史/视频.mp4","error":"","created_at":"2026-11-18","updated_at":"2026-11-20",
    });
    assert_eq!(get("/api/lite-cut/exports/10", false).0, 401);
    assert_eq!(
        get("/api/lite-cut/exports/10", true),
        (200, completed.clone())
    );
    assert_eq!(
        get("/api/lite-cut/exports?project_id=7&limit=1", true),
        (200, json!({"items":[completed],"limit":1,"offset":0})),
    );
    assert_eq!(
        get("/api/lite-cut/exports?limit=1", true).1["items"][0]["error"],
        "MONTAGE_EXPORT_FAILED"
    );
    assert_eq!(
        get("/api/lite-cut/exports/99", true),
        (404, json!({"detail":{"code":"LITECUT_EXPORT_NOT_FOUND"}})),
    );
    assert_eq!(
        [fs::read(&database).unwrap(), fs::read(&wal).unwrap()],
        before
    );
    server.stop();
}

#[test]
fn lite_cut_asset_validation_reports_only_media_needed_for_export() {
    let scratch = tempfile::tempdir().unwrap();
    let data = scratch.path().join("data");
    let bundle = scratch.path().join("bundle");
    let visible = scratch.path().join("缺少的视频.mp4");
    let missing_audio = scratch.path().join("音乐.wav");
    let missing_image = scratch.path().join("画面.png");
    fs::create_dir(&bundle).unwrap();
    let server = RunningServer::launch("3.2.0-fixture", Some((&data, &bundle)));
    let route = "/api/lite-cut/assets/validate";
    let input = json!({"body":{
        "tracks":[
            {"id":"top","label":"Superimposed","type":"video","clips":[
                {"id":"image","source_type":"file","file_path":missing_image},
            ]},
            {"id":"base","label":"Base","type":"video","clips":[
                {"id":"visible","source_type":"file","file_path":visible},
                {"id":"recorded","source_id":29,"timeline_start":4},
            ]},
            {"id":"hidden","label":"Hidden","type":"video","hidden":true,"clips":[
                {"id":"not_rendered","source_type":"file","file_path":"absent-but-hidden.mp4"},
            ]},
            {"id":"a1","label":"Audio","type":"audio","solo":true,"clips":[
                {"id":"audio","source_type":"file","file_path":missing_audio},
            ]},
            {"id":"a2","label":"Audio 2","type":"audio","clips":[
                {"id":"silent","source_type":"file","file_path":"not-in-solo.wav"},
            ]},
        ],
        "overlay_tracks":[{"id":"blocked","hidden":true}],
        "overlays":[
            {"id":"repeat","type":"sticker","asset_path":missing_image},
            {"id":"invisible","type":"sticker","asset_path":"hidden.png","meta":{"overlay_track_id":"blocked"}},
        ],
    }});
    assert_eq!(
        profile_request(&server, "POST", route, &input, false).0,
        401
    );
    assert_eq!(
        profile_request(&server, "POST", route, &input, true),
        (
            200,
            json!({"items":[
                {"kind":"video","name":"缺少的视频.mp4","path":visible},
                {"kind":"overlay","name":"画面.png","path":missing_image},
                {"kind":"audio","name":"音乐.wav","path":missing_audio},
                {"kind":"recording","name":"Insight recording #29","path":"","source_id":29},
            ]})
        ),
    );
    fs::write(&visible, b"test media").unwrap();
    fs::write(&missing_image, b"test media").unwrap();
    fs::write(&missing_audio, b"test media").unwrap();
    assert_eq!(
        profile_request(&server, "POST", route, &input, true).1["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let database = data.join("cs2-insight.db");
    assert!(!database.exists());
    fs::create_dir_all(&data).unwrap();
    let db = rusqlite::Connection::open(&database).unwrap();
    db.execute_batch(
        "CREATE TABLE recorded_clips (
             id INTEGER PRIMARY KEY, clip_id TEXT, demo_path TEXT, demo_filename TEXT, player_name TEXT,
             output_path TEXT, duration_sec REAL, status TEXT, created_at TEXT, clip_meta TEXT
         );",
    ).unwrap();
    db.execute(
        "INSERT INTO recorded_clips (id, output_path, clip_meta) VALUES (29, 'old-location.mp4', ?1)",
        [json!({"output_path": visible}).to_string()],
    ).unwrap();
    let database_before = fs::read(&database).unwrap();
    assert_eq!(
        profile_request(&server, "POST", route, &input, true),
        (200, json!({"items":[]})),
    );
    assert_eq!(fs::read(&database).unwrap(), database_before);
    server.stop();
}

#[test]
fn saves_existing_lite_cut_projects_and_restores_a_snapshot_atomically() {
    let scratch = tempfile::tempdir().unwrap();
    let db = scratch.path().join("cs2-insight.db");
    let settings = scratch.path().join("settings.json");
    let data = scratch.path().join("data");
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE lite_cut_projects (
             id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL DEFAULT '', body_json TEXT NOT NULL,
             created_at TEXT NOT NULL, updated_at TEXT NOT NULL
         );
         INSERT INTO lite_cut_projects VALUES(1,'Straße','{}','2026-11-18','2026-11-18');
         CREATE TABLE custom_profile_metadata(value TEXT);
         INSERT INTO custom_profile_metadata VALUES('keep');",
    ).unwrap();
    let server = RunningServer::launch_with_config(
        "3.2.0-fixture",
        Some((&data, scratch.path())),
        Some(&settings),
    );
    let request = |method: &str, route: &str, body: &Value, auth: bool| {
        profile_request(&server, method, route, body, auth)
    };
    let create = json!({"name":"  STRASSE  "});
    assert_eq!(
        request("POST", "/api/lite-cut/projects", &create, false).0,
        401
    );
    let (code, new_project) = request("POST", "/api/lite-cut/projects", &create, true);
    assert_eq!(code, 200);
    assert_eq!(new_project["name"], "STRASSE (1)");
    assert_eq!(new_project["body"]["schema_version"], 2);
    assert_eq!(new_project["body"]["tracks"].as_array().unwrap().len(), 2);
    assert_eq!(new_project["created_at"], new_project["updated_at"]);

    let route = "/api/lite-cut/projects/1";
    let history = "/api/lite-cut/projects/1/snapshots";
    let save = json!({
        "name":"保留工程", "body":{"tracks":[{"id":"track-a","type":"video","label":"历史片段",
        "clips":[{"id":"clip-a","trim_in":2.5,"trim_out":4.0}]}]}
    });
    let (code, first) = request("PATCH", route, &save, true);
    assert_eq!(code, 200);
    assert_eq!(first["name"], "保留工程");
    assert_eq!(first["created_at"], "2026-11-18");
    assert_eq!(first["body"]["tracks"][0]["clips"][0]["trim_in"], 2.5);
    assert_eq!(request("PATCH", route, &save, true).0, 200);
    let (_, before_failure) = request("GET", history, &Value::Null, true);
    assert_eq!(before_failure["items"].as_array().unwrap().len(), 1);
    let first_snapshot = before_failure["items"][0]["id"].as_i64().unwrap();
    assert_eq!(
        request(
            "PATCH",
            route,
            &json!({"body":{"output":{"width":2}}}),
            true
        )
        .0,
        422
    );
    assert_eq!(
        request("GET", history, &Value::Null, true).1,
        before_failure
    );

    let (code, second) = request(
        "PATCH",
        route,
        &json!({"body":{"output":{"width":1280}}}),
        true,
    );
    assert_eq!(code, 200);
    assert_eq!(second["body"]["output"]["width"], 1280);
    let target = format!("{history}/{first_snapshot}/restore");
    let (code, restored) = request("POST", &target, &Value::Null, true);
    assert_eq!(code, 200);
    assert_eq!(restored["body"], first["body"]);
    assert_eq!(restored["name"], "保留工程");
    assert_eq!(request("GET", route, &Value::Null, true).1, restored);
    let other_profile: String = connection
        .query_row("SELECT value FROM custom_profile_metadata", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(other_profile, "keep");
    server.stop();
}

#[test]
fn streams_existing_lite_cut_media_and_its_saved_proxy_from_selected_storage() {
    let scratch = tempfile::tempdir().unwrap();
    let data = scratch.path().join("data");
    let assets = scratch.path().join("自定义素材");
    fs::create_dir(&assets).unwrap();
    let settings = scratch.path().join("settings.json");
    fs::write(&settings, json!({"lite_cut_assets_dir":assets}).to_string()).unwrap();
    let outside = scratch.path().join("elsewhere.mp4");
    let original = assets.join("original.mov");
    let proxy = assets.join("original.preview-alpha-v3.webm");
    let image = assets.join("thumb.png");
    let header = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\x01\x40\0\0\0\xb4";
    fs::write(&original, b"old motion source").unwrap();
    fs::write(&proxy, b"old preview cache").unwrap();
    fs::write(&outside, b"outside storage").unwrap();
    fs::write(&image, header).unwrap();

    let db = scratch.path().join("cs2-insight.db");
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE lite_cut_assets (
             id INTEGER PRIMARY KEY, project_id INTEGER, name TEXT NOT NULL, kind TEXT NOT NULL,
             mime_type TEXT, file_path TEXT NOT NULL, duration_sec REAL, created_at TEXT NOT NULL
         );",
        )
        .unwrap();
    for (id, project, name, path) in [
        (1, Some(7), "旧素材", &original),
        (2, Some(8), "另一工程", &original),
        (3, None, "外部素材", &outside),
    ] {
        connection.execute(
            "INSERT INTO lite_cut_assets VALUES(?1, ?2, ?3, 'video', NULL, ?4, 2.5, '2026-11-20')",
            rusqlite::params![id, project, name, path.to_str().unwrap()],
        ).unwrap();
    }
    connection.execute(
        "INSERT INTO lite_cut_assets VALUES(4, 8, '旧图', 'image', 'image/png', ?1, NULL, '2026-11-20')",
        [image.to_str().unwrap()],
    ).unwrap();
    let old_db = fs::read(&db).unwrap();
    let old_settings = fs::read(&settings).unwrap();
    let server = RunningServer::launch_with_config(
        "3.2.0-fixture",
        Some((&data, scratch.path())),
        Some(&settings),
    );
    let (status, page) = profile_request(
        &server,
        "GET",
        "/api/lite-cut/assets?project_id=7",
        &Value::Null,
        true,
    );
    assert_eq!(status, 200);
    let items = page["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    let material = items.iter().find(|entry| entry["id"] == 1).unwrap();
    assert_eq!(material["preview_proxy_status"], "ready");
    assert_eq!(material["has_alpha"], true);
    assert_eq!(material["width"], Value::Null);
    assert_eq!(material["file_path"], original.to_str().unwrap());
    assert_eq!(page["limit"], 500);
    assert_eq!(
        profile_request(
            &server,
            "GET",
            "/api/lite-cut/assets/1/stream",
            &Value::Null,
            false
        )
        .0,
        401,
    );
    assert_eq!(
        profile_request(
            &server,
            "GET",
            "/api/lite-cut/assets/3/stream",
            &Value::Null,
            true
        ),
        (403, json!({"detail":"asset path outside storage"})),
    );
    let response = send_request(server.addr, &format!(
        "GET /api/lite-cut/assets/1/stream?_session=synthetic-token HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nRange: bytes=4-10\r\n\r\n",
        server.addr,
    ));
    assert_eq!(response.status, 206);
    assert_eq!(response.headers["content-type"], "video/webm");
    assert_eq!(response.headers["content-range"], "bytes 4-10/17");
    assert_eq!(response.body, "preview");
    let metadata = profile_request(
        &server,
        "GET",
        "/api/lite-cut/assets/4/metadata",
        &Value::Null,
        true,
    );
    assert_eq!(
        metadata,
        (
            200,
            json!({
                "id":4,"kind":"image","name":"旧图","extension":"PNG","mime_type":"image/png",
                "duration_sec":null,"width":320,"height":180,"fps":null,"codec_name":null,"has_audio":null,
            })
        )
    );
    assert_eq!(
        profile_request(
            &server,
            "GET",
            "/api/lite-cut/assets/3/metadata",
            &Value::Null,
            true
        )
        .0,
        403,
    );
    assert_eq!(fs::read(db).unwrap(), old_db);
    assert_eq!(fs::read(settings).unwrap(), old_settings);
    assert_eq!(fs::read(original).unwrap(), b"old motion source");
    assert_eq!(fs::read(proxy).unwrap(), b"old preview cache");
    assert_eq!(fs::read(image).unwrap(), header);
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
                json!({"valid":true,"path":resolved_fixture_path(&root),"demo_count":demos,"zip_count":zips,"max_depth":depth})
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
