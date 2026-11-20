//! Opt-in loopback listener for the native health and CS2 readiness routes.

use crate::{gsi_ready::GsiReady, session_auth::strip_session_query};
use futures_util::future::{select, Either};
use http_body_util::{BodyExt, Full, LengthLimitError, Limited};
use hyper::{
    body::{Body, Bytes, Incoming},
    header::{
        HeaderValue, ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
        ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_MAX_AGE, ACCESS_CONTROL_REQUEST_HEADERS,
        ACCESS_CONTROL_REQUEST_METHOD, ALLOW, CONTENT_TYPE, LOCATION, ORIGIN, VARY,
    },
    server::conn::http1,
    service::service_fn,
    Method, Request, Response, StatusCode,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use serde_json::{json, Value};
use std::{
    convert::Infallible,
    future::Future,
    io,
    net::{Ipv4Addr, TcpListener},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{net::TcpStream, sync::watch, task::JoinSet, time::timeout};

pub const DEFAULT_PORT: u16 = 19871;
const MAX_CONNECTIONS: usize = 16;
const MAX_HEADER_BYTES: usize = 32 * 1024;
const MAX_GSI_BYTES: usize = 1024 * 1024;
const CONNECTION_LIMIT: Duration = Duration::from_secs(10);
const DRAIN_LIMIT: Duration = Duration::from_secs(1);
const ALLOWED_ORIGINS: &[&str] = &[
    "http://tauri.localhost",
    "https://tauri.localhost",
    "tauri://localhost",
    "http://localhost:5173",
    "http://127.0.0.1:5173",
];
const PREFLIGHT_METHODS: &[&str] = &["DELETE", "GET", "HEAD", "OPTIONS", "PATCH", "POST", "PUT"];

type HttpResponse = Response<Full<Bytes>>;

struct State {
    version: String,
    backend_url: String,
    started: Instant,
    gsi: GsiReady,
}

pub struct NativeHttpServer {
    listener: TcpListener,
    state: Arc<State>,
}

impl NativeHttpServer {
    pub fn bind(port: u16, version: String) -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
        listener.set_nonblocking(true)?;
        let backend_url = format!("http://{}", listener.local_addr()?);
        Ok(Self {
            listener,
            state: Arc::new(State {
                version,
                backend_url,
                started: Instant::now(),
                gsi: GsiReady::default(),
            }),
        })
    }

    pub fn backend_url(&self) -> &str {
        &self.state.backend_url
    }

    /// Run on a Tokio runtime with I/O and time enabled, until shutdown resolves.
    pub async fn serve(self, shutdown: impl Future<Output = ()>) -> io::Result<()> {
        let listener = tokio::net::TcpListener::from_std(self.listener)?;
        let (stop_connections, _) = watch::channel(false);
        let mut connections = JoinSet::new();
        let mut shutdown = Box::pin(shutdown);
        let result = loop {
            let stream = match select(shutdown.as_mut(), Box::pin(listener.accept())).await {
                Either::Left(_) => break Ok(()),
                Either::Right((Ok((stream, _)), _)) => stream,
                Either::Right((Err(error), _)) => match error.kind() {
                    io::ErrorKind::Interrupted
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::ConnectionReset => continue,
                    _ => break Err(error),
                },
            };
            while connections.try_join_next().is_some() {}
            if connections.len() < MAX_CONNECTIONS {
                connections.spawn(serve_connection(
                    stream,
                    Arc::clone(&self.state),
                    stop_connections.subscribe(),
                ));
            }
        };
        drop(listener);
        let _ = stop_connections.send(true);
        let drain = async { while connections.join_next().await.is_some() {} };
        if timeout(DRAIN_LIMIT, drain).await.is_err() {
            connections.shutdown().await;
        }
        result
    }
}

async fn serve_connection(
    stream: TcpStream,
    state: Arc<State>,
    mut shutdown: watch::Receiver<bool>,
) {
    let service = service_fn(move |request| {
        let state = Arc::clone(&state);
        async move { Ok::<_, Infallible>(respond(&state, request).await) }
    });
    let mut connection = Box::pin(
        http1::Builder::new()
            .timer(TokioTimer::new())
            .header_read_timeout(CONNECTION_LIMIT)
            .max_buf_size(MAX_HEADER_BYTES)
            .keep_alive(false)
            .serve_connection(TokioIo::new(stream), service),
    );
    let serve = async {
        let stopping = matches!(
            select(connection.as_mut(), Box::pin(shutdown.changed())).await,
            Either::Right(_)
        );
        if stopping {
            connection.as_mut().graceful_shutdown();
            let _ = connection.await;
        }
    };
    let _ = timeout(CONNECTION_LIMIT, serve).await;
}

async fn respond(state: &State, request: Request<Incoming>) -> HttpResponse {
    let origin = request.headers().get(ORIGIN).cloned();
    if let (Some(origin), Some(method)) = (
        origin
            .as_ref()
            .filter(|_| request.method() == Method::OPTIONS),
        request.headers().get(ACCESS_CONTROL_REQUEST_METHOD),
    ) {
        return preflight(origin, method, request.headers());
    }
    let mut response = route(state, request).await;
    if let Some(origin) = origin.filter(allowed_origin) {
        response
            .headers_mut()
            .insert(ACCESS_CONTROL_ALLOW_ORIGIN, origin);
        response
            .headers_mut()
            .insert(VARY, HeaderValue::from_static("Origin"));
    }
    response
}

fn allowed_origin(header: &HeaderValue) -> bool {
    ALLOWED_ORIGINS
        .iter()
        .any(|origin| header.as_bytes() == origin.as_bytes())
}

fn preflight(
    origin: &HeaderValue,
    method: &HeaderValue,
    headers: &hyper::HeaderMap,
) -> HttpResponse {
    let mut failures = Vec::new();
    if !allowed_origin(origin) {
        failures.push("origin");
    }
    if !PREFLIGHT_METHODS
        .iter()
        .any(|allowed| method.as_bytes() == allowed.as_bytes())
    {
        failures.push("method");
    }
    if headers.contains_key("access-control-request-private-network") {
        failures.push("private-network");
    }
    let (status, text) = if failures.is_empty() {
        (StatusCode::OK, "OK".to_owned())
    } else {
        (
            StatusCode::BAD_REQUEST,
            format!("Disallowed CORS {}", failures.join(", ")),
        )
    };
    let mut response = reply(status, "text/plain; charset=utf-8", text);
    let outgoing = response.headers_mut();
    outgoing.insert(VARY, HeaderValue::from_static("Origin"));
    outgoing.insert(
        ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("DELETE, GET, HEAD, OPTIONS, PATCH, POST, PUT"),
    );
    outgoing.insert(ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("600"));
    if !failures.contains(&"origin") {
        outgoing.insert(ACCESS_CONTROL_ALLOW_ORIGIN, origin.clone());
    }
    if let Some(request_headers) = headers.get(ACCESS_CONTROL_REQUEST_HEADERS) {
        outgoing.insert(ACCESS_CONTROL_ALLOW_HEADERS, request_headers.clone());
    }
    response
}

enum Route {
    Health,
    Gsi,
    GsiStatus,
}

impl Route {
    fn from_path(path: &str) -> Option<Self> {
        match path {
            "/api/health" => Some(Self::Health),
            "/api/gsi/cs2" => Some(Self::Gsi),
            "/api/gsi/status" => Some(Self::GsiStatus),
            _ => None,
        }
    }
}

async fn route(state: &State, request: Request<Incoming>) -> HttpResponse {
    let path = request.uri().path();
    let Some(route) = Route::from_path(path) else {
        if let Some(canonical) = path
            .strip_suffix('/')
            .filter(|path| Route::from_path(path).is_some())
        {
            let mut location = format!("{}{canonical}", state.backend_url);
            if let Some(query) = request.uri().query() {
                let query = strip_session_query(query.as_bytes());
                if !query.is_empty() {
                    location.push('?');
                    location.push_str(&String::from_utf8_lossy(&query));
                }
            }
            return Response::builder()
                .status(StatusCode::TEMPORARY_REDIRECT)
                .header(LOCATION, location)
                .body(Full::new(Bytes::new()))
                .expect("loopback redirect");
        }
        return detail(StatusCode::NOT_FOUND, "Not Found");
    };
    // All three routes are guest routes in the deployed backend. CS2 sends no
    // desktop session header and the status check is also used by the web UI.
    let allowed = if matches!(route, Route::Gsi) {
        "POST"
    } else {
        "GET"
    };
    if request.method().as_str() != allowed {
        let mut response = detail(StatusCode::METHOD_NOT_ALLOWED, "Method Not Allowed");
        response
            .headers_mut()
            .insert(ALLOW, HeaderValue::from_static(allowed));
        return response;
    }
    match route {
        Route::Health => json_reply(
            StatusCode::OK,
            json!({"status": "ok", "version": state.version}),
        ),
        Route::GsiStatus => json_reply(StatusCode::OK, json!(state.gsi.status())),
        Route::Gsi => notify_gsi(state, request).await,
    }
}

fn is_json(header: Option<&HeaderValue>) -> bool {
    let mime = header
        .and_then(|header| header.to_str().ok())
        .unwrap_or_default();
    let mime = mime.split(';').next().unwrap_or_default().trim();
    mime.split_once('/').is_some_and(|(kind, subtype)| {
        let subtype = subtype.to_ascii_lowercase();
        kind.eq_ignore_ascii_case("application")
            && (subtype == "json" || subtype.ends_with("+json"))
    })
}

async fn notify_gsi(state: &State, request: Request<Incoming>) -> HttpResponse {
    let is_json = is_json(request.headers().get(CONTENT_TYPE));
    let body = request.into_body();
    if body
        .size_hint()
        .upper()
        .is_some_and(|size| size > MAX_GSI_BYTES as u64)
    {
        return oversized();
    }
    let bytes = match Limited::new(body, MAX_GSI_BYTES).collect().await {
        Ok(body) => body.to_bytes(),
        Err(error) if error.is::<LengthLimitError>() => return oversized(),
        Err(_) => {
            return detail(
                StatusCode::BAD_REQUEST,
                "There was an error parsing the body",
            )
        }
    };
    let payload = if bytes.is_empty() {
        json!({})
    } else {
        let Ok(text) = std::str::from_utf8(&bytes) else {
            return detail(
                StatusCode::BAD_REQUEST,
                "There was an error parsing the body",
            );
        };
        if !is_json {
            return invalid_dictionary(json!(text));
        }
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        match serde_json::from_str::<Value>(text) {
            Ok(value @ Value::Object(_)) => value,
            Ok(Value::Null) => json!({}),
            Ok(value) => return invalid_dictionary(value),
            Err(error) => return invalid_json(text, &error),
        }
    };
    let ready = state
        .gsi
        .notify(payload, state.started.elapsed().as_secs_f64());
    json_reply(StatusCode::OK, json!({"ok": true, "ready": ready}))
}

fn oversized() -> HttpResponse {
    detail(
        StatusCode::PAYLOAD_TOO_LARGE,
        "GSI request body is too large",
    )
}

fn invalid_dictionary(input: Value) -> HttpResponse {
    json_reply(
        StatusCode::UNPROCESSABLE_ENTITY,
        json!({"detail": [{
            "type": "dict_type", "loc": ["body"], "msg": "Input should be a valid dictionary", "input": input
        }]}),
    )
}

fn invalid_json(text: &str, error: &serde_json::Error) -> HttpResponse {
    let offset = if error.is_eof() {
        text.chars().count()
    } else {
        let before_line: usize = text
            .split_inclusive('\n')
            .take(error.line().saturating_sub(1))
            .map(str::len)
            .sum();
        let byte_offset = (before_line + error.column().saturating_sub(1)).min(text.len());
        text.char_indices()
            .take_while(|(index, _)| *index < byte_offset)
            .count()
    };
    let message = error.to_string();
    let description = if message.starts_with("trailing characters") {
        "Extra data"
    } else if message.starts_with("key must be a string") {
        "Expecting property name enclosed in double quotes"
    } else if message.starts_with("expected :") || message.starts_with("expected `:`") {
        "Expecting ':' delimiter"
    } else if message.starts_with("expected `,` or") {
        "Expecting ',' delimiter"
    } else {
        "Expecting value"
    };
    json_reply(
        StatusCode::UNPROCESSABLE_ENTITY,
        json!({"detail": [{
            "type": "json_invalid", "loc": ["body", offset], "msg": "JSON decode error",
            "input": {}, "ctx": {"error": description}
        }]}),
    )
}

fn detail(status: StatusCode, text: &str) -> HttpResponse {
    json_reply(status, json!({"detail": text}))
}

fn json_reply(status: StatusCode, value: Value) -> HttpResponse {
    reply(status, "application/json", value.to_string())
}

fn reply(status: StatusCode, content_type: &'static str, body: String) -> HttpResponse {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .body(Full::new(Bytes::from(body)))
        .expect("valid response")
}
