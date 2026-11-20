//! Disconnect-safe snapshots from the local League client for the existing renderer.

use crate::{
    aram_automation::{AramOptions, AramSchedule, SelectionScope},
    client_discovery::{discover_clients, ClientInstance, LocalClient},
    desktop::emit,
    lcu::{
        game_art::{GameArt, GameArtPath},
        player_search::PlayerSearchShard,
        GameflowSession, LcuError, LcuTransport,
    },
    resg_window::{ResgAction, ResgWindow},
};
pub(crate) use maxgamestudio_league_core::shell_preferences::LEAGUE_CLIENT as CLIENT;
use serde_json::{json, Map, Value};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
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

struct Sample {
    client: ActiveClient,
    selection: Option<(GameflowSession, Value)>,
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
    aram_options: AramOptions,
    aram_paused: bool,
    aram: AramSchedule,
    data_client: Option<ClientInstance>,
    data_ticket: u64,
    data_task: Option<tauri::async_runtime::JoinHandle<()>>,
    game_data: Map<String, Value>,
    art_client: Option<Arc<LocalClient>>,
}

pub(crate) fn empty_game_data() -> Value {
    json!({
        "champions":{},"augments":{},"perks":{},"perkstyles":{"schemaVersion":0,"styles":{}},
        "queues":{},"items":{},"summonerSpells":{},"gameModeMutators":{},"maps":{},
    })
}

impl RuntimeState {
    fn refresh_game_data(&mut self, app: &AppHandle) {
        let active = self.snapshot.active.as_ref().map(|client| client.instance);
        if self.data_client == active {
            return;
        }
        self.data_client = active;
        self.data_ticket += 1;
        if let Some(task) = self.data_task.take() {
            task.abort();
        }
        let defaults = empty_game_data();
        for (key, _) in std::mem::take(&mut self.game_data) {
            publish_property(
                app,
                CLIENT,
                "gameData",
                &key,
                &defaults[&key],
                self.revision,
            );
        }
        if let Some(client) = active {
            self.data_task = Some(tauri::async_runtime::spawn(load_game_data(
                app.clone(),
                client,
                self.data_ticket,
            )));
        }
    }
}

pub(crate) struct ClientRuntime {
    transport: LcuTransport,
    state: Mutex<RuntimeState>,
    wake: tokio::sync::Notify,
    art_reads: tokio::sync::Semaphore,
}

impl ClientRuntime {
    pub(crate) fn new(auto_connect: bool, aram_options: AramOptions) -> Result<Self, LcuError> {
        Ok(Self {
            transport: LcuTransport::new()?,
            state: Mutex::new(RuntimeState {
                auto_connect,
                aram_options,
                ..Default::default()
            }),
            wake: tokio::sync::Notify::new(),
            art_reads: tokio::sync::Semaphore::new(4),
        })
    }

    pub(crate) fn fields(
        &self,
        namespace: &str,
        name: &str,
    ) -> Result<Option<(u64, Value)>, &'static str> {
        let state = self.state.lock().map_err(|_| "客户端状态不可用。")?;
        if (namespace, name) == (CLIENT, "gameData") {
            let mut fields = empty_game_data();
            fields
                .as_object_mut()
                .expect("known game data fields")
                .extend(state.game_data.clone());
            return Ok(Some((state.revision, fields)));
        }
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

    pub(crate) async fn game_art(&self, path: &GameArtPath) -> Option<GameArt> {
        let _permit = tokio::time::timeout(Duration::from_secs(3), self.art_reads.acquire())
            .await
            .ok()?
            .ok()?;
        let client = self.state.lock().ok()?.art_client.clone()?;
        let target = client.target().ok()?;
        let art = self.transport.read_game_art(&target, path).await.ok()??;
        self.state
            .lock()
            .ok()?
            .art_client
            .as_ref()
            .is_some_and(|current| current.instance == client.instance && client.alive())
            .then_some(art)
    }

