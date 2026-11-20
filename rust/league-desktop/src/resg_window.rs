//! A lazily opened WebView2 for the public chart for the current Hextech ARAM champion.

use crate::{
    client_runtime::{ClientRuntime, ResgTarget},
    desktop::emit,
    profile::DesktopProfile,
};
use maxgamestudio_league_core::resg::{
    is_resg_content, is_resg_page, resg_champion_url, RESG_NAMESPACE,
};
use serde_json::{json, Value};
use std::{sync::Mutex, time::Duration};
use tauri::{
    webview::{NewWindowResponse, PageLoadEvent},
    AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};

pub(crate) const LABEL: &str = "resg";
const TITLE: &str = "RESG · 海克斯大乱斗";
const CHART_WAIT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResgAction {
    Synchronize,
    Show,
    Close,
    Retry,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Waiting,
    Loading,
    Ready,
    Error(&'static str),
}

struct ViewState {
    preferences: Value,
    target: Option<ResgTarget>,
    auto_shown: bool,
    loaded: bool,
    display: Option<bool>,
    display_fallback: bool,
    status: Status,
    navigation: u64,
    published: Value,
}

impl ViewState {
    fn enabled(&self) -> bool {
        self.preferences["enabled"] == true
    }

    fn snapshot(&self, visible: bool) -> Value {
        let mut result = self.preferences.clone();
        result["visible"] = json!(visible);
        result["available"] = json!(self.target.is_some());
        result["championId"] = json!(self.target.map_or(0, |target| target.champion_id));
        let (status, error) = match self.status {
            _ if !self.enabled() => ("disabled", None),
            Status::Waiting => ("waiting", None),
            Status::Loading => ("loading", None),
            Status::Ready => ("ready", None),
            Status::Error(reason) => ("error", Some(reason)),
        };
        result["status"] = json!(status);
        result["error"] = json!(error);
        result["displayFallback"] = json!(self.display_fallback);
        result
    }

    fn publish(&mut self, app: &AppHandle) -> Value {
        let visible = app
            .get_webview_window(LABEL)
            .is_some_and(|window| window.is_visible().unwrap_or(false));
        let snapshot = self.snapshot(visible);
        if snapshot != self.published {
            if let Some(main) = app.get_webview_window("main") {
                let _ = emit(&main, RESG_NAMESPACE, "changed", json!([snapshot]));
            }
            self.published = snapshot.clone();
        }
        snapshot
    }

    fn invalidate_navigation(&mut self) {
        self.navigation += 1;
        self.loaded = false;
        self.display = None;
        self.display_fallback = false;
        self.status = Status::Waiting;
    }
}

pub(crate) struct ResgWindow(Mutex<ViewState>);

impl ResgWindow {
    pub(crate) fn new(preferences: Value) -> Self {
        Self(Mutex::new(ViewState {
            preferences,
            target: None,
            auto_shown: false,
            loaded: false,
            display: None,
            display_fallback: false,
            status: Status::Waiting,
            navigation: 0,
            published: Value::Null,
        }))
    }

    pub(crate) fn enabled(&self) -> bool {
        self.0.lock().is_ok_and(|state| state.enabled())
    }

    pub(crate) fn replace_preferences(&self, preferences: Value) -> Result<(), &'static str> {
        self.0.lock().map_err(|_| "RESG 状态不可用。")?.preferences = preferences;
        Ok(())
    }

    pub(crate) fn reconcile(&self, app: &AppHandle, action: ResgAction) -> Result<Value, String> {
        let mut state = self.0.lock().map_err(|_| "RESG 状态不可用。")?;
        let previous = state.target;
        state.target = app
            .state::<ClientRuntime>()
            .resg_target()
            .filter(|_| state.enabled());
        let mut window = app.get_webview_window(LABEL);
        let Some(target) = state.target else {
            state.auto_shown = false;
            if window.is_some() || previous.is_some() {
                state.invalidate_navigation();
            }
            if let Some(window) = window {
                window.destroy().map_err(|error| error.to_string())?;
            }
            return Ok(state.publish(app));
        };

        let new_match = previous.is_none_or(|old| {
            old.instance != target.instance
                || matches!((old.game_id, target.game_id), (Some(a), Some(b)) if a != b)
        });
        if new_match {
            state.auto_shown = false;
        }
        if action == ResgAction::Close {
            state.auto_shown = true;
            state.invalidate_navigation();
            if let Some(window) = window {
                window.destroy().map_err(|error| error.to_string())?;
            }
            return Ok(state.publish(app));
        }

        let manual = matches!(action, ResgAction::Show | ResgAction::Retry);
        let automatic = !state.auto_shown && state.preferences["autoShow"] == true;
        let pinned = state.preferences["alwaysOnTop"] == true;
        let opening = window.is_none() && (manual || automatic);
        let navigate = opening
            || (window.is_some()
                && (new_match
                    || action == ResgAction::Retry
                    || previous.map(|old| old.champion_id) != Some(target.champion_id)));

        if navigate {
            let destination =
                resg_champion_url(target.champion_id).ok_or("无效的 RESG 英雄编号。")?;
            state.invalidate_navigation();
            state.status = Status::Loading;
            state.auto_shown = true;
            let started = if let Some(active) = window.as_ref() {
                let _ = active.set_title(TITLE);
                active.navigate(destination).map(|_| active.clone())
            } else {
                open(app, destination, manual, pinned)
            };
            match started {
                Ok(active) => {
                    tauri::async_runtime::spawn(watch_chart(
                        active.clone(),
                        target.champion_id,
                        state.navigation,
                    ));
                    window = Some(active);
                }
                Err(_) => state.status = Status::Error("page-unavailable"),
            }
        } else if window.is_none() && state.status != Status::Error("page-unavailable") {
            state.invalidate_navigation();
        }

        if let Some(window) = window {
            apply_display(&window, &mut state);
            if state.published["alwaysOnTop"] != pinned {
                let _ = window.set_always_on_top(pinned);
            }
            if manual {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }
        Ok(state.publish(app))
    }
}

fn open(
    app: &AppHandle,
    url: tauri::Url,
    focus: bool,
    pinned: bool,
) -> tauri::Result<WebviewWindow> {
    let webview_directory = app.state::<DesktopProfile>().webview_directory()?;
    WebviewWindowBuilder::new(app, LABEL, WebviewUrl::External(url))
        .data_directory(webview_directory)
        .title(TITLE)
        .inner_size(930.0, 710.0)
        .min_inner_size(480.0, 420.0)
        .focused(focus)
        .always_on_top(pinned)
        .incognito(true)
        .on_navigation(is_resg_page)
        .on_new_window(|_, _| NewWindowResponse::Deny)
        .on_download(|_, _| false)
        .on_page_load(|window, payload| {
            if payload.event() == PageLoadEvent::Finished && is_resg_content(payload.url()) {
                let champion = payload
                    .url()
                    .path()
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .and_then(|part| part.parse::<u32>().ok());
                tauri::async_runtime::spawn(async move {
                    let controller = window.state::<ResgWindow>();
                    let Ok(mut state) = controller.0.lock() else {
                        return;
                    };
                    if state.status == Status::Loading
                        && state
                            .target
                            .is_some_and(|target| champion == Some(target.champion_id))
                    {
                        state.loaded = true;
                    }
                });
            }
        })
        .build()
}

async fn watch_chart(window: WebviewWindow, champion: u32, ticket: u64) {
    let controller = window.state::<ResgWindow>();
    let deadline = tokio::time::Instant::now() + CHART_WAIT;
    // A 200 response only loads the shell. The public hero element signals the loaded champion data.
    let script = format!(
        r#"(() => {{
        const route = /^\/v\/[^/]+\/champions\/{champion}\/?$/;
        const name = document.querySelector('#app .entity-hero .entity-identity h1')?.textContent?.trim();
        return route.test(location.pathname) && name ? name.slice(0, 96) : null;
    }})()"#
    );

    loop {
        let loaded = {
            let Ok(state) = controller.0.lock() else {
                return;
            };
            if state.navigation != ticket || state.status != Status::Loading {
                return;
            }
            state.loaded
        };
        if loaded {
            let deadline = deadline.min(tokio::time::Instant::now() + Duration::from_millis(750));
            if let Some(name) = evaluate::<String>(&window, &script, deadline).await {
                finish(&window, &controller, ticket, Status::Ready, Some(&name));
                return;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            finish(
                &window,
                &controller,
                ticket,
                Status::Error("champion-unavailable"),
                None,
            );
            return;
        }
        tokio::time::sleep(Duration::from_millis(180)).await;
    }
}

