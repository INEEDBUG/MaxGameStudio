//! Disconnect-safe snapshots from the local League client for the existing renderer.

use crate::{
    client_discovery::{discover_clients, ClientInstance, LocalClient},
    desktop::emit,
    lcu::{LcuError, LcuTransport},
    resg_window::{ResgAction, ResgWindow},
};
pub(crate) use maxgamestudio_league_core::shell_preferences::LEAGUE_CLIENT as CLIENT;
use serde_json::{json, Value};
use std::{sync::Mutex, time::Duration};
use tauri::{AppHandle, Manager};

const CLIENT_DISCOVERY: &str = "league-client-ux-main";

#[derive(Default, PartialEq)]
struct Snapshot {
    known_clients: Vec<ClientInstance>,
    active: Option<ActiveClient>,
}

#[derive(PartialEq)]
struct ActiveClient {
    instance: ClientInstance,
    phase: String,
    session: Value,
    summoner: Value,
    resg: Option<ResgTarget>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResgTarget {
    pub(crate) instance: ClientInstance,
    pub(crate) game_id: Option<u64>,
    pub(crate) champion_id: u32,
}

fn client_identity(instance: ClientInstance) -> Value {
    // The lockfile is local and does not identify the Riot or Tencent shard.
    json!({
        "pid": instance.pid,
        "processCreationTime": instance.created_at.to_string(),
        "region": "",
        "rsoPlatformId": "",
    })
}

pub(crate) fn client_from_view(value: &Value) -> Option<ClientInstance> {
    Some(ClientInstance {
        pid: u32::try_from(value["pid"].as_u64()?).ok()?,
        created_at: value["processCreationTime"].as_str()?.parse().ok()?,
    })
}

impl Snapshot {
    fn fields(&self, namespace: &str, name: &str) -> Option<Value> {
        let active = self.active.as_ref();
        match (namespace, name) {
            (CLIENT_DISCOVERY, "state") => Some(json!({
                "launchedClients": self.known_clients.iter().copied().map(client_identity).collect::<Vec<_>>(),
            })),
            (CLIENT, "state") => Some(json!({
                "connectionState": if active.is_some() { "connected" } else { "disconnected" },
                "auth": active.map(|client| client_identity(client.instance)),
                "connectingClient": null,
            })),
            (CLIENT, "gameflow") => Some(json!({
                "phase": active.map(|client| client.phase.as_str()),
                "session": active.map(|client| &client.session),
            })),
            (CLIENT, "summoner") => Some(json!({"me": active.map(|client| &client.summoner)})),
            _ => None,
        }
    }
}

const PUBLIC_STATES: [(&str, &str); 4] = [
    (CLIENT_DISCOVERY, "state"),
    (CLIENT, "state"),
    (CLIENT, "gameflow"),
    (CLIENT, "summoner"),
];

#[derive(Default)]
struct RuntimeState {
    generation: u64,
    revision: u64,
    auto_connect: bool,
    manual_client: Option<ClientInstance>,
    manually_disconnected: bool,
    snapshot: Snapshot,
}

pub(crate) struct ClientRuntime {
    transport: LcuTransport,
    state: Mutex<RuntimeState>,
    wake: tokio::sync::Notify,
}

impl ClientRuntime {
    pub(crate) fn new(auto_connect: bool) -> Result<Self, LcuError> {
        Ok(Self {
            transport: LcuTransport::new()?,
            state: Mutex::new(RuntimeState {
                auto_connect,
                ..Default::default()
            }),
            wake: tokio::sync::Notify::new(),
        })
    }

    pub(crate) fn fields(
        &self,
        namespace: &str,
        name: &str,
    ) -> Result<Option<(u64, Value)>, &'static str> {
        let state = self.state.lock().map_err(|_| "客户端状态不可用。")?;
        Ok(state
            .snapshot
            .fields(namespace, name)
            .map(|fields| (state.revision, fields)))
    }

    pub(crate) fn resg_target(&self) -> Option<ResgTarget> {
        self.state.lock().ok()?.snapshot.active.as_ref()?.resg
    }

    pub(crate) fn refresh_soon(&self) {
        self.wake.notify_one();
    }

