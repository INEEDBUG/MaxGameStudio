//! Project verified League champion-select data onto the existing ARAM planner.

use crate::{client_discovery::ClientInstance, lcu::GameflowSession};
use maxgamestudio_league_core::auto_select::{
    plan_auto_select, ActionKind, AutoSelectAction, AutoSelectInput, AutoSelectPlan,
    ChampSelectPhase, ChampionAction, ChampionGridEntry, ChampionPreferences, PhaseTiming,
    PickPreferences, SelectStrategy,
};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
pub(crate) struct AramOptions {
    pick: PickPreferences,
    ban: ChampionPreferences,
}

impl AramOptions {
    pub(crate) fn from_saved(pick: &Value, ban: &Value) -> Self {
        let pick = &pick["aram"];
        Self {
            pick: PickPreferences {
                choice: preferences(pick),
                show_intent: pick["showIntent"] == true,
                ignore_intent: pick["ignoreIntent"] == true,
                upgrade_bench: pick["benchSelectFirstAvailableChampion"] == true,
                bench_delay_ms: seconds(&pick["benchSwapAccumulatedDelaySeconds"], 2.9) * 1000.0,
            },
            ban: preferences(&ban["aram"]),
        }
    }

    pub(crate) fn any_enabled(&self) -> bool {
        self.pick.choice.enabled || self.ban.enabled
    }

    pub(crate) fn want_pick(&self) -> bool {
        self.pick.choice.enabled
    }

    pub(crate) fn want_ban(&self) -> bool {
        self.ban.enabled
    }
}

fn preferences(config: &Value) -> ChampionPreferences {
    let strategy = match config["strategy"].as_str() {
        Some("just-show") => SelectStrategy::JustShow,
        Some("lock-in-immediately") => SelectStrategy::LockInImmediately,
        _ => SelectStrategy::ShowAndLockIn,
    };
    ChampionPreferences {
        enabled: config["enabled"] == true,
        champions: ids(&config["champions"]["default"]).collect(),
        strategy,
        delay_ms: seconds(&config["delaySeconds"], 0.0) * 1000.0,
    }
}

fn seconds(value: &Value, fallback: f64) -> f64 {
    value
        .as_f64()
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
        .unwrap_or(fallback)
}

fn entries(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_array().into_iter().flatten()
}

fn id(value: &Value) -> Option<i32> {
    let id = i32::try_from(value.as_i64()?).ok()?;
    (id == -1 || (1..100_000).contains(&id)).then_some(id)
}

fn ids(value: &Value) -> impl Iterator<Item = i32> + '_ {
    entries(value).filter_map(id)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct SelectionScope {
    client: ClientInstance,
    game: u64,
    cell: i64,
}

impl SelectionScope {
    pub(crate) fn from_sample(
        client: ClientInstance,
        flow: &GameflowSession,
        select: &Value,
    ) -> Option<Self> {
        let game = flow.game_data.as_ref()?;
        let queue = game.queue.as_ref()?;
        if flow.phase != "ChampSelect"
            || !matches!(queue.game_mode.as_deref(), Some("ARAM" | "KIWI"))
            || queue.queue_type.as_deref().is_none_or(str::is_empty)
            || game.is_custom_game != Some(false)
            || select["isCustomGame"] != false
            || select["isSpectating"] != false
        {
            return None;
        }
        let game = game.game_id.filter(|&id| id != 0)?;
        let cell = select["localPlayerCellId"]
            .as_i64()
            .filter(|&cell| cell >= 0)?;
        (select["gameId"].as_u64() == Some(game) && local_player(select, cell).is_some())
            .then_some(Self { client, game, cell })
    }
}

fn local_player(select: &Value, cell: i64) -> Option<&Value> {
    entries(&select["myTeam"]).find(|player| player["cellId"].as_i64() == Some(cell))
}

fn remaining_actions(select: &Value, cell: i64) -> impl Iterator<Item = &Value> {
    entries(&select["actions"])
        .flat_map(entries)
        .filter(move |action| {
            action["completed"] == false && action["actorCellId"].as_i64() == Some(cell)
        })
}

