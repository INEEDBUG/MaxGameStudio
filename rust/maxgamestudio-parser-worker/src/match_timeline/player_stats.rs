//! Player metrics from the deduplicated visible-round death ledger.

use super::{
    damage_stats::{self, DamageInput, Tally},
    round_score::{RoundOutcome, TeamKey},
};
use crate::demo_inspection::player_name;
use serde_json::{json, Value};
use std::collections::{hash_map::Entry, HashMap, HashSet};

// The released workspace uses five seconds at its fixed 64-tick reference rate.
const TRADE_WINDOW_TICKS: i64 = 5 * 64;

pub(super) struct Player<'a> {
    pub(super) name: &'a str,
    pub(super) steam_id64: &'a str,
    pub(super) team_key: TeamKey,
    pub(super) final_group: i64,
}

pub(super) fn build(
    roster: &[Player<'_>],
    rounds: &[Value],
    outcomes: &[RoundOutcome],
    damage: DamageInput<'_>,
) -> Vec<Value> {
    debug_assert_eq!(rounds.len(), outcomes.len());
    let mut by_name = HashMap::new();
    let mut players = Vec::new();
    let mut players_alive = [HashSet::new(), HashSet::new()];
    for player in roster {
        let name = clean_name(player.name);
        if name.is_empty() {
            continue;
        }
        if let Entry::Vacant(entry) = by_name.entry(name.to_lowercase()) {
            entry.insert(players.len());
            players_alive[team_index(player.team_key)].insert(players.len());
            players.push(player);
        }
    }
    let mut totals = vec![Totals::default(); players.len()];
    for (round, outcome) in rounds.iter().zip(outcomes) {
        debug_assert_eq!(round["round_number"].as_i64(), Some(outcome.round_number));
        let deaths: Vec<_> = round["events"]
            .as_array()
            .expect("projected round events")
            .iter()
            .filter(|event| event["type"] == "kill")
            .map(|event| Death::read(event, &by_name))
            .collect();
        count_round(
            &players,
            &players_alive,
            &deaths,
            outcome.winner,
            &mut totals,
        );
    }
    let damage = damage_stats::accumulate(damage, &players, &by_name);
    players
        .into_iter()
        .zip(totals)
        .zip(damage)
        .map(|((player, totals), damage)| totals.as_value(player, damage, rounds.len()))
        .collect()
}

pub(super) fn ranked_player_ids(stats: &[Value]) -> Vec<&str> {
    let mut ranked: Vec<_> = stats
        .iter()
        .map(|player| {
            (
                player["steam_id64"].as_str().expect("stat player identity"),
                player["kills"].as_u64().expect("player kill count"),
                player["adr"].as_f64().expect("player ADR"),
                player["name"]
                    .as_str()
                    .expect("stat player name")
                    .to_lowercase(),
            )
        })
        .collect();
    ranked.sort_by(
        |(_, left_kills, left_adr, left_name), (_, right_kills, right_adr, right_name)| {
            right_kills
                .cmp(left_kills)
                .then_with(|| right_adr.total_cmp(left_adr))
                .then_with(|| left_name.cmp(right_name))
        },
    );
    ranked.into_iter().map(|(id, _, _, _)| id).collect()
}

struct Death<'a> {
    tick: i64,
    actor_name: &'a str,
    target_name: &'a str,
    actor: Option<usize>,
    target: Option<usize>,
    assister: Option<usize>,
    headshot: bool,
    awp: bool,
}

impl<'a> Death<'a> {
    fn read(event: &'a Value, by_name: &HashMap<String, usize>) -> Self {
        let name = |key: &str| clean_name(event[key].as_str().unwrap_or_default());
        let index = |name: &str| by_name.get(&name.to_lowercase()).copied();
        let actor_name = name("actor");
        let target_name = name("target");
        let assister = name("assister");
        Self {
            tick: event["tick"].as_i64().unwrap_or_default(),
            actor_name,
            target_name,
            actor: index(actor_name),
            target: index(target_name),
            assister: (assister != actor_name && assister != target_name)
                .then(|| index(assister))
                .flatten(),
            headshot: event["headshot"] == true,
            awp: event["weapon"] == "awp",
        }
    }

    fn is_kill(&self) -> bool {
        !self.actor_name.is_empty()
            && !self.target_name.is_empty()
            && self.actor_name != self.target_name
            && !self.actor_name.eq_ignore_ascii_case("world")
    }
}

#[derive(Clone, Default)]
struct Totals {
    kills: usize,
    deaths: usize,
    assists: usize,
    headshots: usize,
    first_kills: usize,
    first_deaths: usize,
    trade_kills: usize,
    trade_deaths: usize,
    clutch_attempts: usize,
    clutch_wins: usize,
    awp_kills: usize,
    multi_kills: [usize; 5],
    survived_rounds: usize,
    kast_rounds: usize,
}

