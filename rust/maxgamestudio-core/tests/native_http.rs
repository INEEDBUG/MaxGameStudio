#![cfg(feature = "native-http")]

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
        let mut command = Command::new(env!("CARGO_BIN_EXE_maxgamestudio-native-http"));
        command
            .args(["--port", "0", "--app-version", version])
            .env("CS2_INSIGHT_SESSION_TOKEN", "synthetic-token")
            .env_remove("CS2_INSIGHT_DATA_DIR")
            .env_remove("CS2_INSIGHT_CONFIG")
            .env_remove("CS2_INSIGHT_BUNDLE_DATA_DIR");
        if let Some((data, bundle)) = profile {
            command
                .env("CS2_INSIGHT_DATA_DIR", data)
                .env("CS2_INSIGHT_CONFIG", data.join("custom-config.json"))
                .env("CS2_INSIGHT_BUNDLE_DATA_DIR", bundle)
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
    let response = send_request(server.addr, &request);
    (
        response.status,
        serde_json::from_str(&response.body).unwrap(),
    )
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
