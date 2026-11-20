//! Opt-in loopback listener for native health, CS2 readiness and desktop configuration.

mod demo_query;

use crate::{
    demo_library::{DemoLibrary, DemoReadError},
    gsi_ready::GsiReady,
    profile_config::{ConfigUpdateError, ProfileConfig},
    profile_platform::{backup_status, data_dir_info, path_text, resolved_path},
    runtime_session::RuntimeSession,
    session_auth::{strip_session_query, SessionAuth, SESSION_TOKEN_HEADER},
    wire_json::json_decode_error,
};
use demo_query::DemoRoute;
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
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    net::TcpStream,
    sync::{watch, Semaphore},
    task::JoinSet,
    time::timeout,
};

pub const DEFAULT_PORT: u16 = 19871;
const MAX_CONNECTIONS: usize = 16;
const MAX_HEADER_BYTES: usize = 32 * 1024;
const MAX_BODY_BYTES: usize = 1024 * 1024;
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
    profile: Option<Arc<ProfileState>>,
}

/// Paths and identity owned by the desktop launcher. Bundle data is read-only.
pub struct NativeProfile {
    pub data_dir: PathBuf,
    pub config_path: Option<PathBuf>,
    pub bundle_data_dir: Option<PathBuf>,
    pub legacy_config_path: Option<PathBuf>,
    pub instance_id: String,
    pub session_token: String,
}

struct ProfileState {
    config: ProfileConfig,
    auth: SessionAuth,
    instance_id: String,
    session: RuntimeSession,
    disk: Arc<Semaphore>,
}

pub struct NativeHttpServer {
    listener: TcpListener,
    state: Arc<State>,
}

impl NativeHttpServer {
    pub fn bind(port: u16, version: String) -> io::Result<Self> {
        Self::bind_inner(port, version, None)
    }

    pub fn bind_profile(port: u16, version: String, profile: NativeProfile) -> io::Result<Self> {
        if profile.session_token.trim().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "desktop session token is required",
            ));
        }
        let state = ProfileState {
            config: ProfileConfig::new(
                profile.data_dir,
                profile.config_path,
                profile.bundle_data_dir,
                profile.legacy_config_path,
            )?,
            auth: SessionAuth::new(&profile.session_token),
            instance_id: profile.instance_id.trim().to_owned(),
            session: RuntimeSession::default(),
            disk: Arc::new(Semaphore::new(1)),
        };
        Self::bind_inner(port, version, Some(Arc::new(state)))
    }

    fn bind_inner(
        port: u16,
        version: String,
        profile: Option<Arc<ProfileState>>,
    ) -> io::Result<Self> {
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
                profile,
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
    Config,
    QuickCheck,
    FfmpegCheck,
    DataDirInfo,
    BackupStatus,
    RuntimeState,
    Demos(DemoRoute),
}

impl Route {
    fn from_path(path: &str, has_profile: bool) -> Option<Self> {
        match path {
            "/api/health" => Some(Self::Health),
            "/api/gsi/cs2" => Some(Self::Gsi),
            "/api/gsi/status" => Some(Self::GsiStatus),
            "/api/config" if has_profile => Some(Self::Config),
            "/api/config/quick-check" if has_profile => Some(Self::QuickCheck),
            "/api/config/ffmpeg-check" if has_profile => Some(Self::FfmpegCheck),
            "/api/config/data-dir-info" if has_profile => Some(Self::DataDirInfo),
            "/api/config-backup/status" if has_profile => Some(Self::BackupStatus),
            "/api/app/runtime-state" if has_profile => Some(Self::RuntimeState),
            _ if has_profile => DemoRoute::from_path(path).map(Self::Demos),
            _ => None,
        }
    }

    fn is_guest(&self) -> bool {
        // These are guest routes in the deployed backend. The game sends no desktop header.
        matches!(self, Self::Health | Self::Gsi | Self::GsiStatus)
    }
}