impl Totals {
    fn as_value(&self, player: &Player<'_>, damage: Tally, rounds: usize) -> Value {
        let percent = |n, d| rounded_ratio(n, d, 100.0, 1);
        let ratio = |n, d| rounded_ratio(n, d, 1.0, 2);
        json!({
            "name": player.name, "steam_id64": player.steam_id64, "team_key": player.team_key.label(),
            "kills": self.kills, "deaths": self.deaths, "assists": self.assists,
            "kd": ratio(self.kills, self.deaths), "kpr": ratio(self.kills, rounds),
            "dpr": ratio(self.deaths, rounds), "kast": percent(self.kast_rounds, rounds),
            "adr": rounded_ratio(damage.total, rounds, 1.0, 1),
            "utility_damage": damage.utility,
            "utility_damage_per_round": rounded_ratio(damage.utility, rounds, 1.0, 1),
            "headshots": self.headshots, "hs_percent": percent(self.headshots, self.kills),
            "first_kills": self.first_kills, "first_deaths": self.first_deaths,
            "opening_duel_win_rate": percent(self.first_kills, self.first_kills + self.first_deaths),
            "trade_kills": self.trade_kills, "trade_deaths": self.trade_deaths,
            "trade_kill_rate": percent(self.trade_kills, rounds), "survival_rate": percent(self.survived_rounds, rounds),
            "one_kill_rounds": self.multi_kills[0], "two_kill_rounds": self.multi_kills[1],
            "three_kill_rounds": self.multi_kills[2], "four_kill_rounds": self.multi_kills[3],
            "five_kill_rounds": self.multi_kills[4],
            "clutch_attempts": self.clutch_attempts, "clutch_wins": self.clutch_wins,
            "awp_kills": self.awp_kills,
        })
    }
}

#[derive(Clone, Default)]
struct RoundPlayer {
    kills: usize,
    died: bool,
    contributed: bool,
    clutch_attempted: bool,
}

