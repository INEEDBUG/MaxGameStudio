use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CompanionWindowKind {
    Aux,
    Opgg,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanionWindowState {
    pub enabled: bool,
    pub initialized: bool,
    pub connected: bool,
    pub auto_show: bool,
    pub ready: bool,
    pub phase: Option<String>,
    pub game_mode: Option<String>,
    pub spectating: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowDemand {
    Ignore,
    Prepare,
    Show,
    Hide,
}

pub fn companion_window_demand(
    kind: CompanionWindowKind,
    state: &CompanionWindowState,
) -> WindowDemand {
    use CompanionWindowKind::{Aux, Opgg};
    use WindowDemand::{Hide, Ignore, Prepare, Show};

    if !state.enabled || !state.initialized || !state.connected || !state.auto_show {
        return Ignore;
    }
    match (kind, state.phase.as_deref()) {
        (Aux, Some("ChampSelect")) if state.spectating == Some(true) => Ignore,
        (_, Some("ChampSelect")) | (Aux, Some("Lobby" | "Matchmaking" | "ReadyCheck")) => {
            if !matches!(state.game_mode.as_deref(), Some("ARAM" | "KIWI")) {
                Ignore
            } else if state.ready {
                Show
            } else {
                Prepare
            }
        }
        // A manually opened companion must not change the idle decision when it becomes ready.
        (Aux, _) => Hide,
        (Opgg, _) => Ignore,
    }
}
