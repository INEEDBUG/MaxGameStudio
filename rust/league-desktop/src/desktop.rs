use crate::{
    client_runtime::{client_from_view, monitor, ClientRuntime, CLIENT},
    profile::DesktopProfile,
};
use maxgamestudio_league_core::{
    profile_database::PROFILE_DB_VERSION,
    shell_preferences::{ShellPreference, APP_COMMON, MAIN_WINDOW},
};
use serde_json::{json, Map, Value};
use std::io;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
    WindowEvent,
};
use tauri_plugin_opener::OpenerExt;

const SYNC_NAMESPACE: &str = "mobx-utils-main";

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
    emit(
        window,
        SYNC_NAMESPACE,
        &format!("update-state-prop/{MAIN_WINDOW}:state"),
        json!([key, value, { "action": "update", "raw": false }]),
    )
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

#[tauri::command]
fn league_call(
    window: WebviewWindow,
    profile: State<'_, DesktopProfile>,
    client: State<'_, ClientRuntime>,
    namespace: String,
    method: String,
    args: Vec<Value>,
) -> Result<Value, String> {
    if !matches!(window.label(), "main" | "aux" | "opgg") {
        return Err("此窗口无法调用工具箱。".into());
    }
    let invoke = || -> Result<Value, Box<dyn std::error::Error>> {
        match (namespace.as_str(), method.as_str()) {
            (APP_COMMON, "getVersion") => Ok(json!(env!("CARGO_PKG_VERSION"))),
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
                            values.insert(preference.key().into(), field(value));
                        }
                        Ok(Value::Object(values))
                    }
                    (Some(CLIENT), Some("settings")) => {
                        let db = profile.database.lock().map_err(|_| "数据库连接中断。")?;
                        let enabled = ShellPreference::ClientAutoConnect
                            .read(&db, profile.system_locale)
                            .map_err(|error| error.to_string())?;
                        Ok(json!({ "autoConnect": field(enabled) }))
                    }
                    (Some(MAIN_WINDOW), Some("state")) if window.label() == "main" => Ok(json!({
                        "status": field(json!(window_status(&window)?)),
                        "focus": field(json!(if window.is_focused()? { "focused" } else { "blurred" })),
                        "show": field(json!(window.is_visible()?)),
                        "ready": field(json!(true)),
                    })),
                    (Some(MAIN_WINDOW), Some("settings")) if window.label() == "main" => {
                        Ok(json!({
                            "closeAction": field(json!(saved_close_action(&profile)?)),
                        }))
                    }
                    (Some(namespace), Some(name)) => match client.fields(namespace, name)? {
                        Some((revision, Value::Object(fields))) => Ok(Value::Object(
                            fields
                                .into_iter()
                                .map(|(key, value)| {
                                    (
                                        key,
                                        json!({
                                            "value": value,
                                            "config": { "raw": false, "revision": revision },
                                        }),
                                    )
                                })
                                .collect(),
                        )),
                        _ => Err("对应的工具箱状态尚未接入 Rust 运行时。".into()),
                    },
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
                let preference = ShellPreference::from_ipc(namespace, key)
                    .filter(|setting| {
                        *setting != ShellPreference::CloseAction || window.label() == "main"
                    })
                    .ok_or("对应的工具箱设置尚未接入 Rust 运行时。")?;
                let db = profile.database.lock().map_err(|_| "数据库连接中断。")?;
                if method == "get" {
                    return preference
                        .read(&db, profile.system_locale)
                        .map_err(|error| error.to_string().into());
                }
                let value = args.get(2).ok_or("缺少设置值。")?;
                preference
                    .write(&db, value)
                    .map_err(|error| error.to_string())?;
                drop(db);
                if preference == ShellPreference::ClientAutoConnect {
                    client.set_auto_connect(value.as_bool().ok_or("无效的自动连接设置。")?)?;
                }

                let event = format!("update-state-prop/{namespace}:settings");
                let args = json!([key, value, {"action": "update", "raw": false}]);
                for label in ["main", "aux", "opgg"] {
                    if let Some(target) = window.app_handle().get_webview_window(label) {
                        emit(&target, SYNC_NAMESPACE, &event, args.clone())?;
                    }
                }
                Ok(Value::Null)
            }
            (MAIN_WINDOW, method) if window.label() == "main" => {
                match method {
                    "minimize" => window.minimize()?,
                    "maximize" => window.maximize()?,
                    "unmaximize" => window.unmaximize()?,
                    "restore" => show_main(window.app_handle())?,
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
            _ => Err("该工具箱功能尚未接入 Rust 运行时。".into()),
        }
    };
    Ok(match invoke() {
        Ok(data) => json!({ "success": true, "data": data }),
        Err(error) => json!({ "success": false, "error": { "message": error.to_string() } }),
    })
}

pub(crate) fn run() -> tauri::Result<()> {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            let _ = show_main(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![league_call])
        .setup(|app| {
            // Setup runs after the single-instance plugin has taken ownership of this process.
            let profile = DesktopProfile::prepare(app)
                .map_err(|error| io::Error::other(format!("工具箱数据无法准备。{error}")))?;
            let webview_directory = profile.webview_directory()?;
            let auto_connect = {
                let db = profile
                    .database
                    .lock()
                    .map_err(|_| io::Error::other("数据库连接中断。"))?;
                ShellPreference::ClientAutoConnect
                    .read(&db, profile.system_locale)
                    .map_err(io::Error::other)?
                    .as_bool()
                    .unwrap_or(true)
            };
            app.manage(profile);
            app.manage(ClientRuntime::new(auto_connect)?);
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
                .on_navigation(|url| {
                    matches!(
                        (url.scheme(), url.host_str()),
                        ("tauri", Some("localhost")) | ("http", Some("tauri.localhost"))
                    )
                })
                .build()?;
            tauri::async_runtime::spawn(monitor(app.handle().clone()));
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            let Some(webview) = window.app_handle().get_webview_window("main") else {
                return;
            };
            match event {
                WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    let profile = window.state::<DesktopProfile>();
                    let _ = close_main(&webview, saved_close_action(&profile).unwrap_or("ask"));
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