async fn route(state: &State, request: Request<Incoming>) -> HttpResponse {
    let path = request.uri().path();
    let has_profile = state.profile.is_some();
    let Some(route) = Route::from_path(path, has_profile) else {
        if let Some(canonical) = path
            .strip_suffix('/')
            .filter(|path| Route::from_path(path, has_profile).is_some())
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
    if !route.is_guest() {
        let profile = state.profile.as_ref().expect("profile route");
        let header = request
            .headers()
            .get(SESSION_TOKEN_HEADER)
            .map(|value| value.to_str().unwrap_or("\u{fffd}"));
        let query = request.uri().query().unwrap_or_default();
        if !profile.auth.authorize_http(header, query.as_bytes()) {
            return detail(StatusCode::UNAUTHORIZED, "Desktop session required");
        }
    }
    let allowed = if matches!(route, Route::Gsi) {
        "POST"
    } else {
        "GET"
    };
    let config_update = matches!(route, Route::Config) && request.method() == Method::PUT;
    if request.method().as_str() != allowed && !config_update {
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
        Route::Demos(route) => {
            demo_response(
                state.profile.as_ref().expect("profile route"),
                route,
                request,
            )
            .await
        }
        _ => profile_route(state, route, request).await,
    }
}

async fn demo_response(
    profile: &Arc<ProfileState>,
    route: DemoRoute,
    request: Request<Incoming>,
) -> HttpResponse {
    match route {
        DemoRoute::List(kind) => {
            let query =
                match demo_query::parse_list(request.uri().query().unwrap_or_default(), kind) {
                    Ok(query) => query,
                    Err(errors) => {
                        return json_reply(
                            StatusCode::UNPROCESSABLE_ENTITY,
                            json!({"detail": errors}),
                        )
                    }
                };
            profile_disk(profile, move |config| {
                match DemoLibrary::new(config.database_path()).list(query) {
                    Ok(value) => json_reply(StatusCode::OK, value),
                    Err(error) => demo_read_error(&error),
                }
            })
            .await
        }
        DemoRoute::Detail | DemoRoute::PlayerStats => {
            let id = match demo_query::parse_id(request.uri().path()) {
                Ok(id) => id,
                Err(errors) => {
                    return json_reply(StatusCode::UNPROCESSABLE_ENTITY, json!({"detail": errors}))
                }
            };
            profile_disk(profile, move |config| {
                let library = DemoLibrary::new(config.database_path());
                let result = if matches!(route, DemoRoute::Detail) {
                    library.detail(id)
                } else {
                    library.player_stats(id)
                };
                match result {
                    Ok(Some(value)) => json_reply(StatusCode::OK, value),
                    Ok(None) => detail(StatusCode::NOT_FOUND, &format!("Demo not found: {id}")),
                    Err(error) => demo_read_error(&error),
                }
            })
            .await
        }
    }
}

fn demo_read_error(error: &DemoReadError) -> HttpResponse {
    eprintln!("Native Demo library read failed: {error}");
    if matches!(error, DemoReadError::InvalidFullResult) {
        return reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            "text/plain; charset=utf-8",
            "Internal Server Error".to_owned(),
        );
    }
    detail(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Demo library database is unreadable",
    )
}

async fn profile_route(state: &State, route: Route, request: Request<Incoming>) -> HttpResponse {
    let profile = state.profile.as_ref().expect("profile route");
    if matches!(route, Route::RuntimeState) {
        let data = profile.config.data_dir();
        let data_path = resolved_path(data).unwrap_or_else(|_| data.to_owned());
        return json_reply(
            StatusCode::OK,
            json!({
                "pid": std::process::id(),
                "instance_id": profile.instance_id,
                "version": state.version,
                "data_dir": path_text(&data_path),
                "recovery_required": data.join("recovery-required.json").is_file(),
                "runtime_session": profile.session.state(),
                "demoparser_runtime": {
                    "ready": false,
                    "installed_version": null,
                    "required_version": "0.41.4+cs2insight7",
                    "missing_methods": ["decode_smoke_voxel_journal", "write_replay_parquet", "read_replay_parquet_round", "read_replay_parquet_round_binary"],
                    "import_error": "Native replay integration is unavailable",
                },
            }),
        );
    }
    if request.method() == Method::PUT {
        let input = match config_body(request).await {
            Ok(input) => input,
            Err(response) => return *response,
        };
        return profile_disk(profile, move |config| match config.update(input) {
            Ok(()) => json_reply(StatusCode::OK, json!({"status": "ok"})),
            Err(ConfigUpdateError::Validation(errors)) => {
                json_reply(StatusCode::UNPROCESSABLE_ENTITY, json!({"detail": errors}))
            }
            Err(ConfigUpdateError::Storage(error)) => profile_io_error(&error),
        })
        .await;
    }
    profile_disk(profile, move |config| {
        let result = match route {
            Route::Config => config.view(),
            Route::QuickCheck => config.quick_check(),
            Route::FfmpegCheck => config.ffmpeg_check(),
            Route::DataDirInfo => data_dir_info(config.data_dir()),
            Route::BackupStatus => backup_status(config.data_dir()),
            _ => unreachable!("profile read route"),
        };
        match result {
            Ok(value) => json_reply(StatusCode::OK, value),
            Err(error) => profile_io_error(&error),
        }
    })
    .await
}