    pub(crate) fn replace_aram_options(&self, options: AramOptions) -> Result<(), &'static str> {
        let mut state = self.state.lock().map_err(|_| "客户端状态不可用。")?;
        state.aram_options = options;
        state.aram.cancel();
        state.generation += 1;
        self.refresh_soon();
        Ok(())
    }

    pub(crate) fn aram_paused(&self) -> Result<(u64, bool), &'static str> {
        let state = self.state.lock().map_err(|_| "客户端状态不可用。")?;
        Ok((state.revision, state.aram_paused))
    }

    pub(crate) fn pause_aram(&self, app: &AppHandle, paused: bool) -> Result<(), &'static str> {
        let mut state = self.state.lock().map_err(|_| "客户端状态不可用。")?;
        if state.aram_paused != paused {
            state.aram_paused = paused;
            state.aram.cancel();
            state.generation += 1;
            state.revision += 1;
            if let Some(main) = app.get_webview_window("main") {
                let _ = emit(
                    &main,
                    "mobx-utils-main",
                    "update-state-prop/auto-select-main:state",
                    json!(["temporarilyDisabled", paused, {"action":"update","raw":false,"revision":state.revision}]),
                );
            }
            self.refresh_soon();
        }
        Ok(())
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
        state.art_client = None;
        state.aram.reset();
        let previous = Snapshot {
            active: state.snapshot.active.take(),
            known_clients: state.snapshot.known_clients.clone(),
        };
        state.revision += 1;
        publish_changes(app, &previous, &state.snapshot, state.revision);
        state.refresh_game_data(app);
        self.wake.notify_one();
        Ok(())
    }

    async fn sample(
        &self,
        client: &LocalClient,
        resg_enabled: bool,
        previous_chart: Option<ResgTarget>,
        previous_shard: Option<(String, PlayerSearchShard)>,
        automate: bool,
    ) -> Result<Option<Sample>, LcuError> {
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
        let should_read = phase == "ChampSelect"
            && session.as_ref().is_some_and(|game| {
                (automate && game.is_aram()) || (resg_enabled && game.is_hextech_aram())
            });
        let selection = if should_read {
            // The optional reads should leave time to publish connected state.
            tokio::time::timeout(
                Duration::from_millis(750),
                self.transport.read_champ_select(&target),
            )
            .await
            .ok()
            .and_then(Result::ok)
        } else {
            None
        };
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
            let chart_selection = if phase == "ChampSelect" {
                selection.as_ref()
            } else {
                Some(&Value::Null)
            };
            resg = if let Some(selection) = chart_selection {
                let puuid = summoner.as_ref().map(|player| player.puuid.as_str());
                game.resg_champion(selection, puuid)
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
        Ok(client.alive().then(|| {
            let client = ActiveClient {
                instance: client.instance,
                phase,
                session: json!(session),
                summoner: json!(summoner),
                resg,
                shard,
            };
            let selection = if automate {
                session.zip(selection)
            } else {
                None
            };
            Sample { client, selection }
        }))
    }

    async fn refresh(&self, app: &AppHandle, resg_enabled: bool) {
        let (generation, manual, connected, auto_connect, previous_chart, previous_shard, options) = {
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
                (!state.aram_paused && state.aram_options.any_enabled())
                    .then(|| state.aram_options.clone()),
            )
        };
        let clients = tauri::async_runtime::spawn_blocking(discover_clients)
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default()
            .into_iter()
            .map(Arc::new)
            .collect::<Vec<_>>();
        let find_client = |instance| clients.iter().find(|client| client.instance == instance);
        let candidate = if let Some(selection) = manual {
            find_client(selection)
        } else {
            connected
                .and_then(find_client)
                .or_else(|| (auto_connect && clients.len() == 1).then(|| &clients[0]))
        };
        let sample = match candidate {
            Some(client) => tokio::time::timeout(
                Duration::from_secs(4),
                self.sample(
                    client,
                    resg_enabled,
                    previous_chart,
                    previous_shard.filter(|_| Some(client.instance) == connected),
                    options.is_some(),
                ),
            )
            .await
            .ok()
            .and_then(Result::ok)
            .flatten(),
            None => None,
        };
        let (active, selection) = match sample {
            Some(sample) => (Some(sample.client), sample.selection),
            None => (None, None),
        };
        let mut known_clients: Vec<_> = clients.iter().map(|client| client.instance).collect();
        known_clients.sort_unstable();
        let snapshot = Snapshot {
            known_clients,
            active,
        };
        {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            if state.generation != generation {
                return;
            }
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
            state.refresh_game_data(app);
            state.art_client = candidate
                .filter(|candidate| {
                    state
                        .snapshot
                        .active
                        .as_ref()
                        .is_some_and(|active| active.instance == candidate.instance)
                })
                .cloned();
            if state
                .snapshot
                .active
                .as_ref()
                .is_none_or(|client| client.phase != "ChampSelect")
            {
                state.aram.reset();
            } else if selection.is_none() {
                state.aram.cancel();
            }
        }
        if let (Some(client), Some(options), Some((flow, select))) = (candidate, options, selection)
        {
            self.automate_aram(generation, client, &flow, &select, &options)
                .await;
        }
    }

    async fn automate_aram(
        &self,
        generation: u64,
        client: &LocalClient,
        flow: &GameflowSession,
        select: &Value,
        options: &AramOptions,
    ) {
        let cancel = || {
            if let Ok(mut state) = self.state.lock() {
                if state.generation == generation {
                    state.aram.cancel();
                }
            }
        };
        let Some(scope) = SelectionScope::from_sample(client.instance, flow, select) else {
            return cancel();
        };
        let Ok(target) = client.target() else {
            return cancel();
        };
        let Ok(Ok(lists)) = tokio::time::timeout(
            Duration::from_millis(1200),
            self.transport.read_selection_lists(
                &target,
                options,
                select["allowSubsetChampionPicks"] == true,
            ),
        )
        .await
        else {
            return cancel();
        };
        let due = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            if state.generation != generation || state.aram_paused {
                return;
            }
            state
                .aram
                .observe(scope, flow, select, &lists, options, Instant::now())
        };
        let Some(due) = due else {
            return;
        };
        let latest = tokio::time::timeout(Duration::from_millis(650), async {
            if self.transport.read_phase(&target).await?.as_deref() != Some("ChampSelect") {
                return Ok(Value::Null);
            }
            self.transport.read_champ_select(&target).await
        })
        .await;
        let Ok(Ok(latest)) = latest else {
            return cancel();
        };
        if SelectionScope::from_sample(client.instance, flow, &latest) != Some(due.scope)
            || !due.still_applies(select, &latest, &lists.subset)
        {
            return cancel();
        }
        let Ok(target) = client.target() else {
            return cancel();
        };
        let authorized = self.state.lock().is_ok_and(|mut state| {
            state.generation == generation && !state.aram_paused && state.aram.mark_attempted(due)
        });
        if authorized {
            let _ = self.transport.apply_selection(&target, due).await;
        }
    }
}

