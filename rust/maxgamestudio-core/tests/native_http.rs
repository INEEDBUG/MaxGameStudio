#![cfg(feature = "native-http")]

use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream},
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
        let child = Command::new(env!("CARGO_BIN_EXE_maxgamestudio-native-http"))
            .args(["--port", "0", "--app-version", version])
            .env("CS2_INSIGHT_SESSION_TOKEN", "synthetic-token")
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