async fn evaluate<T: serde::de::DeserializeOwned>(
    window: &WebviewWindow,
    script: &str,
    deadline: tokio::time::Instant,
) -> Option<T> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let tx = Mutex::new(Some(tx));
    window
        .eval_with_callback(script, move |value| {
            if let Some(sender) = tx.lock().expect("callback only drops the sender").take() {
                let _ = sender.send(value);
            }
        })
        .ok()?;
    let value = tokio::time::timeout_at(deadline, rx).await.ok()?.ok()?;
    serde_json::from_str(&value).ok()
}

fn apply_display(window: &WebviewWindow, state: &mut ViewState) {
    if state.status != Status::Ready {
        return;
    }
    let compact = state.preferences["displayMode"] == "compact";
    if state.display == Some(compact) {
        return;
    }
    state.display = Some(compact);
    state.display_fallback = false;
    let champion = state
        .target
        .expect("loaded chart has a champion")
        .champion_id;
    let app = window.app_handle();
    let Some(program) = app.asset_resolver().get("resg-display.js".to_owned()) else {
        state.display_fallback = compact;
        return;
    };
    let Ok(program) = String::from_utf8(program.bytes) else {
        state.display_fallback = compact;
        return;
    };
    let english = app
        .state::<DesktopProfile>()
        .system_locale
        .starts_with("en");
    let dark = window.theme().unwrap_or(tauri::Theme::Dark) == tauri::Theme::Dark;
    let mode = if compact { "compact" } else { "all" };
    // WebView2's callback returns before a Promise settles. Expose only the layout result as a local DOM attribute.
    let script = format!(
        r#"(() => {{
        if (!/^\/v\/[^/]+\/champions\/{champion}\/?$/.test(location.pathname)) return;
        document.body.removeAttribute('data-mgs-resg-result');
        {program}('{mode}', {champion}, {english}, {dark}).then((result) => {{
            document.body.setAttribute('data-mgs-resg-result', result.kind);
        }});
    }})()"#
    );
    if window.eval(script).is_err() {
        state.display_fallback = compact;
    } else if compact {
        tauri::async_runtime::spawn(watch_layout(window.clone(), state.navigation));
    }
}

