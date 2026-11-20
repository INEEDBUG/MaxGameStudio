//! Disconnect-safe snapshots from the local League client for the existing renderer.

use crate::{
    client_discovery::{discover_clients, ClientInstance, LocalClient},
    desktop::emit,
    lcu::{player_search::PlayerSearchShard, LcuError, LcuTransport},
    resg_window::{ResgAction, ResgWindow},
};
pub(crate) use maxgamestudio_league_core::shell_preferences::LEAGUE_CLIENT as CLIENT;
use serde_json::{json, Value};
use std::{sync::Mutex, time::Duration};
use tauri::{AppHandle, Manager};

const CLIENT_DISCOVERY: &str = "league-client-ux-main";
const SGP: &str = "sgp-main";

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
    shard: Option<PlayerSearchShard>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResgTarget {
    pub(crate) instance: ClientInstance,
    pub(crate) game_id: Option<u64>,
    pub(crate) champion_id: u32,
}

fn client_identity(instance: ClientInstance, shard: Option<&PlayerSearchShard>) -> Value {
    json!({
        "pid": instance.pid,
        "processCreationTime": instance.created_at.to_string(),
        "region": shard.map_or("", |shard| shard.region.as_str()),
        "rsoPlatformId": shard.map_or("", |shard| shard.rso_platform.as_str()),
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
        let shard = active.and_then(|client| client.shard.as_ref());
        match (namespace, name) {
            (CLIENT_DISCOVERY, "state") => Some(json!({
                "launchedClients": self.known_clients.iter().copied().map(|instance| {
                    let known_shard = active.filter(|client| client.instance == instance).and(shard);
                    client_identity(instance, known_shard)
                }).collect::<Vec<_>>(),
            })),
            (CLIENT, "state") => Some(json!({
                "connectionState": if active.is_some() { "connected" } else { "disconnected" },
                "auth": active.map(|client| client_identity(client.instance, shard)),
                "connectingClient": null,
            })),
            (CLIENT, "gameflow") => Some(json!({
                "phase": active.map(|client| client.phase.as_str()),
                "session": active.map(|client| &client.session),
            })),
            (CLIENT, "summoner") => Some(json!({"me": active.map(|client| &client.summoner)})),
            (SGP, "state") => Some(json!({
                "availability": {
                    "region": shard.map_or("", |shard| shard.region.as_str()),
                    "rsoPlatform": shard.map_or("", |shard| shard.rso_platform.as_str()),
                    "sgpServerId": shard.map_or("", |shard| shard.sgp_server_id.as_str()),
                    "serversSupported": { "matchHistory": false, "common": false },
                },
                "isTokenReady": false, "supportedQueues": [],
                "connectionSuccessesCounted": 0, "connectionFailuresCounted": 0,
            })),
            _ => None,
        }
    }
}

const PUBLIC_STATES: [(&str, &str); 5] = [
    (CLIENT_DISCOVERY, "state"),
    (CLIENT, "state"),
    (CLIENT, "gameflow"),
    (CLIENT, "summoner"),
    (SGP, "state"),
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

    pub(crate) async fn lookup_player(&self, shard: &str, input: &str) -> Result<Value, String> {
        let (instance, own_puuid, generation) = {
            let state = self.state.lock().map_err(|_| "客户端状态不可用。")?;
            let active = state
                .snapshot
                .active
                .as_ref()
                .ok_or("请启动并登录英雄联盟客户端。")?;
            let own_puuid = active.summoner["puuid"]
                .as_str()
                .ok_or("请登录英雄联盟客户端后查询。")?;
            if active
                .shard
                .as_ref()
                .is_none_or(|active| active.sgp_server_id != shard)
            {
                return Err("当前区服不可用，请检查客户端登录状态。".to_owned());
            }
            (active.instance, own_puuid.to_owned(), state.generation)
        };
        let local = tauri::async_runtime::spawn_blocking(move || {
            discover_clients()
                .ok()?
                .into_iter()
                .find(|client| client.instance == instance)
        })
        .await
        .map_err(|_| "无法读取当前客户端。")?
        .ok_or("客户端已退出，请重新连接。")?;
        let target = local.target().map_err(|error| error.to_string())?;
        let result = tokio::time::timeout(
            Duration::from_secs(8),
            self.transport
                .lookup_player(&target, &own_puuid, Some(shard), input),
        )
        .await
        .map_err(|_| "玩家查询超时，请重试。")?
        .map_err(|error| error.to_string())?;
        let state = self.state.lock().map_err(|_| "客户端状态不可用。")?;
        let same_account = state.snapshot.active.as_ref().is_some_and(|active| {
            active.instance == instance
                && active.summoner["puuid"].as_str() == Some(own_puuid.as_str())
                && active
                    .shard
                    .as_ref()
                    .is_some_and(|current| current.sgp_server_id == shard)
        });
        if state.generation != generation || !same_account || !local.alive() {
            return Err("客户端或登录账号已变更，请重新查询。".to_owned());
        }
        serde_json::to_value(result).map_err(|error| error.to_string())
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
        previous_shard: Option<(String, PlayerSearchShard)>,
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
        let mut shard = None;
        if let Some(player) = summoner.as_ref() {
            shard = if let Some((_, shard)) =
                previous_shard.filter(|(puuid, _)| *puuid == player.puuid)
            {
                Some(shard)
            } else {
                tokio::time::timeout(
                    Duration::from_millis(550),
                    self.transport
                        .read_player_search_shard(&target, &player.puuid),
                )
                .await
                .ok()
                .and_then(Result::ok)
            };
        }
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
            shard,
        }))
    }

    async fn refresh(&self, app: &AppHandle, resg_enabled: bool) {
        let (generation, manual, connected, auto_connect, previous_chart, previous_shard) = {
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
                state.snapshot.active.as_ref().and_then(|client| {
                    Some((
                        client.summoner["puuid"].as_str()?.to_owned(),
                        client.shard.clone()?,
                    ))
                }),
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
                self.sample(
                    client,
                    resg_enabled,
                    previous_chart,
                    previous_shard.filter(|_| Some(client.instance) == connected),
                ),
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
