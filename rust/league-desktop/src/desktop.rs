use crate::{
    client_runtime::{client_from_view, monitor, ClientRuntime, CLIENT},
    lcu::game_art::GameArtPath,
    opgg_window::{self, OpggPreferences, OpggWindow},
    profile::DesktopProfile,
    resg_window::{self, ResgAction, ResgWindow},
};
use maxgamestudio_league_core::{
    profile_database::PROFILE_DB_VERSION,
    resg::RESG_NAMESPACE,
    shell_preferences::{ShellPreference, APP_COMMON, MAIN_WINDOW},
};
use serde_json::{json, Map, Value};
use std::{
    io,
    sync::atomic::{AtomicU64, Ordering},
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
    WindowEvent,
};
use tauri_plugin_opener::OpenerExt;

#[path = "bootstrap.rs"]
mod bootstrap;

#[path = "updater.rs"]
mod updater;

use bootstrap::{AUTO_SELECT, AUX_WINDOW, OPGG_WINDOW};

const SYNC_NAMESPACE: &str = "mobx-utils-main";
const LOCAL_WINDOWS: [&str; 3] = ["main", "aux", "opgg"];
const GAME_ART_SCHEME: &str = "league-art";

fn serve_game_art(
    context: tauri::UriSchemeContext<'_, tauri::Wry>,
    request: tauri::http::Request<Vec<u8>>,
    responder: tauri::UriSchemeResponder,
) {
    use tauri::http::{Method, Response, StatusCode};
    let allowed = LOCAL_WINDOWS.contains(&context.webview_label())
        && request.method() == Method::GET
        && request.uri().query().is_none();
    let path = allowed
        .then(|| GameArtPath::parse(request.uri().path()))
        .flatten();
    let Some(path) = path else {
        responder.respond(
            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Vec::new())
                .expect("valid HTTP response"),
        );
        return;
    };
    let app = context.app_handle().clone();
    tauri::async_runtime::spawn(async move {
        let runtime = app.state::<ClientRuntime>();
        let result = runtime.game_art(&path).await;
        let builder = Response::builder()
            .header("Access-Control-Allow-Origin", "http://tauri.localhost")
            .header("Cache-Control", "no-store")
            .header("X-Content-Type-Options", "nosniff");
        let response = if let Some(art) = result {
            builder
                .header("Content-Type", art.content_type)
                .body(art.body)
        } else {
            builder.status(StatusCode::NOT_FOUND).body(Vec::new())
        };
        responder.respond(response.expect("valid game image HTTP response"));
    });
}

#[derive(Default)]
struct SettingsRevision(AtomicU64);

impl SettingsRevision {
    // Both operations run while holding the profile database lock.
    fn current(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }

    fn next(&self) -> u64 {
        self.0.fetch_add(1, Ordering::Relaxed) + 1
    }
}

fn saved_close_action(profile: &DesktopProfile) -> Result<&'static str, String> {
    let db = profile.database.lock().map_err(|_| "数据库连接中断。")?;
    let saved = ShellPreference::CloseAction
        .read(&db, profile.system_locale)
        .map_err(|error| error.to_string())?;
    Ok(match saved.as_str() {
        Some("minimize-to-tray") => "minimize-to-tray",
        Some("quit") => "quit",
        _ => "ask",
    })
}

fn show_main(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("main") {
        window.show()?;
        window.unminimize()?;
        window.set_focus()?;
        publish_window_property(&window, "show", json!(true))?;
    }
    Ok(())
}

pub(crate) fn emit(
    window: &WebviewWindow,
    namespace: &str,
    event_name: &str,
    args: Value,
) -> tauri::Result<()> {
    window.emit_to(
        window.label(),
        "league-ipc-event",
        json!({ "namespace": namespace, "eventName": event_name, "args": args }),
    )
}