fn count_round(
    players: &[&Player<'_>],
    players_alive: &[HashSet<usize>; 2],
    deaths: &[Death<'_>],
    winner: Option<TeamKey>,
    totals: &mut [Totals],
) {
    let (trade_kills, trade_deaths) = trade_pairs(players, deaths);
    let mut round = vec![RoundPlayer::default(); players.len()];
    let mut alive = players_alive.clone();
    let mut first_kill_seen = false;
    note_clutches(&alive, &mut round);
    for (index, death) in deaths.iter().enumerate() {
        if let Some(victim) = death.target {
            totals[victim].deaths += 1;
            round[victim].died = true;
            alive[team_index(players[victim].team_key)].remove(&victim);
            note_clutches(&alive, &mut round);
            if trade_deaths[index] {
                totals[victim].trade_deaths += 1;
                round[victim].contributed = true;
            }
        }
        if death.is_kill() {
            if let Some(attacker) = death.actor {
                let stats = &mut totals[attacker];
                stats.kills += 1;
                stats.headshots += usize::from(death.headshot);
                stats.awp_kills += usize::from(death.awp);
                stats.trade_kills += usize::from(trade_kills[index]);
                stats.first_kills += usize::from(!first_kill_seen);
                round[attacker].kills += 1;
                round[attacker].contributed = true;
            }
            if !first_kill_seen {
                if let Some(victim) = death.target {
                    totals[victim].first_deaths += 1;
                }
                first_kill_seen = true;
            }
        }
        if let Some(assister) = death.assister {
            totals[assister].assists += 1;
            round[assister].contributed = true;
        }
    }
    for ((totals, player), round) in totals.iter_mut().zip(players).zip(round) {
        let survived = !round.died;
        totals.survived_rounds += usize::from(survived);
        totals.kast_rounds += usize::from(survived || round.contributed);
        if round.kills > 0 {
            totals.multi_kills[round.kills.min(5) - 1] += 1;
        }
        if round.clutch_attempted {
            totals.clutch_attempts += 1;
            totals.clutch_wins += usize::from(winner == Some(player.team_key));
        }
    }
}

fn trade_pairs(players: &[&Player<'_>], deaths: &[Death<'_>]) -> (Vec<bool>, Vec<bool>) {
    let mut trade_kills = vec![false; deaths.len()];
    let mut trade_deaths = vec![false; deaths.len()];
    for (index, death) in deaths.iter().enumerate() {
        if death.actor_name.is_empty()
            || death.target_name.is_empty()
            || death.actor_name == death.target_name
        {
            continue;
        }
        let victim_team = death.target.map(|victim| players[victim].team_key);
        for (reply_index, reply) in deaths.iter().enumerate().skip(index + 1) {
            if reply.tick - death.tick > TRADE_WINDOW_TICKS {
                break;
            }
            let reply_team = reply.actor.map(|actor| players[actor].team_key);
            if reply.target_name == death.actor_name
                && (victim_team.is_none() || victim_team == reply_team)
            {
                trade_kills[reply_index] = true;
                trade_deaths[index] = true;
                break;
            }
        }
    }
    (trade_kills, trade_deaths)
}

fn note_clutches(alive: &[HashSet<usize>; 2], round: &mut [RoundPlayer]) {
    for (team, teammates) in alive.iter().enumerate() {
        if teammates.len() == 1 && !alive[1 - team].is_empty() {
            let &solo = teammates.iter().next().expect("one surviving teammate");
            round[solo].clutch_attempted = true;
        }
    }
}

fn team_index(team: TeamKey) -> usize {
    match team {
        TeamKey::A => 0,
        TeamKey::B => 1,
    }
}

pub(super) fn clean_name(value: &str) -> &str {
    player_name(value)
        .filter(|name| !name.eq_ignore_ascii_case("nat") && !name.eq_ignore_ascii_case("none"))
        .unwrap_or_default()
}

fn rounded_ratio(numerator: usize, denominator: usize, scale: f64, places: usize) -> f64 {
    let value = numerator as f64 / denominator.max(1) as f64 * scale;
    // Decimal formatting rounds the original binary float at the requested precision.
    format!("{value:.places$}")
        .parse()
        .expect("finite player ratio")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::match_timeline::round_score::{self, Side};
    use std::collections::BTreeMap;

    #[test]
    fn player_stats_match_legacy_trade_boundaries_clutches_and_kast() {
        let roster = ["Alpha", "Bravo", "Charlie", "Delta", "Echo", "Foxtrot"]
            .into_iter()
            .zip(["1", "2", "3", "4", "5", "6"])
            .enumerate()
            .map(|(i, (name, steam_id64))| Player {
                name,
                steam_id64,
                team_key: if i < 3 { TeamKey::A } else { TeamKey::B },
                final_group: if i < 3 { 2 } else { 3 },
            })
            .collect::<Vec<_>>();
        let rounds = [
            json!({"round_number":1,"events":[
                death(100,"World","Charlie","",false,"ak47"),
                death(200,"Delta","Bravo","Echo",false,"ak47"),
                {"type":"plant","tick":210,"actor":"Echo"},
                death(520,"Alpha","Delta","Alpha",true,"awp"),
                death(530,"Alpha","Echo","Bravo",false,"ak47"),
                death(540,"Alpha","Foxtrot","",false,"ak47"),
            ]}),
            json!({"round_number":2,"events":[
                death(1000,"Alpha","Delta","Bravo",false,"ak47"),
                death(1321,"Echo","Alpha","",false,"ak47"),
                death(1350,"Foxtrot","Foxtrot","",false,"ak47"),
            ]}),
            json!({"round_number":3,"events":[]}),
        ];
        let outcomes = round_score::build(
            Some([1, 1]),
            &BTreeMap::new(),
            &[(1, Some(Side::T)), (2, Some(Side::Ct)), (3, None)],
        )
        .rounds;
        let actual = build(
            &roster,
            &rounds,
            &outcomes,
            DamageInput {
                events: &[],
                match_start_tick: 0,
                freeze_ticks: &[],
            },
        );
        let keys = [
            "kills",
            "deaths",
            "assists",
            "first_kills",
            "first_deaths",
            "trade_kills",
            "trade_deaths",
            "kast",
            "clutch_attempts",
            "clutch_wins",
            "headshots",
            "awp_kills",
            "three_kill_rounds",
        ];
        // Captured from the legacy reducer with these deduplicated rounds.
        let expected = [
            json!([4, 1, 0, 1, 0, 1, 0, 100.0, 1, 1, 1, 1, 1]),
            json!([0, 1, 2, 0, 1, 0, 1, 100.0, 0, 0, 0, 0, 0]),
            json!([0, 1, 0, 0, 0, 0, 0, 66.7, 0, 0, 0, 0, 0]),
            json!([1, 2, 0, 1, 1, 0, 0, 66.7, 0, 0, 0, 0, 0]),
            json!([1, 1, 1, 0, 0, 0, 0, 100.0, 1, 1, 0, 0, 0]),
            json!([0, 2, 0, 0, 0, 0, 0, 33.3, 1, 0, 0, 0, 0]),
        ];
        assert_eq!(actual.len(), roster.len());
        for ((row, expected), player) in actual.iter().zip(expected).zip(roster) {
            assert_eq!(row["name"], player.name);
            assert_eq!(json!(keys.map(|key| row[key].clone())), expected);
        }
    }

    #[test]
    fn ranking_keeps_kills_first_then_adr_then_folded_name() {
        let stats = [
            json!({"steam_id64":"1","name":"zion","kills":8,"adr":99.9}),
            json!({"steam_id64":"2","name":"Bravo","kills":8,"adr":100.0}),
            json!({"steam_id64":"3","name":"alpha","kills":8,"adr":100.0}),
            json!({"steam_id64":"4","name":"Delta","kills":9,"adr":55.0}),
        ];
        assert_eq!(ranked_player_ids(&stats), ["4", "3", "2", "1"]);
    }

    fn death(
        tick: i64,
        actor: &str,
        target: &str,
        assister: &str,
        headshot: bool,
        weapon: &str,
    ) -> Value {
        json!({ "type":"kill", "tick":tick,"actor":actor,"target":target,"assister":assister,
                "headshot":headshot,"weapon":weapon })
    }
}