    pub(crate) fn set_auto_connect(&self, enabled: bool) -> Result<(), &'static str> {
        let mut state = self.state.lock().map_err(|_| "客户端状态不可用。")?;
        if state.auto_connect != enabled {
            state.auto_connect = enabled;
            if enabled {
                state.manually_disconnected = false;
            }
            state.generation += 1;
            self.wake.notify_one();
        }
        Ok(())
    }

    pub(crate) fn select(
        &self,
        app: &AppHandle,
        selection: Option<ClientInstance>,
    ) -> Result<(), &'static str> {
        let mut state = self.state.lock().map_err(|_| "客户端状态不可用。")?;
        if selection.is_some_and(|client| !state.snapshot.known_clients.contains(&client)) {
            return Err("客户端已退出或尚未完成检测。");
        }
        state.generation += 1;
        state.manual_client = selection;
        state.manually_disconnected = selection.is_none();
        let previous = Snapshot {
            active: state.snapshot.active.take(),
            known_clients: state.snapshot.known_clients.clone(),
        };
        state.revision += 1;
        publish_changes(app, &previous, &state.snapshot, state.revision);
        self.wake.notify_one();
        Ok(())
    }

    async fn sample(
        &self,
        client: &LocalClient,
        resg_enabled: bool,
        previous_chart: Option<ResgTarget>,
    ) -> Result<Option<ActiveClient>, LcuError> {
        if !client.alive() {
            return Ok(None);
        }
        let target = client.target()?;
        let Some(phase) = self.transport.read_phase(&target).await? else {
            return Ok(None);
        };
        let summoner = self.transport.read_current_summoner(&target).await?;
        let session = self
            .transport
            .read_gameflow_session(&target)
            .await?
            .filter(|session| session.phase == phase);
        let mut resg = None;
        if let Some(game) = session
            .as_ref()
            .filter(|game| resg_enabled && game.is_hextech_aram())
        {
            let game_id = game
                .game_data
                .as_ref()
                .and_then(|data| data.game_id)
                .filter(|&id| id != 0);
            let selection = if phase == "ChampSelect" {
                // An optional chart request cannot hold up the connection status.
                tokio::time::timeout(
                    Duration::from_millis(750),
                    self.transport.read_champ_select(&target),
                )
                .await
                .ok()
                .and_then(Result::ok)
            } else {
                Some(Value::Null)
            };
            resg = if let Some(selection) = selection {
                let puuid = summoner.as_ref().map(|player| player.puuid.as_str());
                game.resg_champion(&selection, puuid)
                    .map(|champion_id| ResgTarget {
                        instance: client.instance,
                        game_id,
                        champion_id,
                    })
            } else {
                // Keep the current chart during an optional read timeout for the same verified game.
                previous_chart.filter(|previous| {
                    game_id.is_some()
                        && previous.game_id == game_id
                        && previous.instance == client.instance
                })
            };
        }
        Ok(client.alive().then(|| ActiveClient {
            instance: client.instance,
            phase,
            session: json!(session),
            summoner: json!(summoner),
            resg,
        }))
    }

    async fn refresh(&self, app: &AppHandle, resg_enabled: bool) {
        let (generation, manual, connected, auto_connect, previous_chart) = {
            let Ok(state) = self.state.lock() else {
                return;
            };
            (
                state.generation,
                state.manual_client,
                state.snapshot.active.as_ref().map(|client| client.instance),
                state.auto_connect && !state.manually_disconnected,
                state
                    .snapshot
                    .active
                    .as_ref()
                    .and_then(|client| client.resg),
            )
        };
        let clients = tauri::async_runtime::spawn_blocking(discover_clients)
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
        let find_client = |instance| clients.iter().find(|client| client.instance == instance);
        let candidate = if let Some(selection) = manual {
            find_client(selection)
        } else {
            connected
                .and_then(find_client)
                .or_else(|| (auto_connect && clients.len() == 1).then(|| &clients[0]))
        };
        let active = match candidate {
            Some(client) => tokio::time::timeout(
                Duration::from_secs(4),
                self.sample(client, resg_enabled, previous_chart),
            )
            .await
            .ok()
            .and_then(Result::ok)
            .flatten(),
            None => None,
        };
        let mut known_clients: Vec<_> = clients.iter().map(|client| client.instance).collect();
        known_clients.sort_unstable();
        let snapshot = Snapshot {
            known_clients,
            active,
        };
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if generation == state.generation {
            if snapshot.active.is_some()
                || state
                    .manual_client
                    .is_some_and(|client| !snapshot.known_clients.contains(&client))
            {
                state.manual_client = None;
            }
            if state.snapshot != snapshot {
                state.revision += 1;
                publish_changes(app, &state.snapshot, &snapshot, state.revision);
                state.snapshot = snapshot;
            }
        }
    }
}

pub(crate) async fn monitor(app: AppHandle) {
    let runtime = app.state::<ClientRuntime>();
    let resg = app.state::<ResgWindow>();
    loop {
        runtime.refresh(&app, resg.enabled()).await;
        let _ = resg.reconcile(&app, ResgAction::Synchronize);
        let _ = tokio::time::timeout(Duration::from_secs(2), runtime.wake.notified()).await;
    }
}

fn publish_changes(app: &AppHandle, before: &Snapshot, after: &Snapshot, revision: u64) {
    for (namespace, name) in PUBLIC_STATES {
        let (Some(Value::Object(before)), Some(Value::Object(after))) = (
            before.fields(namespace, name),
            after.fields(namespace, name),
        ) else {
            continue;
        };
        for (key, value) in after {
            if before.get(&key) == Some(&value) {
                continue;
            }
            let event = format!("update-state-prop/{namespace}:{name}");
            let args =
                json!([key, value, { "action": "update", "raw": false, "revision": revision }]);
            for label in ["main", "aux", "opgg"] {
                if let Some(window) = app.get_webview_window(label) {
                    let _ = emit(&window, "mobx-utils-main", &event, args.clone());
                }
            }
        }
    }
}
