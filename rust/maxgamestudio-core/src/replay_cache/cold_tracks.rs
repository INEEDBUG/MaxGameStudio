use super::Round;
use crate::replay_cache::{entries, integer};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

pub(super) fn clip_effects(effects: Vec<Value>, start: i64, end: i64) -> Vec<Value> {
    effects
        .into_iter()
        .filter_map(|mut effect| {
            let first = effect["start_tick"].as_i64()?;
            let last = effect["end_tick"].as_i64()?;
            if last < start || first > end {
                return None;
            }
            let Value::Array(samples) = effect["samples"].take() else {
                return None;
            };
            let mut prior = None;
            let mut kept = Vec::new();
            for sample in samples {
                let Some(tick) = sample["tick"].as_i64() else {
                    continue;
                };
                if tick < start {
                    prior = Some(sample);
                } else if tick <= end {
                    kept.push(sample);
                } else {
                    break;
                }
            }
            if let Some(mut prior) = prior.filter(|_| {
                kept.first()
                    .is_none_or(|sample| sample["tick"].as_i64() != Some(start))
            }) {
                prior["tick"] = json!(start);
                kept.insert(0, prior);
            }
            if kept.is_empty() {
                return None;
            }
            effect["start_tick"] = json!(first.max(start));
            effect["end_tick"] = json!(last.min(end));
            effect["samples"] = json!(kept);
            // The parser's detonation vector consists of f32 coordinates. Preserve their
            // exact widened representation when the worker's JSON uses decimal floats.
            if let Some(origin) = effect
                .get_mut("stable_origin")
                .and_then(Value::as_array_mut)
            {
                for coordinate in origin {
                    if let Some(value) = coordinate.as_f64() {
                        *coordinate = json!(f64::from(value as f32));
                    }
                }
            }
            Some(effect)
        })
        .collect()
}

pub(super) fn tag_effect_sides(
    effects: &mut [Value],
    rounds: &[Round],
    workspace: &Value,
    tick_rate: f64,
) {
    let teams: HashMap<String, &str> = entries(&workspace["players"])
        .iter()
        .filter_map(|player| {
            let name = clean(&player["name"]).to_lowercase();
            let team = clean(&player["team_key"]);
            match team.to_ascii_lowercase().as_str() {
                "a" if !name.is_empty() => Some((name, "a")),
                "b" if !name.is_empty() => Some((name, "b")),
                _ => None,
            }
        })
        .collect();
    let mut candidates = Vec::new();
    for round in rounds {
        for event in &round.events {
            if event["type"] != "grenade" {
                continue;
            }
            let kind = clean(&event["kind"]).to_lowercase();
            let effect = if kind.contains('烟') || kind.contains("smoke") {
                "smoke"
            } else if ["燃", "火", "inferno", "molotov", "incendiary"]
                .iter()
                .any(|token| kind.contains(token))
            {
                "inferno"
            } else {
                continue;
            };
            let actor = clean(&event["actor"]);
            let team = *teams.get(&actor.to_lowercase()).unwrap_or(&"");
            candidates.push(Candidate {
                effect,
                actor,
                team,
                side: side(round, team),
                tick: integer(&event["tick"]),
                xy: finite(&event["x"]).zip(finite(&event["y"])),
                used: false,
            });
        }
    }
    let limit = (tick_rate * 3.0).max(64.0) as u64;
    for effect in effects {
        let tick = integer(&effect["start_tick"]);
        let owner = [clean(&effect["thrower_name"]), clean(&effect["name"])]
            .into_iter()
            .find(|name| !name.is_empty())
            .unwrap_or_default();
        if let Some((team, window)) = teams.get(&owner.to_lowercase()).zip(
            rounds
                .iter()
                .find(|round| (round.start..=round.end).contains(&tick)),
        ) {
            let owner_side = side(window, team);
            if !owner_side.is_empty() {
                effect["thrower_name"] = json!(owner);
                effect["team_key"] = json!(team);
                effect["side"] = json!(owner_side);
                continue;
            }
        }
        let center = centroid(effect);
        let candidate = candidates
            .iter_mut()
            .filter_map(|candidate| {
                let delta = candidate.tick.abs_diff(tick);
                if candidate.used || effect["type"] != candidate.effect || delta > limit {
                    return None;
                }
                let distance = center
                    .zip(candidate.xy)
                    .map_or(0.0, |((cx, cy), (x, y))| (x - cx).hypot(y - cy));
                Some((delta as f64 + distance / 64.0, candidate))
            })
            .min_by(|(a, _), (b, _)| a.total_cmp(b));
        if let Some((_, candidate)) = candidate {
            candidate.used = true;
            for (field, value) in [
                ("thrower_name", candidate.actor),
                ("team_key", candidate.team),
                ("side", candidate.side),
            ] {
                if !value.is_empty() {
                    effect[field] = json!(value);
                }
            }
        }
    }
}

