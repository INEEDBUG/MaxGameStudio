use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Mutex,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tauri::window::Color;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, LogicalSize, Manager, RunEvent, State, WebviewUrl, WebviewWindowBuilder,
    WindowEvent,
};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_window_state::{AppHandleExt, StateFlags};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn league_window_defaults(kind: &str) -> Option<(&'static str, f64, f64)> {
    match kind {
        "mini" => Some(("league-mini", 340.0, 420.0)),
        "ongoing" => Some(("league-ongoing", 1360.0, 840.0)),
        "cooldown" => Some(("league-cd-timer", 132.0, 252.0)),
        _ => None,
    }
}

/// `sync_league_mini` is called by a polling UI, so it also defends against
/// stale or hand-written requests. Mini is a lounge/champ-select surface and
/// must never be auto-shown for a game phase.
fn mini_auto_context_allows_show(context: &str) -> bool {
    matches!(
        context.split(':').nth(1),
        Some("Lobby") | Some("Matchmaking") | Some("ReadyCheck") | Some("ChampSelect")
    )
}

/// The independent ongoing window is an in-game surface. Keep the phase gate
/// in the native layer as well as React so stale UI snapshots cannot create it
/// after the client has already returned to the lobby.
fn ongoing_auto_context_allows_show(context: &str) -> bool {
    let mut parts = context.split(':');
    matches!(parts.next(), Some("connected"))
        && matches!(
            parts.next(),
            Some("GameStart") | Some("InProgress") | Some("Reconnect")
        )
}

#[tauri::command]
async fn open_league_mini(app: AppHandle) -> Result<(), String> {
    show_league_mini(app, true).await
}

