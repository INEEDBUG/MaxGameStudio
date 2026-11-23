//! Released per-kill action tags for the recording compilations.

pub(super) const ATTACKER_LOOKBACKS: [i64; 6] = [0, 2, 8, 16, 24, 32];

#[derive(Clone, Copy, Default)]
pub(super) struct Position {
    pub(super) x: Option<f64>,
    pub(super) y: Option<f64>,
    pub(super) z: Option<f64>,
}

#[derive(Clone, Copy)]
pub(super) struct Snapshot {
    pub(super) position: Position,
    pub(super) yaw: Option<f64>,
    pub(super) vel_z: Option<f64>,
}

#[derive(Default)]
pub(super) struct Snapshots {
    /// Exact tick samples in ATTACKER_LOOKBACKS order, with missing rows left absent.
    pub(super) attacker: [Option<Snapshot>; ATTACKER_LOOKBACKS.len()],
    pub(super) victim: Option<Snapshot>,
}

pub(super) struct Kill<'a> {
    pub(super) tick: i64,
    pub(super) weapon: &'a str,
    pub(super) headshot: bool,
    pub(super) noscope: bool,
    pub(super) through_smoke: bool,
    pub(super) attacker_blind: bool,
    pub(super) assisted_flash: bool,
    pub(super) attacker_in_air: bool,
    pub(super) penetrated: i64,
    pub(super) penetrated_objects: i64,
    pub(super) attacker_position: Position,
    pub(super) victim_position: Position,
}

pub(super) struct BlindSample {
    pub(super) tick: i64,
    pub(super) duration: f64,
}

pub(super) fn tags(
    kill: &Kill<'_>,
    samples: &Snapshots,
    victim_blinds: &[BlindSample],
) -> Vec<String> {
    let sniper = matches!(kill.weapon, "awp" | "ssg08");
    let pistol = matches!(
        kill.weapon,
        "glock" | "hkp2000" | "usp_silencer" | "p250" | "cz75a" | "fiveseven" | "tec9" | "elite"
    );
    let mut tags = Vec::new();
    if sniper && kill.noscope {
        tags.push("🙈 盲狙");
    }
    if kill.penetrated > 0 {
        tags.push("🧱 穿墙杀");
    }
    if kill.through_smoke {
        tags.push("🌫️ 混烟");
    }
    if kill.attacker_blind {
        tags.push("😎 全白反杀");
    }
    if kill.assisted_flash && keep_flash(kill.tick, victim_blinds) {
        tags.push("🤝 好闪配好人");
    }
    if kill.headshot {
        tags.push("爆头");
    }
    if pistol && kill.headshot {
        tags.push("🔫 手枪哥");
    }
    if kill.attacker_in_air {
        tags.push("🛸 乌鸦坐飞机");
    }
    if kill.penetrated > 0 && kill.penetrated_objects >= 2 {
        tags.push("🔀 连穿");
    }

    let [now, before_2, before_8, before_16, before_24, before_32] = samples.attacker;
    let attacker = kill.attacker_position.with_snapshot_fallback(now);
    let victim = kill.victim_position.with_snapshot_fallback(samples.victim);
    if let Some(distance) = attacker.distance(victim) {
        if distance <= 60.0 && kill.headshot {
            tags.push("👃 零距离");
        } else if distance <= 120.0 {
            tags.push("🫵 贴脸超度");
        }
        if kill.penetrated > 0 && distance > 400.0 {
            tags.push("🎯 超远穿墙");
        }
    }

    if !known_knife(kill.weapon)
        && now
            .zip(samples.victim)
            .and_then(|(attacker, victim)| behind_victim(attacker, victim))
            .unwrap_or(false)
    {
        tags.push("🔙 偷背身");
    }

    let jump = is_jump(now, before_8, before_16);
    if let Some((dx, dy)) = displacement(now, before_8) {
        // Legacy movement samples are at fixed intervals on the 64-tick timeline.
        let speed = dx.hypot(dy) * 8.0;
        if speed > 220.0 {
            tags.push("🚀 上去就是干");
        } else if (120.0..=220.0).contains(&speed) && !jump && !kill.noscope {
            let immediate = displacement(now, before_2);
            if immediate.is_none_or(|(dx, dy)| dx.hypot(dy) * 32.0 >= 70.0) {
                tags.push("🏃‍♂️ 跑打");
            }
        }
    }

    let yaw = now.and_then(|sample| sample.yaw);
    if !jump {
        if let Some(((dx, dy), yaw)) = displacement(now, before_16).zip(yaw) {
            if dx.hypot(dy) * 4.0 > 150.0 && angle_diff(dy.atan2(dx).to_degrees(), yaw) >= 45.0 {
                tags.push("🎿 一个大拉");
            }
        }
    }
    if !kill.attacker_in_air
        && now
            .and_then(|sample| sample.vel_z)
            .is_some_and(|speed| speed > 80.0)
    {
        tags.push("🛸 乌鸦坐飞机");
    }
    if sniper
        && yaw.is_some_and(|yaw| {
            [before_8, before_16, before_24, before_32]
                .into_iter()
                .flatten()
                .filter_map(|sample| sample.yaw)
                .any(|past| angle_diff(yaw, past) >= 25.0)
        })
    {
        tags.push("🌪️ 甩狙");
    }

    tags.into_iter().map(str::to_owned).collect()
}