struct Candidate<'a> {
    effect: &'static str,
    actor: &'a str,
    team: &'a str,
    side: &'a str,
    tick: i64,
    xy: Option<(f64, f64)>,
    used: bool,
}

fn side<'a>(round: &'a Round, team: &str) -> &'a str {
    let side = match team {
        "a" => round.team_a.as_str(),
        "b" => round.team_b.as_str(),
        _ => "",
    };
    if matches!(side, "T" | "CT") {
        side
    } else {
        ""
    }
}

fn centroid(effect: &Value) -> Option<(f64, f64)> {
    for sample in entries(&effect["samples"]) {
        let (mut x, mut y, mut count) = (0.0, 0.0, 0usize);
        for cell in entries(&sample["cells"]) {
            if let Some((cx, cy)) = finite(&cell[0]).zip(finite(&cell[1])) {
                x += cx;
                y += cy;
                count += 1;
            }
        }
        if count > 0 {
            return Some((x / count as f64, y / count as f64));
        }
    }
    None
}

pub(super) fn attach_projectiles(rounds: &mut [Round], projectiles: &[Value], tick_rate: f64) {
    let mut by_round = HashMap::<(i64, &str), Vec<usize>>::new();
    for (index, projectile) in projectiles.iter().enumerate() {
        let tick = integer(&projectile["throw_tick"]);
        if let Some(round) = rounds
            .iter()
            .find(|round| (round.start..=round.end).contains(&tick))
        {
            by_round
                .entry((round.number, clean(&projectile["kind"])))
                .or_default()
                .push(index);
        }
    }
    let tolerance = (tick_rate * 2.0).round_ties_even().max(32.0) as u64;
    let release_tolerance = (tick_rate * 0.75).round_ties_even().max(8.0) as u64;
    let mut used = HashSet::<usize>::new();
    for round in rounds {
        let mut used_release = HashSet::<usize>::new();
        for event in &mut round.events {
            if event["type"] != "grenade" {
                continue;
            }
            let kind = clean(&event["kind"]);
            let actor = clean(&event["actor"]).to_lowercase();
            let event_tick = integer(&event["tick"]);
            let index = by_round
                .get(&(round.number, kind))
                .into_iter()
                .flatten()
                .filter(|index| !used.contains(index))
                .filter_map(|&index| {
                    let projectile = &projectiles[index];
                    let end_delta = integer(&projectile["end_tick"]).abs_diff(event_tick);
                    (integer(&projectile["throw_tick"]) <= event_tick
                        && compatible(&actor, &clean(&projectile["actor"]).to_lowercase())
                        && end_delta <= tolerance)
                        .then_some((end_delta, index))
                })
                .min_by_key(|&(delta, _)| delta);
            let Some((_, index)) = index else { continue };
            used.insert(index);
            let projectile = &projectiles[index];
            let mut start = integer(&projectile["throw_tick"]);
            let mut points = entries(&projectile["points"]).to_vec();
            points.sort_by_key(|point| integer(&point["tick"]));
            let launch_actor = clean(&projectile["actor"]).to_lowercase();
            let steam = clean(&projectile["steamid64"]);
            let release = round
                .releases
                .iter()
                .enumerate()
                .filter(|(index, release)| release["kind"] == kind && !used_release.contains(index))
                .filter_map(|(index, release)| {
                    let tick = integer(&release["tick"]);
                    let name = clean(&release["actor"]).to_lowercase();
                    let delta = tick.abs_diff(start);
                    (tick <= start
                        && delta <= release_tolerance
                        && compatible(&actor, &name)
                        && compatible(&launch_actor, &name)
                        && compatible(steam, clean(&release["steamid64"]))
                        && finite(&release["x"]).is_some()
                        && finite(&release["y"]).is_some())
                    .then_some((delta, index, release))
                })
                .min_by_key(|&(delta, _, _)| delta);
            if let (Some((_, release_index, launch)), Some(first)) = (release, points.first()) {
                used_release.insert(release_index);
                let launch_tick = integer(&launch["tick"]);
                if launch_tick < integer(&first["tick"]) {
                    let mut point = json!({
                        "tick":launch_tick, "x":round_two(finite(&launch["x"]).unwrap()),
                        "y":round_two(finite(&launch["y"]).unwrap()),
                    });
                    if let Some(z) = finite(&first["z"]).or_else(|| finite(&launch["z"])) {
                        point["z"] = json!(round_two(z));
                    }
                    points.insert(0, point);
                    start = launch_tick;
                }
            }
            event["throw_tick"] = json!(start);
            event["trajectory"] = json!(points);
        }
    }
}

fn clean(value: &Value) -> &str {
    value.as_str().map(str::trim).unwrap_or_default()
}

fn finite(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse().ok())
        .filter(|value| value.is_finite())
}

fn round_two(value: f64) -> f64 {
    format!("{value:.2}")
        .parse()
        .expect("formatted finite coordinate")
}

fn compatible(first: &str, second: &str) -> bool {
    first.is_empty() || second.is_empty() || first == second
}
