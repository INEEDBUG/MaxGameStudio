//! Fixed-team scoreboard projection for the legacy match workspace.

use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Side {
    T,
    Ct,
}

impl Side {
    pub(super) fn from_engine_team(team: i64) -> Option<Self> {
        match team {
            2 => Some(Self::T),
            3 => Some(Self::Ct),
            _ => None,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::T => "T",
            Self::Ct => "CT",
        }
    }

    fn opposite(self) -> Self {
        match self {
            Self::T => Self::Ct,
            Self::Ct => Self::T,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TeamKey {
    A,
    B,
}

impl TeamKey {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::A => "a",
            Self::B => "b",
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct RoundOutcome {
    pub(super) round_number: i64,
    pub(super) sides: [Side; 2],
    pub(super) winner: Option<TeamKey>,
    pub(super) score_before: [i64; 2],
    pub(super) score_after: [i64; 2],
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct RoundScores {
    pub(super) match_score: Option<[i64; 2]>,
    pub(super) counted_rounds: Option<i64>,
    pub(super) team_groups: Option<[i64; 2]>,
    pub(super) halftime_round: Option<i64>,
    pub(super) regulation_end_round: Option<i64>,
    pub(super) rounds: Vec<RoundOutcome>,
}

impl RoundScores {
    pub(super) fn team_assignment_source(&self) -> &'static str {
        if self.team_groups.is_some() {
            "round_side_groups"
        } else {
            "roster_order_fallback"
        }
    }

    pub(super) fn team_key(&self, final_group: i64) -> Option<TeamKey> {
        let [group_a, group_b] = self.team_groups.unwrap_or([2, 3]);
        if final_group == group_a {
            Some(TeamKey::A)
        } else if final_group == group_b {
            Some(TeamKey::B)
        } else {
            None
        }
    }
}

/// `match_score` comes from the full match summary. Visible windows supply their own tally.
/// `observations` contains every freeze round with pair slots for final groups 2 and 3.
pub(super) fn build(
    match_score: Option<[i64; 2]>,
    observations: &BTreeMap<i64, [Option<Side>; 2]>,
    visible_rounds: &[(i64, Option<Side>)],
) -> RoundScores {
    let opening = observations.values().copied().find_map(group_two_side);
    let team_groups = opening.map(|side| match side {
        Side::T => [2, 3],
        Side::Ct => [3, 2],
    });
    // Seed the leading gaps with the first observation, then carry each known side forward.
    let mut last_observed = opening;
    let group_two_by_round: BTreeMap<_, _> = observations
        .iter()
        .filter_map(|(&round, &seen)| {
            last_observed = group_two_side(seen).or(last_observed);
            last_observed.map(|side| (round, side))
        })
        .collect();
    let halftime_round = group_two_by_round
        .iter()
        .take_while(|(round, _)| **round <= 24)
        .find_map(|(&round, &side)| (Some(side) != opening).then_some(round));
    let regulation_end_round = halftime_round
        .filter(|round| *round > 1)
        .map(|round| (round - 1) * 2);

    let mut tally = [0, 0];
    let rounds = visible_rounds
        .iter()
        .map(|&(round_number, winner_side)| {
            let side_a = match (group_two_by_round.get(&round_number), opening) {
                (Some(&side), Some(Side::T)) => side,
                (Some(&side), Some(Side::Ct)) => side.opposite(),
                _ if round_number <= 12 => Side::T,
                _ => Side::Ct,
            };
            let winner = winner_side.map(|side| {
                if side == side_a {
                    TeamKey::A
                } else {
                    TeamKey::B
                }
            });
            let score_before = tally;
            match winner {
                Some(TeamKey::A) => tally[0] += 1,
                Some(TeamKey::B) => tally[1] += 1,
                None => {}
            }
            RoundOutcome {
                round_number,
                sides: [side_a, side_a.opposite()],
                winner,
                score_before,
                score_after: tally,
            }
        })
        .collect();

    RoundScores {
        match_score,
        counted_rounds: match_score.map(|[a, b]| a + b),
        team_groups,
        halftime_round,
        regulation_end_round,
        rounds,
    }
}

fn group_two_side([group_two, group_three]: [Option<Side>; 2]) -> Option<Side> {
    group_two.or_else(|| group_three.map(Side::opposite))
}

#[cfg(test)]
mod tests {
    use super::*;
    use Side::{Ct, T};
    use TeamKey::{A, B};

    #[test]
    fn public_mirage_keeps_ten_counted_rounds_and_nine_visible_rounds() {
        // Fixed-group freeze samples and visible winner sides recorded from the
        // released workspace on the pinned public test_demo.dem blob.
        let observations = (1..=9).map(|round| (round, [Some(T), Some(Ct)])).collect();
        let visible: Vec<_> = (1..=9)
            .zip([3, 2, 2, 3, 2, 2, 2, 2, 2].map(Side::from_engine_team))
            .collect();
        let scores = build(Some([8, 2]), &observations, &visible);
        assert_eq!(scores.match_score, Some([8, 2]));
        assert_eq!(scores.counted_rounds, Some(10));
        assert_eq!(scores.team_groups, Some([2, 3]));
        assert_eq!(scores.team_key(2), Some(A));
        assert_eq!(scores.team_assignment_source(), "round_side_groups");
        assert_eq!(
            (scores.halftime_round, scores.regulation_end_round),
            (None, None)
        );
        let actual: Vec<_> = scores
            .rounds
            .iter()
            .map(|round| {
                (
                    round.round_number,
                    round.sides.map(Side::label),
                    round.winner.map(TeamKey::label),
                    round.score_before,
                    round.score_after,
                )
            })
            .collect();
        assert_eq!(
            actual,
            [
                (1, ["T", "CT"], Some("b"), [0, 0], [0, 1]),
                (2, ["T", "CT"], Some("a"), [0, 1], [1, 1]),
                (3, ["T", "CT"], Some("a"), [1, 1], [2, 1]),
                (4, ["T", "CT"], Some("b"), [2, 1], [2, 2]),
                (5, ["T", "CT"], Some("a"), [2, 2], [3, 2]),
                (6, ["T", "CT"], Some("a"), [3, 2], [4, 2]),
                (7, ["T", "CT"], Some("a"), [4, 2], [5, 2]),
                (8, ["T", "CT"], Some("a"), [5, 2], [6, 2]),
                (9, ["T", "CT"], Some("a"), [6, 2], [7, 2]),
            ]
        );
    }

    #[test]
    fn sparse_observations_follow_fixed_groups_through_halftime_and_overtime() {
        let observations = BTreeMap::from([
            (1, [None, None]),
            (2, [Some(Ct), Some(Ct)]),
            (12, [None, None]),
            (13, [None, Some(Ct)]),
            (14, [None, None]),
            (25, [Some(Ct), None]),
            (26, [None, None]),
        ]);
        let scores = build(
            Some([15, 15]),
            &observations,
            &[
                (1, Some(T)),
                (2, Some(Ct)),
                (12, None),
                (13, Some(T)),
                (14, Some(Ct)),
                (25, Some(Ct)),
                (26, Some(T)),
            ],
        );
        assert_eq!(scores.match_score, Some([15, 15]));
        assert_eq!(scores.counted_rounds, Some(30));
        assert_eq!(scores.team_groups, Some([3, 2]));
        assert_eq!(
            (scores.team_key(2), scores.team_key(3), scores.team_key(1)),
            (Some(B), Some(A), None)
        );
        assert_eq!(
            (scores.halftime_round, scores.regulation_end_round),
            (Some(13), Some(24))
        );
        let actual: Vec<_> = scores
            .rounds
            .iter()
            .map(|round| {
                (
                    round.sides,
                    round.winner,
                    round.score_before,
                    round.score_after,
                )
            })
            .collect();
        assert_eq!(
            actual,
            [
                ([T, Ct], Some(A), [0, 0], [1, 0]),
                ([T, Ct], Some(B), [1, 0], [1, 1]),
                ([T, Ct], None, [1, 1], [1, 1]),
                ([Ct, T], Some(B), [1, 1], [1, 2]),
                ([Ct, T], Some(A), [1, 2], [2, 2]),
                ([T, Ct], Some(B), [2, 2], [2, 3]),
                ([T, Ct], Some(A), [2, 3], [3, 3]),
            ]
        );
    }

    #[test]
    fn unobserved_groups_use_visible_side_fallback_without_inventing_a_match_total() {
        let scores = build(
            None,
            &BTreeMap::from([(12, [None, None]), (13, [None, None])]),
            &[(12, Some(T)), (13, Some(T))],
        );
        assert_eq!((scores.match_score, scores.counted_rounds), (None, None));
        assert_eq!(scores.team_groups, None);
        assert_eq!(scores.team_key(2), Some(A));
        assert_eq!(scores.team_assignment_source(), "roster_order_fallback");
        assert_eq!(
            (scores.halftime_round, scores.regulation_end_round),
            (None, None)
        );
        assert_eq!(scores.rounds[0].winner, Some(A));
        assert_eq!(scores.rounds[1].winner, Some(B));
        assert_eq!(scores.rounds[1].score_after, [1, 1]);

        let empty = build(Some([0, 0]), &BTreeMap::new(), &[]);
        assert_eq!(empty.counted_rounds, Some(0));
        assert!(empty.rounds.is_empty());
    }

    #[test]
    fn an_overtime_only_side_switch_does_not_mark_regulation_halftime() {
        let scores = build(
            Some([12, 13]),
            &BTreeMap::from([(1, [Some(T), None]), (25, [Some(Ct), None])]),
            &[(25, Some(T))],
        );
        assert_eq!(
            (scores.halftime_round, scores.regulation_end_round),
            (None, None)
        );
        assert_eq!(scores.rounds[0].winner, Some(B));
    }
}