fn publish_window_property(window: &WebviewWindow, key: &str, value: Value) -> tauri::Result<()> {
    let namespace = match window.label() {
        "main" => MAIN_WINDOW,
        "aux" => AUX_WINDOW,
        "opgg" => OPGG_WINDOW,
        _ => return Ok(()),
    };
    let event = format!("update-state-prop/{namespace}:state");
    let args = json!([key, value, {"action":"update","raw":false}]);
    for label in LOCAL_WINDOWS {
        if let Some(target) = window.app_handle().get_webview_window(label) {
            emit(&target, SYNC_NAMESPACE, &event, args.clone())?;
        }
    }
    Ok(())
}

fn window_status(window: &WebviewWindow) -> tauri::Result<&'static str> {
    if window.is_minimized()? {
        Ok("minimized")
    } else if window.is_maximized()? {
        Ok("maximized")
    } else {
        Ok("normal")
    }
}

fn close_main(window: &WebviewWindow, action: &str) -> Result<(), String> {
    let result = match action {
        "minimize-to-tray" => window
            .hide()
            .and_then(|()| publish_window_property(window, "show", json!(false))),
        "ask" => show_main(window.app_handle())
            .and_then(|()| emit(window, MAIN_WINDOW, "close-asking", json!([]))),
        "quit" => {
            window.app_handle().exit(0);
            Ok(())
        }
        _ => return Err("无效的关闭方式。".into()),
    };
    result.map_err(|error| error.to_string())
}

fn field(value: Value) -> Value {
    json!({ "value": value, "config": { "raw": false } })
}

fn response(result: Result<Value, impl std::fmt::Display>) -> Value {
    match result {
        Ok(data) => json!({"success":true,"data":data}),
        Err(error) => json!({"success":false,"error":{"message":error.to_string()}}),
    }
}

async fn update_call(
    window: &WebviewWindow,
    method: &str,
    args: &[Value],
) -> Result<Value, String> {
    let updater = window.state::<updater::LeagueUpdater>();
    match (method, args) {
        ("check", []) => {
            let installed = window.app_handle().package_info().version.to_string();
            let check = updater
                .check(&installed)
                .await
                .map_err(|error| error.to_string())?;
            Ok(json!(check))
        }
        ("openDownload", [Value::String(version)]) => {
            let url = updater
                .download_url(version)
                .map_err(|error| error.to_string())?;
            window
                .opener()
                .open_url(url, None::<&str>)
                .map_err(|_| "无法打开 League 下载页面。".to_owned())?;
            Ok(Value::Null)
        }
        _ => Err("无效的 League 更新操作。".into()),
    }
}

fn state_fields(fields: Value, revision: Option<u64>) -> Value {
    let Value::Object(fields) = fields else {
        unreachable!("renderer state is a JSON object");
    };
    Value::Object(
        fields
            .into_iter()
            .map(|(key, value)| {
                let mut field = field(value);
                if let Some(revision) = revision {
                    field["config"]["revision"] = json!(revision);
                }
                (key, field)
            })
            .collect(),
    )
}

fn settings_changed(
    app: &AppHandle,
    namespace: &str,
    key: &str,
    value: &Value,
    revision: u64,
) -> tauri::Result<()> {
    let event = format!("update-state-prop/{namespace}:settings");
    let args = json!([key, value, {"action":"update","raw":false,"revision":revision}]);
    for label in LOCAL_WINDOWS {
        if let Some(target) = app.get_webview_window(label) {
            emit(&target, SYNC_NAMESPACE, &event, args.clone())?;
        }
    }
    Ok(())
}

fn window_label(namespace: &str) -> Option<&'static str> {
    match namespace {
        MAIN_WINDOW => Some("main"),
        AUX_WINDOW => Some("aux"),
        OPGG_WINDOW => Some("opgg"),
        _ => None,
    }
}

fn window_state(app: &AppHandle, label: &str) -> tauri::Result<Value> {
    let Some(window) = app.get_webview_window(label) else {
        return Ok(
            json!({"status":"normal","focus":"blurred","show":false,"ready":false,"trackedBounds":null}),
        );
    };
    Ok(json!({
        "status":window_status(&window)?,
        "focus":if window.is_focused()? { "focused" } else { "blurred" },
        "show":window.is_visible()?,
        "ready":true,
    }))
}