fn champion_action(value: &Value) -> Option<ChampionAction> {
    let kind = match value["type"].as_str()? {
        "pick" => ActionKind::Pick,
        "ban" => ActionKind::Ban,
        "vote" => ActionKind::Vote,
        _ => ActionKind::Other,
    };
    Some(ChampionAction {
        id: value["id"].as_i64().filter(|&id| id >= 0)?,
        kind,
        champion_id: id(&value["championId"]),
    })
}

#[derive(Default)]
pub(crate) struct SelectionLists {
    pub(crate) pickable: Value,
    pub(crate) bannable: Value,
    pub(crate) grid: Value,
    pub(crate) subset: Value,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScheduledAction {
    pub(crate) scope: SelectionScope,
    pub(crate) action: AutoSelectAction,
    pub(crate) champion: i32,
    due: Instant,
}

impl ScheduledAction {
    pub(crate) fn still_applies(self, before: &Value, select: &Value, subset: &Value) -> bool {
        if before["timer"]["phase"] != select["timer"]["phase"] {
            return false;
        }
        match self.action {
            AutoSelectAction::Intent { action_id } => remaining_actions(select, self.scope.cell)
                .any(|action| {
                    action["id"].as_i64() == Some(action_id)
                        && action["type"] == "pick"
                        && id(&action["championId"]).is_none()
                }),
            AutoSelectAction::Pick { action_id, .. } | AutoSelectAction::Ban { action_id, .. } => {
                let kind = if matches!(self.action, AutoSelectAction::Pick { .. }) {
                    "pick"
                } else {
                    "ban"
                };
                let previous = remaining_actions(before, self.scope.cell)
                    .find(|action| action["id"].as_i64() == Some(action_id));
                remaining_actions(select, self.scope.cell).any(|action| {
                    action["id"].as_i64() == Some(action_id)
                        && action["type"] == kind
                        && action["isInProgress"] == true
                        && previous
                            .is_some_and(|before| before["championId"] == action["championId"])
                })
            }
            AutoSelectAction::BenchSwap => {
                select["benchEnabled"] == true
                    && local_player(before, self.scope.cell).map(|player| &player["championId"])
                        == local_player(select, self.scope.cell).map(|player| &player["championId"])
                    && if select["timer"]["phase"] == "BAN_PICK"
                        && select["allowSubsetChampionPicks"] == true
                    {
                        ids(subset).any(|id| id == self.champion)
                    } else {
                        entries(&select["benchChampions"])
                            .any(|entry| id(&entry["championId"]) == Some(self.champion))
                    }
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct AramSchedule {
    scope: Option<SelectionScope>,
    pending: Option<ScheduledAction>,
    attempted: Vec<(AutoSelectAction, i32)>,
    bench_since: HashMap<i32, Instant>,
}

impl AramSchedule {
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn cancel(&mut self) {
        self.pending = None;
    }

    pub(crate) fn observe(
        &mut self,
        scope: SelectionScope,
        flow: &GameflowSession,
        select: &Value,
        available: &SelectionLists,
        options: &AramOptions,
        now: Instant,
    ) -> Option<ScheduledAction> {
        if self.scope != Some(scope) {
            self.reset();
            self.scope = Some(scope);
        }
        let queue = flow.game_data.as_ref()?.queue.as_ref()?;
        let active_action = remaining_actions(select, scope.cell)
            .find(|action| action["isInProgress"] == true)
            .and_then(champion_action);
        let first_unfinished_pick = remaining_actions(select, scope.cell)
            .find(|action| action["type"] == "pick")
            .and_then(champion_action);
        let timer = &select["timer"];
        let phase = match timer["phase"].as_str() {
            Some("PLANNING") => ChampSelectPhase::Planning,
            Some("BAN_PICK") => ChampSelectPhase::BanPick,
            Some("FINALIZATION") => ChampSelectPhase::Finalization,
            _ => {
                self.pending = None;
                return None;
            }
        };
        let timing = timer["adjustedTimeLeftInPhase"]
            .as_f64()
            .zip(timer["totalTimeInPhase"].as_f64())
            .map(|(left, total)| PhaseTiming {
                remaining_ms: left.max(0.0),
                elapsed_ms: (total - left).max(0.0),
            });
        let subset_champions: Vec<_> = ids(&available.subset).collect();
        let bench_champions: Vec<_> = entries(&select["benchChampions"])
            .filter_map(|entry| id(&entry["championId"]))
            .collect();
        let on_bench: HashSet<_> =
            if phase == ChampSelectPhase::BanPick && !subset_champions.is_empty() {
                subset_champions.iter().copied().collect()
            } else {
                bench_champions.iter().copied().collect()
            };
        self.bench_since.retain(|id, _| on_bench.contains(id));
        for id in on_bench {
            self.bench_since.entry(id).or_insert(now);
        }
        let mut champion_grid: HashMap<_, _> = entries(&available.grid)
            .filter_map(|entry| {
                let champion = id(&entry["id"])?;
                let status = entry["selectionStatus"].as_object()?;
                let flag = |name: &str| status.get(name).and_then(Value::as_bool);
                Some((
                    champion,
                    ChampionGridEntry {
                        banned: flag("isBanned")? || entry["disabled"] == true,
                        teammate_intent: flag("pickIntented")? && !flag("pickIntentedByMe")?,
                        picked_by_other: flag("pickedByOtherOrBanned")?,
                    },
                ))
            })
            .collect();
        let bannable: HashSet<_> = ids(&available.bannable).collect();
        if bannable.contains(&-1) {
            champion_grid.entry(-1).or_default();
        }
        let pending = |kind| {
            self.pending
                .filter(|plan| match plan.action {
                    AutoSelectAction::Pick { .. } => kind == ActionKind::Pick,
                    AutoSelectAction::Ban { .. } => kind == ActionKind::Ban,
                    _ => false,
                })
                .map(|plan| plan.champion)
        };
        let input = AutoSelectInput {
            game_mode: queue.game_mode.clone(),
            queue_type: queue.queue_type.clone(),
            phase,
            timing,
            active_action,
            first_unfinished_pick,
            current_champion: id(&local_player(select, scope.cell)?["championId"]),
            allow_duplicate_picks: select["allowDuplicatePicks"] == true
                || select["allowPlayerPickSameChampion"] == true,
            allow_subset_picks: select["allowSubsetChampionPicks"] == true,
            subset_champions,
            bench_enabled: select["benchEnabled"] == true,
            bench_champions,
            bench_elapsed_ms: self
                .bench_since
                .iter()
                .map(|(&id, &start)| {
                    (
                        id,
                        now.saturating_duration_since(start).as_secs_f64() * 1000.0,
                    )
                })
                .collect(),
            pickable: ids(&available.pickable).collect(),
            bannable,
            champion_grid,
            pick: options.pick.clone(),
            ban: options.ban.clone(),
            pending_pick: pending(ActionKind::Pick),
            pending_ban: pending(ActionKind::Ban),
            ..Default::default()
        };
        match plan_auto_select(&input) {
            AutoSelectPlan::Idle => self.pending = None,
            AutoSelectPlan::KeepScheduled => {}
            AutoSelectPlan::Schedule {
                action,
                champion_id,
                delay_ms,
            } => {
                if self.attempted.contains(&(action, champion_id)) {
                    self.pending = None;
                } else {
                    let delay = Duration::try_from_secs_f64(delay_ms.max(0.0) / 1000.0).ok()?;
                    let due = now.checked_add(delay)?;
                    let due = self
                        .pending
                        .filter(|pending| {
                            pending.action == action && pending.champion == champion_id
                        })
                        .map_or(due, |pending| pending.due.min(due));
                    self.pending = Some(ScheduledAction {
                        scope,
                        action,
                        champion: champion_id,
                        due,
                    });
                }
            }
        }
        self.pending.filter(|pending| pending.due <= now)
    }

    pub(crate) fn mark_attempted(&mut self, action: ScheduledAction) -> bool {
        if self.pending != Some(action) {
            return false;
        }
        self.pending = None;
        self.attempted.push((action.action, action.champion));
        true
    }

    pub(crate) fn next_poll(&self) -> Duration {
        self.pending.map_or(Duration::from_secs(2), |pending| {
            pending
                .due
                .saturating_duration_since(Instant::now())
                .clamp(Duration::from_millis(40), Duration::from_secs(2))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn current_aram_action_is_bound_to_local_slot_game_and_phase() {
        let flow: GameflowSession = serde_json::from_value(json!({
            "phase": "ChampSelect",
            "gameData": {"gameId": 3807, "isCustomGame": false,
                "queue": {"gameMode": "KIWI", "type": "ARAM_UNRANKED_5x5"}},
        }))
        .unwrap();
        let mut selection = json!({
            "gameId": 3807, "localPlayerCellId": 3, "isCustomGame": false, "isSpectating": false,
            "myTeam": [{"cellId": 3, "championId": 0}],
            "actions": [[{"id": 4, "actorCellId": 3, "championId": 0, "type": "pick",
                "isInProgress": true, "completed": false}]],
            "timer": {"phase": "BAN_PICK", "adjustedTimeLeftInPhase": 19700, "totalTimeInPhase": 20000}
        });
        let client = ClientInstance {
            pid: 22,
            created_at: 556,
        };
        let scope = SelectionScope::from_sample(client, &flow, &selection).unwrap();
        let available = SelectionLists {
            pickable: json!([57, 63]),
            grid: json!([
                {"id": 57, "selectionStatus": {"isBanned": true, "pickIntented": false, "pickIntentedByMe": false, "pickedByOtherOrBanned": true}},
                {"id": 63, "selectionStatus": {"isBanned": false, "pickIntented": false, "pickIntentedByMe": false, "pickedByOtherOrBanned": false}}
            ]),
            ..Default::default()
        };
        let options = AramOptions::from_saved(
            &json!({"aram": {"enabled": true, "champions": {"default": [57, 63]}, "delaySeconds":1}}),
            &Value::Null,
        );
        let mut scheduler = AramSchedule::default();
        let now = Instant::now();
        assert!(scheduler
            .observe(scope, &flow, &selection, &available, &options, now)
            .is_none());
        let due = now + Duration::from_secs(1);
        let action = scheduler
            .observe(scope, &flow, &selection, &available, &options, due)
            .unwrap();
        assert_eq!(action.champion, 63);
        assert_eq!(
            action.action,
            AutoSelectAction::Pick {
                action_id: 4,
                completed: false
            }
        );
        assert!(action.still_applies(&selection, &selection, &Value::Null));
        let mut changed = selection.clone();
        changed["actions"][0][0]["actorCellId"] = json!(4);
        assert!(!action.still_applies(&selection, &changed, &Value::Null));
        changed = selection.clone();
        changed["actions"][0][0]["championId"] = json!(29);
        assert!(!action.still_applies(&selection, &changed, &Value::Null));
        assert!(scheduler.mark_attempted(action));
        assert!(!scheduler.mark_attempted(action));
        assert!(scheduler
            .observe(scope, &flow, &selection, &available, &options, due)
            .is_none());

        selection["isSpectating"] = json!(true);
        assert!(SelectionScope::from_sample(client, &flow, &selection).is_none());
        selection["isSpectating"] = json!(false);
        selection["gameId"] = json!(3808);
        assert!(SelectionScope::from_sample(client, &flow, &selection).is_none());
        selection["gameId"] = json!(3807);
        let mut other = serde_json::to_value(flow).unwrap();
        other["gameData"]["queue"]["gameMode"] = json!("CLASSIC");
        let other: GameflowSession = serde_json::from_value(other).unwrap();
        assert!(SelectionScope::from_sample(client, &other, &selection).is_none());
    }
}
