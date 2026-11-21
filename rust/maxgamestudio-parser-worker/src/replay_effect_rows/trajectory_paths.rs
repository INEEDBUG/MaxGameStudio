//! Compact native projectile rows into the released per-throw flight paths.

use super::{visit_rows, Pass, RawRow, TickWindow};
use crate::{demo_inspection::player_name, Failure};
use parser::{parse_demo::DemoOutput, second_pass::variants::VarVec};
use serde_json::{json, Value};

pub(super) fn build(
    parsed: &DemoOutput,
    window: TickWindow,
    tick_rate: f64,
) -> Result<Vec<Value>, Failure> {
    let mut samples = Vec::new();
    visit_rows(parsed, Pass::Trajectories, window, |tick, row| {
        if let Some(sample) = Sample::read(tick, row) {
            samples.push(sample);
        }
        Ok(())
    })?;
    Ok(compact(&mut samples, tick_rate))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Projectile {
    Smoke,
    Flash,
    He,
    Fire,
    Decoy,
}

impl Projectile {
    fn from_class(class: &str) -> Option<Self> {
        Some(match class {
            "CSmokeGrenadeProjectile" => Self::Smoke,
            "CFlashbangProjectile" => Self::Flash,
            "CHEGrenadeProjectile" => Self::He,
            "CMolotovProjectile" => Self::Fire,
            "CDecoyProjectile" => Self::Decoy,
            _ => return None,
        })
    }

    fn label(self) -> &'static str {
        match self {
            Self::Smoke => "烟雾弹",
            Self::Flash => "闪光弹",
            Self::He => "HE 手雷",
            Self::Fire => "燃烧弹",
            Self::Decoy => "诱饵弹",
        }
    }
}

struct Sample<'a> {
    entity: Option<i64>,
    tick: i32,
    kind: Projectile,
    actor: &'a str,
    steamid: Option<u64>,
    x: f64,
    y: f64,
    z: Option<f64>,
}

impl<'a> Sample<'a> {
    fn read(tick: i32, row: RawRow<'_, 'a>) -> Option<Self> {
        let x = coordinate(&row, "x")?;
        let y = coordinate(&row, "y")?;
        let Some(VarVec::String(classes)) = row.field("grenade_type") else {
            return None;
        };
        let kind = Projectile::from_class(classes[row.index].as_deref()?)?;
        let (actor, steamid) = row.thrower();
        let actor = actor
            .and_then(player_name)
            .filter(|name| !name.eq_ignore_ascii_case("nat") && !name.eq_ignore_ascii_case("none"))
            .unwrap_or_default();
        // The released trajectory shape stringifies an actual numeric zero.
        let steamid = if row.integer("steamid") == Some(0) {
            Some(0)
        } else {
            steamid
        };
        Some(Self {
            entity: row.entity(),
            tick,
            kind,
            actor,
            steamid,
            x,
            y,
            z: coordinate(&row, "z"),
        })
    }

    fn point(&self) -> Value {
        let round = |number: f64| (number * 100.0).round_ties_even() / 100.0;
        let mut point = json!({"tick":self.tick, "x":round(self.x), "y":round(self.y)});
        if let Some(z) = self.z {
            point["z"] = json!(round(z));
        }
        point
    }
}

fn coordinate(row: &RawRow<'_, '_>, name: &str) -> Option<f64> {
    match row.field(name)? {
        VarVec::F32(values) => values[row.index]
            .map(f64::from)
            .filter(|value| value.is_finite()),
        _ => row.integer(name).map(|value| value as f64),
    }
}