async fn watch_layout(window: WebviewWindow, ticket: u64) {
    let controller = window.state::<ResgWindow>();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let fallback = loop {
        if !controller
            .0
            .lock()
            .is_ok_and(|state| state.navigation == ticket && state.display == Some(true))
        {
            return;
        }
        match evaluate::<String>(
            &window,
            "document.body.getAttribute('data-mgs-resg-result')",
            deadline,
        )
        .await
        .as_deref()
        {
            Some("compact") => break false,
            Some("fallback") => break true,
            _ if tokio::time::Instant::now() >= deadline => break true,
            _ => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    };
    let Ok(mut state) = controller.0.lock() else {
        return;
    };
    if state.navigation == ticket && state.display == Some(true) {
        state.display_fallback = fallback;
        state.publish(window.app_handle());
    }
}

fn finish(
    window: &WebviewWindow,
    controller: &ResgWindow,
    ticket: u64,
    status: Status,
    name: Option<&str>,
) {
    let Ok(mut state) = controller.0.lock() else {
        return;
    };
    if state.navigation != ticket || state.status != Status::Loading {
        return;
    }
    state.status = status;
    if let Some(name) = name {
        let _ = window.set_title(&format!("{TITLE} · {name}"));
    }
    apply_display(window, &mut state);
    state.publish(window.app_handle());
}