fn bundled_navigation(url: &tauri::Url) -> bool {
    url.port().is_none()
        && matches!(
            (url.scheme(), url.host_str()),
            ("tauri", Some("localhost")) | ("http", Some("tauri.localhost"))
        )
}

pub(crate) fn show_opgg_companion(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    show_companion(app, OPGG_WINDOW, true)
}

pub(crate) fn close_opgg_companion(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("opgg") {
        window.destroy()?;
        publish_window_property(&window, "show", json!(false))?;
        publish_window_property(&window, "ready", json!(false))?;
    }
    Ok(())
}

pub(crate) fn show_companion(
    app: &AppHandle,
    namespace: &str,
    inactive: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let opgg_draft = if namespace == OPGG_WINDOW {
        Some(
            app.state::<OpggWindow>()
                .target(app)
                .ok_or("请先开启 OP.GG 并进入大乱斗选人。")?
                .scope,
        )
    } else {
        None
    };
    let (label, entry, title, width, height, min_height) = match namespace {
        AUX_WINDOW => ("aux", "aux-window.html", "小型辅助面板", 340., 420., 420.),
        OPGG_WINDOW => ("opgg", "opgg-window.html", "OP.GG", 530., 720., 530.),
        _ => return Err("未知的辅助窗口。".into()),
    };
    let profile = app.state::<DesktopProfile>();
    let (enabled, pinned) = {
        let db = profile.database.lock().map_err(|_| "数据库连接中断。")?;
        let preferences = bootstrap::initial_state(&db, namespace, "settings")
            .map_err(|error| error.to_string())?
            .ok_or("辅助窗口设置不可用。")?;
        (
            preferences["enabled"] == true,
            preferences["pinned"] == true,
        )
    };
    if !enabled {
        return Err("该辅助窗口已关闭。".into());
    }
    let target = if let Some(target) = app.get_webview_window(label) {
        target
    } else {
        let created = WebviewWindowBuilder::new(app, label, WebviewUrl::App(entry.into()))
            .data_directory(profile.webview_directory()?)
            .title(title)
            .inner_size(width, height)
            .min_inner_size(width, min_height)
            .decorations(false)
            .always_on_top(pinned)
            .visible(false)
            .focused(!inactive)
            .on_navigation(bundled_navigation)
            .build();
        match created {
            Ok(window) => window,
            Err(error) => app.get_webview_window(label).ok_or(error)?,
        }
    };
    // A setting can change while WebView2 is creating the window.
    let enabled = {
        let db = profile.database.lock().map_err(|_| "数据库连接中断。")?;
        bootstrap::backend_pref(namespace, "enabled")
            .expect("known companion preference")
            .read(&db)
            .map_err(|error| error.to_string())?
            == true
    };
    if !enabled {
        if opgg_draft.is_some() {
            let _ = close_opgg_companion(app);
        }
        return Err("该辅助窗口已关闭。".into());
    }
    if let Some(expected) = opgg_draft {
        let opgg = app.state::<OpggWindow>();
        match opgg.target(app) {
            Some(current) if current.scope == expected => opgg.mark_shown(expected),
            Some(_) => return Err("当前大乱斗选人已变更。".into()),
            None => {
                let _ = close_opgg_companion(app);
                return Err("当前大乱斗选人已结束。".into());
            }
        }
    }
    target.show()?;
    if !inactive {
        target.unminimize()?;
        target.set_focus()?;
    }
    publish_window_property(&target, "ready", json!(true))?;
    publish_window_property(&target, "show", json!(true))?;
    Ok(())
}

