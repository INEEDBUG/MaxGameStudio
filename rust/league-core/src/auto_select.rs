use serde::Deserialize;
use std::collections::{HashMap, HashSet};

pub type ChampionId = i32;

const NO_BAN: ChampionId = -1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ChampSelectPhase {
    Planning,
    BanPick,
    Finalization,
    #[default]
    #[serde(other)]
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActionKind {
    Pick,
    Ban,
    Vote,
    #[serde(other)]
    Other,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChampionAction {
    pub id: i64,
    pub kind: ActionKind,
    pub champion_id: Option<ChampionId>,
}

impl ChampionAction {
    fn champion_shown(self) -> bool {
        self.champion_id.is_some_and(|id| id != 0)
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhaseTiming {
    pub remaining_ms: f64,
    pub elapsed_ms: f64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChampionGridEntry {
    pub banned: bool,
    pub teammate_intent: bool,
    pub picked_by_other: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SelectStrategy {
    JustShow,
    #[default]
    ShowAndLockIn,
    LockInImmediately,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChampionPreferences {
    pub enabled: bool,
    pub champions: Vec<ChampionId>,
    pub strategy: SelectStrategy,
    pub delay_ms: f64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PickPreferences {
    pub choice: ChampionPreferences,
    pub show_intent: bool,
    pub ignore_intent: bool,
    pub upgrade_bench: bool,
    pub bench_delay_ms: f64,
}

impl Default for PickPreferences {
    fn default() -> Self {
        Self {
            choice: ChampionPreferences::default(),
            show_intent: false,
            ignore_intent: false,
            upgrade_bench: false,
            bench_delay_ms: 2_900.0,
        }
    }
}

/// One champion-select snapshot and the saved preferences for the player's slot.
///
/// Resolve the current local action and the queue from the same session before planning.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AutoSelectInput {
    pub game_mode: Option<String>,
    pub queue_type: Option<String>,
    pub is_custom_game: bool,
    pub is_spectating: bool,
    pub temporarily_disabled: bool,
    pub phase: ChampSelectPhase,
    pub timing: Option<PhaseTiming>,
    pub active_action: Option<ChampionAction>,
    pub first_unfinished_pick: Option<ChampionAction>,
    pub current_champion: Option<ChampionId>,
    pub allow_duplicate_picks: bool,
    pub allow_subset_picks: bool,
    pub subset_champions: Vec<ChampionId>,
    pub bench_enabled: bool,
    pub bench_champions: Vec<ChampionId>,
    /// How long each observed champion has continuously been available on the bench or subset.
    pub bench_elapsed_ms: HashMap<ChampionId, f64>,
    pub pickable: HashSet<ChampionId>,
    pub bannable: HashSet<ChampionId>,
    pub champion_grid: HashMap<ChampionId, ChampionGridEntry>,
    pub pick: PickPreferences,
    pub ban: ChampionPreferences,
    /// Existing scheduled tasks for this same session.
    pub pending_pick: Option<ChampionId>,
    pub pending_ban: Option<ChampionId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum AutoSelectAction {
    Intent { action_id: i64 },
    Pick { action_id: i64, completed: bool },
    Ban { action_id: i64, completed: bool },
    BenchSwap,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum AutoSelectPlan {
    /// No action is eligible on this snapshot. Cancel any previous plan.
    Idle,
    /// Retain the instant-lock task during the brief show-to-lock transition.
    KeepScheduled,
    Schedule {
        action: AutoSelectAction,
        champion_id: ChampionId,
        /// Raw delay in milliseconds. A nonpositive value is due immediately.
        delay_ms: f64,
    },
}

pub fn plan_auto_select(input: &AutoSelectInput) -> AutoSelectPlan {
    use AutoSelectPlan::Idle;

    if !matches!(input.game_mode.as_deref(), Some("ARAM" | "KIWI"))
        || input.queue_type.as_deref().is_none_or(str::is_empty)
        || input.is_custom_game
        || input.is_spectating
        || input.temporarily_disabled
    {
        return Idle;
    }

    let kind = input.active_action.map(|action| action.kind);
    if input.phase == ChampSelectPhase::Planning
        || (input.first_unfinished_pick.is_some()
            && !matches!(kind, Some(ActionKind::Pick | ActionKind::Ban)))
    {
        if let (Some(_), Some(pick)) = (input.active_action, input.first_unfinished_pick) {
            return plan_pick(input, pick, true);
        }
        return Idle;
    }

    match input.active_action {
        Some(action) if action.kind == ActionKind::Pick => plan_pick(input, action, false),
        Some(action) if action.kind == ActionKind::Ban => plan_ban(input, action),
        Some(action) if action.kind == ActionKind::Vote => Idle,
        _ => plan_bench_swap(input),
    }
}

fn plan_pick(input: &AutoSelectInput, action: ChampionAction, intent: bool) -> AutoSelectPlan {
    use AutoSelectPlan::{Idle, KeepScheduled, Schedule};
    use SelectStrategy::{JustShow, LockInImmediately, ShowAndLockIn};

    let choice = &input.pick.choice;
    if !choice.enabled || (intent && (!input.pick.show_intent || action.champion_shown())) {
        return Idle;
    }
    let complete_phase = !intent && !input.allow_subset_picks && action.champion_shown();
    if complete_phase && choice.strategy == JustShow {
        return Idle;
    }

    let Some(champion_id) = choice
        .champions
        .iter()
        .copied()
        .find(|id| can_pick(input, *id))
    else {
        return Idle;
    };

    if complete_phase
        && choice.strategy == LockInImmediately
        && input.pending_pick == Some(champion_id)
    {
        return KeepScheduled;
    }

    let second_offset = complete_phase && choice.strategy == ShowAndLockIn;
    let selected_action = if intent {
        AutoSelectAction::Intent {
            action_id: action.id,
        }
    } else {
        AutoSelectAction::Pick {
            action_id: action.id,
            completed: input.allow_subset_picks
                || choice.strategy == LockInImmediately
                || second_offset,
        }
    };
    Schedule {
        action: selected_action,
        champion_id,
        delay_ms: phase_adjusted_delay(choice.delay_ms, second_offset, input.timing),
    }
}

fn can_pick(input: &AutoSelectInput, id: ChampionId) -> bool {
    let Some(entry) = input.champion_grid.get(&id) else {
        return false;
    };
    if !input.pickable.contains(&id) || entry.banned {
        return false;
    }
    if !input.allow_duplicate_picks {
        if entry.teammate_intent {
            return input.pick.ignore_intent;
        }
        if entry.picked_by_other {
            return false;
        }
    }
    !input.allow_subset_picks || input.subset_champions.contains(&id)
}

fn plan_ban(input: &AutoSelectInput, action: ChampionAction) -> AutoSelectPlan {
    use AutoSelectPlan::{Idle, KeepScheduled, Schedule};
    use SelectStrategy::{JustShow, LockInImmediately, ShowAndLockIn};

    let choice = &input.ban;
    let complete_phase = action.champion_shown();
    if !choice.enabled || (complete_phase && choice.strategy == JustShow) {
        return Idle;
    }

    let Some(champion_id) = choice.champions.iter().copied().find(|id| {
        input.bannable.contains(id)
            && input
                .champion_grid
                .get(id)
                .is_some_and(|entry| (!entry.banned || *id == NO_BAN) && !entry.teammate_intent)
    }) else {
        return Idle;
    };

    if complete_phase
        && choice.strategy == LockInImmediately
        && input.pending_ban == Some(champion_id)
    {
        return KeepScheduled;
    }

    let second_offset = complete_phase && choice.strategy == ShowAndLockIn;
    Schedule {
        action: AutoSelectAction::Ban {
            action_id: action.id,
            completed: choice.strategy == LockInImmediately || second_offset,
        },
        champion_id,
        delay_ms: phase_adjusted_delay(choice.delay_ms, second_offset, input.timing),
    }
}

fn plan_bench_swap(input: &AutoSelectInput) -> AutoSelectPlan {
    use AutoSelectPlan::{Idle, Schedule};

    let preferences = &input.pick;
    let Some(held) = input.current_champion.filter(|id| *id != 0) else {
        return Idle;
    };
    if !input.bench_enabled || !preferences.choice.enabled {
        return Idle;
    }
    let in_ban_pick = input.phase == ChampSelectPhase::BanPick;
    let bench = if in_ban_pick && !input.subset_champions.is_empty() {
        &input.subset_champions
    } else {
        &input.bench_champions
    };
    let subset_required = input.allow_subset_picks && in_ban_pick;
    let preferred = &preferences.choice.champions;
    let Some((priority, &champion_id)) = preferred.iter().enumerate().find(|(_, id)| {
        bench.contains(id)
            && input.pickable.contains(id)
            && (!subset_required || input.subset_champions.contains(id))
    }) else {
        return Idle;
    };
    if let Some(held_priority) = preferred.iter().position(|id| *id == held) {
        if !preferences.upgrade_bench || priority >= held_priority {
            return Idle;
        }
    }
    let Some(elapsed_ms) = input.bench_elapsed_ms.get(&champion_id) else {
        return Idle;
    };
    let delay_ms = preferences.bench_delay_ms - elapsed_ms;
    Schedule {
        action: AutoSelectAction::BenchSwap,
        champion_id,
        delay_ms: input
            .timing
            .map_or(delay_ms, |timer| delay_ms.min(timer.remaining_ms)),
    }
}

fn phase_adjusted_delay(delay_ms: f64, second_offset: bool, timing: Option<PhaseTiming>) -> f64 {
    let target_offset_ms = if second_offset {
        delay_ms * 2.0
    } else {
        delay_ms
    };
    timing.map_or(delay_ms, |timer| {
        delay_ms
            .min(target_offset_ms - timer.elapsed_ms)
            .min(timer.remaining_ms)
            .max(0.0)
    })
}
