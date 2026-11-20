//! Projection of the verified same-shard LCU match history for the player search result.

use super::{shard_from_platform, PlayerSearchShard, Puuid};
use serde::{Deserialize, Serialize};
use std::{cmp::Reverse, collections::HashSet};

pub(super) const HISTORY_PAGE_SIZE: usize = 20;
const DISPLAY_LIMIT: usize = 6;
const MAX_DATE_MS: u64 = 8_640_000_000_000_000; // JavaScript Date limit.

#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "camelCase")]
pub(super) enum RecentAramHistory {
    Ready { matches: Vec<RecentMatch> },
    Unavailable,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct RecentMatch {
    game_id: String,
    game_mode: AramMode,
    started_at: u64,
    duration_seconds: u32,
    champion_id: u32,
    outcome: MatchOutcome,
    kills: u32,
    deaths: u32,
    assists: u32,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
enum AramMode {
    Aram,
    Kiwi,
}

impl AramMode {
    fn from_game_mode(value: &str) -> Option<Self> {
        match value {
            // The pinned Akari ARAM group classes by game mode across queue types.
            "ARAM" => Some(Self::Aram),
            "KIWI" => Some(Self::Kiwi),
            _ => None,
        }
    }
}

#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum MatchOutcome {
    Win,
    Loss,
    Remake,
    Abort,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MatchHistoryResponse {
    platform_id: String,
    account_id: Option<u64>,
    games: MatchPage,
}

#[derive(Deserialize)]
struct MatchPage {
    games: Vec<Option<HistoryGame>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryGame {
    platform_id: String,
    game_mode: String,
    game_id: u64,
    game_creation: u64,
    game_duration: u32,
    end_of_game_result: Option<String>,
    participant_identities: Vec<Identity>,
    participants: Vec<Participant>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Identity {
    participant_id: u32,
    player: IdentityPlayer,
}

#[derive(Deserialize)]
struct IdentityPlayer {
    puuid: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Participant {
    participant_id: u32,
    champion_id: u32,
    stats: ParticipantStats,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ParticipantStats {
    win: bool,
    kills: u32,
    deaths: u32,
    assists: u32,
    #[serde(default)]
    game_ended_in_early_surrender: bool,
    #[serde(default)]
    team_early_surrendered: bool,
}

impl MatchHistoryResponse {
    pub(super) fn project(
        self,
        target: &Puuid,
        shard: &PlayerSearchShard,
        expected_account: Option<u64>,
    ) -> RecentAramHistory {
        let games = self.games.games;
        let recorded_account = self.account_id.filter(|&id| id > 0);
        let mismatched_account = expected_account
            .zip(recorded_account)
            .is_some_and(|(expected, observed)| expected != observed);
        if !same_shard(&self.platform_id, shard)
            || mismatched_account
            || games.len() > HISTORY_PAGE_SIZE
        {
            return RecentAramHistory::Unavailable;
        }

        let mut matches = Vec::new();
        let mut incomplete = false;
        for game in games {
            let Some(game) = game.filter(|game| !game.game_mode.is_empty()) else {
                incomplete = true;
                continue;
            };
            let Some(mode) = AramMode::from_game_mode(&game.game_mode) else {
                continue;
            };
            if let Some(row) = game.project(mode, target, shard) {
                matches.push(row);
            } else {
                incomplete = true;
            }
        }
        if incomplete && matches.is_empty() {
            // An unreadable ARAM row is different from a valid page with no ARAM.
            return RecentAramHistory::Unavailable;
        }
        matches.sort_by_key(|game| Reverse(game.started_at));
        let mut seen = HashSet::new();
        matches.retain(|game| seen.insert(game.game_id.clone()));
        matches.truncate(DISPLAY_LIMIT);
        RecentAramHistory::Ready { matches }
    }
}

impl HistoryGame {
    fn project(
        self,
        mode: AramMode,
        target: &Puuid,
        shard: &PlayerSearchShard,
    ) -> Option<RecentMatch> {
        if !same_shard(&self.platform_id, shard)
            || self.game_id == 0
            || !(1..=MAX_DATE_MS).contains(&self.game_creation)
            || self.game_duration == 0
        {
            return None;
        }

        // List responses can have only the requested summoner, with their
        // original participant ID. Never use the position in either array.
        let identity = exactly_one(self.participant_identities.iter().filter(|record| {
            record
                .player
                .puuid
                .as_deref()
                .is_some_and(|value| value.eq_ignore_ascii_case(&target.0))
        }))?;
        let id = identity.participant_id;
        if id == 0
            || exactly_one(
                self.participant_identities
                    .iter()
                    .filter(|row| row.participant_id == id),
            )
            .is_none()
        {
            return None;
        }
        let participant = exactly_one(
            self.participants
                .iter()
                .filter(|row| row.participant_id == id),
        )?;
        if participant.champion_id == 0 {
            return None;
        }
        let stats = &participant.stats;
        Some(RecentMatch {
            game_id: self.game_id.to_string(),
            game_mode: mode,
            started_at: self.game_creation,
            duration_seconds: self.game_duration,
            champion_id: participant.champion_id,
            outcome: stats.outcome(self.end_of_game_result.as_deref()),
            kills: stats.kills,
            deaths: stats.deaths,
            assists: stats.assists,
        })
    }
}

impl ParticipantStats {
    fn outcome(&self, end_of_game_result: Option<&str>) -> MatchOutcome {
        // Same precedence as Akari's match-history/win-result.ts.
        if end_of_game_result.is_some_and(|reason| reason.starts_with("Abort_")) {
            MatchOutcome::Abort
        } else if self.game_ended_in_early_surrender {
            MatchOutcome::Remake
        } else if self.win && !self.team_early_surrendered {
            MatchOutcome::Win
        } else {
            MatchOutcome::Loss
        }
    }
}

fn same_shard(platform_id: &str, expected: &PlayerSearchShard) -> bool {
    shard_from_platform(platform_id)
        .is_ok_and(|shard| shard.sgp_server_id == expected.sgp_server_id)
}

fn exactly_one<T>(mut candidates: impl Iterator<Item = T>) -> Option<T> {
    let first = candidates.next()?;
    candidates.next().is_none().then_some(first)
}

#[cfg(test)]
#[path = "player_history_tests.rs"]
mod tests;