#[tauri::command]
async fn league_call(
    window: WebviewWindow,
    profile: State<'_, DesktopProfile>,
    client: State<'_, ClientRuntime>,
    revision: State<'_, SettingsRevision>,
    namespace: String,
    method: String,
    args: Vec<Value>,
) -> Result<Value, String> {
    if !LOCAL_WINDOWS.contains(&window.label()) {
        return Err("此窗口无法调用工具箱。".into());
    }
    if namespace == updater::NAMESPACE {
        if window.label() != "main" {
            return Err("只能从主窗检查 League 更新。".into());
        }
        return Ok(response(update_call(&window, &method, &args).await));
    }
    if namespace == "player-search-main" && method == "lookup" && window.label() == "main" {
        let [shard, input] = args.as_slice() else {
            return Err("缺少玩家查询条件。".into());
        };
        let shard = shard.as_str().ok_or("缺少区服。")?;
        let input = input.as_str().ok_or("缺少玩家名称和标签。")?;
        return Ok(response(client.lookup_player(shard, input).await));
    }
    let app = window.app_handle();
    let invoke = || -> Result<Value, Box<dyn std::error::Error>> {
        match (namespace.as_str(), method.as_str()) {
            (APP_COMMON, "getVersion") => Ok(json!(app.package_info().version.to_string())),
            (APP_COMMON, "openUserDataDir") => {
                window
                    .opener()
                    .open_path(profile.directory.to_string_lossy(), None::<String>)?;
                Ok(Value::Null)
            }
            (APP_COMMON, "exit") => {
                window.app_handle().exit(0);
                Ok(Value::Null)
            }
            (CLIENT, "connect" | "disconnect") if window.label() == "main" => {
                let selection = if method == "connect" {
                    Some(
                        args.first()
                            .and_then(client_from_view)
                            .ok_or("无效的客户端进程。")?,
                    )
                } else {
                    None
                };
                client.select(window.app_handle(), selection)?;
                Ok(Value::Null)
            }
            ("ongoing-game-main", "getAll") => Ok(bootstrap::ongoing_game_data()),
            ("renderer-debug-main", "setSendAllNativeLcuEvents")
                if args.first() == Some(&json!(false)) =>
            {
                Ok(Value::Null)
            }
            (AUTO_SELECT, "setPickConfig" | "setBanConfig") if window.label() == "main" => {
                let group = args
                    .first()
                    .and_then(Value::as_str)
                    .ok_or("缺少选人分组。")?;
                let patch = args.get(1).ok_or("缺少选人配置。")?;
                let key = if method == "setPickConfig" {
                    "pickConfig"
                } else {
                    "banConfig"
                };
                let db = profile.database.lock().map_err(|_| "数据库连接中断。")?;
                let value = bootstrap::set_aram_config(&db, key, group, patch)
                    .map_err(|error| error.to_string())?;
                client.replace_aram_options(
                    bootstrap::aram_options(&db).map_err(io::Error::other)?,
                )?;
                let revision = revision.next();
                drop(db);
                settings_changed(app, AUTO_SELECT, key, &value, revision)?;
                Ok(Value::Null)
            }
            (AUTO_SELECT, "setTemporarilyDisabled") if window.label() == "main" => {
                let paused = args
                    .first()
                    .and_then(Value::as_bool)
                    .ok_or("缺少自动选禁暂停状态。")?;
                client.pause_aram(app, paused)?;
                Ok(Value::Null)
            }
            (
                RESG_NAMESPACE,
                method @ ("setEnabled" | "setAutoShow" | "setAlwaysOnTop" | "setDisplayMode"),
            ) if window.label() == "main" => {
                let db = profile.database.lock().map_err(|_| "数据库连接中断。")?;
                let key = match method {
                    "setEnabled" => "enabled",
                    "setAutoShow" => "autoShow",
                    "setAlwaysOnTop" => "alwaysOnTop",
                    "setDisplayMode" => "displayMode",
                    _ => unreachable!("known RESG method"),
                };
                let value = args.first().ok_or("缺少 RESG 设置值。")?;
                bootstrap::backend_pref(RESG_NAMESPACE, key)
                    .expect("known RESG preference")
                    .write(&db, value)
                    .map_err(|error| error.to_string())?;
                let controller = app.state::<ResgWindow>();
                controller.replace_preferences(
                    bootstrap::resg_preferences(&db).map_err(io::Error::other)?,
                )?;
                drop(db);
                if key == "enabled" {
                    client.refresh_soon();
                }
                Ok(controller.reconcile(app, ResgAction::Synchronize)?)
            }
            (RESG_NAMESPACE, method @ ("getSnapshot" | "show" | "close" | "retry"))
                if window.label() == "main" =>
            {
                let action = match method {
                    "show" => ResgAction::Show,
                    "close" => ResgAction::Close,
                    "retry" => ResgAction::Retry,
                    _ => ResgAction::Synchronize,
                };
                Ok(app.state::<ResgWindow>().reconcile(app, action)?)
            }
            (opgg_window::NAMESPACE, "openGuide")
                if window.label() == "opgg" && args.is_empty() =>
            {
                app.state::<OpggWindow>().open_guide(app)?;
                Ok(Value::Null)
            }
            (SYNC_NAMESPACE, "subscribeAndGetInitialState") => {
                match (
                    args.first().and_then(Value::as_str),
                    args.get(1).and_then(Value::as_str),
                ) {
                    (Some("storage-main"), Some("state")) => {
                        let version = profile
                            .database
                            .lock()
                            .map_err(|_| "数据库连接中断。")?
                            .version();
                        Ok(json!({
                            "usingHigherVersionDb": field(json!(version > PROFILE_DB_VERSION)),
                        }))
                    }
                    (Some(APP_COMMON), Some("state")) => Ok(json!({
                        "platform": field(json!("win32")),
                    })),
                    (Some(APP_COMMON), Some("settings")) => {
                        let db = profile.database.lock().map_err(|_| "数据库连接中断。")?;
                        let mut values = Map::new();
                        for preference in ShellPreference::APP_SETTINGS {
                            let value = preference
                                .read(&db, profile.system_locale)
                                .map_err(|error| error.to_string())?;
                            values.insert(preference.key().into(), value);
                        }
                        Ok(state_fields(
                            Value::Object(values),
                            Some(revision.current()),
                        ))
                    }
                    (Some(CLIENT), Some("settings")) => {
                        let db = profile.database.lock().map_err(|_| "数据库连接中断。")?;
                        let enabled = ShellPreference::ClientAutoConnect
                            .read(&db, profile.system_locale)
                            .map_err(|error| error.to_string())?;
                        Ok(state_fields(
                            json!({"autoConnect":enabled}),
                            Some(revision.current()),
                        ))
                    }
                    (Some(namespace), Some("state")) if window_label(namespace).is_some() => Ok(
                        state_fields(window_state(app, window_label(namespace).unwrap())?, None),
                    ),
                    (Some(namespace), Some(name)) => {
                        if let Some((revision, fields)) = client.fields(namespace, name)? {
                            return Ok(state_fields(fields, Some(revision)));
                        }
                        let db = profile.database.lock().map_err(|_| "数据库连接中断。")?;
                        let mut state = bootstrap::initial_state(&db, namespace, name)
                            .map_err(|error| error.to_string())?
                            .ok_or("对应的工具箱状态尚未接入 Rust 运行时。")?;
                        if (namespace, name) == (MAIN_WINDOW, "settings") {
                            state["closeAction"] = ShellPreference::CloseAction
                                .read(&db, profile.system_locale)
                                .map_err(|error| error.to_string())?;
                        }
                        if (namespace, name) == (AUTO_SELECT, "state") {
                            let (revision, paused) = client.aram_paused()?;
                            state["temporarilyDisabled"] = json!(paused);
                            return Ok(state_fields(state, Some(revision)));
                        }
                        Ok(state_fields(
                            state,
                            (name == "settings").then(|| revision.current()),
                        ))
                    }
                    _ => Err("无效的工具箱状态订阅。".into()),
                }
            }
            ("setting-factory-main", "get" | "set") => {
                let namespace = args
                    .first()
                    .and_then(Value::as_str)
                    .ok_or("缺少设置分组。")?;
                let key = args
                    .get(1)
                    .and_then(Value::as_str)
                    .ok_or("缺少设置名称。")?;
                let db = profile.database.lock().map_err(|_| "数据库连接中断。")?;
                let value = if method == "set" {
                    Some(args.get(2).ok_or("缺少设置值。")?)
                } else {
                    None
                };
                if let Some(preference) = bootstrap::renderer_pref(window.label(), namespace, key) {
                    if let Some(value) = value {
                        preference
                            .write(&db, value)
                            .map_err(|error| error.to_string())?;
                        return Ok(Value::Null);
                    }
                    return Ok(preference
                        .saved(&db)
                        .map_err(|error| error.to_string())?
                        .unwrap_or(Value::Null));
                }
                let shell = ShellPreference::from_ipc(namespace, key);
                if let Some(preference) = shell {
                    let Some(value) = value else {
                        return preference
                            .read(&db, profile.system_locale)
                            .map_err(|error| error.to_string().into());
                    };
                    if preference == ShellPreference::CloseAction && window.label() != "main" {
                        return Err("只能从主窗修改关闭方式。".into());
                    }
                    preference
                        .write(&db, value)
                        .map_err(|error| error.to_string())?;
                    if preference == ShellPreference::ClientAutoConnect {
                        client.set_auto_connect(value.as_bool().ok_or("无效的自动连接设置。")?)?;
                    }
                } else {
                    let preference = bootstrap::backend_pref(namespace, key)
                        .filter(|_| namespace != RESG_NAMESPACE)
                        .ok_or("对应的工具箱设置尚未接入 Rust 运行时。")?;
                    let Some(value) = value else {
                        return preference
                            .read(&db)
                            .map_err(|error| error.to_string().into());
                    };
                    preference
                        .write(&db, value)
                        .map_err(|error| error.to_string())?;
                }
                let value = value.expect("settings reads have returned");
                let revision = revision.next();
                drop(db);
                if namespace == OPGG_WINDOW && matches!(key, "enabled" | "autoShow") {
                    let opgg = app.state::<OpggWindow>();
                    let value = value.as_bool().ok_or("无效的 OP.GG 设置值。")?;
                    opgg.update_setting(key, value)?;
                    client.refresh_soon();
                    if key == "enabled" && !value {
                        opgg.reconcile(app)?;
                    }
                }
                if let Some(target) =
                    window_label(namespace).and_then(|label| app.get_webview_window(label))
                {
                    match (key, value.as_bool()) {
                        ("pinned", Some(pinned)) => target.set_always_on_top(pinned)?,
                        ("enabled", Some(false)) if target.label() != "main" => {
                            target.hide()?;
                            publish_window_property(&target, "show", json!(false))?;
                        }
                        _ => {}
                    }
                }
                settings_changed(app, namespace, key, value, revision)?;
                Ok(Value::Null)
            }
            (MAIN_WINDOW, method) if window.label() == "main" => {
                match method {
                    "minimize" => window.minimize()?,
                    "maximize" => window.maximize()?,
                    "unmaximize" => window.unmaximize()?,
                    "restore" => show_main(window.app_handle())?,
                    "startDragging" => window.start_dragging()?,
                    "closeMainWindow" => {
                        let action = match args.first() {
                            None | Some(Value::Null) => saved_close_action(&profile)?,
                            Some(action) => action.as_str().ok_or("无效的关闭方式。")?,
                        };
                        close_main(&window, action)?;
                    }
                    "closeMainWindowForce" => close_main(&window, "quit")?,
                    _ => return Err("对应的窗口操作尚未接入 Rust 运行时。".into()),
                }
                Ok(Value::Null)
            }
            (AUX_WINDOW | OPGG_WINDOW, "show") if window.label() == "main" => {
                show_companion(
                    app,
                    &namespace,
                    args.first().and_then(Value::as_bool).unwrap_or(false),
                )?;
                Ok(Value::Null)
            }
            (AUX_WINDOW | OPGG_WINDOW, method)
                if window_label(&namespace) == Some(window.label()) =>
            {
                match method {
                    "minimize" => window.minimize()?,
                    "maximize" => window.maximize()?,
                    "unmaximize" => window.unmaximize()?,
                    "startDragging" => window.start_dragging()?,
                    "close" | "hide" => {
                        window.hide()?;
                        publish_window_property(&window, "show", json!(false))?;
                    }
                    _ => return Err("对应的窗口操作尚未接入 Rust 运行时。".into()),
                }
                Ok(Value::Null)
            }
            _ => Err("该工具箱功能尚未接入 Rust 运行时。".into()),
        }
    };
    Ok(response(invoke()))
}