impl Position {
    fn xy(self) -> Option<(f64, f64)> {
        self.x.zip(self.y)
    }

    fn with_snapshot_fallback(self, snapshot: Option<Snapshot>) -> Self {
        if self.x.is_some() {
            return self;
        }
        match snapshot.map(|row| row.position) {
            Some(position)
                if position.x.is_some() && position.y.is_some() && position.z.is_some() =>
            {
                position
            }
            _ => self,
        }
    }

    fn distance(self, other: Self) -> Option<f64> {
        let ((ax, ay), (bx, by)) = self.xy().zip(other.xy())?;
        let dz = self.z.zip(other.z).map_or(0.0, |(a, b)| a - b);
        let dx = ax - bx;
        let dy = ay - by;
        Some((dx * dx + dy * dy + dz * dz).sqrt())
    }
}

fn displacement(now: Option<Snapshot>, before: Option<Snapshot>) -> Option<(f64, f64)> {
    let ((ax, ay), (bx, by)) = now?.position.xy().zip(before?.position.xy())?;
    Some((ax - bx, ay - by))
}

fn is_jump(now: Option<Snapshot>, before_8: Option<Snapshot>, before_16: Option<Snapshot>) -> bool {
    let Some(current) = now else {
        return false;
    };
    let vertical_speed = [now, before_8, before_16]
        .into_iter()
        .flatten()
        .filter_map(|sample| sample.vel_z)
        .any(|speed| speed.abs() > 80.0);
    vertical_speed
        || current
            .position
            .z
            .zip(before_16.and_then(|sample| sample.position.z))
            .is_some_and(|(z, before)| (z - before).abs() > 20.0)
}

fn behind_victim(attacker: Snapshot, victim: Snapshot) -> Option<bool> {
    let ((ax, ay), (vx, vy)) = attacker.position.xy().zip(victim.position.xy())?;
    let yaw = victim.yaw?;
    if ![ax, ay, vx, vy, yaw].into_iter().all(f64::is_finite) {
        return None;
    }
    let dx = ax - vx;
    let dy = ay - vy;
    Some(dx.hypot(dy) >= 1.0 && angle_diff(dy.atan2(dx).to_degrees(), yaw) > 45.0)
}

fn angle_diff(a: f64, b: f64) -> f64 {
    ((a - b + 180.0).rem_euclid(360.0) - 180.0).abs()
}

fn keep_flash(tick: i64, victim_blinds: &[BlindSample]) -> bool {
    if victim_blinds.is_empty() {
        return true;
    }
    let window = tick.saturating_sub(192)..=tick;
    let duration = victim_blinds
        .iter()
        .filter(|sample| window.contains(&sample.tick))
        .map(|sample| sample.duration)
        // Python max retains the earlier value when the two values are unordered.
        .reduce(|best, next| if next > best { next } else { best })
        .unwrap_or(0.0);
    duration >= 1.5 || duration.is_nan()
}