fn compact(samples: &mut [Sample<'_>], tick_rate: f64) -> Vec<Value> {
    samples.sort_by_key(|row| (row.entity.is_none(), row.entity, row.tick));
    let gap = (tick_rate * 1.25).round_ties_even().max(32.0) as i64;
    samples
        .chunk_by(|left, right| {
            left.entity == right.entity && i64::from(right.tick) - i64::from(left.tick) <= gap
        })
        .map(trajectory)
        .collect()
}

fn trajectory(rows: &[Sample<'_>]) -> Value {
    let first = &rows[0];
    let mut rows = rows;
    if first.kind == Projectile::Smoke && rows.len() > 2 {
        let last_movement = rows.windows(2).rposition(|pair| {
            let dx = pair[1].x - pair[0].x;
            let dy = pair[1].y - pair[0].y;
            (dx * dx + dy * dy).sqrt() > 0.05
        });
        if let Some(index) = last_movement {
            // A pair starts one row before the movement, then keep one landing sample.
            rows = &rows[..rows.len().min(index + 3)];
        }
    }
    let final_index = rows.len() - 1;
    let last = &rows[final_index];
    let stride = rows.len().div_ceil(56);
    let mut points: Vec<_> = rows.iter().step_by(stride).map(Sample::point).collect();
    if !final_index.is_multiple_of(stride) {
        points.push(last.point());
    }
    json!({
        "kind":first.kind.label(), "actor":first.actor,
        "steamid64":first.steamid.map(|id| id.to_string()).unwrap_or_default(),
        "throw_tick":first.tick, "end_tick":last.tick, "points":points,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replay_effect_rows::{create_huffman_lookup_table, parse, replay_io};
    use std::path::PathBuf;

    #[test]
    fn smoke_keeps_one_landing_sample_and_reused_entity_splits_after_gap() {
        let mut samples = [
            Sample {
                z: Some(31.0),
                ..sample(9, 160, Projectile::Smoke, 180.0, 90.0)
            },
            sample(9, 100, Projectile::Smoke, 0.0, 0.0),
            Sample {
                z: Some(20.0),
                ..sample(9, 130, Projectile::Smoke, 180.0, 90.0)
            },
            sample(9, 110, Projectile::Smoke, 100.0, 50.0),
            sample(9, 120, Projectile::Smoke, 180.0, 90.0),
            Sample {
                z: Some(32.0),
                ..sample(9, 240, Projectile::Smoke, 180.0, 90.0)
            },
            Sample {
                actor: "",
                steamid: Some(0),
                ..sample(9, 321, Projectile::Smoke, 200.0, 0.0)
            },
            sample(9, 355, Projectile::Smoke, 200.0, 0.0),
            sample(9, 331, Projectile::Smoke, 200.0, 0.0),
        ];
        let paths = compact(&mut samples, 64.0);
        assert_eq!(
            json!(paths),
            json!([
                {"kind":"烟雾弹", "actor":"Alpha", "steamid64":"1", "throw_tick":100, "end_tick":130,
                 "points":[{"tick":100,"x":0.0,"y":0.0},{"tick":110,"x":100.0,"y":50.0},
                           {"tick":120,"x":180.0,"y":90.0},{"tick":130,"x":180.0,"y":90.0,"z":20.0}]},
                {"kind":"烟雾弹", "actor":"", "steamid64":"0", "throw_tick":321, "end_tick":355,
                 "points":[{"tick":321,"x":200.0,"y":0.0},{"tick":331,"x":200.0,"y":0.0},
                           {"tick":355,"x":200.0,"y":0.0}]},
            ])
        );
    }

    #[test]
    fn stable_entity_tick_order_retains_duplicate_ticks_and_last_sample() {
        let mut samples: Vec<_> = (100..158)
            .rev()
            .map(|tick| sample(3, tick, Projectile::Flash, f64::from(tick), 0.0))
            .collect();
        samples.push(sample(2, 8, Projectile::He, 1.125, -2.375));
        samples.push(Sample {
            z: Some(5.0),
            ..sample(2, 8, Projectile::He, 0.0, 0.0)
        });
        let paths = compact(&mut samples, 64.0);
        assert_eq!(paths.len(), 2);
        assert_eq!(
            paths[0],
            json!({
                "kind":"HE 手雷", "actor":"Alpha", "steamid64":"1", "throw_tick":8, "end_tick":8,
                "points":[{"tick":8,"x":1.12,"y":-2.38},{"tick":8,"x":0.0,"y":0.0,"z":5.0}],
            })
        );
        let ticks: Vec<_> = paths[1]["points"]
            .as_array()
            .unwrap()
            .iter()
            .map(|point| point["tick"].as_i64().unwrap())
            .collect();
        assert_eq!(
            ticks,
            (100..157).step_by(2).chain([157]).collect::<Vec<_>>()
        );
    }

    #[test]
    #[ignore = "requires the existing pinned public demo via MGS_REPLAY_TRAJECTORY_FIXTURE"]
    fn pinned_public_demo_matches_legacy_projectile_paths() {
        let path = PathBuf::from(
            std::env::var_os("MGS_REPLAY_TRAJECTORY_FIXTURE").expect("public demo fixture path"),
        );
        replay_io::with_demo_bytes(&path, |bytes| {
            let parsed = parse(bytes, &create_huffman_lookup_table(), Pass::Trajectories)?;
            // Ordered output from the released wheel and workspace reducer.
            // Only the digest retains the contribution of names and Steam IDs.
            for (start_tick, end_tick, paths, points, hash) in [
                (0, i32::MAX, 156, 7844, 0x2da6ed174e5d295f),
                (10000, 49999, 126, 6382, 0x8f74279bea2d0b87),
            ] {
                let result = build(
                    &parsed,
                    TickWindow {
                        start_tick,
                        end_tick,
                    },
                    64.0,
                )?;
                assert_eq!(result.len(), paths);
                assert_eq!(
                    result
                        .iter()
                        .map(|path| path["points"].as_array().unwrap().len())
                        .sum::<usize>(),
                    points
                );
                assert_eq!(digest(&result), hash);
            }
            Ok(())
        })
        .expect("pinned public Demo trajectory comparison");
    }

    fn sample(entity: i64, tick: i32, kind: Projectile, x: f64, y: f64) -> Sample<'static> {
        Sample {
            entity: Some(entity),
            tick,
            kind,
            actor: "Alpha",
            steamid: Some(1),
            x,
            y,
            z: None,
        }
    }

    fn digest(paths: &[Value]) -> u64 {
        let cents = |value: &Value| value.as_f64().map(|number| (number * 100.0).round() as i64);
        paths.iter().fold(0xcbf29ce484222325, |hash, path| {
            let points: Vec<_> = path["points"]
                .as_array()
                .unwrap()
                .iter()
                .map(|point| {
                    json!([
                        point["tick"],
                        cents(&point["x"]),
                        cents(&point["y"]),
                        cents(&point["z"]),
                    ])
                })
                .collect();
            json!([
                path["kind"],
                path["actor"],
                path["steamid64"],
                path["throw_tick"],
                path["end_tick"],
                points
            ])
            .to_string()
            .bytes()
            .chain(b"\n".iter().copied())
            .fold(hash, |hash: u64, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
            })
        })
    }
}