async fn load_game_data(app: AppHandle, instance: ClientInstance, ticket: u64) {
    let runtime = app.state::<ClientRuntime>();
    let Ok(Some(client)) = tauri::async_runtime::spawn_blocking(move || {
        discover_clients()
            .ok()?
            .into_iter()
            .find(|client| client.instance == instance)
    })
    .await
    else {
        return;
    };
    let Ok(target) = client.target() else {
        return;
    };
    let Ok(read) = runtime.transport.read_game_data(&target).await else {
        return;
    };
    let Ok(mut state) = runtime.state.lock() else {
        return;
    };
    if state.data_ticket != ticket || state.data_client != Some(instance) || !client.alive() {
        return;
    }
    state.revision += 1;
    state.game_data = read.fields;
    for (key, value) in &state.game_data {
        publish_property(&app, CLIENT, "gameData", key, value, state.revision);
    }
}

pub(crate) async fn monitor(app: AppHandle) {
    let runtime = app.state::<ClientRuntime>();
    let resg = app.state::<ResgWindow>();
    loop {
        runtime.refresh(&app, resg.enabled()).await;
        let _ = resg.reconcile(&app, ResgAction::Synchronize);
        let interval = runtime
            .state
            .lock()
            .map_or(Duration::from_secs(2), |state| state.aram.next_poll());
        let _ = tokio::time::timeout(interval, runtime.wake.notified()).await;
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
            publish_property(app, namespace, name, &key, &value, revision);
        }
    }
}

fn publish_property(
    app: &AppHandle,
    namespace: &str,
    name: &str,
    key: &str,
    value: &Value,
    revision: u64,
) {
    let event = format!("update-state-prop/{namespace}:{name}");
    let args = json!([key, value, { "action": "update", "raw": false, "revision": revision }]);
    for label in ["main", "aux", "opgg"] {
        if let Some(window) = app.get_webview_window(label) {
            let _ = emit(&window, "mobx-utils-main", &event, args.clone());
        }
    }
}