async fn show_league_mini(app: AppHandle, request_focus: bool) -> Result<(), String> {
    let mini = app.state::<LeagueMiniLifecycle>();
    if request_focus {
        mini.manually_hidden.store(false, Ordering::SeqCst);
        mini.should_show.store(true, Ordering::SeqCst);
        mini.focus_requested.store(true, Ordering::SeqCst);
    }
    if !mini.should_show.load(Ordering::SeqCst) || mini.manually_hidden.load(Ordering::SeqCst) {
        return Ok(());
    }
    if mini.bootstrapping.load(Ordering::SeqCst) {
        return Ok(());
    }
    if let Some(window) = app.get_webview_window("league-mini") {
        // A dynamically-created WebView may exist before React has committed
        // its first frame. Keep it hidden until the bootstrap explicitly
        // acknowledges that it rendered either the panel or an error view.
        if !mini.ready.load(Ordering::SeqCst) {
            return Ok(());
        }
        window.show().map_err(|error| error.to_string())?;
        window.unminimize().map_err(|error| error.to_string())?;
        if request_focus {
            window.set_focus().map_err(|error| error.to_string())?;
        }
        return Ok(());
    }
    let content_protected = app
        .state::<LeaguePrivacyLifecycle>()
        .content_protected
        .load(Ordering::SeqCst);
    mini.bootstrapping.store(true, Ordering::SeqCst);
    let build_result =
        WebviewWindowBuilder::new(&app, "league-mini", WebviewUrl::App("mini.html".into()))
            .title("MaxGameStudio Mini")
            .inner_size(340.0, 420.0)
            .min_inner_size(340.0, 420.0)
            .resizable(true)
            .maximizable(false)
            .fullscreen(false)
            .decorations(false)
            .background_color(Color(20, 20, 22, 255))
            .visible(false)
            .focused(false)
            .content_protected(content_protected)
            .build();
    if let Err(error) = build_result {
        mini.bootstrapping.store(false, Ordering::SeqCst);
        mini.ready.store(false, Ordering::SeqCst);
        return Err(error.to_string());
    }
    // A previous auto-cycle may have destroyed the old WebView. Never carry
    // its ready bit into the newly-created window.
    mini.ready.store(false, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
async fn open_league_ongoing(app: AppHandle) -> Result<(), String> {
    let lifecycle = app.state::<LeagueOngoingLifecycle>();
    lifecycle.manually_hidden.store(false, Ordering::SeqCst);
    lifecycle.should_show.store(true, Ordering::SeqCst);
    show_league_ongoing(app, true).await
}

fn build_league_ongoing_window(app: &AppHandle) -> Result<(), String> {
    let content_protected = app
        .state::<LeaguePrivacyLifecycle>()
        .content_protected
        .load(Ordering::SeqCst);
    WebviewWindowBuilder::new(
        app,
        "league-ongoing",
        WebviewUrl::App("ongoing.html".into()),
    )
    .title("MaxGameStudio · League 实时对局")
    .inner_size(1360.0, 840.0)
    .min_inner_size(980.0, 640.0)
    .resizable(true)
    .decorations(true)
    // Keep an opaque native dark surface behind WebView2 while the hidden
    // window boots. Transparent decorated windows can render as a persistent
    // white surface on Windows/WebView2, even after the React frame commits.
    .background_color(Color(17, 18, 20, 255))
    // Hidden WebView2 controllers can stay on an unpainted white composition
    // surface when they are shown later. Build this window only when the
    // game phase requests it, and make the opaque boot surface visible from
    // the first frame instead of priming a hidden controller at app startup.
    .visible(true)
    .focused(false)
    .content_protected(content_protected)
    .build()
    .map(|_| ())
    .map_err(|error| error.to_string())
}

async fn show_league_ongoing(app: AppHandle, request_focus: bool) -> Result<(), String> {
    let lifecycle = app.state::<LeagueOngoingLifecycle>();
    if request_focus {
        lifecycle.manually_hidden.store(false, Ordering::SeqCst);
        lifecycle.should_show.store(true, Ordering::SeqCst);
        lifecycle.focus_requested.store(true, Ordering::SeqCst);
    }
    if !lifecycle.should_show.load(Ordering::SeqCst)
        || lifecycle.manually_hidden.load(Ordering::SeqCst)
    {
        return Ok(());
    }
    if let Some(window) = app.get_webview_window("league-ongoing") {
        if !lifecycle.ready.load(Ordering::SeqCst) {
            return Ok(());
        }
        window.show().map_err(|error| error.to_string())?;
        window.unminimize().map_err(|error| error.to_string())?;
        if request_focus {
            window.set_focus().map_err(|error| error.to_string())?;
        }
        return Ok(());
    }
    if lifecycle
        .bootstrapping
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Ok(());
    }
    lifecycle.ready.store(false, Ordering::SeqCst);
    if let Err(error) = build_league_ongoing_window(&app) {
        lifecycle.bootstrapping.store(false, Ordering::SeqCst);
        lifecycle.ready.store(false, Ordering::SeqCst);
        return Err(error);
    }
    // A phase change may arrive while WebView2 is constructing the visible
    // dark boot surface. Honour the newest state before returning so a stale
    // GameStart request cannot leave the auxiliary window on screen.
    if !lifecycle.should_show.load(Ordering::SeqCst)
        || lifecycle.manually_hidden.load(Ordering::SeqCst)
    {
        if let Some(window) = app.get_webview_window("league-ongoing") {
            window.hide().map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

#[tauri::command]
async fn open_league_cd_timer(app: AppHandle) -> Result<(), String> {
    let lifecycle = app.state::<LeagueCdTimerLifecycle>();
    lifecycle.manually_hidden.store(false, Ordering::SeqCst);
    lifecycle.should_show.store(true, Ordering::SeqCst);
    show_league_cd_timer(app).await
}

async fn show_league_cd_timer(app: AppHandle) -> Result<(), String> {
    let lifecycle = app.state::<LeagueCdTimerLifecycle>();
    if !lifecycle.should_show.load(Ordering::SeqCst)
        || lifecycle.manually_hidden.load(Ordering::SeqCst)
    {
        return Ok(());
    }
    if lifecycle.bootstrapping.load(Ordering::SeqCst) {
        return Ok(());
    }
    if let Some(window) = app.get_webview_window("league-cd-timer") {
        if !lifecycle.ready.load(Ordering::SeqCst) {
            return Ok(());
        }
        window.show().map_err(|error| error.to_string())?;
        return Ok(());
    }
    let content_protected = app
        .state::<LeaguePrivacyLifecycle>()
        .content_protected
        .load(Ordering::SeqCst);
    lifecycle.bootstrapping.store(true, Ordering::SeqCst);
    let build_result = WebviewWindowBuilder::new(
        &app,
        "league-cd-timer",
        WebviewUrl::App("cd-timer.html".into()),
    )
    .title("MaxGameStudio · League 技能计时器")
    .inner_size(132.0, 252.0)
    .min_inner_size(112.0, 220.0)
    .resizable(false)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .focusable(false)
    .background_color(Color(21, 21, 24, 255))
    .visible(false)
    .focused(false)
    .skip_taskbar(true)
    .shadow(false)
    .content_protected(content_protected)
    .build();
    if let Err(error) = build_result {
        lifecycle.bootstrapping.store(false, Ordering::SeqCst);
        lifecycle.ready.store(false, Ordering::SeqCst);
        return Err(error.to_string());
    }
    Ok(())
}

/// Called by each auxiliary page after React has committed either its normal
/// content or an error surface. A WebView existing is not enough: WebView2 can
/// expose its native background before the first React paint, which is the
/// source of the intermittent white/black auxiliary windows.
#[tauri::command]
fn mark_league_window_ready(app: AppHandle, kind: String) -> Result<(), String> {
    let label = league_window_defaults(&kind)
        .map(|(label, _, _)| label)
        .ok_or_else(|| "unsupported League auxiliary window".to_string())?;

    match kind.as_str() {
        "mini" => {
            let lifecycle = app.state::<LeagueMiniLifecycle>();
            lifecycle.ready.store(true, Ordering::SeqCst);
            lifecycle.bootstrapping.store(false, Ordering::SeqCst);
            if lifecycle.should_show.load(Ordering::SeqCst)
                && !lifecycle.manually_hidden.load(Ordering::SeqCst)
            {
                if let Some(window) = app.get_webview_window(label) {
                    window.show().map_err(|error| error.to_string())?;
                    window.unminimize().map_err(|error| error.to_string())?;
                    if lifecycle.focus_requested.swap(false, Ordering::SeqCst) {
                        window.set_focus().map_err(|error| error.to_string())?;
                    }
                }
            } else {
                lifecycle.focus_requested.store(false, Ordering::SeqCst);
            }
        }
        "ongoing" => {
            let lifecycle = app.state::<LeagueOngoingLifecycle>();
            lifecycle.ready.store(true, Ordering::SeqCst);
            lifecycle.bootstrapping.store(false, Ordering::SeqCst);
            if lifecycle.should_show.load(Ordering::SeqCst)
                && !lifecycle.manually_hidden.load(Ordering::SeqCst)
            {
                if let Some(window) = app.get_webview_window(label) {
                    window.show().map_err(|error| error.to_string())?;
                    window.unminimize().map_err(|error| error.to_string())?;
                    if lifecycle.focus_requested.swap(false, Ordering::SeqCst) {
                        window.set_focus().map_err(|error| error.to_string())?;
                    }
                }
            } else {
                lifecycle.focus_requested.store(false, Ordering::SeqCst);
                if let Some(window) = app.get_webview_window(label) {
                    window.hide().map_err(|error| error.to_string())?;
                }
            }
        }
        "cooldown" => {
            let lifecycle = app.state::<LeagueCdTimerLifecycle>();
            lifecycle.ready.store(true, Ordering::SeqCst);
            lifecycle.bootstrapping.store(false, Ordering::SeqCst);
            if lifecycle.should_show.load(Ordering::SeqCst)
                && !lifecycle.manually_hidden.load(Ordering::SeqCst)
            {
                if let Some(window) = app.get_webview_window(label) {
                    window.show().map_err(|error| error.to_string())?;
                }
            }
        }
        _ => unreachable!("League auxiliary window kind was validated above"),
    }
    Ok(())
}

#[tauri::command]
async fn toggle_league_aux_window(
    app: AppHandle,
    kind: String,
    visible: Option<bool>,
) -> Result<(), String> {
    let label = match kind.as_str() {
        "mini" => "league-mini",
        "ongoing" => "league-ongoing",
        "cooldown" => "league-cd-timer",
        _ => return Err("unsupported League auxiliary window".to_string()),
    };
    let current_visible = app
        .get_webview_window(label)
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false);
    let should_show = visible.unwrap_or(!current_visible);
    match kind.as_str() {
        "mini" => {
            let lifecycle = app.state::<LeagueMiniLifecycle>();
            lifecycle.should_show.store(should_show, Ordering::SeqCst);
            lifecycle.focus_requested.store(false, Ordering::SeqCst);
            lifecycle
                .manually_hidden
                .store(!should_show, Ordering::SeqCst);
        }
        "ongoing" => {
            let lifecycle = app.state::<LeagueOngoingLifecycle>();
            lifecycle.should_show.store(should_show, Ordering::SeqCst);
            lifecycle.focus_requested.store(false, Ordering::SeqCst);
            lifecycle
                .manually_hidden
                .store(!should_show, Ordering::SeqCst);
        }
        "cooldown" => {
            let lifecycle = app.state::<LeagueCdTimerLifecycle>();
            lifecycle.should_show.store(should_show, Ordering::SeqCst);
            lifecycle.focus_requested.store(false, Ordering::SeqCst);
            lifecycle
                .manually_hidden
                .store(!should_show, Ordering::SeqCst);
        }
        _ => unreachable!("League auxiliary window kind was validated above"),
    }
    if should_show {
        match kind.as_str() {
            "mini" => show_league_mini(app, false).await,
            // Shortcut-triggered auxiliary windows must not steal focus from
            // the game; the explicit toolbar command is the focused path.
            "ongoing" => show_league_ongoing(app, false).await,
            "cooldown" => show_league_cd_timer(app).await,
            _ => unreachable!("League auxiliary window kind was validated above"),
        }
    } else if let Some(window) = app.get_webview_window(label) {
        window.hide().map_err(|error| error.to_string())
    } else {
        Ok(())
    }
}

#[tauri::command]
fn set_league_window_pinned(app: AppHandle, kind: String, pinned: bool) -> Result<(), String> {
    let label = league_window_defaults(&kind)
        .map(|(label, _, _)| label)
        .ok_or_else(|| "unsupported League auxiliary window".to_string())?;
    if let Some(window) = app.get_webview_window(label) {
        window
            .set_always_on_top(pinned)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn reset_league_window_position(app: AppHandle, kind: String) -> Result<(), String> {
    let (label, width, height) = league_window_defaults(&kind)
        .ok_or_else(|| "unsupported League auxiliary window".to_string())?;
    let window = app
        .get_webview_window(label)
        .ok_or_else(|| "League auxiliary window is not open".to_string())?;
    window
        .set_size(LogicalSize::new(width, height))
        .map_err(|error| error.to_string())?;
    window.center().map_err(|error| error.to_string())?;
    window.unminimize().map_err(|error| error.to_string())?;
    match kind.as_str() {
        "mini" => {
            let lifecycle = app.state::<LeagueMiniLifecycle>();
            lifecycle.manually_hidden.store(false, Ordering::SeqCst);
            lifecycle.should_show.store(true, Ordering::SeqCst);
            if lifecycle.ready.load(Ordering::SeqCst) {
                window.show().map_err(|error| error.to_string())?;
            }
        }
        "ongoing" => {
            let lifecycle = app.state::<LeagueOngoingLifecycle>();
            lifecycle.manually_hidden.store(false, Ordering::SeqCst);
            lifecycle.should_show.store(true, Ordering::SeqCst);
            if lifecycle.ready.load(Ordering::SeqCst) {
                window.show().map_err(|error| error.to_string())?;
            }
        }
        "cooldown" => {
            let lifecycle = app.state::<LeagueCdTimerLifecycle>();
            lifecycle.manually_hidden.store(false, Ordering::SeqCst);
            lifecycle.should_show.store(true, Ordering::SeqCst);
            if lifecycle.ready.load(Ordering::SeqCst) {
                window.show().map_err(|error| error.to_string())?;
            }
        }
        _ => {}
    }
    app.save_window_state(StateFlags::POSITION | StateFlags::SIZE)
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn save_league_window_state_best_effort(app: &AppHandle) {
    let _ = app.save_window_state(StateFlags::POSITION | StateFlags::SIZE);
}

#[tauri::command]
fn persist_desktop_window_state(app: AppHandle) -> Result<(), String> {
    app.save_window_state(StateFlags::POSITION | StateFlags::SIZE)
        .map_err(|error| error.to_string())
}

#[derive(Default)]
struct LeagueMiniLifecycle {
    manually_hidden: AtomicBool,
    should_show: AtomicBool,
    bootstrapping: AtomicBool,
    ready: AtomicBool,
    focus_requested: AtomicBool,
    context: Mutex<String>,
}

#[derive(Default)]
struct LeagueOngoingLifecycle {
    manually_hidden: AtomicBool,
    should_show: AtomicBool,
    bootstrapping: AtomicBool,
    ready: AtomicBool,
    focus_requested: AtomicBool,
    context: Mutex<String>,
}

#[derive(Default)]
struct LeagueCdTimerLifecycle {
    manually_hidden: AtomicBool,
    should_show: AtomicBool,
    bootstrapping: AtomicBool,
    ready: AtomicBool,
    focus_requested: AtomicBool,
    context: Mutex<String>,
}

#[derive(Default)]
struct LeaguePrivacyLifecycle {
    content_protected: AtomicBool,
}

#[tauri::command]
fn set_league_content_protection(app: AppHandle, enabled: bool) -> Result<(), String> {
    app.state::<LeaguePrivacyLifecycle>()
        .content_protected
        .store(enabled, Ordering::SeqCst);
    for label in ["main", "league-mini", "league-ongoing", "league-cd-timer"] {
        if let Some(window) = app.get_webview_window(label) {
            window
                .set_content_protected(enabled)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

#[tauri::command]
async fn sync_league_mini(
    app: AppHandle,
    should_show: bool,
    context: String,
) -> Result<(), String> {
    let mini = app.state::<LeagueMiniLifecycle>();
    let should_show = should_show && mini_auto_context_allows_show(&context);
    mini.should_show.store(should_show, Ordering::SeqCst);
    {
        let mut saved_context = mini
            .context
            .lock()
            .map_err(|_| "mini lifecycle lock poisoned".to_string())?;
        if *saved_context != context {
            *saved_context = context;
            mini.manually_hidden.store(false, Ordering::SeqCst);
        }
    }
    if !should_show {
        mini.focus_requested.store(false, Ordering::SeqCst);
        if let Some(window) = app.get_webview_window("league-mini") {
            // Do not keep a stale WebView2 surface alive across the game
            // phase. Hiding it is normally enough, but an already-created
            // Mini can retain a white native surface while its old route is
            // still settling. Destroying it here makes InProgress a hard
            // boundary; the next lounge phase creates a fresh, ready-gated
            // WebView.
            let _ = window.destroy();
        }
        mini.ready.store(false, Ordering::SeqCst);
        mini.bootstrapping.store(false, Ordering::SeqCst);
        return Ok(());
    }
    if mini.manually_hidden.load(Ordering::SeqCst) {
        return Ok(());
    }
    show_league_mini(app, false).await
}

#[tauri::command]
async fn sync_league_cd_timer(
    app: AppHandle,
    should_show: bool,
    context: String,
) -> Result<(), String> {
    let lifecycle = app.state::<LeagueCdTimerLifecycle>();
    lifecycle.should_show.store(should_show, Ordering::SeqCst);
    {
        let mut saved_context = lifecycle
            .context
            .lock()
            .map_err(|_| "cooldown timer lifecycle lock poisoned".to_string())?;
        if *saved_context != context {
            *saved_context = context;
            lifecycle.manually_hidden.store(false, Ordering::SeqCst);
        }
    }
    if !should_show {
        lifecycle.focus_requested.store(false, Ordering::SeqCst);
        if let Some(window) = app.get_webview_window("league-cd-timer") {
            window.hide().map_err(|error| error.to_string())?;
        }
        return Ok(());
    }
    if lifecycle.manually_hidden.load(Ordering::SeqCst) {
        return Ok(());
    }
    show_league_cd_timer(app).await
}

#[tauri::command]
async fn sync_league_ongoing(
    app: AppHandle,
    should_show: bool,
    context: String,
) -> Result<(), String> {
    let lifecycle = app.state::<LeagueOngoingLifecycle>();
    let should_show = should_show && ongoing_auto_context_allows_show(&context);
    lifecycle.should_show.store(should_show, Ordering::SeqCst);
    {
        let mut saved_context = lifecycle
            .context
            .lock()
            .map_err(|_| "ongoing lifecycle lock poisoned".to_string())?;
        if *saved_context != context {
            *saved_context = context;
            lifecycle.manually_hidden.store(false, Ordering::SeqCst);
        }
    }
    if !should_show {
        lifecycle.focus_requested.store(false, Ordering::SeqCst);
        if let Some(window) = app.get_webview_window("league-ongoing") {
            window.hide().map_err(|error| error.to_string())?;
        }
        return Ok(());
    }
    if lifecycle.manually_hidden.load(Ordering::SeqCst) {
        return Ok(());
    }
    show_league_ongoing(app, false).await
}

struct BackendProcess {
    child: Mutex<Option<ManagedBackend>>,
    session_token: String,
}

struct AppLifecycle {
    close_action: AtomicU8,
    quitting: AtomicBool,
}

const CLOSE_ACTION_ASK: u8 = 0;
const CLOSE_ACTION_TRAY: u8 = 1;
const CLOSE_ACTION_EXIT: u8 = 2;

impl Default for AppLifecycle {
    fn default() -> Self {
        Self {
            close_action: AtomicU8::new(CLOSE_ACTION_ASK),
            quitting: AtomicBool::new(false),
        }
    }
}

#[tauri::command]
fn set_close_to_tray(enabled: bool, lifecycle: State<'_, AppLifecycle>) {
    lifecycle.close_action.store(
        if enabled {
            CLOSE_ACTION_ASK
        } else {
            CLOSE_ACTION_EXIT
        },
        Ordering::SeqCst,
    );
}

#[tauri::command]
fn get_close_to_tray(lifecycle: State<'_, AppLifecycle>) -> bool {
    lifecycle.close_action.load(Ordering::SeqCst) != CLOSE_ACTION_EXIT
}

fn parse_close_action(action: &str) -> Result<u8, String> {
    match action.trim().to_ascii_lowercase().as_str() {
        "ask" => Ok(CLOSE_ACTION_ASK),
        "tray" => Ok(CLOSE_ACTION_TRAY),
        "exit" => Ok(CLOSE_ACTION_EXIT),
        _ => Err("close action must be ask, tray, or exit".to_string()),
    }
}

fn close_action_name(action: u8) -> &'static str {
    match action {
        CLOSE_ACTION_TRAY => "tray",
        CLOSE_ACTION_EXIT => "exit",
        _ => "ask",
    }
}

#[tauri::command]
fn set_close_action(action: String, lifecycle: State<'_, AppLifecycle>) -> Result<(), String> {
    lifecycle
        .close_action
        .store(parse_close_action(&action)?, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
fn get_close_action(lifecycle: State<'_, AppLifecycle>) -> String {
    close_action_name(lifecycle.close_action.load(Ordering::SeqCst)).to_string()
}

fn show_main_window(handle: &AppHandle) {
    if let Some(window) = handle.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[tauri::command]
fn hide_to_tray(handle: AppHandle) {
    if let Some(window) = handle.get_webview_window("main") {
        let _ = window.hide();
    }
}

#[tauri::command]
fn quit_app(handle: AppHandle) {
    request_app_exit(&handle);
}

#[tauri::command]
fn restart_as_administrator(handle: AppHandle) -> Result<(), String> {
    #[cfg(windows)]
    {
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-Command",
                "Start-Sleep -Milliseconds 900; Start-Process -FilePath $args[0] -Verb RunAs",
            ])
            .arg(executable)
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| error.to_string())?;
        request_app_exit(&handle);
        Ok(())
    }

    #[cfg(not(windows))]
    {
        let _ = handle;
        Err("管理员重启仅支持 Windows".to_string())
    }
}

fn request_app_exit(handle: &AppHandle) {
    let lifecycle = handle.state::<AppLifecycle>();
    if lifecycle.quitting.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(window) = handle.get_webview_window("main") {
        let _ = window.destroy();
    }
    let handle = handle.clone();
    thread::spawn(move || {
        stop_backend(&handle);
        handle.exit(0);
    });
}

impl BackendProcess {
    fn new() -> Result<Self, String> {
        Ok(Self {
            child: Mutex::new(None),
            session_token: new_session_token()?,
        })
    }
}

struct ManagedBackend {
    child: Child,
    instance_id: String,
    data_root: PathBuf,
}

fn backend_http(method: &str, path: &str, session_token: &str) -> Option<String> {
    let address = SocketAddr::from(([127, 0, 0, 1], 19871));
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(350)).ok()?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:19871\r\nX-CS2-Insight-Token: {session_token}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    Some(response)
}

fn new_instance_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{nanos}", std::process::id())
}

fn new_session_token() -> Result<String, String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| format!("无法生成桌面会话令牌：{error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[tauri::command]
fn backend_session_token(state: State<'_, BackendProcess>) -> String {
    state.session_token.clone()
}

#[tauri::command]
fn read_legacy_ui_state() -> Result<Option<String>, String> {
    #[cfg(windows)]
    {
        let app_data = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| "Windows APPDATA 环境变量不存在".to_string())?;
        let state_file = app_data
            .join("CS2 Insight Agent")
            .join("data")
            .join("desktop-ui-state-v1.json");
        if !state_file.is_file() {
            return Ok(None);
        }
        fs::read_to_string(&state_file)
            .map(Some)
            .map_err(|error| format!("无法读取旧版界面状态 {}：{error}", state_file.display()))
    }

    #[cfg(not(windows))]
    Ok(None)
}

fn writable_data_root(_app: &AppHandle, root: &Path, python: &Path) -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        let app_data = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| "Windows APPDATA 环境变量不存在".to_string())?;
        let migration_script = root.join("backend/app/desktop_data_migration.py");
        if !migration_script.is_file() {
            return Err(format!(
                "未找到桌面数据迁移脚本：{}",
                migration_script.display()
            ));
        }

        let mut command = Command::new(python);
        command
            .arg("-I")
            .arg(&migration_script)
            .arg("--appdata")
            .arg(&app_data)
            .env("PYTHONNOUSERSITE", "1")
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command.creation_flags(CREATE_NO_WINDOW);
        let output = command
            .output()
            .map_err(|error| format!("无法执行桌面数据迁移：{error}"))?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(if detail.is_empty() {
                format!("桌面数据迁移失败，退出码：{}", output.status)
            } else {
                format!("桌面数据迁移失败：{detail}")
            });
        }

        let data_root = app_data.join("CS2 Insight Agent").join("data");
        fs::create_dir_all(data_root.join("logs"))
            .map_err(|error| format!("无法创建应用数据目录 {}：{error}", data_root.display()))?;
        Ok(data_root)
    }

    #[cfg(not(windows))]
    {
        let data_root = _app
            .path()
            .app_data_dir()
            .map_err(|error| format!("无法解析应用数据目录：{error}"))?
            .join("data");
        fs::create_dir_all(data_root.join("logs"))
            .map_err(|error| format!("无法创建应用数据目录 {}：{error}", data_root.display()))?;
        Ok(data_root)
    }
}

fn runtime_root(app: &AppHandle) -> Result<PathBuf, String> {
    let bundled_root = app
        .path()
        .resource_dir()
        .map_err(|error| format!("无法解析安装资源目录：{error}"))?;
    if bundled_root.join("backend/app/run_server.py").is_file()
        && bundled_root.join("python/python.exe").is_file()
    {
        return Ok(bundled_root);
    }
    if cfg!(debug_assertions) {
        return PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .map_err(|error| format!("无法解析开发目录：{error}"));
    }
    Ok(bundled_root)
}

fn python_executable(root: &Path) -> Option<PathBuf> {
    let candidates = if cfg!(debug_assertions) {
        vec![
            root.join(".venv/Scripts/python.exe"),
            root.join("python/python.exe"),
        ]
    } else {
        vec![root.join("python/python.exe")]
    };
    candidates.into_iter().find(|path| path.is_file())
}

fn append_desktop_log(logs_dir: &Path, message: &str) {
    let path = logs_dir.join("desktop.log");
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        use std::io::Write;
        let _ = writeln!(file, "{message}");
    }
}

fn start_backend(app: &AppHandle) -> Result<(), String> {
    let root = runtime_root(app)?;
    let python = python_executable(&root).ok_or_else(|| {
        format!(
            "未找到 Python 运行时。已检查 {}。",
            root.join("python/python.exe").display()
        )
    })?;
    let run_server = root.join("backend/app/run_server.py");
    if !run_server.is_file() {
        return Err(format!("未找到后端入口：{}", run_server.display()));
    }

    let data_root = writable_data_root(app, &root, &python)?;
    let logs_dir = data_root.join("logs");
    let backend_dir = root.join("backend");
    let bundle_data_dir = root.join("data");
    append_desktop_log(
        &logs_dir,
        &format!(
            "[desktop] starting backend: {} {}",
            python.display(),
            run_server.display()
        ),
    );

    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(logs_dir.join("backend-stdio.log"))
        .map_err(|error| format!("无法打开后端日志：{error}"))?;
    let stderr = stdout
        .try_clone()
        .map_err(|error| format!("无法复制后端日志句柄：{error}"))?;

    let instance_id = new_instance_id();
    let session_token = app.state::<BackendProcess>().session_token.clone();
    let mut command = Command::new(&python);
    command
        .arg(&run_server)
        .current_dir(&backend_dir)
        .env("CS2_INSIGHT_PORT", "19871")
        .env("CS2_INSIGHT_INSTANCE_ID", &instance_id)
        .env("CS2_INSIGHT_SESSION_TOKEN", &session_token)
        .env("PYTHONNOUSERSITE", "1")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONFAULTHANDLER", "1")
        .env(
            "CS2_INSIGHT_CONFIG",
            data_root.join("cs2-insight.config.json"),
        )
        .env("CS2_INSIGHT_LOG_DIR", &logs_dir)
        .env("CS2_INSIGHT_DATA_DIR", &data_root)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    if bundle_data_dir.is_dir() {
        command.env("CS2_INSIGHT_BUNDLE_DATA_DIR", bundle_data_dir);
    }
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let child = command
        .spawn()
        .map_err(|error| format!("无法启动 Python 后端：{error}"))?;

    // Register the child immediately so a window close during startup still
    // reaps the backend process instead of leaking it.
    {
        let state = app.state::<BackendProcess>();
        let mut backend_state = state
            .child
            .lock()
            .map_err(|_| "后端进程状态锁已损坏".to_string())?;
        *backend_state = Some(ManagedBackend {
            child,
            instance_id: instance_id.clone(),
            data_root,
        });
    }

    let mut verified = false;
    for _ in 0..120 {
        {
            let state = app.state::<BackendProcess>();
            let mut guard = state
                .child
                .lock()
                .map_err(|_| "后端进程状态锁已损坏".to_string())?;
            let Some(backend) = guard.as_mut() else {
                // stop_backend already took ownership: the app is shutting down.
                return Ok(());
            };
            if backend.child.try_wait().ok().flatten().is_some() {
                *guard = None;
                return Err(
                    "Python 后端在启动阶段退出，请查看应用数据目录中的 backend-stdio.log。"
                        .to_string(),
                );
            }
        }
        if backend_http("GET", "/api/app/runtime-state", &session_token)
            .is_some_and(|response| response.contains(&instance_id))
        {
            verified = true;
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    if !verified {
        let state = app.state::<BackendProcess>();
        if let Ok(mut guard) = state.child.lock() {
            if let Some(mut backend) = guard.take() {
                let _ = backend.child.kill();
                let _ = backend.child.wait();
            }
        }
        return Err(
            "Backend startup identity check failed; port 19871 may belong to another process."
                .to_string(),
        );
    }
    Ok(())
}

fn stop_backend(app: &AppHandle) {
    let state = app.state::<BackendProcess>();
    let session_token = state.session_token.clone();
    let Ok(mut guard) = state.child.lock() else {
        return;
    };
    let Some(mut backend) = guard.take() else {
        return;
    };
    drop(guard);
    if backend.child.try_wait().ok().flatten().is_some() {
        return;
    }

    let response = backend_http("POST", "/api/app/shutdown", &session_token);
    append_desktop_log(
        &backend.data_root.join("logs"),
        &format!(
            "[desktop] shutdown requested for instance {} response={}",
            backend.instance_id,
            response.as_deref().unwrap_or("unavailable")
        ),
    );
    for _ in 0..180 {
        if backend.child.try_wait().ok().flatten().is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }

    let _ = fs::write(
        backend.data_root.join("recovery-required.json"),
        "{\"reason\":\"desktop forced backend termination after graceful shutdown timeout\"}\n",
    );

    #[cfg(windows)]
    {
        let mut taskkill = Command::new("taskkill");
        taskkill.args(["/pid", &backend.child.id().to_string(), "/f", "/t"]);
        taskkill.creation_flags(CREATE_NO_WINDOW);
        let _ = taskkill.status();
    }
    let _ = backend.child.kill();
    let _ = backend.child.wait();
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(StateFlags::POSITION | StateFlags::SIZE)
                .with_filter(|label| label.starts_with("league-"))
                .build(),
        )
        .manage(BackendProcess::new().expect("failed to create desktop session token"))
        .manage(AppLifecycle::default())
        .manage(LeagueMiniLifecycle::default())
        .manage(LeagueOngoingLifecycle::default())
        .manage(LeagueCdTimerLifecycle::default())
        .manage(LeaguePrivacyLifecycle::default())
        .invoke_handler(tauri::generate_handler![
            read_legacy_ui_state,
            backend_session_token,
            set_close_to_tray,
            get_close_to_tray,
            set_close_action,
            get_close_action,
            hide_to_tray,
            quit_app,
            restart_as_administrator,
            open_league_mini,
            open_league_ongoing,
            open_league_cd_timer,
            toggle_league_aux_window,
            mark_league_window_ready,
            set_league_window_pinned,
            reset_league_window_position,
            persist_desktop_window_state,
            sync_league_mini,
            sync_league_ongoing,
            sync_league_cd_timer,
            set_league_content_protection
        ])
        .setup(|app| {
            let show_item = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "退出程序", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &quit_item])?;
            let mut tray = TrayIconBuilder::with_id("main-tray")
                .tooltip("MaxGameStudio")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main_window(app),
                    "quit" => request_app_exit(app),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main_window(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            // Start the backend on a worker thread so the window (and its
            // "connecting to backend" splash) appears immediately instead of
            // after the Python process answers HTTP.
            let handle = app.handle().clone();
            thread::spawn(move || {
                if let Err(error) = start_backend(&handle) {
                    handle
                        .dialog()
                        .message(format!(
                            "{error}\n\n请重新安装完整安装包，或查看应用数据目录中的日志。"
                        ))
                        .title("MaxGameStudio — 后端启动失败")
                        .kind(MessageDialogKind::Error)
                        .blocking_show();
                    handle.exit(1);
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build MaxGameStudio desktop shell");

    app.run(|handle, event| match event {
        RunEvent::WindowEvent {
            label,
            event: WindowEvent::CloseRequested { api, .. },
            ..
        } if label == "main" => {
            api.prevent_close();
            let lifecycle = handle.state::<AppLifecycle>();
            if lifecycle.quitting.load(Ordering::SeqCst) {
                return;
            }
            match lifecycle.close_action.load(Ordering::SeqCst) {
                CLOSE_ACTION_TRAY => {
                    if let Some(window) = handle.get_webview_window(&label) {
                        let _ = window.hide();
                    }
                }
                CLOSE_ACTION_EXIT => request_app_exit(handle),
                _ => {
                    let _ = handle.emit("desktop-close-choice-requested", ());
                }
            }
        }
        RunEvent::WindowEvent {
            label,
            event: WindowEvent::CloseRequested { api, .. },
            ..
        } if label == "league-mini" => {
            if handle
                .state::<AppLifecycle>()
                .quitting
                .load(Ordering::SeqCst)
            {
                return;
            }
            api.prevent_close();
            let mini = handle.state::<LeagueMiniLifecycle>();
            mini.manually_hidden.store(true, Ordering::SeqCst);
            mini.should_show.store(false, Ordering::SeqCst);
            mini.focus_requested.store(false, Ordering::SeqCst);
            if let Some(window) = handle.get_webview_window(&label) {
                let _ = window.hide();
            }
            save_league_window_state_best_effort(handle);
        }
        RunEvent::WindowEvent {
            label,
            event: WindowEvent::CloseRequested { api, .. },
            ..
        } if label == "league-cd-timer" => {
            if handle
                .state::<AppLifecycle>()
                .quitting
                .load(Ordering::SeqCst)
            {
                return;
            }
            api.prevent_close();
            let lifecycle = handle.state::<LeagueCdTimerLifecycle>();
            lifecycle.manually_hidden.store(true, Ordering::SeqCst);
            lifecycle.should_show.store(false, Ordering::SeqCst);
            lifecycle.focus_requested.store(false, Ordering::SeqCst);
            if let Some(window) = handle.get_webview_window(&label) {
                let _ = window.hide();
            }
            save_league_window_state_best_effort(handle);
        }
        RunEvent::WindowEvent {
            label,
            event: WindowEvent::CloseRequested { api, .. },
            ..
        } if label == "league-ongoing" => {
            if handle
                .state::<AppLifecycle>()
                .quitting
                .load(Ordering::SeqCst)
            {
                return;
            }
            api.prevent_close();
            let lifecycle = handle.state::<LeagueOngoingLifecycle>();
            lifecycle.manually_hidden.store(true, Ordering::SeqCst);
            lifecycle.should_show.store(false, Ordering::SeqCst);
            lifecycle.focus_requested.store(false, Ordering::SeqCst);
            if let Some(window) = handle.get_webview_window(&label) {
                let _ = window.hide();
            }
            save_league_window_state_best_effort(handle);
        }
        RunEvent::ExitRequested { code, api, .. } => {
            // The last window closing must not tear down the process while the
            // worker thread is still stopping the backend; explicit exit()
            // calls (which carry a code) pass through.
            if code.is_none() {
                api.prevent_exit();
            }
        }
        RunEvent::Exit => stop_backend(handle),
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    use super::{
        close_action_name, league_window_defaults, mini_auto_context_allows_show,
        new_session_token, ongoing_auto_context_allows_show, parse_close_action,
    };

    #[test]
    fn session_token_is_256_bit_hex() {
        let token = new_session_token().expect("OS random source should be available");
        assert_eq!(token.len(), 64);
        assert!(token.bytes().all(|value| value.is_ascii_hexdigit()));
    }

    #[test]
    fn close_action_round_trips_supported_values() {
        for expected in ["ask", "tray", "exit"] {
            assert_eq!(
                close_action_name(parse_close_action(expected).unwrap()),
                expected
            );
        }
        assert!(parse_close_action("unsupported").is_err());
    }

    #[test]
    fn league_auxiliary_window_defaults_are_stable() {
        assert_eq!(
            league_window_defaults("mini"),
            Some(("league-mini", 340.0, 420.0))
        );
        assert_eq!(
            league_window_defaults("ongoing"),
            Some(("league-ongoing", 1360.0, 840.0))
        );
        assert_eq!(
            league_window_defaults("cooldown"),
            Some(("league-cd-timer", 132.0, 252.0))
        );
        assert_eq!(league_window_defaults("unknown"), None);
    }

    #[test]
    fn mini_auto_context_rejects_in_progress_and_unknown_phases() {
        for context in [
            "connected:InProgress:playing",
            "connected:Reconnect:playing",
            "connected:None:playing",
            "offline:WaitingForStats:playing",
            "connected::playing",
        ] {
            assert!(!mini_auto_context_allows_show(context), "{context}");
        }
        for context in [
            "connected:Lobby:playing",
            "connected:Matchmaking:playing",
            "connected:ReadyCheck:playing",
            "connected:ChampSelect:playing",
        ] {
            assert!(mini_auto_context_allows_show(context), "{context}");
        }
    }

    #[test]
    fn ongoing_auto_context_only_allows_connected_game_phases() {
        for context in [
            "connected:GameStart:ARAM",
            "connected:InProgress:CLASSIC",
            "connected:Reconnect:unknown",
        ] {
            assert!(ongoing_auto_context_allows_show(context), "{context}");
        }
        for context in [
            "offline:InProgress:ARAM",
            "connected:Lobby:ARAM",
            "connected:ChampSelect:ARAM",
            "connected:None:unknown",
            "connected::unknown",
        ] {
            assert!(!ongoing_auto_context_allows_show(context), "{context}");
        }
    }

    #[test]
    fn mini_auto_hide_destroys_the_stale_webview() {
        let source = include_str!("lib.rs");
        let start = source
            .find("async fn sync_league_mini")
            .expect("mini sync command should exist");
        let command = &source[start..];
        let end = command
            .find("#[tauri::command]")
            .expect("next command should delimit mini sync command");
        let command = &command[..end];
        assert!(command.contains("window.destroy()"));
        assert!(command.contains("mini.ready.store(false"));
        assert!(command.contains("mini.bootstrapping.store(false"));
    }

    #[test]
    fn league_html_aux_windows_use_safe_visibility_before_react_ready() {
        let source = include_str!("lib.rs");
        let section = |marker: &str, next_marker: &str| {
            let start = source
                .find(marker)
                .expect("auxiliary window helper should exist");
            let helper = &source[start..];
            let end = helper.find(next_marker).unwrap_or(helper.len());
            &helper[..end]
        };

        let mini = section("async fn show_league_mini", "#[tauri::command]");
        assert!(mini.contains(".visible(false)"));
        assert!(mini.contains("!mini.ready.load"));
        assert!(mini.contains("WebviewUrl::App(\"mini.html\".into())"));

        let ongoing = section("async fn show_league_ongoing", "#[tauri::command]");
        assert!(ongoing.contains("build_league_ongoing_window"));
        assert!(ongoing.contains("!lifecycle.ready.load"));

        let ongoing_builder = section(
            "fn build_league_ongoing_window",
            "async fn show_league_ongoing",
        );
        assert!(ongoing_builder.contains(".visible(true)"));
        assert!(ongoing_builder.contains(".background_color(Color(17, 18, 20, 255))"));
        assert!(!ongoing_builder.contains(".transparent(true)"));
        assert!(ongoing_builder.contains("WebviewUrl::App(\"ongoing.html\".into())"));

        let cd_timer = section("async fn show_league_cd_timer", "#[tauri::command]");
        assert!(cd_timer.contains(".visible(false)"));
        assert!(cd_timer.contains("!lifecycle.ready.load"));
        assert!(cd_timer.contains("WebviewUrl::App(\"cd-timer.html\".into())"));
    }

    #[test]
    fn league_html_aux_windows_recheck_desired_visibility_after_bootstrap() {
        let source = include_str!("lib.rs");
        for (marker, lifecycle) in [
            ("async fn show_league_mini", "mini.should_show.load"),
            ("async fn show_league_ongoing", "lifecycle.should_show.load"),
            (
                "async fn show_league_cd_timer",
                "lifecycle.should_show.load",
            ),
        ] {
            let start = source
                .find(marker)
                .expect("auxiliary show helper should exist");
            let command = &source[start..];
            let next = command.find("#[tauri::command]").unwrap_or(command.len());
            let command = &command[..next];
            assert!(command.contains(lifecycle));
            assert!(command.contains("bootstrapping"));
        }
        assert!(source.contains("mini.should_show.store(should_show"));
        assert!(source.contains("fn mark_league_window_ready"));
        assert!(source.contains("lifecycle.ready.store(true"));
    }

    #[test]
    fn ongoing_window_is_not_created_always_on_top() {
        let source = include_str!("lib.rs");
        let start = source
            .find("async fn show_league_ongoing")
            .expect("ongoing window builder should exist");
        let command = &source[start..];
        let end = command
            .find("#[tauri::command]")
            .expect("next command should delimit ongoing builder");
        assert!(!command[..end].contains(".always_on_top(true)"));
    }

    #[test]
    fn ongoing_window_is_created_on_demand_instead_of_hidden_at_startup() {
        let source = include_str!("lib.rs");
        let setup = source
            .split_once(".setup(|app|")
            .map(|(_, body)| body)
            .expect("Tauri setup should exist");
        let setup_body = setup
            .split_once(".build(tauri::generate_context!())")
            .map(|(body, _)| body)
            .expect("setup should finish before the app build");
        assert!(!setup_body.contains("prime_league_ongoing_window"));
        assert!(!source.contains("fn prime_league_ongoing_window"));

        let builder_start = source
            .find("fn build_league_ongoing_window")
            .expect("ongoing window builder should exist");
        let builder = &source[builder_start..];
        let builder_end = builder
            .find("async fn show_league_ongoing")
            .expect("ongoing show helper should follow builder");
        let builder = &builder[..builder_end];
        assert!(builder.contains("WebviewUrl::App(\"ongoing.html\".into())"));
        assert!(builder.contains(".visible(true)"));
        assert!(builder.contains(".background_color(Color(17, 18, 20, 255))"));
        assert!(!builder.contains(".transparent(true)"));

        let show_start = source
            .find("async fn show_league_ongoing")
            .expect("ongoing show helper should exist");
        let show = &source[show_start..];
        let show_end = show
            .find("#[tauri::command]")
            .expect("next command should delimit ongoing show helper");
        let show = &show[..show_end];
        assert!(show.contains("compare_exchange(false, true"));
        assert!(show.contains("window.hide()"));
    }

    #[test]
    fn mini_window_is_not_created_always_on_top() {
        let source = include_str!("lib.rs");
        let start = source
            .find("async fn show_league_mini")
            .expect("mini window builder should exist");
        let command = &source[start..];
        let end = command
            .find("#[tauri::command]")
            .expect("next command should delimit mini builder");
        assert!(!command[..end].contains(".always_on_top(true)"));
    }
}
