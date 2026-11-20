//! Local OP.GG companion for a verified draft. The guide opens in the user's browser.

use crate::{
    aram_automation::SelectionScope,
    client_discovery::ClientInstance,
    client_runtime::ClientRuntime,
    desktop::{close_opgg_companion, show_opgg_companion},
    lcu::GameflowSession,
};
use serde_json::{json, Value};
use std::sync::Mutex;
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

pub(crate) const NAMESPACE: &str = "opgg-guide-main";
const LABEL: &str = "opgg";

#[derive(Clone, Copy, PartialEq, Eq)]
enum OpggMode {
    Aram,
    Kiwi,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct OpggTarget {
    pub(crate) scope: SelectionScope,
    pub(crate) champion_id: Option<u32>,
    mode: OpggMode,
}

impl OpggTarget {
    pub(crate) fn from_sample(
        instance: ClientInstance,
        flow: &GameflowSession,
        select: &Value,
    ) -> Option<Self> {
        let scope = SelectionScope::from_sample(instance, flow, select)?;
        let mode = if flow.is_hextech_aram() {
            OpggMode::Kiwi
        } else {
            OpggMode::Aram
        };
        let cell = select["localPlayerCellId"].as_i64()?;
        let player = select["myTeam"]
            .as_array()?
            .iter()
            .find(|entry| entry["cellId"].as_i64() == Some(cell))?;
        let champion_id = player["championId"]
            .as_u64()
            .and_then(|id| u32::try_from(id).ok())
            .filter(|&id| id > 0);
        Some(Self {
            scope,
            champion_id,
            mode,
        })
    }

    pub(crate) fn view(self) -> Value {
        json!({
            "gameMode": match self.mode { OpggMode::Aram => "ARAM", OpggMode::Kiwi => "KIWI" },
            "championId": self.champion_id,
        })
    }

    pub(crate) fn guide_url(self, alias: Option<&str>) -> String {
        let mode_page = match self.mode {
            OpggMode::Aram => "https://op.gg/lol/modes/aram",
            OpggMode::Kiwi => "https://op.gg/lol/modes/aram-mayhem",
        };
        // Aliases come from the verified local catalog. Accept one URL path
        // segment and use the official mode page if the name is unavailable.
        let champion = alias.filter(|alias| {
            self.champion_id.is_some()
                && (1..=32).contains(&alias.len())
                && alias.bytes().all(|byte| byte.is_ascii_alphanumeric())
                && alias.as_bytes()[0].is_ascii_alphabetic()
        });
        match champion {
            Some(alias) => format!("{mode_page}/{}/build", alias.to_ascii_lowercase()),
            None => mode_page.to_owned(),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct OpggPreferences {
    pub(crate) enabled: bool,
    pub(crate) auto_show: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum OpggDecision {
    Close,
    Show,
    Ignore,
}

struct OpggState {
    preferences: OpggPreferences,
    announced: Option<SelectionScope>,
}

impl OpggState {
    fn new(preferences: OpggPreferences) -> Self {
        Self {
            preferences,
            announced: None,
        }
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.preferences.enabled = enabled;
        if !enabled {
            self.announced = None;
        }
    }

    fn decision(&mut self, target: Option<OpggTarget>) -> OpggDecision {
        let Some(target) = target.filter(|_| self.preferences.enabled) else {
            return OpggDecision::Close;
        };
        if self.preferences.auto_show && self.announced != Some(target.scope) {
            // Closing or minimizing a window does not re-open it on the next poll.
            self.announced = Some(target.scope);
            OpggDecision::Show
        } else {
            OpggDecision::Ignore
        }
    }
}

pub(crate) struct OpggWindow(Mutex<OpggState>);

impl OpggWindow {
    pub(crate) fn new(preferences: OpggPreferences) -> Self {
        Self(Mutex::new(OpggState::new(preferences)))
    }

    pub(crate) fn enabled(&self) -> bool {
        self.0.lock().is_ok_and(|state| state.preferences.enabled)
    }

    pub(crate) fn update_setting(&self, key: &str, value: bool) -> Result<(), &'static str> {
        let mut state = self.0.lock().map_err(|_| "OP.GG 设置暂不可用。")?;
        match key {
            "enabled" => state.set_enabled(value),
            "autoShow" => state.preferences.auto_show = value,
            _ => return Err("未知的 OP.GG 设置。"),
        }
        Ok(())
    }

    pub(crate) fn target(&self, app: &AppHandle) -> Option<OpggTarget> {
        if self.enabled() {
            app.state::<ClientRuntime>().opgg_target()
        } else {
            None
        }
    }

    pub(crate) fn mark_shown(&self, scope: SelectionScope) {
        if let Ok(mut state) = self.0.lock() {
            state.announced = Some(scope);
        }
    }

    pub(crate) fn reconcile(&self, app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
        let target = app.state::<ClientRuntime>().opgg_target();
        let decision = self
            .0
            .lock()
            .map_err(|_| "OP.GG 设置暂不可用。")?
            .decision(target);
        match decision {
            OpggDecision::Close => close_opgg_companion(app)?,
            OpggDecision::Show => {
                if app
                    .get_webview_window(LABEL)
                    .is_none_or(|window| !window.is_visible().unwrap_or(false))
                {
                    show_opgg_companion(app)?;
                }
            }
            OpggDecision::Ignore => {}
        }
        Ok(())
    }

    pub(crate) fn open_guide(&self, app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
        let runtime = app.state::<ClientRuntime>();
        let (target, url) = runtime
            .opgg_guide()
            .ok_or("当前没有可用的大乱斗选人会话。")?;
        if self.target(app) != Some(target) {
            return Err("大乱斗选人已结束或 OP.GG 已关闭。".into());
        }
        app.opener().open_url(url, None::<&str>)?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "opgg_window_tests.rs"]
mod tests;