pub(crate) fn run() -> tauri::Result<()> {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            let _ = show_main(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .register_asynchronous_uri_scheme_protocol(GAME_ART_SCHEME, serve_game_art)
        .invoke_handler(tauri::generate_handler![league_call])
        .setup(|app| {
            // Setup runs after the single-instance plugin has taken ownership of this process.
            let profile = DesktopProfile::prepare(app)
                .map_err(|error| io::Error::other(format!("工具箱数据无法准备。{error}")))?;
            let webview_directory = profile.webview_directory()?;
            let (auto_connect, resg_preferences, aram_options, opgg_preferences) = {
                let db = profile
                    .database
                    .lock()
                    .map_err(|_| io::Error::other("数据库连接中断。"))?;
                let auto_connect = ShellPreference::ClientAutoConnect
                    .read(&db, profile.system_locale)
                    .map_err(io::Error::other)?
                    .as_bool()
                    .unwrap_or(true);
                let opgg = bootstrap::initial_state(&db, OPGG_WINDOW, "settings")
                    .map_err(io::Error::other)?
                    .expect("known OP.GG window settings");
                (
                    auto_connect,
                    bootstrap::resg_preferences(&db).map_err(io::Error::other)?,
                    bootstrap::aram_options(&db).map_err(io::Error::other)?,
                    OpggPreferences {
                        enabled: opgg["enabled"] == true,
                        auto_show: opgg["autoShow"] == true,
                    },
                )
            };
            app.manage(profile);
            app.manage(ClientRuntime::new(auto_connect, aram_options)?);
            app.manage(ResgWindow::new(resg_preferences));
            app.manage(OpggWindow::new(opgg_preferences));
            app.manage(SettingsRevision::default());
            app.manage(updater::LeagueUpdater::default());
            let show = MenuItem::with_id(app, "show", "显示工具箱", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出工具箱", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let mut tray = TrayIconBuilder::new()
                .tooltip("英雄联盟工具箱")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        let _ = show_main(app);
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let _ = show_main(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("main-window.html".into()))
                .data_directory(webview_directory)
                .title("英雄联盟工具箱")
                .inner_size(1500.0, 860.0)
                .min_inner_size(840.0, 600.0)
                .decorations(false)
                .on_navigation(bundled_navigation)
                .build()?;
            tauri::async_runtime::spawn(monitor(app.handle().clone()));
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == resg_window::LABEL && matches!(event, WindowEvent::Destroyed) {
                let app = window.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Some(controller) = app.try_state::<ResgWindow>() {
                        let _ = controller.reconcile(&app, ResgAction::Synchronize);
                    }
                });
                return;
            }
            if !LOCAL_WINDOWS.contains(&window.label()) {
                return;
            }
            let Some(webview) = window.app_handle().get_webview_window(window.label()) else {
                return;
            };
            match event {
                WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    if window.label() == "main" {
                        let profile = window.state::<DesktopProfile>();
                        let _ = close_main(&webview, saved_close_action(&profile).unwrap_or("ask"));
                    } else if webview.hide().is_ok() {
                        let _ = publish_window_property(&webview, "show", json!(false));
                    }
                }
                WindowEvent::Focused(focused) => {
                    let _ = publish_window_property(
                        &webview,
                        "focus",
                        json!(if *focused { "focused" } else { "blurred" }),
                    );
                }
                WindowEvent::Resized(_) => {
                    if let Ok(status) = window_status(&webview) {
                        let _ = publish_window_property(&webview, "status", json!(status));
                    }
                }
                _ => {}
            }
        })
        .run(tauri::generate_context!())
}