fn known_knife(weapon: &str) -> bool {
    matches!(
        weapon,
        "knife"
            | "knife_t"
            | "knife_ct"
            | "bayonet"
            | "knife_karambit"
            | "knife_m9_bayonet"
            | "knife_butterfly"
            | "knife_flip"
            | "knife_gut"
            | "knife_tactical"
            | "knife_falchion"
            | "knife_survival_bowie"
            | "knife_push"
            | "knife_cord"
            | "knife_canis"
            | "knife_ursus"
            | "knife_gypsy_jackknife"
            | "knife_outdoor"
            | "knife_stiletto"
            | "knife_widowmaker"
            | "knife_skeleton"
            | "knife_css"
            | "knife_kukri"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kill(weapon: &str) -> Kill<'_> {
        Kill {
            tick: 400,
            weapon,
            headshot: false,
            noscope: false,
            through_smoke: false,
            attacker_blind: false,
            assisted_flash: false,
            attacker_in_air: false,
            penetrated: 0,
            penetrated_objects: 0,
            attacker_position: Position::default(),
            victim_position: Position::default(),
        }
    }

    fn snapshot(x: f64, y: f64, z: f64, yaw: f64) -> Snapshot {
        Snapshot {
            position: Position {
                x: Some(x),
                y: Some(y),
                z: Some(z),
            },
            yaw: Some(yaw),
            vel_z: None,
        }
    }

    #[test]
    fn base_order_and_unrounded_event_distance() {
        let mut event = Kill {
            headshot: true,
            through_smoke: true,
            attacker_blind: true,
            assisted_flash: true,
            attacker_in_air: true,
            penetrated: 1,
            penetrated_objects: 2,
            attacker_position: snapshot(0.0, 0.0, 0.0, 0.0).position,
            victim_position: snapshot(60.0, 0.0, 0.0, 0.0).position,
            ..kill("glock")
        };
        let samples = Snapshots {
            victim: Some(snapshot(999.0, 0.0, 0.0, 0.0)),
            ..Snapshots::default()
        };
        let expected = [
            "🧱 穿墙杀",
            "🌫️ 混烟",
            "😎 全白反杀",
            "🤝 好闪配好人",
            "爆头",
            "🔫 手枪哥",
            "🛸 乌鸦坐飞机",
            "🔀 连穿",
            "👃 零距离",
        ];
        assert_eq!(tags(&event, &samples, &[]), expected);
        event.victim_position.x = Some(60.000_1);
        assert_eq!(tags(&event, &samples, &[]).last().unwrap(), "🫵 贴脸超度");
        event.victim_position.x = Some(400.0);
        assert_eq!(tags(&event, &samples, &[]).last().unwrap(), "🔀 连穿");
        event.victim_position.x = Some(400.000_1);
        assert_eq!(tags(&event, &samples, &[]).last().unwrap(), "🎯 超远穿墙");
        let deagle = Kill {
            headshot: true,
            ..kill("deagle")
        };
        assert_eq!(tags(&deagle, &Snapshots::default(), &[]), ["爆头"]);
    }

    #[test]
    fn flash_quality_uses_both_inclusive_edges() {
        let event = Kill {
            assisted_flash: true,
            ..kill("ak47")
        };
        let samples = Snapshots::default();
        assert_eq!(tags(&event, &samples, &[]), ["🤝 好闪配好人"]);
        for (tick, duration, expected) in [
            (207, 9.0, false),
            (208, 1.5, true),
            (400, 1.49, false),
            (400, 1.5, true),
            (401, 9.0, false),
        ] {
            let sample = BlindSample { tick, duration };
            assert_eq!(
                !tags(&event, &samples, &[sample]).is_empty(),
                expected,
                "blind sample at {tick} lasting {duration}",
            );
        }
    }

    #[test]
    fn jump_blocks_run_and_peek_but_preserves_rush() {
        let event = kill("ak47");
        let mut samples = Snapshots {
            attacker: [
                Some(snapshot(0.0, 0.0, 0.0, 0.0)),
                Some(snapshot(-3.0, 0.0, 0.0, 0.0)),
                Some(snapshot(-25.0, 0.0, 0.0, 0.0)),
                Some(snapshot(4.0, -40.0, 20.0, 0.0)),
                None,
                None,
            ],
            victim: Some(snapshot(-1000.0, 0.0, 0.0, 180.0)),
        };
        assert_eq!(
            tags(&event, &samples, &[]),
            ["🔙 偷背身", "🏃‍♂️ 跑打", "🎿 一个大拉"],
        );
        samples.attacker[3].as_mut().unwrap().position.z = Some(20.000_1);
        assert_eq!(tags(&event, &samples, &[]), ["🔙 偷背身"]);
        samples.attacker[2].as_mut().unwrap().position.x = Some(-27.500_1);
        samples.attacker[0].as_mut().unwrap().vel_z = Some(80.000_1);
        assert_eq!(
            tags(&event, &samples, &[]),
            ["🔙 偷背身", "🚀 上去就是干", "🛸 乌鸦坐飞机"],
        );
    }

    #[test]
    fn wrapped_sniper_lookback_and_known_knife() {
        let event = Kill {
            noscope: true,
            ..kill("ssg08")
        };
        let samples = Snapshots {
            attacker: [
                Some(snapshot(0.0, 0.0, 0.0, -175.0)),
                None,
                Some(snapshot(0.0, 0.0, 0.0, 179.0)),
                None,
                None,
                Some(snapshot(0.0, 0.0, 0.0, 160.0)),
            ],
            victim: Some(snapshot(-1000.0, 0.0, 0.0, 180.0)),
        };
        assert_eq!(
            tags(&event, &samples, &[]),
            ["🙈 盲狙", "🔙 偷背身", "🌪️ 甩狙"],
        );
        assert!(tags(&kill("knife_m9_bayonet"), &samples, &[]).is_empty());
        assert_eq!(tags(&kill("knife_unknown"), &samples, &[]), ["🔙 偷背身"]);
    }
}