async fn profile_disk(
    profile: &Arc<ProfileState>,
    operation: impl FnOnce(&ProfileConfig) -> HttpResponse + Send + 'static,
) -> HttpResponse {
    // Keep file work bounded even if a connection times out while an I/O operation is in progress.
    let permit = Arc::clone(&profile.disk)
        .acquire_owned()
        .await
        .expect("profile I/O semaphore");
    let profile = Arc::clone(profile);
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        operation(&profile.config)
    })
    .await
    {
        Ok(response) => response,
        Err(error) => profile_io_error(&error),
    }
}

fn profile_io_error(error: &dyn std::fmt::Display) -> HttpResponse {
    eprintln!("Native profile I/O failed: {error}");
    detail(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Configuration could not be read or saved",
    )
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

async fn config_body(request: Request<Incoming>) -> Result<Value, Box<HttpResponse>> {
    let is_json = is_json(request.headers().get(CONTENT_TYPE));
    let bytes = read_body(
        request.into_body(),
        "Configuration request body is too large",
    )
    .await?;
    if bytes.is_empty() {
        return Err(Box::new(missing_config_body()));
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| Box::new(invalid_body()))?;
    if !is_json {
        return Ok(json!(text));
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Null) => Err(Box::new(missing_config_body())),
        Ok(value) => Ok(value),
        Err(error) => Err(Box::new(invalid_json(text, &error))),
    }
}

fn missing_config_body() -> HttpResponse {
    json_reply(
        StatusCode::UNPROCESSABLE_ENTITY,
        json!({"detail": [
            {"type": "missing", "loc": ["body"], "msg": "Field required", "input": null}
        ]}),
    )
}

async fn read_body(body: Incoming, too_large: &str) -> Result<Bytes, Box<HttpResponse>> {
    if body
        .size_hint()
        .upper()
        .is_some_and(|size| size > MAX_BODY_BYTES as u64)
    {
        return Err(Box::new(detail(StatusCode::PAYLOAD_TOO_LARGE, too_large)));
    }
    match Limited::new(body, MAX_BODY_BYTES).collect().await {
        Ok(body) => Ok(body.to_bytes()),
        Err(error) if error.is::<LengthLimitError>() => {
            Err(Box::new(detail(StatusCode::PAYLOAD_TOO_LARGE, too_large)))
        }
        Err(_) => Err(Box::new(invalid_body())),
    }
}

fn invalid_body() -> HttpResponse {
    detail(
        StatusCode::BAD_REQUEST,
        "There was an error parsing the body",
    )
}

async fn notify_gsi(state: &State, request: Request<Incoming>) -> HttpResponse {
    let is_json = is_json(request.headers().get(CONTENT_TYPE));
    let bytes = match read_body(request.into_body(), "GSI request body is too large").await {
        Ok(bytes) => bytes,
        Err(response) => return *response,
    };
    let payload = if bytes.is_empty() {
        json!({})
    } else {
        let Ok(text) = std::str::from_utf8(&bytes) else {
            return invalid_body();
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

fn invalid_dictionary(input: Value) -> HttpResponse {
    json_reply(
        StatusCode::UNPROCESSABLE_ENTITY,
        json!({"detail": [{
            "type": "dict_type", "loc": ["body"], "msg": "Input should be a valid dictionary", "input": input
        }]}),
    )
}

fn invalid_json(text: &str, error: &serde_json::Error) -> HttpResponse {
    let (offset, description) = json_decode_error(text, error);
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
